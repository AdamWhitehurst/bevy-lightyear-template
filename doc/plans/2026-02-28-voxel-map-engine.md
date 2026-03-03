# Voxel Map Engine Implementation Plan

## Overview

Replace `bevy_voxel_world` with a custom `voxel_map_engine` crate built on the bonsairobo stack (`grid-tree`, `fast-surface-nets`, `block-mesh`, `ndshape`, `ndcopy`). The new crate supports multiple map instances (overworld, homebases, arenas) via entity-based multiplexing instead of type-level generics.

## Current State Analysis

The project uses `bevy_voxel_world` (local fork at `git/bevy_voxel_world/`) across three crates:
- **protocol** (`crates/protocol/src/map.rs`) — `MapWorld` config, `VoxelType`, `attach_chunk_colliders`, network message types
- **client** (`crates/client/src/map.rs`) — `ClientMapPlugin` with VoxelWorldPlugin, broadcast/sync handlers, raycast input
- **server** (`crates/server/src/map.rs`) — `ServerMapPlugin` with VoxelWorldPlugin, persistence, edit handling
- **server** (`crates/server/src/gameplay.rs`) — `ChunkRenderTarget<MapWorld>` on player entities
- **protocol** (`crates/protocol/src/lib.rs:167`) — Lightyear registration of `ChunkRenderTarget<MapWorld>`

Types consumed from `bevy_voxel_world`: `VoxelWorldPlugin`, `VoxelWorld` (SystemParam), `VoxelWorldConfig`, `WorldVoxel`, `Chunk<C>`, `ChunkRenderTarget<C>`, `VoxelLookupDelegate`, `VoxelRaycastResult`.

### Key Discoveries:
- `bevy_voxel_world` uses generic type parameter `C: VoxelWorldConfig` for world multiplexing — exactly one instance per type, impossible for runtime-dynamic instance counts
- The bonsairobo crates have **glam version mismatches**: grid-tree uses 0.25, fast-surface-nets uses 0.29, Bevy 0.17 uses 0.30. All forks need updating.
- `voxel_traversal.rs` is trivially extractable (depends only on a `VoxelFace` enum and a `VOXEL_SIZE` constant)
- `voxel_material.rs` is fully self-contained (no bevy_voxel_world internal deps)
- block-mesh depends on `ilattice 0.1` which brings glam 0.19 — this dependency should be removed or the block-mesh fork updated to use glam 0.30 types directly

## Desired End State

A workspace crate `crates/voxel_map_engine/` that:
1. Provides an entity-based voxel engine where each map instance is a Bevy entity with a `VoxelMapInstance` component
2. Supports spawning/despawning arbitrary numbers of map instances at runtime
3. Uses `fast-surface-nets` for smooth terrain meshing (primary path)
4. Has a stubbed `block-mesh` integration for future blocky meshing
5. Provides `VoxelWorld` SystemParam with `get_voxel(map, pos)`, `set_voxel(map, pos, voxel)`, `raycast(map, ray)`
6. Is physics-engine-agnostic (collider attachment stays in protocol)
7. All existing functionality works: voxel editing, persistence, networking, colliders

### Verification:
- `cargo check-all` and `cargo test-all` pass
- `cargo server` + `cargo client` run with terrain visible and editable
- Multiple map instances can exist simultaneously (demonstrated by example)

## What We're NOT Doing

- LOD system (deferred — accept all chunks at level 0)
- Noise-based terrain generation (current flat terrain preserved; noise is a separate task)
- `bevy_triplanar_splatting` port to Bevy 0.17 (deferred until smooth meshing needs multi-material blending)
- LOD transition meshes (accept seams at LOD boundaries)
- Custom chunk meshing delegates
- Debug drawing plugin

## Implementation Approach

Build the new crate incrementally with standalone tests and examples at each phase, then swap it into the existing protocol/client/server crates. The bonsairobo git submodules are used as path dependencies with glam updated to 0.30.

---

## Phase 1: Crate Scaffold + Remove bevy_voxel_world + Core Data Types

### Overview
Create the `voxel_map_engine` crate with core data types. Remove `bevy_voxel_world` from all workspace crates. Stub protocol/client/server map modules so everything compiles. Update bonsairobo crate forks to glam 0.30.

### Changes Required:

#### 1. Update bonsairobo forks to glam 0.30

**File**: `git/grid-tree-rs/Cargo.toml`
**Changes**: Update `glam = "0.25"` → `glam = "0.30"`

**File**: `git/grid-tree-rs/src/*.rs`
**Changes**: Fix any API breakages from glam 0.25→0.30 (likely minimal — IVec3/Vec3 APIs are stable)

**File**: `git/fast-surface-nets-rs/Cargo.toml`
**Changes**: Update `glam = "0.29"` → `glam = "0.30"`

**File**: `git/fast-surface-nets-rs/src/lib.rs`
**Changes**: Fix any API breakages from glam 0.29→0.30 (likely none)

**File**: `git/block-mesh-rs/Cargo.toml`
**Changes**: Remove `ilattice` dependency (brings glam 0.19). Replace any ilattice usage with direct glam 0.30 types. Add `glam = "0.30"` if needed.

#### 2. Create `crates/voxel_map_engine/`

