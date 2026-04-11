# Research Findings

## Q1: How do current persistence consumers invoke storage operations?

### Findings

**The storage layer** (`voxel_map_engine/src/persistence.rs`) exposes six synchronous free functions using `std::fs` blocking IO. None are async.

| Function | Signature (abbreviated) | IO |
|---|---|---|
| `save_chunk` | `(map_dir, chunk_pos, &ChunkData) -> Result<(), String>` | write+rename |
| `load_chunk` | `(map_dir, chunk_pos) -> Result<Option<ChunkData>, String>` | read |
| `save_chunk_entities` | `(map_dir, chunk_pos, &[WorldObjectSpawn]) -> Result<(), String>` | write+rename |
| `load_chunk_entities` | `(map_dir, chunk_pos) -> Result<Option<Vec<WorldObjectSpawn>>, String>` | read |
| `delete_chunk` | `(map_dir, chunk_pos) -> Result<(), String>` | remove_file |
| `list_saved_chunks` | `(map_dir) -> Result<Vec<IVec3>, String>` | read_dir |

**`server/src/persistence.rs`** exposes four more synchronous free functions:

| Function | Signature (abbreviated) | IO |
|---|---|---|
| `save_map_meta` | `(map_dir, &MapMeta) -> Result<(), String>` | write+rename |
| `load_map_meta` | `(map_dir) -> Result<Option<MapMeta>, String>` | read |
| `save_entities` | `(map_dir, &[SavedEntity]) -> Result<(), String>` | write+rename |
| `load_entities` | `(map_dir) -> Result<Vec<SavedEntity>, String>` | read |

### Call site invocation modes

| Call site | Function(s) called | Where executed |
|---|---|---|
| `generation.rs:57,70` via `spawn_terrain_batch` | `load_chunk`, `load_chunk_entities` | Inside `AsyncComputeTaskPool` task |
| `generation.rs:122` via `spawn_features_task` | `load_chunk_entities` | Inside `AsyncComputeTaskPool` task |
| `lifecycle.rs:570-578` via `drain_pending_saves` | `save_chunk` | Inside `AsyncComputeTaskPool` task (up to 32 concurrent, 16 spawns/frame) |
| `chunk_entities.rs:40,83-89` via `save_new_chunk_entities` | `save_chunk_entities` | Fire-and-forget `.detach()` async task |
| `chunk_entities.rs:134-139` via `evict_chunk_entities` | `save_chunk_entities` | Fire-and-forget `.detach()` async task |
| `chunk_entities.rs:178-181` via `save_all_chunk_entities_on_exit` | `save_chunk_entities` | **Main thread**, synchronous (shutdown) |
| `map.rs:104` via `spawn_overworld` | `load_map_meta` | **Main thread**, synchronous (startup) |
| `map.rs:250` via `save_dirty_chunks_debounced` | `save_map_meta` | **Main thread**, synchronous |
| `map.rs:255,374` via `save_dirty_chunks_debounced` | `save_entities` | **Main thread**, synchronous |
| `map.rs:281,328,333` via `save_world_on_shutdown` | `save_chunk`, `save_map_meta`, `save_entities` | **Main thread**, synchronous (shutdown) |
| `map.rs:387` via `load_startup_entities` | `load_entities` | **Main thread**, synchronous (startup) |

---

## Q2: What data types are persisted, serialization formats, filesystem assumptions?

### Findings

**Chunk voxels** — `ChunkFileEnvelope { version: u32, data: ChunkData }` (`voxel_map_engine/src/persistence.rs:14-18`)
- Format: bincode + zstd level 3 (`persistence.rs:39,43-48`)
- Path: `{map_dir}/terrain/chunk_{x}_{y}_{z}.bin` (`persistence.rs:22-27`)
- Version: `CHUNK_SAVE_VERSION = 3` (`persistence.rs:11`); mismatch returns `Err` (`persistence.rs:71-76`)
- Atomicity: write to `.bin.tmp`, then `fs::rename` (`persistence.rs:41,50`)
- Dir creation: `fs::create_dir_all` on `terrain/` subdir (`persistence.rs:32-33`)

