# Multi-Stage Generation Pipeline Implementation Plan

## Overview

Implement a multi-stage chunk generation pipeline (Empty → Terrain → Features → Mesh → Full) in `voxel_map_engine`, replacing the current single-pass architecture. The Features stage runs entity-only placement via `PlacementRules` and Poisson disk sampling. Per-chunk entity persistence saves/loads world objects with chunk lifecycle. The existing ticket level propagator gates maximum achievable `ChunkStatus` per chunk, creating concentric rings of partially-generated chunks that serve as neighbor data for closer chunks.

## Current State Analysis

**Single-pass pipeline** (`generation.rs:81-102`): `VoxelGenerator` closure produces 18³ voxels → `ChunkData::from_voxels` → `mesh_chunk_greedy` → `ChunkGenResult` with mesh + data. No intermediate stages, no status tracking.

**No feature placement**: `PlacementRules` designed in research but not implemented. Only `spawn_test_tree` (`gameplay.rs:237-260`) spawns one tree using asset-defined position at `OnEnter(AppState::Ready)`.

**Ticket levels already propagated**: `TicketLevelPropagator` computes per-column levels 0–20. Levels 3–20 are "Inaccessible" but loaded — the natural range for stage gating.

**World object system exists**: `WorldObjectId`, `WorldObjectDef`, `spawn_world_object`, `WorldObjectDefRegistry` — all functional. Reflect-based component bags from `.object.ron` files.

**Per-chunk terrain persistence exists** (`persistence.rs`): bincode + zstd + atomic write at `terrain/chunk_X_Y_Z.bin`. Entity persistence is flat `entities.bin` per map (no spatial partitioning).

### Key Discoveries:
- `ChunkData` (`types.rs:36-40`) has no `status` field
- `VoxelGenerator` (`config.rs:12-13`) is `Arc<dyn Fn(IVec3) -> Vec<WorldVoxel>>` — needs trait conversion
- `build_generator` (`terrain.rs:307-331`) reads `HeightMap`/`MoistureMap`/`BiomeRules` from entity, builds closure
- `enqueue_new_chunks` (`lifecycle.rs:580-609`) skips chunks where `get_chunk_data().is_some()` — must check status instead
- `drain_gen_queue` (`lifecycle.rs:615-694`) validates and batches positions — must check stage and neighbor readiness
- `handle_completed_chunk` (`lifecycle.rs:768-812`) spawns mesh entity when present, returns early otherwise — no stage gating, relies on result contents
- `PendingChunks.tasks` (`generation.rs:26-28`) is `Vec<Task<Vec<ChunkGenResult>>>` — works for all stages
- Lifecycle test (`tests/lifecycle.rs:22-26`) constructs `VoxelGenerator(Arc::new(flat_terrain_voxels))` directly — needs update
- `CHUNK_SAVE_VERSION = 2` (`persistence.rs:10`) — bump to 3 for status field
- `overworld.terrain.ron` uses reflect-based RON map format — `PlacementRules` follows same pattern

## Desired End State

Chunks generate through discrete stages gated by ticket level. The Features stage runs Poisson disk placement using `PlacementRules` from `.terrain.ron`, producing `WorldObjectSpawn` entries. Entities spawn server-side via `spawn_world_object`, tagged with `ChunkEntityRef`. On chunk eviction, entities are serialized to `entities/chunk_X_Y_Z.entities.bin`. On reload, entities load from disk (no re-generation). `spawn_test_tree` is removed; trees appear procedurally.

### Verification:
1. `cargo check-all` passes
2. `cargo test -p voxel_map_engine` passes (updated + new tests)
3. `cargo server` — chunks generate in stages (visible in Tracy: separate terrain_gen, features_gen, mesh_gen spans)
4. `cargo server` + `cargo client -c 1` — trees appear procedurally on overworld terrain
5. Server restart — trees persist (no duplicates, destroyed trees stay gone)
6. Walking to new areas — trees appear as chunks generate; walking away and back — same trees from disk

## What We're NOT Doing

- Voxel-modifying features (Option C — caves, ore veins). Trait has upgrade path but no implementation.
- `NeighborAccess` construction. Entity-only placement doesn't need cross-chunk voxel reads. Infrastructure described in research; deferred until voxel-modifying features.
- Structure starts/references system. No multi-chunk structures.
- Light stage. Research resolved: omit, add later if needed.
- Client-side entity prediction. Server-authoritative, clients wait for Lightyear replication.
- `LoadState`-driven entity ticking (freeze at BlockTicking). Deferred to separate plan.

## Implementation Approach

Six phases, each independently compilable and testable. Each phase builds on the previous. Phases 1-3 change pipeline structure without adding features. Phase 4 adds placement. Phase 5 adds persistence. Phase 6 wires everything together with data and cleanup.

---

## Phase 1: ChunkStatus and ChunkData Extension

### Overview
Add `ChunkStatus` enum. Extend `ChunkData` with a `status` field. Bump persistence version. All existing chunks default to `ChunkStatus::Full` so current behavior is preserved.

### Changes Required:

#### 1. ChunkStatus enum
**File**: `crates/voxel_map_engine/src/types.rs`
**Changes**: Add enum after `FillType`

```rust
/// Generation stage of a chunk in the multi-stage pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Reflect)]
pub enum ChunkStatus {
    Empty = 0,
    Terrain = 1,
    Features = 2,
    Mesh = 3,
    Full = 4,
}

impl ChunkStatus {
    /// Returns the next stage, or `None` if already at `Full`.
    pub fn next(self) -> Option<ChunkStatus> {
        match self {
            Self::Empty => Some(Self::Terrain),
            Self::Terrain => Some(Self::Features),
            Self::Features => Some(Self::Mesh),
            Self::Mesh => Some(Self::Full),
            Self::Full => None,
        }
    }

}
```

#### 2. ChunkData extension
**File**: `crates/voxel_map_engine/src/types.rs`
**Changes**: Add `status` field to `ChunkData`

```rust
pub struct ChunkData {
    pub voxels: PalettedChunk,
    pub fill_type: FillType,
    pub hash: u64,
    pub status: ChunkStatus,
}
```

Update `ChunkData::new_empty`. Status is `Full` because this constructor creates a fully-resolved all-air chunk (used by tests and the API layer for explicit voxel edits), not a chunk entering the generation pipeline:
```rust
pub fn new_empty() -> Self {
    Self {
        voxels: PalettedChunk::SingleValue(WorldVoxel::Air),
        fill_type: FillType::Empty,
        hash: 0,
        status: ChunkStatus::Full,
    }
}
```

