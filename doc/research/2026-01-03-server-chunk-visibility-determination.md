---
date: 2026-01-03 10:56:07 PST
researcher: Claude
git_commit: d4619b5e3d938ee4eef0522b3b0c8e059bb76a03
branch: master
repository: bevy-lightyear-template
topic: "Server determination of active chunks for all clients"
tags: [research, codebase, voxel, chunks, server, networking, bevy_voxel_world, lightyear, refactoring, camera, transform]
status: complete
last_updated: 2026-01-03
last_updated_by: Claude
last_updated_note: "Added follow-up research on VoxelWorldCamera to ChunkVisibilityTarget refactoring viability"
---

# Research: Server Determination of Active Chunks for All Clients

**Date**: 2026-01-03 10:56:07 PST
**Researcher**: Claude
**Git Commit**: d4619b5e3d938ee4eef0522b3b0c8e059bb76a03
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How does the server determine which chunks are active for all connected clients, and what mechanisms exist (or could be modified in bevy_voxel_world) to implement per-client chunk visibility?

## Summary

**Current State**: The server does NOT currently determine active chunks on a per-client basis. All voxel modifications are broadcast to all connected clients using `NetworkTarget::All`, with no interest management or per-client chunk tracking.

**Key Findings**:
1. Server uses a single headless camera for chunk management (spawning distance: 10 chunks)
2. Client tracking exists via lightyear's `Connected` component and `ReplicationSender`
3. Client positions are stored in replicated `Position` components
4. bevy_voxel_world provides camera-driven chunk spawning/despawning with LOD
5. No per-client chunk visibility or interest management is implemented
6. All modifications tracked globally in `VoxelModifications` resource
7. bevy_voxel_world source (git/bevy_voxel_world) provides extensible chunk streaming APIs

## Detailed Findings

### Server-Side Chunk Management

**Location**: `crates/server/src/map.rs`

The server maintains voxel state using bevy_voxel_world with a headless camera approach:

```rust
// Line 24-31: Dummy camera for chunk management
fn spawn_voxel_camera(mut commands: Commands) {
    commands.spawn((
        Name::new("VoxelCamera"),
        VoxelWorldCamera::<MapWorld>::default(),
        Camera3d::default(),
        Transform::from_xyz(0.0, 10.0, 0.0),
    ));
}
```

**Server Voxel State Resource** (Line 17-20):
```rust
#[derive(Resource, Default)]
struct VoxelModifications {
    modifications: Vec<(IVec3, VoxelType)>,
}
```

**Key Characteristics**:
- Single camera with 10-chunk spawning distance
- All modifications tracked globally in Vec
- No per-client chunk state
- No client position consideration for chunk loading

### Voxel Edit Handling

**Location**: `crates/server/src/map.rs:39-74`

Edit requests follow this flow:
1. Client sends `VoxelEditRequest` message
2. Server receives via `MessageReceiver<VoxelEditRequest>`
3. Server applies via `voxel_world.set_voxel()`
4. Server tracks in `VoxelModifications`
5. Server broadcasts via `VoxelEditBroadcast` to **ALL** clients

```rust
// Line 62-71: Broadcast to all clients
sender.send::<_, VoxelChannel>(
    &VoxelEditBroadcast {
        position: request.position,
        voxel: request.voxel,
    },
    server_ref,
    &NetworkTarget::All,  // <-- No per-client filtering
).ok();
```

**Current Limitation**: `NetworkTarget::All` means every client receives every voxel edit regardless of their position or camera view.

### Initial State Synchronization

**Location**: `crates/server/src/map.rs:76-89`

New clients receive full modification history:

```rust
fn send_initial_voxel_state(
    trigger: On<Add, Connected>,
    modifications: Res<VoxelModifications>,
    mut sender: Query<&mut MessageSender<VoxelStateSync>>,
) {
    let Ok(mut message_sender) = sender.get_mut(trigger.entity) else {
        return;
    };

    message_sender.send::<VoxelChannel>(VoxelStateSync {
        modifications: modifications.modifications.clone(),  // <-- Full clone
    });
}
```

**Current Limitation**: Sends entire modification history regardless of client spawn position or view distance.

### Client Connection Tracking

**Location**: `crates/server/src/network.rs:197-210`

Server tracks connected clients via lightyear observer pattern:

```rust
fn handle_new_client(
    trigger: On<Add, Connected>,
    mut commands: Commands,
    config: Res<ServerNetworkConfig>,
) {
    info!("New client connected: {:?}", trigger.entity);
    commands
        .entity(trigger.entity)
        .insert(ReplicationSender::new(
            config.replication_interval,
            SendUpdatesMode::SinceLastAck,
            false,
        ));
}
```

