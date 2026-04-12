# Implementation Plan

## Overview

A new `persistence` crate hosts a synchronous `Store<K, V>` trait with static dispatch. All persistence state lives as Components on map entities — matching the existing `PendingChunks`/`PendingSaves`/`PendingRemeshes` pattern. Every map (overworld, homebases, arenas) goes through the same spawn → async load → configure → ready lifecycle.

## Architecture

### Component-only design

`StoreBackend` and `PendingStoreOps` are Components on map entities. No Resources. Each map entity owns its stores, enabling multi-map persistence out of the box.

### Map spawn lifecycle

Every map follows the same pattern:

1. Spawn map entity with store Components + `MapLoadState::AwaitingMeta`
2. `load_map_meta` system: sees `AwaitingMeta`, calls `ops.spawn_load()` for meta
3. `poll_map_meta` system: drains completed meta load, configures entity (seed, generator), transitions to `AwaitingEntities`, spawns entity load
4. `poll_map_entities` system: drains completed entity load, spawns entities, transitions to `Ready`
5. Generation systems gate on `MapLoadState::Ready`

### Store trait — synchronous, static dispatch

```rust
pub trait Store<K, V>: Send + Sync + Clone + 'static {
    fn save(&self, key: &K, value: &V) -> Result<(), PersistenceError>;
    fn load(&self, key: &K) -> Result<Option<V>, PersistenceError>;
}
```

Synchronous — callers wrap in `AsyncComputeTaskPool::spawn`. `Clone` required so stores can be cloned into async tasks. Filesystem stores are cheap to clone (wrap `Arc<PathBuf>`).

### Per-map stores

All stores take `map_dir: Arc<PathBuf>` — the resolved directory for their specific map. No key-based path resolution. Each map entity gets its own store instances at spawn time.

| Data type | Key | Value | Store | On entity |
|---|---|---|---|---|
| Map meta | `()` | `MapMeta` | `FsMapMetaStore` | Yes |
| Map entities | `()` | `Vec<SavedEntity>` | `FsMapEntitiesStore` | Yes |
| Chunk entities | `IVec3` | `Vec<WorldObjectSpawn>` | `FsChunkEntitiesStore` | Yes |
| Chunks | `IVec3` | `ChunkFileEnvelope` | `FsChunkStore` | Yes |

Map-level stores use `()` as key — one meta / one entity set per map. The store already knows its directory.

---

## Phase 1: Persistence Crate + Map Meta Migration

### Changes

#### 1. Workspace Cargo.toml
**File**: `Cargo.toml`
**Action**: modify

Add `"crates/persistence"` to `members`. Add `persistence = { path = "crates/persistence" }` to `[workspace.dependencies]`.

#### 2. New crate: `crates/persistence/Cargo.toml`
**File**: `crates/persistence/Cargo.toml`
**Action**: create

```toml
[package]
name = "persistence"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true, features = ["bevy_log"] }
```

#### 3. Trait + error: `crates/persistence/src/store.rs`
**File**: `crates/persistence/src/store.rs`
**Action**: create

```rust
#[derive(Debug)]
pub enum PersistenceError {
    Io(std::io::Error),
    Serialize(String),
    Deserialize(String),
    VersionMismatch { expected: u32, actual: u32 },
}

impl std::fmt::Display for PersistenceError { /* variant formatting */ }
impl std::error::Error for PersistenceError {}
impl From<std::io::Error> for PersistenceError { /* Io variant */ }

/// Synchronous key-value persistence backend.
///
/// Blocking IO is fine — callers run these from the async task pool.
/// `Clone` required so the store can be cloned into async task closures.
pub trait Store<K, V>: Send + Sync + Clone + 'static {
    fn save(&self, key: &K, value: &V) -> Result<(), PersistenceError>;
    fn load(&self, key: &K) -> Result<Option<V>, PersistenceError>;
}
```

#### 4. Ops: `crates/persistence/src/ops.rs`
**File**: `crates/persistence/src/ops.rs`
**Action**: create

