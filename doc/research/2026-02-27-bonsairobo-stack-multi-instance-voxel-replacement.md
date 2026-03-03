---
date: 2026-02-27T16:29:24-08:00
researcher: Claude
git_commit: da4b04bc61eab50383fb714815905477e42156db
branch: master
repository: bevy-lightyear-template
topic: "How to use bonsairobo crates to replace bevy_voxel_world with multi-instance map support"
tags: [research, codebase, voxel, bonsairobo, grid-tree, block-mesh, multi-instance, replacement]
status: complete
last_updated: 2026-02-27
last_updated_by: Claude
last_updated_note: "Resolved open questions: smooth meshing primary, block-mesh stubbed; StandardVoxelMaterial; extract raycasting; 16³ chunks; accept LOD seams; update glam to 0.29"
---

# Research: Using Bonsairobo Crates to Replace `bevy_voxel_world` with Multi-Instance Map Support

**Date**: 2026-02-27T16:29:24-08:00
**Researcher**: Claude
**Git Commit**: da4b04bc61eab50383fb714815905477e42156db
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to use the crates from bonsairobo's stack (available under `git/`) to replace `bevy_voxel_world` with support for multiple map instances existing at once (infinite procedurally-generated overworld, many finite homebase instances, finite procedurally-generated arena instances) per VISION.md.

## Summary

The bonsairobo stack provides low-level, composable primitives — not a turnkey voxel engine. Replacing `bevy_voxel_world` means building a custom voxel engine layer on top of these crates. The stack decomposes into three layers: **data** (`ndshape`, `ndcopy`), **spatial structure** (`grid-tree`), and **meshing** (`block-mesh`, `fast-surface-nets`, `height-mesh`, `bevy_triplanar_splatting`). Each map instance would own its own `OctreeI32<ChunkData>` from `grid-tree`, and meshing would use `block-mesh`'s `greedy_quads` (for blocky terrain) or `fast-surface-nets` (for smooth terrain). Every map instance — overworld, homebases, arenas — is a Bevy entity with a `VoxelMapInstance` component. Chunks are child entities parented to their map entity. This entity-based approach provides uniform handling across all world types: spawn/despawn map entities naturally, use the map entity's `Transform` to position instances in world space, and query all maps with a single `Query<&mut VoxelMapInstance>`.

---

## Detailed Findings

### 1. The Bonsairobo Crate Stack (Available in `git/`)

#### Foundation Layer

| Crate | Location | Purpose |
|-------|----------|---------|
| **ndshape** | `git/ndshape-rs/` | Linearization of N-D coordinates to flat array indices. Provides `ConstShape3u32<X,Y,Z>`, `RuntimeShape`, power-of-2 shapes with bit ops. Row-major: `linearize([x,y,z]) = x + X*y + X*Y*z`. |
| **ndcopy** | `git/ndcopy-rs/` | Efficient sub-region copy/fill between flat arrays. `copy3(shape, src, src_shape, src_start, dst, dst_shape, dst_start)` copies row-by-row (not element-by-element). |

These two crates are already used internally by `bevy_voxel_world` — switching to them directly just means using the same primitives without the wrapper.

#### Spatial Structure Layer

| Crate | Location | Purpose |
|-------|----------|---------|
| **grid-tree** | `git/grid-tree-rs/` | Generic octree mapping `(Level, IVec3) -> T`. Unbounded space via hash-mapped roots. O(1) access with cached `NodePtr`, O(depth) by coordinate. Supports multi-LOD via tree levels. |

The core type is `OctreeI32<T>` — an alias for `Tree<IVec3, OctreeShapeI32, T, 8>`. Level 0 stores finest-resolution chunks; higher levels store coarser LOD data. Each level doubles the coordinate space (parent coordinates = child coordinates >> 1).

**Not in `git/`**: `grid-ray` (Amanatides & Woo raycasting) and `ilattice` (integer lattice math) are not vendored. The project's `bevy_voxel_world` fork already has its own Amanatides & Woo implementation at `git/bevy_voxel_world/src/voxel_traversal.rs` that can be extracted.

#### Meshing Layer

