# Vox Model Loading & Rendering Pipeline — Implementation Plan

## Overview

Build the pipeline that turns `.vox` files into rendered, collidable meshes in-game. Currently the `world_object` system is fully wired — `.object.ron` definitions load, replicate via Lightyear, and spawn on clients — but `VisualKind::Vox(path)` is ignored and a placeholder collider-derived mesh is shown instead. This plan replaces that placeholder with actual vox model rendering using `dot_vox` + the existing `block-mesh-rs` greedy quads pipeline, with vertex colors, trimesh colliders, and LOD support.

## Current State Analysis

### What exists:
- **World object system** ([world_object/](crates/protocol/src/world_object/)): Full `.object.ron` loading, manifest pattern (WASM), hot-reload, `WorldObjectDefRegistry`, server spawn + Lightyear replication, client replication handler
- **`VisualKind` component** ([types.rs:28-39](crates/protocol/src/world_object/types.rs#L28-L39)): Defined with `Vox(String)` variant, stored on entities but never consumed
- **Client placeholder** ([world_object.rs:55-70](crates/client/src/world_object.rs#L55-L70)): `insert_placeholder_mesh` derives a green primitive mesh from `ColliderConstructor` — to be replaced
- **Greedy quads pipeline** ([meshing.rs:10-62](crates/voxel_map_engine/src/meshing.rs#L10-L62)): `mesh_chunk_greedy` converts padded voxel arrays → Bevy `Mesh` using `block-mesh-rs`
- **Voxel traits** ([types.rs:110-133](crates/voxel_map_engine/src/types.rs#L110-L133)): `WorldVoxel` implements `block_mesh::Voxel` + `MergeVoxel`
- **Reactive colliders** ([colliders.rs:1-40](crates/protocol/src/map/colliders.rs#L1-L40)): `attach_chunk_colliders` inserts `Collider::trimesh_from_mesh` on `Added<Mesh3d>` entities with `VoxelChunk` marker
- **Collision layers** ([layers.rs](crates/protocol/src/hit_detection/layers.rs)): `GameLayer` enum with `Terrain`, `Character`, `Damageable`, etc.
- **Asset models** ([assets/models/](assets/models/)): `.vox` files exist for trees, bushes, and other objects
- **TrackedAssets gate** ([app_state.rs](crates/protocol/src/app_state.rs)): All assets must load before `AppState::Ready`

### What's missing:
- `dot_vox` dependency
- `VoxModelAsset` type and `VoxModelLoader` (custom `AssetLoader` for `.vox` files)
- Voxel rasterization with Z-up → Y-up coordinate remapping
- Greedy quads meshing with vertex colors from palette
- LOD mesh generation (downsampling)
- `models.manifest.ron` for WASM
- Client system that reads `VisualKind::Vox` and attaches the loaded mesh
- Collider generation from vox mesh (trimesh) with fallback to RON `ColliderConstructor`

### Key Discoveries:
- `block-mesh-rs` is a local git dependency at `git/block-mesh-rs` (used as path dependency in crate Cargo.toml files)
- `ndshape` is also local at `git/ndshape-rs`, patched in workspace `[patch.crates-io]`
- The meshing loop in `mesh_chunk_greedy` uses `ConstShape` — vox models need `RuntimeShape` since dimensions vary per model
- `MergeVoxel::MergeValue = u8` (palette index) — greedy meshing won't merge faces with different palette indices, which is correct for vertex colors
- Server needs meshes for trimesh colliders — the `VoxModelLoader` must live in `protocol` (shared crate)

## Desired End State

- `.vox` files load via Bevy's asset system with hot-reload
- World objects with `VisualKind::Vox(path)` render with the actual vox mesh and vertex colors from the MagicaVoxel palette
- Each vox model has 2-3 LOD meshes; `VisibilityRange` handles distance-based switching
- Colliders use trimesh from the vox mesh when no `ColliderConstructor` is specified in the `.object.ron`; manual `ColliderConstructor` takes priority when present
- WASM builds load models via `models.manifest.ron`
- Server spawns world objects with trimesh colliders derived from the loaded vox mesh

### Verification:
- `cargo check-all` passes
- `cargo server` spawns world objects with correct trimesh colliders
- `cargo client` renders vox models with vertex colors at correct positions
- LOD transitions are visible when moving camera away from objects
- Hot-reload: modifying a `.vox` file updates the in-game mesh

## What We're NOT Doing

- Multi-model `.vox` files (scene graph) — one model per file, `models[0]`
- Translucent/alpha voxels — all voxels are opaque
- Animation of vox models
- `VisualKind::SpriteRig` or `VisualKind::Sprite` rendering (separate task)
- Custom materials or shaders — using `StandardMaterial` with vertex colors

## Implementation Approach

The work is split into 4 phases:
1. **Core**: `dot_vox` dependency, voxel type, meshing function with vertex colors
2. **Asset pipeline**: `VoxModelLoader`, LOD generation, manifest loading
3. **Server integration**: Trimesh colliders from vox mesh on server
4. **Client integration**: Replace placeholder with actual vox mesh, LOD entities with `VisibilityRange`

---

## Phase 1: Vox Voxel Type & Meshing Function

### Overview
Add `dot_vox` dependency. Create the vox-specific voxel type and meshing function in `protocol` that converts a `dot_vox::Model` + palette into a centered Bevy `Mesh` with vertex colors.

### Changes Required:

#### 1. Add `dot_vox` dependency
**File**: `Cargo.toml` (workspace)
**Changes**: Add `dot_vox` to workspace dependencies

```toml
[workspace.dependencies]
# ... existing deps ...
dot_vox = "5"
```

**File**: `crates/protocol/Cargo.toml`
**Changes**: Add `dot_vox` and `ndshape` as protocol dependencies

```toml
[dependencies]
# ... existing deps ...
dot_vox = { workspace = true }
ndshape = { path = "../../git/ndshape-rs" }
block-mesh = { path = "../../git/block-mesh-rs" }
```

#### 2. Create vox model module
**File**: `crates/protocol/src/vox_model/mod.rs` (new)

```rust
mod meshing;
mod types;

pub use meshing::mesh_vox_model;
pub use types::VoxModelVoxel;
```

#### 3. Vox voxel type with `Voxel` + `MergeVoxel` impls
**File**: `crates/protocol/src/vox_model/types.rs` (new)

```rust
/// A voxel in a .vox model. Palette index is the merge value so greedy meshing
/// preserves per-face color identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoxModelVoxel {
    Empty,
    /// Palette index (0–254). dot_vox uses 1-based indexing; subtract 1 before storing.
    Filled(u8),
}

impl block_mesh::Voxel for VoxModelVoxel {
    fn get_visibility(&self) -> block_mesh::VoxelVisibility {
        match self {
            Self::Empty => block_mesh::VoxelVisibility::Empty,
            Self::Filled(_) => block_mesh::VoxelVisibility::Opaque,
        }
    }
}

impl block_mesh::MergeVoxel for VoxModelVoxel {
    type MergeValue = u8;
    type MergeValueFacingNeighbour = u8;

    fn merge_value(&self) -> u8 {
        match self {
            Self::Filled(i) => *i,
            Self::Empty => 0,
        }
    }

    fn merge_value_facing_neighbour(&self) -> u8 {
        self.merge_value()
    }
}
```

#### 4. Meshing function: rasterize + greedy quads + vertex colors + centering
**File**: `crates/protocol/src/vox_model/meshing.rs` (new)

The meshing function:
1. Rasterizes sparse `dot_vox::Voxel` list into a padded dense array with Z-up → Y-up remap
2. Runs `greedy_quads` with `RIGHT_HANDED_Y_UP_CONFIG`
3. Builds Bevy `Mesh` with `ATTRIBUTE_POSITION`, `ATTRIBUTE_NORMAL`, `ATTRIBUTE_COLOR` (linear RGBA from palette), and indices
4. Centers the mesh at origin by offsetting vertex positions

```rust
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use block_mesh::{greedy_quads, GreedyQuadsBuffer, RIGHT_HANDED_Y_UP_CONFIG};
use ndshape::RuntimeShape;

use super::types::VoxModelVoxel;

/// Meshes a `dot_vox::Model` into a Bevy `Mesh` with vertex colors from the palette.
///
/// Coordinate remap: MagicaVoxel Z-up → Bevy Y-up (X→X, Y→Z, Z→Y).
/// The mesh is centered at origin.
pub fn mesh_vox_model(model: &dot_vox::Model, palette: &[dot_vox::Color]) -> Option<Mesh> {
    let (voxels, shape, center_offset) = rasterize_model(model);
    let buffer = run_greedy_quads(&voxels, &shape);

    if buffer.quads.num_quads() == 0 {
        return None;
    }

    let linear_palette = precompute_linear_palette(palette);
    Some(build_mesh(&buffer, &linear_palette, center_offset))
}

/// Rasterizes sparse dot_vox voxels into a padded dense array with axis remap.
fn rasterize_model(
    model: &dot_vox::Model,
) -> (Vec<VoxModelVoxel>, RuntimeShape<u32, 3>, Vec3) {
    // Padded dimensions (1-voxel border on each side)
    let sx = model.size.x + 2;
    let sy = model.size.z + 2; // Z-up → Y-up
    let sz = model.size.y + 2; // Y-forward → Z-forward

    let shape = RuntimeShape::<u32, 3>::new([sx, sy, sz]);
    let mut voxels = vec![VoxModelVoxel::Empty; shape.size() as usize];

    for v in &model.voxels {
        let idx = shape.linearize([
            v.x as u32 + 1,
            v.z as u32 + 1, // Z → Y
            v.y as u32 + 1, // Y → Z
        ]);
        voxels[idx as usize] = VoxModelVoxel::Filled(v.i);
    }

    let center = Vec3::new(
        model.size.x as f32 / 2.0,
        model.size.z as f32 / 2.0,
        model.size.y as f32 / 2.0,
    );

    (voxels, shape, center)
}

fn run_greedy_quads(
    voxels: &[VoxModelVoxel],
    shape: &RuntimeShape<u32, 3>,
) -> GreedyQuadsBuffer {
    let dims = shape.as_array();
    let mut buffer = GreedyQuadsBuffer::new(voxels.len());
    let faces = RIGHT_HANDED_Y_UP_CONFIG.faces;
    greedy_quads(
        voxels,
        shape,
        [0; 3],
        [dims[0] - 1, dims[1] - 1, dims[2] - 1],
        &faces,
        &mut buffer,
    );
    buffer
}

fn build_mesh(
    buffer: &GreedyQuadsBuffer,
    linear_palette: &[[f32; 4]],
    center_offset: Vec3,
) -> Mesh {
    let faces = RIGHT_HANDED_Y_UP_CONFIG.faces;
    let num_vertices = buffer.quads.num_quads() * 4;
    let num_indices = buffer.quads.num_quads() * 6;

    let mut positions = Vec::with_capacity(num_vertices);
    let mut normals = Vec::with_capacity(num_vertices);
    let mut colors = Vec::with_capacity(num_vertices);
    let mut indices = Vec::with_capacity(num_indices);

    for (group, face) in buffer.quads.groups.iter().zip(faces.iter()) {
        for quad in group.iter() {
            let base = positions.len() as u32;
            indices.extend_from_slice(&face.quad_mesh_indices(base));

            let quad_positions = face.quad_mesh_positions(quad, 1.0);
            for pos in &quad_positions {
                positions.push([
                    pos[0] - center_offset.x,
                    pos[1] - center_offset.y,
                    pos[2] - center_offset.z,
                ]);
            }

            normals.extend_from_slice(&face.quad_mesh_normals());

            let palette_idx = quad.merge_value_contributing as usize;
            let color = linear_palette
                .get(palette_idx)
                .copied()
                .unwrap_or([1.0, 0.0, 1.0, 1.0]); // magenta fallback
            colors.extend_from_slice(&[color; 4]);
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.try_insert_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .expect("valid position attribute");
    mesh.try_insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .expect("valid normal attribute");
    mesh.try_insert_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .expect("valid color attribute");
    mesh.try_insert_indices(Indices::U32(indices))
        .expect("valid indices");
    mesh
}

/// Pre-converts the dot_vox sRGB palette to linear RGBA f32 for vertex colors.
fn precompute_linear_palette(palette: &[dot_vox::Color]) -> Vec<[f32; 4]> {
    palette.iter().map(|c| {
        [
            srgb_to_linear(c.r),
            srgb_to_linear(c.g),
            srgb_to_linear(c.b),
            c.a as f32 / 255.0,
        ]
    }).collect()
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
```

#### 5. Register module
**File**: `crates/protocol/src/lib.rs`
**Changes**: Add `pub mod vox_model;`

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` passes

#### Manual Verification:
- [x] Unit test: create a tiny 2x2x2 `dot_vox::Model` programmatically, call `mesh_vox_model`, verify mesh has expected vertex count and colors

---

## Phase 2: Asset Pipeline — VoxModelLoader, LOD, Manifest

### Overview
Implement the Bevy `AssetLoader` for `.vox` files that produces a `VoxModelAsset` with LOD mesh sub-assets. Add `models.manifest.ron` for WASM and integrate with `TrackedAssets`.

### Changes Required:

#### 1. VoxModelAsset type and loader
**File**: `crates/protocol/src/vox_model/mod.rs` (update)

```rust
mod loader;
mod loading;
mod lod;
mod meshing;
mod types;

pub use loader::VoxModelAsset;
pub use meshing::mesh_vox_model;
pub use types::VoxModelVoxel;
```

**File**: `crates/protocol/src/vox_model/loader.rs` (new)

```rust
use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::reflect::TypePath;

use super::lod::generate_lod_meshes;
use super::meshing::mesh_vox_model;

/// A loaded .vox model with LOD mesh sub-assets.
///
/// LOD 0 is full resolution. Each subsequent level halves resolution via 2x2x2 downsampling.
/// Mesh handles are labeled sub-assets: `"mesh_lod0"`, `"mesh_lod1"`, etc.
#[derive(Asset, TypePath)]
pub struct VoxModelAsset {
    /// LOD mesh handles, index 0 = full resolution.
    pub lod_meshes: Vec<Handle<Mesh>>,
    /// Model dimensions in voxels (Bevy Y-up space).
    pub size: UVec3,
}

/// Custom asset loader for `.vox` files.
///
/// Parses via `dot_vox`, generates greedy-meshed Bevy `Mesh` with vertex colors,
/// and produces 2-3 LOD levels as labeled sub-assets.
#[derive(TypePath)]
pub(super) struct VoxModelLoader;

impl AssetLoader for VoxModelLoader {
    type Asset = VoxModelAsset;
    type Settings = ();
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn extensions(&self) -> &[&str] {
        &["vox"]
    }

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        load_context: &mut LoadContext<'_>,
    ) -> Result<VoxModelAsset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let data = dot_vox::load_bytes(&bytes)
            .map_err(|e| format!("dot_vox parse error: {e}"))?;
        let model = &data.models[0];

        let lod_meshes = generate_lod_meshes(model, &data.palette, load_context);

        let size = UVec3::new(model.size.x, model.size.z, model.size.y); // Z-up → Y-up

        Ok(VoxModelAsset { lod_meshes, size })
    }
}
```

#### 2. LOD generation via downsampling
**File**: `crates/protocol/src/vox_model/lod.rs` (new)

Generates 2-3 LOD meshes from the full-res model. Each LOD level downsamples by 2x in each axis using majority-vote color selection.

```rust
use bevy::asset::LoadContext;
use bevy::prelude::*;
use std::collections::HashMap;

use super::meshing::mesh_vox_model_from_dense;
use super::types::VoxModelVoxel;

/// Generates LOD meshes and registers them as labeled sub-assets.
///
/// Returns handles to `"mesh_lod0"`, `"mesh_lod1"`, etc.
/// LOD 0 is full resolution. Stops when a dimension drops below 2.
pub fn generate_lod_meshes(
    model: &dot_vox::Model,
    palette: &[dot_vox::Color],
    load_context: &mut LoadContext<'_>,
) -> Vec<Handle<Mesh>> {
    let (dense, size) = rasterize_to_dense(model);
    let mut lod_meshes = Vec::new();

    // LOD 0: full resolution
    if let Some(mesh) = mesh_vox_model(&model, palette) {
        let handle = load_context.add_labeled_asset("mesh_lod0".to_string(), mesh);
        lod_meshes.push(handle);
    }

    // Subsequent LODs via downsampling
    let mut current = dense;
    let mut current_size = size;
    let mut level = 1u32;

    while current_size.min_element() >= 4 && level <= 2 {
        let (downsampled, new_size) = downsample_2x(&current, current_size);
        if let Some(mesh) = mesh_vox_model_from_dense(&downsampled, new_size, palette) {
            let label = format!("mesh_lod{level}");
            let handle = load_context.add_labeled_asset(label, mesh);
            lod_meshes.push(handle);
        }
        current = downsampled;
        current_size = new_size;
        level += 1;
    }

    lod_meshes
}

/// Rasterizes a dot_vox model into a dense 3D array (Y-up, no padding).
fn rasterize_to_dense(model: &dot_vox::Model) -> (Vec<VoxModelVoxel>, UVec3) {
    let size = UVec3::new(model.size.x, model.size.z, model.size.y);
    let len = (size.x * size.y * size.z) as usize;
    let mut voxels = vec![VoxModelVoxel::Empty; len];

    for v in &model.voxels {
        let x = v.x as u32;
        let y = v.z as u32;
        let z = v.y as u32;
        let idx = x + y * size.x + z * size.x * size.y;
        voxels[idx as usize] = VoxModelVoxel::Filled(v.i);
    }

    (voxels, size)
}

/// Downsamples a dense voxel array by 2x in each axis using majority-vote color.
fn downsample_2x(voxels: &[VoxModelVoxel], size: UVec3) -> (Vec<VoxModelVoxel>, UVec3) {
    let new_size = size / 2;
    let len = (new_size.x * new_size.y * new_size.z) as usize;
    let mut result = vec![VoxModelVoxel::Empty; len];

    for z in 0..new_size.z {
        for y in 0..new_size.y {
            for x in 0..new_size.x {
                let mut counts: HashMap<u8, u8> = HashMap::new();
                for dz in 0..2u32 {
                    for dy in 0..2u32 {
                        for dx in 0..2u32 {
                            let sx = x * 2 + dx;
                            let sy = y * 2 + dy;
                            let sz = z * 2 + dz;
                            let idx = sx + sy * size.x + sz * size.x * size.y;
                            if let VoxModelVoxel::Filled(i) = voxels[idx as usize] {
                                *counts.entry(i).or_default() += 1;
                            }
                        }
                    }
                }
                if let Some((&color, _)) = counts.iter().max_by_key(|(_, &c)| c) {
                    let idx = x + y * new_size.x + z * new_size.x * new_size.y;
                    result[idx as usize] = VoxModelVoxel::Filled(color);
                }
            }
        }
    }

    (result, new_size)
}
```

This requires adding a `mesh_vox_model_from_dense` function to `meshing.rs` that takes a pre-rasterized dense array + size instead of a `dot_vox::Model`. The existing `mesh_vox_model` delegates to it internally after rasterization.

**File**: `crates/protocol/src/vox_model/meshing.rs` (update)
Add `pub fn mesh_vox_model_from_dense(voxels: &[VoxModelVoxel], size: UVec3, palette: &[dot_vox::Color]) -> Option<Mesh>` that pads the dense array and runs the same greedy quads + vertex color pipeline.

**Implementation note**: `mesh_vox_model` was kept as-is (not refactored into a thin wrapper) because the double-rasterization for LOD 0 is negligible at load time. `mesh_vox_model_from_dense` was added as a separate function that pads a dense unpadded array before running greedy quads.

#### 3. Loading systems (manifest + TrackedAssets)
**File**: `crates/protocol/src/vox_model/loading.rs` (new)

Follow the exact pattern from `world_object/loading.rs`:

```rust
// Native: load_folder("models") for all .vox files
// WASM: load models.manifest.ron → individual loads
// Both: add handles to TrackedAssets
```

Resource types:
- `VoxModelFolderHandle` (native) / `VoxModelManifestHandle` (WASM)
- `PendingVoxModelHandles` (WASM)
- `VoxModelRegistry` — `HashMap<String, Handle<VoxModelAsset>>` keyed by path relative to `assets/`

Systems:
- `load_vox_models` (Startup) — loads folder or manifest
- `trigger_individual_vox_model_loads` (WASM, PreUpdate during Loading)
- `insert_vox_model_registry` (Update, until registry exists)
**Implementation note**: All early returns use `trace!()` per CLAUDE.md rules. The manifest-based approach is used on all platforms (not just WASM) because the `models/` directory contains non-vox files (`.obj`, `.mtl`, `.png`) that break `load_folder`. The trigger system is gated on `not(resource_exists::<VoxModelRegistry>)` rather than `in_state(AppState::Loading)` to avoid a race condition with asset processing order.

#### 4. Manifest file
**File**: `assets/models.manifest.ron` (new)

```ron
["trees/tree_circle.vox","trees/tree_square.vox","trees/tree_tall.vox","bushes/bush_circle.vox","bushes/bush_square.vox","bushes/bush_rectangle.vox"]
```

Lists all `.vox` files under `models/` that should be loaded. Updated manually when models are added/removed.

#### 5. Plugin registration
**File**: `crates/protocol/src/vox_model/mod.rs` (update) — add `plugin` module
**File**: `crates/protocol/src/vox_model/plugin.rs` (new)

```rust
pub struct VoxModelPlugin;

impl Plugin for VoxModelPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<VoxModelAsset>();
        app.init_asset_loader::<VoxModelLoader>();

        // WASM: manifest loader via bevy_common_assets
        #[cfg(target_arch = "wasm32")]
        app.add_plugins(
            bevy_common_assets::ron::RonAssetPlugin::<VoxModelManifest>::new(&["models.manifest.ron"]),
        );

        app.add_systems(Startup, load_vox_models);

        #[cfg(target_arch = "wasm32")]
        app.add_systems(
            PreUpdate,
            trigger_individual_vox_model_loads
                .run_if(in_state(AppState::Loading)),
        );

        app.add_systems(
            Update,
            insert_vox_model_registry
                .run_if(not(resource_exists::<VoxModelRegistry>)),
        );
    }
}
```

**File**: `crates/protocol/src/lib.rs` — add `VoxModelPlugin` to `SharedGameplayPlugin`

#### 6. VoxModelManifest type
**File**: `crates/protocol/src/vox_model/loading.rs`

```rust
#[derive(Deserialize, Asset, TypePath)]
pub struct VoxModelManifest(pub Vec<String>);
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` passes
- [x] `cargo server` starts without errors (models load, registry populated)

#### Manual Verification:
- [x] Log output shows "Loaded 11 vox models" during startup
- [ ] Hot-reload: modify a `.vox` file → log shows reload message

---

## Phase 3: Server Integration — Trimesh Colliders from Vox Mesh

### Overview
When the server spawns a world object with `VisualKind::Vox`, generate a trimesh collider from the loaded vox mesh instead of relying solely on the manual `ColliderConstructor` in the `.object.ron`. If a `ColliderConstructor` is present in the RON, it takes priority.

### Changes Required:

#### 1. Update server spawn to attach vox-derived colliders
**File**: `crates/server/src/world_object.rs`
**Changes**: After spawning, check if the entity has a `ColliderConstructor`. If not, look up `VisualKind::Vox` path in `VoxModelRegistry`, get the LOD 0 mesh, and insert `Collider::trimesh_from_mesh`.

```rust
pub fn spawn_world_object(
    commands: &mut Commands,
    id: WorldObjectId,
    def: &WorldObjectDef,
    map_id: MapInstanceId,
    registry: &AppTypeRegistry,
    vox_registry: &VoxModelRegistry,
    meshes: &Assets<Mesh>,
) -> Entity {
    // ... existing spawn logic ...

    // If no ColliderConstructor in def, derive from vox mesh
    let has_collider_constructor = def.components.iter().any(|c| {
        c.reflect_type_path().contains("ColliderConstructor")
    });

    if !has_collider_constructor {
        if let Some(vox_path) = extract_vox_path(&def.components) {
            if let Some(mesh) = vox_registry.get_lod0_mesh(vox_path, meshes) {
                if let Some(collider) = Collider::trimesh_from_mesh(mesh) {
                    commands.entity(entity).insert((
                        collider,
                        RigidBody::Static,
                        terrain_collision_layers(),
                    ));
                }
            }
        }
    }

    entity
}
```
%% [VIOLATION] Coherence — Current `spawn_world_object` signature at crates/server/src/world_object.rs:15 takes 5 params, not 7. Actual: `(commands, id, def, map_id, registry)`. This pseudocode adds `vox_registry` and `meshes` params without showing where they come from. Need to specify the system that calls this function and how it acquires these resources.
%% [VIOLATION] Quality — Naming: `has_collider_constructor` uses a broad string match on `reflect_type_path()`. The actual pattern in crates/client/src/world_object.rs:26-29 uses `try_downcast_ref::<ColliderConstructor>()` which is type-safe. Use the same pattern for consistency.
%% [VIOLATION] Pattern — World object components are stored as `Vec<Box<dyn PartialReflect>>` in WorldObjectDef.components. The codebase uses `try_downcast_ref::<T>()` to extract specific types (see client/world_object.rs:29). Use this pattern instead of string matching on `reflect_type_path()`.

#### 2. Add `VoxModelRegistry` helper method
**File**: `crates/protocol/src/vox_model/loading.rs`

```rust
impl VoxModelRegistry {
    /// Returns the LOD 0 mesh for the given vox path (relative to assets/).
    pub fn get_lod0_mesh<'a>(
        &self,
        path: &str,
        meshes: &'a Assets<Mesh>,
    ) -> Option<&'a Mesh> {
        let asset_handle = self.models.get(path)?;
        // LOD 0 mesh is the labeled sub-asset "mesh_lod0"
        // Need access to Assets<VoxModelAsset> to get the handle
        None // placeholder — actual implementation needs Assets<VoxModelAsset>
    }
}
```

The actual pattern: `VoxModelRegistry` stores `Handle<VoxModelAsset>`. The server spawn system takes `Res<Assets<VoxModelAsset>>` + `Res<Assets<Mesh>>` as parameters, resolves `VoxModelAsset` → `lod_meshes[0]` → `Mesh`.
%% [SUGGESTION] Elegance — The helper method signature is incomplete. Actual pattern should be: `get_lod0_mesh(&self, path: &str, vox_assets: &Assets<VoxModelAsset>, meshes: &Assets<Mesh>) -> Option<&Mesh>`. This requires resolving VoxModelAsset first to get the mesh handle, then resolving that handle to the mesh. Consider if this helper adds value or if the call site should do the two-step lookup directly for clarity.

#### 3. Update server gameplay to pass new resources
**File**: `crates/server/src/gameplay.rs`
**Changes**: Update call sites of `spawn_world_object` to pass `Res<VoxModelRegistry>` and `Res<Assets<Mesh>>`.
%% [VIOLATION] Coherence — The plan doesn't specify which system in gameplay.rs calls spawn_world_object. Need to verify actual call site and specify the exact system to modify. From the signature analysis, spawn_world_object is called from other systems — find and document those call sites.

#### 4. Update `.object.ron` files (optional)
Remove `ColliderConstructor` from `.object.ron` files where the trimesh from the vox mesh is preferred. The cylinder collider currently on `tree_circle.object.ron` could be kept for performance (simpler collision shape) or removed to use the accurate trimesh.

**Decision**: Keep existing `ColliderConstructor` entries as-is. The trimesh fallback only activates when no `ColliderConstructor` is present. This lets us add new objects with just `VisualKind::Vox` and get automatic colliders.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` passes
- [ ] `cargo server` starts successfully

#### Manual Verification:
- [ ] Server log shows world objects spawning with colliders
- [ ] Objects with `ColliderConstructor` in RON use the manual collider
- [ ] Objects without `ColliderConstructor` use trimesh from vox mesh
- [ ] Characters collide correctly with world objects

---

## Phase 4: Client Integration — Vox Mesh Rendering & LOD

### Overview
Replace the placeholder mesh on the client with the actual vox model mesh and vertex colors. Spawn LOD entities with `VisibilityRange` for distance-based mesh switching.

### Changes Required:

#### 1. Replace placeholder mesh with vox mesh
**File**: `crates/client/src/world_object.rs`
**Changes**: Rewrite `on_world_object_replicated` to check `VisualKind`. For `Vox(path)`, load the `VoxModelAsset` and attach LOD meshes with `VisibilityRange`.

```rust
pub fn on_world_object_replicated(
    query: Query<(Entity, &WorldObjectId), Added<Replicated>>,
    registry: Res<WorldObjectDefRegistry>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    type_registry: Res<AppTypeRegistry>,
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, id) in &query {
        let Some(def) = registry.get(id) else {
            warn!("Replicated world object has unknown id: {:?}", id.0);
            continue;
        };

        let components = clone_def_components(def);
        apply_object_components(&mut commands, entity, components, type_registry.0.clone());

        // Attach visual based on VisualKind
        if let Some(vox_path) = extract_vox_path(&def.components) {
            attach_vox_visual(
                &mut commands,
                entity,
                vox_path,
                &vox_registry,
                &vox_assets,
                &mut materials,
            );
        }
    }
}
```
%% [VIOLATION] Coherence — Current signature at crates/client/src/world_object.rs:11-18 doesn't match. Actual params: `query`, `registry`, `type_registry`, `commands`, `meshes`, `materials`. This adds `vox_registry` and `vox_assets` but removes `meshes`. Need to specify if `meshes` should be removed or if both are needed.
%% [VIOLATION] Quality — Single responsibility: The function now does three things: 1) lookup definition, 2) apply components, 3) attach visual. The visual attachment should be a separate system that runs after component application. Consider splitting into: `apply_replicated_components` (applies def components) and `attach_vox_visuals` (queries for Added<VisualKind::Vox>, attaches meshes). This matches the reactive pattern used for colliders.
%% [SUGGESTION] Pattern — The existing placeholder pattern extracts ColliderConstructor via try_downcast_ref at line 26-29, then calls insert_placeholder_mesh. The new pattern should extract VisualKind the same way. Consider making extract_vox_path a helper that takes &[Box<dyn PartialReflect>] and uses try_downcast_ref::<VisualKind>() for type safety and consistency.

#### 2. LOD entity spawning with VisibilityRange
**File**: `crates/client/src/world_object.rs`

```rust
fn attach_vox_visual(
    commands: &mut Commands,
    entity: Entity,
    vox_path: &str,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    materials: &mut Assets<StandardMaterial>,
) {
    let Some(asset_handle) = vox_registry.get(vox_path) else {
        warn!("Vox model not found in registry: {vox_path}");
        return;
    };
    let Some(asset) = vox_assets.get(asset_handle) else {
        warn!("VoxModelAsset not yet loaded: {vox_path}");
        return;
    };

    let material = materials.add(StandardMaterial {
        ..default() // base_color WHITE passes vertex colors through
    });

    // LOD distance ranges (in world units)
    let lod_ranges: &[(f32, f32)] = &[
        (0.0, 30.0),   // LOD 0: 0–30m
        (30.0, 60.0),  // LOD 1: 30–60m
        (60.0, 120.0), // LOD 2: 60–120m
    ];
    let margin = 5.0; // crossfade margin

    for (i, mesh_handle) in asset.lod_meshes.iter().enumerate() {
        let (start, end) = lod_ranges.get(i).copied().unwrap_or((0.0, 200.0));

        commands.entity(entity).with_child((
            Mesh3d(mesh_handle.clone()),
            MeshMaterial3d(material.clone()),
            VisibilityRange {
                start_margin: start..(start + if i == 0 { 0.0 } else { margin }),
                end_margin: end..(end + margin),
                use_aabb: false,
            },
        ));
    }
}
```
%% [VIOLATION] Rules — CLAUDE.md System Design: "Expected early-out must include `trace!` explaining why". The two early returns for missing registry entry and missing asset need `trace!` logs explaining that this is expected during asset loading, OR an `expect()` if the early-out should not occur
%% [SUGGESTION] Elegance — Creating per-entity materials defeats the benefit of vertex colors. StandardMaterial::default() has base_color WHITE which passes vertex colors through. This material is identical for all vox objects. Consider using a shared VoxMaterial resource (like DefaultVoxelMaterial) to avoid allocating duplicate materials for every world object. Phase 4.4 mentions this but should be integrated here, not as an "optional optimization".

Each LOD mesh is a child entity of the world object with its own `VisibilityRange`. Bevy handles crossfade dithering automatically.

#### 3. Remove placeholder mesh code
**File**: `crates/client/src/world_object.rs`
**Changes**: Remove `insert_placeholder_mesh`, `collider_to_mesh` functions. The `VisualKind::Vox` branch replaces them. For `VisualKind::None` or missing visual, no mesh is inserted. `VisualKind::SpriteRig` and `VisualKind::Sprite` remain unhandled (out of scope — can keep a placeholder or skip).
%% [SUGGESTION] Quality — Error handling: What happens when VisualKind::Vox is specified but the asset fails to load or doesn't exist? Current code logs warning and returns. Consider if a fallback visual (colored box/sphere) would help debugging. The placeholder mesh system could be kept as a fallback for missing vox models instead of complete removal.

#### 4. Shared vertex-color material (optional optimization)
Since all vox models use the same `StandardMaterial` (white base_color, default PBR), a single shared material handle avoids redundant allocations. Add a `DefaultVoxMaterial(Handle<StandardMaterial>)` resource initialized during startup.

**File**: `crates/client/src/world_object.rs` or `crates/render/src/lib.rs`

```rust
#[derive(Resource)]
pub struct DefaultVoxMaterial(pub Handle<StandardMaterial>);

fn init_default_vox_material(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(DefaultVoxMaterial(
        materials.add(StandardMaterial::default()),
    ));
}
```
%% [SUGGESTION] Pattern — The codebase already has DefaultVoxelMaterial for chunk rendering in crates/voxel_map_engine/src/lifecycle.rs:15-29. Follow the exact same pattern: name it DefaultVoxModelMaterial or similar, initialize in VoxModelPlugin or client plugin, use it in attach_vox_visual. Consistency with existing resource naming and initialization is important.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` passes
- [ ] `cargo server` starts successfully
- [ ] `cargo client` starts and connects successfully

#### Manual Verification:
- [ ] World objects render with vox mesh and correct vertex colors (not green placeholder)
- [ ] Trees/bushes visible in-game with proper palette colors
- [ ] LOD transitions visible when moving camera: high-detail mesh up close, lower-detail at distance
- [ ] No visual popping — crossfade dithering smooth at LOD boundaries
- [ ] Hot-reload: edit `.vox` file in MagicaVoxel → save → mesh updates in-game
- [ ] Objects with `ColliderConstructor` in RON still use that collider shape
- [ ] Performance acceptable with multiple world objects on screen

---

## Testing Strategy

### Unit Tests:
- `mesh_vox_model` with a hand-crafted 2x2x2 model: verify vertex count (quads × 4), index count (quads × 6), vertex colors match palette
- `downsample_2x`: 4x4x4 → 2x2x2, verify majority vote selects correct palette index
- `srgb_to_linear`: spot-check known values (0 → 0.0, 255 → 1.0, 128 → ~0.216)
- Coordinate remap: verify a voxel at MagicaVoxel `(x=1, y=2, z=3)` ends up at Bevy `(1, 3, 2)` in the dense array

### Integration Tests:
- Load a real `.vox` file via `VoxModelLoader` in a minimal Bevy app, verify `VoxModelAsset` has expected LOD count and mesh handles
- `WorldObjectDefRegistry` + `VoxModelRegistry` both populate before `AppState::Ready`

### Manual Testing Steps:
1. `cargo server` — verify world objects spawn with colliders, log shows model count
2. `cargo client` — connect, verify trees/bushes render with colors
3. Move camera close/far to observe LOD transitions
4. Open a `.vox` file in MagicaVoxel, change a color, save — verify hot-reload
5. Remove `ColliderConstructor` from one `.object.ron`, verify trimesh collider works

## Performance Considerations

- **Greedy meshing at load time**: Mesh generation happens once during asset loading, not per-frame. LOD meshes also generated at load time.
- **LOD reduces draw calls**: Distant objects use coarser meshes with fewer triangles.
- **Shared material**: All vox objects share one `StandardMaterial` — single draw call batch potential.
- **Trimesh colliders are heavier than primitives**: Keep `ColliderConstructor` in RON for frequently-collided objects (e.g., trees the player runs into). Trimesh is the fallback for objects without explicit colliders.
- **Memory**: Each LOD mesh is a separate `Mesh` asset. For small models (< 64³), this is negligible.

## References

- Research: [doc/research/2026-03-13-vox-model-loading-rendering-pipeline.md](doc/research/2026-03-13-vox-model-loading-rendering-pipeline.md)
- Earlier research: [doc/research/2026-03-13-vox-loading-without-scenes.md](doc/research/2026-03-13-vox-loading-without-scenes.md)
- Ability loading pattern: [crates/protocol/src/ability/loading.rs](crates/protocol/src/ability/loading.rs)
- World object system: [crates/protocol/src/world_object/](crates/protocol/src/world_object/)
- Existing greedy quads: [crates/voxel_map_engine/src/meshing.rs](crates/voxel_map_engine/src/meshing.rs)
- Reactive colliders: [crates/protocol/src/map/colliders.rs](crates/protocol/src/map/colliders.rs)
