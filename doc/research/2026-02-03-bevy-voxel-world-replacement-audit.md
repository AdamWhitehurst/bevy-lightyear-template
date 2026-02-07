---
date: 2026-02-03T18:00:01-08:00
researcher: Claude
git_commit: 8c199a60d71eb448d2cc2503f7dcbbb890094aab
branch: master
repository: bevy-lightyear-template
topic: "What features and performance optimizations we need to re-implement to replace bevy_voxel_world with a custom-built one"
tags: [research, codebase, bevy_voxel_world, voxel, chunking, meshing, replacement]
status: complete
last_updated: 2026-02-03
last_updated_by: Claude
---

# Research: Features & Performance Optimizations to Re-implement When Replacing `bevy_voxel_world`

**Date**: 2026-02-03T18:00:01-08:00
**Researcher**: Claude
**Git Commit**: 8c199a60d71eb448d2cc2503f7dcbbb890094aab
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

What features and performance optimizations does `bevy_voxel_world` provide that we currently use, and what would need to be re-implemented in a custom voxel engine?

## Summary

The project uses `bevy_voxel_world` (v0.14.0, local fork at `git/bevy_voxel_world/`) across three crates: `protocol`, `client`, and `server`. The library provides a full voxel pipeline: chunk lifecycle management, async mesh generation with greedy meshing, LOD support, voxel raycasting, mesh caching, a custom WGSL material with array textures and ambient occlusion, and a `VoxelWorld` SystemParam for reading/writing voxels. Below is a complete audit of every feature and optimization, organized by subsystem, with notes on which ones this project actively uses.

---

## Detailed Findings

### 1. Voxel Data Model

**Source**: `git/bevy_voxel_world/src/voxel.rs`

- `WorldVoxel<I>` enum: `Unset`, `Air`, `Solid(I)` — generic over material index type `I`
- Implements `block_mesh::Voxel` and `MergeVoxel` traits for greedy meshing
- `VoxelFace` enum for ray traversal face identification
- **Project usage**: `I = u8`. Protocol defines `VoxelType` with conversions to/from `WorldVoxel`. (`crates/protocol/src/map.rs:48-71`)

### 2. Chunk Data Structures

**Source**: `git/bevy_voxel_world/src/chunk.rs`

- Fixed 32³ voxel chunks (`CHUNK_SIZE_U = 32`), with 1-voxel padding → 34³ padded shape
- `ChunkData<I>`: stores voxel array (`Arc<[WorldVoxel<I>]>`), position, LOD level, `is_full`/`is_empty` flags, `FillType` enum, voxel hash, and per-chunk data/mesh shapes
- `FillType::Uniform` optimization: fully-solid single-material chunks discard the voxel array and store only the fill type
- `Chunk<C>` ECS component: position, LOD level, entity, data/mesh shapes
- `ChunkTask<C, I>`: async task struct holding generation + meshing state
- **Marker components**: `NeedsRemesh` (sparse set), `NeedsDespawn`
- **Project usage**: `Chunk<MapWorld>` queried in both client and server for collider attachment and debug monitoring. (`crates/protocol/src/map.rs:96-98`, `crates/server/src/map.rs:273`, `crates/server/src/map.rs:359`)

### 3. Chunk Map (Spatial Index)

**Source**: `git/bevy_voxel_world/src/chunk_map.rs`

- `ChunkMap<C, I>`: `Arc<RwLock<HashMap<IVec3, ChunkData<I>>>>` with AABB bounds tracking
- Double-buffered writes: `ChunkMapInsertBuffer`, `ChunkMapUpdateBuffer`, `ChunkMapRemoveBuffer` — flushed each frame to avoid write contention
- AABB maintained incrementally on insert/update; rebuilt from all keys only when a boundary chunk is removed
- Capacity pre-allocated to 1000 entries
- **Project usage**: accessed implicitly through `VoxelWorld<MapWorld>` SystemParam

### 4. Modified Voxels Persistence

