---
date: 2026-01-02T08:09:39-08:00
researcher: Claude
git_commit: d4619b5e3d938ee4eef0522b3b0c8e059bb76a03
branch: master
repository: bevy-lightyear-template
topic: "How to update bevy_voxel_world to support multiple cameras instead of requiring a single() camera"
tags: [research, codebase, bevy_voxel_world, camera, rendering, multi-camera]
status: complete
last_updated: 2026-01-02
last_updated_by: Claude
last_updated_note: "Added follow-up research with concrete implementation examples"
---

# Research: How to update bevy_voxel_world to support multiple cameras instead of requiring a single() camera

**Date**: 2026-01-02 08:09:39 PST
**Researcher**: Claude
**Git Commit**: d4619b5e3d938ee4eef0522b3b0c8e059bb76a03
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to update `bevy_voxel_world` to support multiple cameras instead of requiring a `single()` camera?

## Summary

The `bevy_voxel_world` library currently requires exactly one camera marked with `VoxelWorldCamera<C>` per voxel world configuration. Three core systems use `.single()` to retrieve camera data: `spawn_chunks()`, `update_chunk_lods()`, and `retire_chunks()`. The camera's `GlobalTransform` is used for position-based calculations (chunk spawning proximity, LOD distances, despawn distances), while the `Camera` component provides viewport methods for raycasting (`viewport_to_world`) and frustum culling (`world_to_ndc`).

The single-camera constraint exists in:
- `git/bevy_voxel_world/src/voxel_world_internal.rs` - Core systems at lines 112, 283, 333
- `crates/client/src/map.rs:72` - Project integration code
- Multiple examples using `.single().unwrap()`

The codebase demonstrates established patterns for handling multiple entities using `.iter()` instead of `.single()`, with filtering through marker components like `With<VoxelWorldCamera<MapWorld>>`.

## Detailed Findings

### Current Single-Camera Implementation

#### Core Systems Using `.single()`

**1. Chunk Spawning System** (`spawn_chunks`)
- Location: `git/bevy_voxel_world/src/voxel_world_internal.rs:99-273`
- Line 112: `let Ok((camera, cam_gtf)) = camera_info.single() else { return; }`
- Usage:
  - Line 115: `cam_gtf.translation()` - camera position for proximity calculations
  - Line 122: `camera.physical_viewport_size()` - viewport dimensions for ray sampling
  - Line 137: `camera.viewport_to_world(cam_gtf, point)` - converts viewport points to world rays
- Purpose: Shoots random rays from viewport to discover chunks within camera frustum and spawning distance
- Behavior with multiple cameras: Returns early, preventing any chunk spawning

**2. LOD Update System** (`update_chunk_lods`)
- Location: `git/bevy_voxel_world/src/voxel_world_internal.rs:276-320`
- Line 283: `let Ok((_, cam_gtf)) = camera_info.single() else { return; }`
- Usage:
  - Line 287: `cam_gtf.translation()` - camera position for distance calculations
  - Lines 290-294: Distance passed to `configuration.chunk_lod()` for each chunk
- Purpose: Adjusts chunk detail levels based on distance from camera
- Behavior with multiple cameras: Returns early, freezing LOD at current state

**3. Chunk Retirement System** (`retire_chunks`)
- Location: `git/bevy_voxel_world/src/voxel_world_internal.rs:322-377`
- Line 333: `let (camera, cam_gtf) = camera_info.single().unwrap();`
- Usage:
  - Line 334: `cam_gtf.translation()` - distance calculations for despawning
  - Lines 345-350: `chunk_visible_to_camera(camera, cam_gtf, chunk.position, 0.0)` - frustum culling
- Purpose: Marks chunks for despawning when far or outside view frustum
- Behavior with multiple cameras: **Panics immediately** if 0 or 2+ cameras exist

#### Camera Query Definition

