# Structure Outline

## Approach

Create a new `persistence` crate hosting a generic `Store<K, V>` trait, generic
event types, and a generic `StorePlugin<K, V>`. Migrate one data type per
phase, simplest first (map meta → map entities → chunk entities → chunks), so
the event/task infrastructure is proven end-to-end before the chunk pipeline
refactor. Each migration deletes its consumer's direct `std::fs` call path and
replaces it with event sends/reads. Old free functions stay compiling until
Phase 5 removes them.

---

## Phase 1: Persistence Crate + Map Meta Migration

Stand up the crate with trait, events, plugin, poll/dispatch systems, and
prove it end-to-end by migrating the simplest data type (`MapMeta`: 1 load
site, 2 save sites, no task pipeline interaction).

**Files**:
- NEW `crates/persistence/Cargo.toml`, `crates/persistence/src/lib.rs`
- NEW `crates/persistence/src/store.rs` — trait, error, events
- NEW `crates/persistence/src/plugin.rs` — `StorePlugin`, systems
- NEW `crates/server/src/persistence/fs_map_meta.rs` — `FsMapMetaStore`
- MOD `Cargo.toml` (workspace member), `crates/server/Cargo.toml`
- MOD `crates/server/src/persistence.rs` → `persistence/mod.rs`; keep
  `MapMeta`, `WorldSavePath`, `map_save_dir`, envelope types
- MOD `crates/server/src/map.rs` — replace `load_map_meta`/`save_map_meta`
  calls with `EventWriter<LoadRequest<MapInstanceId>>`/`SaveRequest`
- MOD `crates/server/src/lib.rs` — `add_plugins(StorePlugin::<MapInstanceId, MapMeta>::new(...))`

