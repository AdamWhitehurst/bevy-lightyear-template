---
date: 2026-01-22 08:32:53 PST
researcher: Claude
git_commit: 5bb4f17febbc7530a8f124cb9a9a4c04f6b3bcd0
branch: master
repository: bevy-lightyear-template
topic: "Noise generation functionality for voxel world terrain and procedural object placement"
tags: [research, codebase, voxel-world, procedural-generation, noise, terrain]
status: complete
last_updated: 2026-01-22
last_updated_by: Claude
---

# Research: Noise Generation for Voxel Terrain and Procedural Object Placement

**Date**: 2026-01-22 08:32:53 PST
**Researcher**: Claude
**Git Commit**: 5bb4f17febbc7530a8f124cb9a9a4c04f6b3bcd0
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

What noise generation functionality currently exists in the codebase to support voxel world terrain generation and procedural placement of world objects like trees, houses, etc.?

## Summary

The codebase has comprehensive infrastructure for procedural terrain generation through the `bevy_voxel_world` library, but noise generation is currently used only in example code, not in the production server/client implementation. The production `MapWorld` configuration generates flat, deterministic terrain (solid below y=0, air above).

**Current state:**
- **Noise library**: `noise` crate v0.9.0 available in dev-dependencies
- **Example implementations**: Three example files demonstrate Perlin/HybridMulti noise for terrain heightmaps
- **Production terrain**: Flat procedural generation (no noise)
- **Procedural object placement**: No systems exist for placing trees, houses, or other world objects
- **Voxel modification system**: Full network-synchronized voxel editing with persistence

## Detailed Findings

### Noise Generation Library

**Location**: `git/bevy_voxel_world/Cargo.toml:46`

```toml
[dev-dependencies]
noise = "0.9.0"
```

The `noise` crate provides:
- `Perlin` noise implementation
- `HybridMulti` fractal noise (combines multiple octaves)
- `NoiseFn` trait for sampling noise at coordinates

Currently listed as dev-dependency, not available in production builds.

### Example Implementations

#### 1. Basic Noise Terrain

**Location**: `git/bevy_voxel_world/examples/noise_terrain.rs`

Demonstrates single-noise heightmap terrain generation:

```rust
let noise = HybridMulti::<Perlin>::new(1234)
    .set_octaves(5)
    .set_frequency(1.1)
    .set_lacunarity(2.8)
    .set_persistence(0.4);
```

**Key implementation pattern** (lines 80-119):
- Voxel lookup delegate returns closure sampling noise per position
- HashMap cache stores noise values per x/z column (expensive computation)
- Noise sampled at 1/1000 scale for terrain frequency control
- Sea level at y < 1 (noise below threshold becomes solid)

#### 2. LOD Noise Terrain

**Location**: `git/bevy_voxel_world/examples/noise_terrain_lod.rs`

Extends basic example with Level-of-Detail support:
- Stores noise generator in `Arc<HybridMulti<Perlin>>` MainWorld resource
- Same noise parameters as basic example
- Implements chunk boundary skirts using `WorldVoxel::Unset` for LOD transitions (lines 72-82)
- Voxel lookup delegate supports LOD level parameter (lines 50-107)

#### 3. Multiple Noise Sources

**Location**: `git/bevy_voxel_world/examples/multiple_noise_terrain.rs`

Demonstrates dual-noise system for terrain + vegetation:

```rust
// Terrain noise (seed 1234)
let terrain_noise = HybridMulti::<Perlin>::new(1234)
    .set_octaves(5)
    .set_frequency(1.1)
    .set_lacunarity(2.8)
    .set_persistence(0.4);

// Vegetation noise (seed 2345)
let vegetation_noise = HybridMulti::<Perlin>::new(2345)
    .set_octaves(3)
    .set_frequency(0.5)
    .set_lacunarity(2.0)
    .set_persistence(0.3);
```

**Key implementation** (lines 92-157):
- Samples both noise sources at different scales (1/1000 for terrain, 1/50 for vegetation)
- Caches both values in HashMap per x/z column
- Places different block types (Grass, Stone, Dirt, Snow) based on noise values and altitude
- Uses custom `BlockTexture` enum for material assignment

**Common pattern across examples:**
1. Initialize `HybridMulti<Perlin>` with seed and parameters
2. Implement `VoxelWorldConfig::voxel_lookup_delegate()`
3. Return boxed closure taking `IVec3` position
4. Use HashMap to cache noise per x/z column (avoid recomputing for vertical voxels)
5. Scale sampling coordinates to control terrain features

### Production Terrain Implementation

**Location**: `crates/protocol/src/map.rs:33-44`

Current `MapWorld` voxel configuration:

