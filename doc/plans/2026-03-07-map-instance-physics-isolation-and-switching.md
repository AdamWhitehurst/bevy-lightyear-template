# Map Instance Physics Isolation and Map Switching Implementation Plan

## Overview

Implement physics isolation between map instances sharing a single Avian physics world, a map switching system with loading states and lightyear room-based entity visibility, and a UI toggle button for testing Overworld/Homebase transitions.

## Current State Analysis

- Single Avian3d physics world shared by all entities, no instance separation
- Collision layers separate entity **types** (Character, Terrain, Hitbox, Projectile) but not map **instances**
- Only one map type exists (Overworld), spawned independently on server and client
- `ChunkTarget` is replicated via lightyear with `add_map_entities()`, but the `map_entity` references a local-only `VoxelMapInstance` — entity mapping is broken
- `ChunkTarget` attached to camera on client, player entity on server — inconsistent
- No `SubStates`, no map transition state machine
- No lightyear rooms — all entities visible to all clients
- `Homebase { owner: Entity }` uses Entity, which doesn't work across network boundaries

### Key Discoveries:
- `CollisionHooks::filter_pairs` does NOT affect `SpatialQuery` operations — raycast in `apply_movement` needs separate predicate filtering ([lib.rs:314-317](crates/protocol/src/lib.rs#L314-L317))
- Only one `CollisionHooks` impl per app — future needs (one-way platforms) must extend the same struct
- `ActiveCollisionHooks` is opt-in per entity — entities without it skip hook evaluation
- `RigidBodyDisabled` preserves Position/Rotation but excludes from solver — ideal for transition pausing
- Lightyear room same-frame transfer (remove old + add new) preserves visibility continuity
- `MAX_TASKS_PER_FRAME = 32` in chunk lifecycle means `pending.tasks.is_empty()` alone is unreliable for detecting load completion

## Desired End State

Players can switch between Overworld and Homebase maps via a UI button. Physics entities in different map instances never interact. The server manages authoritative map transitions with lightyear rooms controlling entity visibility. A loading screen appears during transitions while chunks generate. Homebase maps are lazily spawned on first request.

**Verification**: Two clients connected. Client A presses "Homebase" button → loading screen → appears in homebase with terrain. Client B remains in Overworld, cannot see Client A. Client A presses "Overworld" → returns to Overworld, can see Client B again. No physics interactions between maps at any point.

## What We're NOT Doing

- Arena maps (only Overworld + Homebase)
- Orphaned map cleanup (maps persist for now)
- World-space offsets between maps (physics isolation handles overlap)
- Portal-based or proximity-triggered transitions (UI button only)
- Chunk loading progress bar (binary loading screen)
- Homebase persistence to disk
- Per-homebase voxel modification sync

## Implementation Approach

Six phases, each independently testable. Phase 1-2 establish the data model and physics isolation (testable with one map). Phase 3 adds lightyear rooms. Phase 4-5 implement the transition protocol and UI. Phase 6 adds homebase map spawning.

Every failure point in transition logic uses `panic!` or `expect("reason")` for immediate debuggability.

---

## Phase 1: MapInstanceId, MapRegistry, and MapCollisionHooks

### Overview
Define the core identity and physics isolation primitives. After this phase, all physics entities carry a `MapInstanceId`, and cross-map collisions are filtered.

### Changes Required:

#### 1. MapInstanceId component and MapRegistry resource
**File**: `crates/protocol/src/map.rs`

Add after existing types:

```rust
/// Identifies which map instance an entity belongs to.
/// Semantic enum — safe to replicate, no Entity references.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum MapInstanceId {
    Overworld,
    Homebase { owner: ClientId },
}

/// Maps semantic MapInstanceId to local VoxelMapInstance entities.
/// Each side (server/client) maintains independently.
#[derive(Resource, Default)]
pub struct MapRegistry(pub HashMap<MapInstanceId, Entity>);

impl MapRegistry {
    pub fn get(&self, id: &MapInstanceId) -> Entity {
        *self.0.get(id).unwrap_or_else(|| {
            panic!("MapRegistry lookup failed for {id:?} — map not registered")
        })
    }

    pub fn insert(&mut self, id: MapInstanceId, entity: Entity) {
        self.0.insert(id, entity);
    }
}
```

Register `MapInstanceId` with lightyear in `ProtocolPlugin` (no `add_prediction()` — server-authoritative, no rollback):

```rust
app.register_component::<MapInstanceId>();
```

#### 2. MapCollisionHooks
**File**: `crates/protocol/src/physics.rs` (new file)

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use crate::map::MapInstanceId;

/// Collision hooks for map instance isolation.
/// Only one CollisionHooks impl per app — extend this struct for future needs.
#[derive(SystemParam)]
pub struct MapCollisionHooks<'w, 's> {
    map_ids: Query<'w, 's, &'static MapInstanceId>,
}

impl CollisionHooks for MapCollisionHooks<'_, '_> {
    fn filter_pairs(&self, entity1: Entity, entity2: Entity, _commands: &mut Commands) -> bool {
        let entity1_id = self.map_ids.get(entity1).ok();
        let entity2_id = self.map_ids.get(entity2).ok();
        match (entity1_id, entity2_id) {
            (Some(a), Some(b)) => a == b,
            _ => panic!("Entity missing MapInstanceId. Entity {entity1:?}: {entity1_id:?}. Entity {entity2:?}: {entity2_id:?}"),
        }
    }
}
```

#### 3. Register hooks with PhysicsPlugins
**File**: `crates/protocol/src/lib.rs` (line 242-248)

Change:
```rust
app.add_plugins(
    PhysicsPlugins::default()
        .build()
        .disable::<PhysicsTransformPlugin>()
        .disable::<PhysicsInterpolationPlugin>()
        .disable::<IslandSleepingPlugin>(),
);
```
To:
```rust
app.add_plugins(
    PhysicsPlugins::default()
        .with_collision_hooks::<MapCollisionHooks>()
        .build()
        .disable::<PhysicsTransformPlugin>()
        .disable::<PhysicsInterpolationPlugin>()
        .disable::<IslandSleepingPlugin>(),
);
```

#### 4. Insert MapInstanceId on all physics entity spawn points

**Server character spawn** (`crates/server/src/gameplay.rs:189-207`):
Add `MapInstanceId::Overworld` to the spawn bundle.

**Server dummy spawn** (`crates/server/src/gameplay.rs:61-73`):
Add `MapInstanceId::Overworld` to the spawn bundle.

**Client predicted character** (`crates/client/src/gameplay.rs:47-52`):
When inserting `CharacterPhysicsBundle`, also insert `MapInstanceId::Overworld`.

Note: Once replication is working (Phase 3+), the client will receive `MapInstanceId` from the server. For now, hardcode `Overworld`.

**Chunk colliders** (`crates/protocol/src/map.rs:46-72`):
In `attach_chunk_colliders`, look up the chunk's parent map entity's `MapInstanceId` and insert it on the chunk:

```rust
pub fn attach_chunk_colliders(
    mut commands: Commands,
    chunks: Query<
        (Entity, &Mesh3d, &ChildOf, Option<&Collider>),
        (With<VoxelChunk>, Or<(Changed<Mesh3d>, Added<Mesh3d>)>),
    >,
    map_ids: Query<&MapInstanceId>,
    meshes: Res<Assets<Mesh>>,
) {
    for (entity, mesh_handle, child_of, existing_collider) in chunks.iter() {
        let Some(mesh) = meshes.get(&mesh_handle.0) else {
            warn!("Chunk entity {entity:?} has Mesh3d but mesh asset not found");
            continue;
        };
        let Some(collider) = Collider::trimesh_from_mesh(mesh) else {
            warn!("Failed to create trimesh collider for chunk entity {entity:?}");
            continue;
        };
        if existing_collider.is_some() {
            commands.entity(entity).remove::<Collider>();
        }

        let map_instance_id = map_ids.get(child_of.parent())
            .expect("Chunk parent map entity must have MapInstanceId");

        commands.entity(entity).insert((
            collider,
            RigidBody::Static,
            crate::hit_detection::terrain_collision_layers(),
            map_instance_id.clone(),
        ));
    }
}
```

**Hitbox spawns** (`crates/protocol/src/ability.rs`):
In `spawn_melee_hitbox` (~line 1097) and `spawn_aoe_hitbox` (~line 1138), clone `MapInstanceId` from the caster entity and insert it on the hitbox. Add `MapInstanceId` to the caster query.

**Projectile spawns** (`crates/protocol/src/ability.rs` ~line 1449):
In `handle_ability_projectile_spawn`, clone `MapInstanceId` from the spawn source entity and insert on the projectile.

#### 5. ActiveCollisionHooks opt-in

Use Bevy's `#[require]` on `MapInstanceId`:
```rust
#[derive(Component, ...)]
#[require(ActiveCollisionHooks(|| ActiveCollisionHooks::FILTER_PAIRS))]
pub enum MapInstanceId { ... }
```

If `#[require]` doesn't support closure syntax for this type, insert `ActiveCollisionHooks::FILTER_PAIRS` explicitly alongside every `MapInstanceId` insertion.

#### 6. Fix apply_movement raycast for map filtering
**File**: `crates/protocol/src/lib.rs` (line 296-340)

Add `map_ids: &Query<&MapInstanceId>` parameter to `apply_movement`. Change the jump raycast:

```rust
let player_map_id = map_ids.get(entity).ok();
let filter = SpatialQueryFilter::from_excluded_entities([entity]);
if spatial_query
    .cast_ray_predicate(ray_cast_origin, Dir3::NEG_Y, 4.0, false, &filter, &|hit_entity| {
        match (player_map_id, map_ids.get(hit_entity).ok()) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
    })
    .is_some()
{
    forces.apply_linear_impulse(Vec3::new(0.0, 400.0, 0.0));
}
```

Update both callers:
- `crates/server/src/gameplay.rs:76-101` — add `map_ids: Query<&MapInstanceId>` param, pass to `apply_movement`
- `crates/client/src/gameplay.rs:55-80` — same

#### 7. Register MapRegistry resource and populate on overworld spawn

**Server** (`crates/server/src/map.rs`):
After `commands.insert_resource(OverworldMap(map))`, also insert into `MapRegistry`:
```rust
commands.insert_resource(MapRegistry::default());
// In spawn_overworld, after creating the map entity:
// (Use a separate startup system or init_resource for MapRegistry, then mutate)
```

Better: `init_resource::<MapRegistry>()` in the plugin, then in `spawn_overworld`:
```rust
fn spawn_overworld(mut commands: Commands, map_world: Res<MapWorld>, mut registry: ResMut<MapRegistry>) {
    let map = commands
        .spawn((
            VoxelMapInstance::new(5),
            VoxelMapConfig::new(map_world.seed, 2, None, 5, Arc::new(flat_terrain_voxels)),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();
    commands.insert_resource(OverworldMap(map));
    registry.insert(MapInstanceId::Overworld, map);
}
```

**Client** (`crates/client/src/map.rs`):
Same pattern — `init_resource::<MapRegistry>()` in plugin, insert in `spawn_overworld`.

### Tests:

#### Unit tests (`crates/protocol/src/map.rs`):
```rust
#[test]
fn map_instance_id_equality() {
    assert_eq!(MapInstanceId::Overworld, MapInstanceId::Overworld);
    assert_ne!(MapInstanceId::Overworld, MapInstanceId::Homebase { owner: ClientId::Local });
}

#[test]
fn map_registry_get_panics_on_missing() {
    let registry = MapRegistry::default();
    let result = std::panic::catch_unwind(|| registry.get(&MapInstanceId::Overworld));
    assert!(result.is_err());
}

#[test]
fn map_registry_insert_and_get() {
    let mut registry = MapRegistry::default();
    let entity = Entity::from_raw(42);
    registry.insert(MapInstanceId::Overworld, entity);
    assert_eq!(registry.get(&MapInstanceId::Overworld), entity);
}
```

#### Integration test: physics isolation via CollisionHooks (`crates/protocol/tests/physics_isolation.rs`)

Uses a real Bevy App with `PhysicsPlugins` + `MapCollisionHooks` to verify that entities on different maps don't generate contacts, and entities on the same map do.

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use protocol::map::{MapInstanceId, MapRegistry};
use protocol::physics::MapCollisionHooks;

fn physics_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(
        PhysicsPlugins::default()
            .with_collision_hooks::<MapCollisionHooks>()
            .build()
            .disable::<PhysicsInterpolationPlugin>()
            .disable::<IslandSleepingPlugin>(),
    );
    app.init_resource::<MapRegistry>();
    app
}

/// Spawn a dynamic sphere at a position with a MapInstanceId
fn spawn_physics_entity(app: &mut App, pos: Vec3, map_id: MapInstanceId) -> Entity {
    app.world_mut().spawn((
        RigidBody::Dynamic,
        Collider::sphere(1.0),
        Position(pos),
        CollisionLayers::all::<GameLayer>(),
        map_id,
        ActiveCollisionHooks::FILTER_PAIRS,
        CollidingEntities::default(),
    )).id()
}

#[test]
fn same_map_entities_collide() {
    let mut app = physics_test_app();
    // Two overlapping spheres on the same map
    let a = spawn_physics_entity(&mut app, Vec3::ZERO, MapInstanceId::Overworld);
    let b = spawn_physics_entity(&mut app, Vec3::new(0.5, 0.0, 0.0), MapInstanceId::Overworld);

    // Run physics for several ticks
    for _ in 0..20 { app.update(); }

    // They should have collided — CollidingEntities should be non-empty for at least one
    let colliding_a = app.world().get::<CollidingEntities>(a).unwrap();
    let colliding_b = app.world().get::<CollidingEntities>(b).unwrap();
    assert!(
        colliding_a.contains(&b) || colliding_b.contains(&a),
        "Entities on the same map should collide"
    );
}

#[test]
fn different_map_entities_do_not_collide() {
    let mut app = physics_test_app();
    // Two overlapping spheres on DIFFERENT maps
    let a = spawn_physics_entity(&mut app, Vec3::ZERO, MapInstanceId::Overworld);
    let b = spawn_physics_entity(&mut app, Vec3::new(0.5, 0.0, 0.0), MapInstanceId::Homebase { owner: ClientId::Local });

    for _ in 0..20 { app.update(); }

    let colliding_a = app.world().get::<CollidingEntities>(a).unwrap();
    let colliding_b = app.world().get::<CollidingEntities>(b).unwrap();
    assert!(
        !colliding_a.contains(&b) && !colliding_b.contains(&a),
        "Entities on different maps must not collide"
    );
}

#[test]
fn entity_without_map_id_collides_with_everything() {
    let mut app = physics_test_app();
    // Entity WITH MapInstanceId
    let a = spawn_physics_entity(&mut app, Vec3::ZERO, MapInstanceId::Overworld);
    // Entity WITHOUT MapInstanceId (global entity)
    let b = app.world_mut().spawn((
        RigidBody::Dynamic,
        Collider::sphere(1.0),
        Position(Vec3::new(0.5, 0.0, 0.0)),
        CollisionLayers::all::<GameLayer>(),
        CollidingEntities::default(),
    )).id();

    for _ in 0..20 { app.update(); }

    let colliding_a = app.world().get::<CollidingEntities>(a).unwrap();
    let colliding_b = app.world().get::<CollidingEntities>(b).unwrap();
    assert!(
        colliding_a.contains(&b) || colliding_b.contains(&a),
        "Entity without MapInstanceId should collide with everything"
    );
}
```

#### Integration test: chunk colliders inherit MapInstanceId (`crates/protocol/tests/physics_isolation.rs`)

```rust
#[test]
fn chunk_colliders_inherit_map_instance_id() {
    // Uses VoxelPlugin + attach_chunk_colliders to verify chunks get parent's MapInstanceId
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::transform::TransformPlugin);
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.add_plugins(VoxelPlugin);
    app.add_systems(Update, attach_chunk_colliders);
    app.init_resource::<MapRegistry>();

    let map = app.world_mut().spawn((
        VoxelMapInstance::new(5),
        VoxelMapConfig::new(0, 1, None, 5, Arc::new(flat_terrain_voxels)),
        Transform::default(),
        MapInstanceId::Overworld,
    )).id();

    // Spawn a ChunkTarget to trigger chunk loading
    app.world_mut().spawn((
        ChunkTarget { map_entity: map, distance: 0 },
        Transform::default(),
    ));

    // Tick until chunks load and colliders attach
    for _ in 0..30 { app.update(); }

    // All chunk entities that are children of the map should have MapInstanceId::Overworld
    let mut chunks_with_map_id = 0;
    let mut chunks_without = 0;
    for (chunk_map_id, child_of) in app.world_mut().query::<(&MapInstanceId, &ChildOf)>()
        .iter(app.world())
        .filter(|(_, c)| c.parent() == map)
    {
        assert_eq!(*chunk_map_id, MapInstanceId::Overworld);
        chunks_with_map_id += 1;
    }
    // Also check no VoxelChunk children are missing MapInstanceId
    for (_, child_of) in app.world_mut().query::<(&VoxelChunk, &ChildOf)>().iter(app.world()) {
        if child_of.parent() == map {
            if app.world().get::<MapInstanceId>(child_of.parent()).is_none() {
                chunks_without += 1;
            }
        }
    }
    assert!(chunks_with_map_id > 0, "Should have at least one chunk with MapInstanceId");
    assert_eq!(chunks_without, 0, "No chunks should be missing MapInstanceId on parent");
}
```

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test-native`
- [x] `cargo check-all` passes
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client`

#### Manual Verification:
- [ ] Characters still collide with terrain (jump works)
- [ ] Hitboxes still detect character hits
- [ ] No physics regressions from hook registration

---

## Phase 2: ChunkTarget Decoupling

### Overview
Remove `ChunkTarget` from lightyear replication. Each side derives it locally from `MapInstanceId` + `MapRegistry`. Move client `ChunkTarget` from camera to player entity.

### Changes Required:

#### 1. Remove ChunkTarget lightyear registration
**File**: `crates/protocol/src/lib.rs` (line 167)

Delete:
```rust
app.register_component::<ChunkTarget>().add_map_entities();
```

#### 2. Remove MapEntities impl from ChunkTarget
**File**: `crates/voxel_map_engine/src/chunk.rs` (lines 1, 19-23)

Remove the `use bevy::ecs::entity::{EntityMapper, MapEntities};` import and the `impl MapEntities for ChunkTarget` block. Also remove `Serialize, Deserialize` derives since it's no longer replicated.

#### 3. Client: move ChunkTarget from camera to player entity
**File**: `crates/client/src/map.rs`

Replace `attach_chunk_target_to_camera` with a system that attaches `ChunkTarget` to the predicted player entity:

```rust
fn attach_chunk_target_to_player(
    mut commands: Commands,
    registry: Res<MapRegistry>,
    players: Query<(Entity, &MapInstanceId), (With<Predicted>, With<CharacterMarker>, Without<ChunkTarget>)>,
) {
    for (entity, map_id) in &players {
        let map_entity = registry.get(map_id);
        commands.entity(entity).insert(ChunkTarget::new(map_entity, 4));
    }
}
```

#### 4. Server: derive ChunkTarget from MapInstanceId
**File**: `crates/server/src/gameplay.rs`

In `handle_connected` (line 189-207), replace `ChunkTarget::new(overworld.0, 4)` with deriving from `MapRegistry`:

```rust
fn handle_connected(
    // ... existing params ...
    registry: Res<MapRegistry>,
) {
    // ... existing code ...
    let map_id = MapInstanceId::Overworld;
    let map_entity = registry.get(&map_id);
    commands.spawn((
        // ... existing components ...
        map_id,
        ChunkTarget::new(map_entity, 4),
    ));
}
```

Same for `spawn_dummy_target` — derive from `MapRegistry` instead of `OverworldMap`.

### Tests:

#### Integration test: ChunkTarget derived from MapRegistry (`crates/voxel_map_engine/tests/lifecycle.rs` — extend existing)

The existing `switching_chunk_target_between_maps` test validates core target-switching. Add a test that verifies the MapRegistry-driven derivation pattern:

```rust
#[test]
fn chunk_target_derived_from_map_registry() {
    let mut app = test_app();
    app.init_resource::<MapRegistry>();

    let map_a = spawn_map(&mut app, 1);
    let map_b = spawn_map(&mut app, 1);

    // Register maps in registry
    {
        let mut registry = app.world_mut().resource_mut::<MapRegistry>();
        registry.insert(MapInstanceId::Overworld, map_a);
        registry.insert(MapInstanceId::Homebase { owner: 12345 }, map_b);
    }

    // Derive ChunkTarget from registry (simulates what client/server do)
    let target_map = app.world().resource::<MapRegistry>().get(&MapInstanceId::Overworld);
    let target = spawn_target(&mut app, target_map, Vec3::ZERO, 0);

    tick(&mut app, 20);
    assert_eq!(loaded_chunk_count(&app, map_a), 1);
    assert_eq!(loaded_chunk_count(&app, map_b), 0);

    // Switch to Homebase via registry lookup
    let new_map = app.world().resource::<MapRegistry>()
        .get(&MapInstanceId::Homebase { owner: 12345 });
    app.world_mut().entity_mut(target).insert(ChunkTarget { map_entity: new_map, distance: 0 });

    tick(&mut app, 20);
    assert_eq!(loaded_chunk_count(&app, map_a), 0, "old map should unload");
    assert_eq!(loaded_chunk_count(&app, map_b), 1, "new map should load");
}
```

#### Integration test: player entity (not camera) drives chunk loading (`crates/voxel_map_engine/tests/lifecycle.rs`)

```rust
#[test]
fn player_entity_drives_chunk_loading() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);

    // Simulate player entity with ChunkTarget (instead of camera)
    let player = app.world_mut().spawn((
        ChunkTarget { map_entity: map, distance: 1 },
        Transform::from_translation(Vec3::ZERO),
    )).id();

    tick(&mut app, 20);
    assert_eq!(loaded_chunk_count(&app, map), 27, "player-driven ChunkTarget should load 3^3 chunks");

    // Move player far away — origin chunks should unload
    app.world_mut().entity_mut(player)
        .insert(Transform::from_translation(Vec3::new(10000.0, 0.0, 0.0)));
    tick(&mut app, 20);

    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    assert!(!instance.loaded_chunks.contains(&IVec3::ZERO), "origin chunks should unload after player moves");
}
```
%% Wont this test just load chunks around the player at the teleported position? Why not test unloading of chunks by removing ChunkTarget on player?

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test-native`
- [x] `cargo check-all` passes
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client`

#### Manual Verification:
- [ ] Chunks still load around the player on both server and client
- [ ] Moving the player causes new chunks to load and distant chunks to unload
- [ ] Camera detached from chunk loading (no ChunkTarget on camera)

---

## Phase 3: Lightyear Rooms for Entity Visibility

### Overview
Set up lightyear rooms so that entities are only visible to clients in the same map instance. This is required before map switching — without rooms, a player switching maps would still see entities from their old map.

### Changes Required:

#### 1. RoomRegistry resource
**File**: `crates/server/src/map.rs`

```rust
/// Maps MapInstanceId to lightyear room entities. Server-only.
#[derive(Resource, Default)]
pub struct RoomRegistry(pub HashMap<MapInstanceId, Entity>);

