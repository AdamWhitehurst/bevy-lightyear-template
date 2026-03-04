# Physics Isolation & Map Transitions Implementation Plan

## Overview

Implement physics isolation between map instances (Overworld, Homebase, Arena) that share a single Avian physics world, plus the full map transition system: client/server messaging, loading states, UI, and Lightyear room-based visibility scoping.

Based on: [doc/research/2026-03-03-physics-isolation-avian-collision-hooks.md](../research/2026-03-03-physics-isolation-avian-collision-hooks.md)

## Current State Analysis

- **Single physics world**: All entities share one Avian world. Isolation is type-based only via `CollisionLayers` ([hit_detection.rs:17-53](../../crates/protocol/src/hit_detection.rs#L17-L53)).
- **No CollisionHooks**: `PhysicsPlugins::default()` with no hooks ([lib.rs:242-248](../../crates/protocol/src/lib.rs#L242-L248)).
- **`apply_movement` raycast**: Uses `SpatialQuery::cast_ray` with only self-exclusion — will hit terrain from any map ([lib.rs:310-321](../../crates/protocol/src/lib.rs#L310-L321)).
- **Entity-map association exists**: `ChunkTarget.map_entity` on players, `ChildOf` on chunks pointing to map entity. But no unified `MapInstanceId`.
- **One message channel**: `VoxelChannel` (ordered reliable bidirectional) with `VoxelEditRequest`/`VoxelEditBroadcast`/`VoxelStateSync` ([map.rs:7-43](../../crates/protocol/src/map.rs#L7-L43)).
- **No SubStates, rooms, or transition logic**.
- **`OverworldMap` resource**: Defined separately in server and client crates (not shared via protocol).

### Key Discoveries:
- Chunks are children of their map entity via `ChildOf` ([lifecycle.rs:224](../../crates/voxel_map_engine/src/lifecycle.rs#L224))
- `CollisionHooks::filter_pairs` requires `ReadOnlySystemParam` — no mutable queries ([hooks.rs](../../git/avian/src/collision/hooks.rs))
- `filter_pairs` does NOT affect `SpatialQuery` — separate solution needed for raycasts
- Only one `CollisionHooks` impl per app — future hooks (one-way platforms) must share the same struct
- `ActiveCollisionHooks::FILTER_PAIRS` is opt-in per entity — only tagged entities pay hook cost
- `RoomPlugin` must be explicitly added; entities without `NetworkVisibility` are visible to all clients
- Room shared-count mechanism prevents visibility flicker during same-frame room moves ([room.rs:254-267](../../git/lightyear/lightyear_replication/src/visibility/room.rs#L254-L267))
- Child entities inherit visibility from parent's `NetworkVisibility` via hierarchy fallback ([buffer.rs:202-204](../../git/lightyear/lightyear_replication/src/buffer.rs#L202-L204))

## Desired End State

- Characters, hitboxes, projectiles, and terrain in one map instance cannot collide with or raycast-detect entities in another map instance.
- Players can switch between Overworld and Homebase via an in-game button.
- During map transitions, the player is physics-paused and sees a loading state until the new map's chunks are ready.
- Lightyear rooms scope entity replication so clients only receive entities from their current map.
- The system supports server-initiated transitions (portals, game events) using the same `initiate_map_transition` function.

### Verification:
1. Run server + 2 clients. Client A on Overworld, Client B on Homebase. Neither sees or collides with the other's terrain or entities.
2. Client A presses "Homebase" button → loading screen → arrives in Homebase with chunks loaded → button now shows "Overworld".
3. Pressing "Overworld" returns to Overworld. Transition is smooth with no physics glitches.

## What We're NOT Doing

- Arena map instances (Homebase and Overworld only for now)
- One-way platforms or other `modify_contacts` hooks (future extension to same `MapCollisionHooks` struct)
- Per-map spawn points (fixed spawn at `(0, 30, 0)` for all maps)
- Voxel persistence for Homebases (chunks regenerated from seed each time)
- Client-to-server "chunks loaded" confirmation (server uses timeout or fire-and-forget for now)
- Keyboard shortcut for map switching (button only)

---

## Phase 1: MapInstanceId Component + CollisionHooks

### Overview
Define the `MapInstanceId` component and `MapCollisionHooks` SystemParam. Register hooks with Avian. Insert `MapInstanceId` on all physics entities to enable cross-map collision filtering.

### Changes Required:

#### 1. MapInstanceId Component
**File**: `crates/protocol/src/map.rs`
**Changes**: Add `MapInstanceId` component with required `ActiveCollisionHooks`

```rust
use avian3d::collision::hooks::ActiveCollisionHooks;

/// Identifies which map instance a physics entity belongs to.
/// The inner Entity points to the VoxelMapInstance entity.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
#[require(ActiveCollisionHooks::FILTER_PAIRS)]
pub struct MapInstanceId(pub Entity);

impl MapEntities for MapInstanceId {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.0 = entity_mapper.get_mapped(self.0);
    }
}
```

#### 2. MapCollisionHooks SystemParam
**File**: `crates/protocol/src/physics.rs` (new file)
**Changes**: Implement `CollisionHooks` trait

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use crate::map::MapInstanceId;

/// Physics collision hooks for map instance isolation.
/// Only one CollisionHooks impl is allowed per app — future hooks
/// (one-way platforms, conveyors) must be added to this struct.
#[derive(SystemParam)]
pub struct MapCollisionHooks<'w, 's> {
    map_ids: Query<'w, 's, &'static MapInstanceId>,
}

impl CollisionHooks for MapCollisionHooks<'_, '_> {
    fn filter_pairs(&self, entity1: Entity, entity2: Entity, _commands: &mut Commands) -> bool {
        match (self.map_ids.get(entity1), self.map_ids.get(entity2)) {
            (Ok(a), Ok(b)) => a.0 == b.0,
            _ => true, // allow collision if either lacks MapInstanceId
        }
    }
}
```

#### 3. Register Hooks on PhysicsPlugins
**File**: `crates/protocol/src/lib.rs`
**Changes**: Replace `PhysicsPlugins::default()` with hooks-enabled variant

```rust
// Before:
PhysicsPlugins::default()
    .build()
    .disable::<PhysicsTransformPlugin>()
    // ...

// After:
PhysicsPlugins::default()
    .with_collision_hooks::<MapCollisionHooks>()
    .build()
    .disable::<PhysicsTransformPlugin>()
    // ...
```

#### 4. Register MapInstanceId for Replication
**File**: `crates/protocol/src/lib.rs`
**Changes**: In `ProtocolPlugin::build`, register `MapInstanceId`

```rust
app.register_component::<MapInstanceId>()
    .add_prediction()
    .add_map_entities();
```

#### 5. Insert MapInstanceId on Character Spawn (Server)
**File**: `crates/server/src/gameplay.rs`
**Changes**: Add `MapInstanceId` to character spawn bundle in `handle_connected` and `spawn_dummy_target`

In `handle_connected` (line 189), add to spawn tuple:
```rust
MapInstanceId(overworld.0),
```

In `spawn_dummy_target` (line 61), add to spawn tuple:
```rust
MapInstanceId(overworld.0),
```

#### 6. Insert MapInstanceId on Terrain Chunks
**File**: `crates/protocol/src/map.rs`
**Changes**: Modify `attach_chunk_colliders` to also insert `MapInstanceId` derived from chunk's parent

```rust
pub fn attach_chunk_colliders(
    mut commands: Commands,
    chunks: Query<
        (Entity, &Mesh3d, &ChildOf, Option<&Collider>),
        (With<VoxelChunk>, Or<(Changed<Mesh3d>, Added<Mesh3d>)>),
    >,
    meshes: Res<Assets<Mesh>>,
) {
    for (entity, mesh_handle, child_of, existing_collider) in chunks.iter() {
        // ... existing collider creation logic ...
        commands.entity(entity).insert((
            collider,
            RigidBody::Static,
            crate::hit_detection::terrain_collision_layers(),
            MapInstanceId(child_of.parent()),
        ));
    }
}
```

#### 7. Insert MapInstanceId on Hitboxes and Projectiles
**File**: `crates/protocol/src/ability.rs`
**Changes**: Pass caster's `MapInstanceId` through to hitbox/projectile spawn functions

In `spawn_melee_hitbox` (~line 1097): add `MapInstanceId` parameter, insert on spawned entity.
In `spawn_aoe_hitbox` (~line 1138): same.
In `ability_projectile_spawn` (~line 1391): copy caster's `MapInstanceId` to `AbilityProjectileSpawn`.
In `handle_ability_projectile_spawn` (~line 1450): copy to bullet child entity.

The `apply_on_tick_effects` system that calls these functions needs a `MapInstanceId` query on the caster. Thread it through from the caster entity.

#### 8. Module Registration
**File**: `crates/protocol/src/lib.rs`
**Changes**: Add `pub mod physics;` and import `MapCollisionHooks`

#### 9. Tests
**File**: `crates/protocol/tests/physics_isolation.rs` (new file)
**Changes**: Unit tests for `MapCollisionHooks::filter_pairs` and `MapInstanceId` insertion

Uses the existing test pattern: `MinimalPlugins` app, spawn entities with components as plain data, verify behavior via direct queries.

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use protocol::map::MapInstanceId;
use protocol::physics::MapCollisionHooks;

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    // No PhysicsPlugins needed: these tests call filter_pairs directly as a
    // trait method on the SystemParam, and #[require] works at the ECS level.
    app
}

/// filter_pairs returns true when both entities share the same MapInstanceId.
#[test]
fn filter_pairs_same_map_allows_collision() {
    let mut app = test_app();
    let map = app.world_mut().spawn_empty().id();
    let a = app.world_mut().spawn(MapInstanceId(map)).id();
    let b = app.world_mut().spawn(MapInstanceId(map)).id();

    app.world_mut().run_system_once(move |hooks: MapCollisionHooks, mut commands: Commands| {
        assert!(hooks.filter_pairs(a, b, &mut commands));
    }).unwrap();
}

/// filter_pairs returns false when entities have different MapInstanceIds.
#[test]
fn filter_pairs_different_map_blocks_collision() {
    let mut app = test_app();
    let map_a = app.world_mut().spawn_empty().id();
    let map_b = app.world_mut().spawn_empty().id();
    let a = app.world_mut().spawn(MapInstanceId(map_a)).id();
    let b = app.world_mut().spawn(MapInstanceId(map_b)).id();

    app.world_mut().run_system_once(move |hooks: MapCollisionHooks, mut commands: Commands| {
        assert!(!hooks.filter_pairs(a, b, &mut commands));
    }).unwrap();
}

/// filter_pairs returns true when one or both entities lack MapInstanceId.
#[test]
fn filter_pairs_missing_map_allows_collision() {
    let mut app = test_app();
    let map = app.world_mut().spawn_empty().id();
    let a = app.world_mut().spawn(MapInstanceId(map)).id();
    let b = app.world_mut().spawn_empty().id(); // no MapInstanceId

    app.world_mut().run_system_once(move |hooks: MapCollisionHooks, mut commands: Commands| {
        assert!(hooks.filter_pairs(a, b, &mut commands));
    }).unwrap();
}

/// MapInstanceId's #[require] automatically inserts ActiveCollisionHooks::FILTER_PAIRS.
#[test]
fn map_instance_id_requires_active_collision_hooks() {
    let mut app = test_app();
    let map = app.world_mut().spawn_empty().id();
    let entity = app.world_mut().spawn(MapInstanceId(map)).id();
    app.update();

    assert!(app.world().get::<ActiveCollisionHooks>(entity).is_some());
}
```

**File**: `crates/server/tests/integration.rs` (extend existing)
**Changes**: Add test verifying character spawns with `MapInstanceId`

```rust
/// Character entity spawned on connect includes MapInstanceId matching the overworld.
#[test]
fn test_character_has_map_instance_id() {
    let mut stepper = CrossbeamTestStepper::new();
    // ... register SharedGameplayPlugin or individual systems ...
    stepper.init();
    stepper.wait_for_connection();
    stepper.tick_step(5);

    // Query character entity on server, verify MapInstanceId == overworld entity
    let (_, map_id) = stepper.server_app.world_mut()
        .query_filtered::<(Entity, &MapInstanceId), With<CharacterMarker>>()
        .iter(stepper.server_app.world())
        .next()
        .expect("Character should exist with MapInstanceId");

    let overworld = stepper.server_app.world().resource::<OverworldMap>();
    assert_eq!(map_id.0, overworld.0);
}
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` compiles
- [x] `cargo test --workspace` passes (including new tests above)
- [x] `cargo server` starts without panic
- [x] `cargo client` connects without panic

#### Manual Verification:
- [ ] With one map instance, gameplay is unchanged — characters collide with terrain and each other normally
- [ ] Adding a `warn!` log in `filter_pairs` confirms it is being called for character-terrain pairs

---

## Phase 2: SpatialQuery Filtering in apply_movement

### Overview
Fix the ground-detection raycast in `apply_movement` to only detect terrain from the same map instance. Uses `cast_ray_predicate` instead of `cast_ray`.

### Changes Required:

#### 1. Add MapInstanceId Query to apply_movement
**File**: `crates/protocol/src/lib.rs`
**Changes**: Modify `apply_movement` signature to accept map ID, use `cast_ray_predicate`

```rust
pub fn apply_movement(
    entity: Entity,
    mass: &ComputedMass,
    delta_secs: f32,
    spatial_query: &SpatialQuery,
    action_state: &ActionState<PlayerActions>,
    position: &Position,
    forces: &mut ForcesItem,
    map_id: Option<&MapInstanceId>,
    map_ids: &Query<&MapInstanceId>,
) {
    // ...
    if action_state.just_pressed(&PlayerActions::Jump) {
        let ray_cast_origin = position.0;
        let filter = SpatialQueryFilter::from_excluded_entities([entity]);
        let hit = spatial_query.cast_ray_predicate(
            ray_cast_origin,
            Dir3::NEG_Y,
            4.0,
            false,
            &filter,
            &|hit_entity| match (map_id, map_ids.get(hit_entity).ok()) {
                (Some(a), Some(b)) => a.0 == b.0,
                _ => true,
            },
        );
        if hit.is_some() {
            forces.apply_linear_impulse(Vec3::new(0.0, 400.0, 0.0));
        }
    }
    // ... rest unchanged
}
```

#### 2. Update Callers
**File**: `crates/server/src/gameplay.rs` — `handle_character_movement`
**File**: `crates/client/src/gameplay.rs` — `handle_character_movement`
**Changes**: Add `MapInstanceId` query, pass to `apply_movement`

```rust
fn handle_character_movement(
    time: Res<Time>,
    spatial_query: SpatialQuery,
    map_ids: Query<&MapInstanceId>,
    mut query: Query<
        (Entity, &ActionState<PlayerActions>, &ComputedMass, &Position, Forces, Option<&MapInstanceId>),
        With<CharacterMarker>,
    >,
) {
    for (entity, action_state, mass, position, mut forces, map_id) in &mut query {
        apply_movement(
            entity, mass, time.delta_secs(), &spatial_query,
            action_state, position, &mut forces, map_id, &map_ids,
        );
    }
}
```

#### 3. Tests
**File**: `crates/protocol/tests/physics_isolation.rs` (extend)
**Changes**: Integration test for map-aware ground detection using `PhysicsPlugins` with hooks

This test requires actual physics simulation to verify `SpatialQuery::cast_ray_predicate` behavior. It spawns two static colliders (terrain stand-ins) at the same position with different `MapInstanceId`s, and verifies the raycast only hits same-map terrain.

```rust
use avian3d::prelude::*;

/// SpatialQuery raycast only detects terrain with matching MapInstanceId.
#[test]
fn raycast_ignores_different_map_terrain() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(
        PhysicsPlugins::default()
            .with_collision_hooks::<MapCollisionHooks>()
            .build()
    );
    app.update(); // initialize physics world

    let map_a = app.world_mut().spawn_empty().id();
    let map_b = app.world_mut().spawn_empty().id();

    // Terrain block at y=0 belonging to map_a
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(10.0, 1.0, 10.0),
        Position(Vec3::new(0.0, 0.0, 0.0)),
        MapInstanceId(map_a),
        terrain_collision_layers(),
    ));

    // Terrain block at y=0 belonging to map_b
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(10.0, 1.0, 10.0),
        Position(Vec3::new(0.0, 0.0, 0.0)),
        MapInstanceId(map_b),
        terrain_collision_layers(),
    ));

    // Step physics to build spatial index
    for _ in 0..3 { app.update(); }

    // Raycast from above, filtering for map_a only
    app.world_mut().run_system_once(move |
        spatial_query: SpatialQuery,
        map_ids: Query<&MapInstanceId>,
    | {
        let origin = Vec3::new(0.0, 5.0, 0.0);
        let filter = SpatialQueryFilter::default();

        // With map_a predicate: should hit
        let hit_a = spatial_query.cast_ray_predicate(
            origin, Dir3::NEG_Y, 10.0, false, &filter,
            &|hit_entity| match map_ids.get(hit_entity).ok() {
                Some(id) => id.0 == map_a,
                None => true,
            },
        );
        assert!(hit_a.is_some(), "Should hit map_a terrain");

        // With map_b predicate: should also hit (different terrain)
        let hit_b = spatial_query.cast_ray_predicate(
            origin, Dir3::NEG_Y, 10.0, false, &filter,
            &|hit_entity| match map_ids.get(hit_entity).ok() {
                Some(id) => id.0 == map_b,
                None => true,
            },
        );
        assert!(hit_b.is_some(), "Should hit map_b terrain");

        // With nonexistent map predicate: should miss both
        let map_c = Entity::from_raw(9999);
        let hit_c = spatial_query.cast_ray_predicate(
            origin, Dir3::NEG_Y, 10.0, false, &filter,
            &|hit_entity| match map_ids.get(hit_entity).ok() {
                Some(id) => id.0 == map_c,
                None => true,
            },
        );
        assert!(hit_c.is_none(), "Should not hit terrain from nonexistent map");
    }).unwrap();
}
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` compiles
- [x] `cargo test --workspace` passes (including raycast isolation test)

#### Manual Verification:
- [ ] Character still jumps normally on terrain (single map)
- [ ] Ground detection works correctly near chunk boundaries

---

## Phase 3: MapChannel + Messages

### Overview
Define the network channel and message types for map switching. Follows existing `VoxelChannel`/`VoxelEditRequest` pattern.

### Changes Required:

#### 1. Message and Channel Types
**File**: `crates/protocol/src/map.rs`
**Changes**: Add `MapChannel`, `MapSwitchTarget`, `PlayerMapSwitchRequest`, `MapTransitionStart`

```rust
/// Channel for map transition messages
pub struct MapChannel;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
pub enum MapSwitchTarget {
    Overworld,
    Homebase,
}

/// Client requests to switch maps
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct PlayerMapSwitchRequest {
    pub target: MapSwitchTarget,
}

/// Server notifies client that a map transition is starting
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct MapTransitionStart {
    pub target: MapSwitchTarget,
}
```

#### 2. Channel and Message Registration
**File**: `crates/protocol/src/lib.rs`
**Changes**: In `ProtocolPlugin::build`, after voxel registrations

```rust
// Map transition channel
app.add_channel::<MapChannel>(ChannelSettings {
    mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
    ..default()
})
.add_direction(NetworkDirection::Bidirectional);

// Map transition messages
app.register_message::<PlayerMapSwitchRequest>()
    .add_direction(NetworkDirection::ClientToServer);
app.register_message::<MapTransitionStart>()
    .add_direction(NetworkDirection::ServerToClient);
```

#### 3. Tests
**File**: `crates/server/tests/integration.rs` (extend)
**Changes**: Test `PlayerMapSwitchRequest` round-trip using `CrossbeamTestStepper`

Uses the existing `CrossbeamTestStepper` + `MessageBuffer` pattern from integration.rs.

```rust
/// PlayerMapSwitchRequest sent from client is received by server.
#[test]
fn test_map_switch_request_round_trip() {
    let mut stepper = CrossbeamTestStepper::new();
    stepper.server_app.init_resource::<MessageBuffer<PlayerMapSwitchRequest>>();
    stepper.server_app.add_systems(Update, collect_messages::<PlayerMapSwitchRequest>);
    stepper.init();
    assert!(stepper.wait_for_connection());

    let request = PlayerMapSwitchRequest { target: MapSwitchTarget::Homebase };
    stepper.client_app.world_mut()
        .entity_mut(stepper.client_entity)
        .get_mut::<MessageSender<PlayerMapSwitchRequest>>()
        .expect("Client should have MessageSender")
        .send::<MapChannel>(request.clone());

    stepper.tick_step(5);

    let buffer = stepper.server_app.world().resource::<MessageBuffer<PlayerMapSwitchRequest>>();
    assert_eq!(buffer.messages.len(), 1);
    assert_eq!(buffer.messages[0].1, request);
}

/// MapTransitionStart sent from server is received by client.
#[test]
fn test_map_transition_start_round_trip() {
    let mut stepper = CrossbeamTestStepper::new();
    stepper.client_app.init_resource::<MessageBuffer<MapTransitionStart>>();
    stepper.client_app.add_systems(Update, collect_messages::<MapTransitionStart>);
    stepper.init();
    assert!(stepper.wait_for_connection());

    let msg = MapTransitionStart { target: MapSwitchTarget::Homebase };
    stepper.server_app.world_mut()
        .entity_mut(stepper.client_of_entity)
        .get_mut::<MessageSender<MapTransitionStart>>()
        .expect("ClientOf should have MessageSender")
        .send::<MapChannel>(msg.clone());

    stepper.tick_step(5);

    let buffer = stepper.client_app.world().resource::<MessageBuffer<MapTransitionStart>>();
    assert_eq!(buffer.messages.len(), 1);
    assert_eq!(buffer.messages[0].1, msg);
}
```

**File**: `crates/protocol/src/test_utils.rs` (extend)
**Changes**: Add assertion helpers for new channel and message types

```rust
pub fn assert_map_channel_registered(app: &App) {
    assert_channel_registered::<MapChannel>(app);
}

pub fn assert_map_messages_registered(app: &App) {
    assert_message_registered::<PlayerMapSwitchRequest>(app);
    assert_message_registered::<MapTransitionStart>(app);
}
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` compiles
- [x] `cargo test --workspace` passes (including message round-trip tests)

---

## Phase 4: Server Map Transition Handler

### Overview
Server receives `PlayerMapSwitchRequest`, resolves the target map (spawning Homebase on demand), executes the transition (update components, pause physics), and notifies the client.

### Changes Required:

#### 1. Move OverworldMap to Protocol
**File**: `crates/protocol/src/map.rs`
**Changes**: Define `OverworldMap` in protocol so both server and client share the type

```rust
/// Resource tracking the primary overworld map entity.
#[derive(Resource)]
pub struct OverworldMap(pub Entity);
```

**File**: `crates/server/src/map.rs` — Remove local `OverworldMap` definition, import from protocol
**File**: `crates/client/src/map.rs` — Remove local `OverworldMap` definition, import from protocol

#### 2. Server Map Transition Systems
**File**: `crates/server/src/map_transition.rs` (new file)
**Changes**: Implement request handler, homebase spawn, transition execution

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::prelude::server::ClientOf;
use protocol::map::*;
use protocol::*;
use voxel_map_engine::prelude::*;

pub struct ServerMapTransitionPlugin;

impl Plugin for ServerMapTransitionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, handle_map_switch_requests);
    }
}

fn handle_map_switch_requests(
    mut commands: Commands,
    mut receiver: Query<(Entity, &mut MessageReceiver<PlayerMapSwitchRequest>), With<ClientOf>>,
    mut sender: Query<&mut MessageSender<MapTransitionStart>, With<ClientOf>>,
    overworld: Res<OverworldMap>,
    homebases: Query<(Entity, &Homebase)>,
    players: Query<(Entity, &PlayerId, &ControlledBy), With<CharacterMarker>>,
) {
    for (client_entity, mut message_receiver) in receiver.iter_mut() {
        for request in message_receiver.receive() {
            let Some((player_entity, _, _)) = players.iter()
                .find(|(_, _, ctrl)| ctrl.owner == client_entity)
            else {
                warn!("Map switch request from client {client_entity:?} but no owned character found");
                continue;
            };

            let target_map = match request.target {
                MapSwitchTarget::Overworld => overworld.0,
                MapSwitchTarget::Homebase => {
                    find_or_spawn_homebase(&mut commands, player_entity, &homebases)
                }
            };

            initiate_map_transition(
                &mut commands,
                client_entity,
                player_entity,
                target_map,
            );

            if let Ok(mut msg_sender) = sender.get_mut(client_entity) {
                msg_sender.send::<MapChannel>(MapTransitionStart {
                    target: request.target,
                });
            }
        }
    }
}

fn find_or_spawn_homebase(
    commands: &mut Commands,
    player_entity: Entity,
    homebases: &Query<(Entity, &Homebase)>,
) -> Entity {
    if let Some((map_entity, _)) = homebases.iter()
        .find(|(_, hb)| hb.owner == player_entity)
    {
        return map_entity;
    }

    let (instance, config, marker) = VoxelMapInstance::homebase(
        player_entity,
        IVec3::new(8, 4, 8),
        Arc::new(flat_terrain_voxels),
    );
    commands.spawn((
        instance,
        config,
        marker,
        Transform::default(),
    )).id()
}

/// Execute map transition: pause physics, update map association, teleport.
/// Used by both client-requested and server-initiated transitions.
pub fn initiate_map_transition(
    commands: &mut Commands,
    _client_entity: Entity,
    player_entity: Entity,
    target_map: Entity,
) {
    commands.entity(player_entity).insert((
        RigidBodyDisabled,
        DisableRollback,
        MapInstanceId(target_map),
        ChunkTarget::new(target_map, 4),
        Position(Vec3::new(0.0, 30.0, 0.0)),
        LinearVelocity(Vec3::ZERO),
    ));
}
```

Note: `flat_terrain_voxels` is the same generator used in `spawn_overworld`. It should be extracted to a shared location or passed as a parameter. For now, import from where it's defined.

#### 3. Server Re-enable Physics After Timeout
**File**: `crates/server/src/map_transition.rs`
**Changes**: System that removes `RigidBodyDisabled` + `DisableRollback` after a grace period

```rust
/// Marker for entities in active map transition, with a timer.
#[derive(Component)]
pub struct MapTransitionTimer(pub Timer);

// In initiate_map_transition, also insert:
// MapTransitionTimer(Timer::from_seconds(5.0, TimerMode::Once))

fn tick_map_transition_timers(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut MapTransitionTimer)>,
) {
    for (entity, mut timer) in &mut query {
        timer.0.tick(time.delta());
        if timer.0.finished() {
            commands.entity(entity).remove::<(RigidBodyDisabled, DisableRollback, MapTransitionTimer)>();
        }
    }
}
```

Register in `ServerMapTransitionPlugin::build`.

#### 4. Plugin Registration
**File**: `crates/server/src/main.rs`
**Changes**: Add `ServerMapTransitionPlugin`

#### 5. Tests
**File**: `crates/server/tests/map_transition.rs` (new file)
**Changes**: Tests for server-side transition logic

Uses single-app pattern (no client needed) with minimal plugin setup. Verifies `initiate_map_transition` inserts correct components and `find_or_spawn_homebase` returns/creates correctly.

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use protocol::map::*;
use protocol::*;
use voxel_map_engine::prelude::*;

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app
}

/// initiate_map_transition inserts RigidBodyDisabled, MapInstanceId, Position, etc.
#[test]
fn initiate_transition_inserts_components() {
    let mut app = test_app();
    let target_map = app.world_mut().spawn_empty().id();
    let client = app.world_mut().spawn_empty().id();
    let player = app.world_mut().spawn((
        CharacterMarker,
        Position(Vec3::new(100.0, 100.0, 100.0)),
        LinearVelocity(Vec3::new(5.0, 0.0, 0.0)),
    )).id();

    let mut commands = app.world_mut().commands();
    server::map_transition::initiate_map_transition(
        &mut commands, client, player, target_map,
    );
    app.update();

    let world = app.world();
    assert!(world.get::<RigidBodyDisabled>(player).is_some(), "Should have RigidBodyDisabled");
    assert!(world.get::<DisableRollback>(player).is_some(), "Should have DisableRollback");
    assert_eq!(world.get::<MapInstanceId>(player).unwrap().0, target_map);
    assert_eq!(world.get::<ChunkTarget>(player).unwrap().map_entity, target_map);
    assert_eq!(world.get::<Position>(player).unwrap().0, Vec3::new(0.0, 30.0, 0.0));
    assert_eq!(world.get::<LinearVelocity>(player).unwrap().0, Vec3::ZERO);
}

/// find_or_spawn_homebase returns existing homebase if one exists for the player.
#[test]
fn find_existing_homebase() {
    let mut app = test_app();
    let player = app.world_mut().spawn_empty().id();
    let existing_map = app.world_mut().spawn(Homebase { owner: player }).id();

    app.world_mut().run_system_once(move |
        mut commands: Commands,
        homebases: Query<(Entity, &Homebase)>,
    | {
        let result = server::map_transition::find_or_spawn_homebase(
            &mut commands, player, &homebases,
        );
        assert_eq!(result, existing_map);
    }).unwrap();
}

/// find_or_spawn_homebase spawns new homebase when none exists.
#[test]
fn spawn_new_homebase() {
    let mut app = test_app();
    let player = app.world_mut().spawn_empty().id();

    app.world_mut().run_system_once(move |
        mut commands: Commands,
        homebases: Query<(Entity, &Homebase)>,
    | {
        let result = server::map_transition::find_or_spawn_homebase(
            &mut commands, player, &homebases,
        );
        // Returns a valid entity (commands are deferred, so entity exists after flush)
        assert_ne!(result, Entity::PLACEHOLDER);
    }).unwrap();
    app.update();

    // Verify homebase was spawned with correct owner
    let (_, hb) = app.world_mut()
        .query::<(Entity, &Homebase)>()
        .iter(app.world())
        .next()
        .expect("Homebase should be spawned");
    assert_eq!(hb.owner, player);
}

/// MapTransitionTimer removes RigidBodyDisabled after timeout.
#[test]
fn transition_timer_removes_disabled() {
    let mut app = test_app();
    app.add_systems(Update, server::map_transition::tick_map_transition_timers);

    let entity = app.world_mut().spawn((
        RigidBodyDisabled,
        DisableRollback,
        MapTransitionTimer(Timer::from_seconds(0.1, TimerMode::Once)),
    )).id();

    // Insert enough time to exceed timer
    app.insert_resource(Time::<()>::default());
    // Tick several times with virtual time advancing
    for _ in 0..20 { app.update(); }

    assert!(app.world().get::<RigidBodyDisabled>(entity).is_none(),
        "RigidBodyDisabled should be removed after timer expires");
}
```

Note: `find_or_spawn_homebase` and `initiate_map_transition` must be `pub` (or `pub(crate)` with test access) for these tests. The timer test may need `Time` resource configuration depending on how `MinimalPlugins` handles delta time.

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` compiles
- [x] `cargo test --workspace` passes (including transition logic tests)
- [ ] `cargo server` starts without panic

#### Manual Verification:
- [ ] (Deferred to Phase 6 when UI button exists)

---

## Phase 5: Client MapTransitionState + Loading Gate

### Overview
Client-side SubState for map transitions. Pauses gameplay during transition, shows loading UI, waits for chunks to load before resuming.

### Changes Required:

#### 1. MapTransitionState SubState
**File**: `crates/ui/src/state.rs`
**Changes**: Add SubState

```rust
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, SubStates)]
#[source(ClientState = ClientState::InGame)]
pub enum MapTransitionState {
    #[default]
    Playing,
    Transitioning,
}
```

#### 2. Register SubState and Gate Gameplay
**File**: `crates/ui/src/lib.rs`
**Changes**: Register SubState, add transition systems

```rust
app.add_sub_state::<MapTransitionState>();
```

#### 3. Persist Desired Chunks for Completion Detection
**File**: `crates/voxel_map_engine/src/instance.rs`
**Changes**: Add `desired_chunks` field to `VoxelMapInstance`

```rust
pub struct VoxelMapInstance {
    // ... existing fields ...
    pub desired_chunks: HashSet<IVec3>,
}
```

**File**: `crates/voxel_map_engine/src/lifecycle.rs`
**Changes**: In `update_chunks` (or equivalent), persist the desired set onto the instance

#### 4. Client Transition Handling
**File**: `crates/client/src/map_transition.rs` (new file)
**Changes**: Handle `MapTransitionStart` message, manage transition state

```rust
pub struct ClientMapTransitionPlugin;

impl Plugin for ClientMapTransitionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (
            receive_map_transition_start,
            check_map_transition_complete
                .run_if(in_state(MapTransitionState::Transitioning)),
        ));
        app.add_systems(OnEnter(MapTransitionState::Transitioning), on_enter_transitioning);
        app.add_systems(OnExit(MapTransitionState::Transitioning), on_exit_transitioning);
    }
}

fn receive_map_transition_start(
    mut receiver: Query<&mut MessageReceiver<MapTransitionStart>>,
    mut next_state: ResMut<NextState<MapTransitionState>>,
) {
    for mut msg_receiver in receiver.iter_mut() {
        for _msg in msg_receiver.receive() {
            next_state.set(MapTransitionState::Transitioning);
        }
    }
}

fn on_enter_transitioning(
    mut commands: Commands,
    player: Query<Entity, (With<Predicted>, With<CharacterMarker>)>,
) {
    if let Ok(entity) = player.single() {
        commands.entity(entity).insert((RigidBodyDisabled, DisableRollback));
    }
    // TODO: show loading UI overlay
}

fn on_exit_transitioning(
    mut commands: Commands,
    player: Query<Entity, (With<Predicted>, With<CharacterMarker>)>,
) {
    if let Ok(entity) = player.single() {
        commands.entity(entity).remove::<(RigidBodyDisabled, DisableRollback)>();
    }
    // TODO: hide loading UI overlay
}

fn check_map_transition_complete(
    maps: Query<(&VoxelMapInstance, &PendingChunks)>,
    player: Query<&ChunkTarget, (With<Predicted>, With<CharacterMarker>)>,
    mut next_state: ResMut<NextState<MapTransitionState>>,
) {
    let Ok(target) = player.single() else { return };
    let Ok((instance, pending)) = maps.get(target.map_entity) else { return };
    if pending.tasks.is_empty()
        && instance.desired_chunks.is_subset(&instance.loaded_chunks)
    {
        next_state.set(MapTransitionState::Playing);
    }
}
```

#### 5. Loading UI Overlay
**File**: `crates/ui/src/map_transition_ui.rs` (new file)
**Changes**: Simple "Loading..." overlay spawned on enter, despawned on exit

```rust
#[derive(Component)]
pub struct MapTransitionOverlay;

fn setup_transition_overlay(mut commands: Commands) {
    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            position_type: PositionType::Absolute,
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        MapTransitionOverlay,
        DespawnOnExit(MapTransitionState::Transitioning),
    )).with_children(|parent| {
        parent.spawn((
            Text::new("Loading..."),
            TextFont { font_size: 48.0, ..default() },
            TextColor(Color::WHITE),
        ));
    });
}
```

Register `setup_transition_overlay` on `OnEnter(MapTransitionState::Transitioning)`.

#### 6. Plugin Registration
**File**: `crates/client/src/main.rs` and `crates/web/src/main.rs`
**Changes**: Add `ClientMapTransitionPlugin`

#### 7. Tests
**File**: `crates/ui/tests/ui_plugin.rs` (extend)
**Changes**: Test `MapTransitionState` SubState behavior

Follows existing UI test pattern: `MinimalPlugins` + `StatesPlugin` + `UiPlugin`, drive state via `NextState`.

```rust
/// MapTransitionState defaults to Playing when in InGame state.
#[test]
fn test_map_transition_state_defaults_to_playing() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);
    app.update();

    // Enter InGame state
    app.world_mut().resource_mut::<NextState<ClientState>>().set(ClientState::InGame);
    app.update();

    let state = app.world().resource::<State<MapTransitionState>>();
    assert_eq!(*state.get(), MapTransitionState::Playing);
}

