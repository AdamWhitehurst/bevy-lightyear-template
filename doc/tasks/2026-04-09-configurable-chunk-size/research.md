# Research Findings

## Q1: Where is `CHUNK_SIZE` (and derived constants) referenced? What is each usage for?

### Findings

**Definitions** (`types.rs`):
- `PaddedChunkShape = ConstShape3u32<18, 18, 18>` — `types.rs:7` (literal 18, not derived from `CHUNK_SIZE`)
- `CHUNK_SIZE: u32 = 16` — `types.rs:9`
- `PADDED_CHUNK_SIZE: u32 = 18` — `types.rs:10` (**dead** — never imported outside `types.rs`)
- `PADDED_VOLUME: usize = 18 * 18 * 18` — `palette.rs:6` (local to that file, does not import `PADDED_CHUNK_SIZE`)

**Coordinate math (hot — per-voxel or per-chunk-fill)**:
- `lifecycle.rs:977,981` — world↔chunk position conversion (per-frame)
- `api.rs:38,44,118,124,149,155,167-169` — voxel↔chunk resolution + padded index (per voxel lookup)
- `instance.rs:133,145,164,166,171,178` — voxel edit + boundary neighbor propagation
- `terrain.rs:236,266-267,279-280` — innermost 18³ loop during chunk fill
- `generation.rs:185-193` — triple-nested loop for height map
- `placement.rs:22,94` — jittered grid sample world footprint

**Coordinate math (moderate — per-chunk or per-surface-query)**:
- `terrain.rs:381-383,388,438,447,450` — feature placement local coord, height map stride, slope bounds

**Memory allocation (per-chunk generation, one-time)**:
- `terrain.rs:231,594` — `vec![Air; PaddedChunkShape::SIZE as usize]`
- Many test files — `vec![Air; PaddedChunkShape::USIZE]`

**Mesh generation (per dirty chunk)**:
- `meshing.rs:12` — `debug_assert_eq!(voxels.len(), PaddedChunkShape::USIZE)`
- `meshing.rs:20` — `&PaddedChunkShape {}` passed to `block_mesh::visible_block_faces`
- `meshing.rs:22-23` — hardcoded bounds `[0; 3]` and `[17; 3]`

**Hardcoded literal derived from CHUNK_SIZE**:
- `config.rs:43` — `heights: [Option<f64>; 256]` (256 = 16², not derived from the constant)

---

## Q2: How does `PalettedChunk` allocate and index its voxel storage?

### Findings

**Storage size** comes from `palette.rs:6` `PADDED_VOLUME = 18 * 18 * 18` — a file-local literal, not imported from `types.rs`.

**Structure** (`palette.rs:24-40`): enum with two variants:
- `SingleValue(WorldVoxel)` — zero heap allocation for uniform chunks
- `Indirect { palette: Vec<WorldVoxel>, data: Vec<u64>, bits_per_entry: u8, len: usize }` — packed bit array for 2-256 distinct voxel types

**Creation** (`palette.rs:46-67`): `from_voxels(voxels: &[WorldVoxel])` asserts `voxels.len() == PADDED_VOLUME` at line 47-50. Builds palette via `build_palette` (line 53), then either returns `SingleValue` (1 entry) or packs indices into `Vec<u64>` via `pack_indices` (line 59). Entries never span u64 word boundaries.

**Read** (`palette.rs:95-116`): For `Indirect`, computes `word_idx = index / entries_per_word`, `bit_offset = (index % entries_per_word) * bits_per_entry`, extracts palette index.

**Write** (`palette.rs:123-153`): If voxel type exists in palette, in-place bit update. If new type, full expand → patch → re-encode from scratch.

**Index computation** is always done externally via `PaddedChunkShape::linearize([px, py, pz])` — the palette itself takes a flat `usize` index. The `+1` padding offset is applied by callers (e.g., `api.rs:38`, `instance.rs:145`).

**Terrain fill path**: `generate_terrain` (`generation.rs:204-220`) → trait method returns `Vec<WorldVoxel>` of length 18³ → `ChunkData::from_voxels` (`types.rs:99`) → `PalettedChunk::from_voxels`.

---

## Q3: How does the chunk lifecycle convert world-space to chunk coordinates?

### Findings

**Float path** — `world_to_chunk_pos` (`lifecycle.rs:976-977`):
```rust
(translation / CHUNK_SIZE as f32).floor().as_ivec3()
```
Called via `world_to_column_pos` (`lifecycle.rs:201-203`) which drops Y → `IVec2`.

