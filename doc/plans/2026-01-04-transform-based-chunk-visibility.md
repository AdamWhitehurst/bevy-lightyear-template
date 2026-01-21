# Transform-Based Chunk Visibility Implementation Plan

## Overview

Refactor `bevy_voxel_world` to support transform-based chunk activeness logic, enabling servers and non-camera entities to control chunk spawning/despawning. This replaces the Camera-centric `VoxelWorldCamera` and `CameraInfo` with `ChunkRenderTarget` and `ChunkTarget` that use Transform as the primary input, with optional Camera for rendering-specific features.

## Current State Analysis

**Existing Implementation:**
- `VoxelWorldCamera<C>` component (voxel_world.rs:21-32): Marker component requiring Camera entities
- `CameraInfo<'w, 's, C>` SystemParam (voxel_world_internal.rs:45-48): Queries `(&Camera, &GlobalTransform)` with VoxelWorldCamera marker
- **Camera usage**: 85% position-only (GlobalTransform.translation()), 15% rendering-specific (viewport raycasting, frustum culling)
- **Key systems**: spawn_chunks (line 110), update_chunk_lods (line 315), retire_chunks (line 377)

**Key Discoveries:**
- Position extraction: voxel_world_internal.rs:146, 218, 260, 330, 396 - all use `cam_gtf.translation()`
- Viewport raycasting: voxel_world_internal.rs:148, 153 - uses `camera.physical_viewport_size()` and `camera.viewport_to_world()`
- Frustum culling: voxel_world_internal.rs:420-425, 757-806 - uses `camera.world_to_ndc()`
- Multi-source aggregation: spawn if visible to ANY target, despawn if invisible to ALL targets
- Nearest target for LOD: voxel_world_internal.rs:258-268, 335-348

## Desired End State

After this refactoring:
- Servers can spawn `ChunkRenderTarget` on Transform-only entities to control chunk activeness for connected clients
- Camera-based rendering continues to work with `ChunkRenderTarget` on Camera entities
- All existing examples work without modification (backward compatibility via type aliases)
- Documentation clearly explains the new architecture

**Verification:**
- All tests pass: `cargo test-all`
- All examples run: scripts test each example
- WASM builds successfully: `bevy run web`
- Deprecated warnings appear when using old type names

## What We're NOT Doing

- Not changing chunk spawning/despawning algorithms (only data sources)
- Not modifying LOD calculation logic
- Not adding new spawn/despawn strategies
- Not changing chunk meshing or data generation
- Not modifying the configuration trait
- Not breaking existing code (backward compatibility maintained)

## Implementation Approach

Replace Camera-centric types with Transform-centric types while maintaining full backward compatibility through type aliases. Use conditional logic to handle Camera-specific features (viewport raycasting, frustum culling) only when Camera is present.

## Phase 1: Core Type Refactoring

### Overview
Introduce new types and update internal systems to support optional Camera usage.

### Changes Required:

#### 1. Add ChunkRenderTarget Component
**File**: `git/bevy_voxel_world/src/voxel_world.rs`
**Changes**: Add new component after line 32

```rust
/// Marker component for entities that should control chunk spawning and despawning.
/// Add this to entities with Transform (and optionally Camera) to make them chunk visibility targets.
///
/// For camera-based rendering, add this to Camera entities alongside Camera and Transform.
/// For server-side chunk management, add this to Transform-only entities.
#[derive(Component)]
pub struct ChunkRenderTarget<C> {
    _marker: PhantomData<C>,
}

impl<C> Default for ChunkRenderTarget<C> {
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

/// Deprecated: Use ChunkRenderTarget instead
#[deprecated(since = "0.15.0", note = "Use ChunkRenderTarget instead")]
pub type VoxelWorldCamera<C> = ChunkRenderTarget<C>;
```

#### 2. Add ChunkTarget SystemParam
**File**: `git/bevy_voxel_world/src/voxel_world_internal.rs`
**Changes**: Replace CameraInfo definition at line 45-48

```rust
/// SystemParam providing access to all chunk render targets (entities with ChunkRenderTarget marker).
/// Queries optional Camera with required GlobalTransform.
#[derive(SystemParam, Deref)]
pub struct ChunkTarget<'w, 's, C: VoxelWorldConfig>(
    Query<'w, 's, (Option<&'static Camera>, &'static GlobalTransform), With<ChunkRenderTarget<C>>>,
);

/// Deprecated: Use ChunkTarget instead
#[deprecated(since = "0.15.0", note = "Use ChunkTarget instead")]
pub type CameraInfo<'w, 's, C> = ChunkTarget<'w, 's, C>;
```