Update `ChunkData::from_voxels` — accept a status parameter:
```rust
pub fn from_voxels(voxels: &[WorldVoxel], status: ChunkStatus) -> Self {
    let fill_type = classify_fill_type(voxels);
    let hash = compute_chunk_hash(voxels);
    let palettized = PalettedChunk::from_voxels(voxels);
    Self {
        voxels: palettized,
        fill_type,
        hash,
        status,
    }
}
```

#### 3. Persistence version bump
**File**: `crates/voxel_map_engine/src/persistence.rs`
**Changes**: `CHUNK_SAVE_VERSION = 2` → `3`. No migration needed (fresh world per research decision).

#### 4. Update all `ChunkData::from_voxels` call sites
Pass `ChunkStatus::Full` at existing call sites to preserve current behavior:
- `generation.rs:88` — `ChunkData::from_voxels(&voxels, ChunkStatus::Full)`
- Any test files constructing `ChunkData`

#### 5. Update tests
**Files**: `types.rs` tests, `persistence.rs` tests, `tests/lifecycle.rs`
- All `ChunkData::from_voxels` calls gain status parameter
- Add tests for `ChunkStatus::next()` and `ChunkStatus` ordering

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all`
- [x] `cargo test -p voxel_map_engine`

#### Manual Verification:
- [ ] `cargo server` — chunks still generate and display correctly

---

## Phase 2: VoxelGenerator Trait Conversion

### Overview
Convert `VoxelGenerator` from an `Arc<dyn Fn>` closure to an `Arc<dyn VoxelGeneratorTrait>` trait object. Add `generate_terrain` and `place_features` methods. `build_generator` returns a struct implementing the trait. All existing behavior preserved.

### Changes Required:

#### 1. Define trait
**File**: `crates/voxel_map_engine/src/config.rs`
**Changes**: Replace closure type with trait

```rust
use crate::types::WorldVoxel;

/// Trait for multi-stage chunk generation.
///
/// Implementors produce terrain voxels and optionally place entity-based features.
/// Each method corresponds to a pipeline stage.
pub trait VoxelGeneratorImpl: Send + Sync {
    /// Stage 1: Base terrain shape. Returns 18³ padded voxel array.
    fn generate_terrain(&self, chunk_pos: IVec3) -> Vec<WorldVoxel>;

    /// Stage 2: Entity placement on terrain surface.
    /// Receives a 16×16 surface height map (not raw voxels). Default: no features.
    fn place_features(&self, _chunk_pos: IVec3, _heights: &SurfaceHeightMap) -> Vec<WorldObjectSpawn> {
        Vec::new()
    }
}

/// Spawn data for a world object placed during the Features stage.
///
/// Uses bare `String` for `object_id` (not `WorldObjectId`) because `WorldObjectId`
/// lives in the `protocol` crate, and `voxel_map_engine` must not depend on it.
/// The server spawn system converts to `WorldObjectId` at the boundary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldObjectSpawn {
    pub object_id: String,
    pub position: Vec3,
}

/// 16×16 surface height map built from PalettedChunk on the main thread.
/// `heights[x * 16 + z]` = world Y of highest solid voxel, or `None` if all air.
pub struct SurfaceHeightMap {
    pub chunk_pos: IVec3,
    pub heights: [Option<f64>; 256],
}

/// The chunk generation implementation for a map instance.
///
/// Separate component from `VoxelMapConfig` so maps can exist without a
/// generator while terrain components are being applied (deferred commands).
#[derive(Component, Clone)]
pub struct VoxelGenerator(pub Arc<dyn VoxelGeneratorImpl>);
```

Add necessary imports (`serde`, `Vec3`, `Arc`).

#### 2. Update HeightmapGenerator
**File**: `crates/voxel_map_engine/src/terrain.rs`
**Changes**: Replace closure with struct + trait impl

```rust
/// Terrain generator using 2D heightmap noise with biome-aware material selection.
struct HeightmapGenerator {
    seed: u64,
    height_map: HeightMap,
    moisture_map: Option<MoistureMap>,
    biome_rules: Option<BiomeRules>,
}

impl VoxelGeneratorImpl for HeightmapGenerator {
    fn generate_terrain(&self, chunk_pos: IVec3) -> Vec<WorldVoxel> {
        generate_heightmap_chunk(
            chunk_pos,
            self.seed,
            &self.height_map,
            self.moisture_map.as_ref(),
            self.biome_rules.as_ref(),
        )
    }
}

/// Flat terrain generator (no noise).
struct FlatGenerator;

impl VoxelGeneratorImpl for FlatGenerator {
    fn generate_terrain(&self, chunk_pos: IVec3) -> Vec<WorldVoxel> {
        flat_terrain_voxels(chunk_pos)
    }
}
```

Update `build_generator`:
```rust
pub fn build_generator(entity: EntityRef, seed: u64) -> VoxelGenerator {
    let height = entity.get::<HeightMap>().cloned();
    let moisture = entity.get::<MoistureMap>().cloned();
    let biomes = entity.get::<BiomeRules>().cloned();
    // ... existing debug_asserts ...

    match height {
        Some(height_map) => VoxelGenerator(Arc::new(HeightmapGenerator {
            seed,
            height_map,
            moisture_map: moisture,
            biome_rules: biomes,
        })),
        None => VoxelGenerator(Arc::new(FlatGenerator)),
    }
}
```

#### 3. Update generation.rs
**File**: `crates/voxel_map_engine/src/generation.rs`
**Changes**: Use trait method instead of calling closure directly

```rust
fn generate_chunk(position: IVec3, generator: &dyn VoxelGeneratorImpl) -> ChunkGenResult {
    let voxels = {
        let _span = info_span!("terrain_gen").entered();
        generator.generate_terrain(position)
    };
    // ... rest unchanged, passing ChunkStatus::Full for now
}
```

Update `spawn_chunk_gen_batch` signature: `generator: &VoxelGenerator` → clone `Arc` as before, but call `generator.0.generate_terrain(pos)` inside async block via the trait.

#### 4. Update lifecycle test
**File**: `crates/voxel_map_engine/tests/lifecycle.rs`
**Changes**: `VoxelGenerator(Arc::new(flat_terrain_voxels))` → `VoxelGenerator(Arc::new(FlatGenerator))`.

Export `FlatGenerator` from `terrain.rs` (make `pub`). Note: `flat_terrain_voxels` is defined in `meshing.rs` — `FlatGenerator` in `terrain.rs` wraps it.

#### 5. Export from prelude
**File**: `crates/voxel_map_engine/src/lib.rs`
**Changes**: Ensure `VoxelGeneratorImpl`, `WorldObjectSpawn`, `FlatGenerator` are accessible.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all`
- [ ] `cargo test -p voxel_map_engine`