**Integer path** — `voxel_to_chunk_pos` (`api.rs:165-170`):
```rust
voxel_pos.x.div_euclid(CHUNK_SIZE as i32)  // per axis
```
`div_euclid` handles negatives correctly (unlike `/`).

**Ticket → column flow**:
1. `collect_tickets` (`lifecycle.rs:427-508`): reads `GlobalTransform` for ticket entity, transforms into map-local space via inverted map transform (`lifecycle.rs:474-475`).
2. Calls `world_to_column_pos(local_pos)` → `IVec2` column.
3. Passes to `propagator.set_source(ticket_entity, column, base_level, radius)` for BFS level propagation.

**Column → chunks**: `column_to_chunks(col, y_min, y_max)` (`ticket.rs:146-148`) iterates `y_min..y_max` producing `IVec3::new(col.x, y, col.y)`. Defaults: `DEFAULT_COLUMN_Y_MIN = -8`, `DEFAULT_COLUMN_Y_MAX = 8` (`ticket.rs:141-142`). `CHUNK_SIZE` does not enter this function.

**Chunk → world offset**: `chunk_world_offset` (`lifecycle.rs:980-981`):
```rust
chunk_pos.as_vec3() * CHUNK_SIZE as f32 - Vec3::ONE
```
The `- Vec3::ONE` corrects for the 1-voxel padding border. Used at mesh entity spawn (`lifecycle.rs:919,939,1156,1174`).

---

## Q4: How does the meshing pipeline use chunk dimensions?

### Findings

**Entry**: `mesh_chunk_greedy(voxels: &[WorldVoxel]) -> Option<Mesh>` (`meshing.rs:11`).

**Size assertion** (`meshing.rs:12`): `debug_assert_eq!(voxels.len(), PaddedChunkShape::USIZE)`.

**Greedy quads** (`meshing.rs:18-26`): Calls `greedy_quads` with min `[0; 3]`, max `[17; 3]` — hardcoded to match 18³ padded shape. The function iterates interior `[1..16]` for visible faces.

**Vertex positions** (`meshing.rs:44`): `face.quad_mesh_positions(quad, 1.0)` — scale `1.0` means 1 voxel = 1 world unit. Positions are in padded local space (0-17). No CHUNK_SIZE scalar applied.

**UV coordinates** (`meshing.rs:46-50`): Generated by `block_mesh::tex_coords` with merge enabled. UV values are in 0..N space where N is merged quad extent in voxels. No normalization by chunk size.

**World placement**: `chunk_world_offset` (`lifecycle.rs:980-981`) positions the mesh entity at `chunk_pos * 16 - 1`, aligning padded index 1 with world coordinate `chunk_pos * 16`.

**Colliders** (`protocol/src/map/colliders.rs:11-39`): `Collider::trimesh_from_mesh(mesh)` reads vertex positions directly from the Mesh. No chunk size constant referenced — bounds are purely derived from mesh geometry.

---

## Q5: How does terrain generation use chunk size?

### Findings

**Noise coordinate scaling** (`terrain.rs:258-273`): World coordinates are `chunk_pos * CHUNK_SIZE + px - 1` where `px ∈ 0..18`. The `-1` accounts for padding. Noise is sampled in integer world-space; `NoiseDef.frequency` on the noise function provides spatial scaling (e.g., `0.01`). `CHUNK_SIZE` is not baked into frequency — only into world offset.

**Surface height map** (`config.rs:41-43`): `heights: [Option<f64>; 256]` — literal `256 = 16²`. Indexed as `heights[x * CHUNK_SIZE + z]` where `x, z ∈ 0..CHUNK_SIZE`.

**Height map construction** (`generation.rs:178-201`): Outer loops `x in 0..CHUNK_SIZE`, `z in 0..CHUNK_SIZE`. Inner loop `py in (1..=CHUNK_SIZE).rev()` scans padded interior top-down. World Y: `chunk_pos.y * CHUNK_SIZE + (py - 1) + 1.0` (`generation.rs:192`).

**Feature placement** (`terrain.rs:366-429`): Calls `jittered_grid_sample` (`placement.rs:12-94`) which computes chunk footprint as `[chunk_pos * CHUNK_SIZE, chunk_pos * CHUNK_SIZE + CHUNK_SIZE)` on X/Z. Local coord extraction (`terrain.rs:381-383`): `world - chunk_pos * CHUNK_SIZE`, bounds check `>= CHUNK_SIZE`. Height map stride: `local_x * CHUNK_SIZE + local_z` (`terrain.rs:388`).