**Source**: `git/bevy_voxel_world/src/voxel_world_internal.rs:57-75`

- `ModifiedVoxels<C, I>`: `Arc<RwLock<HashMap<IVec3, WorldVoxel<I>>>>` — persists player edits across chunk spawn/despawn cycles
- `VoxelWriteBuffer<C, I>`: frame-local write buffer flushed at end of frame, triggers `NeedsRemesh` on affected chunks
- During `ChunkTask::generate()`, modified voxels are checked first, overriding procedural generation
- **Project usage**: core to voxel editing. `VoxelWorld::set_voxel()` writes to this buffer. Used in client broadcast handlers and server edit handlers.

### 5. Chunk Lifecycle (Spawn / Despawn / LOD Update)

**Source**: `git/bevy_voxel_world/src/voxel_world_internal.rs`

#### 5a. Chunk Spawning (`spawn_chunks`)
- Driven by `ChunkRenderTarget<C>` entities (any entity with Transform, optionally Camera)
- **Camera-based**: casts `spawning_rays` random rays into the viewport (with `spawning_ray_margin`), queuing unspawned chunks along each ray. Stops early at fully-solid chunks.
- **Transform-only (non-camera)**: radial flood-fill within `spawning_distance`
- Protected radius: `min_despawn_distance` always-spawned zone around each target
- Multi-target: checks visibility against ALL targets; uses nearest target for LOD
- `max_spawn_per_frame` throttle
- Configurable via `ChunkSpawnStrategy`: `CloseAndInView` (default) or `Close`
- **Project usage**: `spawning_distance = 2` configured in `MapWorld`. Server uses transform-only targets (no camera), client uses camera-based. (`crates/protocol/src/map.rs:29-31`)

#### 5b. Chunk Despawning (`retire_chunks` + `despawn_retired_chunks`)
- Chunks despawned if invisible to ALL targets
- `ChunkDespawnStrategy`: `FarAwayOrOutOfView` (default) or `FarAway`
- Frustum culling via `chunk_visible_to_camera()`: tests chunk center + 8 corners against camera NDC
- Transform-only targets have no frustum culling (all in-range chunks are "visible")
- `min_despawn_distance` protects nearby chunks from despawning
- Two-phase: tag with `NeedsDespawn`, then despawn + remove from ChunkMap

#### 5c. LOD Updates (`update_chunk_lods`)
- Per-chunk LOD computed via `VoxelWorldConfig::chunk_lod()` using nearest target position
- Supports hysteresis via `previous_lod` parameter
- LOD changes trigger `NeedsRemesh` and new `data_shape`/`mesh_shape`
- **Project usage**: not actively used (default LOD 0 for all chunks)

#### 5d. Chunk Events
- `ChunkWillSpawn`, `ChunkWillDespawn`, `ChunkWillRemesh`, `ChunkWillChangeLod`, `ChunkWillUpdate` — all Bevy Message events
- **Project usage**: not explicitly observed by project code

### 6. Async Chunk Generation & Meshing

**Source**: `git/bevy_voxel_world/src/voxel_world_internal.rs:527-630`

- Uses Bevy's `AsyncComputeTaskPool` for off-main-thread generation
- `max_active_chunk_threads` throttle (default: unlimited)
- Two-phase per chunk:
  1. **Generate**: populate voxel array via `voxel_lookup_delegate`, checking `ModifiedVoxels` first, optionally reusing previous chunk data (`ChunkRegenerateStrategy::Reuse` default)
  2. **Mesh**: skip if empty or full; check `MeshCache` for hash hit; otherwise run meshing function
- `ChunkThread<C, I>` component (sparse set) tracks in-flight tasks
- Result polling via `futures_lite::future::block_on(poll_once(...))` each frame

### 7. Greedy Meshing

**Source**: `git/bevy_voxel_world/src/meshing.rs`