#### Manual Verification:
- [ ] `cargo server` — identical behavior to before

---

## Phase 3: Multi-Stage Scheduler

### Overview
The scheduler now advances chunks one stage at a time. `drain_gen_queue` checks current status vs. level-gated target status, spawns stage-appropriate async tasks. `handle_completed_chunk` re-enqueues chunks that haven't reached their target. Mesh entity spawn only happens at the Mesh stage.

This phase does not add feature placement — `place_features` returns empty. The observable change is that chunks now pass through Terrain → Features → Mesh → Full in separate async tasks, visible in Tracy profiling.

### Changes Required:

#### 1. Level-to-max-status mapping
**File**: `crates/voxel_map_engine/src/types.rs`
**Changes**: Add mapping function

```rust
impl ChunkStatus {
    /// Maximum achievable status for a chunk at the given effective level.
    /// Levels 0-2 (EntityTicking/BlockTicking/Border) → Full
    /// Level 3 → Mesh, Level 4 → Features, Level 5+ → Terrain
    pub fn max_for_level(effective_level: u32) -> ChunkStatus {
        match effective_level {
            0..=2 => ChunkStatus::Full,
            3 => ChunkStatus::Mesh,
            4 => ChunkStatus::Features,
            _ => ChunkStatus::Terrain,
        }
    }
}
```

#### 2. Stage-specific task functions
**File**: `crates/voxel_map_engine/src/generation.rs`
**Changes**: Add terrain-only and mesh-only task spawners alongside the existing batch function.

Rename `spawn_chunk_gen_batch` → `spawn_terrain_batch` (generates terrain only, sets status=Terrain, no mesh):

```rust
pub fn spawn_terrain_batch(
    pending: &mut PendingChunks,
    positions: Vec<IVec3>,
    generator: &VoxelGenerator,
    save_dir: Option<PathBuf>,
) {
    let gen = Arc::clone(&generator.0);
    let pool = AsyncComputeTaskPool::get();
    let task = pool.spawn(async move {
        let _span = info_span!("terrain_batch", count = positions.len()).entered();
        positions.into_iter().map(|pos| {
            // Try disk load first (returns Full status chunk)
            if let Some(ref dir) = save_dir {
                match crate::persistence::load_chunk(dir, pos) {
                    Ok(Some(chunk_data)) => {
                        let mesh = if chunk_data.fill_type == FillType::Empty {
                            None
                        } else {
                            let voxels = chunk_data.voxels.to_voxels();
                            mesh_chunk_greedy(&voxels)
                        };
                        return ChunkGenResult {
                            position: pos, mesh, chunk_data,
                            entity_spawns: vec![], from_disk: true,
                        };
                    }
                    Ok(None) => {}
                    Err(e) => bevy::log::warn!("Failed to load chunk at {pos}: {e}, regenerating"),
                }
            }
            // Generate terrain only
            let voxels = {
                let _span = info_span!("terrain_gen").entered();
                gen.generate_terrain(pos)
            };
            let chunk_data = {
                let _span = info_span!("palettize_chunk").entered();
                ChunkData::from_voxels(&voxels, ChunkStatus::Terrain)
            };
            ChunkGenResult {
                position: pos, mesh: None, chunk_data,
                entity_spawns: vec![], from_disk: false,
            }
        }).collect()
    });
    pending.tasks.push(task);
}
```

Note: disk-loaded chunks come back at whatever status they were saved at (Full for existing worlds). They include a mesh if non-empty. This preserves the fast path for cached chunks.

Features stage does not modify voxels, so it should not re-palettize. Instead, build a compact 16×16 surface height map on the main thread from `PalettedChunk`, pass it (not the full 18³ array) to the async task. `handle_completed_chunk` updates `chunk_data.status` to `Features` in-place via `get_chunk_data_mut`.

```rust
/// Spawn a features task for a single chunk.
///
/// Builds a surface height map on the main thread, passes it to the async task.
/// Does NOT expand or re-palettize voxels — status is updated in-place on completion.
pub fn spawn_features_task(
    pending: &mut PendingChunks,
    position: IVec3,
    height_map: SurfaceHeightMap,
    generator: &VoxelGenerator,
) {
    let gen = Arc::clone(&generator.0);
    let pool = AsyncComputeTaskPool::get();
    let task = pool.spawn(async move {
        let _span = info_span!("features_gen").entered();
        let entity_spawns = gen.place_features(position, &height_map);
        vec![ChunkGenResult {
            position, mesh: None, chunk_data: None,
            entity_spawns, from_disk: false,
        }]
    });
    pending.tasks.push(task);
}

/// Spawn a mesh task for a single chunk.
pub fn spawn_mesh_task(
    pending: &mut PendingChunks,
    position: IVec3,
    voxels: Vec<WorldVoxel>,
) {
    let pool = AsyncComputeTaskPool::get();
    let task = pool.spawn(async move {
        let _span = info_span!("mesh_gen").entered();
        let mesh = mesh_chunk_greedy(&voxels);
        let chunk_data = ChunkData::from_voxels(&voxels, ChunkStatus::Mesh);
        vec![ChunkGenResult {
            position, mesh, chunk_data,
            entity_spawns: vec![], from_disk: false,
        }]
    });
    pending.tasks.push(task);
}
```

#### 3. ChunkGenResult extension
**File**: `crates/voxel_map_engine/src/generation.rs`
**Changes**: Add `entity_spawns` field (new), make `chunk_data` optional (for Features stage which updates status in-place). Existing fields `position`, `mesh`, `from_disk` unchanged.

```rust
pub struct ChunkGenResult {
    pub position: IVec3,
    pub mesh: Option<Mesh>,
    /// `None` for Features stage (status updated in-place). `Some` for Terrain/Mesh stages.
    pub chunk_data: Option<ChunkData>,
    /// New: entity spawn data from Features stage.
    pub entity_spawns: Vec<WorldObjectSpawn>,
    pub from_disk: bool,
}
```

#### 4. Update `enqueue_new_chunks`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`
**Changes**: Check status vs. max status instead of `is_some()`

