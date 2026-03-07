---
date: 2026-03-07T10:59:13-08:00
researcher: Claude
git_commit: db7639b980a2eb485f2cac017cab7ea6644871b9
branch: master
repository: bevy-lightyear-template
topic: "Map instance physics isolation and map switching implementation"
tags: [research, physics, avian3d, collision-hooks, map-instances, map-transition, lightyear-rooms, voxel-map-engine, ui]
status: complete
last_updated: 2026-03-07
last_updated_by: Claude
---

# Research: Map Instance Physics Isolation and Map Switching

**Date**: 2026-03-07T10:59:13-08:00 **Researcher**: Claude **Git Commit**: db7639b980a2eb485f2cac017cab7ea6644871b9 **Branch**: master **Repository**: bevy-lightyear-template

## Research Question

How to implement physics isolation between map instances (Overworld, Homebase) sharing a single Avian physics world, and a clean map switching system with loading states, entity unloading/loading, and a simple UI toggle button for testing.

## Summary

The project runs a single Avian3d physics world shared by server and client. All entities — characters, terrain chunks, hitboxes, projectiles — coexist in this world. Currently, only collision **layers** separate entity types (Character, Terrain, Hitbox, Projectile). There is no mechanism to separate entities by map **instance**. When multiple maps exist at overlapping world positions, their physics would interact.

Avian 0.4.1 provides `CollisionHooks` — a `ReadOnlySystemParam`-based trait with `filter_pairs` (broad phase) and `modify_contacts` (narrow phase) methods. A `MapInstanceId` component on every physics entity, combined with a `filter_pairs` implementation that returns `false` for cross-map pairs, achieves physics isolation. However, `filter_pairs` does **not** affect `SpatialQuery` operations — the ground-detection raycast in `apply_movement` requires separate filtering via `cast_ray_predicate`.

Map switching requires: a `MapInstanceId` semantic enum (not Entity-based, to avoid entity mapping issues across network boundaries), a `MapRegistry` resource on each side, a client-side `MapTransitionState` sub-state, `RigidBodyDisabled` during transitions, lightyear room management for entity visibility, and a UI toggle button following existing HUD patterns. `ChunkTarget` should become local-only (not replicated) since each side resolves it independently from `MapInstanceId` + `MapRegistry`.

## Detailed Findings

### 1. Current Physics Setup

