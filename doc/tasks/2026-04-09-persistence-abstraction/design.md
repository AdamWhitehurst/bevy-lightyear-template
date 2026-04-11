# Design Discussion

## Current State

Persistence logic is split across two modules with no shared interface:

- `voxel_map_engine/src/persistence.rs` — chunks + per-chunk entities, bincode + zstd
- `server/src/persistence.rs` — map meta + map entities, bincode only

All ten functions are synchronous `std::fs` free functions (research.md §Q1). IO executes in two contexts:

- **Task pool**: chunk reads/writes via `spawn_terrain_batch` (`generation.rs:57,70`), `spawn_features_task` (`generation.rs:122`), `drain_pending_saves` (`lifecycle.rs:570-578`), and fire-and-forget entity saves (`chunk_entities.rs:83-89`).
- **Main thread**: map meta + map entities in `spawn_overworld`, `save_dirty_chunks_debounced`, `save_world_on_shutdown`, `load_startup_entities` (`map.rs:104,250,255,281,387`).

Not-found returns are inconsistent: chunk/meta use `Ok(None)`, map entities use `Ok(Vec::new())` (research.md §Q3). The "generate once, save forever" invariant depends on `Ok(None)` → generate → mark dirty → save; disk-loaded chunks skip the dirty flag (`lifecycle.rs:892-894`).

Zero `EventWriter`/`EventReader` usage in project crates — existing cross-system communication uses observers, lightyear messages, or direct calls.

## Desired End State

A new `persistence` crate hosts a generic async `Store<K, V>` trait plus **generic** event types (`SaveRequest<K, V>`, `LoadRequest<K>`, `LoadedEvent<K, V>`, `NotFoundEvent<K>`, `PersistenceError<K>`) and a `StorePlugin<K, V>` that wires them together. The filesystem backend implements `Store<K, V>` for the four active data types. Consumer crates register one plugin instantiation per data type (e.g. `StorePlugin::<IVec3, ChunkData>::new(fs_backend)`) and send/read the generic events parameterized with their own types. The `persistence` crate itself never names `ChunkData`, `MapMeta`, or any concrete type — coupling lives at the instantiation site, not in trait or event definitions. Callers match on `LoadedEvent` vs. `NotFoundEvent` to preserve the generate-once invariant.

**Verification**: `cargo server` preserves current overworld save/load. Chunks save to disk and reload on next run. Map meta persists the seed. Homebase save paths still resolve correctly. No regression in the generate-once invariant (a loaded chunk is not re-saved).

## Patterns to Follow

**Follow**:
- Trait + `Arc<dyn Trait>` type erasure from `VoxelGeneratorImpl` (`voxel_map_engine/src/config.rs:13-26,51`) and its task-local `Arc::clone` into `pool.spawn(async move { ... })` (`generation.rs:48`).
- `AsyncComputeTaskPool::get().spawn` + `PendingX` component + `check_ready` polling pattern (`generation.rs:42-101`; `lifecycle.rs:824-856`) to drive async work from the ECS. The index-walk + `swap_remove` loop in `drain_pending_saves` (`lifecycle.rs:552-593`) is the reference.
- Atomic write (tmp + `fs::rename`) inside the filesystem backend (`voxel_map_engine/src/persistence.rs:41,50`).
- Versioned envelope structs (`ChunkFileEnvelope`, `EntityFileEnvelope`, `MapMeta`) — per-data-type, owned by the backend, **not** in the trait.

**Don't follow**:
- `Option<Res<T>>` for the backend — install as required resource gated on `AppState::Ready` (per CLAUDE.md and MEMORY.md).
- Mixed main-thread / task-pool execution — all IO routes through the event → task pipeline.
- `Ok(Vec::new())` vs. `Ok(None)` — unify on typed not-found.
- Silent `.detach()` for writes (`chunk_entities.rs:83-89`) — errors surface via a `StoreError<K>` event (fire-and-forget remains the default, but errors are observable).

## Design Decisions

1. **New `persistence` crate** — pure trait + error type + composition helpers. Zero dependency on `voxel_map_engine` or `protocol` (it is type-parametric). Added as a workspace dep of `voxel_map_engine` (needs the filesystem chunk backend) and `server`.

2. **Generic async trait**:
   ```rust
   #[async_trait]
   pub trait Store<K, V>: Send + Sync {
       async fn save(&self, key: &K, value: &V) -> Result<(), PersistenceError>;
       async fn load(&self, key: &K) -> Result<Option<V>, PersistenceError>;
   }
   ```
   Each of the four data types gets its own `(K, V)` pair. Serialization and versioning live entirely in the backend impl. `async_trait` for dyn-compatibility — native AFIT can't be object-safe.

3. **Decorator composition for caching and routing**:
   - `CachingStore<S>` wraps any `S: Store<K, V>` (K: Clone+Eq+Hash, V: Clone). Read checks cache, falls through, populates. Write updates both.
   - `RoutingStore` deferred — only one backend exists today; defer until the nostr backend lands.