```rust
fn enqueue_new_chunks(...) {
    for &(col, level) in diff.loaded.iter().chain(diff.changed.iter()) {
        let distance = propagator.min_distance_to_source(col);
        let max_status = ChunkStatus::max_for_level(level);
        for chunk_pos in column_to_chunks(col, y_min, y_max) {
            if !is_within_bounds(chunk_pos, bounds) {
                trace!("enqueue_new_chunks: {chunk_pos} out of bounds, skipping");
                continue;
            }
            let current = instance.get_chunk_data(chunk_pos)
                .map(|c| c.status)
                .unwrap_or(ChunkStatus::Empty);
            if current >= max_status {
                trace!("enqueue_new_chunks: {chunk_pos} already at {current:?} >= {max_status:?}");
                continue;
            }
            gen_queue.heap.push(ChunkWork {
                position: chunk_pos,
                effective_level: level,
                distance_to_source: distance,
            });
        }
    }
}
```

#### 5. Update `drain_gen_queue`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`
**Changes**: Stage-aware validation and task spawning

Replace the current validation block and task spawning. Key logic:

```rust
fn drain_gen_queue(
    instance: &VoxelMapInstance,
    pending: &mut PendingChunks,
    gen_queue: &mut GenQueue,
    tracker: &mut ChunkWorkTracker,
    config: &VoxelMapConfig,
    generator: &VoxelGenerator,
    budget: &ChunkWorkBudget,
) {
    // ... existing budget/cap checks ...

    let mut terrain_batch = Vec::with_capacity(GEN_BATCH_SIZE);

    while let Some(work) = gen_queue.heap.pop() {
        // ... existing cap checks ...

        let col = chunk_to_column(work.position);
        if !instance.chunk_levels.contains_key(&col) { stale += 1; continue; }
        if tracker.generating.contains(&work.position)
            || tracker.remeshing.contains(&work.position) { stale += 1; continue; }
        if !is_within_bounds(work.position, config.bounds) { stale += 1; continue; }

        let current_status = instance.get_chunk_data(work.position)
            .map(|c| c.status)
            .unwrap_or(ChunkStatus::Empty);
        let max_status = ChunkStatus::max_for_level(work.effective_level);
        let Some(next_stage) = current_status.next() else { stale += 1; continue; };
        if next_stage > max_status { stale += 1; continue; }

        // NOTE: bare continues in this loop are intentional — the `stale` counter
        // (plotted via Tracy) provides aggregate telemetry. Per-entry trace! would
        // fire thousands of times per frame in the common case (lazy deletion).

        tracker.generating.insert(work.position);
        spawned += 1;

        match next_stage {
            ChunkStatus::Terrain => {
                terrain_batch.push(work.position);
                if terrain_batch.len() >= GEN_BATCH_SIZE {
                    spawn_terrain_batch(
                        pending, std::mem::take(&mut terrain_batch),
                        generator, config.save_dir.clone(),
                    );
                    terrain_batch = Vec::with_capacity(GEN_BATCH_SIZE);
                }
            }
            ChunkStatus::Features => {
                let chunk_data = instance.get_chunk_data(work.position)
                    .expect("chunk must exist at Terrain status");
                let height_map = build_surface_height_map(work.position, &chunk_data.voxels);
                spawn_features_task(pending, work.position, height_map, generator);
            }
            ChunkStatus::Mesh => {
                let voxels = instance.get_chunk_data(work.position)
                    .expect("chunk must exist at Features status").voxels.to_voxels();
                spawn_mesh_task(pending, work.position, voxels);
            }
            ChunkStatus::Full => {
                // Full is a promotion, no async work needed.
                // Handled inline: just update status in octree.
                if let Some(chunk_data) = instance.get_chunk_data_mut(work.position) {
                    chunk_data.status = ChunkStatus::Full;
                }
                tracker.generating.remove(&work.position);
                spawned -= 1; // Not actually an async task
            }
            ChunkStatus::Empty => unreachable!("next() never returns Empty"),
        }
    }
    // Flush remaining terrain batch
    if !terrain_batch.is_empty() {
        spawn_terrain_batch(pending, terrain_batch, generator, config.save_dir.clone());
    }
    // ... existing stop reason plots ...
}
```

`Full` promotion is synchronous — handled inline in `drain_gen_queue`. Update function signature to `&mut VoxelMapInstance`. `Full` is retained as the terminal state (distinct from `Mesh`) so that future systems can distinguish "meshed but pipeline incomplete" from "fully ready for gameplay" — e.g., `LoadState`-driven entity ticking checks `>= Full`.

#### 6. Update `handle_completed_chunk`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`
**Changes**: Re-enqueue for next stage, conditional mesh spawn, entity spawn queue

```rust
fn handle_completed_chunk(
    commands: &mut Commands,
    instance: &mut VoxelMapInstance,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    default_material: &DefaultVoxelMaterial,
    map_entity: Entity,
    result: ChunkGenResult,
    gen_queue: &mut GenQueue,
    propagator: &TicketLevelPropagator,
    pending_entity_spawns: &mut PendingEntitySpawns,
) {
    // Insert chunk data (Terrain/Mesh stages) or update status in-place (Features stage)
    let completed_status = if let Some(chunk_data) = result.chunk_data {
        let status = chunk_data.status;
        if !result.from_disk {
            instance.dirty_chunks.insert(result.position);
        }
        instance.insert_chunk_data(result.position, chunk_data);
        status
    } else {
        // Features stage: update status in-place, no re-palettization
        let data = instance.get_chunk_data_mut(result.position)
            .expect("chunk must exist for Features status update");
        data.status = ChunkStatus::Features;
        if !result.from_disk {
            instance.dirty_chunks.insert(result.position);
        }
        ChunkStatus::Features
    };

    // Queue entity spawns from Features stage
    if !result.entity_spawns.is_empty() {
        pending_entity_spawns.0.push((result.position, result.entity_spawns));
    }

    // Spawn mesh entity only at Mesh stage (or from disk-loaded Full chunks)
    if let Some(mesh) = result.mesh {
        // ... existing mesh entity spawn code ...
    }

    // Re-enqueue if chunk can advance further
    let col = chunk_to_column(result.position);
    if let Some(&level) = instance.chunk_levels.get(&col) {
        let max_status = ChunkStatus::max_for_level(level);
        if completed_status < max_status {
            gen_queue.heap.push(ChunkWork {
                position: result.position,
                effective_level: level,
                distance_to_source: propagator.min_distance_to_source(col),
            });
        }
    }
}
```

#### 7. PendingEntitySpawns component
**File**: `crates/voxel_map_engine/src/generation.rs` (or new `placement.rs`)
**Changes**: Add component for cross-crate entity spawn communication

