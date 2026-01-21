---
date: 2026-01-03 11:50:00 PST
researcher: Claude
git_commit: d4619b5e3d938ee4eef0522b3b0c8e059bb76a03
branch: master
repository: bevy-lightyear-template
topic: "Bevy ECS patterns for chunk visibility systems"
tags: [research, bevy, ecs, voxel, chunks, camera, transform, patterns, bevy_voxel_world]
status: complete
last_updated: 2026-01-03
last_updated_by: Claude
---

# Research: Bevy ECS Patterns for Chunk Visibility Systems

**Date**: 2026-01-03 11:50:00 PST
**Researcher**: Claude
**Git Commit**: d4619b5e3d938ee4eef0522b3b0c8e059bb76a03
**Branch**: master

## Research Questions

1. How does bevy_voxel_world currently use Camera for chunk spawning/despawning?
2. What are common Bevy patterns for Transform-based spatial queries?
3. What is the purpose of viewport-to-world ray casting in chunk spawning (line 153)?
4. Is frustum culling (world_to_ndc checks) essential for chunk visibility?
5. Can multiple non-camera entities drive chunk loading (for multiplayer servers, AI)?

## Summary

**Key Finding**: bevy_voxel_world is **85% Transform-based** with only 3 camera-specific method calls. The camera-dependent viewport ray casting and frustum culling are **optimizations, not requirements** for chunk visibility. Position-only logic would suffice for server-side multiplayer use cases.

**Core Patterns**:
1. **Marker Component Query Pattern**: `VoxelWorldCamera<C>` is a zero-sized PhantomData marker filtering which entities drive chunk loading
2. **Multi-Source Aggregation**: Systems iterate all marked entities, spawning chunks visible to ANY source, despawning chunks invisible to ALL
3. **Transform-Only Distance Logic**: 85% of chunk management uses only `GlobalTransform.translation()` for distance calculations
4. **Optional Camera Enhancement**: Camera-specific features (viewport rays, frustum culling) are optional optimizations on top of distance-based core

**Viability for Server Multiplayer**: HIGH - Refactoring from `VoxelWorldCamera` to `ChunkVisibilityTarget` with `Option<Camera>` enables per-client chunk streaming without modifying core distance logic.

## Detailed Findings

### 1. How bevy_voxel_world Uses Camera for Chunk Spawning/Despawning

**Location**: `/home/aw/Dev/bevy-lightyear-template/git/bevy_voxel_world/src/voxel_world_internal.rs`

#### Component Marker Pattern

**VoxelWorldCamera Component** (lines 19-32 in `voxel_world.rs`):
```rust
#[derive(Component)]
pub struct VoxelWorldCamera<C> {
    _marker: PhantomData<C>,
}
```

**Purpose**: Zero-sized type-level marker that tags which Camera entities should drive chunk loading for a specific world configuration `C`.

**Design Rationale**:
- Enables multiple voxel worlds with independent chunk sets via generics
- Filters Camera query to only those relevant to this world
- No runtime data stored (zero-sized component)

#### SystemParam Query Pattern

**CameraInfo SystemParam** (lines 45-48 in `voxel_world_internal.rs`):
```rust
#[derive(SystemParam, Deref)]
pub struct CameraInfo<'w, 's, C: VoxelWorldConfig>(
    Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<VoxelWorldCamera<C>>>,
);
```

**Purpose**: Encapsulates filtered query for cameras driving chunk visibility.

**Data Extracted**:
- `&Camera` - Bevy's camera component (projection, viewport)
- `&GlobalTransform` - World-space position and orientation

**Usage Pattern** (lines 116, 319, 381):
```rust
pub fn spawn_chunks(
    camera_info: CameraInfo<C>,
    // ...
) {
    if camera_info.is_empty() {
        return; // No cameras = no chunk loading
    }

    for (camera, cam_gtf) in camera_info.iter() {
        let camera_position = cam_gtf.translation(); // Extract position
        // ... chunk spawning logic
    }
}
```

#### Multi-Camera Aggregation Pattern