impl RoomRegistry {
    pub fn get_or_create(&mut self, id: &MapInstanceId, commands: &mut Commands) -> Entity {
        *self.0.entry(id.clone()).or_insert_with(|| {
            let room = commands.spawn(Room::default()).id();
            info!("Created room for map {id:?}: {room:?}");
            room
        })
    }
}
```

#### 2. Auto-assign entities to rooms on MapInstanceId insert
**File**: `crates/server/src/map.rs`

Observer on `MapInstanceId` insertion:

```rust
fn on_map_instance_id_added(
    trigger: On<Add, MapInstanceId>,
    mut commands: Commands,
    map_ids: Query<&MapInstanceId>,
    mut room_registry: ResMut<RoomRegistry>,
) {
    let entity = trigger.entity();
    let map_id = map_ids.get(entity)
        .expect("Entity with MapInstanceId trigger must have MapInstanceId");
    let room = room_registry.get_or_create(map_id, &mut commands);
    commands.trigger(RoomEvent {
        room,
        target: RoomTarget::AddEntity(entity),
    });
}
```

#### 3. Add client sender to room on connection
**File**: `crates/server/src/gameplay.rs`

In `handle_connected`, after spawning the character, add the client sender entity to the same room:

```rust
let room = room_registry.get_or_create(&MapInstanceId::Overworld, &mut commands);
commands.trigger(RoomEvent {
    room,
    target: RoomTarget::AddSender(client_entity),
});
```

#### 4. Init resources
**File**: `crates/server/src/map.rs` (plugin build)

```rust
app.init_resource::<RoomRegistry>();
```

### Tests:

#### Integration test: room-based entity visibility (`crates/server/tests/rooms.rs`)

Uses `CrossbeamTestStepper` pattern to verify that entities in different rooms are not visible to each other's clients.

```rust
use ::server::network::{ServerNetworkConfig, ServerNetworkPlugin, ServerTransport};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use lightyear::prelude::*;
use lightyear::prelude::client as lightyear_client;
use lightyear::prelude::server as lightyear_server;
use lightyear_server::*;
use protocol::*;
use protocol::map::{MapInstanceId, MapRegistry};
use server::map::RoomRegistry;
use std::time::Duration;