**Slope check** (`terrain.rs:437-450`): Bounds neighbors with `nx >= CHUNK_SIZE as i32` (`terrain.rs:447`).

---

## Q6: How does chunk networking encode/decode positions?

### Findings

**`ChunkDataSync`** (`protocol/src/map/chunk.rs:12-16`): Fields: `map_id: MapInstanceId`, `chunk_pos: IVec3`, `data: PalettedChunk`. Chunk positions are transmitted as raw chunk-space integers — no chunk size encoded in the message.

**Server sends** (`server/src/map.rs:813-818`): Passes `chunk_pos` from octree directly.

**Client receives** (`client/src/map.rs:155-162`): Inserts `sync.chunk_pos` directly into octree. No decoding math involving chunk size.

**`UnloadColumn`** (`protocol/src/map/chunk.rs:19-23`): Carries `column: IVec2`. Client expands via `column_to_chunks(col, DEFAULT_COLUMN_Y_MIN, DEFAULT_COLUMN_Y_MAX)` (`client/src/map.rs:183`).

**Implicit dependency**: Both sides share `CHUNK_SIZE = 16` as a compile-time constant from `voxel_map_engine`. The constant is never transmitted. If server and client had different compiled values, `chunk_world_offset` and `voxel_to_chunk_pos` would silently disagree.

---

## Q7: How does `VoxelMapConfig` get created and applied?

### Findings

**Fields** (`config.rs:54-67`): `seed`, `generation_version`, `spawning_distance`, `bounds: Option<IVec3>`, `tree_height`, `save_dir: Option<PathBuf>`, `generates_chunks: bool`.

**Server spawn** (`server/src/map.rs:97-126`): `spawn_overworld` at Startup. Loads persisted seed, calls `VoxelMapConfig::new(seed, gen_version, 2, None, 5)`. No `VoxelGenerator` at this point.

**Terrain def application** (`server/src/map.rs:136-150`): `apply_terrain_defs` runs in Update, gated on `resource_exists::<TerrainDefRegistry>`. Looks up terrain def by name (e.g., `"overworld"`), calls `apply_object_components` to insert reflected components onto the map entity, then marks `TerrainDefApplied`.

**Generator build** (`server/src/map.rs:179-196`): `build_terrain_generators` queries maps with `TerrainDefApplied` but without `VoxelGenerator`, calls `build_generator(entity_ref, config.seed)` to construct from terrain components.

**System chain**: `apply_terrain_defs → ApplyDeferred → build_terrain_generators`, runs `.before(lifecycle::ensure_pending_chunks)`.

**Client spawn** (`client/src/map.rs:98-113`): `VoxelMapConfig::new(0, 0, 2, None, 5)` with `generates_chunks = false` and a `FlatGenerator` placeholder.

**Map transitions** (`protocol/src/map/transition.rs:24-30`): `MapTransitionStart` carries `seed`, `generation_version`, `bounds`, `spawn_position`. Client reconstructs `VoxelMapConfig` from these fields (`client/src/map.rs:499-527`).

**Not replicated**: `VoxelMapConfig` is not a replicated component. Fields are manually transported inside `MapTransitionStart`.

**Adding a new field**: Would require updating `VoxelMapConfig::new`, the server spawn call, the `MapTransitionStart` message (if client needs the value), and the client's `handle_map_transition_start`.

---

## Q8: How does the `VoxelWorld` API resolve positions?

### Findings

**`VoxelWorld`** (`api.rs:14-24`): `SystemParam` wrapping query for `(VoxelMapInstance, VoxelMapConfig, VoxelGenerator)`.

**`voxel_to_chunk_pos`** (`api.rs:165-170`): `div_euclid(CHUNK_SIZE as i32)` per axis.

**Local offset** (e.g., `api.rs:38,118,149`): `local = voxel_pos - chunk_pos * CHUNK_SIZE as i32`, then `padded = [local + 1]` per axis, then `PaddedChunkShape::linearize(padded)`.

**Fallback**: If chunk not in octree, `evaluate_voxel_at` (`api.rs:141-145`) generates full chunk on-the-fly using `generator.generate_terrain(chunk_pos)` and reads from the result.