```rust
pub(crate) enum StoreOp<K, V> {
    Load { key: K, result: Result<Option<V>, PersistenceError> },
    Save { key: K, result: Result<(), PersistenceError> },
}

#[derive(Component)]
pub struct PendingStoreOps<K: Send + Sync + 'static, V: Send + Sync + 'static> {
    tasks: Vec<Task<StoreOp<K, V>>>,
    /// Completed load results, drained by consumer systems.
    pub completed_loads: Vec<(K, Option<V>)>,
    /// Load errors, drained by consumer systems.
    pub load_errors: Vec<(K, PersistenceError)>,
}
```

Public API:

```rust
impl<K, V> PendingStoreOps<K, V>
where K: Send + Sync + Clone + 'static, V: Send + Sync + 'static,
{
    /// Spawn an async save task.
    pub fn spawn_save<B: Store<K, V>>(&mut self, store: &B, key: K, value: V) {
        let pool = AsyncComputeTaskPool::get();
        let store = store.clone();
        let key_clone = key.clone();
        self.tasks.push(pool.spawn(async move {
            let result = store.save(&key_clone, &value);
            StoreOp::Save { key: key_clone, result }
        }));
    }

    /// Spawn an async load task.
    pub fn spawn_load<B: Store<K, V>>(&mut self, store: &B, key: K) {
        let pool = AsyncComputeTaskPool::get();
        let store = store.clone();
        let key_clone = key.clone();
        self.tasks.push(pool.spawn(async move {
            let result = store.load(&key_clone);
            StoreOp::Load { key: key_clone, result }
        }));
    }

    /// Poll completed tasks. Moves load results into `completed_loads`.
    /// Save errors are logged directly.
    pub fn poll(&mut self) {
        let mut i = 0;
        while i < self.tasks.len() {
            if let Some(op) = check_ready(&mut self.tasks[i]) {
                self.tasks.swap_remove(i);
                match op {
                    StoreOp::Load { key, result: Ok(value) } => {
                        self.completed_loads.push((key, value));
                    }
                    StoreOp::Load { key, result: Err(e) } => {
                        self.load_errors.push((key, e));
                    }
                    StoreOp::Save { key: _, result: Ok(()) } => {}
                    StoreOp::Save { key, result: Err(e) } => {
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
            let op = bevy::tasks::block_on(task);
            match op {
                StoreOp::Save { key, result: Err(e) } => {
                    error!("Store save error at {key:?} during flush: {e}");
                }
                StoreOp::Load { key, result } => {
                    match result {
                        Ok(value) => self.completed_loads.push((key, value)),
                        Err(e) => self.load_errors.push((key, e)),
                    }
                }
                _ => {}
            }
        }
    }
}
```

Key design: `poll()` is called every frame by consumer systems. `flush()` blocks during shutdown. `spawn_save`/`spawn_load` clone the store into the task (stores are cheap to clone — they wrap `Arc<PathBuf>`). No `Arc<B>` wrapper needed on `StoreBackend` — the store itself is cloneable.

#### 5. StoreBackend component: `crates/persistence/src/plugin.rs`
**File**: `crates/persistence/src/plugin.rs`
**Action**: create

```rust
/// Holds a persistence backend on a map entity.
#[derive(Component)]
pub struct StoreBackend<K, V, B>(pub B, PhantomData<fn(K, V)>)
where
    K: Send + Sync + 'static,
    V: Send + Sync + 'static,
    B: Store<K, V>;

impl<K, V, B> StoreBackend<K, V, B>
where K: Send + Sync + 'static, V: Send + Sync + 'static, B: Store<K, V>,
{
    pub fn new(backend: B) -> Self {
        Self(backend, PhantomData)
    }
}
```

No `StorePlugin` needed — the persistence crate provides types and methods. Consumer code inserts Components on map entities and registers its own poll systems. No generic plugin registration boilerplate.

#### 6. Crate root: `crates/persistence/src/lib.rs`
**File**: `crates/persistence/src/lib.rs`
**Action**: create

```rust
pub mod ops;
pub mod plugin;
pub mod store;

pub use ops::PendingStoreOps;
pub use plugin::StoreBackend;
pub use store::*;
```