/// Verify that a replicated entity in Room A is NOT visible to a client in Room B.
/// Uses CrossbeamTestStepper for in-memory client-server.
#[test]
fn entity_in_different_room_not_replicated() {
    let mut stepper = CrossbeamTestStepper::new();
    stepper.init();
    assert!(stepper.wait_for_connection(), "Client should connect");

    // Create two rooms on server
    let room_a = stepper.server_app.world_mut().spawn(Room::default()).id();
    let room_b = stepper.server_app.world_mut().spawn(Room::default()).id();

    // Add client sender to room_a
    stepper.server_app.world_mut().commands().trigger(RoomEvent {
        room: room_a,
        target: RoomTarget::AddSender(stepper.client_of_entity),
    });

    // Spawn a replicated entity in room_b (client should NOT see it)
    let hidden_entity = stepper.server_app.world_mut().spawn((
        Name::new("HiddenEntity"),
        Position(Vec3::new(99.0, 0.0, 0.0)),
        Replicate::to_clients(NetworkTarget::All),
        CharacterMarker,
    )).id();
    stepper.server_app.world_mut().commands().trigger(RoomEvent {
        room: room_b,
        target: RoomTarget::AddEntity(hidden_entity),
    });

    // Spawn a replicated entity in room_a (client SHOULD see it)
    let visible_entity = stepper.server_app.world_mut().spawn((
        Name::new("VisibleEntity"),
        Position(Vec3::new(1.0, 0.0, 0.0)),
        Replicate::to_clients(NetworkTarget::All),
        CharacterMarker,
    )).id();
    stepper.server_app.world_mut().commands().trigger(RoomEvent {
        room: room_a,
        target: RoomTarget::AddEntity(visible_entity),
    });

    // Tick to replicate
    stepper.tick_step(10);

    // Client should have received the visible entity but not the hidden one
    let mut client_characters = stepper.client_app.world_mut()
        .query_filtered::<&Position, With<CharacterMarker>>();
    let positions: Vec<Vec3> = client_characters.iter(stepper.client_app.world())
        .map(|p| p.0)
        .collect();

    assert!(
        positions.iter().any(|p| p.x == 1.0),
        "Client should see entity in same room (room_a). Positions: {positions:?}"
    );
    assert!(
        !positions.iter().any(|p| p.x == 99.0),
        "Client must NOT see entity in different room (room_b). Positions: {positions:?}"
    );
}