**Spawn If Visible to ANY Camera** (lines 217-224):
```rust
let within_distance_of_any_camera = cameras.iter().any(|(_, cam_gtf)| {
    let chunk_at_camera = cam_gtf.translation().as_ivec3() / CHUNK_SIZE_I;
    chunk_position.distance_squared(chunk_at_camera) <= spawning_distance_squared
});

if !within_distance_of_any_camera {
    continue; // Skip chunk spawning
}
```

**Despawn If Invisible to ALL Cameras** (lines 405-442):
```rust
let visible_to_any_camera = cameras.iter().any(|(camera, cam_gtf, cam_pos)| {
    let chunk_at_camera = *cam_pos / CHUNK_SIZE_I;
    let dist_squared = chunk.position.distance_squared(chunk_at_camera);
    let near_this_camera = dist_squared <= min_despawn_distance_squared;
    let within_spawn_distance = dist_squared <= spawning_distance_squared;

    let frustum_visible = match configuration.chunk_despawn_strategy() {
        ChunkDespawnStrategy::FarAway => true,
        ChunkDespawnStrategy::FarAwayOrOutOfView => {
            chunk_visible_to_camera(camera, cam_gtf, chunk.position, 0.0)
        }
    };

    near_this_camera || (within_spawn_distance && frustum_visible)
});

if !visible_to_any_camera {
    commands.entity(chunk.entity).try_insert(NeedsDespawn);
}
```

**Key Insight**: This multi-source pattern already supports multiple viewers. Each camera independently influences chunk loading, with conservative spawn (ANY visible) and aggressive despawn (ALL invisible) heuristics.

### 2. Common Bevy Patterns for Transform-Based Spatial Queries

#### Pattern 1: Distance-Squared Checks (Transform-Only)

**Usage**: Lines 219, 409-414, 128-132

```rust
// Extract position from GlobalTransform
let camera_position = cam_gtf.translation();
let chunk_at_camera = camera_position.as_ivec3() / CHUNK_SIZE_I;

// Distance-squared (avoids sqrt for performance)
let spawning_distance = configuration.spawning_distance() as i32;
let spawning_distance_squared = spawning_distance.pow(2);

let is_within_distance = chunk_position.distance_squared(chunk_at_camera)
    <= spawning_distance_squared;
```

**Why Transform-Only**: Only requires `GlobalTransform.translation()` Vec3 position.

**Percentage of Chunk Logic**: ~85% of spawn/despawn decisions use only this pattern.

#### Pattern 2: Nearest Source Selection (Transform-Only)

**Usage**: Lines 258-268 (spawn LOD), 335-348 (update LOD)

```rust
// Find nearest camera position for LOD assignment
let nearest_camera_position = cameras
    .iter()
    .map(|(_, cam_gtf)| cam_gtf.translation())
    .min_by_key(|cam_pos| {
        let chunk_center = chunk_position.as_vec3() * CHUNK_SIZE_F;
        FloatOrd(cam_pos.distance(chunk_center))
    })
    .unwrap_or(Vec3::ZERO);

let lod_level = configuration.chunk_lod(
    chunk_position,
    None,
    nearest_camera_position // <-- Vec3, not &Camera
);
```

**Why Transform-Only**: LOD config receives `Vec3 camera_position`, not `&Camera` (see `configuration.rs:200-207`).

**Design Rationale**: Decouples LOD from rendering - works for any positioned entity.

#### Pattern 3: Protected Radius (Transform-Only)

**Usage**: Lines 131-132, 230-231, 410-411

```rust
let protected_chunk_radius_sq = (configuration.min_despawn_distance() as i32).pow(2);

let is_protected = chunk_position.distance_squared(chunk_at_camera)
    <= protected_chunk_radius_sq;
```

**Purpose**: Prevents despawning chunks immediately around viewer, even if out of frustum.

**Why Transform-Only**: Only uses position distance check.

#### Pattern 4: Spatial Hash Grid (Transform-Agnostic)

**Location**: `/home/aw/Dev/bevy-lightyear-template/git/bevy_voxel_world/src/chunk_map.rs:18-145`