**CameraInfo SystemParam**
- Location: `git/bevy_voxel_world/src/voxel_world_internal.rs:34-37`
- Query type: `Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<VoxelWorldCamera<C>>>`
- Filter: `With<VoxelWorldCamera<C>>` marker component
- Generic parameter `C` allows multiple voxel world types, each with own camera

**VoxelWorldCamera Marker Component**
- Location: `git/bevy_voxel_world/src/voxel_world.rs:19-32`
- Definition: Zero-size marker with `PhantomData<C>` for type differentiation
- Purpose: Tags which camera controls chunk spawning for voxel world type `C`

### Camera Data Usage Patterns

#### GlobalTransform Usage
- **Translation**: Used in all three systems for:
  - Chunk spawning proximity checks
  - LOD distance calculations
  - Despawn distance thresholds
  - Ray origin for viewport-to-world conversions

#### Camera Component Methods
- **`viewport_to_world(transform, viewport_pos)`**:
  - Location: Line 137 in `spawn_chunks`
  - Converts 2D viewport coordinates to 3D world rays
  - Used for random ray sampling to discover chunks in view

- **`world_to_ndc(transform, world_pos)`**:
  - Location: Line 703 in `chunk_visible_to_camera` helper
  - Projects 3D world points to normalized device coordinates
  - Used for frustum culling visibility tests

- **`physical_viewport_size()`**:
  - Location: Line 122 in `spawn_chunks`
  - Returns viewport dimensions for ray sampling bounds

### Project Integration Points

**Client Map Input Handler**
- Location: `crates/client/src/map.rs:62-113`
- Line 65: `camera: Query<(&Camera, &GlobalTransform), With<VoxelWorldCamera<MapWorld>>>`
- Line 72: `let Ok((camera, transform)) = camera.single() else { return; }`
- Usage: Line 82 uses `camera.viewport_to_world(transform, cursor)` for voxel raycasting
- Behavior: Silently disables input handling if multiple cameras exist

**Camera Spawning**
- Location: `crates/render/src/lib.rs:41`
- Spawns camera with `VoxelWorldCamera::<MapWorld>::default()` marker
- Single camera assumption throughout project

### Examples in bevy_voxel_world

Multiple examples use `.single().unwrap()` which will panic with multiple cameras:
- `git/bevy_voxel_world/examples/ray_cast.rs:117`
- `git/bevy_voxel_world/examples/navigation.rs:291`
- `git/bevy_voxel_world/examples/fast_traversal_ray.rs:142`
- `git/bevy_voxel_world/examples/bombs.rs:191`

### Frustum Visibility Helper

**chunk_visible_to_camera Function**
- Location: `git/bevy_voxel_world/src/voxel_world_internal.rs:681-730`
- Parameters: `camera: &Camera`, `cam_gtf: &GlobalTransform`, chunk position
- Logic:
  - Lines 690-697: Checks if camera inside chunk bounds
  - Line 703: Projects chunk corners to NDC with `camera.world_to_ndc(cam_gtf, point)`
  - Lines 704-709: Tests if NDC coordinates within viewport bounds
- Purpose: Determines if chunk bounding box is visible to camera frustum
- Used by: `retire_chunks` system for frustum-based despawning

## Code References

Core Systems:
- `git/bevy_voxel_world/src/voxel_world_internal.rs:112` - spawn_chunks camera query
- `git/bevy_voxel_world/src/voxel_world_internal.rs:283` - update_chunk_lods camera query
- `git/bevy_voxel_world/src/voxel_world_internal.rs:333` - retire_chunks camera query (panics)

Camera Query Definition:
- `git/bevy_voxel_world/src/voxel_world_internal.rs:34-37` - CameraInfo SystemParam
- `git/bevy_voxel_world/src/voxel_world.rs:19-32` - VoxelWorldCamera marker component

Project Integration:
- `crates/client/src/map.rs:65` - Client camera query definition
- `crates/client/src/map.rs:72` - Client input handler camera.single()
- `crates/render/src/lib.rs:41` - Camera spawn with VoxelWorldCamera marker