**Per-chunk entity spawns** — `EntityFileEnvelope { version: u32, spawns: Vec<WorldObjectSpawn> }` (`voxel_map_engine/src/persistence.rs:122-126`)
- Format: bincode + zstd level 3 (`persistence.rs:152,156-161`)
- Path: `{map_dir}/entities/chunk_{x}_{y}_{z}.entities.bin` (`persistence.rs:131-136`)
- Version: `ENTITY_SAVE_VERSION = 1` (`persistence.rs:121`)
- Atomicity: same tmp+rename pattern (`persistence.rs:154,163`)
- Dir creation: `fs::create_dir_all` on `entities/` subdir (`persistence.rs:145-146`)

**Map metadata** — `MapMeta { version: u32, seed: u64, generation_version: u32, spawn_points: Vec<Vec3> }` (`server/src/persistence.rs:12-18`)
- Format: bincode only, no compression (`persistence.rs:42`)
- Path: `{map_dir}/map.meta.bin` (`persistence.rs:41`)
- Version: `META_VERSION = 1` (`persistence.rs:9`)
- Atomicity: same tmp+rename pattern (`persistence.rs:43-45`)

**Map entities** — `EntityFileEnvelope { version: u32, entities: Vec<SavedEntity> }` (`server/src/persistence.rs:68-72`)
- Types defined in `protocol/src/map/persistence.rs:9-19`: `SavedEntity { kind: SavedEntityKind, position: Vec3 }`
- Format: bincode only, no compression (`persistence.rs:84`)
- Path: `{map_dir}/entities.bin` (`persistence.rs:79`)
- Version: `ENTITY_SAVE_VERSION = 1` (`persistence.rs:67`)
- Atomicity: same tmp+rename pattern (`persistence.rs:85-87`)

**Path construction**: `WorldSavePath` resource defaults to `"worlds"` (`server/src/persistence.rs:24-27`). `map_save_dir` resolves `Overworld` to `{base}/overworld`, `Homebase { owner }` to `{base}/homebase-{owner}` (`server/src/persistence.rs:31-36`). The resulting `PathBuf` propagates to `VoxelMapConfig::save_dir` (`server/src/map.rs:110`).

**Directory listing**: `list_saved_chunks` uses `fs::read_dir` on `{map_dir}/terrain/`, parsing filenames via `parse_chunk_filename` with `rfind('_')` to handle negative coords (`persistence.rs:91-119`). Returns empty `Vec` if directory missing (`persistence.rs:93-95`).

---

## Q3: "Not found" case handling and generate-once invariant

### Findings

| Operation | Not-found return | Caller behavior |
|---|---|---|
| `load_chunk` | `Ok(None)` (`persistence.rs:57-58`) | Falls through to `generate_terrain()` (`generation.rs:95`) |
| `load_chunk_entities` (terrain path) | `Ok(None)` (`persistence.rs:173-174`) | Empty spawns vec (`generation.rs:74`) |
| `load_chunk_entities` (features path) | `Ok(None)` | Calls `generator.place_features()` (`generation.rs:126`) |
| `load_map_meta` | `Ok(None)` (`server/persistence.rs:52-53`) | Falls back to hardcoded defaults (`map.rs:104-107`) |
| `load_entities` | `Ok(Vec::new())` (`server/persistence.rs:92-96`) | Empty iteration, nothing spawned |

`Err` cases: `load_chunk` and `load_chunk_entities` `Err` cases also fall through to generation with a `warn!` log. `load_map_meta` uses a wildcard `_` arm catching both `None` and `Err`.

**Generate-once invariant** enforced at two levels:

1. **Terrain**: disk-loaded chunks get `from_disk: true` in `ChunkGenResult` (`generation.rs:65`). `handle_completed_chunk` skips `dirty_chunks.insert` for `from_disk` chunks (`lifecycle.rs:892-894`), so they are never re-saved. Generated chunks get `from_disk: false`, are marked dirty, and saved by `drain_pending_saves`. On next load, they return `Ok(Some(...))`.

2. **Per-chunk entities**: `spawn_features_task` generates entities only on `Ok(None)` (`generation.rs:126`). `spawn_chunk_entities` immediately fires a `.detach()` save task for generated spawns (`chunk_entities.rs:40,83-89`). On next load, `load_chunk_entities` returns `Ok(Some(...))`, skipping generation.

No generation-version invalidation exists — old-version chunks loaded from disk are used as-is.

---

## Q4: Bevy event/observer/message patterns in the codebase

