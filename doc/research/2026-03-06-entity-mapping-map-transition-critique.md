---
date: 2026-03-06T22:59:37-08:00
researcher: Claude
git_commit: db7639b980a2eb485f2cac017cab7ea6644871b9
branch: master
repository: bevy-lightyear-template
topic: "Critical analysis of physics isolation research — entity mapping failures in map transition design"
tags: [research, lightyear, entity-mapping, map-transition, physics-isolation, critique]
status: complete
last_updated: 2026-03-06
last_updated_by: Claude
---

# Research: Entity Mapping Failures in Map Transition Design

**Date**: 2026-03-06T22:59:37-08:00 **Researcher**: Claude **Git Commit**: db7639b980a2eb485f2cac017cab7ea6644871b9 **Branch**: master **Repository**: bevy-lightyear-template

## Research Question

The research document `doc/research/2026-03-03-physics-isolation-avian-collision-hooks.md` proposes a `MapInstanceId(Entity)` component and map transition protocol. When implemented, the client fails to leave the `MapTransitionState::Transitioning` state. Critically analyze the research to identify the root causes.

## Summary

The research document contains **five fatal design flaws**, all stemming from a single incorrect assumption: that server-side `Entity` references to `VoxelMapInstance` entities are meaningful on the client. They are not. VoxelMapInstance entities are spawned independently on server and client with different Entity IDs, are never replicated via lightyear, and therefore have no entry in lightyear's entity mapping table. Any `Entity` reference to a VoxelMapInstance that crosses the network boundary becomes `Entity::PLACEHOLDER` on the client.

## The Dual-World Architecture (What the Research Misses)

The server and client each independently spawn their own VoxelMapInstance entities:

**Server** ([server/map.rs:23-31](crates/server/src/map.rs#L23-L31)):
```rust
fn spawn_overworld(mut commands: Commands, map_world: Res<MapWorld>) {
    let map = commands.spawn((VoxelMapInstance::new(5), ...)).id();
    commands.insert_resource(OverworldMap(map)); // Server entity, e.g. 5v1
}
```

**Client** ([client/map.rs:45-53](crates/client/src/map.rs#L45-L53)):
```rust
fn spawn_overworld(mut commands: Commands, map_world: Res<MapWorld>) {
    let map = commands.spawn((VoxelMapInstance::new(5), ...)).id();
    commands.insert_resource(OverworldMap(map)); // Client entity, e.g. 2v1
}
```

These are completely independent entities. Neither carries a `Replicate` component. `VoxelMapInstance` is not registered with lightyear (`register_component::<VoxelMapInstance>` does not exist anywhere). The client produces identical terrain via matching seeds and generator functions, not via replication.

Lightyear's `RemoteEntityMap` only contains entries for entities that have been replicated. Since VoxelMapInstance entities are never replicated, **there is no server-to-client entity mapping for them**.

## Fatal Flaw 1: MapInstanceId(Entity) Becomes Entity::PLACEHOLDER on Client

The research proposes `MapInstanceId(Entity)` where the inner `Entity` is a VoxelMapInstance entity, registered with `add_map_entities()`. Here is what actually happens during replication:

1. **Server serializes** `MapInstanceId(server_map_entity)`. `SendEntityMap` checks its local-to-remote map — the server map entity is not replicated, so no mapping exists. The entity is sent as-is ([entity_map.rs:56-59](git/lightyear/lightyear_serde/src/entity_map.rs#L56-L59)).

2. **Client deserializes** the component. `ReceiveEntityMap::get_mapped` looks up the server entity in `remote_to_local` — no mapping exists because VoxelMapInstance was never replicated. **Returns `Entity::PLACEHOLDER`** ([entity_map.rs:79-83](git/lightyear/lightyear_serde/src/entity_map.rs#L79-L83)):
   ```rust
   self.0.get(&entity).copied().unwrap_or_else(|| {
       debug!("Receive: Failed to map entity {entity:?}");
       Entity::PLACEHOLDER
   })
   ```

3. On the client, `MapInstanceId.0 == Entity::PLACEHOLDER`. This entity does not correspond to any VoxelMapInstance.

**Note**: `ChunkTarget.map_entity` already has this exact problem today. It's registered with `add_map_entities()` at [lib.rs:167](crates/protocol/src/lib.rs#L167), but on replicated player entities the `map_entity` field becomes `Entity::PLACEHOLDER` on the client. This currently has no effect because the client drives chunk loading via the camera's locally-inserted `ChunkTarget` (which correctly points to the client's own overworld entity), not via replicated player ChunkTargets.

## Fatal Flaw 2: Transition Completion Check Queries an Unmapped Entity

The proposed `check_map_transition_complete` system ([research:430-443](doc/research/2026-03-03-physics-isolation-avian-collision-hooks.md)):

```rust
fn check_map_transition_complete(
    maps: Query<(&VoxelMapInstance, &PendingChunks)>,
    player: Query<&ChunkTarget, With<Predicted>>,
    ...
) {
    let Ok(target) = player.single() else { return };
    let Ok((instance, pending)) = maps.get(target.map_entity) else { return };
    // ...
}
```

This fails for **two independent reasons**:

**Reason A**: `ChunkTarget` is registered without `.add_prediction()` ([lib.rs:167](crates/protocol/src/lib.rs#L167)):
```rust
app.register_component::<ChunkTarget>().add_map_entities();
```
Without `add_prediction()`, `ChunkTarget` exists only on the confirmed entity, not the predicted entity. `player.single()` with `With<Predicted>` will fail because the predicted entity has no `ChunkTarget`.

**Reason B**: Even if `ChunkTarget` were predicted, `target.map_entity` is `Entity::PLACEHOLDER` (see Flaw 1). `maps.get(Entity::PLACEHOLDER)` returns `Err` because `Entity::PLACEHOLDER` doesn't have `VoxelMapInstance`. The system returns early. **The client never exits `Transitioning` state.**

## Fatal Flaw 3: OverworldMap Entity Comparison Is Always False

The research proposes comparing `MapInstanceId` against `OverworldMap` on the client ([research:572-574](doc/research/2026-03-03-physics-isolation-avian-collision-hooks.md)):

```rust
let target = match player_map.single() {
    Ok(map_id) if map_id.0 == overworld.0 => MapSwitchTarget::Homebase,
    _ => MapSwitchTarget::Overworld,
};
```

- `map_id.0` = `Entity::PLACEHOLDER` (replicated, mapped to placeholder)
- `overworld.0` = client's locally-spawned entity (e.g. `2v1`)
- `Entity::PLACEHOLDER != 2v1` — **always falls through to `Overworld`**

The button label update system has the same issue — it can never determine which map the player is on.

## Fatal Flaw 4: CollisionHooks filter_pairs Compares Unmapped Entities

The proposed `MapCollisionHooks::filter_pairs` compares `MapInstanceId` values between two colliding entities. On the client (where prediction runs physics), both entities have `MapInstanceId.0 == Entity::PLACEHOLDER` because both were replicated from the server with unmapped entity references. The comparison `Entity::PLACEHOLDER == Entity::PLACEHOLDER` returns `true`, so filter_pairs allows all collisions — **providing no isolation at all** on the client side.

On the server, `MapInstanceId` values are local server entities and the comparison works correctly. So collision filtering works server-side but is useless client-side.

## Fatal Flaw 5: Room Management References Unmapped Entities

The proposed `on_map_instance_id_added` observer ([research:341-350](doc/research/2026-03-03-physics-isolation-avian-collision-hooks.md)):

```rust
fn on_map_instance_id_added(trigger: On<Add, MapInstanceId>, ...) {
    let map_id = map_ids.get(trigger.target()).unwrap();
    let room = map_rooms.get(map_id.0).unwrap().0; // map_id.0 is server entity
    ...
}
```

This is server-only code, so entity references work. However, the `MapRoom` component would need to be on the VoxelMapInstance entity, which means the room-entity association is server-only. This part is actually correct for server-side room management, but the research doesn't clearly distinguish which systems run where.

## Lightyear Entity Mapping Mechanics (Detailed)

Three `EntityMapper` implementations exist in lightyear at [entity_map.rs](git/lightyear/lightyear_serde/src/entity_map.rs):

| Mapper | Used For | On Unmapped Entity |
|--------|----------|-------------------|
| `SendEntityMap` | Serialization (sender side) | Returns original entity unchanged |
| `ReceiveEntityMap` | Deserialization (receiver side) | Returns `Entity::PLACEHOLDER` |
| `EntityMap` | Prediction/Interpolation mapping (Confirmed->Predicted) | Returns original entity unchanged |

Entity mapping flow for a component with `add_map_entities()`:
1. Server serializes: `SendEntityMap` checks `local_to_remote` map. If entity found, marks it with bit 62 and sends mapped entity. If not found, sends raw entity.
2. Client deserializes: `ReceiveEntityMap` checks if bit 62 is set (already mapped by sender). If not, looks up `remote_to_local`. If not found, returns `Entity::PLACEHOLDER`.

An entity enters `RemoteEntityMap` **only** when it is replicated (has `Replicate` component on server → client spawns corresponding local entity and records the mapping at [receive.rs:886](git/lightyear/lightyear_replication/src/receive.rs#L886)).

## What Actually Works in the Original Research

The following aspects are correct and should be preserved:

1. **CollisionHooks API analysis** (sections 4, 7): The `filter_pairs` mechanism, `ActiveCollisionHooks` opt-in, `SystemParam`-based hooks, and single-impl-per-app constraint are all accurately documented.

2. **SpatialQuery gap** (section 5): `filter_pairs` not affecting `SpatialQuery` is a real issue. The `cast_ray_predicate` solution is correct.

3. **RigidBodyDisabled during transition** (Follow-up section): Using `RigidBodyDisabled` to pause physics during transition is sound.

4. **SubState pattern** (Follow-up section): `MapTransitionState` as a `SubStates` of `ClientState::InGame` is a valid pattern.

5. **Lightyear Rooms analysis** (Follow-up section): The room API, `RoomEvent` usage, child visibility inheritance, and same-frame room transfer are all correctly researched.

6. **DisableRollback during transition**: Correct — prevents prediction from rolling back to pre-transition state.

## Correct Approaches for Map Instance Identity

The fundamental problem is: how does the client know which map an entity belongs to, when map entities aren't replicated?

### Option A: Semantic Enum Instead of Entity Reference

Replace `MapInstanceId(Entity)` with a serializable enum:

```rust
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
pub enum MapInstanceId {
    Overworld,
    Homebase { owner: ClientId },
    Arena { id: u32 },
}
```

- No `MapEntities` needed — no entity references cross the network
- Client resolves to its local map entity via a lookup resource: `HashMap<MapInstanceId, Entity>`
- Server does the same lookup for its own entities
- `filter_pairs` compares enum variants (works identically on server and client)
- Transition completion check: client looks up `MapInstanceId::Overworld` → client's local overworld entity → queries `VoxelMapInstance` on that entity

**Trade-off**: Requires a registry resource on both sides. Adding a new map type requires updating the enum. But enum variants are finite and known at compile time (Overworld, Homebase, Arena), so this is natural.

### Option B: Replicate the Map Instance Entity

Make VoxelMapInstance entities replicated so they enter lightyear's entity map:

- Add a lightweight `MapInstanceMarker` component that IS registered with lightyear and replicated
- The VoxelMapInstance entity gets `Replicate` + `MapInstanceMarker`
- `MapInstanceId(Entity)` then works because the entity has a valid server-to-client mapping
- `VoxelMapInstance` itself doesn't need to be replicated (it's not serializable), just the entity needs to exist in the entity map

**Trade-off**: Adds replication overhead for map entities. The client would have an entity with just `MapInstanceMarker` (no VoxelMapInstance data). Requires associating the replicated marker entity with the client's local VoxelMapInstance entity (a join step).

### Option C: Manual Entity Map Registration

Manually insert the server→client map entity correspondence into lightyear's `RemoteEntityMap` during connection setup. Both sides know they spawn an overworld, so the client can tell lightyear "server entity X = my local entity Y".

**Trade-off**: Fragile — requires coordinating entity creation order or explicit ID exchange. Not recommended.

### Recommendation: Option A

The semantic enum is the simplest, most robust approach. It completely avoids entity mapping issues, works identically on server and client, and naturally extends to the known map types (Overworld, Homebase, Arena). The transition completion check becomes:

```rust
fn check_map_transition_complete(
    map_registry: Res<MapRegistry>, // HashMap<MapInstanceId, Entity>
    maps: Query<(&VoxelMapInstance, &PendingChunks)>,
    player: Query<&MapInstanceId, With<Predicted>>,
    mut next_state: ResMut<NextState<MapTransitionState>>,
) {
    let Ok(map_id) = player.single() else { return };
    let Some(&map_entity) = map_registry.get(map_id) else { return };
    let Ok((instance, pending)) = maps.get(map_entity) else { return };
    if pending.tasks.is_empty() && instance.desired_chunks.is_subset(&instance.loaded_chunks) {
        next_state.set(MapTransitionState::Playing);
    }
}
```

## Additional Issues in the Research

### ChunkTarget Lacks add_prediction()

The research states at section 8 that `ChunkTarget` must be updated during transition:
> Update `ChunkTarget.map_entity` to new map entity

But `ChunkTarget` is registered without `.add_prediction()` ([lib.rs:167](crates/protocol/src/lib.rs#L167)). It only exists on the confirmed entity. If transition systems need to read `ChunkTarget` from the predicted entity, prediction must be enabled. However, `ChunkTarget` on the predicted player entity may not even be desirable — the client drives chunk loading from the camera, not from the player entity.

### desired_chunks Not Persisted

The research acknowledges this and proposes persisting `desired_chunks` on `VoxelMapInstance` (section "Chunk Loading Completion Detection"). This is correct but not yet implemented. The `VoxelMapInstance` struct at [instance.rs:26-32](crates/voxel_map_engine/src/instance.rs#L26-L32) has no `desired_chunks` field.

### Client Chunk Loading Is Camera-Driven

The research assumes the client loads chunks based on the player entity's `ChunkTarget`. In reality, the client attaches `ChunkTarget` to the **camera** ([client/map.rs:56-65](crates/client/src/map.rs#L56-L65)), not to the player entity. During a map transition, the camera's `ChunkTarget.map_entity` must be updated to point to the new client-local map entity. The research doesn't address camera ChunkTarget management at all.

### Client Needs to Spawn New Map Instances

For Homebase transitions, the client needs to spawn a new `VoxelMapInstance` entity locally. The research's transition sequence (section "Transition Sequence") only describes server-side entity updates. The client-side sequence must include:
1. Receive transition notification
2. Spawn a new local `VoxelMapInstance` for the target map (if not already spawned)
3. Register it in the `MapRegistry`
4. Update the camera's `ChunkTarget.map_entity` to the new local entity
5. Wait for chunks to load
6. Exit transitioning state

## Code References

- [client/map.rs:42-53](crates/client/src/map.rs#L42-L53) -- Client OverworldMap definition and spawning
- [client/map.rs:56-65](crates/client/src/map.rs#L56-L65) -- Camera ChunkTarget attachment (client chunk loading driver)
- [server/map.rs:19-31](crates/server/src/map.rs#L19-L31) -- Server OverworldMap definition and spawning
- [protocol/lib.rs:167](crates/protocol/src/lib.rs#L167) -- ChunkTarget registration (add_map_entities, no add_prediction)
- [voxel_map_engine/src/chunk.rs:19-22](crates/voxel_map_engine/src/chunk.rs#L19-L22) -- ChunkTarget MapEntities impl
- [voxel_map_engine/src/instance.rs:26-32](crates/voxel_map_engine/src/instance.rs#L26-L32) -- VoxelMapInstance definition (no desired_chunks)
- [git/lightyear/lightyear_serde/src/entity_map.rs:68-90](git/lightyear/lightyear_serde/src/entity_map.rs#L68-L90) -- ReceiveEntityMap returning Entity::PLACEHOLDER
- [git/lightyear/lightyear_serde/src/entity_map.rs:43-66](git/lightyear/lightyear_serde/src/entity_map.rs#L43-L66) -- SendEntityMap passing through unmapped entities
- [git/lightyear/lightyear_replication/src/receive.rs:886](git/lightyear/lightyear_replication/src/receive.rs#L886) -- Entity map insertion only for replicated entities

## Related Research

- [doc/research/2026-03-03-physics-isolation-avian-collision-hooks.md](doc/research/2026-03-03-physics-isolation-avian-collision-hooks.md) -- The research being critiqued
- [doc/plans/2026-02-28-voxel-map-engine.md](doc/plans/2026-02-28-voxel-map-engine.md) -- Original MapInstanceId proposal (same entity-reference flaw)

## Open Questions

1. **~~Should ChunkTarget.map_entity also use the semantic enum approach?~~** **Resolved**: No. `ChunkTarget` is local-only (not replicated). Each side creates it locally from `MapInstanceId` + `MapRegistry`. See Resolved Decision 5 in physics isolation research.

2. **~~How should the client learn about new maps?~~** **Resolved**: `MapTransitionStart` message includes `seed`, `generation_version`, and `bounds`. The server provides generation config at transition time; the client uses it to spawn a matching `VoxelMapInstance`. See revised `MapTransitionStart` in the physics isolation research.

3. **~~Should the camera ChunkTarget switch automatically during transitions?~~** **Resolved**: Per Resolved Decision 5, `ChunkTarget` moves from camera to player entity on both sides. The client observes `MapInstanceId` changes on the predicted entity and updates the player's local `ChunkTarget` accordingly. No camera ChunkTarget management needed.