- Uses `block-mesh` crate (`visible_block_faces` with `RIGHT_HANDED_Y_UP_CONFIG`)
- `ndshape::RuntimeShape` for dynamic padded chunk dimensions
- LOD downsampling: `resample_voxels_nearest()` when `data_shape != mesh_shape`
- Per-face ambient occlusion (`face_aos` → `side_aos` → `ao_value`) baked into vertex colors
- Texture index per-face: top/sides/bottom mapping via `texture_index_mapper`
- Custom vertex attribute `ATTRIBUTE_TEX_INDEX` (`Uint32x3`) for array texture indexing
- Mesh output: positions, normals, UVs, vertex colors (AO), tex indices, triangle indices
- **Project usage**: default meshing (no custom `chunk_meshing_delegate`)

### 8. Mesh Cache

**Source**: `git/bevy_voxel_world/src/mesh_cache.rs`

- `MeshCache<C>`: `WeakValueHashMap<u64, Weak<Handle<Mesh>>>` keyed by voxel hash
- Automatically drops unreferenced meshes (weak references)
- `MeshRef` component on each chunk entity holds `Arc<Handle<Mesh>>` to keep weak map entries alive
- Enables Bevy draw-call batching for identical chunks (e.g., large flat terrain areas)
- Double-buffered inserts via `MeshCacheInsertBuffer`
- Pre-allocated capacity: 2000
- **Project usage**: active (default behavior)

### 9. Voxel Raycasting

**Source**: `git/bevy_voxel_world/src/voxel_world.rs:321-392`, `git/bevy_voxel_world/src/voxel_traversal.rs`

- `VoxelWorld::raycast()` and `VoxelWorld::raycast_fn()` (sendable closure)
- Ray-AABB entry/exit against loaded chunk bounds, then `voxel_line_traversal`
- `voxel_line_traversal`: Amanatides & Woo fast voxel traversal algorithm with face normal tracking
- `voxel_cartesian_traversal`: axis-aligned grid traversal
- Filter function support: `&dyn Fn((Vec3, WorldVoxel<I>)) -> bool`
- Returns `VoxelRaycastResult { position, normal, voxel }`
- **Project usage**: used in `handle_voxel_input` for place/remove voxel raycasts from cursor. (`crates/client/src/map.rs:94`)

### 10. Voxel Material & Shader

**Source**: `git/bevy_voxel_world/src/voxel_material.rs`, `src/shaders/voxel_texture.wgsl`

- `StandardVoxelMaterial`: Bevy `MaterialExtension` for `ExtendedMaterial<StandardMaterial, _>`
- Custom WGSL vertex + fragment shader:
  - Passes `tex_idx` (vec3<u32>) from vertex to fragment
  - Selects array texture layer based on face normal (top/sides/bottom)
  - Multiplies texture color by vertex color (AO)
  - Integrates with Bevy PBR lighting pipeline
- `2d_array` texture binding for multi-material voxels
- Repeating sampler address mode
- Default fallback texture (`default_texture.png` — 4 layers)
- Custom material support via `with_material()`
- **Project usage**: default material (no custom material, no `voxel_texture` configured → uses default 4-layer texture)

### 11. VoxelWorld SystemParam (Public API)

**Source**: `git/bevy_voxel_world/src/voxel_world.rs`

- `VoxelWorld<'w, C>`: SystemParam wrapping `ChunkMap`, `ModifiedVoxels`, `VoxelWriteBuffer`, and config
- `get_voxel(pos)` / `get_voxel_fn()` — checks write buffer → modified voxels → chunk map
- `set_voxel(pos, voxel)` — pushes to write buffer
- `get_chunk_data(chunk_pos)` / `get_chunk_data_fn()`
- `raycast()` / `raycast_fn()` — see §9
- Deprecated: `get_closest_surface_voxel`, `get_random_surface_voxel`, `get_surface_voxel_at_2d_pos`
- **Project usage**: extensively used in client (handle broadcasts, handle input, state sync) and server (handle edits, load world, debug). All via `VoxelWorld<MapWorld>`.

### 12. VoxelWorldConfig Trait