#### 3. Update spawn_chunks System
**File**: `git/bevy_voxel_world/src/voxel_world_internal.rs`
**Changes**: Modify spawn_chunks function (lines 110-312)

**Key changes:**
- Line 116: Change parameter type from `CameraInfo<C>` to `ChunkTarget<C>`
- Line 122-125: Update early return message from "cameras" to "chunk render targets"
- Line 142: Collect `Vec<(Option<&Camera>, &GlobalTransform)>` instead of `Vec<(&Camera, &GlobalTransform)>`
- Lines 145-205: Wrap viewport raycasting logic in `if let Some(camera) = camera`
- Add radial spawning fallback for None camera case:

```rust
// Process each chunk target to collect chunks
for (camera_opt, target_gtf) in chunk_target.iter() {
    let target_position = target_gtf.translation();
    let target_pos = target_position.as_ivec3();

    if let Some(camera) = camera_opt {
        // Camera-based viewport raycasting (existing logic)
        let viewport_size = camera.physical_viewport_size().unwrap_or_default();
        // ... existing raycasting code ...
    } else {
        // Radial spawning for transform-only targets
        let chunk_at_target = target_pos / CHUNK_SIZE_I;
        for x in -spawning_distance..=spawning_distance {
            for y in -spawning_distance..=spawning_distance {
                for z in -spawning_distance..=spawning_distance {
                    let chunk_pos = chunk_at_target + IVec3::new(x, y, z);
                    let dist_sq = chunk_pos.distance_squared(chunk_at_target);
                    if dist_sq <= spawning_distance_squared {
                        chunks_deque.push_back(chunk_pos);
                    }
                }
            }
        }
    }

    // Force-spawn protected chunks (existing logic at lines 194-204)
    let chunk_at_target = target_pos / CHUNK_SIZE_I;
    let distance = configuration.min_despawn_distance() as i32;
    // ... rest of protected chunk logic ...
}
```

- Line 217-244: Update visibility check to handle None camera

```rust
// Check if chunk is visible to ANY target (when using CloseAndInView strategy)
if spawn_strategy == ChunkSpawnStrategy::CloseAndInView {
    let visible_to_any_target = targets.iter().any(|(camera_opt, target_gtf)| {
        let chunk_at_target = target_gtf.translation().as_ivec3() / CHUNK_SIZE_I;
        let is_protected = chunk_position.distance_squared(chunk_at_target)
            <= protected_chunk_radius_sq;

        if let Some(camera) = camera_opt {
            is_protected || chunk_visible_to_camera(
                camera,
                target_gtf,
                chunk_position,
                visibility_margin,
            )
        } else {
            // Transform-only targets: distance check only
            is_protected
        }
    });

    if !visible_to_any_target {
        continue;
    }
}
```

#### 4. Update update_chunk_lods System
**File**: `git/bevy_voxel_world/src/voxel_world_internal.rs`
**Changes**: Modify update_chunk_lods function (lines 315-374)

- Line 319: Change parameter type from `CameraInfo<C>` to `ChunkTarget<C>`
- Line 322-325: Update early return message
- Lines 327-331: Update to collect positions only (Camera not needed for LOD)

```rust
// Collect all chunk target positions
let target_positions: Vec<Vec3> = chunk_target
    .iter()
    .map(|(_, target_gtf)| target_gtf.translation())
    .collect();
```

- Lines 335-342: Rename camera_positions to target_positions (logic unchanged)

#### 5. Update retire_chunks System
**File**: `git/bevy_voxel_world/src/voxel_world_internal.rs`
**Changes**: Modify retire_chunks function (lines 377-453)

- Line 381: Change parameter type from `CameraInfo<C>` to `ChunkTarget<C>`
- Line 384-387: Update early return message
- Lines 393-399: Collect targets with optional Camera

```rust
// Collect chunk target data once
let targets: Vec<(Option<&Camera>, &GlobalTransform, IVec3)> = chunk_target
    .iter()
    .map(|(camera_opt, target_gtf)| {
        let target_pos = target_gtf.translation().as_ivec3();
        (camera_opt, target_gtf, target_pos)
    })
    .collect();
```

- Lines 405-437: Update visibility check to handle None camera