```rust
impl VoxelWorldConfig for MapWorld {
    type MaterialIndex = u8;
    type ChunkUserBundle = ();

    fn spawning_distance(&self) -> u32 {
        2
    }

    fn voxel_lookup_delegate(&self) -> VoxelLookupDelegate<Self::MaterialIndex> {
        Box::new(|_chunk_pos, _lod_level, _chunk_data| {
            Box::new(move |pos: IVec3, _previous| {
                if pos.y < 0 {
                    WorldVoxel::Solid(0)
                } else {
                    WorldVoxel::Air
                }
            })
        })
    }
}
```

**Characteristics:**
- Flat procedural generation (solid below y=0, air above)
- No noise sampling
- Deterministic (no randomness)
- Simple material index (all solid voxels use index 0)

**MapWorld resource** (lines 11-23):
```rust
#[derive(Resource, Clone)]
pub struct MapWorld {
    pub seed: u64,
    pub generation_version: u32,
}
```

Tracks seed and generation version for save compatibility, but doesn't use them for terrain generation yet.

### Voxel World Infrastructure

The production codebase has comprehensive systems supporting procedural generation:

#### Voxel Lookup Delegate Pattern

**Location**: `git/bevy_voxel_world/src/configuration.rs:8-14`

```rust
pub type VoxelLookupFn<I = u8> =
    Box<dyn FnMut(IVec3, Option<WorldVoxel<I>>) -> WorldVoxel<I> + Send + Sync>;
pub type LodLevel = u8;
pub type VoxelLookupDelegate<I = u8> =
    Box<dyn Fn(IVec3, LodLevel, Option<ChunkData<I>>) -> VoxelLookupFn<I> + Send + Sync>;
```

Higher-order function pattern allowing:
- Position-based voxel generation
- LOD level awareness
- Access to previous chunk data
- Thread-safe async generation

#### Chunk Generation System

**Location**: `git/bevy_voxel_world/src/chunk.rs:393-498`

`ChunkTask::generate()` flow:
1. Determines chunk data shape based on LOD and `ChunkRegenerateStrategy`
2. Iterates all voxel positions in chunk
3. Checks `ModifiedVoxels` map first (persists player edits)
4. Falls back to `voxel_lookup_delegate` for procedural generation
5. Optimizes storage using `FillType` (Empty/Mixed/Uniform)

#### Mesh Generation

**Location**: `git/bevy_voxel_world/src/meshing.rs:42-203`

`generate_chunk_mesh_for_shape()`:
- Greedy meshing via `block_mesh` crate
- Ambient occlusion calculation per vertex
- Texture coordinate mapping via `texture_index_mapper`
- LOD resampling (downsampling for distant chunks)

### Procedural Object Placement Systems

**Current state**: No procedural placement systems exist for world objects.

**Related systems found**:

#### Chunk Spawning Strategy

**Location**: `git/bevy_voxel_world/src/voxel_world_internal.rs:117-200`

Distance and visibility-based chunk spawning:
- Viewport raycasting from camera frustum
- Flood-fill algorithm for discovering chunks to spawn
- Configurable strategies: `CloseAndInView` vs `Close`
- Protected radius prevents despawn near targets
- Rate-limited spawning (`max_spawn_per_frame`)

**Chunk lifecycle components**:
- `ChunkRenderTarget<C>` marker component drives chunk visibility
- `ChunkThread<C, I>` wraps async generation tasks
- `NeedsRemesh` marks chunks requiring mesh regeneration
- `NeedsDespawn` marks chunks for removal

#### Voxel Modification System

**Location**: `crates/server/src/map.rs:265-341`

Network-synchronized voxel editing:

```rust
fn handle_voxel_edit_requests(
    mut receiver: Query<&mut MessageReceiver<VoxelEditRequest>>,
    mut sender: ServerMultiMessageSender,
    mut voxel_world: VoxelWorld<MapWorld>,
    // ...
) {
    for request in message_receiver.receive() {
        // Apply voxel change
        voxel_world.set_voxel(request.position, request.voxel.into());

        // Track modification
        modifications.modifications.push((request.position, request.voxel));

        // Broadcast to all clients
        sender.send::<_, VoxelChannel>(
            &VoxelEditBroadcast { position, voxel },
            server,
            &NetworkTarget::All,
        );
    }
}
```

**Features**:
- Request-based placement with server authority
- Modification tracking for persistence
- Debounced save system (1s debounce, 5s max dirty time)
- Network broadcasting to all clients

Could be extended to programmatically place voxels for objects, but currently only handles player input.

## Code References