#### 7. Server dependency
**File**: `crates/server/Cargo.toml`
**Action**: modify

Add `persistence = { workspace = true }` to `[dependencies]`.

#### 8. Server persistence module → directory
**File**: `crates/server/src/persistence.rs` → `crates/server/src/persistence/mod.rs`
**Action**: rename (git mv)

Keep all existing content. Add `pub mod fs_map_meta;`.

#### 9. Filesystem backend: `crates/server/src/persistence/fs_map_meta.rs`
**File**: `crates/server/src/persistence/fs_map_meta.rs`
**Action**: create

```rust
use persistence::{PersistenceError, Store};
use std::path::PathBuf;
use std::sync::Arc;
use super::MapMeta;

#[derive(Clone)]
pub struct FsMapMetaStore {
    pub map_dir: Arc<PathBuf>,
}

impl Store<(), MapMeta> for FsMapMetaStore {
    fn save(&self, _key: &(), value: &MapMeta) -> Result<(), PersistenceError> {
        super::save_map_meta(&self.map_dir, value)
            .map_err(|e| PersistenceError::Serialize(e))
    }

    fn load(&self, _key: &()) -> Result<Option<MapMeta>, PersistenceError> {
        super::load_map_meta(&self.map_dir)
            .map_err(|e| PersistenceError::Deserialize(e))
    }
}
```

Key is `()` — one meta per map, directory already known. Free functions stay until Phase 5.

#### 10. `MapLoadState` component
**File**: `crates/server/src/map.rs`
**Action**: modify

```rust
#[derive(Component, PartialEq, Eq)]
pub enum MapLoadState {
    AwaitingMeta,
    AwaitingEntities,
    Ready,
}
```

Component on every map entity. Systems gate on this. Replaces the old synchronous `spawn_overworld` → `load_startup_entities` chain.

#### 11. Restructure `spawn_overworld`
**File**: `crates/server/src/map.rs`
**Action**: modify

Replace the single `spawn_overworld` function with two systems:

**`init_overworld_entity`** — runs `OnEnter(AppState::Ready)`:
```rust
fn init_overworld_entity(mut commands: Commands, save_path: Res<WorldSavePath>) {
    let map_dir = Arc::new(map_save_dir(&save_path.0, &MapInstanceId::Overworld));

    let map = commands.spawn((
        MapInstanceId::Overworld,
        MapLoadState::AwaitingMeta,
        // Meta store + ops
        StoreBackend::new(FsMapMetaStore { map_dir: map_dir.clone() }),
        PendingStoreOps::<(), MapMeta>::default(),
    )).id();

    commands.insert_resource(OverworldMap(map));
}
```

Spawns the entity with store Components and `MapLoadState::AwaitingMeta`. No config, no generator yet — those come after meta loads.

**`poll_map_meta`** — runs `Update`:
```rust
fn poll_map_meta(
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &mut PendingStoreOps<(), MapMeta>,
        &StoreBackend<(), MapMeta, FsMapMetaStore>,
        &mut MapLoadState,
    )>,
    terrain_registry: Res<TerrainDefRegistry>,
    // ...
) {
    for (entity, mut ops, store, mut state) in &mut query {
        if *state != MapLoadState::AwaitingMeta { continue; }

        // First frame: kick off the load
        if ops.completed_loads.is_empty() && ops.tasks.is_empty() && ops.load_errors.is_empty() {
            ops.spawn_load(&store.0, ());
            return;
        }

        ops.poll();

        if let Some((_, meta_opt)) = ops.completed_loads.pop() {
            let (seed, gen_version) = match meta_opt {
                Some(meta) => (meta.seed, meta.generation_version),
                None => (DEFAULT_OVERWORLD_SEED, GENERATION_VERSION),
            };

            // Configure entity — same logic as old spawn_overworld
            let terrain_def = terrain_registry.get("overworld").expect("...");
            let dimensions = terrain_def.map_dimensions().expect("...");
            let mut config = VoxelMapConfig::new(seed, gen_version, 2, true);
            config.save_dir = Some(store.0.map_dir.as_ref().clone());

            let instance = VoxelMapInstance::new(dimensions.tree_height, dimensions.chunk_size);
            let generator = build_generator_from_def(terrain_def, seed, ...);

            commands.entity(entity).insert((config, instance, dimensions, generator));

            *state = MapLoadState::AwaitingEntities;
        }
    }
}
```