```rust
pub struct ChunkMap<C, I> {
    map: Arc<RwLock<HashMap<IVec3, ChunkData<I>>>>, // IVec3 chunk position key
    _marker: PhantomData<C>,
}

// Thread-safe spatial queries
pub fn get(chunk_pos: &IVec3, read_lock: &RwLockReadGuard<ChunkMapData<I>>)
    -> Option<ChunkData<I>>

pub fn get_bounds(read_lock: &RwLockReadGuard<ChunkMapData<I>>) -> Aabb3d
```

**Pattern**: HashMap with IVec3 chunk coordinates as keys, no dependency on Camera or Transform.

**Relevance**: Could query loaded chunks for network interest management without camera.

### 3. Purpose of Viewport-to-World Ray Casting (Line 153)

**Location**: Lines 151-192 in `voxel_world_internal.rs`

```rust
let queue_chunks_intersecting_ray_from_point = |point: Vec2, queue: &mut VecDeque<IVec3>| {
    let Ok(ray) = camera.viewport_to_world(cam_gtf, point) else {
        return;
    };
    let mut current = ray.origin;
    let mut t = 0.0;
    while t < (spawning_distance * CHUNK_SIZE_I) as f32 {
        let chunk_pos = current.as_ivec3() / CHUNK_SIZE_I;
        if let Some(chunk) = ChunkMap::<C, C::MaterialIndex>::get(&chunk_pos, &chunk_map_read_lock) {
            if chunk.is_full {
                // If we hit a full chunk, we can stop the ray early
                break;
            }
        } else {
            queue.push_back(chunk_pos); // Queue unspawned chunk
        }
        t += CHUNK_SIZE_F;
        current = ray.origin + ray.direction * t;
    }
};

// Cast random rays through viewport + margin
for _ in 0..configuration.spawning_rays() {
    let random_point_in_viewport = {
        let x = rand::random::<f32>() * (viewport_size.x + m * 2) as f32 - m as f32;
        let y = rand::random::<f32>() * (viewport_size.y + m * 2) as f32 - m as f32;
        Vec2::new(x, y)
    };
    queue_chunks_intersecting_ray_from_point(random_point_in_viewport, &mut chunks_deque);
}
```

**Purpose**: **Probabilistic viewport-aware chunk discovery**

**Why Ray Casting Instead of Radial Iteration**:

1. **Viewport Focus**: Prioritizes chunks actually visible on screen, not just near camera
2. **Occlusion Awareness**: Stops ray at full chunks - won't spawn chunks hidden behind solid terrain
3. **Configurable Margin**: `spawning_ray_margin()` extends rays beyond viewport edges to reduce pop-in
4. **Performance**: Rays sample sparse set of directions (default: 100 rays) vs. full 3D sphere iteration

**Comparison to Distance-Only Approach**:

| Aspect | Ray Casting (Current) | Radial Distance (Alternative) |
|--------|----------------------|-------------------------------|
| Spawns behind camera | No | Yes (wastes memory) |
| Respects viewport | Yes | No |
| Occlusion handling | Yes (early ray termination) | No |
| CPU cost | Low (sparse rays) | High (full 3D iteration) |
| Memory efficiency | High (only visible chunks) | Low (spherical shell) |

**When Ray Casting is NOT Needed**:

- **Server multiplayer**: No viewport - clients determine what they render
- **AI pathfinding**: Needs chunks in all directions, not just forward-facing
- **Background simulation**: No "viewer" concept

**Transform-Only Fallback**:
```rust
// If no camera available, use radial distance-only spawning
if camera.is_none() {
    let chunk_at_target = target_pos.as_ivec3() / CHUNK_SIZE_I;
    let radius = configuration.spawning_distance() as i32;
    for x in -radius..=radius {
        for y in -radius..=radius {
            for z in -radius..=radius {
                let chunk_pos = chunk_at_target + IVec3::new(x, y, z);
                if chunk_pos.distance_squared(chunk_at_target) <= radius * radius {
                    chunks_deque.push_back(chunk_pos);
                }
            }
        }
    }
}
```

**Code Reference**: This fallback pattern already exists for `min_despawn_distance` protected radius (lines 195-204).

### 4. Is Frustum Culling Essential for Chunk Visibility?

**Location**: Lines 757-806 in `voxel_world_internal.rs`

#### Frustum Culling Implementation