**Multi-map**: `VoxelWorld` iterates all map entities in its query. Each map has its own `VoxelMapInstance` (octree). But `CHUNK_SIZE` is a compile-time constant — all maps share the same chunk size. If two maps used different chunk sizes at runtime, `voxel_to_chunk_pos` and the padded indexing math would be wrong for one of them.

---

## Q9: How does chunk collider generation use chunk dimensions?

### Findings

**System**: `attach_chunk_colliders` (`protocol/src/map/colliders.rs:11-39`). Runs on `Added<Mesh3d>` or `Changed<Mesh3d>` for `VoxelChunk` entities.

**Method**: Calls `Collider::trimesh_from_mesh(mesh)` — Avian3D builds triangle mesh collider from raw Mesh vertex/index buffers.

**No chunk size reference**: Collider bounds are derived entirely from mesh geometry. The mesh is positioned in world space by `Transform::from_translation(chunk_world_offset(chunk_pos))` at spawn time. The collider inherits this transform.

---

## Cross-Cutting Observations

1. **Three independent encodings of "18"**: `PaddedChunkShape = ConstShape3u32<18,18,18>` (`types.rs:7`), `PADDED_CHUNK_SIZE = 18` (`types.rs:10`, unused), `PADDED_VOLUME = 18*18*18` (`palette.rs:6`). None derives from the others.

2. **One hardcoded derived literal**: `SurfaceHeightMap.heights: [Option<f64>; 256]` (`config.rs:43`) — `256 = 16²` not expressed in terms of `CHUNK_SIZE`.

3. **Compile-time assumption shared across crates**: Server and client both compile against `voxel_map_engine::types::CHUNK_SIZE = 16`. The network protocol never transmits chunk size. `MapTransitionStart` could carry it, but currently does not.

4. **`VoxelMapConfig` propagation path**: Server spawn → terrain def application → generator build (server only). Client receives fields via `MapTransitionStart`. Adding a field requires touching both the config constructor and the transition message.

5. **Padding convention**: Every padded-to-local conversion uses `+1` (or equivalently, world coord formula `chunk_pos * CHUNK_SIZE + padded_index - 1`). The relationship is always `padded = chunk + 2` (1 border on each side).

6. **No runtime chunk-size dispatch**: All coordinate math uses the compile-time `CHUNK_SIZE` constant directly. No function takes chunk size as a parameter. `VoxelWorld` queries config but doesn't read any size field from it.

## Open Areas

1. **`block_mesh` crate internals**: The greedy quads algorithm receives `&PaddedChunkShape {}` and bounds `[0;3]..[17;3]`. Whether `block_mesh` internally assumes cubic chunks or supports non-cubic shapes was not traced into the vendored crate.

2. **Const generic feasibility**: `PaddedChunkShape` is a type alias for `ConstShape3u32<18,18,18>`. Whether `ndshape::ConstShape3u32` can be parameterized by a const generic that propagates from `CHUNK_SIZE` depends on Rust const generics support for computed expressions in type positions — not verified.

3. **`SurfaceHeightMap` array size**: `[Option<f64>; 256]` is a fixed-size array. Changing this to `CHUNK_SIZE * CHUNK_SIZE` requires the array size to be a const expression, which is possible but would change the type signature.

---

## Addendum A: Static → Instance Linearize Dispatch

### The Problem

`PaddedChunkShape` is a ZST type alias (`types.rs:7`). All methods (`linearize`, `delinearize`, `SIZE`, `USIZE`) are resolved statically via `ndshape::ConstShape` trait — no instance needed. Converting to per-map chunk sizes means these become instance method calls on a runtime shape.

### Every Production Call Site

| File | Line | Method | Enclosing function | Has shape/config in scope? |
|---|---|---|---|---|
| `api.rs` | 44 | `linearize` | `VoxelWorld::get_voxel(&self, map, pos)` | No — query has `VoxelMapConfig` but no shape |
| `api.rs` | 124 | `linearize` | `lookup_voxel(voxel_pos, instance, generator, cache)` | No |
| `api.rs` | 155 | `linearize` | `lookup_voxel_in_chunk(voxels, voxel_pos, chunk_pos)` | No |
| `instance.rs` | 145 | `linearize` | `VoxelMapInstance::set_voxel(&mut self, world_pos, voxel)` | No — `self` has no shape field |
| `instance.rs` | 166, 178 | `linearize` | `VoxelMapInstance::update_neighbor_padding(...)` | No |
| `generation.rs` | 190 | `linearize` | `build_surface_height_map(chunk_pos, palette)` | No |
| `terrain.rs` | 235 | `delinearize` | `generate_heightmap_chunk(chunk_pos, seed, ...)` | No |
| `meshing.rs` | 74 | `delinearize` | `flat_terrain_voxels(chunk_pos)` | No |