```rust
/// Queued entity spawns from completed Features stages, awaiting server-side processing.
#[derive(Component, Default)]
pub struct PendingEntitySpawns(pub Vec<(IVec3, Vec<WorldObjectSpawn>)>);
```

Add to `ensure_pending_chunks` auto-insertion.

#### 8. Update `poll_chunk_tasks` and `update_chunks` signatures
Pass `gen_queue`, `propagator`, and `pending_entity_spawns` through to `handle_completed_chunk`. These components are already queried in `update_chunks` / `poll_chunk_tasks` parent queries — add `PendingEntitySpawns` to the query tuple in `poll_chunk_tasks`.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all`
- [ ] `cargo test -p voxel_map_engine`

#### Manual Verification:
- [ ] `cargo server` — chunks generate and display correctly
- [ ] Tracy shows separate `terrain_gen`, `features_gen`, `mesh_gen` spans per chunk
- [ ] No visual regression — terrain and meshes identical to before

---

## Phase 4: PlacementRules, Poisson Disk, and Features Stage

### Overview
Define `PlacementRules`/`PlacementRule` types. Implement Poisson disk sampling. Wire `place_features` into the `HeightmapGenerator`. Trees and objects spawn procedurally during the Features stage.

### Changes Required:

#### 1. PlacementRules types
**File**: `crates/voxel_map_engine/src/terrain.rs`
**Changes**: Add after `BiomeRules`

```rust
/// Rules for procedural world object placement on terrain.
#[derive(Component, Clone, Debug, Serialize, Deserialize, Reflect)]
#[reflect(Component, Serialize, Deserialize)]
pub struct PlacementRules(pub Vec<PlacementRule>);

/// A single placement rule for one type of world object.
#[derive(Clone, Debug, Serialize, Deserialize, Reflect)]
pub struct PlacementRule {
    /// WorldObjectId string (matches `.object.ron` filename stem).
    pub object_id: String,
    /// Biome IDs where this object can spawn. Empty = all biomes.
    pub allowed_biomes: Vec<String>,
    /// Average objects per chunk (before filtering). Controls Poisson disk candidate count.
    pub density: f64,
    /// Minimum distance between objects of this type within a chunk.
    pub min_spacing: f64,
    /// Maximum terrain slope (rise/run) for placement. `None` = no slope filter.
    pub slope_max: Option<f64>,
    /// Height range (world Y) where this object can spawn.
    pub height_range: (f64, f64),
}
```

Register in `VoxelPlugin::build`:
```rust
app.register_type::<terrain::PlacementRules>();
app.register_type::<terrain::PlacementRule>();
```

#### 2. Poisson disk sampling
**File**: `crates/voxel_map_engine/src/placement.rs` (new)
**Changes**: Implement 2D Poisson disk sampling per chunk

```rust
use bevy::prelude::*;
use crate::types::CHUNK_SIZE;

/// 2D Poisson disk sampling within a chunk's XZ footprint.
///
/// Uses Bridson's algorithm with deterministic seeded RNG.
/// Returns positions in chunk-local XZ space [0, CHUNK_SIZE).
pub fn poisson_disk_sample(
    seed: u64,
    chunk_pos: IVec3,
    min_spacing: f64,
    max_candidates: usize,
) -> Vec<Vec2> {
    // Implementation:
    // 1. Seed RNG from hash(seed, chunk_pos)
    // 2. Bridson's algorithm with cell_size = min_spacing / sqrt(2)
    // 3. Grid covers [0, CHUNK_SIZE) x [0, CHUNK_SIZE)
    // 4. Generate up to max_candidates points
    // 5. Return accepted positions
}

/// Derive a deterministic per-chunk, per-rule seed.
pub fn placement_seed(map_seed: u64, chunk_pos: IVec3, rule_index: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    map_seed.hash(&mut hasher);
    chunk_pos.hash(&mut hasher);
    rule_index.hash(&mut hasher);
    hasher.finish()
}
```

Add `pub mod placement;` to `lib.rs`.

#### 3. Height sampling utility
**File**: `crates/voxel_map_engine/src/placement.rs`
**Changes**: Sample terrain surface height from voxel array

```rust
use crate::types::{PADDED_CHUNK_SIZE, PaddedChunkShape, WorldVoxel};
use ndshape::ConstShape;

/// Find the highest solid voxel Y at a given local XZ position within a padded chunk.
/// Returns world Y coordinate, or None if the column is all air.
pub fn sample_terrain_height(
    voxels: &[WorldVoxel],
    chunk_pos: IVec3,
    local_x: u32,
    local_z: u32,
) -> Option<f64> {
    // Iterate Y from top to bottom in padded array
    // padded coords: (local_x + 1, py, local_z + 1)
    let px = local_x + 1;
    let pz = local_z + 1;
    for py in (0..PADDED_CHUNK_SIZE).rev() {
        let idx = PaddedChunkShape::linearize([px, py, pz]) as usize;
        if matches!(voxels[idx], WorldVoxel::Solid(_)) {
            let world_y = chunk_pos.y * CHUNK_SIZE as i32 + py as i32 - 1;
            return Some(world_y as f64 + 1.0); // +1 to place on top of surface
        }
    }
    None
}
```

#### 4. Wire `place_features` into HeightmapGenerator
**File**: `crates/voxel_map_engine/src/terrain.rs`
**Changes**: Add `PlacementRules` to generator struct, implement `place_features`

```rust
struct HeightmapGenerator {
    seed: u64,
    height_map: HeightMap,
    moisture_map: Option<MoistureMap>,
    biome_rules: Option<BiomeRules>,
    placement_rules: Option<PlacementRules>,
}

impl VoxelGeneratorImpl for HeightmapGenerator {
    fn generate_terrain(&self, chunk_pos: IVec3) -> Vec<WorldVoxel> {
        // ... existing implementation ...
    }