| Crate | Location | Purpose |
|-------|----------|---------|
| **block-mesh** | `git/block-mesh-rs/` | Block/voxel meshing. `visible_block_faces` (fast, 1 quad/face) and `greedy_quads` (fewer quads, merges coplanar faces). Input: `&[T]` + `ndshape`. Output: quads grouped by face direction. |
| **fast-surface-nets** | `git/fast-surface-nets-rs/` | Smooth isosurface extraction from signed distance fields. ~20M triangles/sec. Output: positions, normals, indices. No UVs. |
| **height-mesh** | `git/height-mesh-rs/` | 2D heightmap → 3D triangle mesh. Single function `height_mesh()`. |
| **qef** | `git/qef/` | Quadratic error function minimizer for dual contouring vertex placement. Not a mesher itself — a building block for future dual contouring. |

#### Rendering Layer

| Crate | Location | Purpose |
|-------|----------|---------|
| **bevy_triplanar_splatting** | `git/bevy_triplanar_splatting/` | UV-less material for voxel meshes. Biplanar projection for color/PBR, triplanar for normals. Blends up to 4 material layers via `ATTRIBUTE_MATERIAL_WEIGHTS` (packed `u32` per vertex). Targets Bevy 0.14. |

#### Utility Crates (in `git/` but not voxel-specific)

| Crate | Location | Purpose |
|-------|----------|---------|
| **vector_expr** | `git/vector_expr/` | Vectorized math expression parser/evaluator (PEG grammar). Could be used for procedural generation expressions. |
| **rkyv_impl** | `git/rkyv_impl/` | Attribute macro for implementing methods on both `Foo` and `ArchivedFoo` (zero-copy serialization). |
| **smooth-bevy-cameras** | `git/smooth-bevy-cameras/` | Camera controllers with exponential smoothing. |
| **bevy_fsl_box_frame** | `git/bevy_fsl_box_frame/` | Box frame gizmo rendering. |
| **bevy_metrics_dashboard** | `git/bevy_metrics_dashboard/` | Bevy + metrics + egui_plot. |

### 2. Current `bevy_voxel_world` API Surface Used

From the existing replacement audit ([doc/research/2026-02-03-bevy-voxel-world-replacement-audit.md](doc/research/2026-02-03-bevy-voxel-world-replacement-audit.md)), the project uses:

| Feature | bonsairobo Replacement |
|---------|----------------------|
| `VoxelWorld<C>` SystemParam (`get_voxel`, `set_voxel`, `raycast`) | Custom SystemParam wrapping `Query<&mut VoxelMapInstance>` — takes `map_entity` to select instance |
| Chunk lifecycle (spawn/despawn by distance) | Custom systems using `grid-tree` to track which chunks exist |
| Async chunk generation | `AsyncComputeTaskPool` + `block-mesh` or `fast-surface-nets` |
| Greedy meshing with per-face AO | `block-mesh::greedy_quads` + custom AO (or `bevy_triplanar_splatting` for smooth) |
| Modified voxels persistence | Custom `HashMap<IVec3, WorldVoxel>` (same pattern) |
| Chunk map spatial index | `OctreeI32<ChunkData>` from `grid-tree` |
| Mesh cache (hash dedup) | Custom `WeakValueHashMap` (same pattern as current) |
| Raycasting (Amanatides & Woo) | Extract from `git/bevy_voxel_world/src/voxel_traversal.rs` |
| Custom WGSL material | Keep current `StandardVoxelMaterial` (defer `bevy_triplanar_splatting` port) |
| `ChunkRenderTarget<C>` | Custom component on entities whose Transform drives chunk loading |
| `Chunk<C>` component | Custom component with position, LOD level, mesh handle |
| `WorldVoxel` enum | Keep or replace with custom voxel type implementing `block-mesh::Voxel` |
| `VoxelWorldConfig` trait | `VoxelMapConfig` enum variants per world type |

### 3. Multi-Instance Architecture Mapping to VISION.md

VISION.md defines three distinct world types:

| World Type | Characteristics | Spatial Extent | Generation | Instance Count |
|------------|----------------|----------------|------------|----------------|
| **Overworld** | Shared persistent, infinite, procedural + editable | Unbounded | Noise-based terrain, deterministic seed | 1 per server |
| **Homebase** | Private per-player, finite, editable, customizable | Bounded (e.g. 8×8×8 chunks) | Flat/template + player edits | Many (1 per player) |
| **Arena** | Instanced, finite, procedurally generated | Bounded (varies) | Procedural per seed, may be editable during match | Many (created/destroyed dynamically) |

