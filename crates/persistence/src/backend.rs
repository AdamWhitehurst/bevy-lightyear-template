use std::marker::PhantomData;

use bevy::ecs::component::Component;

use crate::store::Store;

/// Holds a persistence backend on a map entity.
#[derive(Component)]
pub struct StoreBackend<K, V, B>(pub B, PhantomData<fn(K, V)>)
where
    K: Send + Sync + 'static,
    V: Send + Sync + 'static,
    B: Store<K, V>;

impl<K, V, B> StoreBackend<K, V, B>
where
    K: Send + Sync + 'static,
    V: Send + Sync + 'static,
    B: Store<K, V>,
{
    pub fn new(backend: B) -> Self {
        Self(backend, PhantomData)
    }
}
