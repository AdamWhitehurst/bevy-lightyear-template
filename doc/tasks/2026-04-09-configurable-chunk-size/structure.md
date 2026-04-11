# Structure Outline

## Approach

Add `chunk_size: u32` + `column_y_range: (i32, i32)` to `VoxelMapConfig` and a `RuntimePow2Shape<u32, 3>` to `VoxelMapInstance`; migrate call sites subsystem-by-subsystem from static `PaddedChunkShape` dispatch to runtime shape dispatch; at each phase baseline `chunk_size=16` must remain working. Final phase removes the `CHUNK_SIZE` constant and demonstrates a second map with `chunk_size=32`.

---

## Phase 1: Config & Network Plumbing

Thread `chunk_size`, `column_y_range`, and the runtime shape instance end-to-end from server spawn through `MapTransitionStart` / `ChunkDataSync` to client reconstruction. Nothing consumes the new fields yet — call sites still use the `CHUNK_SIZE` constant.

**Files**: `voxel_map_engine/src/config.rs`, `voxel_map_engine/src/instance.rs`, `voxel_map_engine/src/types.rs`, `protocol/src/map/transition.rs`, `protocol/src/map/chunk.rs`, `server/src/map.rs`, `client/src/map.rs`

**Key changes**:
- `VoxelMapConfig { ..existing, chunk_size: u32, padded_size: u32, column_y_range: (i32, i32) }` — new fields
- `VoxelMapConfig::new(..., chunk_size: u32, column_y_range: (i32, i32))` with `debug_assert!(chunk_size.is_power_of_two() && chunk_size >= 8)`
- `VoxelMapInstance { ..existing, shape: RuntimePow2Shape<u32, 3>, chunk_size: u32, padded_size: u32 }` — shape built at spawn
- `MapTransitionStart { ..existing, chunk_size: u32, column_y_range: (i32, i32) }`
- `ChunkDataSync { ..existing, chunk_size: u32 }`
- Remove dead `PADDED_CHUNK_SIZE` const from `types.rs`

**Verify**: `cargo check-all` passes; `cargo server` + `cargo client` — overworld loads, player moves, `trace!` on client confirms `MapTransitionStart.chunk_size == 16` received.

---

## Phase 2: Meshing Runtime Dispatch

Switch `mesh_chunk_greedy` to accept a `Shape<3, Coord = u32>` from `VoxelMapInstance.shape`, replacing hardcoded `[17; 3]` bounds and `PaddedChunkShape {}` dispatch. First subsystem fully off the compile-time shape.

**Files**: `voxel_map_engine/src/meshing.rs`, `voxel_map_engine/src/lifecycle.rs` (mesh-building systems), mesh test files using `PaddedChunkShape::USIZE`

**Key changes**:
- `mesh_chunk_greedy<S: Shape<3, Coord = u32>>(voxels: &[WorldVoxel], shape: &S) -> Option<Mesh>`
- `greedy_quads(voxels, shape, [0; 3], [shape.as_array()[0] - 1; 3], ..)`
- `debug_assert_eq!(voxels.len(), shape.usize())`
- Mesh-build system captures `instance.shape.clone()` for the async meshing task

**Verify**: `cargo check-all` passes; `cargo server` + `cargo client` — overworld meshes render visually identical to baseline (spot-check corners, slopes, caves).

---

## Phase 3: API & Coordinate Math Dispatch

Route `voxel_to_chunk_pos`, `world_to_chunk_pos`, `chunk_world_offset`, `VoxelWorld::get_voxel`, `VoxelMapInstance::set_voxel`/`update_neighbor_padding`, `column_to_chunks`, `jittered_grid_sample` footprint, and `lookup_voxel*` through the runtime chunk size and shape. Remove `DEFAULT_COLUMN_Y_MIN/MAX`.

**Files**: `voxel_map_engine/src/api.rs`, `voxel_map_engine/src/instance.rs`, `voxel_map_engine/src/lifecycle.rs`, `voxel_map_engine/src/placement.rs`, `voxel_map_engine/src/ticket.rs`, `client/src/map.rs` (UnloadColumn handler)

**Key changes**:
- `voxel_to_chunk_pos(voxel_pos: IVec3, chunk_size: u32) -> IVec3` (uses `div_euclid`)
- `world_to_chunk_pos(translation: Vec3, chunk_size: u32) -> IVec3`
- `chunk_world_offset(chunk_pos: IVec3, chunk_size: u32) -> Vec3`
- `column_to_chunks(col: IVec2, y_range: (i32, i32)) -> impl Iterator<Item = IVec3>`
- `instance.shape.linearize([px, py, pz])` replaces `PaddedChunkShape::linearize(..)` at all 8 Addendum-A call sites
- `VoxelWorld::get_voxel` pulls `chunk_size` + `shape` from the queried `VoxelMapInstance`