The same `init_*` + `poll_*` pattern works for homebases — just a different trigger and terrain def.

#### 12. Migrate saves
**File**: `crates/server/src/map.rs`
**Action**: modify

`save_dirty_chunks_debounced` — query map entity's meta store component directly:
```rust
for (mut instance, config, map_id, mut pending_saves,
     meta_store, mut meta_ops) in &mut map_query
{
    // ...existing debounce logic...
    let meta = MapMeta { version: 1, seed: config.seed, ... };
    meta_ops.spawn_save(&meta_store.0, (), meta);
}
```

`save_world_on_shutdown` — same query, but `flush()` after:
```rust
for (..., meta_store, mut meta_ops) in &mut map_query {
    meta_ops.spawn_save(&meta_store.0, (), meta);
    meta_ops.flush();
}
```

No events for saves. Direct Component access.

#### 13. Register systems in `ServerMapPlugin`
**File**: `crates/server/src/map.rs`
**Action**: modify

```rust
app.add_systems(OnEnter(AppState::Ready), init_overworld_entity)
   .add_systems(Update, (
       poll_map_meta,
       poll_map_entities,
       // ...existing systems gated on MapLoadState::Ready...
   ));
```

### Verification
#### Automated
- [x] `cargo check-all` passes

#### Manual
- [ ] `cargo server` — delete world dir, start server, confirm fresh seed via trace log
- [ ] Stop server, restart, confirm seed matches across runs
- [ ] Check no persistence errors in logs on happy path

---

## Phase 2: Map Entities Migration

### Changes

#### 1. Filesystem backend: `crates/server/src/persistence/fs_map_entities.rs`
**File**: `crates/server/src/persistence/fs_map_entities.rs`
**Action**: create

```rust
#[derive(Clone)]
pub struct FsMapEntitiesStore {
    pub map_dir: Arc<PathBuf>,
}

impl Store<(), Vec<SavedEntity>> for FsMapEntitiesStore {
    fn save(&self, _key: &(), value: &Vec<SavedEntity>) -> Result<(), PersistenceError> {
        super::save_entities(&self.map_dir, value)
            .map_err(|e| PersistenceError::Serialize(e))
    }

    fn load(&self, _key: &()) -> Result<Option<Vec<SavedEntity>>, PersistenceError> {
        let entities = super::load_entities(&self.map_dir)
            .map_err(|e| PersistenceError::Deserialize(e))?;
        if entities.is_empty() { Ok(None) } else { Ok(Some(entities)) }
    }
}
```

Key is `()` — same per-entity pattern as meta.

#### 2. Insert entity store components at map spawn
**File**: `crates/server/src/map.rs`
**Action**: modify

In `init_overworld_entity`, add to the spawn bundle:
```rust
StoreBackend::new(FsMapEntitiesStore { map_dir: map_dir.clone() }),
PendingStoreOps::<(), Vec<SavedEntity>>::default(),
```

#### 3. `poll_map_entities` system
**File**: `crates/server/src/map.rs`
**Action**: modify

New system, runs in `Update`. Queries entities with `MapLoadState::AwaitingEntities`:
- First frame: `ops.spawn_load(&store.0, ())`
- On completed load: spawn `RespawnPoint` entities, set `MapLoadState::Ready`

#### 4. Migrate `collect_and_save_entities`
**File**: `crates/server/src/map.rs`
**Action**: modify

`save_dirty_chunks_debounced`: query map entity's entity store component, call `ops.spawn_save(&store.0, (), entities)`.

`save_world_on_shutdown`: same pattern + `flush()`.

### Verification
#### Automated
- [ ] `cargo check-all` passes