    fn place_features(&self, chunk_pos: IVec3, heights: &SurfaceHeightMap) -> Vec<WorldObjectSpawn> {
        let Some(ref rules) = self.placement_rules else { return Vec::new(); };
        let mut spawns = Vec::new();

        for (rule_idx, rule) in rules.0.iter().enumerate() {
            let seed = placement_seed(self.seed, chunk_pos, rule_idx);
            let max_candidates = (rule.density * (CHUNK_SIZE as f64).powi(2)).ceil() as usize;
            let candidates = poisson_disk_sample(seed, chunk_pos, rule.min_spacing, max_candidates);

            for pos_xz in candidates {
                let local_x = pos_xz.x as u32;
                let local_z = pos_xz.y as u32;
                if local_x >= CHUNK_SIZE || local_z >= CHUNK_SIZE {
                    trace!("place_features: candidate ({local_x}, {local_z}) out of chunk bounds");
                    continue;
                }

                let Some(height) = heights.heights[(local_x * 16 + local_z) as usize] else {
                    trace!("place_features: no surface at ({local_x}, {local_z}), skipping");
                    continue;
                };

                // Height range filter
                if height < rule.height_range.0 || height > rule.height_range.1 { continue; }

                // Biome filter (if biome rules exist)
                if !rule.allowed_biomes.is_empty() {
                    if let (Some(ref moisture_map), Some(ref biome_rules)) =
                        (&self.moisture_map, &self.biome_rules)
                    {
                        let biome = select_biome_at_pos(
                            chunk_pos, local_x, local_z,
                            self.seed, &self.height_map, moisture_map, biome_rules,
                        );
                        if !rule.allowed_biomes.iter().any(|b| b == &biome.biome_id) {
                            continue;
                        }
                    }
                }

                let world_x = chunk_pos.x as f32 * CHUNK_SIZE as f32 + pos_xz.x;
                let world_z = chunk_pos.z as f32 * CHUNK_SIZE as f32 + pos_xz.y;
                spawns.push(WorldObjectSpawn {
                    object_id: rule.object_id.clone(),
                    position: Vec3::new(world_x, height as f32, world_z),
                });
            }
        }
        spawns
    }
}
```

Add a helper `select_biome_at_pos` that builds noise at a single XZ position and calls `select_biome`. This avoids building a full 18x18 cache for a handful of point samples.

Update `build_generator` to read `PlacementRules`:
```rust
pub fn build_generator(entity: EntityRef, seed: u64) -> VoxelGenerator {
    let height = entity.get::<HeightMap>().cloned();
    let moisture = entity.get::<MoistureMap>().cloned();
    let biomes = entity.get::<BiomeRules>().cloned();
    let placement = entity.get::<PlacementRules>().cloned();
    // ... existing asserts ...
    match height {
        Some(height_map) => VoxelGenerator(Arc::new(HeightmapGenerator {
            seed, height_map,
            moisture_map: moisture,
            biome_rules: biomes,
            placement_rules: placement,
        })),
        None => VoxelGenerator(Arc::new(FlatGenerator)),
    }
}
```

#### 5. Tests
**File**: `crates/voxel_map_engine/src/placement.rs`
- Test `poisson_disk_sample` produces points within bounds
- Test `poisson_disk_sample` respects minimum spacing
- Test determinism: same seed → same points
- Test `sample_terrain_height` returns correct Y
- Test `placement_seed` uniqueness across chunks/rules

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all`
- [ ] `cargo test -p voxel_map_engine`

#### Manual Verification:
- [ ] `cargo server` — no entity spawns yet (no PlacementRules in terrain.ron)
- [ ] Unit tests for Poisson disk and height sampling pass

---

## Phase 5: Per-Chunk Entity Persistence

### Overview
Entities spawned during the Features stage persist with their chunk. `ChunkEntityRef` tags entities with their originating chunk. On chunk eviction, entities are serialized and saved. On reload, entities load from disk instead of re-generating.

### Changes Required:

#### 1. ChunkEntityRef component
**File**: `crates/protocol/src/map/mod.rs` (or a new `chunk_entity.rs`)
**Changes**: Shared component for server and (future) client

```rust
/// Tags an entity as belonging to a specific chunk on a specific map.
/// Used to save/despawn entities when their chunk is evicted.
#[derive(Component, Clone, Debug)]
pub struct ChunkEntityRef {
    pub chunk_pos: IVec3,
    pub map_entity: Entity,
}
```

#### 2. Entity file persistence
**File**: `crates/voxel_map_engine/src/persistence.rs`
**Changes**: Add per-chunk entity save/load alongside terrain persistence

```rust
const ENTITY_SAVE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct EntityFileEnvelope {
    version: u32,
    spawns: Vec<WorldObjectSpawn>,
}

/// File path for per-chunk entity data.
pub fn entity_file_path(map_dir: &Path, chunk_pos: IVec3) -> PathBuf {
    map_dir.join("entities").join(format!(
        "chunk_{}_{}_{}.entities.bin",
        chunk_pos.x, chunk_pos.y, chunk_pos.z
    ))
}

/// Save entity spawn data for a chunk. Atomic write via tmp+rename.
pub fn save_chunk_entities(
    map_dir: &Path,
    chunk_pos: IVec3,
    spawns: &[WorldObjectSpawn],
) -> Result<(), String> {
    // Same pattern as save_chunk: bincode + zstd + atomic rename
}

/// Load entity spawn data for a chunk. Returns None if no file exists.
pub fn load_chunk_entities(
    map_dir: &Path,
    chunk_pos: IVec3,
) -> Result<Option<Vec<WorldObjectSpawn>>, String> {
    // Same pattern as load_chunk
}
```

#### 3. Load entities during generation
**File**: `crates/voxel_map_engine/src/generation.rs`
**Changes**: In `spawn_terrain_batch`, after disk terrain load, also try loading entities

When a chunk is loaded from disk at Full/Mesh status, also load its entity file:
```rust
// Inside the disk-load path of spawn_terrain_batch:
let entity_spawns = if let Some(ref dir) = save_dir {
    match crate::persistence::load_chunk_entities(dir, pos) {
        Ok(Some(spawns)) => spawns,
        Ok(None) => vec![],
        Err(e) => {
            bevy::log::warn!("Failed to load entities at {pos}: {e}");
            vec![]
        }
    }
} else {
    vec![]
};
```

When a chunk is newly generated (Terrain stage), entity loading is deferred to the Features stage.

In `spawn_features_task`, try loading entities from disk first. If found, skip placement generation:
```rust
pub fn spawn_features_task(
    pending: &mut PendingChunks,
    position: IVec3,
    voxels: Vec<WorldVoxel>,
    generator: &VoxelGenerator,
    save_dir: Option<PathBuf>,
) {
    let gen = Arc::clone(&generator.0);
    let pool = AsyncComputeTaskPool::get();
    let task = pool.spawn(async move {
        let _span = info_span!("features_gen").entered();

        // Try disk load first (generate-once, save-forever)
        let entity_spawns = if let Some(ref dir) = save_dir {
            match crate::persistence::load_chunk_entities(dir, position) {
                Ok(Some(spawns)) => spawns,
                Ok(None) => gen.place_features(position, &voxels),
                Err(e) => {
                    bevy::log::warn!("Failed to load entities at {position}: {e}");
                    gen.place_features(position, &voxels)
                }
            }
        } else {
            gen.place_features(position, &voxels)
        };

        let chunk_data = ChunkData::from_voxels(&voxels, ChunkStatus::Features);
        vec![ChunkGenResult {
            position, mesh: None, chunk_data,
            entity_spawns, from_disk: false,
        }]
    });
    pending.tasks.push(task);
}
```