/// MapTransitionState::Transitioning is only valid while ClientState::InGame.
#[test]
fn test_map_transition_state_removed_on_leave_ingame() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);
    app.update();

    // Enter InGame, then Transitioning
    app.world_mut().resource_mut::<NextState<ClientState>>().set(ClientState::InGame);
    app.update();
    app.world_mut().resource_mut::<NextState<MapTransitionState>>()
        .set(MapTransitionState::Transitioning);
    app.update();

    // Leave InGame → SubState should be removed
    app.world_mut().resource_mut::<NextState<ClientState>>().set(ClientState::MainMenu);
    app.update();

    // MapTransitionState resource should no longer exist (SubState removed)
    assert!(app.world().get_resource::<State<MapTransitionState>>().is_none());
}
```

**File**: `crates/client/tests/map_transition.rs` (new file)
**Changes**: Test transition overlay spawns/despawns

```rust
/// Loading overlay spawns on Transitioning, despawns on Playing.
#[test]
fn test_transition_overlay_lifecycle() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.add_plugins(UiPlugin);
    // Add client transition plugin
    app.add_plugins(ClientMapTransitionPlugin);
    app.update();

    // Enter InGame → Playing
    app.world_mut().resource_mut::<NextState<ClientState>>().set(ClientState::InGame);
    app.update();

    // No overlay yet
    let overlay_count = app.world_mut()
        .query_filtered::<Entity, With<MapTransitionOverlay>>()
        .iter(app.world()).count();
    assert_eq!(overlay_count, 0);

    // Enter Transitioning
    app.world_mut().resource_mut::<NextState<MapTransitionState>>()
        .set(MapTransitionState::Transitioning);
    app.update();

    let overlay_count = app.world_mut()
        .query_filtered::<Entity, With<MapTransitionOverlay>>()
        .iter(app.world()).count();
    assert_eq!(overlay_count, 1, "Overlay should spawn during Transitioning");

    // Return to Playing
    app.world_mut().resource_mut::<NextState<MapTransitionState>>()
        .set(MapTransitionState::Playing);
    app.update();

    let overlay_count = app.world_mut()
        .query_filtered::<Entity, With<MapTransitionOverlay>>()
        .iter(app.world()).count();
    assert_eq!(overlay_count, 0, "Overlay should despawn when Playing resumes");
}
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` compiles
- [ ] `cargo test --workspace` passes (including SubState and overlay tests)

#### Manual Verification:
- [ ] (Deferred to Phase 6 when UI button exists)

---

## Phase 6: Map Switch UI Button

### Overview
Add a toggle button to the in-game HUD that sends `PlayerMapSwitchRequest`. Label updates based on current map.

### Changes Required:

#### 1. Button Marker Component
**File**: `crates/ui/src/components.rs`
**Changes**: Add marker

```rust
/// Marker for map switch toggle button in in-game HUD
#[derive(Component)]
pub struct MapSwitchButton;
```

#### 2. Spawn Button in HUD
**File**: `crates/ui/src/lib.rs`
**Changes**: Add map switch button to `setup_ingame_hud`, before existing buttons

Add as first child of the HUD root node:
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

#### 3. Button Interaction and Label Update
**File**: `crates/ui/src/map_switch.rs` (new file)
**Changes**: Handle button press → send message, update label reactively

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use protocol::map::*;
use crate::components::MapSwitchButton;
use crate::state::MapTransitionState;

pub struct MapSwitchPlugin;

impl Plugin for MapSwitchPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                handle_map_switch_button
                    .run_if(in_state(MapTransitionState::Playing)),
                update_map_switch_button_text,
            ),
        );
    }
}

fn handle_map_switch_button(
    button_query: Query<&Interaction, (Changed<Interaction>, With<MapSwitchButton>)>,
    mut message_sender: Query<&mut MessageSender<PlayerMapSwitchRequest>>,
    player_map: Query<&MapInstanceId, With<Predicted>>,
    overworld: Res<OverworldMap>,
) {
    for interaction in button_query.iter() {
        if *interaction != Interaction::Pressed { continue; }
        let target = match player_map.single() {
            Ok(map_id) if map_id.0 == overworld.0 => MapSwitchTarget::Homebase,
            _ => MapSwitchTarget::Overworld,
        };
        for mut sender in message_sender.iter_mut() {
            sender.send::<MapChannel>(PlayerMapSwitchRequest { target });
        }
    }
}

fn update_map_switch_button_text(
    player_map: Query<&MapInstanceId, (With<Predicted>, Changed<MapInstanceId>)>,
    overworld: Res<OverworldMap>,
    button: Query<&Children, With<MapSwitchButton>>,
    mut text: Query<&mut Text>,
) {
    let Ok(map_id) = player_map.single() else { return };
    let label = if map_id.0 == overworld.0 { "Homebase" } else { "Overworld" };
    let Ok(children) = button.single() else { return };
    for child in children.iter() {
        if let Ok(mut t) = text.get_mut(*child) {
            **t = label.to_string();
        }
    }
}
```