#### Manual
- [ ] `cargo server` — place a world object, trigger debounced save, restart, confirm object reappears
- [ ] Verify startup entity loading: respawn points load correctly after restart

---

## Phase 3: Chunk Entities Migration

### Changes

#### 1. `voxel_map_engine` dependency on `persistence`
**File**: `crates/voxel_map_engine/Cargo.toml`
**Action**: modify

Add `persistence = { workspace = true }` to `[dependencies]`.

#### 2. Convert `voxel_map_engine/src/persistence.rs` to directory
**File**: `crates/voxel_map_engine/src/persistence.rs` → `crates/voxel_map_engine/src/persistence/mod.rs`
**Action**: rename (git mv)

Keep all existing content. Add `pub mod fs_chunk_entities;`.

#### 3. Filesystem backend: `crates/voxel_map_engine/src/persistence/fs_chunk_entities.rs`
**File**: `crates/voxel_map_engine/src/persistence/fs_chunk_entities.rs`
**Action**: create

```rust
#[derive(Clone)]
pub struct FsChunkEntitiesStore {
    pub map_dir: Arc<PathBuf>,
}

impl Store<IVec3, Vec<WorldObjectSpawn>> for FsChunkEntitiesStore {
    fn save(&self, key: &IVec3, value: &Vec<WorldObjectSpawn>) -> Result<(), PersistenceError> {
        super::save_chunk_entities(&self.map_dir, *key, value)
            .map_err(|e| PersistenceError::Serialize(e))
    }

    fn load(&self, key: &IVec3) -> Result<Option<Vec<WorldObjectSpawn>>, PersistenceError> {
        super::load_chunk_entities(&self.map_dir, *key)
            .map_err(|e| PersistenceError::Deserialize(e))
    }
}
```

#### 4. Insert components at map spawn
**File**: `crates/server/src/map.rs`
**Action**: modify

In `init_overworld_entity`, add:
```rust
StoreBackend::new(FsChunkEntitiesStore { map_dir: map_dir.clone() }),
PendingStoreOps::<IVec3, Vec<WorldObjectSpawn>>::default(),
```

#### 5. Migrate `save_new_chunk_entities` and `evict_chunk_entities`
**File**: `crates/server/src/chunk_entities.rs`
**Action**: modify

Replace fire-and-forget `.detach()` calls with `ops.spawn_save()` on the map entity's `PendingStoreOps`:
```rust
let Ok((store, mut ops)) = store_query.get_mut(chunk_ref.map_entity) else { continue };
ops.spawn_save(&store.0, chunk_pos, spawns);
```

Both systems already query per map entity — they have `chunk_ref.map_entity`.

#### 6. Migrate `save_all_chunk_entities_on_exit`
**File**: `crates/server/src/chunk_entities.rs`
**Action**: modify

```rust
pub fn save_all_chunk_entities_on_exit(
    // ...existing params...
    mut store_query: Query<(
        &StoreBackend<IVec3, Vec<WorldObjectSpawn>, FsChunkEntitiesStore>,
        &mut PendingStoreOps<IVec3, Vec<WorldObjectSpawn>>,
    )>,
) {
    // ...detect AppExit, collect by_chunk grouped by map_entity...
    for ((map_entity, chunk_pos), spawns) in by_chunk {
        let Ok((store, mut ops)) = store_query.get_mut(map_entity) else { continue };
        ops.spawn_save(&store.0, chunk_pos, spawns);
    }
    for (_, mut ops) in &mut store_query {
        ops.flush();
    }
}
```

#### 7. Tasks in `generation.rs` use store directly
**File**: `crates/voxel_map_engine/src/generation.rs`
**Action**: modify

Add `entity_store: Option<FsChunkEntitiesStore>` parameter to `spawn_terrain_batch` and `spawn_features_task`. Store is cloned from the Component and moved into the async task:
```rust
let entity_spawns = if let Some(ref store) = entity_store {
    match store.load(&pos) {
        Ok(Some(spawns)) => spawns,
        Ok(None) => vec![],
        Err(e) => { bevy::log::warn!("..."); vec![] }
    }
} else { vec![] };
```