```rust
// Check visibility against ALL targets - only despawn if invisible to ALL
let visible_to_any_target = targets.iter().any(|(camera_opt, target_gtf, target_pos)| {
    let chunk_at_target = *target_pos / CHUNK_SIZE_I;

    // Check if chunk is near THIS target (protected from despawning)
    let dist_squared = chunk.position.distance_squared(chunk_at_target);
    let near_this_target = dist_squared
        <= (CHUNK_SIZE_I * configuration.min_despawn_distance() as i32).pow(2);

    // Check if chunk is within spawning distance
    let within_spawn_distance = dist_squared <= spawning_distance_squared + 1;

    // Check frustum visibility for this target
    let frustum_visible = match configuration.chunk_despawn_strategy() {
        ChunkDespawnStrategy::FarAway => true,
        ChunkDespawnStrategy::FarAwayOrOutOfView => {
            if let Some(camera) = camera_opt {
                let frustum_culled = !chunk_visible_to_camera(
                    camera,
                    target_gtf,
                    chunk.position,
                    0.0,
                );
                if let Some(visibility) = view_visibility {
                    visibility.get() && !frustum_culled
                } else {
                    !frustum_culled
                }
            } else {
                // Transform-only targets: no frustum check
                true
            }
        }
    };

    // Chunk is "visible" to this target if:
    // - Near target (protected), OR
    // - Within spawn distance AND frustum visible
    near_this_target || (within_spawn_distance && frustum_visible)
});
```

#### 6. Update Public Exports
**File**: `git/bevy_voxel_world/src/lib.rs`
**Changes**: Export new types

```rust
pub use voxel_world::{ChunkRenderTarget, VoxelWorldCamera}; // VoxelWorldCamera is deprecated alias
```

**File**: `git/bevy_voxel_world/src/voxel_world_internal.rs`
**Changes**: Update use statement at line 29

```rust
use crate::voxel_world::{
    get_chunk_voxel_position, ChunkWillChangeLod, ChunkWillDespawn, ChunkWillRemesh,
    ChunkWillSpawn, ChunkWillUpdate, ChunkRenderTarget,
};
```

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test` (in bevy_voxel_world directory)
- [x] Project builds: `cargo build` (in bevy_voxel_world directory)
- [x] No new clippy warnings: `cargo clippy`
- [x] Documentation builds: `cargo doc --no-deps`

#### Manual Verification:
- [x] ChunkRenderTarget and ChunkTarget are public and accessible
- [x] VoxelWorldCamera type alias shows deprecation warning
- [x] CameraInfo type alias shows deprecation warning
- [x] Code compiles with existing VoxelWorldCamera usage (backward compatibility)

---

## Phase 2: Update Examples

### Overview
Update all examples to use ChunkRenderTarget instead of VoxelWorldCamera (demonstrating migration path).

### Changes Required:

#### Update Example Files
**Files**: All 13 example files in `git/bevy_voxel_world/examples/`
- bombs.rs
- custom_material.rs
- custom_meshing.rs
- fast_traversal_ray.rs
- multiple_noise_terrain.rs
- multiple_worlds.rs
- navigation.rs
- noise_terrain_lod.rs
- noise_terrain.rs
- ray_cast.rs
- set_voxel.rs
- textures_custom_idx.rs
- textures.rs

**Pattern to find**: Search for `VoxelWorldCamera` usage
**Replacement**: Replace with `ChunkRenderTarget`

Example change in noise_terrain.rs:
```rust
// Before
commands.spawn((
    Camera3d::default(),
    Transform::from_xyz(0.0, 64.0, 0.0),
    VoxelWorldCamera::<MyWorld>::default(),
));

// After
commands.spawn((
    Camera3d::default(),
    Transform::from_xyz(0.0, 64.0, 0.0),
    ChunkRenderTarget::<MyWorld>::default(),
));
```

**Implementation approach**: Use Edit tool with replace_all=true for each file.

### Success Criteria:

#### Automated Verification:
- [x] All examples compile: `cargo build --examples` (in bevy_voxel_world directory)
- [x] No deprecated warnings in examples

#### Manual Verification:
- [x] Run each example and verify chunks spawn/despawn correctly
- [x] Examples show same visual behavior as before refactoring

---

## Phase 3: Update Documentation

### Overview
Update CHANGELOG.md, README.md, and code documentation to reflect the new architecture.

### Changes Required:

#### 1. Update CHANGELOG.md
**File**: `git/bevy_voxel_world/CHANGELOG.md`
**Changes**: Add new version entry at the top

```markdown
## [0.15.0] - 2026-01-04