Helper Functions:
- `git/bevy_voxel_world/src/voxel_world_internal.rs:681-730` - chunk_visible_to_camera

## Existing Multi-Entity Iteration Patterns in Codebase

The codebase contains established patterns for handling multiple entities instead of using `.single()`:

### Pattern: Using `.iter()` for Multiple Entities

**Example 1: Processing Multiple Chunks**
- Location: `crates/protocol/src/map.rs:90`
- Query: `chunks: Query<(Entity, &Mesh3d, Option<&Collider>), (With<Chunk<MapWorld>>, ...)>`
- Iteration: `for (entity, mesh_handle, existing_collider) in chunks.iter()`
- Demonstrates iterating over all chunks with filtering

**Example 2: Processing Multiple Buttons**
- Location: `crates/ui/src/lib.rs:203`
- Separate queries for different button types with marker filters
- Iteration: `for interaction in connect_query.iter()`
- Shows how multiple entities of same type are handled independently

**Example 3: Processing Messages from Multiple Clients**
- Location: `crates/server/src/map.rs:47`
- Query: `receiver: Query<&mut MessageReceiver<VoxelEditRequest>>`
- Iteration: `for mut message_receiver in receiver.iter_mut()`
- Nested iteration over receivers and messages

### Pattern: Marker Components for Filtering

**Example 1: Camera Markers**
- Location: `crates/client/src/map.rs:65`
- Filter: `With<VoxelWorldCamera<MapWorld>>`
- Demonstrates how marker components distinguish entity types

**Example 2: Character Markers**
- Location: `crates/client/src/gameplay.rs:77`
- Filters: `(With<Predicted>, With<CharacterMarker>)`
- Shows combining multiple marker filters

### Pattern: Safe Single-Entity Access with Error Handling

**Example: Early Return Pattern**
- Location: `crates/client/src/map.rs:69-75`
- Code: `let Ok((camera, transform)) = camera.single() else { return; }`
- Gracefully handles missing or multiple entities by returning early
- Used when entity is expected but not guaranteed to exist

### Pattern: Alternative Syntax Forms

**Reference Iteration:**
- `for item in &query` - equivalent to `query.iter()`
- `for item in &mut query` - equivalent to `query.iter_mut()`

**Functional Style:**
- `query.iter().for_each(|item| ...)` - functional iteration
- `query.iter().count()` - count matching entities

## Architecture Documentation

### Current Design: Single Camera Per Voxel World

**Assumptions:**
1. Each `VoxelWorldConfig` type `C` has exactly one camera with `VoxelWorldCamera<C>` marker
2. All chunk operations (spawning, LOD, despawning) reference this single camera's position and frustum
3. Viewport raycasting uses this single camera's projection matrix

**Benefits of Current Design:**
- Simple and unambiguous - no camera selection logic needed
- Chunk operations always reference same viewpoint
- LOD calculations consistent across all chunks

**Limitations:**
- Panics in `retire_chunks` if camera missing or multiple exist
- Silent failures in `spawn_chunks` and `update_chunk_lods` with multiple cameras
- Cannot support split-screen or picture-in-picture rendering
- Cannot support multiple viewports with independent chunk loading

### Plugin Registration

**System Registration:**
- Location: `git/bevy_voxel_world/src/plugin.rs:102-124`
- Schedule: `PreUpdate`
- Systems:
  - Line 107: `Internals::<C>::spawn_chunks`
  - Line 108: `Internals::<C>::update_chunk_lods`
  - Line 109: `Internals::<C>::retire_chunks`
- All three systems depend on camera query succeeding

### Configuration Integration

**VoxelWorldConfig Methods Used:**
- `spawning_distance()`: Maximum distance from camera to spawn chunks
- `min_despawn_distance()`: Protected radius around camera preventing despawn
- `chunk_spawn_strategy()`: Determines if viewport frustum used for spawning
- `chunk_despawn_strategy()`: Controls frustum-based despawning
- `chunk_lod()`: Receives camera distance to determine detail level