**Avian3d 0.4.1** configured in `SharedGameplayPlugin` at [lib.rs:242-248](crates/protocol/src/lib.rs#L242-L248):

```rust
PhysicsPlugins::default()
    .build()
    .disable::<PhysicsTransformPlugin>()
    .disable::<PhysicsInterpolationPlugin>()
    .disable::<IslandSleepingPlugin>()
```

- `PhysicsTransformPlugin` disabled — lightyear handles position replication
- `PhysicsInterpolationPlugin` disabled — lightyear handles interpolation
- `IslandSleepingPlugin` disabled — bodies never sleep

No `CollisionHooks` are registered. Physics runs in a single global world.

**Lightyear integration** at [lib.rs:237-240](crates/protocol/src/lib.rs#L237-L240): `LightyearAvianPlugin` with `AvianReplicationMode::Position`. Position, Rotation, LinearVelocity, AngularVelocity registered for prediction with custom rollback thresholds.

### 2. Current Collision Layers

Defined at [hit_detection.rs:17-53](crates/protocol/src/hit_detection.rs#L17-L53):

`GameLayer` enum has 5 layers: `Default`, `Character`, `Hitbox`, `Projectile`, `Terrain`.

| Function | Membership | Collides With |
|----------|-----------|---------------|
| `character_collision_layers()` | Character | Character, Terrain, Hitbox, Projectile |
| `terrain_collision_layers()` | Terrain | Character |
| `projectile_collision_layers()` | Projectile | Character |
| `hitbox_collision_layers()` | Hitbox | Character |

These separate entity **types** but not map **instances**. With Avian's 32-bit layer limit, dedicating layers per instance is not scalable.

### 3. CollisionHooks API (avian3d 0.4.1)

From local source at `git/avian/src/collision/hooks.rs`:

```rust
pub trait CollisionHooks: ReadOnlySystemParam + Send + Sync {
    fn filter_pairs(&self, collider1: Entity, collider2: Entity, commands: &mut Commands) -> bool { true }
    fn modify_contacts(&self, contacts: &mut ContactPair, commands: &mut Commands) -> bool { true }
}
```

- **`filter_pairs`**: Called in broad phase. Returns `false` to skip narrow phase entirely (efficient early-out). Only called when at least one entity has `ActiveCollisionHooks::FILTER_PAIRS`.
- **`modify_contacts`**: Called in narrow phase after contact computation. Can modify friction, restitution, contact points.
- **Requires `ReadOnlySystemParam`**: No mutable queries. Deferred writes via `Commands` only.
- **Cannot access `ContactGraph`**: Panics if attempted.
- **One impl per app**: `PhysicsPlugins::default().with_collision_hooks::<T>()` accepts exactly one type.

**ActiveCollisionHooks** component (bitflag `u8`):
- `FILTER_PAIRS` (0b01) — enables `filter_pairs` calls for this entity
- `MODIFY_CONTACTS` (0b10) — enables `modify_contacts` calls for this entity

Hooks are **opt-in per entity**. Entities without this component skip hook evaluation entirely. Avian does not call hooks when both entities are `RigidBody::Static` or `Sleeping` — irrelevant here since terrain-terrain non-interaction is already handled by collision layers.

**Registration** — current code at [lib.rs:242](crates/protocol/src/lib.rs#L242) must change from `PhysicsPlugins::default().build()` to `PhysicsPlugins::default().with_collision_hooks::<MapCollisionHooks>().build()`.

### 4. SpatialQuery — Not Affected by CollisionHooks

`SpatialQuery::cast_ray` operates independently from the collision pipeline. It uses `SpatialQueryFilter` which supports collision layer masks, entity include/exclude sets, and `ColliderDisabled` exclusion — but **not** `CollisionHooks`.

The ground-detection raycast at [lib.rs:310-321](crates/protocol/src/lib.rs#L310-L321) currently uses only self-exclusion:

```rust
let filter = &SpatialQueryFilter::from_excluded_entities([entity]);
if spatial_query.cast_ray(ray_cast_origin, Dir3::NEG_Y, 4.0, false, filter).is_some() {
    forces.apply_linear_impulse(Vec3::new(0.0, 400.0, 0.0));
}
```

Without additional filtering, a character in the Overworld could detect ground from a Homebase terrain chunk at an overlapping world position.

**Solution**: `SpatialQuery::cast_ray_predicate` accepts a closure for per-entity filtering:

```rust
let map_id = map_ids.get(entity).ok();
let filter = SpatialQueryFilter::from_excluded_entities([entity]);
spatial_query.cast_ray_predicate(ray_cast_origin, Dir3::NEG_Y, 4.0, false, &filter, &|hit_entity| {
    match (map_id, map_ids.get(hit_entity).ok()) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
})
```

This requires passing a `MapInstanceId` query into `apply_movement`.

### 5. Physics Entity Types and Where They Spawn

All entity types that need `MapInstanceId` + `ActiveCollisionHooks::FILTER_PAIRS`:

| Entity Type | RigidBody | Collider | Spawn Location |
|------------|-----------|---------|----------------|
| Character | Dynamic | Capsule(r=2, h=2) | Server: [gameplay.rs:160-208](crates/server/src/gameplay.rs#L160-L208), Client: [gameplay.rs:16-53](crates/client/src/gameplay.rs#L16-L53) |
| Terrain chunk | Static | Trimesh (from mesh) | [map.rs:46-72](crates/protocol/src/map.rs#L46-L72) |
| Melee hitbox | Kinematic | Cuboid(1.5, 2.0, 1.0) | [ability.rs:1097-1136](crates/protocol/src/ability.rs#L1097-L1136) |
| AoE hitbox | Kinematic | Sphere(radius) | [ability.rs:1138-1178](crates/protocol/src/ability.rs#L1138-L1178) |
| Projectile | Kinematic | Sphere(0.5) | [ability.rs:1449-1477](crates/protocol/src/ability.rs#L1449-L1477) |
| Dummy target | Dynamic | Capsule(r=2, h=2) | [gameplay.rs:60-74](crates/server/src/gameplay.rs#L60-L74) |

**MapInstanceId insertion points**:
- **Characters/dummies**: Set at spawn based on target map
- **Terrain chunks**: Inserted in `attach_chunk_colliders` by looking up parent map entity's `MapInstanceId` via `ChildOf`
- **Hitboxes/projectiles**: Cloned from caster's `MapInstanceId` at spawn time

### 6. MapInstanceId — Semantic Enum, Not Entity Reference

`MapInstanceId` must be a **semantic enum**, not an `Entity` wrapper. VoxelMapInstance entities are independently spawned on server and client (never replicated). If `MapInstanceId` held a raw `Entity` reference, lightyear's entity mapping would produce `Entity::PLACEHOLDER` on the client.

```rust
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum MapInstanceId {
    Overworld,
    Homebase { owner: ClientId },
    Arena { id: u32 },
}
```

Both sides resolve the enum to their local map entity via a `MapRegistry` resource:

```rust
#[derive(Resource, Default)]
pub struct MapRegistry(pub HashMap<MapInstanceId, Entity>);
```

Each side populates this when spawning map instances. No `MapEntities` impl needed — no Entity references cross the network. Enum comparison works identically on server and client.

**Required change**: `Homebase { owner: Entity }` at [instance.rs:14-16](crates/voxel_map_engine/src/instance.rs#L14-L16) must change to `Homebase { owner: ClientId }` to match the enum. `ClientId` is consistent between server and client.

### 7. Voxel Map Engine — Existing Infrastructure

The voxel map engine provides entity-based multiplexing:

- **`VoxelMapInstance`** component at [instance.rs:25-32](crates/voxel_map_engine/src/instance.rs#L25-L32) — holds `loaded_chunks: HashSet<IVec3>`, octree, modified_voxels
- **`VoxelMapConfig`** at [config.rs:10-17](crates/voxel_map_engine/src/config.rs#L10-L17) — seed, spawning_distance, bounds, tree_height, generator function
- **`ChunkTarget`** at [chunk.rs:13-17](crates/voxel_map_engine/src/chunk.rs#L13-L17) — `map_entity: Entity`, `distance: u32`
- **Marker components**: `Overworld`, `Homebase { owner }`, `Arena { id }` at [instance.rs:8-22](crates/voxel_map_engine/src/instance.rs#L8-L22)
- **Chunks are children** of their map entity — inserted at [lifecycle.rs:224](crates/voxel_map_engine/src/lifecycle.rs#L224)
- **`PendingChunks`** at [generation.rs:20-24](crates/voxel_map_engine/src/generation.rs#L20-L24) — `tasks: Vec<Task<ChunkGenResult>>`, `pending_positions: HashSet<IVec3>`
- **`OverworldMap(Entity)`** resource — defined separately on server ([server/map.rs:21](crates/server/src/map.rs#L21)) and client ([client/map.rs:43](crates/client/src/map.rs#L43))

**Constructor methods**: `VoxelMapInstance::overworld()`, `homebase()`, `arena()` at [instance.rs:46-90](crates/voxel_map_engine/src/instance.rs#L46-L90) return `(VoxelMapInstance, VoxelMapConfig, MarkerComponent)` bundles.

**Chunk lifecycle** at [lifecycle.rs](crates/voxel_map_engine/src/lifecycle.rs):
- `collect_desired_positions` computes desired set from all `ChunkTarget` entities pointing at each map (line 60)
- `spawn_missing_chunks` caps at `MAX_TASKS_PER_FRAME = 32` (line 11, 108)
- `despawn_out_of_range_chunks` removes chunk entities whose position is no longer in `loaded_chunks` (line 232)

### 8. ChunkTarget — Currently Replicated, Should Not Be

`ChunkTarget` is registered with lightyear at [lib.rs:167](crates/protocol/src/lib.rs#L167):

```rust
app.register_component::<ChunkTarget>().add_map_entities();
```

It implements `MapEntities` at [chunk.rs:19-23](crates/voxel_map_engine/src/chunk.rs#L19-L23), mapping `map_entity`. However, this is broken: the `map_entity` references a local `VoxelMapInstance` entity that is never replicated, so the mapped entity on the client side would not correspond to the client's local map entity.

Current client behavior: `ChunkTarget` is attached to the camera locally at [client/map.rs:56-66](crates/client/src/map.rs#L56-L66), pointing at the client's `OverworldMap.0` entity. The server attaches `ChunkTarget` to player entities at [gameplay.rs:207](crates/server/src/gameplay.rs#L207).

%% ChunkTarget should also be moved from camera to player on clients

`ChunkTarget` should become local-only. Each side derives it from `MapInstanceId` + `MapRegistry`. Remove `register_component::<ChunkTarget>().add_map_entities()` from lightyear registration.

### 9. MapCollisionHooks Implementation

A `SystemParam`-based implementation for `filter_pairs`:

```rust
impl<'w, 's> CollisionHooks for MapCollisionHooks<'w, 's> {
    fn filter_pairs(&self, entity1: Entity, entity2: Entity, _commands: &mut Commands) -> bool {
        match (self.map_ids.get(entity1).ok(), self.map_ids.get(entity2).ok()) {
            (Some(a), Some(b)) => a == b,
            _ => true, // entities without MapInstanceId interact with everything
        }
    }
}
```

`MapCollisionHooks` is a `SystemParam` containing a read-only `Query<&MapInstanceId>`. The `_ => true` fallthrough allows global physics entities (if any) to interact with everything.

**ActiveCollisionHooks opt-in**: Use `#[require(ActiveCollisionHooks::FILTER_PAIRS)]` on `MapInstanceId` so inserting the component automatically opts in to hook evaluation.

**Hook extensibility**: Only one `CollisionHooks` impl per app. Future needs (one-way platforms, conveyors) must be added as additional queries to the same `MapCollisionHooks` SystemParam.

### 10. Lightyear Rooms for Entity Visibility

Lightyear rooms at `git/lightyear/lightyear_replication/src/visibility/room.rs` provide interest management. Each map instance should have a corresponding room. Entities are visible to a client only if they share at least one room.

**API**:
- Spawn a room: `commands.spawn(Room::default())`
- Add/remove via `commands.trigger(RoomEvent { room, target: RoomTarget::AddEntity(entity) })`
- Variants: `AddEntity`, `RemoveEntity`, `AddSender`, `RemoveSender`

**Same-frame transfer**: Remove from old room + add to new room in the same frame preserves visibility (shared count never drops to 0). Confirmed by `test_move_client_entity_room`.

**Child entities**: Do not inherit room membership explicitly, but lightyear's hierarchy system falls back to the root entity's `NetworkVisibility` for children with `ReplicateLike`.

**Automation**: An observer on `MapInstanceId` insert/change (server-side) can automatically fire `RoomEvent` to add entities to the corresponding room. Requires a `RoomRegistry` mapping `MapInstanceId` variants to room entities.

### 11. Map Transition State Machine

No `SubStates` exist currently. `ClientState` at [state.rs:3-13](crates/ui/src/state.rs#L3-L13) has `MainMenu`, `Connecting`, `InGame`. `AppState` at [app_state.rs:4-9](crates/protocol/src/app_state.rs#L4-L9) has `Loading`, `Ready`.

A `MapTransitionState` sub-state under `ClientState::InGame`:

```rust
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, SubStates)]
#[source(ClientState = ClientState::InGame)]
enum MapTransitionState {
    #[default]
    Playing,
    Transitioning,
}
```

- Gameplay systems run in `in_state(MapTransitionState::Playing)`
- `OnEnter(Transitioning)`: show loading UI, insert `RigidBodyDisabled` on player
- `OnExit(Transitioning)`: hide loading UI, remove `RigidBodyDisabled`

### 12. RigidBodyDisabled for Transition Pausing

`RigidBodyDisabled` at `git/avian/src/dynamics/rigid_body/mod.rs:376-380` is a marker component that:
- Excludes the entity from the solver (no forces/impulses)
- Disables contact response
- **Preserves** Position, Rotation, LinearVelocity — so teleporting during transition works
- Does **not** disable collision detection or spatial queries for attached colliders

During transition, gameplay systems (including `apply_movement` raycasts) are gated on `MapTransitionState::Playing`, so raycasts don't run while the player is disabled.

### 13. Lightyear Messages and Channels

Currently one channel (`VoxelChannel`, ordered reliable, bidirectional) and three messages at [lib.rs:148-164](crates/protocol/src/lib.rs#L148-L164):

| Message | Direction | Defined at |
|---------|-----------|------------|
| `VoxelEditRequest` | ClientToServer | [map.rs:26-30](crates/protocol/src/map.rs#L26-L30) |
| `VoxelEditBroadcast` | ServerToClient | [map.rs:33-37](crates/protocol/src/map.rs#L33-L37) |
| `VoxelStateSync` | ServerToClient | [map.rs:40-43](crates/protocol/src/map.rs#L40-L43) |

**Message send pattern** (client): `Query<&mut MessageSender<T>>`, iterate senders, call `sender.send::<Channel>(msg)`. See [client/map.rs:157-166](crates/client/src/map.rs#L157-L166).

**Message receive pattern** (server): `Query<&mut MessageReceiver<T>>`, iterate receivers, call `receiver.receive()` to drain. See [server/map.rs:291-333](crates/server/src/map.rs#L291-L333).

Map switching needs a new `MapChannel` (ordered reliable, bidirectional) with `PlayerMapSwitchRequest` (C→S) and `MapTransitionStart` (S→C).

### 14. DisableRollback Usage

Used on melee hitboxes ([ability.rs:1128](crates/protocol/src/ability.rs#L1128)), AoE hitboxes ([ability.rs:1167](crates/protocol/src/ability.rs#L1167)), and projectiles ([ability.rs:1470](crates/protocol/src/ability.rs#L1470)). These are ephemeral server-authoritative entities that should not be rolled back during prediction resimulation. The player entity should also get `DisableRollback` during transitions to prevent rollback to stale pre-transition state.

### 15. MapInstanceId Replication

Register with `register_component::<MapInstanceId>()` only (no `add_prediction()`). In current lightyear, `Predicted` is a marker on the same entity that receives replicated components, so `Query<&MapInstanceId, With<Predicted>>` matches without prediction registration. Map transitions are server-authoritative and must not be rolled back.

### 16. UI Button Patterns

The in-game HUD at [lib.rs:312-385](crates/ui/src/lib.rs#L312-L385) has "Main Menu" and "Quit" buttons in the top-right. Pattern:

1. Marker component in [components.rs](crates/ui/src/components.rs) (e.g. `MainMenuButton`)
2. Spawn in HUD setup with `(Button, Node, BorderColor, BackgroundColor, MarkerComponent)` + child `Text`
3. Interaction system queries `Query<&Interaction, (Changed<Interaction>, With<MarkerComponent>)>`, checks `== Interaction::Pressed`

A `MapSwitchButton` follows this pattern. The button text toggles dynamically — shows "Homebase" when on Overworld, "Overworld" when on Homebase — driven by observing `MapInstanceId` changes on the predicted player entity.

### 17. Map Switch Messages

**`PlayerMapSwitchRequest`** (client → server):

```rust
pub enum MapSwitchTarget {
    Overworld,
    Homebase,
}

pub struct PlayerMapSwitchRequest {
    pub target: MapSwitchTarget,
}
```

**`MapTransitionStart`** (server → client):

```rust
pub struct MapTransitionStart {
    pub target: MapInstanceId,
    pub seed: u64,
    pub generation_version: u32,
    pub bounds: Option<IVec3>,
}
```

`target` is `MapInstanceId` (not `MapSwitchTarget`) so the client gets the full identity (e.g. `Homebase { owner: client_id }`). The generator function is implicit from the variant. Config fields let the client spawn a matching `VoxelMapInstance` with identical terrain generation.

### 18. Server-Side Transition

The server handler receives `PlayerMapSwitchRequest`, resolves or spawns the target map, then executes:

1. Insert `RigidBodyDisabled` + `DisableRollback` on player
2. Room transitions (remove from old, add to new) — both client sender and player entity
3. Update `MapInstanceId` to new variant (replicates to client)
4. Update `ChunkTarget.map_entity` to server-local map entity
5. Set `Position` to new map spawn point, zero `LinearVelocity`
6. Send `MapTransitionStart` to the client

After client confirms chunks loaded (or timeout): remove `RigidBodyDisabled` + `DisableRollback`.

For server-initiated transitions (portals, game events), the server calls the same transition function directly, bypassing `PlayerMapSwitchRequest`.

**Homebase spawn-on-demand**: The server lazily spawns a player's homebase the first time they request it. `Homebase { owner }` marker tracks ownership for lookup on subsequent requests.

### 19. Client-Side Transition

1. Receive `MapTransitionStart` → set `MapTransitionState::Transitioning`
2. `OnEnter(Transitioning)`: insert `RigidBodyDisabled` + `DisableRollback` on player, show loading UI
3. Spawn new client-local `VoxelMapInstance` for target map if not already in `MapRegistry`
4. Register in `MapRegistry`
5. Update player entity's local `ChunkTarget.map_entity` to new client-local map entity
6. Wait for chunks to load

**Chunk loading completion**: The reliable check is `desired_chunks ⊆ loaded_chunks && pending.tasks.is_empty()`. Since `collect_desired_positions` computes the desired set transiently each frame (not persisted), a `desired_chunks` field should be added to `VoxelMapInstance` (or a separate component). `MAX_TASKS_PER_FRAME = 32` means `pending.tasks.is_empty()` can be momentarily true while more chunks still need spawning — checking pending alone is unreliable.

7. When loaded → set `MapTransitionState::Playing`
8. `OnExit(Transitioning)`: remove `RigidBodyDisabled` + `DisableRollback`, hide loading UI

### 20. Spatial Overlap Between Concurrent Maps

Multiple maps may have terrain at overlapping world positions. Physics isolation via `filter_pairs` prevents cross-map collisions. The loading screen during `MapTransitionState::Transitioning` hides visual overlap:

1. Loading screen appears → old map's `ChunkTarget` removed → old chunks unload via `despawn_out_of_range_chunks`
2. New map's chunks load
3. Loading screen disappears → only new map visible

No world-space offset needed.

### 21. Orphaned Map Cleanup

When a map has no `ChunkTarget` pointing to it, its chunks unload but the `VoxelMapInstance` entity persists (leaking octree, modified_voxels, etc.). A shared cleanup system despawns `VoxelMapInstance` entities with no active `ChunkTarget`:

- Skip the Overworld (persists always)
- Server: run with a delay/cooldown to avoid despawning during momentary transitions
- Client: can run immediately
- Also remove the entry from `MapRegistry`

### 22. Entity Unloading/Loading During Transitions

When a player transitions maps, entities from the old map should stop being visible and entities in the new map should appear. Lightyear rooms handle this:

- **Server-side**: Moving the client sender from old room to new room causes entities in the old room (other players, dummies, hitboxes) to lose visibility. Entities in the new room gain visibility. Lightyear's replication system handles despawn/spawn on the client automatically.
- **Terrain chunks**: Children of the map entity. If the map entity is in a room with `Replicate`, child chunks with `ReplicateLike` inherit visibility via hierarchy fallback. However, terrain chunks are currently spawned locally on both sides (not replicated via lightyear), so room visibility primarily matters for replicated game entities (players, hitboxes, projectiles).
- **Local-only entities** (terrain chunks, map entities): Unloading is driven by `ChunkTarget` removal → `despawn_out_of_range_chunks`. Loading is driven by new `ChunkTarget` pointing at the new map → `spawn_missing_chunks`.

### 23. Component Registration Overview

Components registered with lightyear at [lib.rs:167-209](crates/protocol/src/lib.rs#L167-L209):

| Component | `add_prediction()` | `add_map_entities()` |
|-----------|:--:|:--:|
| `ChunkTarget` | no | **yes** |
| `PlayerId` | no | no |
| `CharacterMarker` | **yes** | no |
| `Health` | **yes** | no |
| `Position` | **yes** | no |
| `Rotation` | **yes** | no |
| `LinearVelocity` | **yes** | no |
| `AngularVelocity` | **yes** | no |
| `ActiveAbility` | **yes** | **yes** |
| `AbilityCooldowns` | **yes** | no |

## Code References

- [lib.rs:237-248](crates/protocol/src/lib.rs#L237-L248) — LightyearAvianPlugin + PhysicsPlugins registration
- [lib.rs:296-322](crates/protocol/src/lib.rs#L296-L322) — `apply_movement` with SpatialQuery ground detection
- [lib.rs:148-164](crates/protocol/src/lib.rs#L148-L164) — Channel + message registration
- [lib.rs:167-209](crates/protocol/src/lib.rs#L167-L209) — Component replication registration
- [hit_detection.rs:17-53](crates/protocol/src/hit_detection.rs#L17-L53) — GameLayer and collision layer functions
- [map.rs:46-72](crates/protocol/src/map.rs#L46-L72) — `attach_chunk_colliders` system
- [ability.rs:1097-1178](crates/protocol/src/ability.rs#L1097-L1178) — Hitbox spawning (melee + AoE)
- [ability.rs:1449-1477](crates/protocol/src/ability.rs#L1449-L1477) — Projectile spawning
- [server/gameplay.rs:60-74](crates/server/src/gameplay.rs#L60-L74) — Dummy target spawn
- [server/gameplay.rs:160-208](crates/server/src/gameplay.rs#L160-L208) — Character spawn with ChunkTarget
- [client/gameplay.rs:16-53](crates/client/src/gameplay.rs#L16-L53) — Client predicted character setup
- [client/map.rs:45-66](crates/client/src/map.rs#L45-L66) — Client overworld spawn + camera ChunkTarget
- [instance.rs:8-32](crates/voxel_map_engine/src/instance.rs#L8-L32) — VoxelMapInstance + marker components
- [chunk.rs:13-23](crates/voxel_map_engine/src/chunk.rs#L13-L23) — ChunkTarget + MapEntities impl
- [lifecycle.rs](crates/voxel_map_engine/src/lifecycle.rs) — Chunk lifecycle (desired, spawn, despawn)
- [generation.rs:20-24](crates/voxel_map_engine/src/generation.rs#L20-L24) — PendingChunks
- [config.rs:10-17](crates/voxel_map_engine/src/config.rs#L10-L17) — VoxelMapConfig
- [state.rs:3-13](crates/ui/src/state.rs#L3-L13) — ClientState
- [app_state.rs:4-9](crates/protocol/src/app_state.rs#L4-L9) — AppState
- [lib.rs:312-425](crates/ui/src/lib.rs#L312-L425) — HUD setup + button interactions
- [components.rs](crates/ui/src/components.rs) — UI marker components
- `git/avian/src/collision/hooks.rs` — CollisionHooks trait
- `git/avian/src/dynamics/rigid_body/mod.rs:376-380` — RigidBodyDisabled
- `git/lightyear/lightyear_replication/src/visibility/room.rs` — Room system

## Architecture Documentation

**Single physics world**: All entities share one Avian physics world. Isolation is currently type-based only (CollisionLayers). Instance-based isolation requires broad-phase hook filtering + SpatialQuery predicate filtering.

**Map identity split**: `MapInstanceId` (semantic enum) is the network-safe identity — replicated, works on both sides. `ChunkTarget.map_entity` is the local-only entity reference — each side derives it from `MapInstanceId` + `MapRegistry`. Chunk-parent hierarchy tracks map membership for terrain entities.

**Hook extensibility constraint**: Only one `CollisionHooks` impl per app. The `MapCollisionHooks` SystemParam must accommodate future hook needs (one-way platforms, etc.) by adding more queries to the same struct.

**Entity lifecycle during transitions**: Replicated entities (players, hitboxes) are managed by lightyear rooms — visibility changes cause automatic spawn/despawn on clients. Local-only entities (terrain chunks) are managed by `ChunkTarget` addition/removal driving the chunk lifecycle systems.

## Historical Context (from doc/)

- `doc/plans/2026-02-28-voxel-map-engine.md:868-919` — Original proposal for physics isolation via CollisionHooks, scoped as "Future Work"
- `doc/research/2026-02-27-bonsairobo-stack-multi-instance-voxel-replacement.md` — Multi-instance voxel architecture research
- `doc/research/2026-02-13-hit-detection-system.md` — Hit detection system research (Avian3d sensor/collision APIs)
- `doc/research/2026-01-09-raycast-chunk-collider-detection.md` — Jump raycasts not detecting chunk colliders (schedule mismatch)
- `doc/plans/2026-02-14-hit-detection-knockback.md` — Hit detection + knockback implementation plan

## Related Research

- [doc/research/2026-02-27-bonsairobo-stack-multi-instance-voxel-replacement.md](doc/research/2026-02-27-bonsairobo-stack-multi-instance-voxel-replacement.md)
- [doc/research/2026-02-13-hit-detection-system.md](doc/research/2026-02-13-hit-detection-system.md)
- [doc/research/2026-01-09-raycast-chunk-collider-detection.md](doc/research/2026-01-09-raycast-chunk-collider-detection.md)

## External Sources

- [Avian 0.3 Blog Post (CollisionHooks introduction)](https://joonaa.dev/blog/08/avian-0-3)
- [One-way platform example (avian2d)](https://github.com/Jondolf/avian/blob/main/crates/avian2d/examples/one_way_platform_2d.rs)
