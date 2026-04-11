# Implementation Plan

## Overview

Thread per-map `chunk_size`, `padded_size`, `column_y_range`, and a `RuntimeShape<u32, 3>` instance through coordinate math, meshing, terrain generation, persistence, and networking — replacing the compile-time `CHUNK_SIZE` / `PaddedChunkShape` with runtime dispatch — then demonstrate by running two maps with different chunk sizes simultaneously.

**Deviation from design:** Design Decision #1 specifies `RuntimePow2Shape<u32, 3>`, but `RuntimePow2Shape::new` takes log2 of each dimension and **requires** dimensions to be powers of two. Padded size is `chunk_size + 2` (e.g. `18`, `34`) — never a power of two. The correct type is `RuntimeShape<u32, 3>` (research Addendum B), which takes actual dimensions and computes via stored strides. User approved this correction before plan was written. Design Decision #4 (powers-of-two `chunk_size`) still holds — it keeps `div_euclid` in coordinate math cheap.

---

## Phase 1: Config & Network Plumbing

Thread new fields end-to-end; no call site consumes them yet. Baseline `chunk_size=16` must remain fully working.

### Changes

#### 1. `crates/voxel_map_engine/src/types.rs`

**Action**: modify

- Delete dead `pub const PADDED_CHUNK_SIZE: u32 = 18;` (line 10). Keep `CHUNK_SIZE`, `PaddedChunkShape` — removed in Phase 6.
- Update doc comment on `PaddedChunkShape` to note it is transitional.

#### 2. `crates/voxel_map_engine/src/config.rs`

**Action**: modify

Add fields and validated constructor:

```rust
#[derive(Component)]
pub struct VoxelMapConfig {
    pub seed: u64,
    pub generation_version: u32,
    pub spawning_distance: u32,
    pub bounds: Option<IVec3>,
    pub tree_height: u32,
    pub save_dir: Option<PathBuf>,
    pub generates_chunks: bool,
    /// Edge length of a chunk in voxels. Power of two, >= 8.
    pub chunk_size: u32,
    /// `chunk_size + 2` — precomputed so consumers don't re-derive.
    pub padded_size: u32,
    /// Inclusive-exclusive Y chunk range for column expansion: `(y_min, y_max)`.
    pub column_y_range: (i32, i32),
}

impl VoxelMapConfig {
    pub fn new(
        seed: u64,
        generation_version: u32,
        spawning_distance: u32,
        bounds: Option<IVec3>,
        tree_height: u32,
        chunk_size: u32,
        column_y_range: (i32, i32),
    ) -> Self {
        debug_assert!(tree_height > 0, "VoxelMapConfig: tree_height must be > 0");
        debug_assert!(spawning_distance > 0, "VoxelMapConfig: spawning_distance must be > 0");
        debug_assert!(
            chunk_size.is_power_of_two() && chunk_size >= 8,
            "VoxelMapConfig: chunk_size must be a power of two >= 8, got {chunk_size}"
        );
        debug_assert!(
            column_y_range.0 < column_y_range.1,
            "VoxelMapConfig: column_y_range y_min must be < y_max"
        );
        if let Some(b) = bounds {
            debug_assert!(
                b.x > 0 && b.y > 0 && b.z > 0,
                "VoxelMapConfig: bounded maps must have all-positive bounds, got {b}"
            );
        }
        Self {
            seed,
            generation_version,
            spawning_distance,
            bounds,
            tree_height,
            save_dir: None,
            generates_chunks: true,
            chunk_size,
            padded_size: chunk_size + 2,
            column_y_range,
        }
    }
}
```

#### 3. `crates/voxel_map_engine/src/instance.rs`

**Action**: modify

Store shape + dims on the instance:

```rust
use ndshape::RuntimeShape;

#[derive(Component)]
pub struct VoxelMapInstance {
    pub tree: OctreeI32<Option<ChunkData>>,
    pub chunk_levels: HashMap<IVec2, u32>,
    pub dirty_chunks: HashSet<IVec3>,
    pub chunks_needing_remesh: HashSet<IVec3>,
    pub debug_colors: bool,
    pub chunk_size: u32,
    pub padded_size: u32,
    /// Padded chunk shape `[padded_size; 3]`. Cloned into async tasks.
    pub shape: RuntimeShape<u32, 3>,
}

impl VoxelMapInstance {
    pub fn new(tree_height: u32, chunk_size: u32) -> Self {
        debug_assert!(chunk_size.is_power_of_two() && chunk_size >= 8);
        let padded_size = chunk_size + 2;
        Self {
            tree: OctreeI32::new(tree_height as u8),
            chunk_levels: HashMap::new(),
            dirty_chunks: HashSet::new(),
            chunks_needing_remesh: HashSet::new(),
            debug_colors: false,
            chunk_size,
            padded_size,
            shape: RuntimeShape::<u32, 3>::new([padded_size, padded_size, padded_size]),
        }
    }

    pub fn overworld(seed: u64) -> (Self, VoxelMapConfig, Overworld) {
        let tree_height = 5;
        let chunk_size = 16;
        (
            Self::new(tree_height, chunk_size),
            VoxelMapConfig::new(seed, 0, 10, None, tree_height, chunk_size, (-8, 8)),
            Overworld,
        )
    }

    pub fn homebase(owner_id: u64, bounds: IVec3) -> (Self, VoxelMapConfig, Homebase) {
        let tree_height = 3;
        let chunk_size = 16;
        let spawning_distance = bounds_to_spawning_distance(bounds);
        (
            Self::new(tree_height, chunk_size),
            VoxelMapConfig::new(
                seed_from_id(owner_id),
                0,
                spawning_distance,
                Some(bounds),
                tree_height,
                chunk_size,
                (-8, 8),
            ),
            Homebase { owner: owner_id },
        )
    }

    pub fn arena(id: u64, seed: u64, bounds: IVec3) -> (Self, VoxelMapConfig, Arena) {
        let tree_height = 3;
        let chunk_size = 16;
        let spawning_distance = bounds_to_spawning_distance(bounds);
        (
            Self::new(tree_height, chunk_size),
            VoxelMapConfig::new(seed, 0, spawning_distance, Some(bounds), tree_height, chunk_size, (-8, 8)),
            Arena { id },
        )
    }
}
```