#### How `grid-tree` Maps to Each

**Overworld** — `OctreeI32<ChunkData>` with height 5+ (covers 2^5 = 32 chunk radius at root level). Unbounded because roots are hash-mapped. LOD levels useful here: level 0 = full detail near players, levels 1-4 = progressively coarser for distant terrain. Multiple `ChunkRenderTarget` entities (one per connected player) drive chunk loading.

**Homebase** — `OctreeI32<ChunkData>` with height 3-4 (covers 2^4 = 16 chunk radius, more than enough for a bounded homebase). Only one `ChunkRenderTarget` (the player). Could skip LOD (all level 0) since the space is small. All chunk data persisted.

**Arena** — `OctreeI32<ChunkData>` with height 3-4. Generated from a seed. Multiple `ChunkRenderTarget` entities (all participants). Ephemeral — no persistence needed. Could use different meshing (smooth terrain via `fast-surface-nets` for variety).

### 4. How Each Crate Composes in the Replacement

```
┌─────────────────────────────────────────────────────────────────┐
│                    Custom Voxel Engine Layer                     │
│  (replaces bevy_voxel_world)                                    │
│                                                                  │
│  Entity: VoxelMapInstance (Component)                            │
│    ├─ OctreeI32<ChunkData>     (grid-tree)   spatial index      │
│    ├─ ModifiedVoxels           (HashMap)      edit persistence   │
│    ├─ WriteBuffer              (Vec)          batched writes     │
│    ├─ MeshCache                (WeakHashMap)  mesh dedup         │
│    ├─ VoxelMapConfig           (enum)         gen/spawn params   │
│    └─ Transform                (Bevy)         world placement    │
│                                                                  │
│  Child Entities: Chunk components parented to map entity         │
│                                                                  │
│  Systems (operate on Query<&mut VoxelMapInstance>):              │
│    chunk_lifecycle       → spawn/despawn/LOD update              │
│    async_chunk_gen       → AsyncComputeTaskPool                  │
│    mesh_generation       → block-mesh or fast-surface-nets       │
│    write_buffer_flush    → apply edits, trigger remesh           │
│    collider_attachment   → avian3d trimesh from mesh             │
└───────────┬──────────────────┬──────────────────┬───────────────┘
            │                  │                  │
   ┌────────▼────────┐  ┌─────▼──────────┐  ┌───▼──────────────┐
   │   grid-tree     │  │   block-mesh   │  │  ndshape+ndcopy  │
   │   OctreeI32<T>  │  │   greedy_quads │  │  array indexing  │
   │   NodePtr, Key  │  │   Voxel trait  │  │  region copy     │
   └─────────────────┘  │   MergeVoxel   │  └──────────────────┘
                         ├────────────────┤
                         │fast-surface-nets│ (optional, for smooth terrain)
                         │  SignedDistance │
                         ├────────────────┤
                         │  height-mesh   │ (optional, for heightmap LOD)
                         └────────────────┘
```

### 5. Key Type Mappings

#### Voxel Data Type

Current `WorldVoxel<u8>` → implement `block-mesh::Voxel` + `block-mesh::MergeVoxel`:

```rust
// Satisfies block-mesh requirements
impl Voxel for WorldVoxel {
    fn get_visibility(&self) -> VoxelVisibility {
        match self {
            WorldVoxel::Air | WorldVoxel::Unset => VoxelVisibility::Empty,
            WorldVoxel::Solid(_) => VoxelVisibility::Opaque,
        }
    }
}

impl MergeVoxel for WorldVoxel {
    type MergeValue = u8;           // material index
    type MergeValueFacingNeighbour = u8;
    fn merge_value(&self) -> u8 { /* material index */ }
    fn merge_value_facing_neighbour(&self) -> u8 { /* same */ }
}
```

#### Chunk Data Storage

Current: `ChunkData<u8>` with `Arc<[WorldVoxel<u8>]>` of size 34³ (32³ + 1 padding each side).

With bonsairobo: 16³ chunks with 18³ padded shape, using `ndshape` explicitly:

```rust
type PaddedChunkShape = ConstShape3u32<18, 18, 18>;

struct ChunkData {
    voxels: Vec<WorldVoxel>,  // length = PaddedChunkShape::USIZE (5832)
    fill_type: FillType,       // Empty/Mixed/Uniform optimization
    hash: u64,                 // for mesh cache dedup
}
```