/// Verify same-frame room transfer: remove from room_a + add to room_b does not cause
/// a visibility gap (entity doesn't flicker/despawn on client).
#[test]
fn same_frame_room_transfer_preserves_visibility() {
    let mut stepper = CrossbeamTestStepper::new();
    stepper.init();
    assert!(stepper.wait_for_connection());

    let room_a = stepper.server_app.world_mut().spawn(Room::default()).id();
    let room_b = stepper.server_app.world_mut().spawn(Room::default()).id();

    // Client sender in BOTH rooms (so it sees entities in either)
    stepper.server_app.world_mut().commands().trigger(RoomEvent {
        room: room_a, target: RoomTarget::AddSender(stepper.client_of_entity),
    });
    stepper.server_app.world_mut().commands().trigger(RoomEvent {
        room: room_b, target: RoomTarget::AddSender(stepper.client_of_entity),
    });

    // Spawn entity in room_a
    let entity = stepper.server_app.world_mut().spawn((
        Name::new("TransferEntity"),
        Position(Vec3::new(42.0, 0.0, 0.0)),
        Replicate::to_clients(NetworkTarget::All),
        CharacterMarker,
    )).id();
    stepper.server_app.world_mut().commands().trigger(RoomEvent {
        room: room_a, target: RoomTarget::AddEntity(entity),
    });

    stepper.tick_step(10);

    // Verify client sees entity
    let count_before = stepper.client_app.world_mut()
        .query_filtered::<Entity, With<CharacterMarker>>()
        .iter(stepper.client_app.world()).count();
    assert!(count_before >= 1, "Client should see entity before transfer");

    // Same-frame transfer: remove from room_a, add to room_b
    stepper.server_app.world_mut().commands().trigger(RoomEvent {
        room: room_a, target: RoomTarget::RemoveEntity(entity),
    });
    stepper.server_app.world_mut().commands().trigger(RoomEvent {
        room: room_b, target: RoomTarget::AddEntity(entity),
    });

    stepper.tick_step(5);

    // Entity should still be visible (never dropped to 0 rooms while client is in both)
    let count_after = stepper.client_app.world_mut()
        .query_filtered::<Entity, With<CharacterMarker>>()
        .iter(stepper.client_app.world()).count();
    assert!(count_after >= 1, "Entity should remain visible after same-frame room transfer");
}
```

### Success Criteria:

#### Automated Verification:
- [ ] All tests pass: `cargo test-native`
- [ ] `cargo check-all` passes
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client`

#### Manual Verification:
- [ ] With two clients connected, both see each other in the Overworld (same room)
- [ ] Server logs show room creation and entity/sender assignment
- [ ] No regressions — entities still replicate correctly

---

## Phase 4: Map Transition Protocol and State Machine

### Overview
Implement the full map switching flow: client requests via message, server executes transition, client handles loading state. This is the largest phase.

### Changes Required:

#### 1. Messages and channel
**File**: `crates/protocol/src/map.rs`

```rust
/// Channel for map transition messages
pub struct MapChannel;

/// Client requests to switch maps
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct PlayerMapSwitchRequest {
    pub target: MapSwitchTarget,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
pub enum MapSwitchTarget {
    Overworld,
    Homebase,
}

/// Server tells client to begin transition
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
pub struct MapTransitionStart {
    pub target: MapInstanceId,
    pub seed: u64,
    pub generation_version: u32,
    pub bounds: Option<IVec3>,
}
```

**File**: `crates/protocol/src/lib.rs`

Register channel and messages:
```rust
app.add_channel::<MapChannel>(ChannelSettings {
    mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
    ..default()
})
.add_direction(NetworkDirection::Bidirectional);

app.register_message::<PlayerMapSwitchRequest>()
    .add_direction(NetworkDirection::ClientToServer);
app.register_message::<MapTransitionStart>()
    .add_direction(NetworkDirection::ServerToClient);
```

#### 2. MapTransitionState sub-state (client)
**File**: `crates/ui/src/state.rs`

```rust
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, SubStates)]
#[source(ClientState = ClientState::InGame)]
pub enum MapTransitionState {
    #[default]
    Playing,
    Transitioning,
}
```

**File**: `crates/ui/src/lib.rs`

Register the sub-state:
```rust
app.add_sub_state::<MapTransitionState>();
```

#### 3. Server-side transition handler
**File**: `crates/server/src/map.rs`