- The 3 `VoxelMapInstance::new(X)` call sites in `lifecycle.rs` tests and elsewhere become `VoxelMapInstance::new(X, 16)`. Update in-file tests to pass `16`.
- The `VoxelMapInstance::new` test assertions stay the same (no new behavior observable from outside — all new fields are plumbing).

#### 4. `crates/protocol/src/map/transition.rs`

**Action**: modify

```rust
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct MapTransitionStart {
    pub target: MapInstanceId,
    pub seed: u64,
    pub generation_version: u32,
    pub bounds: Option<IVec3>,
    pub spawn_position: Vec3,
    pub chunk_size: u32,
    pub column_y_range: (i32, i32),
}
```

#### 5. `crates/protocol/src/map/chunk.rs`

**Action**: modify

```rust
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
pub struct ChunkDataSync {
    pub map_id: MapInstanceId,
    pub chunk_pos: IVec3,
    pub chunk_size: u32,
    pub data: PalettedChunk,
}
```

#### 6. `crates/server/src/map.rs`

**Action**: modify

- `spawn_overworld` (line 109): `VoxelMapConfig::new(seed, generation_version, 2, None, 5, 16, (-8, 8))`; `VoxelMapInstance::new(5) → VoxelMapInstance::new(5, 16)`.
- `MapTransitionParams` struct (line 926): add `chunk_size: u32`, `column_y_range: (i32, i32)` fields.
- `ensure_map_exists` (line 1014) and `spawn_homebase` (line 1050): populate the two new params fields from `config.chunk_size`, `config.column_y_range`.
- `execute_server_transition` (line ~991): extend `MapTransitionStart { ... }` with `chunk_size: params.chunk_size, column_y_range: params.column_y_range`.
- `send_initial_chunks_for_map` / chunk-send path at line 813: extend `ChunkDataSync { ... }` with `chunk_size: instance.chunk_size` (or equivalent — pull from the `VoxelMapInstance` already in scope).
- `handle_map_switch_requests`: no logic change, just propagates through params.
- `spawn_homebase` uses `VoxelMapInstance::homebase`; no explicit chunk_size param here yet — baseline 16 carried through the bundle.

#### 7. `crates/client/src/map.rs`

**Action**: modify