#### Spatial Index

Current: `ChunkMap<C, I>` — `Arc<RwLock<HashMap<IVec3, ChunkData>>>` with AABB tracking.

With `grid-tree`: `OctreeI32<Option<ChunkData>>` — the octree handles spatial hierarchy natively. Level 0 stores chunk data. Higher levels can store LOD summaries.

The key advantage: `grid-tree`'s `visit_tree_depth_first` with `VisitCommand::SkipDescendants` enables efficient frustum culling — skip entire octants that are out of view. This is free spatial acceleration that `HashMap` doesn't provide.

#### Meshing

Current: `block-mesh::visible_block_faces` via `bevy_voxel_world`'s `meshing.rs`.

Primary path: `fast-surface-nets::surface_nets` for smooth terrain:

```rust
fn mesh_chunk_smooth(sdf: &[f32]) -> Mesh {
    let mut buffer = SurfaceNetsBuffer::default();
    surface_nets(sdf, &PaddedChunkShape {}, [0; 3], [17; 3], &mut buffer);
    // buffer.positions, buffer.normals, buffer.indices → Bevy Mesh
}
```

Stubbed for future: `block-mesh::greedy_quads` for blocky terrain:

```rust
fn mesh_chunk_blocky(chunk: &ChunkData) -> Mesh {
    let mut buffer = GreedyQuadsBuffer::new(PaddedChunkShape::USIZE);
    greedy_quads(
        &chunk.voxels,
        &PaddedChunkShape {},
        [0; 3],
        [17; 3],  // max inclusive for 18³ padded shape
        &RIGHT_HANDED_Y_UP_CONFIG.faces,
        &mut buffer,
    );
    // Convert buffer.quads.groups into Bevy Mesh
    // using OrientedBlockFace::quad_mesh_positions/normals/indices
}
```

Both paths implement a common meshing trait so `VoxelMapConfig` can select the approach per instance.

### 6. Multi-Instance Design: Entity-Based

#### Why `bevy_voxel_world` Can't Support Multiple Instances

`bevy_voxel_world` multiplexes worlds via **generic type parameters** (`C: VoxelWorldConfig`). Each type `C` creates exactly one set of `Resource`s — one `ChunkMap<C>`, one `ModifiedVoxels<C>`, one `MeshCache<C>`, one `VoxelWorld<C>` SystemParam. So `VoxelWorldPlugin<Overworld>` gives you one overworld and `VoxelWorldPlugin<Homebase>` gives you one homebase. But 50 homebases would require 50 distinct Rust types (`Homebase1`, `Homebase2`, ...) each registered as a separate plugin — impossible at runtime. The type-level approach is fundamentally limited to a compile-time-known number of instances.

#### Entity-Based Approach

Every map instance is a Bevy entity with a `VoxelMapInstance` component. Chunks are child entities parented to their map entity. This replaces type-level multiplexing with ECS-level multiplexing — spawning 50 homebases is just 50 `commands.spawn(VoxelMapInstance::new(...))` calls. No new types, no new plugin registrations, no new system registrations. The same `Query<(Entity, &mut VoxelMapInstance)>` iterates all of them.

```rust
/// Configuration for a map instance
#[derive(Clone)]
enum VoxelMapConfig {
    Overworld {
        seed: u64,
        spawning_distance: u32,  // chunks
        lod_enabled: bool,
    },
    Homebase {
        owner: PlayerId,
        bounds: IVec3,           // max chunk coords
    },
    Arena {
        seed: u64,
        bounds: IVec3,
    },
}

/// Core component on every map entity
#[derive(Component)]
struct VoxelMapInstance {
    tree: OctreeI32<Option<ChunkData>>,
    modified_voxels: HashMap<IVec3, WorldVoxel>,
    write_buffer: Vec<(IVec3, WorldVoxel)>,
    mesh_cache: WeakValueHashMap<u64, Weak<Handle<Mesh>>>,
    config: VoxelMapConfig,
}

/// Marker components for filtering queries by world type
#[derive(Component)] struct Overworld;
#[derive(Component)] struct Homebase(PlayerId);
#[derive(Component)] struct Arena(ArenaId);

/// Chunk entity — child of its map entity
#[derive(Component)]
struct VoxelChunk {
    position: IVec3,
    lod_level: u8,
}
```