**Client Entity Structure**:
- Each connected client = entity with `Connected` component
- `ReplicationSender` added for state updates
- No custom component for tracking client view position

### Client Position Storage

**Location**: `crates/protocol/src/lib.rs:135-140`, `crates/server/src/gameplay.rs:79-92`

Client positions stored in character entities:

```rust
// Character spawned with replicated Position
commands.spawn((
    Name::new("Character"),
    Position(Vec3::new(x, 3.0, z)),
    Replicate::to_clients(NetworkTarget::All),
    PredictionTarget::to_clients(NetworkTarget::All),
    ControlledBy {
        owner: client_entity,  // <-- Links character to client
        lifetime: Default::default(),
    },
    CharacterPhysicsBundle::default(),
    // ...
));
```

**Key Insight**: `ControlledBy.owner` links character entity to client connection entity, enabling position queries per client.

### Current Visibility Mechanism

**Location**: `crates/server/src/gameplay.rs:79-92`

Currently uses broadcast-all approach:

```rust
Replicate::to_clients(NetworkTarget::All),  // All clients see everything
```

**No Interest Management**:
- No distance-based culling
- No frustum culling on server
- No per-client chunk tracking

### bevy_voxel_world Architecture

**Source Location**: `git/bevy_voxel_world/`

bevy_voxel_world provides sophisticated chunk streaming but is camera-centric:

#### Chunk Spawning System

**File**: `git/bevy_voxel_world/src/voxel_world_internal.rs:110-312`

Key mechanisms:
- **Ray casting**: Shoots random rays through camera viewport + margin
- **Spawning distance**: Configurable radius (currently 10 chunks)
- **Frustum culling**: Optional `CloseAndInView` strategy
- **LOD assignment**: Based on nearest camera distance
- **Multi-camera support**: Evaluates visibility against all cameras

```rust
// Ray-based chunk discovery
for _ in 0..configuration.spawning_rays() {
    let random_point_in_viewport = Vec2::new(
        rand::random::<f32>() * (viewport_size.x + m * 2) - m,
        rand::random::<f32>() * (viewport_size.y + m * 2) - m,
    );
    queue_chunks_intersecting_ray_from_point(
        random_point_in_viewport,
        &mut chunks_deque,
    );
}
```

**Per-Camera Chunk Discovery**: Each camera triggers chunk loading via viewport ray projection.

#### Chunk Despawning System

**File**: `git/bevy_voxel_world/src/voxel_world_internal.rs:376-472`

Multi-camera despawn logic:

```rust
// Only despawn if invisible to ALL cameras
let visible_to_any_camera = cameras.iter().any(|(camera, cam_gtf, cam_pos)| {
    let chunk_at_camera = *cam_pos / CHUNK_SIZE_I;

    // Protected chunks near camera
    let near_this_camera = dist_squared <= min_despawn_distance_squared;

    // Within spawning distance
    let within_spawn_distance = dist_squared <= spawning_distance_squared;

    // Frustum visibility check
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

**Key Insight**: bevy_voxel_world already supports multiple cameras with independent visibility - could be adapted for per-client chunk management.

#### VoxelWorld API

**File**: `git/bevy_voxel_world/src/voxel_world.rs:135-407`

Public API for voxel manipulation:

```rust
pub struct VoxelWorld<C: VoxelWorldConfig> {
    chunk_map: Res<ChunkMap<C, C::MaterialIndex>>,
    modified_voxels: ResMut<ModifiedVoxels<C, C::MaterialIndex>>,
    voxel_write_buffer: ResMut<VoxelWriteBuffer<C, C::MaterialIndex>>,
}

impl<C: VoxelWorldConfig> VoxelWorld<'_, '_, C> {
    pub fn get_voxel(&self, position: IVec3) -> WorldVoxel<C::MaterialIndex> {...}
    pub fn set_voxel(&mut self, position: IVec3, voxel: WorldVoxel<C::MaterialIndex>) {...}
    pub fn get_chunk_data(&self, chunk_pos: IVec3) -> Option<ChunkData<C::MaterialIndex>> {...}
    pub fn raycast(&self, ray: Ray3d, filter: &impl Fn(...) -> bool) -> Option<VoxelRaycastResult<C::MaterialIndex>> {...}
}
```

**Relevant for Server**:
- `get_chunk_data()` - Query chunks by position
- `ModifiedVoxels` - Persistent modification tracking
- `ChunkMap` - Spatial hash of loaded chunks

#### ChunkMap Structure

**File**: `git/bevy_voxel_world/src/chunk_map.rs:18-145`

Thread-safe spatial chunk storage:

```rust
pub struct ChunkMap<C, I> {
    map: Arc<RwLock<ChunkMapData<I>>>,
    _marker: PhantomData<C>,
}