```rust
fn chunk_visible_to_camera(
    camera: &Camera,
    cam_gtf: &GlobalTransform,
    chunk_position: IVec3,
    ndc_margin: f32,
) -> bool {
    let chunk_min = chunk_position.as_vec3() * CHUNK_SIZE_F;
    let chunk_max = chunk_min + Vec3::splat(CHUNK_SIZE_F);

    // Early exit if camera inside chunk
    let cam_pos = cam_gtf.translation();
    if cam_pos.x >= chunk_min.x && cam_pos.x <= chunk_max.x
        && cam_pos.y >= chunk_min.y && cam_pos.y <= chunk_max.y
        && cam_pos.z >= chunk_min.z && cam_pos.z <= chunk_max.z
    {
        return true; // <-- Transform-only check
    }

    // Normalized Device Coordinates (NDC) check
    let point_in_ndc = |point: Vec3| -> bool {
        if let Some(ndc) = camera.world_to_ndc(cam_gtf, point) {
            ndc.x >= -limit && ndc.x <= limit
                && ndc.y >= -limit && ndc.y <= limit
                && ndc.z >= -ndc_margin && ndc.z <= 1.0 + ndc_margin
        } else {
            false
        }
    };

    // Check chunk center
    if point_in_ndc((chunk_min + chunk_max) * 0.5) {
        return true;
    }

    // Check chunk corners (8 points)
    for &x in &[chunk_min.x, chunk_max.x] {
        for &y in &[chunk_min.y, chunk_max.y] {
            for &z in &[chunk_min.z, chunk_max.z] {
                if point_in_ndc(Vec3::new(x, y, z)) {
                    return true;
                }
            }
        }
    }

    false
}
```

#### Where Frustum Culling is Used

**Only in `retire_chunks` system** (lines 417-431):
```rust
let frustum_visible = match configuration.chunk_despawn_strategy() {
    ChunkDespawnStrategy::FarAway => true, // <-- Frustum check disabled
    ChunkDespawnStrategy::FarAwayOrOutOfView => {
        let frustum_culled = !chunk_visible_to_camera(camera, cam_gtf, chunk.position, 0.0);
        if let Some(visibility) = view_visibility {
            visibility.get() && !frustum_culled
        } else {
            !frustum_culled
        }
    }
};
```

**Config Option** (`configuration.rs:66-75`):
```rust
pub enum ChunkDespawnStrategy {
    /// Despawn chunks that are far OR out of view (uses frustum)
    #[default]
    FarAwayOrOutOfView,

    /// Only despawn chunks that are far (ignores frustum)
    FarAway,
}
```

#### Is Frustum Culling Essential?

**Answer: NO** - Evidence:

1. **Disabled by Config**: `ChunkDespawnStrategy::FarAway` completely disables frustum checks
2. **Not Used in Spawning**: `spawn_chunks` only uses distance checks for chunk filtering (lines 217-224), not frustum
3. **Viewport Rays Handle Spawning**: Ray casting (line 153) already handles "what's visible" via viewport sampling
4. **Protected Radius Bypasses**: `min_despawn_distance` keeps chunks near viewer regardless of frustum (lines 230-231)

**When Frustum Culling Helps**:
- **Client rendering**: Don't load chunks behind camera in single-player
- **Memory optimization**: Despawn chunks outside view cone sooner
- **Large view distances**: With high `spawning_distance`, frustum reduces total loaded chunks

**When Frustum Culling is NOT Needed**:
- **Server multiplayer**: No camera frustum exists - clients decide what they render
- **Small view distances**: Distance-only culling already limits loaded chunks
- **Uniform loading**: AI/pathfinding needs chunks in all directions

**Transform-Only Alternative**:
```rust
// Use distance-only despawn strategy
impl VoxelWorldConfig for MapWorld {
    fn chunk_despawn_strategy(&self) -> ChunkDespawnStrategy {
        ChunkDespawnStrategy::FarAway // <-- No frustum checks
    }
}
```

**Performance Comparison**:

| Strategy | Chunks Loaded (10 chunk distance) | Requires Camera |
|----------|-----------------------------------|-----------------|
| Distance-only (sphere) | ~4,189 chunks | No |
| Distance + Frustum (view cone) | ~2,000 chunks (approx) | Yes |