## Historical Context (from doc/)

**Render Crate Camera Setup:**
- Document: `doc/research/2025-11-27-render-crate-camera-setup.md`
- Details camera spawning at `crates/client/src/network.rs:77` (now moved to render crate)
- Camera positioned at `(-5.0, 3.0, 8.0)` looking at origin
- Conditional compilation: DefaultPlugins for client/web, MinimalPlugins for headless server

**Voxel Map Plugins Integration:**
- Document: `doc/research/2025-12-24-bevy-voxel-world-map-plugins.md`
- Describes VoxelWorldConfig API, chunk-based terrain with async meshing
- Integration with Avian3D physics and lightyear networking
- Current implementation: 100x1x100 static floor replaced by voxel terrain
- Physics: Chunk colliders attached via `attach_chunk_colliders` system

**Implementation Plans:**
- `doc/plans/2025-12-24-voxel-map-plugins.md` - Voxel terrain replacement plan
- `doc/plans/2025-11-27-render-crate-camera-setup.md` - Render crate creation plan

## Related Research

- `doc/research/2025-12-24-bevy-voxel-world-map-plugins.md` - VoxelWorldConfig and chunk system
- `doc/research/2025-11-27-render-crate-camera-setup.md` - Camera spawning and positioning
- `doc/research/2025-12-07-game-world-loading-validated.md` - Rendering and camera handling sections

## Open Questions

1. **Multi-Camera Selection Strategy**: If supporting multiple cameras, how should the system choose which camera controls chunk spawning for a given world?
   - Per-camera chunk sets (each camera loads own chunks)?
   - Primary camera designation?
   - Union of all camera frustums?

2. **LOD Resolution**: With multiple cameras at different distances, which camera's distance determines chunk LOD?
   - Use nearest camera?
   - Use highest LOD needed by any camera?
   - Per-camera LOD tracking?

3. **Performance Impact**: What is the performance cost of:
   - Iterating cameras instead of `.single()`?
   - Maintaining multiple chunk sets per camera?
   - Calculating LOD for multiple viewpoints?

4. **Despawn Strategy**: Should chunks only despawn when invisible to ALL cameras, or per-camera chunk management?

5. **Backward Compatibility**: Should single-camera use case remain optimized, or unified with multi-camera code path?

6. **API Design**: Should multi-camera support be opt-in (new config flag), or replace single-camera implementation entirely?

## Follow-up Research 2026-01-02 08:10 PST

### Concrete Implementation Examples

This section provides code examples for specific multi-camera implementation approaches requested: camera selection strategies, nearest-camera LOD, and invisible-to-all despawn logic.

### Option 1: Union of All Camera Frustums (Shared Chunk Set)

All cameras share the same chunk set. Chunks spawn if visible to ANY camera, despawn only when invisible to ALL cameras.

**Characteristics:**
- Single shared ChunkMap for all cameras
- Chunks loaded if ANY camera needs them
- Highest memory usage (superset of all camera views)
- Simplest for consistency - all cameras see same world state

**spawn_chunks Implementation:**