**Key changes**:
- `trait Store<K, V>: Send + Sync` with `async fn save(&self, &K, &V) -> Result<(), PersistenceError>` and `async fn load(&self, &K) -> Result<Option<V>, PersistenceError>` (via `async_trait`)
- `enum PersistenceError { Io, Serialize, Deserialize, VersionMismatch { expected, actual } }`
- `SaveRequest<K, V> { key: K, value: Arc<V> }`, `LoadRequest<K> { key: K }`, `LoadedEvent<K, V> { key: K, value: Arc<V> }`, `NotFoundEvent<K> { key: K }`, `StoreError<K> { key: K, error: PersistenceError }`
- `StoreBackend<K, V>(Arc<dyn Store<K, V>>)` resource; `PendingStoreOps<K, V>` resource holding `Vec<Task<…>>`
- `StorePlugin<K, V>::new(backend)` adds the resource, three generic systems (`handle_save_requests`, `handle_load_requests`, `poll_store_ops`), and registers all five event types
- `FsMapMetaStore { base: PathBuf }` implements `Store<MapInstanceId, MapMeta>`; preserves existing bincode format and tmp+rename atomicity
- `spawn_overworld` gates on a `PendingStartupLoads` marker cleared when the first `LoadedEvent`/`NotFoundEvent<MapInstanceId>` arrives (Open Risk #1 mitigation)

**Verify**: `cargo check-all` passes. `cargo server` — (1) delete a world dir, start server, confirm fresh seed via trace log; (2) stop, restart, confirm seed matches across runs via `trace!("loaded map meta seed={}")` in the `LoadedEvent` consumer.

---

## Phase 2: Map Entities Migration

Migrate `save_entities`/`load_entities` (bincode `Vec<SavedEntity>`). Validates
the second generic `(K, V)` instantiation works alongside the first.

**Files**:
- NEW `crates/server/src/persistence/fs_map_entities.rs` — `FsMapEntitiesStore`
- MOD `crates/server/src/map.rs` — `save_dirty_chunks_debounced`, `save_world_on_shutdown`, `load_startup_entities` send/read events instead of calling the free functions
- MOD `crates/server/src/lib.rs` — register `StorePlugin::<MapInstanceId, Vec<SavedEntity>>`

**Key changes**:
- `FsMapEntitiesStore` implements `Store<MapInstanceId, Vec<SavedEntity>>`
- `NotFoundEvent` handling replaces the `Ok(Vec::new())` pattern (design §7: unify on typed not-found)
- Startup-load gating extended to wait on both `MapMeta` and `Vec<SavedEntity>` before `AppState::Ready`

**Verify**: `cargo server` — (1) place a world object via edit; (2) trigger debounced save (idle 1s); (3) restart server; (4) confirm object reappears at saved position.

---

## Phase 3: Chunk Entities Migration

Migrate `save_chunk_entities`/`load_chunk_entities`. First slice that touches
`.detach()` fire-and-forget writes — errors now flow through
`StoreError<IVec3>` with a `warn!` consumer.

**Files**:
- NEW `crates/voxel_map_engine/src/persistence/fs_chunk_entities.rs` — `FsChunkEntitiesStore`
- MOD `crates/voxel_map_engine/src/persistence.rs` → `persistence/mod.rs`; keep `EntityFileEnvelope`, `entity_file_path`, `parse_chunk_filename`
- MOD `crates/voxel_map_engine/Cargo.toml` — add `persistence` dep
- MOD `crates/server/src/chunk_entities.rs` — `save_new_chunk_entities`, `evict_chunk_entities`, `save_all_chunk_entities_on_exit` send `SaveRequest<IVec3, Vec<WorldObjectSpawn>>`
- MOD `crates/voxel_map_engine/src/generation.rs:70,122` — now the only remaining sync call (chunk-entity reads inside `spawn_terrain_batch`/`spawn_features_task`); migrated here by having the task `Arc::clone` the `StoreBackend` and `.await` load directly inside the task (Phase 4 unifies the load-request path once the pipeline is restructured)
- MOD `crates/voxel_map_engine/src/lib.rs` or `crates/server/src/lib.rs` — register `StorePlugin::<IVec3, Vec<WorldObjectSpawn>>`
- NEW `warn_on_store_errors<IVec3>` observer system in `chunk_entities.rs`

**Key changes**:
- `FsChunkEntitiesStore { map_dir: Arc<PathBuf> }` implements `Store<IVec3, Vec<WorldObjectSpawn>>`; reuses existing zstd + envelope format
- Fire-and-forget writes now go through `EventWriter<SaveRequest>` — no `.detach()`; `StoreError` events drive a `warn!` log (design §Open Risk #6)

**Note**: Tasks in `generation.rs` use `Arc<dyn Store>` directly (not events) because they execute inside the task pool. This is the "tasks use `StoreBackend` resource; systems use events" split — both are supported by the same plugin.

**Verify**: `cargo server` — (1) place an object in a chunk; (2) unload the chunk (walk away); (3) reload by walking back; (4) confirm object persisted; (5) `grep warn .*StoreError` in server logs returns zero entries on the happy path.

---

## Phase 4: Chunk Migration + Task Pipeline Refactor

Migrate `save_chunk`/`load_chunk` and restructure `spawn_terrain_batch` to
drive chunk loads through the event pipeline. This is the riskiest slice
(Open Risk #2, #5). Preserve the `from_disk: bool` flag on `ChunkGenResult`
so loaded chunks are not re-marked dirty.

**Files**:
- NEW `crates/voxel_map_engine/src/persistence/fs_chunk.rs` — `FsChunkStore`
- MOD `crates/voxel_map_engine/src/generation.rs` — split `spawn_terrain_batch` into: (a) `request_chunk_loads` system emits `LoadRequest<IVec3, ChunkData>`; (b) `on_chunk_loaded` / `on_chunk_not_found` handlers drive generate vs. reuse; (c) `spawn_features_task` unchanged in shape but now reads via events
- MOD `crates/voxel_map_engine/src/lifecycle.rs:552-593` — `drain_pending_saves` emits `SaveRequest<IVec3, ChunkData>` events; `PendingSaveTasks` is retired in favor of `PendingStoreOps<IVec3, ChunkData>` owned by the plugin
- MOD `crates/voxel_map_engine/src/lib.rs` — register `StorePlugin::<IVec3, ChunkData>`

**Key changes**:
- `FsChunkStore { map_dir: Arc<PathBuf> }` implements `Store<IVec3, ChunkData>`; reuses zstd + `ChunkFileEnvelope` format + version check
- `ChunkGenResult` gains a path where `from_disk = true` is set from the `LoadedEvent` branch, preserving the `lifecycle.rs:892-894` dirty-skip check
- `spawn_terrain_batch`'s 8-chunk batching is replaced by N in-flight `LoadRequest`s with fan-in on `LoadedEvent`/`NotFoundEvent`; the feature-generation step waits until all chunks in a terrain batch have resolved

**Verify**: `cargo server` — (1) delete world dir, start, walk around, observe trace logs for "chunk generated" vs "chunk loaded" counts; (2) stop, restart, walk the same area; expect all trace logs read "chunk loaded", zero "chunk generated", zero "chunk saved" (generate-once invariant). Manual: FPS and chunk-streaming latency subjectively unchanged.

---

## Phase 5: Cleanup & Dead Code Removal

Delete the old free-function API surface. Types stay where they are per
design §What We're NOT Doing.

**Files**:
- MOD `crates/voxel_map_engine/src/persistence/mod.rs` — delete `save_chunk`, `load_chunk`, `save_chunk_entities`, `load_chunk_entities`, `delete_chunk`, `list_saved_chunks`
- MOD `crates/server/src/persistence/mod.rs` — delete `save_map_meta`, `load_map_meta`, `save_entities`, `load_entities`
- MOD `README.md` — if persistence/save section mentions removed functions
- KEEP: `MapMeta`, `WorldSavePath`, `map_save_dir`, `MapSaveTarget`, `SavedEntity`, `SavedEntityKind`, all envelope structs

**Verify**: `cargo check-all` passes with no dead-code warnings. Full repeat of Phase 4 verification to confirm nothing regressed during cleanup.

---

## Testing Checkpoints

- **After Phase 1**: `cargo server` preserves map seed across restarts via the event pipeline. If this works, the generic `Store`/`StorePlugin` design is validated end-to-end.
- **After Phase 2**: World entities (trees, props, resources) persist across restarts via debounced save.
- **After Phase 3**: Per-chunk entity spawns persist via fire-and-forget event writes; `StoreError` surfaces as `warn!` logs instead of silent loss.
- **After Phase 4**: Full chunk save/load via the event pipeline. Generate-once invariant preserved — restarting the server never re-saves chunks loaded from disk. This is the green-light for removing the old API.
- **After Phase 5**: Zero call sites of the old free functions. Old persistence modules contain only types and helpers.

## Deferred / Not-Sliced

- **`CachingStore<S>` decorator** (design §3): no production consumer in this task. Add alongside Phase 1 as an un-wired generic helper with a unit test, or defer entirely. Prefer defer unless a test-time consumer materializes.
- **`RoutingStore`, `delete`/`list` events**: explicitly deferred per design §What We're NOT Doing.