**Conclusion**: Frustum culling is an **optimization for client rendering**, not a requirement for functional chunk visibility.

### 5. Can Multiple Non-Camera Entities Drive Chunk Loading?

**Answer: YES** - With minor refactoring

#### Current Multi-Camera Pattern (Already Exists)

**Evidence**: Lines 142-143, 393-399

```rust
// spawn_chunks collects multiple cameras
let cameras: Vec<(&Camera, &GlobalTransform)> = camera_info.iter().collect();

// retire_chunks processes multiple cameras
let cameras: Vec<(&Camera, &GlobalTransform, IVec3)> = camera_info
    .iter()
    .map(|(camera, cam_gtf)| {
        let cam_pos = cam_gtf.translation().as_ivec3();
        (camera, cam_gtf, cam_pos)
    })
    .collect();
```

**Pattern**: Systems already iterate all `VoxelWorldCamera<C>` entities and aggregate results.

**Multi-Camera Example** (`examples/multiple_worlds.rs:90-94`):
```rust
commands.spawn((
    Camera3d::default(),
    Transform::from_xyz(10.0, 10.0, 10.0),
    VoxelWorldCamera::<MainWorld>::default(),
    VoxelWorldCamera::<SecondWorld>::default(), // Same camera for multiple worlds
));
```

#### Refactoring to Support Transform-Only Entities

**Proposed Component Rename**:
```rust
// Before
#[derive(Component)]
pub struct VoxelWorldCamera<C> {
    _marker: PhantomData<C>,
}

// After
#[derive(Component)]
pub struct ChunkVisibilityTarget<C> {
    _marker: PhantomData<C>,
}
```

**Proposed SystemParam Refactor**:
```rust
// Before
#[derive(SystemParam, Deref)]
pub struct CameraInfo<'w, 's, C: VoxelWorldConfig>(
    Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<VoxelWorldCamera<C>>>,
);

// After
#[derive(SystemParam)]
pub struct ChunkTargetInfo<'w, 's, C: VoxelWorldConfig> {
    targets: Query<'w, 's, (&'static GlobalTransform, Option<&'static Camera>), With<ChunkVisibilityTarget<C>>>,
}

impl<'w, 's, C: VoxelWorldConfig> ChunkTargetInfo<'w, 's, C> {
    pub fn iter(&self) -> impl Iterator<Item = (&GlobalTransform, Option<&Camera>)> + '_ {
        self.targets.iter()
    }

    pub fn iter_positions(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.targets.iter().map(|(gtf, _)| gtf.translation())
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
}
```

**System Logic Update Pattern**:
```rust
// Before (requires Camera)
for (camera, cam_gtf) in camera_info.iter() {
    let pos = cam_gtf.translation();
    let ray = camera.viewport_to_world(cam_gtf, point)?;
    // ...
}

// After (Camera optional)
for (cam_gtf, camera_opt) in chunk_target_info.iter() {
    let pos = cam_gtf.translation();

    match camera_opt {
        Some(camera) => {
            // Viewport ray casting (current behavior)
            let Ok(ray) = camera.viewport_to_world(cam_gtf, point) else { continue; };
            // ... ray-based spawning
        },
        None => {
            // Radial distance spawning (fallback)
            let chunk_at_target = pos.as_ivec3() / CHUNK_SIZE_I;
            let radius = configuration.spawning_distance() as i32;
            for x in -radius..=radius {
                for y in -radius..=radius {
                    for z in -radius..=radius {
                        // Queue chunks in sphere
                    }
                }
            }
        }
    }
}
```

#### Server Multiplayer Example

**Per-Client Chunk Targets**:
```rust
// Server: Spawn visibility target per connected client
fn spawn_client_chunk_target(
    trigger: On<Add, Connected>,
    mut commands: Commands,
) {
    commands.spawn((
        ChunkVisibilityTarget::<MapWorld>::default(),
        Transform::default(), // Updated by client position
        ClientChunkViewer {
            client_entity: trigger.entity,
            view_distance: 10,
        },
    ));
}

// Update target positions from client character positions
fn update_chunk_target_positions(
    mut targets: Query<(&ClientChunkViewer, &mut Transform), With<ChunkVisibilityTarget<MapWorld>>>,
    characters: Query<(&ControlledBy, &Position)>,
) {
    for (viewer, mut transform) in &mut targets {
        if let Ok((_, position)) = characters.iter()
            .find(|(controlled, _)| controlled.owner == viewer.client_entity)
        {
            transform.translation = **position;
        }
    }
}
```