```rust
/// Marker: player is currently transitioning maps. Prevents double-transitions.
#[derive(Component)]
pub struct MapTransitioning;

fn handle_map_switch_requests(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<PlayerMapSwitchRequest>>,
    mut senders: Query<&mut MessageSender<MapTransitionStart>>,
    controlled_query: Query<(Entity, &ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    transitioning: Query<(), With<MapTransitioning>>,
    mut registry: ResMut<MapRegistry>,
    mut room_registry: ResMut<RoomRegistry>,
    map_world: Res<MapWorld>,
) {
    for mut receiver in &mut receivers {
        for (request, client_entity) in receiver.receive() {
            let (player_entity, controlled_by, current_map_id) = controlled_query
                .iter()
                .find(|(_, ctrl, _)| ctrl.owner == client_entity)
                .unwrap_or_else(|| {
                    panic!("No character entity found for client {client_entity:?} during map switch")
                });

            if transitioning.get(player_entity).is_ok() {
                warn!("Player {player_entity:?} already transitioning, ignoring request");
                continue;
            }

            let target_map_id = resolve_switch_target(&request.target, client_entity);

            if *current_map_id == target_map_id {
                warn!("Player {player_entity:?} already on target map {target_map_id:?}");
                continue;
            }

            execute_server_transition(
                &mut commands,
                player_entity,
                client_entity,
                current_map_id,
                &target_map_id,
                &mut registry,
                &mut room_registry,
                &map_world,
                &mut senders,
            );
        }
    }
}

fn resolve_switch_target(target: &MapSwitchTarget, client_entity: Entity) -> MapInstanceId {
    match target {
        MapSwitchTarget::Overworld => MapInstanceId::Overworld,
        MapSwitchTarget::Homebase => MapInstanceId::Homebase {
            owner: ClientId::from(client_entity), // TODO: resolve actual ClientId from RemoteId
        },
    }
}

fn execute_server_transition(
    commands: &mut Commands,
    player_entity: Entity,
    client_entity: Entity,
    current_map_id: &MapInstanceId,
    target_map_id: &MapInstanceId,
    registry: &mut MapRegistry,
    room_registry: &mut RoomRegistry,
    map_world: &MapWorld,
    senders: &mut Query<&mut MessageSender<MapTransitionStart>>,
) {
    info!("Transitioning player {player_entity:?} from {current_map_id:?} to {target_map_id:?}");

    // 1. Freeze player physics
    commands.entity(player_entity).insert((
        RigidBodyDisabled,
        DisableRollback,
        MapTransitioning,
    ));

    // 2. Room transitions — remove from old, add to new (same frame = no visibility gap)
    let old_room = room_registry.get_or_create(current_map_id, commands);
    let new_room = room_registry.get_or_create(target_map_id, commands);

    commands.trigger(RoomEvent { room: old_room, target: RoomTarget::RemoveEntity(player_entity) });
    commands.trigger(RoomEvent { room: old_room, target: RoomTarget::RemoveSender(client_entity) });
    commands.trigger(RoomEvent { room: new_room, target: RoomTarget::AddEntity(player_entity) });
    commands.trigger(RoomEvent { room: new_room, target: RoomTarget::AddSender(client_entity) });

    // 3. Update MapInstanceId (replicates to client)
    commands.entity(player_entity).insert(target_map_id.clone());

    // 4. Update ChunkTarget
    let map_entity = registry.get(target_map_id);
    commands.entity(player_entity).insert(ChunkTarget::new(map_entity, 4));

    // 5. Teleport to spawn point
    commands.entity(player_entity).insert((
        Position(Vec3::new(0.0, 30.0, 0.0)),
        LinearVelocity(Vec3::ZERO),
    ));

    // 6. Determine config for target map
    let (seed, bounds) = match target_map_id {
        MapInstanceId::Overworld => (map_world.seed, None),
        MapInstanceId::Homebase { owner } => {
            let homebase_seed = owner.to_bits();
            (homebase_seed, Some(IVec3::new(4, 4, 4)))
        }
    };

    // 7. Send transition start to client
    let mut sender = senders.get_mut(client_entity)
        .expect("Client entity must have MessageSender<MapTransitionStart>");
    sender.send::<MapChannel>(&MapTransitionStart {
        target: target_map_id.clone(),
        seed,
        generation_version: map_world.generation_version,
        bounds,
    });

    // 8. Unfreeze after a short delay (or client confirmation — for now, fixed tick delay)
    // TODO: Replace with client confirmation message. For now, use a timer component.
    commands.entity(player_entity).insert(TransitionUnfreezeTimer(Timer::from_seconds(3.0, TimerMode::Once)));
}

/// Timer-based unfreeze until client confirmation is implemented.
#[derive(Component)]
pub struct TransitionUnfreezeTimer(pub Timer);

fn tick_transition_unfreeze(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut TransitionUnfreezeTimer)>,
) {
    for (entity, mut timer) in &mut query {
        timer.0.tick(time.delta());
        if timer.0.finished() {
            info!("Unfreezing player {entity:?} after transition timer");
            commands.entity(entity).remove::<(RigidBodyDisabled, DisableRollback, MapTransitioning, TransitionUnfreezeTimer)>();
        }
    }
}
```

#### 4. Client-side transition handler
**File**: `crates/client/src/map.rs`

```rust
fn handle_map_transition_start(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<MapTransitionStart>>,
    mut next_transition: ResMut<NextState<MapTransitionState>>,
    mut registry: ResMut<MapRegistry>,
    player_query: Query<Entity, (With<Predicted>, With<CharacterMarker>)>,
) {
    for mut receiver in &mut receivers {
        for (transition, _) in receiver.receive() {
            info!("Received MapTransitionStart for {:?}", transition.target);

            // Freeze local predicted player
            let player = player_query.get_single()
                .expect("Predicted player must exist when receiving MapTransitionStart");
            commands.entity(player).insert((RigidBodyDisabled, DisableRollback));

            // Spawn map instance if not in registry
            if !registry.0.contains_key(&transition.target) {
                let generator = generator_for_map(&transition.target);
                let map_entity = spawn_map_instance(
                    &mut commands,
                    &transition.target,
                    transition.seed,
                    transition.bounds,
                    generator,
                );
                registry.insert(transition.target.clone(), map_entity);
            }

            // Update ChunkTarget on player
            let map_entity = registry.get(&transition.target);
            commands.entity(player).insert(ChunkTarget::new(map_entity, 4));

            // Enter transitioning state
            next_transition.set(MapTransitionState::Transitioning);

            // Store pending transition target for completion check
            commands.insert_resource(PendingTransition(transition.target.clone()));
        }
    }
}

#[derive(Resource)]
pub struct PendingTransition(pub MapInstanceId);

fn generator_for_map(map_id: &MapInstanceId) -> VoxelGenerator {
    match map_id {
        MapInstanceId::Overworld => Arc::new(flat_terrain_voxels),
        MapInstanceId::Homebase { .. } => Arc::new(flat_terrain_voxels), // TODO: homebase generator
    }
}

fn spawn_map_instance(
    commands: &mut Commands,
    map_id: &MapInstanceId,
    seed: u64,
    bounds: Option<IVec3>,
    generator: VoxelGenerator,
) -> Entity {
    let tree_height = match map_id {
        MapInstanceId::Overworld => 5,
        MapInstanceId::Homebase { .. } => 3,
    };
    let spawning_distance = bounds.map(|b| b.max_element().max(1) as u32).unwrap_or(10);

    let entity = commands.spawn((
        VoxelMapInstance::new(tree_height),
        VoxelMapConfig::new(seed, spawning_distance, bounds, tree_height, generator),
        Transform::default(),
        map_id.clone(),
    )).id();

    info!("Spawned client map instance for {map_id:?}: {entity:?}");
    entity
}
```

#### 5. Chunk loading completion check
**File**: `crates/client/src/map.rs`

```rust
fn check_transition_chunks_loaded(
    mut commands: Commands,
    pending: Option<Res<PendingTransition>>,
    registry: Res<MapRegistry>,
    maps: Query<(&VoxelMapInstance, &PendingChunks)>,
    player_query: Query<Entity, (With<Predicted>, With<CharacterMarker>)>,
    mut next_transition: ResMut<NextState<MapTransitionState>>,
) {
    let Some(pending) = pending else { return };
    let map_entity = registry.get(&pending.0);
    let (map, pending_chunks) = maps.get(map_entity)
        .expect("Pending transition map must exist in ECS");

    // Loaded if: has some chunks AND no pending generation tasks
    if map.loaded_chunks.is_empty() || !pending_chunks.tasks.is_empty() || !pending_chunks.pending_positions.is_empty() {
        return; // Still loading
    }

    info!("Transition chunks loaded for {:?}, resuming play", pending.0);

    // Unfreeze player
    let player = player_query.get_single()
        .expect("Predicted player must exist when completing transition");
    commands.entity(player).remove::<(RigidBodyDisabled, DisableRollback)>();

    // Return to playing
    next_transition.set(MapTransitionState::Playing);
    commands.remove_resource::<PendingTransition>();
}
```

#### 6. Loading screen UI
**File**: `crates/ui/src/lib.rs`

```rust
fn setup_transition_loading_screen(mut commands: Commands) {
    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85)),
        DespawnOnExit(MapTransitionState::Transitioning),
    ))
    .with_children(|parent| {
        parent.spawn((
            Text::new("Loading..."),
            TextFont { font_size: 48.0, ..default() },
            TextColor(Color::WHITE),
        ));
    });
}
```