pub fn get_bounds(read_lock: &RwLockReadGuard<ChunkMapData<I>>) -> Aabb3d {
    read_lock.bounds  // AABB of loaded chunks
}
```

**Key Methods**:
- `get(position)` - Retrieve chunk data
- `contains_chunk(position)` - Check if chunk loaded
- `get_bounds()` - Get AABB of loaded chunks
- `get_world_bounds()` - AABB in world units

**Relevance**: Could query which chunks are loaded to determine what to send to clients.

### Configuration System

**Location**: `crates/protocol/src/map.rs:10-33`

Current configuration:

```rust
impl VoxelWorldConfig for MapWorld {
    type MaterialIndex = u8;
    type ChunkUserBundle = ();

    fn spawning_distance(&self) -> u32 {
        10  // 10 chunks = 320 voxels
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

**Extensibility**: Could add custom chunk tracking via `ChunkUserBundle` or new resource.

## Code References

- `crates/server/src/map.rs:24-31` - Headless camera setup
- `crates/server/src/map.rs:39-74` - Voxel edit handling and broadcast
- `crates/server/src/map.rs:76-89` - Initial state sync observer
- `crates/server/src/network.rs:197-210` - Client connection tracking
- `crates/server/src/gameplay.rs:79-92` - Character spawning with ControlledBy
- `crates/protocol/src/map.rs:10-33` - VoxelWorldConfig implementation
- `crates/protocol/src/map.rs:62-77` - Network message definitions
- `git/bevy_voxel_world/src/voxel_world_internal.rs:110-312` - Chunk spawning system
- `git/bevy_voxel_world/src/voxel_world_internal.rs:376-472` - Chunk despawning system
- `git/bevy_voxel_world/src/voxel_world.rs:135-407` - VoxelWorld public API
- `git/bevy_voxel_world/src/chunk_map.rs:18-145` - ChunkMap implementation

## Architecture Documentation

### Current Data Flow

```
Client Edit Request
    ↓
Server receives VoxelEditRequest
    ↓
Server applies to VoxelWorld (single headless camera)
    ↓
Server tracks in VoxelModifications Vec
    ↓
Server broadcasts VoxelEditBroadcast (NetworkTarget::All)
    ↓
ALL clients receive and apply
```

### Client Connection Flow

```
Client connects
    ↓
Server creates entity with Connected component
    ↓
Observer triggers: send_initial_voxel_state
    ↓
Client receives FULL VoxelModifications history
    ↓
Character spawned with ControlledBy linking to client entity
    ↓
Position replicated to all clients (NetworkTarget::All)
```

### Chunk Loading (bevy_voxel_world)

```
Camera exists with VoxelWorldCamera marker
    ↓
spawn_chunks system:
  - Cast random rays through viewport
  - Queue chunk positions intersected
  - Filter by distance (spawning_distance)
  - Filter by frustum (if CloseAndInView)
  - Spawn chunks with LOD
    ↓
Chunks loaded in ChunkMap
    ↓
retire_chunks system:
  - Evaluate each chunk against ALL cameras
  - Only despawn if invisible to ALL cameras
  - Consider distance + frustum
```

## Current Gaps and Limitations

### 1. No Per-Client Chunk Tracking

**Gap**: Server has no data structure mapping clients to their visible chunks.

**Evidence**:
- `VoxelModifications` is global, not per-client
- No `ChunkViewer` or similar component on client entities
- No queries for client position → chunk visibility

### 2. Broadcast-All Network Pattern

**Gap**: All voxel edits sent to all clients.

**Evidence**: `crates/server/src/map.rs:62-71` uses `NetworkTarget::All`

**Impact**:
- Bandwidth waste for distant clients
- Client receives edits for chunks they'll never load
- Scalability limited by total edit rate × client count

### 3. Single Headless Camera

**Gap**: Server uses one camera, not one per client.

**Evidence**: `crates/server/src/map.rs:24-31` spawns single camera

**Impact**:
- Server chunks loaded based on fixed camera position (0, 10, 0)
- Client positions don't influence server chunk loading
- Server may not have chunks loaded that clients need

### 4. Full State Sync on Connect

**Gap**: New clients receive all modifications regardless of relevance.

**Evidence**: `crates/server/src/map.rs:86-88` clones entire Vec

**Impact**:
- Connection time scales with total world edits
- Network spike on player join
- No spatial filtering

### 5. bevy_voxel_world Camera Constraint

**Gap**: bevy_voxel_world designed for single camera per world.

**Evidence**: From research, multiple `VoxelWorldCamera<MapWorld>` markers cause `.single()` panics

**Impact**:
- Cannot spawn one camera per client directly
- Would require bevy_voxel_world modifications or wrapper

## Potential Solution Approaches

### Approach 1: Per-Client Camera System (Requires bevy_voxel_world Changes)

Modify bevy_voxel_world to support multiple cameras with independent chunk sets:

**Changes Needed in bevy_voxel_world**:
1. Replace `.single()` with `.iter()` for camera queries
2. Add `ChunkOwner` component to associate chunks with cameras
3. Modify `ChunkMap` to partition by owner
4. Update spawn/despawn systems to respect ownership

**Server Implementation**:
```rust
// Spawn camera per connected client
#[derive(Component)]
struct ClientVoxelViewer {
    client_entity: Entity,
    view_distance: u32,
}

fn spawn_client_viewer(
    trigger: On<Add, Connected>,
    mut commands: Commands,
) {
    commands.spawn((
        ClientVoxelViewer {
            client_entity: trigger.entity,
            view_distance: 10,
        },
        VoxelWorldCamera::<MapWorld>::default(),
        Transform::default(),  // Updated by client position
    ));
}

fn update_viewer_positions(
    viewers: Query<(&ClientVoxelViewer, &mut Transform)>,
    characters: Query<(&ControlledBy, &Position)>,
) {
    for (viewer, mut transform) in &mut viewers {
        if let Ok((_, position)) = characters.iter()
            .find(|(controlled, _)| controlled.owner == viewer.client_entity)
        {
            transform.translation = **position;
        }
    }
}
```

**Pros**: Leverages existing bevy_voxel_world streaming logic per-client
**Cons**: Requires forking/modifying bevy_voxel_world source

### Approach 2: Custom Interest Management Layer (No bevy_voxel_world Changes)

Keep single server camera, add custom per-client chunk tracking:

**Server Implementation**:
```rust
#[derive(Component)]
struct ChunkInterest {
    known_chunks: HashSet<IVec3>,
    view_distance: u32,
}

fn track_client_chunk_interest(
    mut clients: Query<(&mut ChunkInterest, &ControlledBy)>,
    characters: Query<&Position, With<CharacterMarker>>,
) {
    for (mut interest, controlled) in &mut clients {
        if let Ok(position) = characters.get(controlled.owner) {
            let player_chunk = position.as_ivec3() / 32;
            let radius = interest.view_distance as i32;

            let mut new_chunks = HashSet::new();
            for x in -radius..=radius {
                for y in -radius..=radius {
                    for z in -radius..=radius {
                        new_chunks.insert(player_chunk + IVec3::new(x, y, z));
                    }
                }
            }
            interest.known_chunks = new_chunks;
        }
    }
}

fn send_filtered_edits(
    mut sender: ServerMultiMessageSender,
    server: Single<&Server>,
    clients: Query<(Entity, &ChunkInterest), With<Connected>>,
    modifications: Res<VoxelModifications>,
) {
    for (client_entity, interest) in &clients {
        let relevant_mods: Vec<_> = modifications.modifications.iter()
            .filter(|(pos, _)| {
                let chunk = *pos / 32;
                interest.known_chunks.contains(&chunk)
            })
            .cloned()
            .collect();

        if !relevant_mods.is_empty() {
            sender.send::<_, VoxelChannel>(
                &VoxelStateSync { modifications: relevant_mods },
                server.into_inner(),
                &NetworkTarget::Only(vec![client_entity]),
            ).ok();
        }
    }
}
```

**Pros**: No bevy_voxel_world changes needed
**Cons**: Duplicates chunk tracking logic; server still loads all chunks with single camera

### Approach 3: Hybrid - Server Loads Superset, Filters Network

Keep single camera with large spawning distance, filter sends:

**Configuration Change**:
```rust
fn spawning_distance(&self) -> u32 {
    30  // Large enough to cover all players
}
```

**Filtered Broadcasting**:
```rust
fn broadcast_voxel_edit(
    edit_position: IVec3,
    edit_voxel: VoxelType,
    clients: Query<(Entity, &Position), With<Connected>>,
    mut sender: ServerMultiMessageSender,
    server: Single<&Server>,
) {
    let edit_chunk = edit_position / 32;
    let view_distance_chunks = 10;

    let interested_clients: Vec<Entity> = clients.iter()
        .filter(|(_, position)| {
            let player_chunk = position.as_ivec3() / 32;
            player_chunk.distance_squared(edit_chunk) <= view_distance_chunks.pow(2)
        })
        .map(|(entity, _)| entity)
        .collect();

    if !interested_clients.is_empty() {
        sender.send::<_, VoxelChannel>(
            &VoxelEditBroadcast { position: edit_position, voxel: edit_voxel },
            server.into_inner(),
            &NetworkTarget::Only(interested_clients),
        ).ok();
    }
}
```

**Pros**: Simple, no bevy_voxel_world changes, server has all chunks
**Cons**: Server memory scales with total world size (within large radius)

## Related Research

- `doc/research/2025-12-24-bevy-voxel-world-map-plugins.md` - bevy_voxel_world integration research
- `doc/plans/2025-12-24-voxel-map-plugins.md` - Original voxel map implementation plan

## Open Questions

1. **Memory constraints**: How many chunks can the server reasonably keep loaded?
2. **Modification history growth**: Should `VoxelModifications` be bounded or persisted?
3. **Chunk priority**: Should server prioritize loading chunks for certain clients?
4. **Delta compression**: Should initial state sync use delta encoding for large edit sets?
5. **bevy_voxel_world fork vs. wrapper**: Is modifying the source worth per-client cameras?

---

## Follow-up Research: 2026-01-03 11:35:05 PST

### Research Question

What is the viability of refactoring bevy_voxel_world's camera-centric chunk visibility to use generic Transform-based targets? Specifically:
- Changing `VoxelWorldCamera` component to `ChunkVisibilityTarget`
- Changing `CameraInfo` to `ChunkTargetInfo` using `Transform` instead of `Camera` for activeness logic
- Adding `Option<Camera>` to `ChunkTargetInfo` to optionally support camera-reliant logic when needed

### Summary

**Viability: HIGH** - This refactoring is highly viable with minimal disruption to existing functionality.

**Key Findings**:
1. **85% of chunk logic is Transform-only**: Distance calculations, LOD assignment, and spawn distance checks use only position from `GlobalTransform.translation()`
2. **Camera methods used in only 3 places**: `viewport_to_world` (ray casting), `physical_viewport_size` (viewport dimensions), and `world_to_ndc` (frustum culling)
3. **VoxelWorldCamera is a zero-sized marker**: PhantomData-only component that tags entities, easy to rename
4. **CameraInfo is a thin SystemParam wrapper**: Simply wraps a Query, straightforward to refactor
5. **Multi-camera pattern already exists**: Systems iterate over multiple cameras and aggregate results
6. **Frustum culling is optional**: Distance-only logic works without Camera component via `ChunkDespawnStrategy::FarAway`

### Detailed Findings

#### 1. VoxelWorldCamera Component Usage

**Definition**: `git/bevy_voxel_world/src/voxel_world.rs:22-32`

```rust
#[derive(Component)]
pub struct VoxelWorldCamera<C> {
    _marker: PhantomData<C>,
}
```

**Current Role**:
- Zero-sized marker component (only `PhantomData<C>`)
- Tags which Camera entities should affect chunk spawning/despawning
- Generic over `C: VoxelWorldConfig` to support multi-world configurations

**Refactoring Path**:
```rust
// Proposed: ChunkVisibilityTarget
#[derive(Component)]
pub struct ChunkVisibilityTarget<C> {
    _marker: PhantomData<C>,
}
```

**Impact**: Simple rename, no data structure changes needed. All usages are filtered queries with `With<VoxelWorldCamera<C>>`.

**Usage Locations**:
- **3 core systems**: `spawn_chunks`, `update_chunk_lods`, `retire_chunks` (all in `voxel_world_internal.rs`)
- **5 test files**: Spawn cameras with marker for testing
- **14 example files**: Spawn cameras with marker for demos
- **Public API export**: Exported from prelude (`lib.rs:19-21`)

#### 2. CameraInfo SystemParam Analysis

**Current Definition**: `git/bevy_voxel_world/src/voxel_world_internal.rs:45-48`

```rust
#[derive(SystemParam, Deref)]
pub struct CameraInfo<'w, 's, C: VoxelWorldConfig>(
    Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<VoxelWorldCamera<C>>>,
);
```

**Data Extracted**:
- `&Camera` - Bevy camera component (viewport, projection)
- `&GlobalTransform` - World-space transform

**Refactoring Path**:
```rust
#[derive(SystemParam)]
pub struct ChunkTargetInfo<'w, 's, C: VoxelWorldConfig> {
    targets: Query<'w, 's, (&'static GlobalTransform, Option<&'static Camera>), With<ChunkVisibilityTarget<C>>>,
}

impl<'w, 's, C: VoxelWorldConfig> ChunkTargetInfo<'w, 's, C> {
    pub fn iter(&self) -> impl Iterator<Item = (&GlobalTransform, Option<&Camera>)> + '_ {
        self.targets.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
}
```

**Impact**: Minor API change from tuple `(&Camera, &GlobalTransform)` to `(&GlobalTransform, Option<&Camera>)`. Systems would adapt pattern matching.

#### 3. Camera-Specific Method Call Inventory

**Found only 3 Camera methods beyond Transform:**

##### Method 1: viewport_to_world
**Location**: `voxel_world_internal.rs:153`
**Usage**: Chunk spawning system - ray generation for visibility-based loading

```rust
// Current code
let Ok(ray) = camera.viewport_to_world(cam_gtf, point) else {
    return;
};
```

**Transform-Only Alternative**:
- Could use simple radial distance checks instead of viewport ray casting
- Or make viewport ray casting optional when `Option<Camera>` is `Some`

**Proposed Refactor**:
```rust
// If camera available, use viewport rays; otherwise, use radial flood fill
match camera_opt {
    Some(camera) => {
        // Viewport ray casting (current behavior)
        let Ok(ray) = camera.viewport_to_world(cam_gtf, point) else { return; };
        // ... ray traversal
    },
    None => {
        // Radial spawn (distance-only strategy)
        // Queue chunks within spawning_distance using 3D iteration
    }
}
```

##### Method 2: physical_viewport_size
**Location**: `voxel_world_internal.rs:148`
**Usage**: Chunk spawning - viewport dimensions for random point generation

```rust
let viewport_size = camera.physical_viewport_size().unwrap_or_default();
```

**Transform-Only Alternative**: Not needed if using distance-only spawn strategy

**Proposed Refactor**:
```rust
let viewport_size = camera_opt
    .and_then(|cam| cam.physical_viewport_size())
    .unwrap_or(UVec2::new(1920, 1080)); // Default dimensions for non-camera targets
```

##### Method 3: world_to_ndc
**Location**: `voxel_world_internal.rs:779`
**Usage**: Chunk despawning - frustum culling checks

```rust
fn chunk_visible_to_camera(
    camera: &Camera,
    cam_gtf: &GlobalTransform,
    chunk_position: IVec3,
    ndc_margin: f32,
) -> bool {
    // ... frustum checks via world_to_ndc
}
```

**Transform-Only Alternative**: Already exists via `ChunkDespawnStrategy::FarAway` config option

**Proposed Refactor**:
```rust
fn chunk_visible_to_target(
    camera_opt: Option<&Camera>,
    target_gtf: &GlobalTransform,
    chunk_position: IVec3,
    despawn_strategy: ChunkDespawnStrategy,
) -> bool {
    match (camera_opt, despawn_strategy) {
        (Some(camera), ChunkDespawnStrategy::FarAwayOrOutOfView) => {
            // Use existing frustum logic
            chunk_visible_to_camera(camera, target_gtf, chunk_position, 0.0)
        },
        _ => {
            // Distance-only logic (already exists)
            true // Handled by distance checks in caller
        }
    }
}
```

#### 4. Transform-Only Logic Patterns

**Percentage Breakdown** (from line count analysis):
- **Transform-only logic**: ~270 lines (85%)
- **Camera-dependent logic**: ~45 lines (15%)

**Transform-Only Operations**:

##### Distance Calculations (`voxel_world_internal.rs:146-265`)

```rust
// Extract position from GlobalTransform
let camera_position = cam_gtf.translation();
let cam_pos = camera_position.as_ivec3();
let chunk_at_camera = cam_pos / CHUNK_SIZE_I;

// Distance-squared checks (no Camera needed)
let spawning_distance_squared = spawning_distance.pow(2);
let within_distance_of_any_camera = cameras.iter().any(|(_, cam_gtf)| {
    let chunk_at_camera = cam_gtf.translation().as_ivec3() / CHUNK_SIZE_I;
    chunk_position.distance_squared(chunk_at_camera) <= spawning_distance_squared
});
```

**Works with**: `GlobalTransform.translation()` only

##### LOD Assignment (`voxel_world_internal.rs:328-348`)

```rust
// Collect all camera positions (Transform-only)
let camera_positions: Vec<Vec3> = camera_info
    .iter()
    .map(|(_, cam_gtf)| cam_gtf.translation())
    .collect();

// Find nearest camera by distance (Transform-only)
let nearest_camera_position = camera_positions
    .iter()
    .min_by_key(|cam_pos| {
        let chunk_center = chunk.position.as_vec3() * CHUNK_SIZE_F;
        FloatOrd(cam_pos.distance(chunk_center))
    })
    .copied()
    .unwrap_or(Vec3::ZERO);

// Call LOD config (receives position only)
let target_lod = configuration.chunk_lod(
    chunk.position,
    Some(chunk.lod_level),
    nearest_camera_position,  // <-- Vec3 position, not Camera
);
```

**Works with**: `GlobalTransform.translation()` only

##### Protected Radius Checks (`voxel_world_internal.rs:230-231, 410-411`)

```rust
let protected_chunk_radius_sq = (configuration.min_despawn_distance() as i32).pow(2);
let near_this_camera = dist_squared <= (CHUNK_SIZE_I * configuration.min_despawn_distance() as i32).pow(2);
```

**Works with**: Transform position only (prevents despawn near any target)

##### Multi-Target Aggregation (`voxel_world_internal.rs:217-220, 405-438`)

```rust
// Spawn if within distance of ANY target
let within_distance_of_any_camera = cameras.iter().any(|(_, cam_gtf)| {
    let chunk_at_camera = cam_gtf.translation().as_ivec3() / CHUNK_SIZE_I;
    chunk_position.distance_squared(chunk_at_camera) <= spawning_distance_squared
});

// Despawn only if invisible to ALL targets
let visible_to_any_camera = cameras.iter().any(|(camera, cam_gtf, cam_pos)| {
    let chunk_at_camera = *cam_pos / CHUNK_SIZE_I;
    let dist_squared = chunk.position.distance_squared(chunk_at_camera);
    let near_this_camera = dist_squared <= ...;
    let within_spawn_distance = dist_squared <= spawning_distance_squared;
    // ... optional frustum check
    near_this_camera || (within_spawn_distance && frustum_visible)
});
```

**Works with**: Transform positions, Camera optional for frustum

#### 5. Refactoring Implementation Plan

**Phase 1: Rename Components (Low Risk)**

```rust
// Before
VoxelWorldCamera<C>
// After
ChunkVisibilityTarget<C>
```

- Simple find-replace across codebase
- No behavior change
- Zero runtime cost

**Phase 2: Refactor SystemParam (Medium Risk)**

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

**Migration Pattern**:
```rust
// Before
for (camera, cam_gtf) in camera_info.iter() {
    let pos = cam_gtf.translation();
    // ... use camera for viewport/frustum
}

// After
for (cam_gtf, camera_opt) in chunk_target_info.iter() {
    let pos = cam_gtf.translation();
    if let Some(camera) = camera_opt {
        // ... use camera for viewport/frustum
    } else {
        // ... use position-only logic
    }
}
```

**Phase 3: Update System Logic (Medium Risk)**

**spawn_chunks system**:
```rust
pub fn spawn_chunks(
    // ... other params
    chunk_target_info: ChunkTargetInfo<C>,
) {
    if chunk_target_info.is_empty() {
        return;
    }

    // Collect targets with optional camera data
    let targets: Vec<(Vec3, IVec3, Option<&Camera>)> = chunk_target_info
        .iter()
        .map(|(cam_gtf, camera_opt)| {
            let pos = cam_gtf.translation();
            (pos, pos.as_ivec3(), camera_opt)
        })
        .collect();

    for (pos, ipos, camera_opt) in &targets {
        // Use camera for viewport rays if available
        if let Some(camera) = camera_opt {
            // Existing ray-based spawning
            let viewport_size = camera.physical_viewport_size().unwrap_or_default();
            // ... ray casting logic
        } else {
            // Radial distance-based spawning
            let chunk_at_target = *ipos / CHUNK_SIZE_I;
            let radius = configuration.spawning_distance() as i32;
            for x in -radius..=radius {
                for y in -radius..=radius {
                    for z in -radius..=radius {
                        let chunk_pos = chunk_at_target + IVec3::new(x, y, z);
                        chunks_deque.push_back(chunk_pos);
                    }
                }
            }
        }
    }
    // ... rest of spawn logic (all Transform-only)
}
```

**update_chunk_lods system**: No changes needed (already Transform-only)

**retire_chunks system**:
```rust
pub fn retire_chunks(
    // ... other params
    chunk_target_info: ChunkTargetInfo<C>,
) {
    if chunk_target_info.is_empty() {
        return;
    }

    let targets: Vec<(Option<&Camera>, &GlobalTransform, IVec3)> = chunk_target_info
        .iter()
        .map(|(cam_gtf, camera_opt)| {
            let cam_pos = cam_gtf.translation().as_ivec3();
            (camera_opt, cam_gtf, cam_pos)
        })
        .collect();

    for (chunk, view_visibility) in all_chunks.iter() {
        let visible_to_any_target = targets.iter().any(|(camera_opt, cam_gtf, cam_pos)| {
            // Distance checks (Transform-only)
            let chunk_at_target = *cam_pos / CHUNK_SIZE_I;
            let dist_squared = chunk.position.distance_squared(chunk_at_target);
            let near_this_target = dist_squared <= min_despawn_distance_squared;
            let within_spawn_distance = dist_squared <= spawning_distance_squared;

            // Frustum check (requires Camera)
            let frustum_visible = match (camera_opt, configuration.chunk_despawn_strategy()) {
                (Some(camera), ChunkDespawnStrategy::FarAwayOrOutOfView) => {
                    chunk_visible_to_camera(camera, cam_gtf, chunk.position, 0.0)
                },
                _ => true, // No frustum check for Transform-only targets
            };

            near_this_target || (within_spawn_distance && frustum_visible)
        });

        if !visible_to_any_target {
            commands.entity(chunk.entity).try_insert(NeedsDespawn);
        }
    }
}
```

#### 6. Backward Compatibility

**Existing Camera Entities**:
```rust
// Before (still works)
commands.spawn((
    Camera3d::default(),
    Transform::from_xyz(10.0, 10.0, 10.0),
    ChunkVisibilityTarget::<MainWorld>::default(),  // Renamed from VoxelWorldCamera
));
```

**New Transform-Only Entities**:
```rust
// After (new capability)
commands.spawn((
    Transform::from_xyz(50.0, 10.0, 50.0),  // Just position, no Camera
    ChunkVisibilityTarget::<MainWorld>::default(),
));
```

**Per-Client Targets on Server**:
```rust
// Server-side chunk visibility per client
fn spawn_client_chunk_target(
    trigger: On<Add, Connected>,
    mut commands: Commands,
) {
    commands.spawn((
        ChunkVisibilityTarget::<MapWorld>::default(),
        Transform::default(),  // Updated by client character position
        ClientChunkViewer { client_entity: trigger.entity, view_distance: 10 },
    ));
}

fn update_target_positions(
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

#### 7. Configuration Changes

**VoxelWorldConfig LOD method** (already Transform-only):
```rust
// git/bevy_voxel_world/src/configuration.rs:200-207
fn chunk_lod(
    &self,
    _chunk_position: IVec3,
    _previous_lod: Option<LodLevel>,
    _camera_position: Vec3,  // <-- Already uses position, not Camera
) -> LodLevel {
    // Default implementation uses distance tiers
}
```

**No changes needed** - already receives `Vec3` position, not `&Camera`.

**ChunkDespawnStrategy** (already supports Transform-only):
```rust
pub enum ChunkDespawnStrategy {
    FarAway,               // Distance-only (Transform)
    FarAwayOrOutOfView,    // Distance + frustum (requires Camera)
}
```

**No changes needed** - `FarAway` already works without Camera.

#### 8. Testing Strategy

**Unit Tests**:
1. Test Transform-only target spawning chunks
2. Test mixed Camera + Transform-only targets
3. Test LOD assignment with Transform-only targets
4. Test despawning with `FarAway` strategy (no Camera)

**Integration Tests**:
1. Server with per-client Transform-only targets
2. Multiple Camera targets + Transform-only targets
3. Migrating from Camera-only to mixed targets

#### 9. Performance Implications

**Positive**:
- Reduces coupling to Camera component
- Enables server-side chunk streaming without rendering overhead
- Option<Camera> pattern avoids unwrap panics

**Neutral**:
- Transform-only distance checks already used (no performance change)
- Optional Camera check adds minimal branching

**Negative**:
- None identified (refactor maintains existing performance characteristics)

### Code Reference Summary

- `git/bevy_voxel_world/src/voxel_world.rs:22-32` - VoxelWorldCamera definition
- `git/bevy_voxel_world/src/voxel_world_internal.rs:45-48` - CameraInfo SystemParam
- `git/bevy_voxel_world/src/voxel_world_internal.rs:110-312` - spawn_chunks (85% Transform-only)
- `git/bevy_voxel_world/src/voxel_world_internal.rs:315-374` - update_chunk_lods (100% Transform-only)
- `git/bevy_voxel_world/src/voxel_world_internal.rs:377-453` - retire_chunks (75% Transform-only)
- `git/bevy_voxel_world/src/voxel_world_internal.rs:148` - physical_viewport_size usage
- `git/bevy_voxel_world/src/voxel_world_internal.rs:153` - viewport_to_world usage
- `git/bevy_voxel_world/src/voxel_world_internal.rs:779` - world_to_ndc usage

### Viability Assessment

**Overall Viability: HIGH (8/10)**

**Pros**:
1. ✅ Minimal Camera dependency (only 3 method calls)
2. ✅ Majority of logic already Transform-only (85%)
3. ✅ Simple component rename (zero-sized marker)
4. ✅ Backward compatible (Camera entities still work)
5. ✅ Enables server-side per-client chunk streaming
6. ✅ Optional Camera pattern already common in Bevy
7. ✅ Configuration already uses Vec3 positions, not Camera

**Cons**:
1. ⚠️ Viewport ray casting needs fallback for Transform-only targets
2. ⚠️ Frustum culling only works with Camera (acceptable - distance-only is sufficient)
3. ⚠️ API change from `(&Camera, &GlobalTransform)` to `(&GlobalTransform, Option<&Camera>)`

**Risk Level**: Low-Medium
- Component rename: Low risk (simple find-replace)
- SystemParam refactor: Medium risk (changes iteration pattern)
- System logic updates: Medium risk (requires careful Option handling)

**Recommendation**: Proceed with refactoring in phases. Start with component rename, then SystemParam, then system logic. Maintain backward compatibility by ensuring Camera entities continue to work as before.