### Findings

**Observers (`On<Add/Remove, T>`)** — 12 observers registered across crates:

| Observer | Component | File |
|---|---|---|
| `handle_connected` | `On<Add, Connected>` | `server/src/gameplay.rs:234` |
| `on_connected` | `On<Add, Connected>` | `client/src/network.rs:129` |
| `on_client_connected` | `On<Add, Connected>` | `ui/src/lib.rs:132` |
| `on_client_disconnected` | `On<Add, Disconnected>` | `ui/src/lib.rs:120` |
| `on_disconnected` | `On<Add, Disconnected>` | `client/src/network.rs:133` |
| `on_map_instance_id_added` | `On<Add, MapInstanceId>` | `server/src/map.rs:414` |
| `add_health_bars` | `On<Add, Health>` | `render/src/lib.rs:83` |
| `on_invulnerable_added` | `On<Add, Invulnerable>` | `render/src/health_bar.rs:93` |
| `on_invulnerable_removed` | `On<Remove, Invulnerable>` | `render/src/health_bar.rs:108` |
| `add_visual_interpolation` | `On<Add, Position>` | `render/src/lib.rs:62` |
| `on_respawn_timer_added` | `On<Add, RespawnTimer>` | `client/src/gameplay.rs:105` |
| `on_respawn_timer_removed` | `On<Remove, RespawnTimer>` | `client/src/gameplay.rs:118` |
| `despawn_ability_projectile_spawn` | `On<Remove, AbilityBulletOf>` | `protocol/src/ability/spawn.rs:280` |
| `cleanup_effect_markers_on_removal` | `On<Remove, ActiveAbility>` | `protocol/src/ability/lifecycle.rs:26` |

**Lightyear messages** — component-based `MessageSender<T>` / `MessageReceiver<T>`. Key types: `VoxelEditRequest`, `VoxelEditAck`, `VoxelEditReject`, `VoxelEditBroadcast`, `SectionBlocksUpdate`, `ChunkDataSync`, `UnloadColumn`, `PlayerMapSwitchRequest`, `MapTransitionStart/Ready/End`. Broadcasting via `ServerMultiMessageSender::send_to_entities` (`server/src/map.rs:656,668,842`).

**EventWriter/EventReader** — zero uses of Bevy's `EventWriter`/`EventReader` in project crates. `MessageReader<AppExit>` used for shutdown detection (`server/src/map.rs:290`, `chunk_entities.rs:155`). `MessageReader<AssetEvent<T>>` used for hot-reload in asset loaders.

**Debounced saves** (`server/src/map.rs:198-259`):
- `WorldDirtyState` resource tracks `is_dirty`, `last_edit_time`, `first_dirty_time` as `f64` timestamps
- Triggers save when idle >= 1s (`SAVE_DEBOUNCE_SECONDS`) OR dirty >= 5s (`MAX_DIRTY_SECONDS`) (`map.rs:61-62`)
- Dirtied by `apply_voxel_edit` (`map.rs:518-536`)
- No `Timer` component — pure `Time::elapsed_secs_f64()` comparison

**`commands.trigger`** — used for lightyear room events (`RoomEvent`/`RoomTarget`) in `gameplay.rs:296`, `map.rs:426,957-971`, and transport lifecycle (`Connect`/`Disconnect`/`Start`) in `network.rs`, `ui/src/lib.rs`.

---

## Q5: Async task pipeline and persistence interaction

### Findings

**Pipeline structure**: Three-phase: `spawn_terrain_batch` -> `spawn_features_task` -> mesh generation. Each phase dispatches to `AsyncComputeTaskPool`, results polled by `poll_chunk_tasks` (`lifecycle.rs:791-866`).

**IO inside tasks**: All persistence reads happen inside task pool threads via synchronous `std::fs` calls:
- `spawn_terrain_batch` (`generation.rs:57,70`): up to 2 sequential reads per chunk (terrain + entities), batched 8 chunks per task — up to 16 blocking reads per task
- `spawn_features_task` (`generation.rs:122`): 1 read per task
- `drain_pending_saves` (`lifecycle.rs:570-578`): 1 write per task, capped at 32 concurrent / 16 spawns per frame