Register:
```rust
app.add_systems(OnEnter(MapTransitionState::Transitioning), setup_transition_loading_screen);
```

#### 7. System registration

**Server** (`crates/server/src/map.rs` plugin):
```rust
app.add_systems(Update, (handle_map_switch_requests, tick_transition_unfreeze));
```

**Client** (`crates/client/src/map.rs` plugin):
```rust
app.add_systems(Update, handle_map_transition_start);
app.add_systems(Update, check_transition_chunks_loaded.run_if(in_state(MapTransitionState::Transitioning)));
```

### Tests:

#### Integration test: map switch request → transition start roundtrip (`crates/server/tests/map_transition.rs`)

Uses `CrossbeamTestStepper` to verify the full message flow: client sends `PlayerMapSwitchRequest`, server processes it, client receives `MapTransitionStart`.

```rust
#[test]
fn map_switch_request_triggers_transition_start() {
    let mut stepper = CrossbeamTestStepper::new();

    // Add server map systems and resources
    stepper.server_app.init_resource::<MapRegistry>();
    stepper.server_app.init_resource::<RoomRegistry>();
    stepper.server_app.insert_resource(MapWorld::default());
    // Spawn overworld on server and register it
    let server_overworld = stepper.server_app.world_mut().spawn((
        VoxelMapInstance::new(5),
        VoxelMapConfig::new(999, 2, None, 5, Arc::new(flat_terrain_voxels)),
        Transform::default(),
        MapInstanceId::Overworld,
    )).id();
    stepper.server_app.world_mut().resource_mut::<MapRegistry>()
        .insert(MapInstanceId::Overworld, server_overworld);
    stepper.server_app.add_systems(Update, handle_map_switch_requests);
    stepper.server_app.add_systems(Update, tick_transition_unfreeze);

    // Add client message collection
    stepper.client_app.init_resource::<MessageBuffer<MapTransitionStart>>();
    stepper.client_app.add_systems(Update, collect_messages::<MapTransitionStart>);

    stepper.init();
    assert!(stepper.wait_for_connection());

    // Spawn a character entity on server owned by the test client
    let player = stepper.server_app.world_mut().spawn((
        CharacterMarker,
        MapInstanceId::Overworld,
        Position(Vec3::new(0.0, 30.0, 0.0)),
        ControlledBy { owner: stepper.client_of_entity, lifetime: Default::default() },
        ChunkTarget::new(server_overworld, 4),
    )).id();

    // Add player + client to overworld room
    let overworld_room = stepper.server_app.world_mut().spawn(Room::default()).id();
    stepper.server_app.world_mut().resource_mut::<RoomRegistry>()
        .0.insert(MapInstanceId::Overworld, overworld_room);
    stepper.server_app.world_mut().commands().trigger(RoomEvent {
        room: overworld_room, target: RoomTarget::AddEntity(player),
    });
    stepper.server_app.world_mut().commands().trigger(RoomEvent {
        room: overworld_room, target: RoomTarget::AddSender(stepper.client_of_entity),
    });

    stepper.tick_step(3);

    // Client sends map switch request
    stepper.client_app.world_mut()
        .entity_mut(stepper.client_entity)
        .get_mut::<MessageSender<PlayerMapSwitchRequest>>()
        .expect("Client should have MessageSender<PlayerMapSwitchRequest>")
        .send::<MapChannel>(&PlayerMapSwitchRequest { target: MapSwitchTarget::Homebase });

    // Tick to deliver request and process response
    stepper.tick_step(10);

    // Server should have:
    // 1. Inserted RigidBodyDisabled on player
    // 2. Sent MapTransitionStart to client
    let buffer = stepper.client_app.world().resource::<MessageBuffer<MapTransitionStart>>();
    assert_eq!(buffer.messages.len(), 1, "Client should receive MapTransitionStart");
    let transition = &buffer.messages[0].1;
    assert!(matches!(transition.target, MapInstanceId::Homebase { .. }));
    assert!(transition.bounds.is_some(), "Homebase should have bounds");
}

/// Verify server rejects duplicate transition request while already transitioning
#[test]
fn duplicate_switch_request_ignored() {
    // Similar setup to above, but send two requests in quick succession
    // Second request should be ignored (MapTransitioning marker prevents it)
    let mut stepper = CrossbeamTestStepper::new();
    // ... (setup as above)
    stepper.init();
    assert!(stepper.wait_for_connection());
    // ... spawn character, rooms, etc.

    // Send first request
    stepper.client_app.world_mut()
        .entity_mut(stepper.client_entity)
        .get_mut::<MessageSender<PlayerMapSwitchRequest>>()
        .unwrap()
        .send::<MapChannel>(&PlayerMapSwitchRequest { target: MapSwitchTarget::Homebase });

    stepper.tick_step(5);

    // Send second request (should be ignored)
    stepper.client_app.world_mut()
        .entity_mut(stepper.client_entity)
        .get_mut::<MessageSender<PlayerMapSwitchRequest>>()
        .unwrap()
        .send::<MapChannel>(&PlayerMapSwitchRequest { target: MapSwitchTarget::Overworld });

    stepper.tick_step(5);

    // Client should still have only 1 MapTransitionStart (the first one)
    let buffer = stepper.client_app.world().resource::<MessageBuffer<MapTransitionStart>>();
    assert_eq!(buffer.messages.len(), 1, "Second request should be ignored while transitioning");
}
```

#### Integration test: client chunk loading completion (`crates/client/tests/map_transition.rs`)

Tests the client-side transition state machine: receiving `MapTransitionStart` → entering `Transitioning` → chunks load → returning to `Playing`.

```rust
#[test]
fn client_transitions_to_playing_after_chunks_load() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(bevy::transform::TransformPlugin);
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.add_plugins(VoxelPlugin);
    app.init_resource::<MapRegistry>();
    // Initialize states
    app.insert_state(ClientState::InGame);
    app.add_sub_state::<MapTransitionState>();
    app.add_systems(Update, check_transition_chunks_loaded
        .run_if(in_state(MapTransitionState::Transitioning)));

    // Register a map in the registry
    let map = app.world_mut().spawn((
        VoxelMapInstance::new(5),
        VoxelMapConfig::new(0, 1, None, 5, Arc::new(flat_terrain_voxels)),
        Transform::default(),
        MapInstanceId::Overworld,
    )).id();
    app.world_mut().resource_mut::<MapRegistry>().insert(MapInstanceId::Overworld, map);

    // Spawn a fake predicted player
    let player = app.world_mut().spawn((
        CharacterMarker,
        Predicted,
        MapInstanceId::Overworld,
        RigidBodyDisabled,
        DisableRollback,
        ChunkTarget { map_entity: map, distance: 0 },
        Transform::default(),
    )).id();

    // Set pending transition and enter transitioning state
    app.insert_resource(PendingTransition(MapInstanceId::Overworld));
    app.world_mut().resource_mut::<NextState<MapTransitionState>>()
        .set(MapTransitionState::Transitioning);

    // Tick until chunks load
    for _ in 0..30 { app.update(); }

    // Should have transitioned back to Playing
    let state = app.world().resource::<State<MapTransitionState>>();
    assert_eq!(*state.get(), MapTransitionState::Playing,
        "Should return to Playing after chunks load");

    // PendingTransition resource should be removed
    assert!(app.world().get_resource::<PendingTransition>().is_none(),
        "PendingTransition should be cleaned up");

    // RigidBodyDisabled should be removed from player
    assert!(app.world().get::<RigidBodyDisabled>(player).is_none(),
        "Player should be unfrozen after transition completes");
}
```

### Success Criteria:

#### Automated Verification:
- [ ] All tests pass: `cargo test-native`
- [ ] `cargo check-all` passes
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client`

#### Manual Verification:
- [ ] (Deferred to Phase 5 — requires UI button to trigger)

---

## Phase 5: UI Toggle Button

### Overview
Add a "Homebase"/"Overworld" toggle button to the in-game HUD. Pressing it sends `PlayerMapSwitchRequest` to the server.

### Changes Required:

#### 1. MapSwitchButton marker
**File**: `crates/ui/src/components.rs`

```rust
/// Marker for the map switch toggle button in in-game HUD
#[derive(Component)]
pub struct MapSwitchButton;
```

#### 2. Add button to HUD
**File**: `crates/ui/src/lib.rs`

In `setup_ingame_hud`, add a third button before "Main Menu":

```rust
// Map Switch Button
parent
    .spawn((
        Button,
        Node {
            width: Val::Px(150.0),
            height: Val::Px(50.0),
            border: UiRect::all(Val::Px(3.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BorderColor::all(Color::WHITE),
        BackgroundColor(Color::srgba(0.2, 0.2, 0.2, 0.8)),
        MapSwitchButton,
    ))
    .with_children(|parent| {
        parent.spawn((
            Text::new("Homebase"),
            TextFont { font_size: 24.0, ..default() },
            TextColor(Color::WHITE),
        ));
    });
```

#### 3. Button interaction — send message
**File**: `crates/ui/src/lib.rs`

```rust
fn map_switch_button_interaction(
    switch_query: Query<&Interaction, (Changed<Interaction>, With<MapSwitchButton>)>,
    player_query: Query<&MapInstanceId, (With<Predicted>, With<CharacterMarker>)>,
    mut senders: Query<&mut MessageSender<PlayerMapSwitchRequest>>,
    transition_state: Res<State<MapTransitionState>>,
) {
    if *transition_state.get() == MapTransitionState::Transitioning {
        return; // Don't allow switching during transition
    }

    for interaction in &switch_query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let current_map = player_query.get_single()
            .expect("Predicted player must exist when pressing map switch button");

        let target = match current_map {
            MapInstanceId::Overworld => MapSwitchTarget::Homebase,
            MapInstanceId::Homebase { .. } => MapSwitchTarget::Overworld,
        };

        info!("Map switch button pressed, requesting {target:?}");
        for mut sender in &mut senders {
            sender.send::<MapChannel>(&PlayerMapSwitchRequest { target });
        }
    }
}
```

#### 4. Dynamic button label
**File**: `crates/ui/src/lib.rs`

```rust
fn update_map_switch_button_label(
    player_query: Query<&MapInstanceId, (With<Predicted>, With<CharacterMarker>)>,
    button_query: Query<&Children, With<MapSwitchButton>>,
    mut text_query: Query<&mut Text>,
) {
    let Ok(map_id) = player_query.get_single() else { return };
    let Ok(children) = button_query.get_single() else { return };

    let label = match map_id {
        MapInstanceId::Overworld => "Homebase",
        MapInstanceId::Homebase { .. } => "Overworld",
    };

    for &child in children.iter() {
        if let Ok(mut text) = text_query.get_mut(child) {
            text.0 = label.to_string();
        }
    }
}
```

#### 5. Register systems
**File**: `crates/ui/src/lib.rs`

```rust
app.add_systems(Update, (
    map_switch_button_interaction,
    update_map_switch_button_label,
).run_if(in_state(ClientState::InGame)));
```

### Tests:

#### Integration test: button spawns and label updates (`crates/ui/tests/ui_plugin.rs` — extend existing)

Following the existing UI test patterns (e.g. `test_ingame_state_spawns_hud`):

```rust
#[test]
fn ingame_hud_spawns_map_switch_button() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);

    app.world_mut().spawn((Name::new("Test Client"), Client::default()));

    // Transition to InGame
    app.world_mut().resource_mut::<NextState<ClientState>>().set(ClientState::InGame);
    app.update();

    let mut query = app.world_mut().query_filtered::<Entity, With<MapSwitchButton>>();
    assert_eq!(query.iter(app.world()).count(), 1, "Should have one MapSwitchButton");
}

#[test]
fn map_switch_button_label_shows_homebase_when_on_overworld() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);

    app.world_mut().spawn((Name::new("Test Client"), Client::default()));

    // Transition to InGame
    app.world_mut().resource_mut::<NextState<ClientState>>().set(ClientState::InGame);
    app.update();

    // Spawn a predicted player on Overworld
    app.world_mut().spawn((
        CharacterMarker,
        Predicted,
        MapInstanceId::Overworld,
    ));
    app.update(); // Run update_map_switch_button_label

    // Find button's child Text and verify label
    let button_entity = app.world_mut()
        .query_filtered::<Entity, With<MapSwitchButton>>()
        .single(app.world()).unwrap();
    let children = app.world().get::<Children>(button_entity).unwrap();
    let text = app.world().get::<Text>(children[0]).unwrap();
    assert_eq!(text.0, "Homebase", "Button should say 'Homebase' when player is on Overworld");
}

#[test]
fn map_switch_button_label_shows_overworld_when_on_homebase() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);

    app.world_mut().spawn((Name::new("Test Client"), Client::default()));
    app.world_mut().resource_mut::<NextState<ClientState>>().set(ClientState::InGame);
    app.update();

    // Spawn predicted player on Homebase
    app.world_mut().spawn((
        CharacterMarker,
        Predicted,
        MapInstanceId::Homebase { owner: 42 },
    ));
    app.update();

    let button_entity = app.world_mut()
        .query_filtered::<Entity, With<MapSwitchButton>>()
        .single(app.world()).unwrap();
    let children = app.world().get::<Children>(button_entity).unwrap();
    let text = app.world().get::<Text>(children[0]).unwrap();
    assert_eq!(text.0, "Overworld", "Button should say 'Overworld' when player is on Homebase");
}
```

### Success Criteria:

#### Automated Verification:
- [ ] All tests pass: `cargo test-native`
- [ ] `cargo check-all` passes
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client`

#### Manual Verification:
- [ ] "Homebase" button visible in top-right HUD
- [ ] Pressing button → loading screen appears → transitions to new map
- [ ] Button label changes to "Overworld" after arriving in Homebase
- [ ] Pressing "Overworld" → returns to Overworld with original terrain
- [ ] Button disabled/ignored during transition
- [ ] Two clients: switching one doesn't affect the other's view

---

## Phase 6: Homebase Map Spawning

### Overview
Implement lazy server-side homebase creation and the client-side generator. Fix `Homebase { owner }` to use `ClientId` instead of `Entity`.

### Changes Required:

#### 1. Fix Homebase marker to use ClientId
**File**: `crates/voxel_map_engine/src/instance.rs` (line 13-16)

Change:
```rust
pub struct Homebase {
    pub owner: Entity,
}
```
To:
```rust
pub struct Homebase {
    /// `lightyear` `ClientId` bits, raw `u64` because voxel_map_engine doesn't depend on lightyear
    /// `ClientId` bits because Entity is not network-stable
    pub owner: u64, 
}
```

Update `VoxelMapInstance::homebase()` signature to take `owner_id: u64` instead of `owner: Entity`. Update `seed_from_entity` to `seed_from_id(id: u64) -> u64`.

#### 2. Server: lazy homebase spawn
**File**: `crates/server/src/map.rs`

```rust
fn ensure_homebase_exists(
    commands: &mut Commands,
    owner: ClientId,
    registry: &mut MapRegistry,
    map_world: &MapWorld,
) -> Entity {
    let map_id = MapInstanceId::Homebase { owner };
    if let Some(&entity) = registry.0.get(&map_id) {
        return entity;
    }

    let owner_bits = owner.to_bits();
    let bounds = IVec3::new(4, 4, 4);
    let (instance, config, marker) = VoxelMapInstance::homebase(
        owner_bits,
        bounds,
        Arc::new(flat_terrain_voxels),
    );

    let entity = commands.spawn((
        instance,
        config,
        marker,
        Transform::default(),
        map_id.clone(),
    )).id();

    registry.insert(map_id, entity);
    info!("Spawned homebase for client {owner:?}: {entity:?}");
    entity
}
```

Integrate into `execute_server_transition` — when target is `Homebase`, call `ensure_homebase_exists` before looking up in registry.

#### 3. Update existing homebase tests
**File**: `crates/voxel_map_engine/src/instance.rs` (tests)

Update `homebase_bundle_has_correct_config` to use `u64` instead of `Entity`.

### Tests:

```rust
#[test]
fn homebase_seed_deterministic() {
    let id: u64 = 12345;
    let (_, config1, _) = VoxelMapInstance::homebase(id, IVec3::new(4, 4, 4), dummy_generator());
    let (_, config2, _) = VoxelMapInstance::homebase(id, IVec3::new(4, 4, 4), dummy_generator());
    assert_eq!(config1.seed, config2.seed);
}

#[test]
fn different_owners_different_seeds() {
    let (_, config1, _) = VoxelMapInstance::homebase(1, IVec3::new(4, 4, 4), dummy_generator());
    let (_, config2, _) = VoxelMapInstance::homebase(2, IVec3::new(4, 4, 4), dummy_generator());
    assert_ne!(config1.seed, config2.seed);
}
```

