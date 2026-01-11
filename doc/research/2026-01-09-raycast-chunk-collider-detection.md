---
date: 2026-01-09 09:00:07 PST
researcher: claude
git_commit: 6891d84bb1148e4d5b025b498f9369490e2ba806
branch: second
repository: second
topic: "Jump Raycasts Not Detecting Chunk Colliders"
tags: [research, physics, avian3d, raycasts, colliders, voxel-world, jump-detection, schedule-mismatch]
status: complete
last_updated: 2026-01-09
last_updated_by: claude
---

# Research: Jump Raycasts Not Detecting Chunk Colliders

**Date**: 2026-01-09 09:00:07 PST
**Researcher**: claude
**Git Commit**: 6891d84bb1148e4d5b025b498f9369490e2ba806
**Branch**: second
**Repository**: second

## Research Question
Why are players' jump raycasts not detecting the voxel world's chunk colliders despite them being applied, and how can this be fixed?

## Summary
The root cause is a **schedule mismatch** between when chunk colliders are attached and when raycasts are performed. The `attach_chunk_colliders` system runs in the `Update` schedule, while player movement with jump raycasts runs in `FixedUpdate` schedule. This timing discrepancy prevents the physics engine from consistently having colliders available when raycasts execute.

## Root Cause Analysis

### Schedule Mismatch

**Collider Attachment Schedule** - Runs in `Update`:
- Client: `crates/client/src/map.rs:21` - `protocol::attach_chunk_colliders` in `Update`
- Server: `crates/server/src/map.rs:18` - `protocol::attach_chunk_colliders` in `Update`

**Movement/Raycast Schedule** - Runs in `FixedUpdate`:
- Client: `crates/client/src/gameplay.rs:12` - `handle_character_movement` in `FixedUpdate`
- Server: `crates/server/src/gameplay.rs:14` - `handle_character_movement` in `FixedUpdate`

### The Problem

1. `FixedUpdate` runs at 64 Hz fixed timestep (defined in `crates/protocol/src/lib.rs:17`)
2. `Update` runs once per frame (variable rate)
3. These schedules are independent - `FixedUpdate` may run 0, 1, or multiple times per `Update`
4. When a chunk mesh is generated:
   - `bevy_voxel_world` adds `Mesh3d` component in some schedule
   - `attach_chunk_colliders` reacts to `Added<Mesh3d>` or `Changed<Mesh3d>` in `Update`
   - Collider is inserted on the chunk entity
   - **But**: The Avian3D physics world may not immediately register this collider
5. When `FixedUpdate` runs and performs raycasts:
   - The physics `SpatialQuery` may not see the newly added colliders yet
   - Raycasts pass through chunks as if they have no collision

## Detailed Findings

### Jump Raycast Implementation

**Location**: `crates/protocol/src/lib.rs:182-202`

The jump detection uses a downward raycast from the character's feet:

```rust
if action_state.just_pressed(&PlayerActions::Jump) {
    let ray_cast_origin = position.0
        + Vec3::new(
            0.0,
            -CHARACTER_CAPSULE_HEIGHT / 2.0 - CHARACTER_CAPSULE_RADIUS,
            0.0,
        );

    if spatial_query
        .cast_ray(
            ray_cast_origin,
            Dir3::NEG_Y,
            0.1,
            true,
            &SpatialQueryFilter::from_excluded_entities([entity]),
        )
        .is_some()
    {
        forces.apply_linear_impulse(Vec3::new(0.0, 5.0, 0.0));
    }
}
```

**Raycast Parameters**:
- Origin: Character position minus capsule dimensions (at feet)
- Direction: `Dir3::NEG_Y` (straight down)
- Distance: 0.1 units
- Solid: `true` (queries all solid colliders)
- Filter: Excludes self entity only

**Character Dimensions** (`crates/protocol/src/lib.rs:19-20`):
- `CHARACTER_CAPSULE_RADIUS = 0.5`
- `CHARACTER_CAPSULE_HEIGHT = 0.5`

### Chunk Collider Setup

**Location**: `crates/protocol/src/map.rs:82-107`

The `attach_chunk_colliders` system creates trimesh colliders for chunks:

```rust
pub fn attach_chunk_colliders(
    mut commands: Commands,
    chunks: Query<
        (Entity, &Mesh3d, Option<&Collider>),
        (With<Chunk<MapWorld>>, Or<(Changed<Mesh3d>, Added<Mesh3d>)>),
    >,
    meshes: Res<Assets<Mesh>>,
) {
    for (entity, mesh_handle, existing_collider) in chunks.iter() {
        let Some(mesh) = meshes.get(&mesh_handle.0) else {
            continue;
        };

        let Some(collider) = Collider::trimesh_from_mesh(mesh) else {
            continue;
        };

        if existing_collider.is_some() {
            commands.entity(entity).remove::<Collider>();
        }

        commands
            .entity(entity)
            .insert((collider, RigidBody::Static));
    }
}
```

**System Triggers**:
- Queries chunks with `Added<Mesh3d>` or `Changed<Mesh3d>`
- Creates `Collider::trimesh_from_mesh()` from the chunk mesh
- Inserts `(Collider, RigidBody::Static)` on chunk entity

**Voxel World Config** (`crates/protocol/src/map.rs:17-18`):
- Spawning distance: 10 chunks
- Terrain: Flat, solid below y=0, air above

### Physics Configuration

**Engine**: Avian3D 0.4.1 (`Cargo.toml:13`)

**Physics Setup** (`crates/protocol/src/lib.rs:151-162`):
```rust
app.add_plugins(lightyear::avian3d::plugin::LightyearAvianPlugin {
    replication_mode: lightyear::avian3d::plugin::AvianReplicationMode::Position,
    ..default()
});

app.add_plugins(
    PhysicsPlugins::default()
        .build()
        .disable::<PhysicsTransformPlugin>()
        .disable::<PhysicsInterpolationPlugin>()
        .disable::<IslandSleepingPlugin>(),
);
```