**Source**: `git/bevy_voxel_world/src/configuration.rs`

Full configuration surface (project overrides in **bold**):

| Method | Default | Project Override |
|--------|---------|-----------------|
| `MaterialIndex` type | `u8` | **`u8`** |
| `ChunkUserBundle` type | `()` | **`()`** |
| **`spawning_distance()`** | 10 | **2** |
| `min_despawn_distance()` | 1 | default |
| `chunk_despawn_strategy()` | `FarAwayOrOutOfView` | default |
| `chunk_spawn_strategy()` | `CloseAndInView` | default |
| `max_spawn_per_frame()` | 10000 | default |
| `spawning_rays()` | 100 | default |
| `spawning_ray_margin()` | 25 | default |
| `debug_draw_chunks()` | false | default |
| `texture_index_mapper()` | all `[0,0,0]` | default |
| **`voxel_lookup_delegate()`** | returns `Unset` | **flat terrain: solid below y=0** |
| `chunk_meshing_delegate()` | `None` (default greedy mesh) | default |
| `voxel_texture()` | `None` (built-in 4-layer) | default |
| `init_custom_materials()` | true | default |
| `chunk_lod()` | 0 | default |
| `chunk_data_shape()` | 34³ | default |
| `chunk_meshing_shape()` | 34³ | default |
| `chunk_regenerate_strategy()` | `Reuse` | default |
| `max_active_chunk_threads()` | `usize::MAX` | default |
| `attach_chunks_to_root()` | true | default |

### 13. Chunk Collider Integration

**Source**: `crates/protocol/src/map.rs:94-119`

- `attach_chunk_colliders()`: shared system used by both client and server
- Watches for `Changed<Mesh3d>` or `Added<Mesh3d>` on `Chunk<MapWorld>` entities
- Converts mesh to `Collider::trimesh_from_mesh()` (avian3d)
- Attaches `Collider` + `RigidBody::Static`
- This is project code, not library code, but depends on `Chunk<C>` component and mesh lifecycle

### 14. Debug Drawing

**Source**: `git/bevy_voxel_world/src/debug_draw.rs`

- `VoxelWorldDebugDrawPlugin<C>`: optional plugin for gizmo visualization
- `VoxelWorldDebugDraw<C>` SystemParam for adding/clearing voxel and ray gizmos
- `debug_draw_chunks()`: draws cuboid gizmos for non-empty chunks
- `ChunkGizmos` gizmo config group
- **Project usage**: not used (no debug draw plugin added)

### 15. World Root Entity

**Source**: `git/bevy_voxel_world/src/voxel_world_internal.rs:95-114`

- `WorldRoot<C>` component on a root entity
- All chunks parented to root when `attach_chunks_to_root() = true` (default)
- Enables transforming/hiding entire voxel world at once
- Trade-off: transform propagation cost on chunk spawn/despawn
- `init_root()` hook for custom initialization

---

## Dependencies to Replace

The `bevy_voxel_world` crate depends on:

| Crate | Version | Purpose |
|-------|---------|---------|
| `block-mesh` | 0.2.0 | Greedy meshing (`visible_block_faces`) |
| `ndshape` | 0.3.0 | N-dimensional array indexing |
| `futures-lite` | 2.6.1 | Async task polling |
| `rand` | 0.9.2 | Random viewport ray points for chunk spawning |
| `weak-table` | 0.3.2 | Weak-reference mesh cache |
| `hashbrown` | 0.16.1 | Fast HashMap for chunk map |
| `bevy_shader` | 0.17 | Shader asset loading |

---

## Features Actively Used by This Project

### Must Re-implement (actively used)