```rust
pub fn spawn_chunks(
    mut commands: Commands,
    mut chunk_map_insert_buffer: ResMut<ChunkMapInsertBuffer<C, C::MaterialIndex>>,
    world_root: Query<Entity, With<WorldRoot<C>>>,
    chunk_map: Res<ChunkMap<C, C::MaterialIndex>>,
    configuration: Res<C>,
    camera_info: CameraInfo<C>,  // Query with .iter() instead of .single()
) {
    let world_root = world_root.single().unwrap();
    let attach_to_root = configuration.attach_chunks_to_root();

    // Early return if NO cameras exist
    if camera_info.is_empty() {
        return;
    }

    let spawning_distance = configuration.spawning_distance() as i32;
    let spawning_distance_squared = spawning_distance.pow(2);
    let spawn_strategy = configuration.chunk_spawn_strategy();
    let protected_chunk_radius_sq = (configuration.min_despawn_distance() as i32).pow(2);

    let mut visited = HashSet::new();
    let mut chunks_deque = VecDeque::with_capacity(
        configuration.spawning_rays() * spawning_distance as usize * camera_info.iter().count()
    );

    let chunk_map_read_lock = chunk_map.get_read_lock();

    // Process each camera
    for (camera, cam_gtf) in camera_info.iter() {
        let camera_position = cam_gtf.translation();
        let cam_pos = camera_position.as_ivec3();
        let viewport_size = camera.physical_viewport_size().unwrap_or_default();

        // Queue chunks from viewport rays for THIS camera
        let mut camera_queue = VecDeque::new();
        let queue_chunks_intersecting_ray_from_point =
            |point: Vec2, queue: &mut VecDeque<IVec3>| {
                let Ok(ray) = camera.viewport_to_world(cam_gtf, point) else {
                    return;
                };
                // ... existing ray-chunk intersection logic ...
                // (same as current implementation)
            };

        // Shoot rays from this camera's viewport
        for _ in 0..configuration.spawning_rays() {
            let point = Vec2::new(
                rand::random::<f32>() * viewport_size.x as f32,
                rand::random::<f32>() * viewport_size.y as f32,
            );
            queue_chunks_intersecting_ray_from_point(point, &mut camera_queue);
        }

        // Merge camera's chunks into global visited set
        chunks_deque.extend(camera_queue);
    }

    // Deduplicate and spawn chunks (existing logic)
    // ... process chunks_deque as before ...
}
```

**update_chunk_lods Implementation (Nearest Camera):**

```rust
pub fn update_chunk_lods(
    mut commands: Commands,
    mut chunks: Query<(Entity, &mut Chunk<C>), Without<NeedsDespawn>>,
    configuration: Res<C>,
    camera_info: CameraInfo<C>,
    mut ev_chunk_will_change_lod: MessageWriter<ChunkWillChangeLod<C>>,
) {
    // Early return if no cameras
    if camera_info.is_empty() {
        return;
    }

    // Collect all camera positions
    let camera_positions: Vec<Vec3> = camera_info
        .iter()
        .map(|(_, cam_gtf)| cam_gtf.translation())
        .collect();

    for (entity, mut chunk) in chunks.iter_mut() {
        // Find NEAREST camera distance to this chunk
        let nearest_camera_distance = camera_positions
            .iter()
            .map(|cam_pos| {
                let chunk_center = chunk.position.as_vec3() * CHUNK_SIZE_F;
                cam_pos.distance(chunk_center)
            })
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(f32::MAX);

        // Use nearest camera position for LOD calculation
        let nearest_camera_position = camera_positions
            .iter()
            .min_by_key(|cam_pos| {
                let chunk_center = chunk.position.as_vec3() * CHUNK_SIZE_F;
                FloatOrd(cam_pos.distance(chunk_center))
            })
            .copied()
            .unwrap_or(Vec3::ZERO);

        let target_lod = configuration.chunk_lod(
            chunk.position,
            Some(chunk.lod_level),
            nearest_camera_position,  // Use NEAREST camera
        );

        if target_lod == chunk.lod_level {
            continue;
        }

        // ... existing LOD update logic ...
        ev_chunk_will_change_lod.write(ChunkWillChangeLod::<C>::new(chunk.position, entity));

        let data_shape = configuration.chunk_data_shape(target_lod);
        let mesh_shape = configuration.chunk_meshing_shape(target_lod);

        if chunk.data_shape == data_shape && chunk.mesh_shape == mesh_shape {
            chunk.lod_level = target_lod;
            continue;
        }

        chunk.data_shape = data_shape;
        chunk.mesh_shape = mesh_shape;
        chunk.lod_level = target_lod;

        let mut entity_commands = commands.entity(entity);
        entity_commands.try_insert(NeedsRemesh);
        entity_commands.remove::<ChunkThread<C, C::MaterialIndex>>();
    }
}

// Helper to make f32 orderable for min_by_key
#[derive(PartialEq, PartialOrd)]
struct FloatOrd(f32);
```