#### Spawning Map Instances

```rust
// Overworld — single entity, spawned at server start
fn spawn_overworld(mut commands: Commands) {
    commands.spawn((
        VoxelMapInstance::new(VoxelMapConfig::Overworld {
            seed: 12345,
            spawning_distance: 10,
            lod_enabled: true,
        }),
        Overworld,
        Transform::default(),
    ));
}

// Homebase — spawned per player
fn spawn_homebase(mut commands: Commands, player_id: PlayerId) {
    commands.spawn((
        VoxelMapInstance::new(VoxelMapConfig::Homebase {
            owner: player_id,
            bounds: IVec3::new(8, 8, 8),
        }),
        Homebase(player_id),
        Transform::from_translation(/* instance-specific offset or origin */),
    ));
}

// Arena — spawned per match
fn spawn_arena(mut commands: Commands, arena_id: ArenaId, seed: u64) {
    commands.spawn((
        VoxelMapInstance::new(VoxelMapConfig::Arena {
            seed,
            bounds: IVec3::new(16, 8, 16),
        }),
        Arena(arena_id),
        Transform::default(),
    ));
}
```

#### Why Entity-Based for Everything

- **Uniform systems**: A single `Query<(Entity, &mut VoxelMapInstance)>` handles chunk lifecycle for all map types. Type-specific behavior is driven by `VoxelMapConfig` variants, not separate system registrations.
- **Natural spawn/despawn**: Creating a homebase = `commands.spawn(...)`. Destroying an arena = `commands.entity(arena).despawn()`. No manual cleanup of resource maps.
- **Transform hierarchy**: Chunks are child entities of their map entity. The map's `Transform` positions the entire instance in world space. Moving a homebase to a different world-space location is just a `Transform` change.
- **Filtered queries**: Type-specific logic uses marker components — `Query<&VoxelMapInstance, With<Overworld>>` for overworld-only systems (e.g., LOD), `Query<&VoxelMapInstance, With<Homebase>>` for persistence.
- **Lightyear scoping**: Each map entity can carry lightyear visibility/relevance components, scoping network messages to players who are in that instance.

### 7. Chunk Lifecycle with `grid-tree`

Currently `bevy_voxel_world` handles chunk spawn/despawn internally. With `grid-tree` and entity-based maps, this becomes explicit:

```rust
/// ChunkTarget links an entity (player, camera) to the map it loads chunks for
#[derive(Component)]
struct ChunkTarget {
    map_entity: Entity,
    distance: u32,  // chunk radius
}

fn update_chunks(
    targets: Query<(&Transform, &ChunkTarget)>,
    mut maps: Query<(Entity, &mut VoxelMapInstance)>,
    mut commands: Commands,
) {
    for (map_entity, mut map) in &mut maps {
        // Gather all targets pointing at this map
        let target_positions: Vec<IVec3> = targets.iter()
            .filter(|(_, ct)| ct.map_entity == map_entity)
            .map(|(t, ct)| world_to_chunk(t.translation))
            .collect();

        // Spawn missing chunks within distance of any target
        for &center in &target_positions {
            let distance = map.config.spawning_distance();
            for coord in chunks_in_radius(center, distance) {
                let key = NodeKey { level: 0, coordinates: coord };
                if map.tree.find_node(key).is_none() {
                    map.tree.fill_path_to_node_from_root(key, |entry| {
                        entry.or_insert_with(|| None);
                        VisitCommand::Continue
                    });
                    // Spawn chunk as child entity of the map entity
                    let chunk_entity = commands.spawn((
                        VoxelChunk { position: coord, lod_level: 0 },
                    )).set_parent(map_entity).id();
                    // Queue async generation task
                }
            }
        }

        // Despawn chunks outside all targets' ranges
        // via visit_tree_depth_first with SkipDescendants for far octants
    }
}
```

The `VisitCommand::SkipDescendants` pattern is particularly powerful for despawning: traverse the octree, and if an entire octant is outside all targets' ranges, skip all its descendants in one operation.

### 8. Bevy Version Compatibility