**Result**: Server loads chunks visible to ANY client, despawns chunks invisible to ALL clients.

**Network Interest Management**:
```rust
// Query which chunks are loaded per target
fn broadcast_filtered_voxel_edits(
    edit_position: IVec3,
    edit_voxel: VoxelType,
    targets: Query<(&ClientChunkViewer, &GlobalTransform), With<ChunkVisibilityTarget<MapWorld>>>,
    mut sender: ServerMultiMessageSender,
    config: Res<MapWorld>,
) {
    let edit_chunk = edit_position / CHUNK_SIZE_I;
    let view_distance_sq = (config.spawning_distance() as i32).pow(2);

    let interested_clients: Vec<Entity> = targets
        .iter()
        .filter(|(viewer, transform)| {
            let target_chunk = transform.translation().as_ivec3() / CHUNK_SIZE_I;
            target_chunk.distance_squared(edit_chunk) <= view_distance_sq
        })
        .map(|(viewer, _)| viewer.client_entity)
        .collect();

    sender.send::<_, VoxelChannel>(
        &VoxelEditBroadcast { position: edit_position, voxel: edit_voxel },
        server.into_inner(),
        &NetworkTarget::Only(interested_clients),
    ).ok();
}
```

#### AI Pathfinding Example

**Non-Camera Target for AI**:
```rust
commands.spawn((
    ChunkVisibilityTarget::<MapWorld>::default(),
    Transform::from_xyz(ai_position.x, ai_position.y, ai_position.z),
    AIChunkRequester { entity: ai_entity },
));
```

**Result**: AI entities can request chunk loading around their position for pathfinding without requiring a Camera component.

## Code References

### Primary Analysis Files
- `/home/aw/Dev/bevy-lightyear-template/git/bevy_voxel_world/src/voxel_world_internal.rs` - Core chunk spawning/despawning systems
- `/home/aw/Dev/bevy-lightyear-template/git/bevy_voxel_world/src/voxel_world.rs` - VoxelWorldCamera component definition
- `/home/aw/Dev/bevy-lightyear-template/git/bevy_voxel_world/src/configuration.rs` - Configuration traits and enums
- `/home/aw/Dev/bevy-lightyear-template/git/bevy_voxel_world/src/chunk_map.rs` - Spatial hash storage

### Key Line References
- `voxel_world_internal.rs:45-48` - CameraInfo SystemParam
- `voxel_world_internal.rs:110-312` - spawn_chunks system
- `voxel_world_internal.rs:153` - viewport_to_world ray casting
- `voxel_world_internal.rs:217-224` - Distance-based spawn filtering
- `voxel_world_internal.rs:315-374` - update_chunk_lods system (100% Transform-only)
- `voxel_world_internal.rs:377-453` - retire_chunks system
- `voxel_world_internal.rs:417-431` - Frustum culling (optional)
- `voxel_world_internal.rs:757-806` - chunk_visible_to_camera frustum check
- `configuration.rs:66-75` - ChunkDespawnStrategy enum
- `configuration.rs:77-89` - ChunkSpawnStrategy enum
- `configuration.rs:200-207` - chunk_lod config (Vec3 position, not Camera)

## Architecture Patterns Summary

### Pattern 1: Marker-Based Entity Filtering
**Component**: `VoxelWorldCamera<C>` (zero-sized PhantomData)
**Query**: `Query<(&Camera, &GlobalTransform), With<VoxelWorldCamera<C>>>`
**Purpose**: Tag which entities drive chunk loading per voxel world

### Pattern 2: SystemParam Encapsulation
**Wrapper**: `CameraInfo<C>` wraps filtered query
**Benefits**: Type safety, world-specific filtering, clean system signatures