**Result return path**: `poll_chunk_tasks` calls `bevy::tasks::futures::check_ready` (non-blocking poll) on each task each frame, capped at `MAX_GEN_POLLS_PER_FRAME = 256`. Completed results flow to `handle_completed_chunk` which updates ECS synchronously on main thread (`lifecycle.rs:877`).

**Non-filesystem backend constraints**:
- All IO is synchronous blocking — a network round-trip would hold a task pool thread for the entire duration. With `MAX_PENDING_GEN_TASKS = 512` possible in-flight tasks and a fixed thread pool (CPU core count), threads could saturate on latency.
- `spawn_terrain_batch` serializes up to 16 blocking reads within a single task — no suspension points to yield the thread.
- Atomic write relies on `tmp + fs::rename` — no filesystem-level equivalent for network backends.
- `list_saved_chunks` uses `fs::read_dir` for chunk enumeration — requires a different discovery mechanism for non-filesystem backends.
- Fire-and-forget `.detach()` tasks for entity saves (`chunk_entities.rs:83-89`) have no error propagation path — a network failure would be silently lost.

---

## Q6: Persistence type locations and crate dependency edges

### Findings

**Type locations**:

| Crate | File | Types/Functions |
|---|---|---|
| `protocol` | `src/map/persistence.rs` | `MapSaveTarget`, `SavedEntityKind`, `SavedEntity` |
| `server` | `src/persistence.rs` | `MapMeta`, `WorldSavePath`, `map_save_dir`, `save_map_meta`, `load_map_meta`, `save_entities`, `load_entities` |
| `voxel_map_engine` | `src/persistence.rs` | `save_chunk`, `load_chunk`, `delete_chunk`, `list_saved_chunks`, `save_chunk_entities`, `load_chunk_entities`, `chunk_file_path`, `entity_file_path`, `parse_chunk_filename` |

**Crate dependency graph** (workspace-internal):
```
voxel_map_engine  -> (no workspace deps)
protocol          -> voxel_map_engine
sprite_rig        -> protocol
render            -> protocol, sprite_rig
ui                -> protocol
client            -> protocol, voxel_map_engine, render, ui
server            -> protocol, voxel_map_engine
web               -> protocol, client, render, ui
```

**Trait hosting options** (based on existing edges):
- **`voxel_map_engine`**: leaf crate, reachable from `protocol`, `server`, `client`. Cannot reference `protocol` types (`MapInstanceId`, `SavedEntity`) without adding a new `voxel_map_engine -> protocol` edge (currently the arrow goes the other direction).
- **`protocol`**: depends on `voxel_map_engine`, consumed by all other crates. A trait here could reference both `voxel_map_engine` types (`ChunkData`, `WorldObjectSpawn`) and its own types (`SavedEntity`, `MapInstanceId`). No new edges required.
- **New crate**: a `persistence` crate below `voxel_map_engine` could host pure trait definitions without any type dependencies, but would require adding it as a dep to both `voxel_map_engine` and `server`.

---

## Cross-Cutting Observations

- **Uniform serialization pattern**: all four data types use bincode serialization with a versioned envelope struct. Chunk data types additionally apply zstd compression; map-level types do not.
- **Uniform atomicity pattern**: all write paths use `write-to-tmp + fs::rename`. No data type uses a different write strategy.
- **Two persistence modules, no shared trait**: `voxel_map_engine/src/persistence.rs` and `server/src/persistence.rs` follow the same pattern (free functions, `Result<Option<T>, String>` returns, version checks, atomic writes) but share no common interface.
- **Inconsistent not-found returns**: chunk operations return `Ok(None)`, map entities returns `Ok(Vec::new())`. Both are "nothing here" but typed differently.
- **Mixed execution contexts**: reads are always in task pool threads; writes are mixed (task pool for chunks, main thread for map meta/entities during debounced save and shutdown).
- **No EventWriter/EventReader in project crates**: all cross-system communication uses either lightyear messages, observers, or direct function calls.

## Open Areas

- Whether `delete_chunk` (`voxel_map_engine/src/persistence.rs`) is called anywhere — no call sites were found in the research. It may be dead code or test-only.
- How `list_saved_chunks` is used — no call site was traced in the production code paths examined. It exists in the public API but its consumers (if any) were not identified.
- The interaction between `generation_version` in `MapMeta` and chunk invalidation — the field is stored but never checked against saved chunks, leaving no mechanism for forced regeneration.