Callers in `lifecycle.rs` query `Option<&StoreBackend<IVec3, Vec<WorldObjectSpawn>, FsChunkEntitiesStore>>` on the map entity. `Option` — clients have no store components.

#### 8. Add poll system
**File**: `crates/server/src/chunk_entities.rs` or `crates/server/src/map.rs`
**Action**: modify

Add a system that calls `ops.poll()` each frame for chunk entity ops:
```rust
fn poll_chunk_entity_ops(
    mut query: Query<&mut PendingStoreOps<IVec3, Vec<WorldObjectSpawn>>>,
) {
    for mut ops in &mut query { ops.poll(); }
}
```

Save errors are logged inside `poll()`. No separate error observer needed.

### Verification
#### Automated
- [ ] `cargo check-all` passes

#### Manual
- [ ] `cargo server` — place object, unload chunk (walk away), reload (walk back), confirm persisted
- [ ] No persistence errors in logs on happy path
- [ ] Shutdown and restart: all chunk entities preserved

---

## Phase 4: Chunk Migration + Task Pipeline Refactor

### Changes

#### 1. Filesystem backend: `crates/voxel_map_engine/src/persistence/fs_chunk.rs`
**File**: `crates/voxel_map_engine/src/persistence/fs_chunk.rs`
**Action**: create

```rust
#[derive(Clone)]
pub struct FsChunkStore {
    pub map_dir: Arc<PathBuf>,
}

impl Store<IVec3, ChunkFileEnvelope> for FsChunkStore {
    fn save(&self, key: &IVec3, value: &ChunkFileEnvelope) -> Result<(), PersistenceError> {
        let path = chunk_file_path(&self.map_dir, *key);
        // zstd + bincode serialize envelope, atomic tmp+rename
    }

    fn load(&self, key: &IVec3) -> Result<Option<ChunkFileEnvelope>, PersistenceError> {
        let path = chunk_file_path(&self.map_dir, *key);
        // read, zstd decode, bincode deserialize
        // Version check (PersistenceError::VersionMismatch on mismatch)
        // chunk_size validation is the consumer's job
    }
}
```

**Make `ChunkFileEnvelope` public** and export from `voxel_map_engine::persistence`.

#### 2. Insert components at map spawn
**File**: `crates/server/src/map.rs`
**Action**: modify

In `init_overworld_entity`, add:
```rust
StoreBackend::new(FsChunkStore { map_dir: map_dir.clone() }),
PendingStoreOps::<IVec3, ChunkFileEnvelope>::default(),
```

#### 3. Migrate `spawn_terrain_batch` — chunk load via store
**File**: `crates/voxel_map_engine/src/generation.rs`
**Action**: modify

Add `chunk_store: Option<FsChunkStore>` parameter. Inside the async task:
```rust
match chunk_store.load(&pos) {
    Ok(Some(envelope)) => {
        let chunk_data = envelope.data;
        /* existing disk-load path */
    }
    Ok(None) => { /* generate_terrain */ }
    Err(e) => { bevy::log::warn!("..."); /* generate_terrain */ }
}
```

`from_disk: true` flag MUST be preserved on the loaded path.

Remove `save_dir: Option<PathBuf>` and `chunk_size: u32` parameters.

#### 4. Migrate `drain_pending_saves` — chunk save via component
**File**: `crates/voxel_map_engine/src/lifecycle.rs`
**Action**: modify

Replace task-spawning loop with `spawn_save()` on the map entity's `PendingStoreOps`:

```rust
while !pending.queue.is_empty() && spawned < MAX_SAVE_SPAWNS_PER_FRAME {
    let save = pending.queue.pop_front().unwrap();
    chunk_ops.spawn_save(&chunk_store.0, save.position, save.envelope);
    spawned += 1;
}
```

`PendingSaves` simplified to queue-only:
```rust
struct PendingSave { position: IVec3, envelope: ChunkFileEnvelope }
```