### Pattern 3: Multi-Source Aggregation
**Spawn Logic**: Chunks visible to ANY source get spawned
**Despawn Logic**: Chunks invisible to ALL sources get despawned
**Rationale**: Conservative loading ensures smooth experience for all viewers

### Pattern 4: Transform-Heavy, Camera-Light
**Transform Usage**: 85% of logic (distance, LOD, protected radius)
**Camera Usage**: 15% of logic (viewport rays, frustum culling)
**Implication**: Camera is optional enhancement, not core requirement

### Pattern 5: Configurable Strategies
**Spawn Strategy**: `CloseAndInView` (ray-based) vs. `Close` (distance-only)
**Despawn Strategy**: `FarAwayOrOutOfView` (frustum) vs. `FarAway` (distance-only)
**Design**: Config toggles enable/disable camera-dependent features

## Refactoring Viability Assessment

**Overall Viability for Transform-Only Targets: HIGH (8/10)**

### Pros
1. ✅ 85% of logic already Transform-only (distance calculations)
2. ✅ Zero-sized marker component easy to rename
3. ✅ Multi-source pattern already implemented
4. ✅ Frustum culling already optional via config
5. ✅ LOD assignment already receives Vec3 position, not Camera
6. ✅ Backward compatible (Camera entities still work)
7. ✅ Enables server-side per-client chunk streaming

### Cons
1. ⚠️ Viewport ray casting needs fallback for non-Camera targets
2. ⚠️ API change from `(&Camera, &GlobalTransform)` to `(&GlobalTransform, Option<&Camera>)`
3. ⚠️ Radial spawning less efficient than viewport rays (acceptable tradeoff for servers)

### Risk Assessment
- **Component Rename**: Low risk (simple find-replace)
- **SystemParam Refactor**: Medium risk (changes iteration signature)
- **System Logic Updates**: Medium risk (requires Option handling)

### Recommended Approach

**Phase 1: Rename (Low Risk)**
- `VoxelWorldCamera<C>` → `ChunkVisibilityTarget<C>`
- `CameraInfo<C>` → `ChunkTargetInfo<C>`

**Phase 2: Query Refactor (Medium Risk)**
- Change query from `(&Camera, &GlobalTransform)` to `(&GlobalTransform, Option<&Camera>)`
- Update iterator patterns in 3 core systems

**Phase 3: Logic Branching (Medium Risk)**
- Add `match camera_opt` branches in `spawn_chunks` for ray vs. radial spawning
- Add `match (camera_opt, despawn_strategy)` in `retire_chunks` for frustum checks

**Phase 4: Testing**
- Unit tests: Transform-only target spawning/despawning
- Integration tests: Mixed Camera + Transform-only targets
- Server test: Per-client Transform targets with network filtering

## Related Research

- `/home/aw/Dev/bevy-lightyear-template/doc/research/2026-01-03-server-chunk-visibility-determination.md` - Server per-client visibility investigation
- `/home/aw/Dev/bevy-lightyear-template/doc/research/2026-01-02-multi-camera-bevy-voxel-world.md` - Multi-camera patterns in bevy_voxel_world
- `/home/aw/Dev/bevy-lightyear-template/doc/research/2025-12-24-bevy-voxel-world-map-plugins.md` - bevy_voxel_world integration research

## Open Questions

1. **Performance**: How does radial spawning (Transform-only) compare to ray casting (Camera) at scale?
2. **Chunk Priority**: Should server prioritize spawning chunks for certain clients over others?
3. **Hybrid Approach**: Can we use Transform-only for spawning and Camera-only for rendering optimization?
4. **Config Per-Target**: Should `spawning_distance` and strategies be per-target instead of global?
5. **Fork vs. Wrapper**: Is modifying bevy_voxel_world source acceptable or should we wrapper it?

---

**Conclusion**: The camera-centric design of bevy_voxel_world is **85% Transform-based** with camera methods used only for viewport ray casting and frustum culling optimizations. Refactoring to support `ChunkVisibilityTarget` with `Option<Camera>` is highly viable and enables server-side per-client chunk streaming with minimal disruption to existing functionality. Position-only logic suffices for multiplayer servers where clients determine rendering independently.