#### 4. Plugin Registration
**File**: `crates/ui/src/lib.rs`
**Changes**: Add `MapSwitchPlugin` in `UiPlugin::build`

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` compiles
- [ ] `cargo test --workspace` passes
- [ ] `cargo server` starts
- [ ] `cargo client` connects

#### Manual Verification:
- [ ] "Homebase" button visible in top-right HUD
- [ ] Pressing button → loading screen → arrives in Homebase (flat terrain, separate from Overworld)
- [ ] Button label changes to "Overworld"
- [ ] Pressing again → loading screen → back to Overworld
- [ ] Character physics work correctly in both maps (walking, jumping)
- [ ] Second client in Overworld is unaffected by first client's transition

---

## Phase 7: Lightyear Rooms

### Overview
Scope entity replication per-map using Lightyear's room system. Each map instance gets a `Room`. Replicated entities are only visible to clients in the same room.

### Changes Required:

#### 1. Add RoomPlugin
**File**: `crates/server/src/main.rs`
**Changes**: Add `RoomPlugin` to server app

```rust
use lightyear::prelude::*;
// In app setup:
app.add_plugins(RoomPlugin);
```

`RoomPlugin` is server-only — it controls which entities are replicated to which clients.

#### 2. MapRoom Component
**File**: `crates/protocol/src/map.rs`
**Changes**: Track room entity per map

```rust
/// Room entity associated with a map instance for visibility scoping.
#[derive(Component)]
pub struct MapRoom(pub Entity);
```

#### 3. Spawn Room with Map
**File**: `crates/server/src/map.rs`
**Changes**: When spawning Overworld, also spawn a Room and insert `MapRoom`

```rust
pub fn spawn_overworld(mut commands: Commands, map_world: Res<MapWorld>) {
    let room = commands.spawn(Room::default()).id();
    let map = commands
        .spawn((
            VoxelMapInstance::new(5),
            VoxelMapConfig::new(map_world.seed, 2, None, 5, Arc::new(flat_terrain_voxels)),
            Transform::default(),
            MapRoom(room),
        ))
        .id();
    commands.insert_resource(OverworldMap(map));
}
```

**File**: `crates/server/src/map_transition.rs`
**Changes**: In `find_or_spawn_homebase`, also spawn a Room

```rust
fn find_or_spawn_homebase(...) -> Entity {
    // ... existing lookup ...
    let room = commands.spawn(Room::default()).id();
    commands.spawn((
        instance, config, marker,
        Transform::default(),
        MapRoom(room),
    )).id()
}
```

#### 4. Auto-Room Observer for MapInstanceId
**File**: `crates/server/src/map_transition.rs`
**Changes**: Observer that adds entities to rooms when `MapInstanceId` is inserted or changed

```rust
fn on_map_instance_id_set(
    trigger: On<Insert, MapInstanceId>,
    map_ids: Query<&MapInstanceId>,
    map_rooms: Query<&MapRoom>,
    mut commands: Commands,
) {
    let entity = trigger.target();
    let Ok(map_id) = map_ids.get(entity) else { return };
    let Ok(room) = map_rooms.get(map_id.0) else {
        warn!("MapInstanceId points to {0:?} which has no MapRoom", map_id.0);
        return;
    };
    commands.trigger(RoomEvent {
        room: room.0,
        target: RoomTarget::AddEntity(entity),
    });
}
```

Register as observer in `ServerMapTransitionPlugin::build`.

Note: When `MapInstanceId` changes (e.g., during transition), Bevy fires `Insert` again. The room system's shared-count mechanism handles the add-to-new-room. We also need to remove from the old room — this requires tracking the previous room. Add a `PreviousMapRoom` component or handle removal in `initiate_map_transition`.

#### 5. Room Transition in initiate_map_transition
**File**: `crates/server/src/map_transition.rs`
**Changes**: Add room removal from old map, room addition for new map

```rust
pub fn initiate_map_transition(
    commands: &mut Commands,
    client_entity: Entity,
    player_entity: Entity,
    target_map: Entity,
    old_map_room: Option<Entity>,
    new_map_room: Entity,
) {
    // Remove from old room
    if let Some(old_room) = old_map_room {
        commands.trigger(RoomEvent { room: old_room, target: RoomTarget::RemoveSender(client_entity) });
        commands.trigger(RoomEvent { room: old_room, target: RoomTarget::RemoveEntity(player_entity) });
    }

    // Add to new room
    commands.trigger(RoomEvent { room: new_map_room, target: RoomTarget::AddSender(client_entity) });
    commands.trigger(RoomEvent { room: new_map_room, target: RoomTarget::AddEntity(player_entity) });

    // Physics + map update
    commands.entity(player_entity).insert((
        RigidBodyDisabled,
        DisableRollback,
        MapInstanceId(target_map),
        ChunkTarget::new(target_map, 4),
        Position(Vec3::new(0.0, 30.0, 0.0)),
        LinearVelocity(Vec3::ZERO),
        MapTransitionTimer(Timer::from_seconds(5.0, TimerMode::Once)),
    ));
}
```

#### 6. Add Client to Room on Connect
**File**: `crates/server/src/gameplay.rs`
**Changes**: In `handle_connected`, after spawning character, add client sender + player entity to overworld room

```rust
let room = map_rooms.get(overworld.0).expect("Overworld should have MapRoom").0;
commands.trigger(RoomEvent { room, target: RoomTarget::AddSender(client_entity) });
commands.trigger(RoomEvent { room, target: RoomTarget::AddEntity(player_entity) });
```

This requires querying `MapRoom` in `handle_connected`. Also add the dummy target to the overworld room in `spawn_dummy_target`.

#### 7. Tests
**File**: `crates/server/tests/room_visibility.rs` (new file)
**Changes**: Integration test verifying room-scoped visibility using `CrossbeamTestStepper`

Uses the existing `CrossbeamTestStepper` pattern with two clients. One client switches maps; verify the other stops seeing the switched client's character.

```rust
use lightyear::prelude::*;
use protocol::map::*;