4. **Generic events + `StorePlugin<K, V>`** — the crucial decoupling piece. `persistence` provides:
   ```rust
   #[derive(Event)] pub struct SaveRequest<K, V>  { pub key: K, pub value: Arc<V> }
   #[derive(Event)] pub struct LoadRequest<K>     { pub key: K }
   #[derive(Event)] pub struct LoadedEvent<K, V>  { pub key: K, pub value: Arc<V> }
   #[derive(Event)] pub struct NotFoundEvent<K>   { pub key: K }
   #[derive(Event)] pub struct StoreError<K>      { pub key: K, pub error: PersistenceError }

   pub struct StorePlugin<K, V> { backend: Arc<dyn Store<K, V>>, _marker: … }
   ```
   Each generic instantiation becomes a distinct Bevy event type — `LoadRequest<IVec3>` and `LoadRequest<MapInstanceId>` never collide. Values ride in events as `Arc<V>` so `ChunkData` (~6KB) is not cloned through the event bus. Correlation is by key (chunk pos / map id are unique), so no separate requester token.

5. **Generic resource and systems owned by `persistence`**: `StorePlugin::<K,V>::build` adds a `StoreResource<K,V>(Arc<dyn Store<K,V>>)`, a `PendingStoreTasks<K,V>` resource holding `Vec<Task<StoreOp<K,V>>>`, a save-handler and load-handler system, and a poll system that drains completed tasks and emits the result events. All systems are generic functions — no per-data-type code anywhere except the plugin instantiation call.

6. **Consumer setup (server)**:
   ```rust
   app.add_plugins((
       StorePlugin::<IVec3, ChunkData>::new(fs_chunks),
       StorePlugin::<IVec3, Vec<WorldObjectSpawn>>::new(fs_chunk_entities),
       StorePlugin::<MapInstanceId, MapMeta>::new(fs_map_meta),
       StorePlugin::<MapInstanceId, Vec<SavedEntity>>::new(fs_map_entities),
   ));
   ```
   Existing systems replace direct `save_chunk(...)` / `load_chunk(...)` calls with `EventWriter<SaveRequest<IVec3, ChunkData>>` / `EventReader<LoadedEvent<IVec3, ChunkData>>`.

7. **Typed not-found**: the trait returns `Result<Option<V>, PersistenceError>`. The poll system collapses `Ok(None)` into `NotFoundEvent`, `Ok(Some(v))` into `LoadedEvent`, and `Err(_)` into `StoreError`. Consumers match on three distinct variants, not `Option<V>`.

8. **Scope — 8 active operations only**: chunks save/load, chunk entities save/load, map meta save/load, map entities save/load. `delete_chunk` (dead) and `list_saved_chunks` (test-only) are not abstracted. Task.md mentions `delete`/`list` events; deferred since no production consumer exists.

## What We're NOT Doing

- Nostr backend implementation — separate task.
- `delete` / `list` events and trait methods — revisit when a production consumer appears.
- `RoutingStore` for multi-backend dispatch — single-backend startup install suffices until backend #2 lands.
- Unifying zstd vs. plain-bincode across data types — the serialization difference stays inside each backend impl.
- Reworking the `spawn_terrain_batch` → `spawn_features_task` → mesh pipeline structure — only the IO calls inside those tasks change (they now send/receive events instead of calling `load_chunk` / `save_chunk` inline).
- Migrating `generation_version` to invalidate saved chunks — pre-existing gap, orthogonal.
- Moving `MapMeta` / `WorldSavePath` out of `server/src/persistence.rs` — types stay where they are, only the IO functions are replaced.

## Open Risks

1. **Event/task latency for startup loads**: `load_map_meta` currently runs synchronously in `spawn_overworld` (`map.rs:104`). An event → task → event round-trip adds ≥1 frame. Acceptable if we gate `AppState::Ready` on the loaded event; unacceptable if systems assume `MapMeta` is ready during the same frame. Mitigation: block startup on a `PendingStartupLoads` component with an explicit readiness check, same pattern as `TrackedAssets`.

2. **Task pipeline refactor in `spawn_terrain_batch`**: the current task performs up to 2 sequential sync reads per chunk, batched 8 chunks per task (16 reads). Converting to async-event-driven means a chunk batch becomes multiple in-flight load requests with fan-in before feature generation begins. This is a non-trivial restructure of `generation.rs:42-101`. Contain in a dedicated implementation slice.

3. **Async runtime compatibility for future nostr backend**: Bevy's task pool is `async-executor`-based, while `nostr-sdk` is tokio-based. A bridge (dedicated tokio runtime + channels) will be needed in the nostr backend. Out of scope here but noted because it validates the async trait choice.

4. **`async_trait` boxing cost**: one `Box::pin` allocation per call. Negligible for IO-bound persistence; flagged only for completeness.

5. **Generate-once invariant preservation**: the `from_disk: bool` flag on `ChunkGenResult` (`generation.rs:65`) must keep threading through the new event-driven path. Any test regression here silently re-saves every loaded chunk — include an explicit check in the implementation slice for chunk load.

6. **Fire-and-forget error path decision**: entity saves via `.detach()` currently lose errors. The event-driven design emits `StoreError<K>` events but no consumer exists yet. Decision deferred: emit the event, log a `warn!`, leave consumer wiring for a follow-up.