| Crate | Bevy Version | Needs Port? |
|-------|-------------|-------------|
| `block-mesh` | No Bevy dependency | No |
| `fast-surface-nets` | No Bevy dependency | No |
| `ndshape` | No Bevy dependency | No |
| `ndcopy` | No Bevy dependency | No |
| `grid-tree` | No Bevy dependency (uses glam 0.25) | Glam version may need alignment |
| `height-mesh` | No Bevy dependency | No |
| `qef` | nalgebra (no Bevy) | No |
| `bevy_triplanar_splatting` | Bevy 0.14 | **Yes — needs port to 0.17** |

Most crates are pure data/algorithm libraries with no Bevy dependency. Only `bevy_triplanar_splatting` needs porting. The current project's `StandardVoxelMaterial` (from `bevy_voxel_world`) already works with Bevy 0.17 and could be kept initially, deferring the triplanar material port.

### 9. `grid-tree` API Details Relevant to This Use Case

**Construction:**
```rust
let tree: OctreeI32<Option<ChunkData>> = OctreeI32::new(5);
// height=5: levels 0-4, root level covers 2^4=16 chunks per axis
```

**Insert a chunk (ensuring ancestors exist):**
```rust
let key = NodeKey { level: 0, coordinates: IVec3::new(3, 0, 5) };
tree.fill_path_to_node_from_root(key, |entry| {
    entry.or_insert_with(|| None); // create empty ancestors
    VisitCommand::Continue
});
// Now set the actual chunk data at level 0
if let Some(relation) = tree.find_node(key) {
    *tree.get_value_mut(relation.child) = Some(chunk_data);
}
```

**O(1) chunk access (with cached pointer):**
```rust
let ptr: NodePtr = /* cached from insert/find */;
let data: &Option<ChunkData> = tree.get_value(ptr);
```

**Frustum-culled traversal:**
```rust
tree.visit_tree_depth_first(root_ptr, 0, |ptr, coords| {
    let aabb = chunk_aabb(coords, ptr.level);
    if !frustum.intersects(aabb) {
        return VisitCommand::SkipDescendants; // skip entire subtree
    }
    // Process visible chunk
    VisitCommand::Continue
});
```

**LOD via tree levels:**
Level 0 chunks cover 16³ voxels at full detail. Level 1 chunks cover 32³ voxels at half detail (same 16³ samples, each covering 2× area). Level 2 covers 64³, etc. Higher-level data can be generated by downsampling level 0 data using `ndcopy::copy3` with stride adjustments.

### 10. What Must Be Built Custom (Not Provided by Any Crate)

| Component | Description |
|-----------|-------------|
| **VoxelMapInstance component** | Owns `OctreeI32`, modified voxels, write buffer, mesh cache per map entity |
| **Chunk lifecycle systems** | Spawn/despawn/LOD update driven by ChunkTarget Transforms |
| **Async generation pipeline** | `AsyncComputeTaskPool` tasks that run voxel generation + meshing |
| **Write buffer flush system** | Batch `set_voxel` calls, trigger remesh on affected chunks |
| **Mesh-to-Bevy conversion** | Convert `GreedyQuadsBuffer`/`SurfaceNetsBuffer` → Bevy `Mesh` with positions, normals, UVs, AO, indices |
| **VoxelWorld SystemParam** | `get_voxel(map_entity, pos)`, `set_voxel(map_entity, pos, voxel)`, `raycast(map_entity, ray)` API |
| **Raycasting** | Extract Amanatides & Woo from `bevy_voxel_world` |
| **Collider integration** | Same as current `attach_chunk_colliders` system |
| **Instance management** | Create/destroy homebase/arena instances, assign players to instances |
| **Inter-instance transitions** | Move player entity between overworld and instance (portal system) |
| **Per-instance networking** | Scope lightyear messages to relevant instance |
| **Material/shader** | Keep current `StandardVoxelMaterial` (defer triplanar port) |

### 11. `block-mesh` Trait Requirements

To use `greedy_quads`, voxel types must implement two traits:

**`Voxel`** (required by both algorithms):
```rust
pub trait Voxel {
    fn get_visibility(&self) -> VoxelVisibility; // Empty, Translucent, Opaque
}
```

**`MergeVoxel`** (required by `greedy_quads` only):
```rust
pub trait MergeVoxel: Voxel {
    type MergeValue: Eq;
    type MergeValueFacingNeighbour: Eq;
    fn merge_value(&self) -> Self::MergeValue;
    fn merge_value_facing_neighbour(&self) -> Self::MergeValueFacingNeighbour;
}
```