/// Entity in room A is not replicated to client in room B.
#[test]
fn test_room_visibility_isolation() {
    let mut stepper = CrossbeamTestStepper::new();
    // Add RoomPlugin and full gameplay plugins to server
    stepper.server_app.add_plugins(RoomPlugin);
    stepper.server_app.add_plugins(ServerMapTransitionPlugin);
    // ... setup ...
    stepper.init();
    assert!(stepper.wait_for_connection());

    // Both clients start in overworld room
    // Spawn a test entity in a different room and verify client A cannot see it
    let other_room = stepper.server_app.world_mut().spawn(Room::default()).id();
    let test_entity = stepper.server_app.world_mut().spawn((
        CharacterMarker,
        Replicate::to_clients(NetworkTarget::All),
        Position(Vec3::ZERO),
    )).id();

    // Add test_entity to other_room only (not the overworld room)
    stepper.server_app.world_mut().commands()
        .trigger(RoomEvent { room: other_room, target: RoomTarget::AddEntity(test_entity) });

    stepper.tick_step(10);

    // Client should NOT have a predicted/replicated copy of test_entity
    // because client is in overworld room, not other_room
    let client_has_entity = stepper.client_app.world_mut()
        .query_filtered::<Entity, (With<CharacterMarker>, With<Replicated>)>()
        .iter(stepper.client_app.world())
        .any(|e| {
            // Check if this is the test entity (via some identifying component)
            // This depends on how entity mapping works in the test
            true // simplified — real test would check entity identity
        });

    // Exact assertion depends on test infrastructure, but the pattern is:
    // entities not in a shared room should not appear on the client
}