#### 4. Server-side entity spawn system
**File**: `crates/server/src/map.rs` (or new `crates/server/src/chunk_entities.rs`)
**Changes**: System that drains `PendingEntitySpawns` and calls `spawn_world_object`

```rust
/// Spawns world objects from completed Features stages.
fn spawn_chunk_entities(
    mut commands: Commands,
    mut map_query: Query<(Entity, &MapInstanceId, &mut PendingEntitySpawns)>,
    defs: Res<WorldObjectDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
) {
    for (map_entity, map_id, mut pending) in &mut map_query {
        for (chunk_pos, spawns) in pending.0.drain(..) {
            for spawn in spawns {
                let id = WorldObjectId(spawn.object_id.clone());
                let Some(def) = defs.get(&id) else {
                    warn!("Unknown world object '{}' in placement rules", spawn.object_id);
                    continue;
                };
                let entity = spawn_world_object(
                    &mut commands, id, def, map_id.clone(),
                    &type_registry, &vox_registry, &vox_assets, &meshes,
                );
                // Override position from placement
                commands.entity(entity).insert((
                    Position(spawn.position.into()),
                    ChunkEntityRef { chunk_pos, map_entity },
                ));
            }
        }
    }
}
```

Register this system in `ServerMapPlugin` after `poll_chunk_tasks`.

#### 5. Entity eviction on chunk unload
**File**: `crates/server/src/map.rs` (or `chunk_entities.rs`)
**Changes**: Save and despawn chunk entities when columns unload

Collects entities by chunk, saves per-chunk entity files async, then despawns:

```rust
fn evict_chunk_entities(
    mut commands: Commands,
    entity_query: Query<(Entity, &ChunkEntityRef, &WorldObjectId, &Position)>,
    map_query: Query<(&VoxelMapInstance, &VoxelMapConfig)>,
    mut pending_saves: Query<&mut PendingSaves>,
) {
    // Group entities by (map_entity, chunk_pos)
    let mut by_chunk: HashMap<(Entity, IVec3), Vec<(Entity, WorldObjectSpawn)>> = HashMap::new();

    for (entity, chunk_ref, obj_id, pos) in &entity_query {
        let Ok((instance, _)) = map_query.get(chunk_ref.map_entity) else { continue; };
        let col = chunk_to_column(chunk_ref.chunk_pos);
        if instance.chunk_levels.contains_key(&col) { continue; }

        by_chunk.entry((chunk_ref.map_entity, chunk_ref.chunk_pos))
            .or_default()
            .push((entity, WorldObjectSpawn {
                object_id: obj_id.0.clone(),
                position: Vec3::from(pos.0),
            }));
    }

    for ((map_entity, chunk_pos), entities) in by_chunk {
        let Ok((_, config)) = map_query.get(map_entity) else { continue; };
        let spawns: Vec<WorldObjectSpawn> = entities.iter().map(|(_, s)| s.clone()).collect();

        // Save entity file async
        if let Some(ref dir) = config.save_dir {
            let dir = dir.clone();
            let pool = AsyncComputeTaskPool::get();
            pool.spawn(async move {
                if let Err(e) = save_chunk_entities(&dir, chunk_pos, &spawns) {
                    error!("Failed to save chunk entities at {chunk_pos}: {e}");
                }
            }).detach();
        }

        // Despawn entities
        for (entity, _) in entities {
            commands.entity(entity).despawn();
        }
    }
}
```

#### 6. Save entities for newly generated chunks
When Features stage generates new entities (not loaded from disk), they need to be saved immediately so the "generate once, save forever" invariant holds.

**Coexistence with existing `entities.bin`**: The flat per-map `entities.bin` (`server/src/persistence.rs:76-107`) continues to handle map-global entities (`SavedEntityKind::RespawnPoint`, `MapSaveTarget` component). The two systems are orthogonal:
- `entities.bin`: loaded once at map spawn, saved on map save. Entities with `MapSaveTarget`.
- `entities/chunk_*.entities.bin`: loaded/saved with chunk lifecycle. Entities with `ChunkEntityRef`.

No migration of the old system is needed. An entity has either `MapSaveTarget` or `ChunkEntityRef`, never both.

In `spawn_chunk_entities`:
```rust
// After spawning all entities for a chunk, save to disk
if let Some(ref dir) = config.save_dir {
    let dir = dir.clone();
    let chunk_pos = chunk_pos;
    let spawns_clone = spawns.clone();
    let pool = AsyncComputeTaskPool::get();
    pool.spawn(async move {
        if let Err(e) = save_chunk_entities(&dir, chunk_pos, &spawns_clone) {
            error!("Failed to save new chunk entities at {chunk_pos}: {e}");
        }
    }).detach();
}
```

The `spawns` vec is already available: `spawn_chunk_entities` drains `PendingEntitySpawns.0` which contains `(IVec3, Vec<WorldObjectSpawn>)` tuples. After the inner spawn loop, save using the same `spawns` slice before moving to the next chunk:

#### 7. Shutdown save for loaded chunk entities

Eviction saves entities when chunks unload during gameplay. But if a tree is destroyed while its chunk is still loaded, the initial entity file (from Phase 5 step 6) still contains it. On server shutdown, loaded chunks are not evicted — the entity file is stale.

Add a shutdown system (`OnExit(AppState::Ready)` or `AppExit` observer) that saves all loaded chunk entities:

```rust
/// On server shutdown, save entity files for all loaded chunks.
fn save_all_chunk_entities_on_exit(
    entity_query: Query<(&ChunkEntityRef, &WorldObjectId, &Position)>,
    map_query: Query<&VoxelMapConfig>,
) {
    let mut by_chunk: HashMap<(Entity, IVec3), Vec<WorldObjectSpawn>> = HashMap::new();
    for (chunk_ref, obj_id, pos) in &entity_query {
        by_chunk.entry((chunk_ref.map_entity, chunk_ref.chunk_pos))
            .or_default()
            .push(WorldObjectSpawn {
                object_id: obj_id.0.clone(),
                position: Vec3::from(pos.0),
            });
    }
    for ((map_entity, chunk_pos), spawns) in by_chunk {
        let Ok(config) = map_query.get(map_entity) else { continue; };
        if let Some(ref dir) = config.save_dir {
            if let Err(e) = save_chunk_entities(dir, chunk_pos, &spawns) {
                error!("Shutdown save failed for chunk {chunk_pos}: {e}");
            }
        }
    }
}
```