#### Integration test: server and client produce identical maps from MapTransitionStart data (`crates/server/tests/map_transition.rs`)

Verifies that when the server sends `MapTransitionStart` with specific config, the client spawning logic produces a map with matching seed and bounds — ensuring deterministic terrain generation.

```rust
#[test]
fn client_spawns_matching_map_from_transition_data() {
    // Simulate what the server sends
    let transition = MapTransitionStart {
        target: MapInstanceId::Homebase { owner: 12345 },
        seed: 12345,
        generation_version: 0,
        bounds: Some(IVec3::new(4, 4, 4)),
    };

    // Client-side: spawn map instance from transition data (same logic as handle_map_transition_start)
    let mut client_app = App::new();
    client_app.add_plugins(MinimalPlugins);
    client_app.add_plugins(bevy::transform::TransformPlugin);
    client_app.init_resource::<Assets<Mesh>>();
    client_app.init_resource::<Assets<StandardMaterial>>();
    client_app.add_plugins(VoxelPlugin);

    let client_map = spawn_map_instance(
        &mut client_app.world_mut().commands(),
        &transition.target,
        transition.seed,
        transition.bounds,
        Arc::new(flat_terrain_voxels),
    );
    client_app.update();

    // Server-side: spawn homebase with same parameters
    let mut server_app = App::new();
    server_app.add_plugins(MinimalPlugins);
    server_app.add_plugins(bevy::transform::TransformPlugin);
    server_app.init_resource::<Assets<Mesh>>();
    server_app.init_resource::<Assets<StandardMaterial>>();
    server_app.add_plugins(VoxelPlugin);

    let (instance, config, _marker) = VoxelMapInstance::homebase(
        12345, // owner_id bits
        IVec3::new(4, 4, 4),
        Arc::new(flat_terrain_voxels),
    );
    let server_map = server_app.world_mut().spawn((instance, config, Transform::default())).id();
    server_app.update();

    // Both should have identical config
    let client_config = client_app.world().get::<VoxelMapConfig>(client_map).unwrap();
    let server_config = server_app.world().get::<VoxelMapConfig>(server_map).unwrap();

    assert_eq!(client_config.seed, server_config.seed, "Seeds must match");
    assert_eq!(client_config.bounds, server_config.bounds, "Bounds must match");
    assert_eq!(client_config.tree_height, server_config.tree_height, "Tree heights must match");
}

/// Verify that two maps spawned with different owner IDs produce different terrain seeds
#[test]
fn different_homebase_owners_produce_different_seeds() {
    let (_, config_a, _) = VoxelMapInstance::homebase(111, IVec3::new(4, 4, 4), dummy_generator());
    let (_, config_b, _) = VoxelMapInstance::homebase(222, IVec3::new(4, 4, 4), dummy_generator());
    assert_ne!(config_a.seed, config_b.seed, "Different owners must produce different seeds");
}
```

### Success Criteria:

#### Automated Verification:
- [ ] All tests pass: `cargo test-native`
- [ ] `cargo check-all` passes
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client`

#### Manual Verification:
- [ ] First homebase visit creates a new map with terrain
- [ ] Second visit reuses the same map (chunks match)
- [ ] Different clients get different homebases
- [ ] Full round-trip: Overworld → Homebase → Overworld works without crashes

---

## Testing Strategy

### Unit Tests (`#[cfg(test)]` modules):
- `MapInstanceId` equality and hashing (`crates/protocol/src/map.rs`)
- `MapRegistry` lookup — hit, miss/panic (`crates/protocol/src/map.rs`)
- Homebase seed determinism and uniqueness (`crates/voxel_map_engine/src/instance.rs`)

### Integration Tests (test files using real Bevy Apps):

| Test File | What It Tests | Infrastructure |
|-----------|--------------|----------------|
| `crates/protocol/tests/physics_isolation.rs` | `MapCollisionHooks` filter — same map collides, different map doesn't, missing ID falls through. Chunk colliders inherit parent's `MapInstanceId`. | `PhysicsPlugins` + `MapCollisionHooks` + `VoxelPlugin` |
| `crates/voxel_map_engine/tests/lifecycle.rs` (extended) | `ChunkTarget` derived from `MapRegistry`, player entity drives chunk loading/unloading | `VoxelPlugin` + `MapRegistry` |
| `crates/server/tests/rooms.rs` | Entities in different lightyear rooms not replicated to wrong clients. Same-frame room transfer preserves visibility. | `CrossbeamTestStepper` + `Room` |
| `crates/server/tests/map_transition.rs` | `PlayerMapSwitchRequest` → server processes → client receives `MapTransitionStart`. Duplicate request rejected during transition. Server and client produce identical map configs. | `CrossbeamTestStepper` + server map systems |
| `crates/client/tests/map_transition.rs` | Client enters `Transitioning` state → chunks load → returns to `Playing`. `PendingTransition` cleaned up. `RigidBodyDisabled` removed. | `VoxelPlugin` + `StatesPlugin` + `MapTransitionState` |
| `crates/ui/tests/ui_plugin.rs` (extended) | `MapSwitchButton` spawns in HUD. Label shows "Homebase" on Overworld, "Overworld" on Homebase. | `UiPlugin` + `StatesPlugin` + mock player entity |

### Manual Testing Steps:
1. Start server + 1 client. Verify Overworld works normally (movement, jumping, abilities).
2. Start server + 2 clients. Verify both see each other.
3. Client A presses "Homebase". Verify: loading screen → homebase terrain appears → button says "Overworld".
4. Client B still sees Overworld. Client B does NOT see Client A.
5. Client A presses "Overworld". Verify: loading screen → back in Overworld → sees Client B again.
6. Both clients switch to Homebase simultaneously. Verify: each gets their own homebase, no cross-visibility.
7. Rapid button mashing during transition. Verify: no crash, request ignored.

### Panic Points (Debug Aid):
Every `expect()` message in the plan describes the invariant being violated. These should fire immediately if:
- `MapRegistry` lookup fails (map not registered)
- `RoomRegistry` lookup fails
- Character entity not found for a connected client
- Predicted player entity missing during transition
- Map entity missing `PendingChunks` component
- Client entity missing `MessageSender`

## Performance Considerations

- `filter_pairs` is called per broad-phase pair — with one map, zero overhead (all entities match). With N maps, cost scales with cross-map AABB overlaps, which is bounded by the same spatial extent.
- `cast_ray_predicate` closure adds one `MapInstanceId` lookup per ray hit candidate. Negligible for short-range ground detection rays.
- `ActiveCollisionHooks::FILTER_PAIRS` is opt-in per entity — entities without it skip hook evaluation entirely.

## References

- Research: [doc/research/2026-03-07-map-instance-physics-isolation-and-switching.md](doc/research/2026-03-07-map-instance-physics-isolation-and-switching.md)
- Physics plugins: [crates/protocol/src/lib.rs:242-248](crates/protocol/src/lib.rs#L242-L248)
- Collision layers: [crates/protocol/src/hit_detection.rs:17-53](crates/protocol/src/hit_detection.rs#L17-L53)
- apply_movement raycast: [crates/protocol/src/lib.rs:310-321](crates/protocol/src/lib.rs#L310-L321)
- Character spawn (server): [crates/server/src/gameplay.rs:160-208](crates/server/src/gameplay.rs#L160-L208)
- Character spawn (client): [crates/client/src/gameplay.rs:16-53](crates/client/src/gameplay.rs#L16-L53)
- Chunk colliders: [crates/protocol/src/map.rs:46-72](crates/protocol/src/map.rs#L46-L72)
- ChunkTarget: [crates/voxel_map_engine/src/chunk.rs:13-36](crates/voxel_map_engine/src/chunk.rs#L13-L36)
- VoxelMapInstance: [crates/voxel_map_engine/src/instance.rs:8-96](crates/voxel_map_engine/src/instance.rs#L8-L96)
- Client overworld + ChunkTarget: [crates/client/src/map.rs:45-66](crates/client/src/map.rs#L45-L66)
- HUD buttons: [crates/ui/src/lib.rs:312-425](crates/ui/src/lib.rs#L312-L425)
- UI components: [crates/ui/src/components.rs](crates/ui/src/components.rs)
- ClientState: [crates/ui/src/state.rs:4-13](crates/ui/src/state.rs#L4-L13)