- `spawn_overworld` (line 99): `VoxelMapConfig::new(0, 0, 2, None, 5, 16, (-8, 8))`; `VoxelMapInstance::new(5) → VoxelMapInstance::new(5, 16)`.
- `handle_map_transition_start` (line 395): read `transition.chunk_size` and `transition.column_y_range`, pass to `spawn_map_instance`.
- `spawn_map_instance` (line 499): add `chunk_size: u32, column_y_range: (i32, i32)` parameters; `VoxelMapConfig::new(..., chunk_size, column_y_range)`; `VoxelMapInstance::new(tree_height, chunk_size)`.
- `handle_chunk_data_sync` (line 133): on receive, `trace!("recv ChunkDataSync chunk_size={}", sync.chunk_size);` — **no validation yet** (that's Phase 5).
- After `handle_map_transition_start` inserts, add `trace!("recv MapTransitionStart target={:?} chunk_size={}", transition.target, transition.chunk_size);`.

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test -p voxel_map_engine` passes
- [x] `cargo test -p protocol` passes

#### Manual
- [ ] `cargo server` launches without panic
- [ ] `cargo client` connects, overworld streams chunks, player can walk around
- [ ] Client log shows `recv MapTransitionStart target=Overworld chunk_size=16` (or equivalent — may be gated on map switch, so trigger one via homebase)
- [ ] Walking produces no visual regressions vs. baseline

---

## Phase 2: Meshing Runtime Dispatch

Switch `mesh_chunk_greedy` to accept any `Shape<3, Coord = u32>`. Mesh-building async tasks capture a cloned shape. First subsystem fully off the compile-time shape.

### Changes

#### 1. `crates/voxel_map_engine/src/meshing.rs`

**Action**: modify

```rust
use ndshape::Shape;

pub fn mesh_chunk_greedy<S: Shape<3, Coord = u32>>(
    voxels: &[WorldVoxel],
    shape: &S,
) -> Option<Mesh> {
    debug_assert_eq!(voxels.len(), shape.usize());

    let mut buffer = GreedyQuadsBuffer::new(voxels.len());
    let faces = RIGHT_HANDED_Y_UP_CONFIG.faces;
    let dims = shape.as_array();
    let max = [dims[0] - 1, dims[1] - 1, dims[2] - 1];
    {
        let _span = info_span!("greedy_quads").entered();
        greedy_quads(voxels, shape, [0; 3], max, &faces, &mut buffer);
    }
    // ... rest unchanged
}

pub fn flat_terrain_voxels<S: Shape<3, Coord = u32>>(
    chunk_pos: IVec3,
    chunk_size: u32,
    shape: &S,
) -> Vec<WorldVoxel> {
    let mut voxels = vec![WorldVoxel::Air; shape.usize()];
    for i in 0..shape.size() {
        let [_x, y, _z] = shape.delinearize(i);
        let world_y = chunk_pos.y * chunk_size as i32 + y as i32 - 1;
        if world_y <= 0 {
            voxels[i as usize] = WorldVoxel::Solid(0);
        }
    }
    voxels
}
```

- Drop `use crate::types::{CHUNK_SIZE, PaddedChunkShape, ...};` — import only `WorldVoxel`.
- In-file tests: construct a local `let shape = RuntimeShape::<u32, 3>::new([18, 18, 18]);` and pass by reference, and pass `16` as `chunk_size` to `flat_terrain_voxels`.

#### 2. `crates/voxel_map_engine/src/lifecycle.rs`

**Action**: modify

- `spawn_remesh_tasks` (line 1021): before `pool.spawn(async move { mesh_chunk_greedy(&voxels) })`, clone `instance.shape` into a local, then `let shape = instance.shape.clone(); pool.spawn(async move { mesh_chunk_greedy(&voxels, &shape) })`.
- `generation.rs::spawn_mesh_task` (line 148): add `shape: RuntimeShape<u32, 3>` parameter; same `move` semantics.
- Call sites of `spawn_mesh_task` in `lifecycle.rs` (`drain_gen_queue` around line 758): pass `instance.shape.clone()`.
- `generation.rs::spawn_terrain_batch` disk-load fast path (line 67): uses `mesh_chunk_greedy(&voxels)` → needs shape. Thread a `shape: RuntimeShape<u32, 3>` parameter into `spawn_terrain_batch`. `drain_gen_queue` passes `instance.shape.clone()`.

#### 3. `crates/voxel_map_engine/src/generation.rs`

**Action**: modify

- `spawn_terrain_batch`, `spawn_mesh_task`: add `shape: RuntimeShape<u32, 3>` parameter; capture in `async move` block. Use `mesh_chunk_greedy(&voxels, &shape)` at disk-load fast path and in mesh stage.
- In-file tests that call `flat_terrain_voxels` / `mesh_chunk_greedy`: construct a local `RuntimeShape<u32, 3>::new([18, 18, 18])` and pass.

#### 4. Test files using `mesh_chunk_greedy` / `flat_terrain_voxels`

**Action**: modify — mechanical updates only

- `crates/voxel_map_engine/src/meshing.rs` tests: construct local shape.
- `crates/voxel_map_engine/examples/multi_instance.rs` lines 100, 116: replace `PaddedChunkShape::USIZE` with `shape.usize()`; replace `PaddedChunkShape::SIZE` with `shape.size()`; replace `PaddedChunkShape::delinearize(i)` with `shape.delinearize(i)`; construct `let shape = RuntimeShape::<u32, 3>::new([18, 18, 18]);` at top of each generator fn. Leave `CHUNK_SIZE` references as-is (they're from `types.rs`, still present until Phase 6).

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test -p voxel_map_engine` passes
- [x] No `PaddedChunkShape` references remain in `meshing.rs`

#### Manual
- [ ] `cargo server` + `cargo client` — overworld meshes render
- [ ] Visual spot check: corners, slopes, caves look identical to pre-Phase baseline screenshots
- [ ] Block break/place remeshes correctly

---

## Phase 3: API & Coordinate Math Dispatch

Route all coordinate math and linearize dispatch through `VoxelMapInstance.chunk_size` / `.shape`. Remove `DEFAULT_COLUMN_Y_MIN/MAX` const usage.

### Changes

#### 1. `crates/voxel_map_engine/src/api.rs`

**Action**: modify

```rust
pub fn voxel_to_chunk_pos(voxel_pos: IVec3, chunk_size: u32) -> IVec3 {
    let cs = chunk_size as i32;
    IVec3::new(
        voxel_pos.x.div_euclid(cs),
        voxel_pos.y.div_euclid(cs),
        voxel_pos.z.div_euclid(cs),
    )
}
```

- `VoxelWorld::get_voxel`: read `instance.chunk_size` / `instance.shape`, pass through to `voxel_to_chunk_pos(pos, instance.chunk_size)` and `instance.shape.linearize([..])`.
- `lookup_voxel` / `evaluate_voxel_at` / `lookup_voxel_in_chunk`: add `chunk_size: u32` and `shape: &impl Shape<3, Coord = u32>` parameters; threaded from the caller that already has the instance.
- `raycast` caller loop: hoist `let chunk_size = instance.chunk_size; let shape = instance.shape.clone();` before the closure captures.
- In-file test `voxel_to_chunk_pos_basic` now passes `16` explicitly. The lookup tests build a local `shape` for `lookup_voxel_in_chunk`.

Remove `use crate::types::{CHUNK_SIZE, PaddedChunkShape, ...};` and `use ndshape::ConstShape;`.

#### 2. `crates/voxel_map_engine/src/instance.rs`

**Action**: modify

- `set_voxel`: `let local = world_pos - chunk_pos * self.chunk_size as i32;` and `let index = self.shape.linearize(padded) as usize;`.
- `update_neighbor_padding`: replace `CHUNK_SIZE as i32` with `self.chunk_size as i32` and `PaddedChunkShape::linearize` with `self.shape.linearize`.
- `voxel_to_chunk_pos` call becomes `voxel_to_chunk_pos(world_pos, self.chunk_size)`.
- Tests: `VoxelMapInstance::new(5)` → `VoxelMapInstance::new(5, 16)`; allocation `vec![WorldVoxel::Air; PaddedChunkShape::USIZE]` stays (removed in Phase 6).
- Remove `use ndshape::ConstShape;` and `use crate::types::{CHUNK_SIZE, PaddedChunkShape, ...};` (keep `ChunkData`, `WorldVoxel`).

#### 3. `crates/voxel_map_engine/src/lifecycle.rs`

**Action**: modify

```rust
pub fn world_to_chunk_pos(translation: Vec3, chunk_size: u32) -> IVec3 {
    (translation / chunk_size as f32).floor().as_ivec3()
}

pub fn world_to_column_pos(translation: Vec3, chunk_size: u32) -> IVec2 {
    let chunk = world_to_chunk_pos(translation, chunk_size);
    IVec2::new(chunk.x, chunk.z)
}

fn chunk_world_offset(chunk_pos: IVec3, chunk_size: u32) -> Vec3 {
    chunk_pos.as_vec3() * chunk_size as f32 - Vec3::ONE
}
```

- `update_chunks`: replace `let y_min = DEFAULT_COLUMN_Y_MIN; let y_max = DEFAULT_COLUMN_Y_MAX;` with `let (y_min, y_max) = config.column_y_range;` inside the per-map loop.
- `collect_tickets` (~line 476): pass `config.chunk_size` (need to thread through — add `chunk_size` param to `collect_tickets` or read per-map inside). Simplest: pass the config from the per-map loop.
- Actually, `collect_tickets` receives `map_query` and does its own per-map lookup (it iterates tickets by ticket.map_entity). Add a `chunk_size` read from the query when calling `world_to_column_pos`: `let cs = instance_query_for(ticket.map_entity).chunk_size;`. Review the existing structure — add a helper closure or a second query pass.
- `remove_column_chunks`, `enqueue_new_chunks`: signature already takes `y_min, y_max`. Just forwarded from config now.
- `handle_completed_chunk` and `spawn_remesh_tasks`: replace `chunk_world_offset(result.position)` with `chunk_world_offset(result.position, instance.chunk_size)`.
- `despawn_out_of_range_chunks`: uses `chunk_to_column`, no `chunk_size` needed.
- Drop `use crate::ticket::{... DEFAULT_COLUMN_Y_MAX, DEFAULT_COLUMN_Y_MIN, ...};`.
- Drop `use crate::types::{CHUNK_SIZE, ...};` (keep `ChunkStatus`, `FillType`).

- Update in-file tests `world_to_chunk_pos_positive` etc. to pass `16` and assert existing values (no behavior change).

#### 4. `crates/voxel_map_engine/src/ticket.rs`

**Action**: modify

```rust
pub fn column_to_chunks(col: IVec2, y_range: (i32, i32)) -> impl Iterator<Item = IVec3> {
    (y_range.0..y_range.1).map(move |y| IVec3::new(col.x, y, col.y))
}
```

- Delete `pub const DEFAULT_COLUMN_Y_MIN: i32 = -8;` and `pub const DEFAULT_COLUMN_Y_MAX: i32 = 8;`.
- In-file tests update: `column_to_chunks(col, -2, 2)` → `column_to_chunks(col, (-2, 2))`.

#### 5. `crates/voxel_map_engine/src/placement.rs`

**Action**: modify

```rust
pub fn jittered_grid_sample(
    seed: u64,
    chunk_pos: IVec3,
    chunk_size: u32,
    min_spacing: f64,
    density: f64,
) -> Vec<Vec2> {
    ...
    let chunk_size_f = chunk_size as f64;
    let chunk_world_x = chunk_pos.x as f64 * chunk_size_f;
    ...
}
```

Remove `use crate::types::CHUNK_SIZE;`. Update in-file tests to pass `16`.

#### 6. `crates/voxel_map_engine/src/terrain.rs`

**Action**: modify (just the call sites touched this phase — full rewrite in Phase 4)

- `place_features` (line 378): `jittered_grid_sample(self.seed, chunk_pos, CHUNK_SIZE, rule.min_spacing, rule.density)`. The `CHUNK_SIZE` reference here stays until Phase 4, where `place_features` learns chunk_size from the generator context.

#### 7. `crates/client/src/map.rs`

**Action**: modify

- `handle_unload_column` (line 183): read `config.column_y_range` from the map entity and pass to `column_to_chunks(col, config.column_y_range)`. Needs a `&VoxelMapConfig` addition to the query.
- Drop `DEFAULT_COLUMN_Y_MAX, DEFAULT_COLUMN_Y_MIN` from imports.

#### 8. `crates/server/src/map.rs`

**Action**: modify

- `send_initial_chunks_for_map` loop (line 788) and `unload_stale_columns` (line 850): call `column_to_chunks(col, config.column_y_range)` — requires `&VoxelMapConfig` on the relevant per-map queries. The map_query in `update_chunks`-adjacent systems already has it; thread through.
- Drop `DEFAULT_COLUMN_Y_MIN/MAX` references.

#### 9. `crates/client/tests/chunk_sync.rs`

**Action**: modify

- Drop `DEFAULT_COLUMN_Y_MAX, DEFAULT_COLUMN_Y_MIN` imports; replace with inline `(-8, 8)` or pass from a test config.

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test -p voxel_map_engine` passes
- [x] `cargo test -p client` passes (chunk_sync test)
- [x] No remaining `DEFAULT_COLUMN_Y_MIN`/`DEFAULT_COLUMN_Y_MAX` references anywhere in the workspace (`Grep` check)
- [x] No `CHUNK_SIZE` references in `api.rs`, `instance.rs`, `lifecycle.rs`, `placement.rs`, `ticket.rs`

#### Manual
- [ ] `cargo server` + `cargo client` — overworld loads
- [ ] Walk across chunk boundary at `x=0` (negative → positive) — no visual or collision seams
- [ ] Walk down into Y range `-128..0` and back up — chunks stream correctly
- [ ] Break/place a block across chunk boundaries — neighbor padding propagates

---

## Phase 4: Terrain Generation & `SurfaceHeightMap`

Runtime-size the terrain fill loops, rewrite `SurfaceHeightMap` as boxed padded slice, pad the slope check.

### Changes

#### 1. `crates/voxel_map_engine/src/config.rs`

**Action**: modify `SurfaceHeightMap`

```rust
pub struct SurfaceHeightMap {
    pub chunk_pos: IVec3,
    pub padded_size: u32,
    pub heights: Box<[Option<f64>]>,
}

impl SurfaceHeightMap {
    pub fn new(chunk_pos: IVec3, padded_size: u32) -> Self {
        let len = (padded_size as usize) * (padded_size as usize);
        Self {
            chunk_pos,
            padded_size,
            heights: vec![None; len].into_boxed_slice(),
        }
    }

    /// Look up height by padded XZ coordinate (both in `0..padded_size`).
    pub fn at(&self, px: u32, pz: u32) -> Option<f64> {
        debug_assert!(px < self.padded_size && pz < self.padded_size);
        self.heights[(px * self.padded_size + pz) as usize]
    }

    pub fn set(&mut self, px: u32, pz: u32, h: Option<f64>) {
        debug_assert!(px < self.padded_size && pz < self.padded_size);
        self.heights[(px * self.padded_size + pz) as usize] = h;
    }
}
```

#### 2. `crates/voxel_map_engine/src/generation.rs`

**Action**: modify

```rust
pub fn build_surface_height_map<S: Shape<3, Coord = u32>>(
    chunk_pos: IVec3,
    palette: &PalettedChunk,
    chunk_size: u32,
    padded_size: u32,
    shape: &S,
) -> SurfaceHeightMap {
    let voxels = palette.to_voxels();
    let mut map = SurfaceHeightMap::new(chunk_pos, padded_size);

    for px in 0..padded_size {
        for pz in 0..padded_size {
            for py in (1..=chunk_size).rev() {
                let idx = shape.linearize([px, py, pz]) as usize;
                if matches!(voxels[idx], WorldVoxel::Solid(_)) {
                    let world_y = chunk_pos.y as f64 * chunk_size as f64 + (py - 1) as f64 + 1.0;
                    map.set(px, pz, Some(world_y));
                    break;
                }
            }
        }
    }
    map
}
```

Note: outer loop is now `0..padded_size` (full padded footprint, including the 1-voxel border), producing heights for the slope check at chunk edges.

- `generate_terrain` helper (line 204): unchanged signature; the vec-allocation lives inside `VoxelGeneratorImpl::generate_terrain` on the concrete generator.
- `spawn_features_task` / `spawn_terrain_batch` / `spawn_mesh_task`: already take `shape` from Phase 2. `spawn_features_task` now receives a `SurfaceHeightMap` built with the new signature — caller in `lifecycle.rs` passes `instance.chunk_size`, `instance.padded_size`, and `&instance.shape`.
- Tests: pass explicit `16`, `18`, and a local shape. `heights[(x * CHUNK_SIZE + z) as usize]` becomes `map.at(x, z)` or `map.at(x + 1, z + 1)` depending on padding. Assertions iterate `0..16` and read via `map.at(x + 1, z + 1)`.

#### 3. `crates/voxel_map_engine/src/terrain.rs`

**Action**: modify

- Delete `const PADDED_XZ: usize = 18;` and `const CACHE_LEN: usize = PADDED_XZ * PADDED_XZ;` (lines 204-205). Replace with runtime values computed from `padded_size`.
- `generate_heightmap_chunk`: add `chunk_size: u32, padded_size: u32, shape: &S` parameters, where `S: Shape<3, Coord = u32>`:

```rust
pub fn generate_heightmap_chunk<S: Shape<3, Coord = u32>>(
    chunk_pos: IVec3,
    seed: u64,
    height_map: &HeightMap,
    moisture_map: Option<&MoistureMap>,
    biome_rules: Option<&BiomeRules>,
    chunk_size: u32,
    padded_size: u32,
    shape: &S,
) -> Vec<WorldVoxel> {
    let cache_len = (padded_size as usize).pow(2);
    let height_cache = build_height_cache(chunk_pos, &*height_noise, height_map, chunk_size, padded_size);
    let moisture_cache = moisture_noise.as_ref().map(|noise| build_2d_cache(chunk_pos, &**noise, chunk_size, padded_size));

    let total = shape.usize();
    let mut voxels = vec![WorldVoxel::Air; total];
    for i in 0..(shape.size()) {
        let [px, py, pz] = shape.delinearize(i);
        let world_y = chunk_pos.y * chunk_size as i32 + py as i32 - 1;
        let terrain_height = height_cache[xz_index(px, pz, padded_size)];
        ...
    }
    voxels
}

fn xz_index(px: u32, pz: u32, padded_size: u32) -> usize {
    px as usize * padded_size as usize + pz as usize
}

fn build_height_cache(chunk_pos: IVec3, noise: &dyn NoiseFn<f64, 2>, height_map: &HeightMap, chunk_size: u32, padded_size: u32) -> Vec<f64> {
    let mut cache = vec![0.0; (padded_size as usize).pow(2)];
    for px in 0..padded_size {
        for pz in 0..padded_size {
            let world_x = chunk_pos.x * chunk_size as i32 + px as i32 - 1;
            let world_z = chunk_pos.z * chunk_size as i32 + pz as i32 - 1;
            let sample = noise.get([world_x as f64, world_z as f64]);
            cache[xz_index(px, pz, padded_size)] = height_map.base_height as f64 + sample * height_map.amplitude;
        }
    }
    cache
}
// build_2d_cache mirrors build_height_cache shape
```

- `HeightmapGenerator` needs `chunk_size` + `padded_size` + `shape`. Extend the struct:

```rust
struct HeightmapGenerator {
    seed: u64,
    height_map: HeightMap,
    moisture_map: Option<MoistureMap>,
    biome_rules: Option<BiomeRules>,
    placement_rules: Option<PlacementRules>,
    chunk_size: u32,
    padded_size: u32,
    shape: RuntimeShape<u32, 3>,
}
```

- `VoxelGeneratorImpl::generate_terrain` / `place_features` already take `&self`, so the generator reads its own `chunk_size`/`padded_size`/`shape`.
- `place_features` (lines 366-429):
  - `jittered_grid_sample(self.seed, chunk_pos, self.chunk_size, rule.min_spacing, rule.density)`.
  - `local_x = (world_pos.x - chunk_pos.x as f32 * self.chunk_size as f32).floor() as u32;` (same for Z).
  - Bounds check `local_x >= self.chunk_size`.
  - `heights.at(local_x + 1, local_z + 1)` (padded index).
  - Slope check: `if exceeds_slope(local_x + 1, local_z + 1, heights, slope_max, self.padded_size)` — now indexed in padded space.

- `exceeds_slope` (lines 437-458): take `padded_size: u32` parameter; delete the `nx < 0 || nx >= CHUNK_SIZE as i32` clip at line 447 — replace with `nx < 0 || nx >= padded_size as i32`. With padded lookup, valid `(x, z)` is `1..=chunk_size`, so neighbors at `0` and `chunk_size + 1` resolve to padded-border values (1-voxel neighbor from the adjacent chunk).

```rust
fn exceeds_slope(px: u32, pz: u32, heights: &SurfaceHeightMap, max_slope: f64, padded_size: u32) -> bool {
    let center = match heights.at(px, pz) {
        Some(h) => h,
        None => return false,
    };
    let neighbors: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
    for (dx, dz) in neighbors {
        let nx = px as i32 + dx;
        let nz = pz as i32 + dz;
        if nx < 0 || nx >= padded_size as i32 || nz < 0 || nz >= padded_size as i32 {
            continue;
        }
        if let Some(nh) = heights.at(nx as u32, nz as u32) {
            let slope = (nh - center).abs();
            if slope > max_slope {
                return true;
            }
        }
    }
    false
}
```

- `build_generator` (line 473): read `chunk_size`, `padded_size`, `shape` from `VoxelMapConfig` (pass the config into `build_generator` as a second arg, **OR** pass them as explicit u32/shape params from the caller). Simplest signature:

```rust
pub fn build_generator(entity: EntityRef, seed: u64, chunk_size: u32, padded_size: u32, shape: RuntimeShape<u32, 3>) -> VoxelGenerator {
    ...
    match height {
        Some(height_map) => VoxelGenerator(Arc::new(HeightmapGenerator {
            seed, height_map, moisture_map: moisture, biome_rules: biomes, placement_rules: placement,
            chunk_size, padded_size, shape,
        })),
        None => VoxelGenerator(Arc::new(FlatGenerator { chunk_size, shape: shape.clone() })),
    }
}
```

- `FlatGenerator` needs `chunk_size: u32` + `shape: RuntimeShape<u32, 3>` (to pass to `flat_terrain_voxels`):

```rust
pub struct FlatGenerator {
    pub chunk_size: u32,
    pub shape: RuntimeShape<u32, 3>,
}

impl VoxelGeneratorImpl for FlatGenerator {
    fn generate_terrain(&self, chunk_pos: IVec3) -> Vec<WorldVoxel> {
        flat_terrain_voxels(chunk_pos, self.chunk_size, &self.shape)
    }
}
```

- Update `FlatGenerator` usages in `client/src/map.rs` (lines 106, 494-496): construct with `FlatGenerator { chunk_size: 16, shape: RuntimeShape::<u32, 3>::new([18, 18, 18]) }`.

- Tests in `terrain.rs`: pass `16`, `18`, shape constructed locally.

#### 4. `crates/voxel_map_engine/src/lifecycle.rs`

**Action**: modify

- `drain_gen_queue` `ChunkStatus::Features` arm (line 740): `build_surface_height_map(work.position, &chunk_data.voxels, instance.chunk_size, instance.padded_size, &instance.shape)`.

#### 5. `crates/server/src/map.rs`

**Action**: modify

- `build_terrain_generators` (line 179): after reading seed, also capture `config.chunk_size`, `config.padded_size`, and the corresponding `instance.shape`. Pass to `build_generator`. **Note**: `instance` is queried separately — fetch it in the same pass.

```rust
let entities: Vec<(Entity, u64, u32, u32, RuntimeShape<u32, 3>)> = world
    .query_filtered::<(Entity, &VoxelMapConfig, &VoxelMapInstance), (With<TerrainDefApplied>, Without<VoxelGenerator>)>()
    .iter(world)
    .map(|(e, config, inst)| (e, config.seed, config.chunk_size, config.padded_size, inst.shape.clone()))
    .collect();

for (entity, seed, chunk_size, padded_size, shape) in entities {
    let entity_ref = world.entity(entity);
    let generator = build_generator(entity_ref, seed, chunk_size, padded_size, shape);
    world.entity_mut(entity).insert(generator);
}
```

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test -p voxel_map_engine` passes
- [x] No `CHUNK_SIZE` references in `terrain.rs`, `generation.rs`, `config.rs`
- [x] No `PaddedChunkShape` references in `terrain.rs`, `generation.rs`

#### Manual
- [ ] `cargo server` + `cargo client` — overworld generates terrain with baseline-like topology
- [ ] Trees and rocks spawn (placement works)
- [ ] Walk along chunk edges where slopes exist — features at edges no longer clip oddly compared to baseline
- [ ] Generate a fresh world (delete `assets/sprites/humanoid/worlds/overworld/`) — seeds and features are deterministic

---

## Phase 5: Persistence Format + Cross-Boundary Validation

Bump save version, embed `chunk_size` in the header, validate on load and on `ChunkDataSync` receipt.

### Changes

#### 1. `crates/voxel_map_engine/src/persistence.rs`

**Action**: modify

```rust
const CHUNK_SAVE_VERSION: u32 = 4;

#[derive(Serialize, Deserialize)]
struct ChunkFileEnvelope {
    version: u32,
    chunk_size: u32,
    data: ChunkData,
}

pub fn save_chunk(
    map_dir: &Path,
    chunk_pos: IVec3,
    chunk_size: u32,
    data: &ChunkData,
) -> Result<(), String> {
    ...
    let envelope = ChunkFileEnvelope {
        version: CHUNK_SAVE_VERSION,
        chunk_size,
        data: data.clone(),
    };
    ...
}

pub fn load_chunk(
    map_dir: &Path,
    chunk_pos: IVec3,
    expected_chunk_size: u32,
) -> Result<Option<ChunkData>, String> {
    ...
    if envelope.version != CHUNK_SAVE_VERSION {
        return Err(format!(
            "chunk version mismatch: expected {CHUNK_SAVE_VERSION}, got {}",
            envelope.version
        ));
    }
    if envelope.chunk_size != expected_chunk_size {
        return Err(format!(
            "chunk_size mismatch at {chunk_pos:?}: expected {expected_chunk_size}, got {}",
            envelope.chunk_size
        ));
    }
    Ok(Some(envelope.data))
}
```

- In-file tests: pass `16` to both `save_chunk` and `load_chunk`. Update the `chunk_data_zstd_compression_reduces_size` test to include the new field in the manually-constructed `ChunkFileEnvelope`.

#### 2. `crates/voxel_map_engine/src/generation.rs`

**Action**: modify

- `spawn_terrain_batch`: add `chunk_size: u32` to the existing signature (already has `shape` from Phase 2). Pass to `load_chunk(dir, pos, chunk_size)`.
- `spawn_features_task`: same — its `load_chunk_entities` path doesn't need chunk_size but if it ever reads chunk data, propagate.

#### 3. `crates/voxel_map_engine/src/lifecycle.rs`

**Action**: modify

- `spawn_terrain_batch` call sites in `drain_gen_queue` and disk-load path: pass `instance.chunk_size`.
- `drain_pending_saves` (line 552): the `save_chunk(&save.save_dir, save.position, &save.data)` call becomes `save_chunk(&save.save_dir, save.position, save.chunk_size, &save.data)`. Add `chunk_size: u32` field to `PendingSave` struct; populate from `instance.chunk_size` at enqueue time.

#### 4. `crates/server/src/map.rs`

**Action**: modify

- `save_dirty_chunks_sync` (line 281): `chunk_persist::save_chunk(map_dir, chunk_pos, instance.chunk_size, chunk_data)`.
- `enqueue_dirty_chunks` populates the new `chunk_size` field on `PendingSave` (per step 3 above).

#### 5. `crates/client/src/map.rs`

**Action**: modify

- `handle_chunk_data_sync` (line 133): validate `sync.chunk_size == instance.chunk_size`:

```rust
if sync.chunk_size != instance.chunk_size {
    error!(
        "ChunkDataSync chunk_size mismatch for {:?}: server={}, client={}",
        sync.map_id, sync.chunk_size, instance.chunk_size
    );
    continue;
}
```

- Drop the Phase 1 debug `trace!`.

#### 6. `crates/server/tests/voxel_persistence.rs`, `crates/server/tests/world_persistence.rs`

**Action**: modify

- All `save_chunk(...)` and `load_chunk(...)` calls: pass `16` as `chunk_size`.
- `PaddedChunkShape::USIZE` and `PaddedChunkShape::linearize` calls remain (removed in Phase 6).
- Add one new test in `voxel_persistence.rs`:

```rust
#[test]
fn load_chunk_with_mismatched_chunk_size_errors() {
    let dir = tempfile::tempdir().unwrap();
    let voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
    let chunk = ChunkData::from_voxels(&voxels, ChunkStatus::Full);
    save_chunk(dir.path(), IVec3::ZERO, 16, &chunk).unwrap();
    let err = load_chunk(dir.path(), IVec3::ZERO, 32);
    assert!(err.is_err(), "load with wrong chunk_size must error");
}
```

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test -p voxel_map_engine` passes
- [x] `cargo test -p server` passes (including the new mismatch test)

#### Manual
- [ ] Delete `assets/sprites/humanoid/worlds/overworld/terrain/`
- [ ] `cargo server` generates and saves fresh chunks
- [ ] Restart server, `cargo client` connects, saved chunks load without error
- [ ] Hand-edit a save file's `chunk_size` field (or use a hex editor to corrupt it) → server logs an error, falls back to regen, does not crash

---

## Phase 6: Remove `CHUNK_SIZE` Constant + Two-Map Demo

Delete the last compile-time chunk-size references; flip homebase to `chunk_size=32`; verify two maps with different sizes coexist.

### Changes

#### 1. `crates/voxel_map_engine/src/types.rs`

**Action**: modify

- Delete `pub type PaddedChunkShape = ndshape::ConstShape3u32<18, 18, 18>;`.
- Delete `pub const CHUNK_SIZE: u32 = 16;`.
- Delete the `padded_chunk_shape_size` and `padded_chunk_shape_linearize_roundtrip` tests.
- Other in-file tests that use `PaddedChunkShape::USIZE`: replace with a literal `5832` (== `18 * 18 * 18`) or construct a local shape.

#### 2. `crates/voxel_map_engine/src/palette.rs`

**Action**: modify

- Delete `const PADDED_VOLUME: usize = 18 * 18 * 18;` (line 6).
- `from_voxels`: drop the `assert_eq!(voxels.len(), PADDED_VOLUME, ...)` hard-coded length check; keep `voxels.len()` as the sole source of truth (stored as `len` inside the `Indirect` variant and reconstituted on `to_voxels`).
- `SingleValue::to_voxels`: need the length. Option: embed `len: usize` in `SingleValue { voxel, len }` OR make `to_voxels` take `len: usize` parameter. Pick the parameter form:

```rust
pub enum PalettedChunk {
    SingleValue(WorldVoxel),
    Indirect { palette: Vec<WorldVoxel>, data: Vec<u64>, bits_per_entry: u8, len: usize },
}

impl PalettedChunk {
    pub fn to_voxels(&self) -> Vec<WorldVoxel> {
        match self {
            Self::SingleValue(v) => {
                // Length unknown — callers that need a dense array must use `to_voxels_with_len`.
                panic!("to_voxels on SingleValue requires explicit length");
            }
            Self::Indirect { .. } => { ... }
        }
    }

    pub fn to_voxels_with_len(&self, len: usize) -> Vec<WorldVoxel> {
        match self {
            Self::SingleValue(v) => vec![*v; len],
            Self::Indirect { ... } => { ... as before ... }
        }
    }
}
```

Actually simpler: add a `len()` method that reads from `Indirect.len` for the `Indirect` case and requires a caller-provided fallback for `SingleValue`. Or store `len` on both variants:

```rust
pub enum PalettedChunk {
    SingleValue { voxel: WorldVoxel, len: usize },
    Indirect { palette: Vec<WorldVoxel>, data: Vec<u64>, bits_per_entry: u8, len: usize },
}
```

This second form is the minimal correct change — `len` is always knowable from the enum. Update `ChunkData::new_empty()` (in `types.rs`) to take a `len` parameter:

```rust
impl ChunkData {
    pub fn new_empty(padded_volume: usize) -> Self {
        Self {
            voxels: PalettedChunk::SingleValue { voxel: WorldVoxel::Air, len: padded_volume },
            fill_type: FillType::Empty,
            hash: 0,
            status: ChunkStatus::Full,
        }
    }
}
```

- All `ChunkData::new_empty()` call sites: pass `instance.padded_size.pow(3) as usize`. Only a handful of sites — each either has an `instance` in scope or hard-codes the value in a test.
- `PalettedChunk::get` / `set` for `SingleValue`: use the stored `len` for the bounds debug assert.
- `memory_usage()`, `is_uniform()`, `palette_size()`: unchanged pattern matches, just destructure the new shape.
- `bincode` serialization: adding a field to `SingleValue` is a format change, but Phase 5 already bumped `CHUNK_SAVE_VERSION`. Confirm serde still handles the enum — with `#[derive(Serialize, Deserialize)]`, variant tag + struct fields serialize cleanly.
- In-file tests: `PalettedChunk::SingleValue(x)` → `PalettedChunk::SingleValue { voxel: x, len: 5832 }`.

#### 3. `crates/voxel_map_engine/src/instance.rs`

**Action**: modify

- Drop `use ndshape::ConstShape;` and any residual `PaddedChunkShape` import.
- `set_voxel` and `update_neighbor_padding` already use `self.shape.linearize` from Phase 3 — nothing more to do.
- Tests that still use `PaddedChunkShape::USIZE`: replace with `(18usize).pow(3)` literal or with `instance.padded_size.pow(3) as usize`.
- `ChunkData::new_empty()` calls in tests: pass `5832`.

#### 4. `crates/voxel_map_engine/src/meshing.rs`, `generation.rs`, `terrain.rs`, `config.rs`

**Action**: modify

- Remove any residual imports from `types.rs` of `CHUNK_SIZE` or `PaddedChunkShape`.

#### 5. `crates/voxel_map_engine/examples/multi_instance.rs`

**Action**: modify

- Replace `CHUNK_SIZE as i32` usages with a local `const CHUNK_SIZE: i32 = 16;` at the top of each generator fn, or pass via `self.chunk_size`. Example still uses chunk_size=16 everywhere.

#### 6. `crates/voxel_map_engine/tests/api.rs`, `crates/server/tests/voxel_persistence.rs`, `crates/server/tests/world_persistence.rs`

**Action**: modify

- Replace `PaddedChunkShape::USIZE` with `5832usize` (the padded volume for chunk_size=16, which these tests use).
- Replace `PaddedChunkShape::linearize(padded)` with a local `RuntimeShape::<u32, 3>::new([18, 18, 18]).linearize(padded)`.

#### 7. `crates/server/src/map.rs` — Two-map demo

**Action**: modify

- `VoxelMapInstance::homebase` (in `instance.rs`): bump `chunk_size` from `16` to `32` and `column_y_range` from `(-8, 8)` to `(-4, 4)`:

```rust
pub fn homebase(owner_id: u64, bounds: IVec3) -> (Self, VoxelMapConfig, Homebase) {
    let tree_height = 3;
    let chunk_size = 32;
    let column_y_range = (-4, 4);
    let spawning_distance = bounds_to_spawning_distance(bounds);
    (
        Self::new(tree_height, chunk_size),
        VoxelMapConfig::new(
            seed_from_id(owner_id),
            0, spawning_distance, Some(bounds),
            tree_height, chunk_size, column_y_range,
        ),
        Homebase { owner: owner_id },
    )
}
```

- Client `generator_for_map` and `spawn_map_instance`: no change needed — they already accept chunk_size from `MapTransitionStart`.
- `client/src/map.rs::spawn_overworld` `FlatGenerator { chunk_size: 16, shape }` remains. Client `handle_map_transition_start` must construct `FlatGenerator { chunk_size: transition.chunk_size, shape: RuntimeShape::<u32, 3>::new([ps, ps, ps]) }` where `ps = transition.chunk_size + 2`. Update `generator_for_map` to take `chunk_size: u32`:

```rust
fn generator_for_map(map_id: &MapInstanceId, chunk_size: u32) -> VoxelGenerator {
    let padded = chunk_size + 2;
    let shape = RuntimeShape::<u32, 3>::new([padded, padded, padded]);
    match map_id {
        MapInstanceId::Overworld => VoxelGenerator(Arc::new(FlatGenerator { chunk_size, shape })),
        MapInstanceId::Homebase { .. } => VoxelGenerator(Arc::new(FlatGenerator { chunk_size, shape })),
    }
}
```

### Verification

#### Automated
- [ ] `cargo check-all` passes with **zero** `CHUNK_SIZE`, `PaddedChunkShape`, `PADDED_CHUNK_SIZE`, `PADDED_VOLUME`, `DEFAULT_COLUMN_Y_MIN`, `DEFAULT_COLUMN_Y_MAX` references in `crates/` (grep check)
- [ ] `cargo test -p voxel_map_engine` passes
- [ ] `cargo test -p server` passes
- [ ] `cargo test -p client` passes

#### Manual
- [ ] Delete `assets/sprites/humanoid/worlds/` fresh state
- [ ] `cargo server` + `cargo client` — overworld (chunk_size=16) loads and renders
- [ ] Transition to homebase (`/go home` or equivalent) — homebase loads with chunk_size=32
- [ ] Homebase chunks are visibly ~2× the size of overworld chunks (or check Tracy / debug render)
- [ ] No seams, no coordinate desync, no collision issues inside homebase
- [ ] Transition back to overworld — chunk streaming resumes correctly
- [ ] Player can block-edit in both maps
- [ ] Save homebase, restart server, reload — saved chunks load with chunk_size=32

---

## Deviations from Structure Outline

1. **`RuntimePow2Shape` → `RuntimeShape`** (documented at top of plan). Structure and design both reference `RuntimePow2Shape<u32, 3>`; this is incompatible with the `+2`-padded dimensions (18, 34 are not powers of two). `RuntimeShape<u32, 3>` is the correct type per research Addendum B. User approved before plan was written.

2. **`PalettedChunk::SingleValue` becomes a struct variant** (`{ voxel, len }`) rather than a tuple variant. Structure outline's Phase 6 says "palette.rs storage size derived from `PalettedChunk::Indirect.len` field", but `SingleValue` also needs a length to reconstitute a dense voxel array in `to_voxels`. Storing `len` on both variants is the cleanest fix.

3. **Phase 5 adds a `chunk_size` field to `PendingSave`** in `lifecycle.rs`. This wasn't explicitly in the structure outline but is required to propagate `chunk_size` into the async save task since `instance` is not moved into the async closure.

4. **Phase 4 adds `chunk_size`/`padded_size`/`shape` to `FlatGenerator`**. Structure outline doesn't mention `FlatGenerator`, but it implements `VoxelGeneratorImpl::generate_terrain` and calls `flat_terrain_voxels`, which needs `chunk_size` + `shape` after Phase 2.