### `::SIZE` / `::USIZE` Usage (allocation and loop bounds)

- `terrain.rs:231` — `vec![Air; PaddedChunkShape::SIZE as usize]` in `generate_heightmap_chunk`
- `meshing.rs:12` — `debug_assert_eq!(voxels.len(), PaddedChunkShape::USIZE)` in `mesh_chunk_greedy`
- `meshing.rs:73` — `for i in 0..PaddedChunkShape::SIZE` in `flat_terrain_voxels`

### Key Observation

**No production call site currently has a shape or config instance in scope.** Every function that calls `linearize`/`delinearize` would need a shape parameter threaded in. The `VoxelMapInstance` struct (which is already accessible in most paths) is the natural carrier — it is present at `instance.rs:145`, reachable via query in `api.rs:44`, and passed to `lookup_voxel` at `api.rs:124`.

### ndshape Runtime Shape

`ndshape` provides `RuntimeShape<u32, 3>` (`git/ndshape-rs/src/runtime_shape.rs:66-110`) which stores `array: [u32; 3]`, `strides: [u32; 3]`, `size: u32` as instance fields. It implements `Shape<3, Coord = u32>` directly (not via `ConstShape` blanket). Construction: `RuntimeShape::<u32, 3>::new([18, 18, 18])`.

---

## Addendum B: Hardcoded Meshing Bounds and Shape Dispatch

### Current State

**`mesh_chunk_greedy` signature** (`meshing.rs:11`):
```rust
pub fn mesh_chunk_greedy(voxels: &[WorldVoxel]) -> Option<Mesh>
```
No size parameter. Shape and bounds are entirely internal.

**`greedy_quads` call** (`meshing.rs:18-25`):
```rust
greedy_quads(
    voxels,
    &PaddedChunkShape {},
    [0; 3],          // min bounds
    [17; 3],         // max bounds (= PADDED_CHUNK_SIZE - 1)
    &faces,
    &mut buffer,
);
```

- `&PaddedChunkShape {}` — ZST literal, concrete type, resolved at compile time via monomorphization
- `[0; 3]` and `[17; 3]` — hardcoded bounds encoding the 18³ padded array dimensions
- `greedy_quads` iterates interior `[1..16]` for visible faces (padding never generates geometry)

### `greedy_quads` Trait Bound

**`git/block-mesh-rs/src/greedy.rs:64-73`**:
```rust
pub fn greedy_quads<T, S>(
    voxels: &[T],
    voxels_shape: &S,
    min: [u32; 3],
    max: [u32; 3],
    faces: &[OrientedBlockFace; 6],
    output: &mut GreedyQuadsBuffer,
) where
    T: MergeVoxel,
    S: Shape<3, Coord = u32>,
```

The only requirement on `S` is `Shape<3, Coord = u32>`.

### `Shape<3>` Trait Methods Required

**`git/ndshape-rs/src/lib.rs:101-114`**:
```rust
pub trait Shape<const N: usize> {
    type Coord;
    fn size(&self) -> Self::Coord;
    fn usize(&self) -> usize;
    fn as_array(&self) -> [Self::Coord; N];
    fn linearize(&self, p: [Self::Coord; N]) -> Self::Coord;
    fn delinearize(&self, i: Self::Coord) -> [Self::Coord; N];
}
```

Five methods. `greedy_quads` calls `linearize` in its inner loop (`greedy.rs:155-157, 177`) and `as_array`/`size` for bounds (`bounds.rs:11,16`).

### `RuntimeShape<u32, 3>` Compatibility

**`git/ndshape-rs/src/runtime_shape.rs:66-110`**: Already implements `Shape<3, Coord = u32>` with instance-stored strides. Can be passed directly to `greedy_quads` with no changes to `block_mesh`. Construction: `RuntimeShape::<u32, 3>::new([padded_size, padded_size, padded_size])`.

### What Changes

To make meshing work with variable chunk sizes, `mesh_chunk_greedy` needs:
1. A `Shape<3, Coord = u32>` implementor (or just the padded size as `u32`)
2. Max bounds computed as `[padded_size - 1; 3]` instead of `[17; 3]`
3. The voxels slice length assertion updated from `PaddedChunkShape::USIZE` to `shape.usize()`