This ensures destroyed entities (no longer in the query) are excluded from the saved file.

#### 8. Tests
**File**: `crates/voxel_map_engine/src/persistence.rs` tests
- `save_load_chunk_entities_roundtrip`
- `load_nonexistent_entities_returns_none`
- `empty_entities_save_load`

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all`
- [ ] `cargo test -p voxel_map_engine`
- [ ] `cargo test -p server`

#### Manual Verification:
- [ ] `cargo server` — no entity spawns yet (no PlacementRules in terrain.ron)
- [ ] Entity persistence tests pass

---

## Phase 6: Integration, Data, and Cleanup

### Overview
Add `PlacementRules` to `overworld.terrain.ron`. Remove `spawn_test_tree`. Verify end-to-end: trees spawn procedurally, persist across restarts, don't duplicate.

### Changes Required:

#### 1. Add PlacementRules to overworld terrain
**File**: `assets/terrain/overworld.terrain.ron`
**Changes**: Add `PlacementRules` component

```ron
{
    "voxel_map_engine::terrain::HeightMap": (
        // ... existing ...
    ),
    "voxel_map_engine::terrain::MoistureMap": (
        // ... existing ...
    ),
    "voxel_map_engine::terrain::BiomeRules": ([
        // ... existing ...
    ]),
    "voxel_map_engine::terrain::PlacementRules": ([
        (
            object_id: "tree_circle",
            allowed_biomes: ["forest", "grassland"],
            density: 0.015,
            min_spacing: 5.0,
            slope_max: Some(0.5),
            height_range: (-5.0, 30.0),
        ),
    ]),
}
```

Density 0.015 ≈ ~4 trees per 16x16 chunk in qualifying biomes. Tune at runtime.

#### 2. Remove spawn_test_tree
**File**: `crates/server/src/gameplay.rs`
**Changes**:
- Delete `spawn_test_tree` function (lines 237-260)
- Remove `app.add_systems(OnEnter(AppState::Ready), spawn_test_tree)` from plugin registration (line 37)
- Remove unused imports (`WorldObjectDefRegistry`, `WorldObjectId`, `VoxModelRegistry`, etc.) if no longer needed

#### 3. Delete saved world data
The persistence version bump (Phase 1) means old chunk files won't load. But entity files are new. Instruct developer to delete `worlds/` directory before testing to start fresh.

#### 4. Update README.md
If any documented features, commands, or architecture changed, update accordingly.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all`
- [ ] `cargo test -p voxel_map_engine`
- [ ] `cargo test -p server`
- [ ] `cargo server` builds and runs
- [ ] `cargo client -c 1` builds and runs

#### Manual Verification:
- [ ] Trees appear procedurally on overworld terrain in forest and grassland biomes
- [ ] Trees respect min_spacing (no overlapping trees)
- [ ] Trees only appear on terrain surface (correct Y position)
- [ ] No trees in desert or mountain biomes
- [ ] Server restart: same trees reappear (loaded from disk, not regenerated)
- [ ] No duplicate trees after restart
- [ ] Walking to new area → new trees generate; walking back → same trees from disk
- [ ] Tracy profiling shows separate terrain_gen, features_gen, mesh_gen spans
- [ ] Destroying a tree (reducing health to 0) → tree despawns → server restart → tree does not reappear

---

## Testing Strategy

### Unit Tests:
- `ChunkStatus::next()`, ordering, `max_for_level()`
- `poisson_disk_sample` bounds, spacing, determinism
- `sample_terrain_height` correctness
- `placement_seed` uniqueness
- Entity file save/load roundtrip
- `ChunkData` with status field serialization roundtrip

### Integration Tests:
- `tests/lifecycle.rs`: Update existing tests for multi-stage; add test that chunks progress through stages
- Lifecycle test with `PlacementRules`: verify entity_spawns populated
- Entity persistence: generate → save → load → compare

### Manual Testing Steps:
1. Delete `worlds/` directory
2. `cargo server` — observe terrain + trees generating
3. Walk around — new chunks generate with trees
4. Kill server, restart — same trees appear
5. `cargo client -c 1` — trees replicate to client
6. Destroy a tree, kill server, restart — tree stays gone

## Performance Considerations

- **Multi-stage overhead**: Each chunk requires 3 async tasks (Terrain, Features, Mesh) instead of 1. `Full` promotion is synchronous. Terrain is batched (8 per task). When no `PlacementRules` exist, `drain_gen_queue` can skip the Features async task and promote Terrain→Features synchronously (same pattern as Full promotion).
- **Palette expansion**: Mesh stage expands PalettedChunk → Vec on the main thread (O(5832), same as remesh). Features stage avoids expansion entirely — builds a compact 16×16 surface height map from PalettedChunk (O(256 × 18) column scans) and passes only that to the async task.
- **Entity file I/O**: Per-chunk entity files are small (~100-500 bytes compressed). Read during Features stage (async). Write on eviction (async, fire-and-forget).
- **Poisson disk sampling**: O(n) per chunk where n = candidate count. Typically <20 candidates per rule. Negligible.
- **Frame budget**: Multi-stage naturally spreads work across frames. Budget-gated drain prevents overcommit.

## Migration Notes

- `CHUNK_SAVE_VERSION` bumps from 2 to 3. Old chunk files won't load (version mismatch error → regenerated from noise). Delete `worlds/` directory for clean start.
- `VoxelGenerator` API changes from closure to trait. External consumers (`tests/lifecycle.rs`, server `build_terrain_generators`) must update.
- `ChunkData::from_voxels` gains a `status` parameter. All call sites must be updated.

## References

- Research: `doc/research/2026-03-26-multi-stage-generation-pipeline.md`
- World object placement research: `doc/research/2026-03-22-world-object-placement-and-per-chunk-entity-persistence.md`
- Minecraft ticket system research: `doc/research/2026-03-20-minecraft-chunk-ticket-system.md`
- Chunk pipeline optimizations plan: `doc/plans/2026-03-22-chunk-pipeline-optimizations.md`
- Procedural generation plan: `doc/plans/2026-03-19-procedural-map-generation.md`