1. **VoxelWorld SystemParam** — `get_voxel()`, `set_voxel()`, `raycast()` API
2. **Chunk lifecycle** — spawn/despawn based on ChunkRenderTarget distance (transform-only for server, camera-based for client)
3. **Async chunk generation** — off-thread voxel data population from delegate function
4. **Greedy meshing** — `block-mesh` based mesh generation with per-face AO
5. **Modified voxels persistence** — edits survive chunk despawn/respawn cycles
6. **Voxel write buffer** — batched writes flushed per frame, triggering remesh
7. **Chunk map** — spatial index for fast chunk lookup by IVec3 position
8. **Mesh cache** — hash-based deduplication with weak references
9. **Raycasting** — Amanatides & Woo traversal for voxel picking
10. **Custom WGSL material** — array texture + AO vertex colors + PBR integration
11. **ChunkRenderTarget** — multi-target chunk visibility (server has transform targets, client has camera)
12. **Chunk component** — `Chunk<C>` with position for collider attachment queries
13. **Voxel data model** — `WorldVoxel` enum with serde support

### Not Currently Used (can defer or skip)

1. **LOD system** — chunk_lod(), data_shape/mesh_shape per LOD, voxel resampling
2. **Custom chunk meshing delegate** — project uses default greedy mesher
3. **Custom materials** — project uses default StandardVoxelMaterial
4. **Custom voxel textures** — project uses built-in default texture
5. **ChunkUserBundle** — set to `()`
6. **Debug drawing plugin** — not added
7. **Chunk events** — not observed by project code
8. **ChunkRegenerateStrategy** — using default `Reuse`
9. **Flood-fill spawning** (`ChunkSpawnStrategy::Close`) — using default `CloseAndInView`

---

## Performance Optimizations in `bevy_voxel_world`

| Optimization | Location | Description |
|---|---|---|
| **Async compute meshing** | `voxel_world_internal.rs:527-630` | Chunk gen+mesh runs on `AsyncComputeTaskPool`, non-blocking main thread |
| **Mesh cache (hash dedup)** | `mesh_cache.rs` | Identical voxel configurations share a single mesh handle; enables GPU draw batching |
| **Empty/full chunk skip** | `chunk.rs:483-495` | No mesh generated for all-air or all-solid chunks |
| **Uniform chunk optimization** | `chunk.rs:486-488` | Single-material full chunks discard voxel array, storing only FillType::Uniform |
| **Double-buffered writes** | `chunk_map.rs`, `mesh_cache.rs`, `voxel_world_internal.rs` | ChunkMap/MeshCache use insert/update/remove buffers to minimize RwLock contention |
| **Ray-AABB early exit** | `voxel_world.rs:339-354` | Raycast clips to loaded chunk AABB before traversal |
| **Full-chunk ray stop** | `voxel_world_internal.rs:177-179` | Spawning rays stop at fully-solid chunks |
| **Sparse set components** | `chunk.rs:47,63` | `ChunkThread` and `NeedsRemesh` use sparse set storage for fast add/remove |
| **Write buffer lookup** | `voxel_world.rs:170-175` | `get_voxel()` checks pending writes before chunk map |
| **Incremental AABB** | `chunk_map.rs:98-103` | Bounds updated incrementally on insert, full rebuild only on boundary removal |
| **Weak mesh references** | `mesh_cache.rs` | Unused meshes automatically collected when no chunks reference them |
| **Max spawn throttle** | `voxel_world_internal.rs:238` | `max_spawn_per_frame` prevents frame stalls |
| **Max thread throttle** | `voxel_world_internal.rs:543-548` | `max_active_chunk_threads` limits concurrent gen tasks |
| **Greedy meshing** | `meshing.rs` (via block-mesh) | Merges adjacent same-material faces into larger quads |
| **Vertex AO baking** | `meshing.rs:126-198` | Ambient occlusion computed at mesh time, stored as vertex colors (no runtime cost) |
| **Pre-allocated capacities** | Various | ChunkMap: 1000, MeshCache: 2000, chunks_deque adaptive |
| **LOD resampling** | `meshing.rs:55-61` | Nearest-neighbor downsample for coarser LOD meshes (not currently used) |
| **Data reuse on LOD change** | `chunk.rs:404-425` | `ChunkRegenerateStrategy::Reuse` preserves high-res data when downgrading LOD |