Remove `tasks` field, `MAX_PENDING_SAVE_TASKS` cap, `save_tasks_in_flight` plot.

#### 5. Migrate `save_dirty_chunks_sync` → flush
**File**: `crates/server/src/map.rs`
**Action**: modify

```rust
pub fn save_dirty_chunks_flush(
    instance: &mut VoxelMapInstance,
    chunk_store: &StoreBackend<IVec3, ChunkFileEnvelope, FsChunkStore>,
    chunk_ops: &mut PendingStoreOps<IVec3, ChunkFileEnvelope>,
) {
    let dirty: Vec<IVec3> = instance.dirty_chunks.drain().collect();
    for chunk_pos in dirty {
        if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
            let envelope = ChunkFileEnvelope {
                version: CHUNK_SAVE_VERSION,
                chunk_size: instance.chunk_size,
                data: chunk_data.clone(),
            };
            chunk_ops.spawn_save(&chunk_store.0, chunk_pos, envelope);
        }
    }
    chunk_ops.flush();
}
```

`save_world_on_shutdown` calls this per map entity.

#### 6. Update callers of `spawn_terrain_batch`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`
**Action**: modify

`update_chunks` extends its map entity query:
```rust
Option<&StoreBackend<IVec3, ChunkFileEnvelope, FsChunkStore>>,
Option<&StoreBackend<IVec3, Vec<WorldObjectSpawn>, FsChunkEntitiesStore>>,
```

Clone stores from Components, pass to `spawn_terrain_batch` / `spawn_features_task`. `Option` — clients have no store components.

#### 7. Add poll system + generate-once invariant check
**File**: `crates/voxel_map_engine/src/lifecycle.rs` or `crates/server/src/map.rs`
**Action**: modify

Poll system for chunk ops (same pattern as Phase 3 §8). Verify `handle_completed_chunk` still checks `result.from_disk` and skips `dirty_chunks.insert`.

### Verification
#### Automated
- [ ] `cargo check-all` passes

#### Manual
- [ ] `cargo server` — delete world dir, start, walk around, observe "chunk generated" vs "chunk loaded" in logs
- [ ] Stop, restart, same area: all "chunk loaded", zero "chunk generated", zero "chunk saved"
- [ ] FPS and chunk-streaming latency subjectively unchanged

---

## Phase 5: Cleanup & Dead Code Removal

### Changes

#### 1. Delete free functions from `voxel_map_engine/src/persistence/mod.rs`
Delete: `save_chunk`, `load_chunk`, `save_chunk_entities`, `load_chunk_entities`, `delete_chunk`, `list_saved_chunks`.

Keep: `chunk_file_path`, `entity_file_path`, `parse_chunk_filename`, `ChunkFileEnvelope`, `EntityFileEnvelope`, constants.

#### 2. Inline backend implementations
Move deleted function bodies into `Store::save`/`Store::load` impls in `fs_chunk.rs`, `fs_chunk_entities.rs`. Adjust error types to `PersistenceError` directly.

#### 3. Delete free functions from `server/src/persistence/mod.rs`
Delete: `save_map_meta`, `load_map_meta`, `save_entities`, `load_entities`.

Keep: `MapMeta`, `WorldSavePath`, `map_save_dir`, `EntityFileEnvelope`, constants.

#### 4. Inline server backend implementations
Same as §2 for `fs_map_meta.rs`, `fs_map_entities.rs`.

#### 5. Delete `save_dirty_chunks_sync`
Replaced by `save_dirty_chunks_flush` in Phase 4.

#### 6. Update README.md
If persistence section mentions removed functions.

### Verification
#### Automated
- [ ] `cargo check-all` passes with no dead-code warnings
- [ ] `cargo test --workspace` passes

#### Manual
- [ ] Full repeat of Phase 4 verification
- [ ] Shutdown save: dirty world → Ctrl+C → restart → data intact
- [ ] `grep -rn "persistence::(save_chunk|load_chunk|save_map_meta|load_map_meta|save_entities|load_entities|save_chunk_entities|load_chunk_entities)" crates/` — zero hits outside `fs_*.rs` backend impls