Adjacent faces merge into one quad only if both `merge_value()` and the neighbor's `merge_value_facing_neighbour()` match. Setting `MergeValue = u8` (material index) preserves material boundaries in the mesh.

**Padding requirement**: Voxel arrays must include 1-voxel padding on all sides. For 16³ chunks → 18³ padded arrays using `ConstShape3u32<18, 18, 18>`.

### 12. `fast-surface-nets` for Smooth Terrain (Primary Meshing Path)

`fast-surface-nets` is the primary meshing approach. It provides:

**Input**: `&[f32]` signed distance field (negative = inside, positive = outside) + `ndshape::Shape`.

**Output**: `SurfaceNetsBuffer { positions, normals, indices, ... }`. Normals are gradient-estimated (not normalized — defer to GPU). No UVs generated.

**Chunk stitching**: The `eval-max-plane` feature flag (disabled by default) skips faces on positive chunk boundaries, so adjacent chunks mesh seamlessly.

**Material weights**: Not built-in. For multi-material terrain with `bevy_triplanar_splatting`, material weights would need to be computed from surrounding voxel data and attached as a custom vertex attribute.

**Configurable meshing**: The engine defines a meshing trait so `VoxelMapConfig` can select `fast-surface-nets` (default, smooth) or `block-mesh` (future, blocky) per instance. Both produce standard positions + normals + indices. `block-mesh` support is stubbed for future implementation.

---

## Code References

### Bonsairobo Crates
- `git/grid-tree-rs/src/tree.rs:167` — `Tree<V, S, T, CHILDREN>` core type
- `git/grid-tree-rs/src/impl_glam.rs:24` — `OctreeI32<T>` type alias
- `git/grid-tree-rs/src/tree.rs:493` — `fill_path_to_node_from_root` (insert with ancestor creation)
- `git/grid-tree-rs/src/tree.rs:670` — `visit_tree_depth_first` (frustum culling traversal)
- `git/grid-tree-rs/src/tree.rs:833-837` — `VisitCommand` enum (Continue/SkipDescendants)
- `git/block-mesh-rs/src/lib.rs:108-110` — `Voxel` trait
- `git/block-mesh-rs/src/greedy.rs:12-21` — `MergeVoxel` trait
- `git/block-mesh-rs/src/greedy.rs:61-80` — `greedy_quads` function
- `git/block-mesh-rs/src/simple.rs:13-32` — `visible_block_faces` function
- `git/block-mesh-rs/src/geometry.rs:171-183` — `RIGHT_HANDED_Y_UP_CONFIG`
- `git/fast-surface-nets-rs/src/lib.rs:126-145` — `surface_nets` function
- `git/fast-surface-nets-rs/src/lib.rs:54-56` — `SignedDistance` trait
- `git/ndshape-rs/src/lib.rs:101-114` — `Shape<N>` trait
- `git/ndshape-rs/src/const_shape.rs:50-86` — `ConstShape3u32` generation
- `git/ndcopy-rs/src/lib.rs:172-207` — `copy3` function
- `git/bevy_triplanar_splatting/src/triplanar_material.rs:24-75` — `TriplanarMaterial` type
- `git/bevy_triplanar_splatting/src/triplanar_material.rs:143-144` — `ATTRIBUTE_MATERIAL_WEIGHTS`

### Current Project Usage (to be replaced)
- `crates/protocol/src/map.rs:10-45` — `MapWorld` config and `VoxelWorldConfig` impl
- `crates/protocol/src/map.rs:94-121` — `attach_chunk_colliders` system
- `crates/client/src/map.rs:11-28` — `ClientMapPlugin` with `VoxelWorldPlugin`
- `crates/client/src/map.rs:65-116` — `handle_voxel_input` with raycast
- `crates/server/src/map.rs:13-110` — `ServerMapPlugin` with persistence
- `crates/server/src/map.rs:277-353` — `handle_voxel_edit_requests`
- `crates/server/src/gameplay.rs:57-69` — `ChunkRenderTarget` on entities
- `crates/protocol/src/lib.rs:167` — `ChunkRenderTarget` lightyear registration

### bevy_voxel_world Code to Extract/Reuse
- `git/bevy_voxel_world/src/voxel_traversal.rs` — Amanatides & Woo raycasting (reusable)
- `git/bevy_voxel_world/src/meshing.rs:126-198` — AO calculation (reusable pattern)
- `git/bevy_voxel_world/src/mesh_cache.rs` — WeakValueHashMap pattern (reusable)