/// Moving entity between rooms: visible → invisible → visible.
#[test]
fn test_room_transition_visibility_change() {
    // Similar setup with two rooms
    // 1. Entity starts in room A (shared with client) → client sees it
    // 2. Move entity to room B (not shared) → client stops seeing it
    // 3. Move entity back to room A → client sees it again
    // Each step: trigger RoomEvents, tick_step, assert entity presence/absence
}
```

**File**: `crates/server/tests/map_transition.rs` (extend)
**Changes**: Test that auto-room observer adds entity to correct room

```rust
/// MapInstanceId insertion triggers auto-room add via observer.
#[test]
fn test_auto_room_observer() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(RoomPlugin);
    app.add_plugins(ServerMapTransitionPlugin);

    let room = app.world_mut().spawn(Room::default()).id();
    let map = app.world_mut().spawn(MapRoom(room)).id();
    let entity = app.world_mut().spawn(MapInstanceId(map)).id();
    app.update();

    // Verify entity was added to the room
    let room_data = app.world().get::<Room>(room).unwrap();
    assert!(room_data.entities.contains(&entity),
        "Entity should be auto-added to room via MapInstanceId observer");
}
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` compiles
- [ ] `cargo test --workspace` passes (including room visibility tests)
- [ ] `cargo server` starts
- [ ] `cargo client` connects

#### Manual Verification:
- [ ] Client A on Overworld, Client B switches to Homebase → Client A no longer sees Client B's character
- [ ] Client B switches back to Overworld → Client A sees Client B again
- [ ] No entity flicker during room transitions (same-frame add/remove)
- [ ] Dummy target visible to all Overworld clients
- [ ] Hitboxes/projectiles scoped to correct room

---

## Testing Strategy

### Unit Tests:
- `MapCollisionHooks::filter_pairs` returns `false` for mismatched `MapInstanceId`, `true` for matching or missing
- `MapInstanceId` implements `MapEntities` correctly
- `find_or_spawn_homebase` returns existing homebase if found, spawns new if not

### Integration Tests:
- Character spawned with `MapInstanceId` matching the overworld map entity
- `attach_chunk_colliders` inserts `MapInstanceId` matching chunk's parent map
- `PlayerMapSwitchRequest` round-trip: client sends, server receives and processes
- Room visibility: entity in room A not visible to client in room B

### Manual Testing Steps:
1. Start server + 2 clients
2. Verify both clients spawn in Overworld, can see each other, physics works
3. Client A presses "Homebase" → loading overlay → new terrain loads → button shows "Overworld"
4. Verify Client A is alone in Homebase, Client B still in Overworld
5. Client A presses "Overworld" → returns, sees Client B
6. Rapidly toggle between maps — no crashes or physics glitches
7. Test jumping near world origin where both maps' terrain might overlap at same coordinates

## Performance Considerations

- `filter_pairs` is called in broad phase — only for entities with `ActiveCollisionHooks::FILTER_PAIRS`. Cost is one `Query::get` per entity per pair.
- `cast_ray_predicate` closure is called per potential hit — adds one `Query::get` per candidate. Negligible for raycast with short distance (4.0 units).
- Room shared-count tracking is O(clients * entities_in_room) per room event. Fine for expected scale (<100 entities per room).
- Homebase spawn-on-demand means first transition has chunk generation cost. Subsequent transitions reuse existing homebase.

## Migration Notes

- `OverworldMap` moves from server/client local definition to protocol. Existing imports must update.
- All physics entities gain `MapInstanceId` + `ActiveCollisionHooks::FILTER_PAIRS`. No behavioral change for single-map gameplay since all entities share the same ID.
- `apply_movement` signature changes — any test calling it directly must be updated.

## References

- Research: [doc/research/2026-03-03-physics-isolation-avian-collision-hooks.md](../research/2026-03-03-physics-isolation-avian-collision-hooks.md)
- Voxel map engine plan: [doc/plans/2026-02-28-voxel-map-engine.md](2026-02-28-voxel-map-engine.md)
- Avian CollisionHooks source: `git/avian/src/collision/hooks.rs`
- Lightyear Room source: `git/lightyear/lightyear_replication/src/visibility/room.rs`
- Lightyear Room tests: `git/lightyear/lightyear_replication/src/visibility/room.rs:343-795`