**retire_chunks Implementation (Invisible to ALL Cameras):**

```rust
pub fn retire_chunks(
    mut commands: Commands,
    all_chunks: Query<(&Chunk<C>, Option<&ViewVisibility>)>,
    configuration: Res<C>,
    camera_info: CameraInfo<C>,
    mut ev_chunk_will_despawn: MessageWriter<ChunkWillDespawn<C>>,
) {
    // Early return if no cameras - don't despawn anything
    if camera_info.is_empty() {
        return;
    }

    let spawning_distance = configuration.spawning_distance() as i32;
    let spawning_distance_squared = spawning_distance.pow(2);

    // Collect camera data once
    let cameras: Vec<(&Camera, &GlobalTransform, IVec3)> = camera_info
        .iter()
        .map(|(camera, cam_gtf)| {
            let cam_pos = cam_gtf.translation().as_ivec3();
            (camera, cam_gtf, cam_pos)
        })
        .collect();

    let chunks_to_remove = {
        let mut remove = Vec::with_capacity(1000);

        for (chunk, view_visibility) in all_chunks.iter() {
            // Check visibility against ALL cameras
            let visible_to_any_camera = cameras.iter().any(|(camera, cam_gtf, cam_pos)| {
                let chunk_at_camera = *cam_pos / CHUNK_SIZE_I;

                // Check if chunk is near THIS camera
                let dist_squared = chunk.position.distance_squared(chunk_at_camera);
                let near_this_camera = dist_squared
                    <= (CHUNK_SIZE_I * configuration.min_despawn_distance() as i32).pow(2);

                // Check if chunk is within spawning distance
                let within_spawn_distance = dist_squared <= spawning_distance_squared + 1;

                // Check frustum visibility for this camera
                let frustum_visible = match configuration.chunk_despawn_strategy() {
                    ChunkDespawnStrategy::FarAway => true,
                    ChunkDespawnStrategy::FarAwayOrOutOfView => {
                        chunk_visible_to_camera(camera, cam_gtf, chunk.position, 0.0)
                    }
                };

                // Chunk is "visible" to this camera if:
                // - Near camera (protected), OR
                // - Within spawn distance AND frustum visible
                near_this_camera || (within_spawn_distance && frustum_visible)
            });

            // Only despawn if invisible to ALL cameras
            if !visible_to_any_camera {
                remove.push(chunk);
            }
        }
        remove
    };

    for chunk in chunks_to_remove {
        commands.entity(chunk.entity).try_insert(NeedsDespawn);
        ev_chunk_will_despawn.write(ChunkWillDespawn::<C>::new(chunk.position, chunk.entity));
    }
}
```

### Option 2: Per-Camera Chunk Sets

Each camera maintains its own independent chunk set. Requires tracking which camera owns which chunks.

**Characteristics:**
- Separate ChunkMap per camera (or chunks tagged with owning camera)
- Most memory efficient for non-overlapping camera views
- Complex bookkeeping - chunks may be loaded multiple times
- Required for truly independent viewports (e.g., different game worlds)

**Implementation Sketch:**