**File**: `crates/voxel_map_engine/Cargo.toml`
```toml
[package]
name = "voxel_map_engine"
version = "0.1.0"
edition = "2024"

[dependencies]
bevy = { workspace = true }
ndshape = { path = "../../git/ndshape-rs" }
ndcopy = { path = "../../git/ndcopy-rs" }
grid-tree = { path = "../../git/grid-tree-rs" }
fast-surface-nets = { path = "../../git/fast-surface-nets-rs" }
block-mesh = { path = "../../git/block-mesh-rs" }
weak-table = "0.3"
serde = { workspace = true }

[dev-dependencies]
approx = { workspace = true }
```

**File**: `crates/voxel_map_engine/src/lib.rs`
```rust
pub mod types;

pub mod prelude {
    pub use crate::types::*;
}
```

**File**: `crates/voxel_map_engine/src/types.rs`
```rust
use bevy::prelude::*;
use ndshape::ConstShape;
use serde::{Deserialize, Serialize};

/// 16³ voxel chunks with 1-voxel padding on each side → 18³ padded array
pub type PaddedChunkShape = ndshape::ConstShape3u32<18, 18, 18>;

pub const CHUNK_SIZE: u32 = 16;
pub const PADDED_CHUNK_SIZE: u32 = 18;

/// Voxel data stored per position
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub enum WorldVoxel {
    Air,
    Unset,
    Solid(u8),
}

impl Default for WorldVoxel {
    fn default() -> Self {
        Self::Unset
    }
}

/// How a chunk is filled (optimization for uniform chunks)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillType {
    Empty,
    Mixed,
    Uniform(WorldVoxel),
}

/// Voxel data for one chunk (16³ with 1-voxel padding = 18³)
#[derive(Clone)]
pub struct ChunkData {
    pub voxels: Vec<WorldVoxel>,
    pub fill_type: FillType,
    pub hash: u64,
}

impl ChunkData {
    pub fn new_empty() -> Self {
        Self {
            voxels: vec![WorldVoxel::Air; PaddedChunkShape::SIZE as usize],
            fill_type: FillType::Empty,
            hash: 0,
        }
    }
}

/// Network-serializable voxel type (mirrors WorldVoxel without Unset)
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub enum VoxelType {
    Air,
    Solid(u8),
}

impl From<VoxelType> for WorldVoxel {
    fn from(v: VoxelType) -> Self {
        match v {
            VoxelType::Air => WorldVoxel::Air,
            VoxelType::Solid(m) => WorldVoxel::Solid(m),
        }
    }
}

impl From<WorldVoxel> for VoxelType {
    fn from(v: WorldVoxel) -> Self {
        match v {
            WorldVoxel::Air | WorldVoxel::Unset => VoxelType::Air,
            WorldVoxel::Solid(m) => VoxelType::Solid(m),
        }
    }
}
```

#### 3. Add to workspace

**File**: `Cargo.toml` (root)
**Changes**:
- Add `"crates/voxel_map_engine"` to `workspace.members`
- Add `voxel_map_engine = { path = "crates/voxel_map_engine" }` to `[workspace.dependencies]`
- Remove `bevy_voxel_world` from `[workspace.dependencies]`

#### 4. Update protocol crate

**File**: `crates/protocol/Cargo.toml`
**Changes**: Replace `bevy_voxel_world` dependency with `voxel_map_engine`

**File**: `crates/protocol/src/map.rs`
**Changes**: Remove all bevy_voxel_world imports. Move `VoxelType`, `VoxelEditRequest`, `VoxelEditBroadcast`, `VoxelStateSync` to use types from `voxel_map_engine`. Stub `attach_chunk_colliders` (empty system for now — will be restored in Phase 5). Remove `MapWorld` VoxelWorldConfig impl (replaced by `VoxelMapConfig` later). Keep `MapWorld` as a resource with `seed` and `generation_version` for persistence compatibility.

**File**: `crates/protocol/src/lib.rs`
**Changes**: Remove `ChunkRenderTarget<MapWorld>` lightyear registration (will be replaced with `ChunkTarget` registration in Phase 5). Remove `use bevy_voxel_world`.

#### 5. Update client crate

**File**: `crates/client/Cargo.toml`
**Changes**: Replace `bevy_voxel_world` with `voxel_map_engine`

**File**: `crates/client/src/map.rs`
**Changes**: Stub `ClientMapPlugin` — keep only the network message handler systems (handle_voxel_broadcasts, handle_state_sync) but have them log warnings ("voxel engine not yet integrated") instead of calling voxel_world. Comment out handle_voxel_input (depends on raycast). Remove VoxelWorldPlugin usage.

#### 6. Update server crate

**File**: `crates/server/Cargo.toml`
**Changes**: Replace `bevy_voxel_world` with `voxel_map_engine`

**File**: `crates/server/src/map.rs`
**Changes**: Stub `ServerMapPlugin` — keep persistence resources and systems (VoxelModifications, VoxelDirtyState, VoxelSavePath, save/load functions unchanged since they use VoxelType not WorldVoxel). Stub edit handling to only track modifications and broadcast, skip voxel_world.set_voxel. Remove VoxelWorldPlugin.

**File**: `crates/server/src/gameplay.rs`
**Changes**: Remove `ChunkRenderTarget<MapWorld>` usage. Will be replaced with `ChunkTarget` in Phase 5.

#### 7. Update render crate

**File**: `crates/render/Cargo.toml`
**Changes**: Remove `bevy_voxel_world` dependency if present

#### 8. Update tests