### Noise Generation
- `git/bevy_voxel_world/Cargo.toml:46` - noise crate dependency
- `git/bevy_voxel_world/examples/noise_terrain.rs:80-119` - Basic noise terrain implementation
- `git/bevy_voxel_world/examples/noise_terrain_lod.rs:50-107` - LOD noise terrain delegate
- `git/bevy_voxel_world/examples/multiple_noise_terrain.rs:92-157` - Dual-noise terrain+vegetation

### Production Terrain
- `crates/protocol/src/map.rs:11-23` - MapWorld resource with seed/version
- `crates/protocol/src/map.rs:33-44` - Flat terrain voxel_lookup_delegate
- `crates/server/src/map.rs:144-261` - Save/load system for voxel modifications

### Voxel Infrastructure
- `git/bevy_voxel_world/src/configuration.rs:8-14` - VoxelLookupDelegate types
- `git/bevy_voxel_world/src/chunk.rs:393-498` - ChunkTask::generate()
- `git/bevy_voxel_world/src/meshing.rs:42-203` - Mesh generation
- `git/bevy_voxel_world/src/voxel_world_internal.rs:117-200` - Chunk spawning system

### Voxel Modification
- `crates/server/src/map.rs:265-341` - handle_voxel_edit_requests system
- `crates/server/src/map.rs:107-128` - VoxelModifications and VoxelDirtyState resources

## Architecture Documentation

### Noise-Based Terrain Generation Pattern (from examples)

```
Initialize HybridMulti<Perlin>
    ↓
Configure octaves, frequency, lacunarity, persistence
    ↓
Implement VoxelWorldConfig::voxel_lookup_delegate()
    ↓
Return Box<dyn Fn(IVec3, LodLevel, ChunkData) -> VoxelLookupFn>
    ↓
Inner closure samples noise at scaled coordinates
    ↓
Cache noise values in HashMap per x/z column
    ↓
Map noise value to WorldVoxel (Air or Solid with material index)
```

### Voxel Generation Data Flow

```
Chunk spawning system detects camera movement
    ↓
Creates ChunkTask for unloaded chunks
    ↓
ChunkTask::generate() called
    ├─ Check ModifiedVoxels (player edits)
    ├─ Call voxel_lookup_delegate (procedural)
    └─ Optimize with FillType (Empty/Mixed/Uniform)
    ↓
Meshing system generates geometry
    ↓
Bevy entity spawned with Mesh + Material
```

### Network Synchronization Flow

```
Client sends VoxelEditRequest
    ↓
Server: handle_voxel_edit_requests
    ├─ Apply to VoxelWorld
    ├─ Track in VoxelModifications
    ├─ Mark VoxelDirtyState
    └─ Broadcast VoxelEditBroadcast to all clients
    ↓
Clients receive broadcast and apply locally
    ↓
Server debounced save (1s debounce, 5s max)
```

## Historical Context (from doc/)

### Voxel Persistence Architecture

**Document**: `doc/plans/2026-01-18-voxel-world-persistence.md`

Notes on procedural terrain and save system:
- Server terrain designed to be flat/deterministic
- Save system only tracks voxel modifications (delta from procedural base)
- Warns on misalignment if procedural generation changes (lines 734-736)
- Future procedural changes should update `generation_version` field

### Chunk Visibility Evolution

**Documents**:
- `doc/research/2026-01-03-bevy-ecs-chunk-visibility-patterns.md`
- `doc/plans/2026-01-04-transform-based-chunk-visibility.md`

Evolution from camera-only to transform-based chunk visibility:
- `ChunkRenderTarget<C>` component marks visibility drivers
- Supports multiple targets (e.g., player + spectator camera)
- Server-side chunk loading for physics/simulation

### Voxel World Plugin Architecture

**Documents**:
- `doc/plans/2025-12-24-voxel-map-plugins.md`
- `doc/research/2025-12-24-bevy-voxel-world-map-plugins.md`

Client-server plugin split:
- Client: `VoxelWorldPlugin<MapWorld>` with mesh rendering
- Server: `VoxelWorldPlugin<MapWorld>` minimal mode (no meshes)
- Shared `MapWorld` configuration in `protocol` crate

## Related Research

- `doc/research/2025-12-24-bevy-voxel-world-map-plugins.md` - MapPlugins architecture
- `doc/research/2026-01-17-voxel-world-save-load.md` - Save/load mechanisms
- `doc/research/2026-01-03-bevy-voxel-world-chunk-visibility-components.md` - Chunk visibility
- `doc/research/2026-01-09-raycast-chunk-collider-detection.md` - Raycast/collision

## Open Questions

None - research complete. The codebase has noise generation examples but doesn't use them in production, and has no procedural object placement systems.