```rust
// New component to track which camera(s) need a chunk
#[derive(Component)]
struct ChunkCameraOwners<C: VoxelWorldConfig> {
    camera_entities: HashSet<Entity>,
    _phantom: PhantomData<C>,
}

pub fn spawn_chunks(
    // ... existing params ...
    camera_query: Query<(Entity, &Camera, &GlobalTransform), With<VoxelWorldCamera<C>>>,
) {
    // Process each camera INDEPENDENTLY
    for (camera_entity, camera, cam_gtf) in camera_query.iter() {
        // ... spawn chunks for THIS camera only ...
        // Tag spawned chunks with camera_entity
    }
}

pub fn retire_chunks(
    // ... existing params ...
    camera_query: Query<(Entity, &Camera, &GlobalTransform), With<VoxelWorldCamera<C>>>,
    mut chunks: Query<(&Chunk<C>, &mut ChunkCameraOwners<C>)>,
) {
    let camera_entities: HashSet<Entity> = camera_query.iter().map(|(e, _, _)| e).collect();

    for (chunk, mut owners) in chunks.iter_mut() {
        // Remove cameras that no longer exist
        owners.camera_entities.retain(|e| camera_entities.contains(e));

        // Check each camera independently
        for (camera_entity, camera, cam_gtf) in camera_query.iter() {
            let chunk_visible = /* ... visibility check ... */;

            if chunk_visible {
                owners.camera_entities.insert(camera_entity);
            } else {
                owners.camera_entities.remove(&camera_entity);
            }
        }

        // Despawn only if NO cameras need this chunk
        if owners.camera_entities.is_empty() {
            // Mark for despawn
        }
    }
}
```

### Option 3: Primary Camera Designation

Designate one camera as "primary" for chunk operations. Other cameras are passive observers.

**Characteristics:**
- Similar to current single-camera, but gracefully handles multiple cameras
- Simple fallback: first camera in iteration becomes primary
- No additional memory overhead
- Other cameras may see pop-in if they view areas primary camera hasn't loaded

**Implementation Sketch:**

```rust
pub fn spawn_chunks(
    // ... existing params ...
    camera_info: CameraInfo<C>,
) {
    // Use FIRST camera as primary (or could be marked with component)
    let Some((camera, cam_gtf)) = camera_info.iter().next() else {
        return;
    };

    // Existing single-camera logic unchanged
    let camera_position = cam_gtf.translation();
    // ... rest of current implementation ...
}

// Similarly for update_chunk_lods and retire_chunks
```

### Implementation Comparison

| Aspect | Union of Frustums | Per-Camera Sets | Primary Camera |
|--------|------------------|-----------------|----------------|
| **Complexity** | Medium | High | Low |
| **Memory** | Highest | Variable | Lowest |
| **Consistency** | All cameras see same chunks | Independent per camera | Only primary guaranteed |
| **Use Case** | Split-screen same world | Picture-in-picture different worlds | Fallback/simple multi-cam |
| **LOD** | Nearest camera | Per-camera | Primary camera only |
| **Despawn** | Invisible to ALL | Per-camera tracking | Primary camera only |

### Recommended Approach: Union with Nearest-Camera LOD

For most use cases (split-screen, multiple viewports in same world), **Option 1: Union of All Camera Frustums** with **nearest-camera LOD** provides the best balance:

**Benefits:**
1. All cameras see consistent world state (no pop-in between views)
2. Chunks load if ANY camera needs them
3. LOD uses nearest camera, so close-up views get high detail
4. Despawn only when ALL cameras done viewing
5. No complex per-camera bookkeeping

**Trade-offs:**
1. Higher memory than primary-only (but acceptable for 2-4 cameras)
2. Slightly more CPU for iteration (negligible with small camera counts)

**Implementation Changes Required:**
- `spawn_chunks`: Replace `.single()` with `.iter()`, merge chunk queues from all cameras
- `update_chunk_lods`: Find nearest camera position per chunk
- `retire_chunks`: Check visibility against ALL cameras, despawn only if none need it
- `crates/client/src/map.rs:72`: Handle input with specific camera (cursor position determines which camera)

### Code Locations to Modify

**bevy_voxel_world library (git/bevy_voxel_world):**
1. `src/voxel_world_internal.rs:111-113` - spawn_chunks camera query
2. `src/voxel_world_internal.rs:282-284` - update_chunk_lods camera query
3. `src/voxel_world_internal.rs:332-333` - retire_chunks camera query (REMOVE .unwrap())

**This project:**
4. `crates/client/src/map.rs:72-75` - Client input handler (determine which camera cursor is over)
