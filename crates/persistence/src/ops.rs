use bevy::ecs::component::Component;
use bevy::log::error;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{block_on, AsyncComputeTaskPool, Task};

use crate::store::{PersistenceError, Store};

enum StoreOp<K, V> {
    Load {
        key: K,
        result: Result<Option<V>, PersistenceError>,
    },
    Save {
        key: K,
        result: Result<(), PersistenceError>,
    },
}

/// Manages async persistence tasks for a `Store<K, V>`.
///
/// Attach as a Component on map entities alongside their `StoreBackend`.
/// Consumer systems call `poll()` each frame and drain `completed_loads`.
#[derive(Component)]
pub struct PendingStoreOps<K: Send + Sync + 'static, V: Send + Sync + 'static> {
    tasks: Vec<Task<StoreOp<K, V>>>,
    /// Completed load results, drained by consumer systems.
    pub completed_loads: Vec<(K, Option<V>)>,
    /// Load errors, drained by consumer systems.
    pub load_errors: Vec<(K, PersistenceError)>,
}

impl<K: Send + Sync + 'static, V: Send + Sync + 'static> Default for PendingStoreOps<K, V> {
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            completed_loads: Vec::new(),
            load_errors: Vec::new(),
        }
    }
}

impl<K, V> PendingStoreOps<K, V>
where
    K: Send + Sync + Clone + std::fmt::Debug + 'static,
    V: Send + Sync + 'static,
{
    /// Returns `true` if there are in-flight tasks.
    pub fn has_pending(&self) -> bool {
        !self.tasks.is_empty()
    }

    /// Spawn an async save task.
    pub fn spawn_save<B: Store<K, V>>(&mut self, store: &B, key: K, value: V) {
        let pool = AsyncComputeTaskPool::get();
        let store = store.clone();
        let key_clone = key.clone();
        self.tasks.push(pool.spawn(async move {
            let result = store.save(&key_clone, &value);
            StoreOp::Save {
                key: key_clone,
                result,
            }
        }));
    }

    /// Spawn an async load task.
    pub fn spawn_load<B: Store<K, V>>(&mut self, store: &B, key: K) {
        let pool = AsyncComputeTaskPool::get();
        let store = store.clone();
        let key_clone = key.clone();
        self.tasks.push(pool.spawn(async move {
            let result = store.load(&key_clone);
            StoreOp::Load {
                key: key_clone,
                result,
            }
        }));
    }

    /// Poll completed tasks. Moves load results into `completed_loads`.
    /// Save errors are logged directly.
    pub fn poll(&mut self) {
        let mut i = 0;
        while i < self.tasks.len() {
            if let Some(op) = check_ready(&mut self.tasks[i]) {
                let _ = self.tasks.swap_remove(i);
                match op {
                    StoreOp::Load {
                        key,
                        result: Ok(value),
                    } => {
                        self.completed_loads.push((key, value));
                    }
                    StoreOp::Load {
                        key,
                        result: Err(e),
                    } => {
                        self.load_errors.push((key, e));
                    }
                    StoreOp::Save {
                        key: _,
                        result: Ok(()),
                    } => {}
                    StoreOp::Save {
                        key,
                        result: Err(e),
                    } => {
                        error!("Store save error at {key:?}: {e}");
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    /// Block until all in-flight tasks complete. Used during shutdown.
    pub fn flush(&mut self) {
        for task in self.tasks.drain(..) {
            let op = block_on(task);
            match op {
                StoreOp::Save {
                    key,
                    result: Err(e),
                } => {
                    error!("Store save error at {key:?} during flush: {e}");
                }
                StoreOp::Load { key, result } => match result {
                    Ok(value) => self.completed_loads.push((key, value)),
                    Err(e) => self.load_errors.push((key, e)),
                },
                _ => {}
            }
        }
    }
}