**File**: `crates/server/tests/voxel_persistence.rs`
**Changes**: Update imports to use `voxel_map_engine::prelude::VoxelType` instead of `protocol::VoxelType` (or keep using protocol's re-export). The save/load tests should still pass since they test the serialization functions directly and those don't depend on bevy_voxel_world.

**File**: `crates/server/tests/integration.rs`
**Changes**: Remove or skip voxel message tests that depend on VoxelWorld SystemParam. Keep message registration tests.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` passes (all crates compile)
- [ ] `cargo test-all` passes (persistence tests pass, integration tests pass or are skipped)
- [ ] `grid-tree`, `fast-surface-nets`, `block-mesh` compile with glam 0.30

#### Manual Verification:
- [ ] `cargo server` starts without crash (no terrain visible — expected, voxel systems stubbed)
- [ ] `cargo client` starts without crash (no terrain visible — expected)

---

## Phase 2: Spatial Index + Meshing

### Overview
Implement `VoxelMapInstance` component with `OctreeI32` spatial index, the `VoxelMesher` trait with `fast-surface-nets` implementation, and Bevy Mesh conversion. Validate with unit tests and a standalone example.

### Changes Required:

#### 1. VoxelMapInstance component

**File**: `crates/voxel_map_engine/src/instance.rs`
```rust
use bevy::prelude::*;
use grid_tree::OctreeI32;
use std::collections::HashMap;
use crate::types::{ChunkData, WorldVoxel};

/// Core component on every map entity. Owns the spatial index and per-instance state.
#[derive(Component)]
pub struct VoxelMapInstance {
    pub tree: OctreeI32<Option<ChunkData>>,
    pub modified_voxels: HashMap<IVec3, WorldVoxel>,
    pub write_buffer: Vec<(IVec3, WorldVoxel)>,
}

impl VoxelMapInstance {
    pub fn new(tree_height: u32) -> Self {
        Self {
            tree: OctreeI32::new(tree_height),
            modified_voxels: HashMap::new(),
            write_buffer: Vec::new(),
        }
    }
}
```

#### 2. VoxelMapConfig

**File**: `crates/voxel_map_engine/src/config.rs`
```rust
use bevy::prelude::*;

/// Generation function: given chunk position, returns voxel data for the padded array
pub type VoxelGenerator = Box<dyn Fn(IVec3) -> Vec<f32> + Send + Sync>;

/// Configuration for a map instance
#[derive(Component)]
pub struct VoxelMapConfig {
    pub seed: u64,
    pub spawning_distance: u32,
    pub bounds: Option<IVec3>,
    pub tree_height: u32,
    pub generator: VoxelGenerator,
}
```

#### 3. Meshing trait + surface nets implementation

**File**: `crates/voxel_map_engine/src/meshing.rs`

Define `VoxelMesher` trait with a `mesh_chunk(&[f32], IVec3) -> Option<Mesh>` method. Implement `SurfaceNetsMesher` using `fast_surface_nets::surface_nets`. Convert `SurfaceNetsBuffer` (positions, normals, indices) to a Bevy `Mesh`.

The SDF generation: for the initial flat terrain, the SDF is computed as `sdf[i] = -(voxel_world_y - 0.0)` — negative below y=0 (solid), positive above (air). This matches the current `WorldVoxel::Solid(0)` below y=0 behavior but in continuous form for surface nets.

#### 4. Mesh cache

**File**: `crates/voxel_map_engine/src/mesh_cache.rs`

`MeshCache` component on the `VoxelMapInstance` entity: `WeakValueHashMap<u64, Weak<Handle<Mesh>>>` keyed by chunk data hash. Each map instance owns its own cache — no cross-instance mesh sharing. Same weak-reference pattern as current bevy_voxel_world.

#### 5. Wire up modules

**File**: `crates/voxel_map_engine/src/lib.rs`
```rust
pub mod types;
pub mod instance;
pub mod config;
pub mod meshing;
pub mod mesh_cache;

pub mod prelude {
    pub use crate::types::*;
    pub use crate::instance::*;
    pub use crate::config::*;
    pub use crate::meshing::*;
    pub use crate::mesh_cache::*;
}
```

#### 6. Standalone example

**File**: `crates/voxel_map_engine/examples/terrain.rs`

Minimal Bevy app that:
1. Spawns a `VoxelMapInstance` entity with flat-terrain SDF generator
2. Manually inserts chunk data for a 5×5 grid of chunks around origin
3. Meshes each chunk using `SurfaceNetsMesher`
4. Spawns mesh entities as children of the map entity
5. Adds a camera and light so terrain is visible

This validates the full data path: SDF generation → surface nets → Bevy Mesh → rendering.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` passes
- [ ] `cargo test -p voxel_map_engine` passes (unit tests for ChunkData, PaddedChunkShape indexing, SDF→mesh conversion, mesh cache insertion/lookup)
- [ ] `cargo test-all` passes

#### Manual Verification:
- [ ] `cargo run --example terrain -p voxel_map_engine` shows smooth terrain surface at y=0

---

## Phase 3: Chunk Lifecycle + Material + Colliders

### Overview
Implement the chunk spawn/despawn system driven by `ChunkTarget` entities, async chunk generation via `AsyncComputeTaskPool`, the `StandardVoxelMaterial` (copied from bevy_voxel_world), and collider attachment. After this phase, a standalone example renders textured terrain that loads around a moving entity.

### Changes Required:

#### 1. Chunk and ChunkTarget components

**File**: `crates/voxel_map_engine/src/chunk.rs`
```rust
use bevy::prelude::*;

/// Marker on chunk mesh entities (children of map entity)
#[derive(Component)]
pub struct VoxelChunk {
    pub position: IVec3,
    pub lod_level: u8,
}

/// Attach to entities whose Transform drives chunk loading for a specific map
#[derive(Component)]
pub struct ChunkTarget {
    pub map_entity: Entity,
    pub distance: u32,
}
```

#### 2. Chunk lifecycle systems

**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Systems:
- `update_chunks` — For each `VoxelMapInstance`, gather all `ChunkTarget` entities pointing at it. Compute which chunk positions should exist (within `distance` of any target). Spawn missing chunks as async generation tasks. Tag out-of-range chunks for despawn.
- `poll_chunk_tasks` — Poll `AsyncComputeTaskPool` tasks. When complete, insert chunk data into the octree and spawn the mesh entity as a child of the map entity.
- `despawn_chunks` — Remove tagged chunks from the octree and despawn their entities.
- `flush_write_buffer` — Drain `VoxelMapInstance.write_buffer`, apply to `modified_voxels`, mark affected chunks for remesh.

#### 3. Async generation

**File**: `crates/voxel_map_engine/src/generation.rs`

A task struct that runs on `AsyncComputeTaskPool`:
1. Generate SDF values for the padded chunk (18³) using the config's `VoxelGenerator`
2. Apply any `modified_voxels` overrides
3. Mesh via `SurfaceNetsMesher`
4. Return the `ChunkData` + `Option<Mesh>`

#### 4. Material (copy from bevy_voxel_world)

**File**: `crates/voxel_map_engine/src/material.rs`

Copy `StandardVoxelMaterial` from `git/bevy_voxel_world/src/voxel_material.rs`. It has zero internal bevy_voxel_world dependencies. Copy the WGSL shader file from `git/bevy_voxel_world/src/shaders/voxel_texture.wgsl` into `crates/voxel_map_engine/src/shaders/`. Copy the default texture (`default_texture.png`).

Adapt for surface nets output: surface nets produces positions + normals + indices but no UVs or tex indices. The material may need a simpler mode initially (solid color or basic PBR without array textures) until we have proper vertex attributes. Alternatively, generate UVs from world-space position (triplanar-style) in the shader.

Decision: For Phase 3, use a **simple PBR material** (StandardMaterial with a single color/texture). The full `StandardVoxelMaterial` with array textures is designed for blocky greedy-quads output. Surface nets output needs a different texturing approach (triplanar or world-space UVs). Defer the full material to when `bevy_triplanar_splatting` is ported.

#### 5. VoxelPlugin

**File**: `crates/voxel_map_engine/src/lib.rs`

```rust
pub struct VoxelPlugin;

impl Plugin for VoxelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (
            lifecycle::update_chunks,
            lifecycle::poll_chunk_tasks,
            lifecycle::despawn_chunks,
            lifecycle::flush_write_buffer,
        ).chain());
    }
}
```

#### 6. Update example

**File**: `crates/voxel_map_engine/examples/terrain.rs`

Update to use VoxelPlugin, spawn a map entity with `VoxelMapInstance` + `VoxelMapConfig`, and a camera entity with `ChunkTarget`. Terrain should generate and render around the camera. Add keyboard controls to move the camera and observe chunks loading/unloading.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` passes
- [ ] `cargo test -p voxel_map_engine` passes (lifecycle unit tests: chunk spawn within range, despawn outside range)
- [ ] `cargo test-all` passes

#### Manual Verification:
- [ ] `cargo run --example terrain -p voxel_map_engine` shows terrain generating around a moving camera
- [ ] Chunks load/unload as camera moves
- [ ] FPS is acceptable (>30fps) with spawning_distance=5

---

## Phase 4: Public API + Raycasting + Write Buffer

### Overview
Implement the `VoxelWorld` SystemParam equivalent for get/set/raycast operations, the Amanatides & Woo raycasting algorithm, and the write buffer flush cycle. After this phase, voxels can be read, written, and raycasted against.

### Changes Required:

#### 1. Raycasting module

**File**: `crates/voxel_map_engine/src/raycast.rs`

Extract from `git/bevy_voxel_world/src/voxel_traversal.rs`:
- Copy `voxel_line_traversal` function
- Copy `voxel_cartesian_traversal` function
- Define `VoxelFace` enum locally (7 variants: None, Bottom, Top, Left, Right, Back, Forward)
- Replace `VOXEL_SIZE` constant (1.0) with a local const
- Add `VoxelRaycastResult` struct: `position: IVec3, normal: Option<Vec3>, voxel: WorldVoxel`

Add a `raycast` function that:
1. Computes ray-AABB intersection against loaded chunk bounds
2. Calls `voxel_line_traversal` from entry to exit
3. Looks up each visited voxel position in the VoxelMapInstance
4. Returns `Option<VoxelRaycastResult>` for the first hit matching a filter

#### 2. Public API (VoxelWorld SystemParam)

**File**: `crates/voxel_map_engine/src/api.rs`

```rust
use bevy::prelude::*;
use crate::instance::VoxelMapInstance;
use crate::types::{WorldVoxel, CHUNK_SIZE, PaddedChunkShape};
use crate::raycast::VoxelRaycastResult;

/// SystemParam for reading/writing voxels on any map instance
#[derive(SystemParam)]
pub struct VoxelWorld<'w, 's> {
    maps: Query<'w, 's, &'static mut VoxelMapInstance>,
}

impl VoxelWorld<'_, '_> {
    /// Get voxel at world position on a specific map instance
    pub fn get_voxel(&self, map: Entity, pos: IVec3) -> WorldVoxel { ... }

    /// Queue a voxel write (applied during flush_write_buffer)
    pub fn set_voxel(&mut self, map: Entity, pos: IVec3, voxel: WorldVoxel) { ... }

    /// Raycast against a specific map instance
    pub fn raycast(
        &self,
        map: Entity,
        ray: Ray3d,
        filter: &dyn Fn(WorldVoxel) -> bool,
    ) -> Option<VoxelRaycastResult> { ... }
}
```

Key difference from bevy_voxel_world: every operation takes a `map: Entity` parameter to select which map instance to operate on. This enables multi-instance support.

#### 3. Write buffer improvements

Enhance `lifecycle::flush_write_buffer`:
- Drain write buffer entries
- Update `modified_voxels` HashMap
- Determine which chunks are affected (world pos → chunk pos)
- Mark affected chunks for remesh (despawn old mesh entity, queue new generation task)

#### 4. Standalone editing example

**File**: `crates/voxel_map_engine/examples/editing.rs`

Bevy app that:
1. Spawns terrain (same as Phase 3 example)
2. On mouse click, raycasts into the voxel world
3. Left click: remove voxel (set to Air)
4. Right click: place voxel (set to Solid(0) on adjacent face)
5. Shows the edit taking effect (chunk remeshes)

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` passes
- [ ] `cargo test -p voxel_map_engine` passes:
  - `get_voxel`/`set_voxel` round-trip
  - Raycast against known geometry (flat plane at y=0, ray from above hits at y=0)
  - Write buffer flush triggers remesh
  - Modified voxels survive chunk despawn/respawn cycle
- [ ] `cargo test-all` passes

#### Manual Verification:
- [ ] `cargo run --example editing -p voxel_map_engine` allows placing/removing voxels
- [ ] Edits are visible immediately (chunk remeshes within 1-2 frames)
- [ ] Edits persist when moving away and returning (modified_voxels survives chunk cycle)

---

## Phase 5: Integration with Protocol/Client/Server

### Overview
Un-stub the protocol, client, and server map modules. Wire up the new `VoxelWorld` API, networking, and persistence. After this phase, the game works end-to-end with the new voxel engine.

### Changes Required:

#### 1. Protocol crate

**File**: `crates/protocol/src/map.rs`
**Changes**:
- Remove `MapWorld`'s `VoxelWorldConfig` impl (replaced by `VoxelMapConfig`)
- Keep `MapWorld` resource (used by persistence for seed/version validation)
- Keep `VoxelType`, `VoxelEditRequest`, `VoxelEditBroadcast`, `VoxelStateSync` (now using `voxel_map_engine::prelude::VoxelType`)
- Restore `attach_chunk_colliders` system, querying `VoxelChunk` instead of `Chunk<MapWorld>`
- Keep `VoxelChannel` message channel

```rust
pub fn attach_chunk_colliders(
    mut commands: Commands,
    chunks: Query<
        (Entity, &Mesh3d, Option<&Collider>),
        (With<VoxelChunk>, Or<(Changed<Mesh3d>, Added<Mesh3d>)>),
    >,
    meshes: Res<Assets<Mesh>>,
) {
    // Same logic as before, querying VoxelChunk instead of Chunk<MapWorld>
}
```

**File**: `crates/protocol/src/lib.rs`
**Changes**:
- Register `ChunkTarget` as a lightyear component (replaces `ChunkRenderTarget<MapWorld>`)
- Update imports

#### 2. Server crate

**File**: `crates/server/src/map.rs`
**Changes**:
- `ServerMapPlugin` now adds `VoxelPlugin` from voxel_map_engine
- Spawns a `VoxelMapInstance` entity with flat-terrain SDF generator on startup
- Stores the map entity in a resource for easy access
- `load_voxel_world` uses `VoxelWorld` SystemParam to apply loaded modifications
- `handle_voxel_edit_requests` uses `VoxelWorld::set_voxel(map_entity, pos, voxel)`
- `send_initial_voxel_state` unchanged (uses VoxelModifications resource)

```rust
/// Resource tracking the primary overworld map entity
#[derive(Resource)]
pub struct OverworldMap(pub Entity);

fn spawn_overworld(mut commands: Commands) {
    let map = commands.spawn((
        VoxelMapInstance::new(5),
        VoxelMapConfig {
            seed: 999,
            spawning_distance: 2,
            bounds: None,
            tree_height: 5,
            generator: Box::new(flat_terrain_sdf),
        },
        Transform::default(),
    )).id();
    commands.insert_resource(OverworldMap(map));
}
```

**File**: `crates/server/src/gameplay.rs`
**Changes**: Replace `ChunkRenderTarget::<MapWorld>::default()` with `ChunkTarget { map_entity: overworld.0, distance: 2 }` when spawning player entities.

#### 3. Client crate

**File**: `crates/client/src/map.rs`
**Changes**:
- `ClientMapPlugin` now adds `VoxelPlugin`
- Spawns a `VoxelMapInstance` entity on startup (matching server config)
- `handle_voxel_broadcasts` uses `VoxelWorld::set_voxel(map_entity, ...)`
- `handle_state_sync` uses `VoxelWorld::set_voxel(map_entity, ...)`
- Restore `handle_voxel_input` with `VoxelWorld::raycast(map_entity, ray, filter)`
- Camera entity gets `ChunkTarget { map_entity, distance: 2 }`

#### 4. Persistence compatibility

The persistence format (`VoxelWorldSave`) uses `Vec<(IVec3, VoxelType)>`. `VoxelType` is now defined in `voxel_map_engine::types` and re-exported by protocol. The save format is unchanged — existing save files remain compatible.

#### 5. Update tests

**File**: `crates/server/tests/voxel_persistence.rs`
**Changes**: Update imports. Tests should pass unchanged since they test save/load functions that use `VoxelType` and `MapWorld` directly.

**File**: `crates/server/tests/integration.rs`
**Changes**: Update voxel message tests to work with new crate. The message types are unchanged, so lightyear registration tests should pass.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` passes
- [ ] `cargo test-all` passes
- [ ] `cargo build-all` succeeds

#### Manual Verification:
- [ ] `cargo server` starts and generates terrain
- [ ] `cargo client` connects, terrain is visible
- [ ] Left/right click places/removes voxels
- [ ] Voxel edits sync between client and server
- [ ] Server shutdown saves modifications, restart restores them
- [ ] Multiple clients see each other's edits

---

## Phase 6: Multi-Instance Support

### Overview
Enable multiple `VoxelMapInstance` entities to coexist. Add marker components for world types (Overworld, Homebase, Arena). Add instance lifecycle management. Validate with a standalone multi-instance example and server/client testing.

### Changes Required:

#### 1. Instance marker components

**File**: `crates/voxel_map_engine/src/instance.rs` (extend)

```rust
/// Marker: this map is the shared overworld
#[derive(Component)]
pub struct Overworld;

/// Marker: this map is a player's homebase
#[derive(Component)]
pub struct Homebase {
    pub owner: Entity, // player entity
}

/// Marker: this map is a competition arena
#[derive(Component)]
pub struct Arena {
    pub id: u64,
}
```

#### 2. ChunkTarget routing

The existing `ChunkTarget.map_entity` field already routes targets to specific maps. Multiple maps with different ChunkTargets "just work" — the lifecycle system already iterates `Query<(Entity, &mut VoxelMapInstance)>` and filters ChunkTargets by `map_entity`.

Verify and test that:
- Player A's ChunkTarget pointing at Overworld only spawns overworld chunks
- Player A's ChunkTarget pointing at their Homebase only spawns homebase chunks
- Switching a player between instances = changing their ChunkTarget.map_entity

#### 3. Instance lifecycle helpers

**File**: `crates/voxel_map_engine/src/instance.rs` (extend)

```rust
impl VoxelMapInstance {
    /// Spawn a new overworld instance
    pub fn overworld(seed: u64) -> (Self, VoxelMapConfig, Overworld) { ... }

    /// Spawn a new homebase instance for a player
    pub fn homebase(owner: Entity, bounds: IVec3) -> (Self, VoxelMapConfig, Homebase) { ... }

    /// Spawn a new arena instance
    pub fn arena(id: u64, seed: u64, bounds: IVec3) -> (Self, VoxelMapConfig, Arena) { ... }
}
```

#### 4. Bounded maps

For Homebase and Arena, `VoxelMapConfig.bounds` is `Some(IVec3)`. The lifecycle system must respect bounds — don't spawn chunks outside the bounded region.

#### 5. Multi-instance example

**File**: `crates/voxel_map_engine/examples/multi_instance.rs`

Bevy app that:
1. Spawns an overworld map at origin with flat terrain
2. Spawns a homebase map offset by `Transform::from_translation(Vec3::new(200.0, 0.0, 0.0))`
3. Spawns an arena map offset by `Transform::from_translation(Vec3::new(-200.0, 0.0, 0.0))`
4. Camera starts at overworld, keyboard shortcuts teleport between instances (update ChunkTarget.map_entity)
5. Each instance has different terrain (different SDF generators)

#### 6. VoxelWorld API for multi-instance

The `VoxelWorld` SystemParam already takes `map: Entity` for every operation. Verify:
- `get_voxel(overworld, pos)` and `get_voxel(homebase, pos)` return independent data
- `set_voxel(homebase, pos, voxel)` does not affect overworld
- `raycast(arena, ray, filter)` only hits arena voxels

#### 7. Server-side multi-instance (preparation)

Update server to demonstrate spawning a homebase:

```rust
fn spawn_player_homebase(
    mut commands: Commands,
    overworld: Res<OverworldMap>,
    // triggered when player connects
) {
    let homebase_entity = commands.spawn((
        VoxelMapInstance::homebase(player_entity, IVec3::new(8, 8, 8)),
        Transform::default(), // positioned in its own coordinate space
    )).id();
    // Player's ChunkTarget initially points at overworld
    // Portal system would later switch it to homebase_entity
}
```

This is preparation only — full portal/transition systems are out of scope for this plan.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` passes
- [ ] `cargo test -p voxel_map_engine` passes:
  - Multiple VoxelMapInstance entities coexist
  - ChunkTarget routing is correct (chunks only spawn for targeted map)
  - Bounded maps don't spawn chunks outside bounds
  - VoxelWorld API operations are instance-isolated
- [ ] `cargo test-all` passes

#### Manual Verification:
- [ ] `cargo run --example multi_instance -p voxel_map_engine` shows three distinct terrain instances
- [ ] Teleporting between instances shows different terrain
- [ ] Each instance loads/unloads chunks independently
- [ ] Server can spawn a homebase instance alongside the overworld (verified via log output)

---

## Testing Strategy

### Unit Tests (in `crates/voxel_map_engine/src/`):
- `types.rs`: PaddedChunkShape linearization, WorldVoxel→VoxelType conversions, ChunkData construction
- `instance.rs`: VoxelMapInstance creation, chunk insertion/lookup via OctreeI32
- `meshing.rs`: SDF→mesh conversion produces valid vertex/index data, empty SDF produces no mesh
- `mesh_cache.rs`: cache hit/miss, weak reference cleanup
- `raycast.rs`: ray hits flat plane, ray misses empty space, ray with filter, face normal correctness
- `api.rs`: get/set round-trip, write buffer accumulation, modified voxels persistence
- `lifecycle.rs`: chunk spawn within range, despawn outside range, bounded map enforcement

### Integration Tests (in `crates/voxel_map_engine/tests/`):
- Full Bevy App with VoxelPlugin, spawn map + target, verify chunks spawn after update ticks
- Multi-instance isolation: two maps, verify edits don't cross
- Save/load compatibility: VoxelType serialization matches existing format

### Existing Tests:
- `crates/server/tests/voxel_persistence.rs` — verify save/load still works
- `crates/server/tests/integration.rs` — verify message types still register correctly

## Debug Assertions

Use `debug_assert!` liberally throughout the crate to catch impossible states early during development. These compile away in release builds so have zero runtime cost. Key locations:

- **types.rs**: `ChunkData::new()` — assert voxel array length equals `PaddedChunkShape::SIZE`
- **instance.rs**: `VoxelMapInstance` operations — assert chunk coordinates are within octree bounds before insertion
- **lifecycle.rs**: `poll_chunk_tasks` — assert returned chunk position matches the position that was queued
- **lifecycle.rs**: `despawn_chunks` — assert chunk entity is actually a child of the expected map entity before despawning
- **lifecycle.rs**: `flush_write_buffer` — assert world position → chunk position conversion is reversible
- **meshing.rs**: SDF input array — assert length equals `PaddedChunkShape::SIZE`; assert mesh output has equal-length position/normal arrays
- **api.rs**: `get_voxel`/`set_voxel` — assert the `map: Entity` actually has a `VoxelMapInstance` component (the Query will already handle this, but assert on the unwrap path)
- **api.rs**: `set_voxel` — assert the voxel being set is not `WorldVoxel::Unset` (Unset is an internal sentinel, not a valid write value)
- **raycast.rs**: `voxel_line_traversal` — assert ray direction is not zero-length; assert start != end
- **chunk.rs**: `ChunkTarget` — assert `map_entity` is not `Entity::PLACEHOLDER`
- **config.rs**: `VoxelMapConfig` — assert `tree_height > 0`; assert `spawning_distance > 0`; assert bounded maps have all-positive bounds

Pattern: prefer `debug_assert!` with descriptive messages over silent early returns for conditions that indicate programmer error rather than runtime state.

## Performance Considerations

- **Async meshing**: Surface nets runs on `AsyncComputeTaskPool`, same as current bevy_voxel_world
- **Chunk size reduction**: 16³ vs current 32³ means 8× more chunks but 8× faster per-chunk meshing. Net effect should be neutral or positive due to better cache locality and finer-grained async work.
- **Mesh cache**: Same weak-reference dedup pattern. Surface nets output is less likely to produce identical meshes than greedy quads (smooth surfaces vary more), so cache hit rate may be lower.
- **OctreeI32 vs HashMap**: `grid-tree`'s octree provides O(1) cached access and spatial culling via `VisitCommand::SkipDescendants`. This is an improvement over the current flat HashMap for large worlds.
- **Max spawn throttle**: Carry forward `max_spawn_per_frame` from bevy_voxel_world (default 10000)

## Migration Notes

- Existing `world_save/voxel_world.bin` files remain compatible (same `VoxelType` enum, same bincode format)
- The `bevy_voxel_world` git submodule can be removed from `.gitmodules` after Phase 5 is verified
- The `VoxelType` type moves from `protocol::map` to `voxel_map_engine::types` (protocol re-exports it)
- `Chunk<MapWorld>` queries become `VoxelChunk` queries
- `ChunkRenderTarget<MapWorld>` becomes `ChunkTarget { map_entity, distance }`
- `VoxelWorld<MapWorld>` becomes `VoxelWorld` with explicit `map: Entity` parameter

## File Structure Summary

```
crates/voxel_map_engine/
├── Cargo.toml
├── src/
│   ├── lib.rs           # VoxelPlugin, module declarations, re-exports
│   ├── types.rs         # WorldVoxel, VoxelType, ChunkData, FillType, PaddedChunkShape
│   ├── instance.rs      # VoxelMapInstance component, Overworld/Homebase/Arena markers
│   ├── config.rs        # VoxelMapConfig component, VoxelGenerator type
│   ├── chunk.rs         # VoxelChunk + ChunkTarget components
│   ├── lifecycle.rs     # Chunk spawn/despawn/remesh systems, write buffer flush
│   ├── generation.rs    # Async SDF generation tasks
│   ├── meshing.rs       # VoxelMesher trait, SurfaceNetsMesher, (future: GreedyQuadsMesher)
│   ├── api.rs           # VoxelWorld SystemParam (get_voxel, set_voxel, raycast)
│   ├── raycast.rs       # VoxelFace, voxel_line_traversal, VoxelRaycastResult
│   ├── mesh_cache.rs    # WeakValueHashMap mesh dedup
│   └── shaders/         # (if keeping StandardVoxelMaterial later)
├── examples/
│   ├── terrain.rs       # Phase 2-3: terrain rendering
│   ├── editing.rs       # Phase 4: voxel editing
│   └── multi_instance.rs # Phase 6: multiple map instances
└── tests/
    └── integration.rs   # Full Bevy App integration tests
```

## Future Work: Physics Isolation via Avian Collision Hooks

Multi-instance maps (Phase 6) share a single Avian physics world. Without isolation, a character in the Overworld could collide with Homebase terrain if maps overlap spatially, and `SpatialQuery::cast_ray` (used for ground detection) has no per-map filtering.

This plan does **not** include physics isolation — it is a follow-up task. The recommended approach uses Avian's `CollisionHooks::filter_pairs` (available in avian3d 0.4.x) rather than `CollisionLayers`, which is limited to 32 bits.

### Approach: `MapInstanceId` + `filter_pairs`

Add a `MapInstanceId(Entity)` component to every physics entity (terrain chunks, characters, hitboxes, projectiles) indicating which `VoxelMapInstance` it belongs to. Register a `CollisionHooks` impl that rejects broad-phase pairs with mismatched instance IDs:

```rust
#[derive(Component)]
pub struct MapInstanceId(pub Entity);

#[derive(SystemParam)]
struct MapCollisionHooks<'w, 's> {
    map_ids: Query<'w, 's, &'static MapInstanceId>,
}

impl CollisionHooks for MapCollisionHooks<'_, '_> {
    fn filter_pairs(&self, e1: Entity, e2: Entity, _: &mut Commands) -> bool {
        match (self.map_ids.get(e1), self.map_ids.get(e2)) {
            (Ok(a), Ok(b)) => a.0 == b.0,
            _ => true, // entities without MapInstanceId use normal layer rules
        }
    }
}
```

Registered via `PhysicsPlugins::default().with_collision_hooks::<MapCollisionHooks>()`.

### Why this is compatible with the plan

- `VoxelChunk` entities are children of the map entity — propagating `MapInstanceId` is trivial.
- `ChunkTarget.map_entity` already tracks which map a player belongs to — that same entity becomes the `MapInstanceId` value.
- Rejected pairs skip narrow phase entirely, so cost is minimal.
- No layer limit — supports unlimited simultaneous map instances.

### Constraints

- Only one `CollisionHooks` impl per app. Future hooks (one-way platforms, conveyor belts) must share the same impl as additional queries on the same `SystemParam`.
- `CollisionHooks` requires `ReadOnlySystemParam` — writes must go through `Commands`.
- `ActiveCollisionHooks::FILTER_PAIRS` must be inserted on every entity that needs filtering.

### Changes needed (when implemented)

- `protocol`: Register `PhysicsPlugins` with `.with_collision_hooks::<MapCollisionHooks>()`
- `protocol`: Add `MapInstanceId` to `CharacterPhysicsBundle`
- `protocol/map.rs`: `attach_chunk_colliders` inserts `MapInstanceId` + `ActiveCollisionHooks::FILTER_PAIRS` on terrain chunks
- `server/gameplay.rs`: Set `MapInstanceId` on player spawn, update it on map transitions
- `protocol/ability.rs`: Propagate caster's `MapInstanceId` to hitboxes and projectiles
- `protocol/lib.rs`: Update `SpatialQuery::cast_ray` filter in `apply_movement` to use `SpatialQueryFilter::with_mask` or post-filter by `MapInstanceId`

## References

- Research: [doc/research/2026-02-27-bonsairobo-stack-multi-instance-voxel-replacement.md](doc/research/2026-02-27-bonsairobo-stack-multi-instance-voxel-replacement.md)
- Replacement audit: [doc/research/2026-02-03-bevy-voxel-world-replacement-audit.md](doc/research/2026-02-03-bevy-voxel-world-replacement-audit.md)
- Triplanar migration: [doc/research/2026-02-27-triplanar-splatting-bevy014-to-017-migration.md](doc/research/2026-02-27-triplanar-splatting-bevy014-to-017-migration.md)
- Current protocol map: [crates/protocol/src/map.rs](crates/protocol/src/map.rs)
- Current server map: [crates/server/src/map.rs](crates/server/src/map.rs)
- Current client map: [crates/client/src/map.rs](crates/client/src/map.rs)
- grid-tree API: [git/grid-tree-rs/src/tree.rs](git/grid-tree-rs/src/tree.rs)
- fast-surface-nets: [git/fast-surface-nets-rs/src/lib.rs](git/fast-surface-nets-rs/src/lib.rs)
- Amanatides & Woo source: [git/bevy_voxel_world/src/voxel_traversal.rs](git/bevy_voxel_world/src/voxel_traversal.rs)
- StandardVoxelMaterial source: [git/bevy_voxel_world/src/voxel_material.rs](git/bevy_voxel_world/src/voxel_material.rs)