**Verify**: `cargo server` + `cargo client` — chunk streaming works; block break/place functions; player traversal across chunk boundaries produces no visual or collision seams; ticket columns cover `-8..8` vertical range.

---

## Phase 4: Terrain Generation & `SurfaceHeightMap`

Runtime-size the terrain fill loops and rewrite `SurfaceHeightMap` as a boxed padded slice. Feature placement reads padded heights, and the chunk-edge slope clip is removed.

**Files**: `voxel_map_engine/src/config.rs`, `voxel_map_engine/src/generation.rs`, `voxel_map_engine/src/terrain.rs`

**Key changes**:
- `SurfaceHeightMap { heights: Box<[Option<f64>]>, padded_size: u32 }` with `SurfaceHeightMap::new(padded_size: u32)` and `fn at(&self, px: u32, pz: u32) -> Option<f64>`
- `build_surface_height_map(chunk_pos, palette, shape, padded_size)` — iterates full padded `0..padded_size` XZ footprint
- `generate_terrain(chunk_pos, chunk_size, padded_size)` — replaces `vec![Air; PaddedChunkShape::SIZE as usize]` with `vec![Air; (padded_size as usize).pow(3)]`
- `place_features(..)` candidate lookup uses `heights.at(local_x + 1, local_z + 1)`
- Slope bounds at `terrain.rs:447` deleted; neighbors always read via padded `heights.at`

**Verify**: `cargo server` + `cargo client` — terrain generates matching baseline topology; surface features (trees, rocks) spawn; slopes at chunk edges no longer clip oddly; compare to pre-phase screenshots.

---

## Phase 5: Persistence Format + Cross-Boundary Validation

Bump `CHUNK_SAVE_VERSION`, embed `chunk_size` in the save header, and gate loads + network receipts on matching chunk size. Turn silent corruption into hard errors.

**Files**: palette/save module in `voxel_map_engine` (wherever `CHUNK_SAVE_VERSION` lives), `server/src/map.rs` (chunk load path), `client/src/map.rs` (`ChunkDataSync` handler)

**Key changes**:
- `CHUNK_SAVE_VERSION` bumped; save file header includes `chunk_size: u32`
- Save load returns `Err` on version mismatch or `chunk_size != map.chunk_size`
- `ChunkDataSync` handler validates `sync.chunk_size == instance.chunk_size`, logs and drops mismatched chunks with `error!`

**Verify**: delete `assets/sprites/humanoid/worlds/overworld/terrain/`; `cargo server` + `cargo client` — fresh world saves and reloads; hand-edit a save header to force mismatch → error reported, no crash.

---

## Phase 6: Remove `CHUNK_SIZE` Constant + Two-Map Demo

Delete `CHUNK_SIZE`, `PaddedChunkShape`, `PADDED_VOLUME`; configure one non-overworld map (homebase or arena) with `chunk_size = 32`; confirm client transitions between maps with different chunk sizes. This is the first phase where the feature is actually observable.

**Files**: `voxel_map_engine/src/types.rs`, `voxel_map_engine/src/palette.rs`, `server/src/map.rs` (homebase/arena spawn path around `map.rs:1048`), terrain def asset files if chunk_size is part of the def

**Key changes**:
- `CHUNK_SIZE`, `PaddedChunkShape`, `PADDED_CHUNK_SIZE`, `PADDED_VOLUME` removed
- `palette.rs` storage size derived from `PalettedChunk::Indirect.len` field, not a const
- Homebase (or arena) spawn passes `chunk_size=32, column_y_range=(-4, 4)` to `VoxelMapConfig::new`

**Verify**: `cargo check-all` passes with zero `CHUNK_SIZE` references; `cargo server` + `cargo client` — overworld (`chunk_size=16`) loads, transition to homebase (`chunk_size=32`) loads without seams or coordinate desync, return transition works, two simultaneous maps coexist without cross-contamination.

---

## Testing Checkpoints

- **After Phase 1**: Baseline game loop works. Client `trace!` logs confirm `chunk_size=16` received via `MapTransitionStart` and every `ChunkDataSync`. No behavioral change.
- **After Phase 2**: Meshing no longer references `PaddedChunkShape`. Visual parity vs. pre-phase screenshots.
- **After Phase 3**: No `CHUNK_SIZE` references in `api.rs`, `instance.rs`, `lifecycle.rs`, `placement.rs`, `ticket.rs`. Block edits and chunk streaming still correct across negative/positive coordinates.
- **After Phase 4**: No `CHUNK_SIZE` references in `terrain.rs`, `generation.rs`, `config.rs`. Surface features place; chunk-edge slopes are consistent.
- **After Phase 5**: Save round-trips correctly on fresh dir; mismatched save header produces a loud error, not corruption.
- **After Phase 6**: `CHUNK_SIZE` constant is gone; second map with a different chunk size loads and renders; map transitions work both ways.