---

## Code References

- `git/bevy_voxel_world/src/lib.rs` — module structure and public API exports
- `git/bevy_voxel_world/src/configuration.rs` — `VoxelWorldConfig` trait (full config surface)
- `git/bevy_voxel_world/src/plugin.rs` — `VoxelWorldPlugin` (system registration, material setup)
- `git/bevy_voxel_world/src/voxel_world.rs` — `VoxelWorld` SystemParam, `ChunkRenderTarget`, raycast
- `git/bevy_voxel_world/src/voxel_world_internal.rs` — chunk lifecycle, async gen/mesh, write buffer flush
- `git/bevy_voxel_world/src/chunk.rs` — `Chunk`, `ChunkData`, `ChunkTask`, generation logic
- `git/bevy_voxel_world/src/chunk_map.rs` — `ChunkMap` spatial index with double-buffered writes
- `git/bevy_voxel_world/src/meshing.rs` — greedy meshing, AO, LOD resampling
- `git/bevy_voxel_world/src/mesh_cache.rs` — weak-ref mesh deduplication
- `git/bevy_voxel_world/src/voxel_traversal.rs` — Amanatides & Woo ray traversal
- `git/bevy_voxel_world/src/voxel.rs` — `WorldVoxel`, `VoxelFace`
- `git/bevy_voxel_world/src/voxel_material.rs` — `StandardVoxelMaterial`, texture prep
- `git/bevy_voxel_world/src/shaders/voxel_texture.wgsl` — array texture PBR shader
- `crates/protocol/src/map.rs` — `MapWorld` config, `VoxelType`, collider attachment
- `crates/client/src/map.rs` — `ClientMapPlugin`, broadcast handlers, raycasting input
- `crates/server/src/map.rs` — `ServerMapPlugin`, persistence, edit handling

## Historical Context (from doc/)

- `doc/research/2025-12-24-bevy-voxel-world-map-plugins.md` — early research on structuring server/client map plugins
- `doc/research/2026-01-02-multi-camera-bevy-voxel-world.md` — multi-camera support refactor research
- `doc/research/2026-01-03-voxel-world-camera-to-chunk-visibility-target.md` — refactoring from camera-based to transform-based chunk visibility (led to `ChunkRenderTarget`)
- `doc/research/2026-01-17-voxel-world-save-load.md` — save/load architecture research
- `doc/research/2026-01-22-noise-generation-voxel-terrain.md` — procedural terrain generation research
- `doc/plans/2026-01-18-voxel-world-persistence.md` — persistence implementation plan
- `doc/plans/2026-01-04-transform-based-chunk-visibility.md` — chunk visibility refactor plan

## Related Research

- [doc/research/2026-01-22-noise-generation-voxel-terrain.md](doc/research/2026-01-22-noise-generation-voxel-terrain.md) — procedural terrain (will need custom `voxel_lookup_delegate` replacement)
- [doc/research/2026-01-17-voxel-world-save-load.md](doc/research/2026-01-17-voxel-world-save-load.md) — persistence (depends on `ModifiedVoxels` pattern)

## Open Questions

1. **Noise terrain**: The current `voxel_lookup_delegate` is a flat plane. The noise terrain research (`2026-01-22`) hasn't been integrated yet. A custom engine should be designed with the noise-based delegate in mind from the start.
2. **LOD strategy**: Not currently used, but the VISION.md describes an open world. Will LOD be needed for the overworld? This affects chunk data structure design.
3. **Chunk size**: Fixed at 32³. Is this the right size for the game's use case, or should the custom engine allow configurable chunk sizes?
4. **Serde for voxels**: The `WorldVoxel` type has serde support via feature flag, used for save/load. A custom replacement needs to maintain this.
5. **Multiple voxel worlds**: `bevy_voxel_world` supports multiple world instances via the config generic `C`. The project currently uses only `MapWorld`. Will home-base and overworld need separate voxel world instances?