**Key Details**:
- Position is replicated over network (server authoritative)
- Transform and interpolation plugins disabled (Lightyear handles these)
- Island sleeping disabled for deterministic multiplayer
- **No collision layers/groups configured** - uses default collision behavior

### Character Physics

**Location**: `crates/protocol/src/lib.rs:45-62`

```rust
pub struct CharacterPhysicsBundle {
    pub collider: Collider,              // Capsule(0.5, 0.5)
    pub rigid_body: RigidBody,           // Dynamic
    pub locked_axes: LockedAxes,         // ROTATION_LOCKED
    pub friction: Friction,              // 0.0, combine rule: Min
}
```

## Code References

- `crates/protocol/src/lib.rs:182-202` - Jump raycast implementation
- `crates/protocol/src/lib.rs:17` - Fixed timestep definition (64 Hz)
- `crates/protocol/src/map.rs:82-107` - Chunk collider attachment system
- `crates/client/src/gameplay.rs:12` - Client movement in `FixedUpdate`
- `crates/server/src/gameplay.rs:14` - Server movement in `FixedUpdate`
- `crates/client/src/map.rs:21` - Client collider attachment in `Update`
- `crates/server/src/map.rs:18` - Server collider attachment in `Update`

## How to Fix

### Solution 1: Move Collider Attachment to FixedUpdate (Recommended)

Move `attach_chunk_colliders` to run in the `FixedUpdate` schedule alongside movement systems.

**Client** (`crates/client/src/map.rs:16-24`):
```rust
// Change from:
.add_systems(
    Update,
    (
        handle_voxel_broadcasts,
        handle_state_sync,
        protocol::attach_chunk_colliders,  // <- Move this
        handle_voxel_input,
    ),
)

// To:
.add_systems(
    Update,
    (
        handle_voxel_broadcasts,
        handle_state_sync,
        handle_voxel_input,
    ),
)
.add_systems(FixedUpdate, protocol::attach_chunk_colliders)
```

**Server** (`crates/server/src/map.rs:18`):
```rust
// Change from:
.add_systems(Update, (handle_voxel_edit_requests, protocol::attach_chunk_colliders))

// To:
.add_systems(Update, handle_voxel_edit_requests)
.add_systems(FixedUpdate, protocol::attach_chunk_colliders)
```

**Why this works**: Running in the same schedule ensures colliders are available when raycasts execute.

### Solution 2: Add Explicit System Ordering

Keep schedules as-is but add explicit ordering constraints to ensure collider attachment completes before movement.

```rust
.add_systems(Update, protocol::attach_chunk_colliders)
.add_systems(FixedUpdate, handle_character_movement.after(PhysicsSet::Prepare))
```

**Why this works**: Ensures physics world has processed new colliders before queries run.

### Solution 3: Move Movement to Update Schedule

Move character movement from `FixedUpdate` to `Update`.

**Trade-off**: This would break deterministic physics for multiplayer rollback, so it's **not recommended** for networked games.

### Recommended Solution

**Use Solution 1**: Move `attach_chunk_colliders` to `FixedUpdate` on both client and server. This ensures:
1. Colliders are added in the same schedule as physics queries
2. Deterministic physics for multiplayer
3. Minimal code changes
4. Consistent behavior across client and server

## Additional Considerations

### Missing Components Check

Verify chunks have all required components:
- `Chunk<MapWorld>` - Marker from bevy_voxel_world ✓
- `Mesh3d` - Generated mesh handle ✓
- `Collider` - Added by `attach_chunk_colliders` ✓
- `RigidBody::Static` - Added by `attach_chunk_colliders` ✓

All required components are present.

### Collision Group Configuration

Currently no collision layers/groups are configured. All entities use default collision (collide with everything). This is appropriate for the current setup but could be optimized later with:
- Separate collision layers for terrain vs characters
- Query filters based on collision groups
- Performance optimization for large scenes

### Query Filter Analysis

Current filter: `SpatialQueryFilter::from_excluded_entities([entity])`
- Only excludes the querying entity (self)
- Should detect all static colliders including chunks
- No layer filtering applied

This filter configuration is correct for the intended behavior.

## Architecture Documentation

### System Execution Order

**Update Schedule**:
1. `bevy_voxel_world` generates chunk meshes
2. `attach_chunk_colliders` reacts to new/changed meshes
3. Voxel edit handling systems run

**FixedUpdate Schedule** (64 Hz):
1. Input processing
2. `handle_character_movement` - applies forces and raycasts
3. Physics simulation (Avian3D)
4. Network replication (Lightyear)

**Problem**: These schedules are decoupled, creating race conditions.

### Component Flow

```
Chunk Entity Creation (bevy_voxel_world)
  ↓
Mesh3d added (bevy_voxel_world)
  ↓
attach_chunk_colliders (Update schedule)
  ↓
(Collider, RigidBody::Static) inserted
  ↓
Avian3D physics world registration (????)
  ↓
SpatialQuery sees collider (FixedUpdate schedule)
```

**Gap**: Timing uncertainty between collider insertion and physics world registration.

## Related Research

- Related to voxel world chunk loading mechanisms
- Physics schedule architecture in Avian3D
- Lightyear multiplayer determinism requirements

## Open Questions

1. Does Avian3D register colliders immediately on insertion, or does it wait for a specific schedule/system?
2. Would running `attach_chunk_colliders` in `PhysicsSchedule::Prepare` be more appropriate?
3. Are there other systems that depend on the current schedule placement of these systems?
4. Should we add debug visualization to confirm colliders are actually being created?