## Architecture Documentation

### Dependency Graph

```
         ndshape ←── ndcopy
            ↑            ↑
        grid-tree    block-mesh ←── (ilattice)
            ↑            ↑
            └────────────┤
                         ↓
              Custom Voxel Engine Layer
              ┌──────────────────────┐
              │  VoxelMapInstance    │
              │  (Component on       │
              │   map entities)      │
              │                      │
              │  OctreeI32<Chunk>    │── grid-tree
              │  greedy_quads()      │── block-mesh
              │  surface_nets()      │── fast-surface-nets (optional)
              │  ConstShape3u32      │── ndshape
              │  copy3()             │── ndcopy
              │  TriplanarMaterial   │── bevy_triplanar_splatting (optional)
              └──────────────────────┘
                         ↑
              ┌──────────┴──────────┐
              │ Bevy Plugin Layer   │
              │ Systems, Events,    │
              │ lightyear messages  │
              └─────────────────────┘
```

### Instance Lifecycle

```
Overworld:   Created at server start → lives forever → persisted
Homebase:    Created on player first login → loaded on enter → persisted on exit
Arena:       Created on match start (from seed) → destroyed on match end → ephemeral
```

## Historical Context (from doc/)

- [doc/research/2026-02-03-bevy-voxel-world-replacement-audit.md](doc/research/2026-02-03-bevy-voxel-world-replacement-audit.md) — Complete audit of features to re-implement
- [doc/research/2026-01-22-noise-generation-voxel-terrain.md](doc/research/2026-01-22-noise-generation-voxel-terrain.md) — Noise generation patterns from bevy_voxel_world examples
- [doc/research/2025-12-24-bevy-voxel-world-map-plugins.md](doc/research/2025-12-24-bevy-voxel-world-map-plugins.md) — Original client/server plugin architecture
- [doc/research/2026-01-17-voxel-world-save-load.md](doc/research/2026-01-17-voxel-world-save-load.md) — Save/load architecture
- [doc/plans/2026-01-18-voxel-world-persistence.md](doc/plans/2026-01-18-voxel-world-persistence.md) — Persistence implementation plan

## Related Research

- [doc/research/2026-02-03-bevy-voxel-world-replacement-audit.md](doc/research/2026-02-03-bevy-voxel-world-replacement-audit.md) — Prerequisite: what to re-implement
- [Smooth Voxel Mapping (Medium article by bonsairobo)](https://bonsairobo.medium.com/smooth-voxel-mapping-a-technical-deep-dive-on-real-time-surface-nets-and-texturing-ef06d0f8ca14) — Deep dive on surface nets + triplanar texturing
- [building-blocks DESIGN.md](https://github.com/bonsairobo/building-blocks/blob/main/DESIGN.md) — Original architecture vision
- [feldspar](https://github.com/bonsairobo/feldspar) — Reference Bevy implementation (WIP) using the full stack
- [binary-greedy-meshing](https://github.com/Inspirateur/binary-greedy-meshing) — Alternative: ~30x faster greedy meshing than block-mesh

## Decisions (Resolved)

1. **Meshing approach**: `fast-surface-nets` (smooth) is the primary meshing path. The meshing layer should be trait-based/configurable so `block-mesh` (blocky) can be stubbed in as a future alternative, but smooth terrain ships first.

2. **Material/shader**: Keep `StandardVoxelMaterial` from `bevy_voxel_world` for now. Defer `bevy_triplanar_splatting` port to Bevy 0.17 until smooth meshing is working and needs proper multi-material blending.

3. **Raycasting**: Extract the Amanatides & Woo implementation from `git/bevy_voxel_world/src/voxel_traversal.rs` into the custom engine. Do not vendor `grid-ray`.

4. **Chunk size**: 16³ (with 18³ padded shape). Smaller chunks reduce per-chunk memory and generation time. `grid-tree`'s power-of-2 level scaling works cleanly. `PaddedChunkShape = ConstShape3u32<18, 18, 18>`.

5. **LOD transition meshes**: Accept seams at LOD boundaries for now. No transition mesh implementation — the standalone crates don't provide one and building one is out of scope for the initial replacement.

6. **`glam` version**: Update `grid-tree` fork to `glam 0.29` to match Bevy 0.17.