### Changed
- **BREAKING (with migration path)**: Renamed `VoxelWorldCamera` to `ChunkRenderTarget` to support transform-based chunk visibility
- **BREAKING (with migration path)**: Renamed `CameraInfo` SystemParam to `ChunkTarget`
- `ChunkTarget` now queries `(Option<&Camera>, &GlobalTransform)` instead of `(&Camera, &GlobalTransform)`
- Chunk spawning now supports entities with Transform but no Camera component
- When no Camera is present on a ChunkRenderTarget entity:
  - Viewport raycasting is replaced with radial spawning
  - Frustum culling is skipped (all chunks within distance are considered visible)

### Migration Guide
- Replace `VoxelWorldCamera<C>` with `ChunkRenderTarget<C>` in your code
- Replace `CameraInfo<C>` with `ChunkTarget<C>` in custom systems
- If you iterate over CameraInfo/ChunkTarget, handle `Option<&Camera>` instead of `&Camera`
- Type aliases with `#[deprecated]` attributes are provided for backward compatibility

### Added
- Support for server-side chunk management using Transform-only entities
- Radial spawning strategy for entities without Camera component
```

#### 2. Update README.md
**File**: `git/bevy_voxel_world/README.md`
**Changes**: Update usage examples and add section on transform-based targets

Find VoxelWorldCamera usage and update examples:
```markdown
### Camera Setup

Add the `ChunkRenderTarget` component to your camera entities:

```rust
commands.spawn((
    Camera3d::default(),
    Transform::from_xyz(0.0, 64.0, 0.0),
    ChunkRenderTarget::<MyWorld>::default(),
));
```

### Server-Side Chunk Management

For server-side applications or non-camera-based chunk management, add `ChunkRenderTarget` to Transform-only entities:

```rust
// Server entity controlling chunk spawning for connected clients
commands.spawn((
    Transform::from_xyz(player_position.x, player_position.y, player_position.z),
    ChunkRenderTarget::<MyWorld>::default(),
));
```

Chunks will spawn radially around these entities within the configured `spawning_distance`.

#### 3. Update Component Documentation
**File**: `git/bevy_voxel_world/src/voxel_world.rs`
**Changes**: Already included in Phase 1 changes (lines 19-32 replacement)

### Success Criteria:

#### Automated Verification:
- [x] CHANGELOG.md follows keep-a-changelog format
- [x] README.md renders correctly: `cargo readme > /dev/null`
- [x] Documentation builds without warnings: `cargo doc --no-deps`

#### Manual Verification:
- [x] CHANGELOG clearly explains breaking changes and migration path
- [x] README examples are up-to-date and accurate
- [x] Component docs explain when to use Camera vs Transform-only

---

## Testing Strategy

### Unit Tests
- Existing tests should pass without modification (backward compatibility via type aliases)
- No new unit tests required (behavior unchanged, only API refactoring)

### Integration Tests
If integration tests exist in `git/bevy_voxel_world/src/test.rs`, verify they pass:
- Test camera-based chunk spawning
- Test multi-camera scenarios
- Test chunk despawning logic

### Manual Testing Steps
1. Run noise_terrain example with ChunkRenderTarget on Camera entity
2. Verify chunks spawn/despawn as you move the camera
3. Test server scenario: Create Transform-only entity with ChunkRenderTarget
4. Verify radial spawning occurs around the transform position
5. Test multi-target: Spawn multiple ChunkRenderTarget entities (some with Camera, some without)
6. Verify chunks spawn if visible to ANY target, despawn if invisible to ALL targets
7. Test LOD transitions with multiple targets at different distances

## Performance Considerations

- Minimal performance impact: same algorithms, just conditional Camera access
- Radial spawning for Transform-only targets may be more efficient than viewport raycasting
- No additional allocations or memory overhead
- Option<&Camera> access is zero-cost abstraction

## Migration Notes

**For Library Users:**
1. Search-replace `VoxelWorldCamera` → `ChunkRenderTarget` in your codebase
2. Search-replace `CameraInfo` → `ChunkTarget` in custom systems
3. If you iterate over ChunkTarget, update pattern matching for `Option<&Camera>`
4. Deprecated type aliases allow gradual migration

**For bevy_voxel_world Maintainers:**
- Type aliases can be removed in a future major version (e.g., 1.0.0)
- Consider documenting removal timeline in CHANGELOG

## References

- Original feature request: Transform-based active-chunk logic for server support
- Existing deprecation pattern: voxel_world.rs:220-283 (get_closest_surface_voxel)
- Similar refactoring: ChunkDespawnStrategy enum providing Camera-optional behavior
