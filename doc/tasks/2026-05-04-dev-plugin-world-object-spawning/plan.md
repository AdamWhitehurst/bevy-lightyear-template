# Implementation Plan

## Overview

The def-driven world-object tab becomes a server-authoritative placement workflow: the client selects an object, previews a base-position target,
sends an ordered reliable placement request, and waits for normal Lightyear replication to create the committed object. Free-form spawning remains
explicitly client-local.

Implementation order is mandatory. Phase 1 fixes persistence semantics before any placed object can safely persist. Old per-chunk entity saves are not
migrated; delete existing save data before manual verification.

Before every `cargo check`, `cargo build`, or `cargo test`, check that no other cargo build/check/test is running. If one is running, wait for it to
finish or kill it before starting the next command.

## Phase 1: Explicit World-Object Position Semantics

### Changes

#### 1. World-object spawn storage boundary

**File**: `crates/voxel_map_engine/src/config.rs` **Action**: modify

Add explicit position semantics next to `WorldObjectSpawn`.

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldObjectPositionKind {
    /// `WorldObjectSpawn::position` is the terrain/base placement point;
    /// `PlacementOffset` still needs to be applied.
    #[default]
    PlacementBase,
    /// `WorldObjectSpawn::position` is the final world-space position;
    /// `PlacementOffset` has already been applied.
    Final,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldObjectSpawn {
    pub object_id: String,
    pub position: Vec3,
    #[serde(default)]
    pub position_kind: WorldObjectPositionKind,
    /// RON-serialized persisted components. Empty means no component snapshots;
    /// it is not a fresh-vs-reload signal.
    #[serde(default)]
    pub persisted_components: Vec<PersistedComponent>,
}
```

Do not make `persisted_components` imply fresh/reload state anywhere after this phase.

#### 2. Generated feature spawns

**File**: `crates/voxel_map_engine/src/terrain.rs` **Action**: modify

Update the existing `WorldObjectSpawn` literal in generated feature placement so generated/default spawns remain base-position spawns.

```rust
spawns.push(WorldObjectSpawn {
    object_id: rule.object_id.clone(),
    position: Vec3::new(world_pos.x, height as f32, world_pos.y),
    position_kind: WorldObjectPositionKind::PlacementBase,
    persisted_components: Vec::new(),
});
```

Add `WorldObjectPositionKind` to the local import where `WorldObjectSpawn` is currently imported.

#### 3. Per-chunk entity persistence tests

**File**: `crates/voxel_map_engine/src/persistence/mod.rs` **Action**: modify

Bump `ENTITY_SAVE_VERSION` from `2` to `3`. No legacy migration is required because old saves will be deleted before verification.

Update test helpers to include and assert `position_kind`.

```rust
use crate::config::WorldObjectPositionKind;

WorldObjectSpawn {
    object_id: "tree_oak".to_string(),
    position: Vec3::new(1.0, 2.0, 3.0),
    position_kind: WorldObjectPositionKind::Final,
    persisted_components: Vec::new(),
}
```

Add a focused regression test proving final-position semantics survive with no persisted component snapshots.

```rust
#[test]
fn chunk_entities_preserve_final_position_kind_without_persisted_components() {
    let dir = tempfile::tempdir().unwrap();
    let store = test_entity_store(dir.path());
    let pos = IVec3::new(4, 0, -2);
    let spawns = vec![WorldObjectSpawn {
        object_id: "tree_oak".to_string(),
        position: Vec3::new(1.0, 2.0, 3.0),
        position_kind: WorldObjectPositionKind::Final,
        persisted_components: Vec::new(),
    }];

    store.save(&pos, &spawns).unwrap();
    let loaded = store.load(&pos).unwrap().expect("entities should exist");

    assert_eq!(loaded[0].position_kind, WorldObjectPositionKind::Final);
    assert!(loaded[0].persisted_components.is_empty());
}
```

#### 4. Server chunk materialization and save semantics

**File**: `crates/server/src/chunk_entities.rs` **Action**: modify

Import the new enum.

```rust
use voxel_map_engine::config::{WorldObjectPositionKind, WorldObjectSpawn};
```

Replace reload inference from `persisted_components.is_empty()` with explicit position semantics.

```rust
let offset = extract_placement_offset(def, spawn.position_kind);
let entity = spawn_world_object(/* existing args */);
let position = Vec3::from(spawn.position) + offset;
commands.entity(entity).insert((
    Position(position.into()),
    ChunkEntityRef { chunk_pos, map_entity },
));

if !spawn.persisted_components.is_empty() {
    restore_persisted(/* existing args */);
}
```

Write all saved/evicted live entities as final-position spawns in both `evict_chunk_entities` and `collect_chunk_entities`.

```rust
WorldObjectSpawn {
    object_id: obj_id.0.clone(),
    position: Vec3::from(pos.0),
    position_kind: WorldObjectPositionKind::Final,
    persisted_components: persisted,
}
```

Replace `extract_placement_offset(def, is_reload: bool)` with explicit semantics.

```rust
fn extract_placement_offset(
    def: &protocol::world_object::WorldObjectDef,
    position_kind: WorldObjectPositionKind,
) -> Vec3 {
    match position_kind {
        WorldObjectPositionKind::Final => Vec3::ZERO,
        WorldObjectPositionKind::PlacementBase => def
            .components
            .iter()
            .find_map(|c| c.try_downcast_ref::<PlacementOffset>())
            .map(|offset| offset.0)
            .unwrap_or(Vec3::ZERO),
    }
}
```

Add a unit test for the helper.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bevy::reflect::PartialReflect;
    use protocol::world_object::WorldObjectDef;

    #[test]
    fn extract_placement_offset_uses_position_kind() {
        let offset = Vec3::new(0.0, 1.5, 0.0);
        let def = WorldObjectDef {
            components: vec![Box::new(PlacementOffset(offset)) as Box<dyn PartialReflect>],
        };

        assert_eq!(
            extract_placement_offset(&def, WorldObjectPositionKind::PlacementBase),
            offset
        );
        assert_eq!(
            extract_placement_offset(&def, WorldObjectPositionKind::Final),
            Vec3::ZERO
        );
    }
}
```

### Verification

#### Automated

- [x] `cargo test -p voxel_map_engine persistence::tests` passes
- [x] `cargo test -p server chunk_entities::tests::extract_placement_offset_uses_position_kind` passes
- [x] `cargo test -p server --test voxel_persistence` passes

#### Manual

- [ ] Delete old local save data before runtime verification because entity save v2 is intentionally not migrated.
- [ ] Fresh/generated `WorldObjectSpawn { position_kind: PlacementBase, ... }` inserts `Position(spawn.position + PlacementOffset)`.
- [ ] Saved/reloaded `WorldObjectSpawn { position_kind: Final, persisted_components: Vec::new(), ... }` inserts the exact saved final `Position` with
      no offset.
- [ ] Eviction and all-entity collection write `position_kind: Final` for saved chunk entities.
- [ ] `persisted_components` controls only component restoration, not position offset semantics.

---

## Phase 2: Authoritative Placement Protocol and Server Commit

### Changes

#### 1. Placement protocol types

**File**: `crates/protocol/src/world_object/types.rs` **Action**: modify

Add dedicated ordered reliable placement channel marker, request, ack, reject, and explicit reject reasons. The request must not include authoritative
map scope.

```rust
/// Ordered reliable channel for authoritative world-object placement requests and responses.
pub struct WorldObjectPlacementChannel;

/// Client requests placement of a known world-object definition.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::world_object"]
pub struct WorldObjectPlacementRequest {
    pub sequence: u32,
    pub object_id: WorldObjectId,
    /// Un-offset placement base point. Server applies `PlacementOffset` exactly once.
    pub base_position: Vec3,
}

/// Server acknowledges that placement was accepted. The committed object still arrives by replication.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::world_object"]
pub struct WorldObjectPlacementAck {
    pub sequence: u32,
    pub object_id: WorldObjectId,
    pub final_position: Vec3,
}

/// Server rejects a placement request without spawning an entity.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::world_object"]
pub struct WorldObjectPlacementReject {
    pub sequence: u32,
    pub reason: WorldObjectPlacementRejectReason,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[type_path = "protocol::world_object"]
pub enum WorldObjectPlacementRejectReason {
    NoControlledCharacter,
    UnknownObject,
    NonFinitePosition,
    OutOfBounds,
    ChunkUnavailable,
}
```

#### 2. World-object protocol re-exports

**File**: `crates/protocol/src/world_object/mod.rs` **Action**: modify

Add the new placement protocol symbols to the existing `pub use types::{ ... }` list.

```rust
pub use types::{
    ActiveTransformation, DeathEffect, ObjectCategory, OnDeathEffects, PlacementOffset,
    ReflectPersist, ReflectSpawnOnly, VisualKind, WorldObjectDef, WorldObjectId,
    WorldObjectLoadError, WorldObjectPlacementAck, WorldObjectPlacementChannel,
    WorldObjectPlacementReject, WorldObjectPlacementRejectReason, WorldObjectPlacementRequest,
};
```

#### 3. Protocol root registration

**File**: `crates/protocol/src/lib.rs` **Action**: modify

Expand root re-exports.

```rust
pub use world_object::{
    WorldObjectDefRegistry, WorldObjectId, WorldObjectPlacementAck,
    WorldObjectPlacementChannel, WorldObjectPlacementReject, WorldObjectPlacementRejectReason,
    WorldObjectPlacementRequest, WorldObjectPlugin,
};
```

Register the channel/messages in `ProtocolPlugin::build`, near other world-mutation channels.

```rust
// World object placement channel
app.add_channel::<WorldObjectPlacementChannel>(ChannelSettings {
    mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
    ..default()
})
.add_direction(NetworkDirection::Bidirectional);

// World object placement messages
app.register_message::<WorldObjectPlacementRequest>()
    .add_direction(NetworkDirection::ClientToServer);
app.register_message::<WorldObjectPlacementAck>()
    .add_direction(NetworkDirection::ServerToClient);
app.register_message::<WorldObjectPlacementReject>()
    .add_direction(NetworkDirection::ServerToClient);
```

#### 4. Server placed-object helper

**File**: `crates/server/src/world_object.rs` **Action**: modify

Add a helper that reuses `spawn_world_object`, applies `PlacementOffset` exactly once, and inserts normal chunk-persistence tags.

```rust
pub fn final_placed_world_object_position(def: &WorldObjectDef, base_position: Vec3) -> Vec3 {
    base_position + placement_offset(def)
}

fn placement_offset(def: &WorldObjectDef) -> Vec3 {
    def.components
        .iter()
        .find_map(|c| c.try_downcast_ref::<PlacementOffset>())
        .map(|offset| offset.0)
        .unwrap_or(Vec3::ZERO)
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_placed_world_object(
    commands: &mut Commands,
    object_id: WorldObjectId,
    def: &WorldObjectDef,
    base_position: Vec3,
    map_entity: Entity,
    map_id: MapInstanceId,
    chunk_size: u32,
    type_registry: &AppTypeRegistry,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    meshes: &Assets<Mesh>,
) -> Entity {
    let final_position = final_placed_world_object_position(def, base_position);
    debug_assert!(
        final_position.is_finite(),
        "world object final placement position must be finite after PlacementOffset"
    );

    let entity = spawn_world_object(
        commands,
        object_id,
        def,
        map_id,
        type_registry,
        vox_registry,
        vox_assets,
        meshes,
    );
    let chunk_pos = voxel_map_engine::lifecycle::world_to_chunk_pos(final_position, chunk_size);
    commands.entity(entity).insert((
        Position(final_position.into()),
        ChunkEntityRef {
            chunk_pos,
            map_entity,
        },
    ));
    entity
}
```

#### 5. Existing chunk entity persistence path

**File**: `crates/server/src/chunk_entities.rs` **Action**: modify

Do not add a one-object immediate save path. Ensure placed entities are persistable only by receiving `ChunkEntityRef`, `WorldObjectId`, and
`Position`, so existing eviction, periodic save, and shutdown save systems persist them.

If duplicate chunk-position logic appears, add only this helper and reuse it from server placement validation/spawning.

```rust
pub(crate) fn chunk_pos_for_world_position(position: Vec3, chunk_size: u32) -> IVec3 {
    voxel_map_engine::lifecycle::world_to_chunk_pos(position, chunk_size)
}
```

#### 6. Server placement request handling

**File**: `crates/server/src/map.rs` **Action**: modify

Make `resolve_player_map` visible inside the crate so placement can reuse the voxel authority boundary.

```rust
pub(crate) fn resolve_player_map(
    client_entity: Entity,
    controlled_query: &Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: &MapRegistry,
) -> Option<(Entity, MapInstanceId)> {
    let (_, player_map_id) = controlled_query
        .iter()
        .find(|(ctrl, _)| ctrl.owner == client_entity)?;
    Some((map_registry.get(player_map_id), player_map_id.clone()))
}
```

Add placement request handling following `handle_voxel_edit_requests`: receive per client, derive map scope from controlled character, validate,
spawn, and ack/reject. Every expected `continue`/`return` must have `trace!` first.

```rust
pub fn handle_world_object_placement_requests(
    mut receivers: Query<(Entity, &mut MessageReceiver<WorldObjectPlacementRequest>)>,
    mut ack_senders: Query<&mut MessageSender<WorldObjectPlacementAck>>,
    mut reject_senders: Query<&mut MessageSender<WorldObjectPlacementReject>>,
    controlled_query: Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: Res<MapRegistry>,
    map_query: Query<(&VoxelMapInstance, &MapDimensions)>,
    defs: Res<WorldObjectDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
    mut commands: Commands,
) {
    for (client_entity, mut receiver) in &mut receivers {
        for request in receiver.receive() {
            let Some((map_entity, map_id)) =
                resolve_player_map(client_entity, &controlled_query, &map_registry)
            else {
                trace!("handle_world_object_placement_requests: no character for client {client_entity:?}");
                send_placement_reject(
                    client_entity,
                    request.sequence,
                    WorldObjectPlacementRejectReason::NoControlledCharacter,
                    &mut reject_senders,
                );
                continue;
            };

            if !request.base_position.is_finite() {
                trace!("handle_world_object_placement_requests: non-finite base position");
                send_placement_reject(
                    client_entity,
                    request.sequence,
                    WorldObjectPlacementRejectReason::NonFinitePosition,
                    &mut reject_senders,
                );
                continue;
            }

            let Some(def) = defs.get(&request.object_id) else {
                trace!("handle_world_object_placement_requests: unknown object {}", request.object_id.0);
                send_placement_reject(
                    client_entity,
                    request.sequence,
                    WorldObjectPlacementRejectReason::UnknownObject,
                    &mut reject_senders,
                );
                continue;
            };

            let (instance, dimensions) = map_query
                .get(map_entity)
                .expect("resolved map entity must have VoxelMapInstance and MapDimensions");
            let final_position = crate::world_object::final_placed_world_object_position(
                def,
                request.base_position,
            );
            let chunk_pos = voxel_map_engine::lifecycle::world_to_chunk_pos(
                final_position,
                dimensions.chunk_size,
            );

            if !placement_chunk_in_bounds(chunk_pos, dimensions) {
                trace!("handle_world_object_placement_requests: chunk out of bounds {chunk_pos:?}");
                send_placement_reject(
                    client_entity,
                    request.sequence,
                    WorldObjectPlacementRejectReason::OutOfBounds,
                    &mut reject_senders,
                );
                continue;
            }

            let column = voxel_map_engine::prelude::chunk_to_column(chunk_pos);
            if !instance.chunk_levels.contains_key(&column)
                || instance.get_chunk_data(chunk_pos).is_none()
            {
                trace!("handle_world_object_placement_requests: chunk unavailable {chunk_pos:?}");
                send_placement_reject(
                    client_entity,
                    request.sequence,
                    WorldObjectPlacementRejectReason::ChunkUnavailable,
                    &mut reject_senders,
                );
                continue;
            }

            crate::world_object::spawn_placed_world_object(
                &mut commands,
                request.object_id.clone(),
                def,
                request.base_position,
                map_entity,
                map_id,
                dimensions.chunk_size,
                &type_registry,
                &vox_registry,
                &vox_assets,
                &meshes,
            );
            send_placement_ack(
                client_entity,
                WorldObjectPlacementAck {
                    sequence: request.sequence,
                    object_id: request.object_id,
                    final_position,
                },
                &mut ack_senders,
            );
        }
    }
}
```

Add helpers.

```rust
fn placement_chunk_in_bounds(chunk_pos: IVec3, dimensions: &MapDimensions) -> bool {
    (dimensions.column_y_range.0..dimensions.column_y_range.1).contains(&chunk_pos.y)
        && match dimensions.bounds {
        Some(bounds) => {
            chunk_pos.x.abs() < bounds.x
                && chunk_pos.y.abs() < bounds.y
                && chunk_pos.z.abs() < bounds.z
        }
        None => true,
    }
}

fn send_placement_reject(
    client_entity: Entity,
    sequence: u32,
    reason: WorldObjectPlacementRejectReason,
    reject_senders: &mut Query<&mut MessageSender<WorldObjectPlacementReject>>,
) {
    let Ok(mut sender) = reject_senders.get_mut(client_entity) else {
        trace!("send_placement_reject: no reject sender for {client_entity:?}");
        return;
    };
    sender.send::<WorldObjectPlacementChannel>(WorldObjectPlacementReject { sequence, reason });
}

fn send_placement_ack(
    client_entity: Entity,
    ack: WorldObjectPlacementAck,
    ack_senders: &mut Query<&mut MessageSender<WorldObjectPlacementAck>>,
) {
    let Ok(mut sender) = ack_senders.get_mut(client_entity) else {
        trace!("send_placement_ack: no ack sender for {client_entity:?}");
        return;
    };
    sender.send::<WorldObjectPlacementChannel>(ack);
}
```

Register `handle_world_object_placement_requests` in `ServerMapPlugin` `Update`, near the voxel request systems, gated on world-object/vox resources.

```rust
handle_world_object_placement_requests.run_if(
    resource_exists::<WorldObjectDefRegistry>()
        .and(resource_exists::<VoxModelRegistry>())
),
```

Do not mark `WorldDirtyState`; placed objects persist through chunk-entity tags.

#### 7. Server placement tests

**File**: `crates/server/tests/world_object_placement.rs` **Action**: create

Create direct server-side tests for the placement handler/helper. Use the existing integration-test style: `App::new()`, `MinimalPlugins`, direct
resource/component setup, and system execution.

Minimum scenarios:

```rust
#[test]
fn accepted_placement_spawns_replicated_chunk_entity() {
    // Arrange: loaded map entity with MapInstanceId::Overworld, MapDimensions,
    // VoxelMapInstance containing the target chunk, WorldObjectDefRegistry with PlacementOffset.
    // Arrange: client entity with message sender/receiver and controlled CharacterMarker on Overworld.

    // Act: send WorldObjectPlacementRequest { sequence: 7, object_id, base_position }.

    // Assert: exactly one entity has WorldObjectId, MapInstanceId, Replicate,
    // NetworkVisibility, Position(base + PlacementOffset), and ChunkEntityRef.
    // Assert: ack sequence is 7 and ack.final_position equals committed Position.
}

#[test]
fn rejected_placement_spawns_no_entity() {
    // Cover UnknownObject, NonFinitePosition, OutOfBounds, ChunkUnavailable,
    // and NoControlledCharacter. Each case asserts reject reason and zero new objects.
}
```

Use loaded test chunks by creating `VoxelMapInstance::new(...)`, inserting chunk data for the expected chunk, and inserting
`chunk_to_column(chunk_pos)` into `instance.chunk_levels`.

### Verification

#### Automated

- [ ] `cargo test -p server --test world_object_placement` passes
- [ ] `cargo test -p server --test voxel_persistence` passes after Phase 1 remains green

#### Manual

- [ ] Successful placement entity has `Replicate`, `NetworkVisibility`, `MapInstanceId`, `WorldObjectId`, `Position`, and `ChunkEntityRef`.
- [ ] Ack contains only correlation data and `final_position`; committed object appears through replication, not a placement broadcast.
- [ ] Every reject reason spawns no entity.
- [ ] Expected early-outs in new systems log `trace!` before `return`/`continue`.
- [ ] Unexpected missing resolved-map state uses `expect`, `panic!`, or `debug_assert!` rather than a silent reject.

---

## Phase 3: Dev Panel Placement Cutover

### Changes

#### 1. Expose spawn-panel state to the client crate

**File**: `crates/dev/src/lib.rs` **Action**: modify

Expose the panels module when the inspector feature is enabled so `client` can own placement input/preview systems while reading dev UI state.

```rust
#[cfg(feature = "inspector")]
pub mod panels;
```

#### 2. Spawn panel state and UI cutover

**File**: `crates/dev/src/panels/spawn.rs` **Action**: modify

Update module docs: def-driven placement is authoritative; free-form remains client-local.

Make UI state public enough for client systems, and add placement pending/reject state. Do not add Lightyear/client dependencies to `dev`.

```rust
#[derive(Resource, Default)]
pub struct SpawnPanelUi {
    tab: SpawnTab,
    pub selected_object: Option<WorldObjectId>,
    pub placement: WorldObjectPlacementUi,
    selected_freeform: Vec<String>,
}

#[derive(Default)]
pub struct WorldObjectPlacementUi {
    pub armed: bool,
    pub next_sequence: u32,
    pub pending: Vec<PendingWorldObjectPlacement>,
    pub last_reject: Option<WorldObjectPlacementRejectReason>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingWorldObjectPlacement {
    pub sequence: u32,
    pub object_id: WorldObjectId,
    pub base_position: Vec3,
    pub accepted_final_position: Option<Vec3>,
}

impl WorldObjectPlacementUi {
    pub fn next_sequence(&mut self) -> u32 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }
}
```

Remove `type_registry` and `commands` from `draw_def_tab`. Remove the old def-driven local
`commands.spawn((id, Transform::default(), DevSpawned, MapInstanceId::Overworld, ...))` block entirely.

```rust
fn draw_def_tab(
    ui: &mut egui::Ui,
    ui_state: &mut SpawnPanelUi,
    world_objects: Option<&WorldObjectDefRegistry>,
) {
    ui.label("World Object");
    if let Some(reg) = world_objects {
        // Keep existing sorted ComboBox selection.

        let has_selection = ui_state.selected_object.is_some();
        if ui
            .add_enabled(
                has_selection && !ui_state.placement.armed,
                egui::Button::new("Arm placement"),
            )
            .clicked()
        {
            ui_state.placement.armed = true;
            ui_state.placement.last_reject = None;
        }

        if ui_state.placement.armed && ui.button("Cancel placement").clicked() {
            ui_state.placement.armed = false;
        }

        ui.label(if ui_state.placement.armed {
            "Placement armed: click terrain to request server placement."
        } else {
            "Select an object and arm placement."
        });
        ui.label(format!("Pending placement requests: {}", ui_state.placement.pending.len()));
        if let Some(reason) = &ui_state.placement.last_reject {
            ui.label(format!("Last placement rejected: {reason:?}"));
        }
    } else {
        ui.label("(WorldObjectDefRegistry not yet loaded)");
    }
}
```

Keep `draw_freeform_tab` using `type_registry`, `commands`, `DevSpawned`, `MapInstanceId::Overworld`, and `apply_object_components`. Label the
free-form tab as client-local.

#### 3. Current-map placement targeting and click ownership

**File**: `crates/client/src/map.rs` **Action**: modify

Import spawn-panel state and placement protocol types under `#[cfg(feature = "spawn-panel")]`.

```rust
#[cfg(feature = "spawn-panel")]
use dev::panels::spawn::{PendingWorldObjectPlacement, SpawnPanelUi};
#[cfg(feature = "spawn-panel")]
use protocol::world_object::{
    WorldObjectPlacementAck, WorldObjectPlacementChannel, WorldObjectPlacementReject,
    WorldObjectPlacementRequest,
};
```

Add reusable target output near `camera_ray`.

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacementTarget {
    pub base_position: Vec3,
    pub hit_normal: IVec3,
}

pub fn current_placement_target(
    player_query: &Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    voxel_world: &mut VoxelWorld,
    camera_query: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: &Query<&Window, With<PrimaryWindow>>,
) -> Option<PlacementTarget> {
    let Ok(chunk_ticket) = player_query.single() else {
        trace!("current_placement_target: no predicted player with ChunkTicket");
        return None;
    };
    let Some(ray) = camera_ray(camera_query, window_query) else {
        trace!("current_placement_target: no camera ray");
        return None;
    };
    let Some(hit) = voxel_world.raycast(chunk_ticket.map_entity, ray, RAYCAST_MAX_DISTANCE, |v| {
        matches!(v, WorldVoxel::Solid(_))
    }) else {
        trace!("current_placement_target: raycast hit nothing");
        return None;
    };
    let Some(normal) = hit.normal else {
        trace!("current_placement_target: hit has no normal");
        return None;
    };
    let hit_normal = normal.as_ivec3();
    Some(PlacementTarget {
        base_position: (hit.position + hit_normal).as_vec3(),
        hit_normal,
    })
}
```

Modify `handle_voxel_input` to skip voxel placement/removal when object placement is armed.

```rust
// Optional because the spawn panel resource only exists when the dev spawn-panel feature/plugin is active;
// voxel editing must still compile and run without that plugin.
#[cfg(feature = "spawn-panel")]
placement_ui: Option<Res<SpawnPanelUi>>,
```

```rust
#[cfg(feature = "spawn-panel")]
if placement_ui.as_ref().is_some_and(|ui| ui.placement.armed) {
    trace!("handle_voxel_input: world object placement armed; skipping voxel input");
    return;
}
```

Add client-owned request sending. This is why client map/input code reads the UI state: it is the system that owns the left-click action and must
suppress voxel editing, compute the current-map target, and send the network request.

```rust
#[cfg(feature = "spawn-panel")]
fn handle_world_object_placement_input(
    mut ui_state: ResMut<SpawnPanelUi>,
    action_query: Query<&ActionState<PlayerActions>, With<Controlled>>,
    player_query: Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    mut voxel_world: VoxelWorld,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    mut message_sender: Query<&mut MessageSender<WorldObjectPlacementRequest>>,
) {
    if !ui_state.placement.armed {
        trace!("handle_world_object_placement_input: placement is not armed");
        return;
    }
    let Ok(action_state) = action_query.single() else {
        trace!("handle_world_object_placement_input: no entity with ActionState + Controlled");
        return;
    };
    if !action_state.just_pressed(&PlayerActions::PlaceVoxel) {
        trace!("handle_world_object_placement_input: place action not pressed");
        return;
    }
    let Some(object_id) = ui_state.selected_object.clone() else {
        trace!("handle_world_object_placement_input: placement armed without selected object");
        return;
    };
    let Some(target) = current_placement_target(
        &player_query,
        &mut voxel_world,
        &camera_query,
        &window_query,
    ) else {
        trace!("handle_world_object_placement_input: no placement target");
        return;
    };

    let sequence = ui_state.placement.next_sequence();
    let request = WorldObjectPlacementRequest {
        sequence,
        object_id: object_id.clone(),
        base_position: target.base_position,
    };

    let mut sent = false;
    for mut sender in message_sender.iter_mut() {
        sender.send::<WorldObjectPlacementChannel>(request.clone());
        sent = true;
    }
    if !sent {
        trace!("handle_world_object_placement_input: no WorldObjectPlacementRequest sender");
        return;
    }

    ui_state.placement.pending.push(PendingWorldObjectPlacement {
        sequence,
        object_id,
        base_position: target.base_position,
        accepted_final_position: None,
    });
}
```

Add ack/reject handlers that update pending UI only; they must not spawn committed objects locally.

```rust
#[cfg(feature = "spawn-panel")]
fn handle_world_object_placement_ack(
    mut receivers: Query<&mut MessageReceiver<WorldObjectPlacementAck>>,
    mut ui_state: ResMut<SpawnPanelUi>,
) {
    for mut receiver in &mut receivers {
        for ack in receiver.receive() {
            let Some(pending) = ui_state
                .placement
                .pending
                .iter_mut()
                .find(|pending| pending.sequence == ack.sequence)
            else {
                trace!("handle_world_object_placement_ack: ack seq={} had no pending placement", ack.sequence);
                continue;
            };
            pending.accepted_final_position = Some(ack.final_position);
            ui_state.placement.last_reject = None;
        }
    }
}

#[cfg(feature = "spawn-panel")]
fn handle_world_object_placement_reject(
    mut receivers: Query<&mut MessageReceiver<WorldObjectPlacementReject>>,
    mut ui_state: ResMut<SpawnPanelUi>,
) {
    for mut receiver in &mut receivers {
        for reject in receiver.receive() {
            trace!(
                "handle_world_object_placement_reject: reject seq={} reason={:?}",
                reject.sequence,
                reject.reason,
            );
            ui_state
                .placement
                .pending
                .retain(|pending| pending.sequence != reject.sequence);
            ui_state.placement.last_reject = Some(reject.reason);
        }
    }
}
```

Register systems in `ClientMapPlugin`. Keep object placement input after voxel input in a `.chain()` if voxel input contains the armed early-out.

```rust
.add_systems(
    Update,
    (
        handle_voxel_broadcasts,
        handle_section_blocks_update,
        handle_voxel_edit_ack,
        handle_voxel_edit_reject,
        #[cfg(feature = "spawn-panel")]
        handle_world_object_placement_ack,
        #[cfg(feature = "spawn-panel")]
        handle_world_object_placement_reject,
    )
        .run_if(in_state(ui::ClientState::InGame)),
)
.add_systems(
    PostUpdate,
    (
        handle_voxel_input,
        #[cfg(feature = "spawn-panel")]
        handle_world_object_placement_input,
    )
        .chain()
        .run_if(in_state(ui::ClientState::InGame))
        .after(TransformSystems::Propagate),
);
```

#### 4. Client public targeting re-export

**File**: `crates/client/src/lib.rs` **Action**: modify

Expose the reusable targeting surface.

```rust
pub use map::{current_placement_target, PlacementTarget};
```

#### 5. Client plugin tests

**File**: `crates/client/tests/plugin.rs` **Action**: modify

Add spawn-panel-gated tests for sequence/pending state. Keep these tests pure/lightweight; integration behavior is covered by manual verification and
Phase 2 server tests.

```rust
#[cfg(feature = "spawn-panel")]
#[test]
fn world_object_placement_ui_sequences_and_pending_ack() {
    use bevy::prelude::Vec3;
    use dev::panels::spawn::{PendingWorldObjectPlacement, WorldObjectPlacementUi};
    use protocol::world_object::WorldObjectId;

    let mut ui = WorldObjectPlacementUi::default();
    assert_eq!(ui.next_sequence(), 0);
    assert_eq!(ui.next_sequence(), 1);

    ui.pending.push(PendingWorldObjectPlacement {
        sequence: 1,
        object_id: WorldObjectId("test:crate".to_string()),
        base_position: Vec3::new(1.0, 2.0, 3.0),
        accepted_final_position: None,
    });
    ui.pending[0].accepted_final_position = Some(Vec3::new(1.0, 3.5, 3.0));
    assert_eq!(ui.pending[0].accepted_final_position, Some(Vec3::new(1.0, 3.5, 3.0)));
}
```

### Verification

#### Automated

- [ ] `cargo test -p client --test plugin` passes
- [ ] `cargo check -p client --features spawn-panel` passes

#### Manual

- [ ] Run `cargo server` and `cargo client`.
- [ ] Open the spawn panel, select a world object, arm placement, click terrain, and observe a placement request path rather than local entity
      creation.
- [ ] Confirm the final world object appears through replicated hydration, not from the dev panel's old local `commands.spawn` path.
- [ ] Confirm voxel placement/removal does not also fire on the same click while object placement is armed.
- [ ] Confirm free-form spawning still creates client-local `DevSpawned` entities only.
- [ ] Confirm a reject response removes the matching pending request and displays the reject reason in the panel.

---

## Phase 4: Visual-Only Preview and Replication Reconciliation

### Changes

#### 1. Visual-only helper for previews

**File**: `crates/client/src/world_object.rs` **Action**: modify

Add a local preview visual marker and helper that reuses visual construction without adding authoritative gameplay state. The helper must not call
`apply_object_components`, must not insert colliders, and must not insert `WorldObjectId`, `Position`, `MapInstanceId`, or `Replicated`.

```rust
/// Marker for visual children attached to local-only placement previews.
#[derive(Component)]
pub struct PlacementPreviewVisual;

pub fn preview_visual_from_def(
    commands: &mut Commands,
    parent: Entity,
    def: &WorldObjectDef,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    default_material: &DefaultVoxModelMaterial,
) -> Option<Entity> {
    let visual_kind = def
        .components
        .iter()
        .find_map(|c| c.try_downcast_ref::<VisualKind>());
    match visual_kind {
        Some(VisualKind::Vox(path)) => preview_vox_mesh(
            commands,
            parent,
            path,
            vox_registry,
            vox_assets,
            default_material,
        ),
        _ => {
            trace!("preview_visual_from_def: world object has no Vox visual, skipping preview visual");
            None
        }
    }
}

fn preview_vox_mesh(
    commands: &mut Commands,
    parent: Entity,
    vox_path: &str,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    default_material: &DefaultVoxModelMaterial,
) -> Option<Entity> {
    let Some(asset_handle) = vox_registry.get(vox_path) else {
        trace!("preview_vox_mesh: Vox model not found in registry: {vox_path}");
        return None;
    };
    let Some(asset) = vox_assets.get(asset_handle) else {
        trace!("preview_vox_mesh: VoxModelAsset not yet loaded: {vox_path}");
        return None;
    };
    let Some(mesh_handle) = asset.lod_meshes.first() else {
        trace!("preview_vox_mesh: VoxModelAsset has no LOD meshes: {vox_path}");
        return None;
    };

    let child = commands
        .spawn((
            Mesh3d(mesh_handle.clone()),
            MeshMaterial3d(default_material.0.clone()),
            PlacementPreviewVisual,
        ))
        .id();
    commands.entity(parent).insert(Visibility::default()).add_child(child);
    Some(child)
}
```

#### 2. Preview state and systems in client-owned map/input code

**File**: `crates/client/src/map.rs` **Action**: modify

Add the preview marker in client code, not in `dev`, to avoid a `client` -> `dev` -> `client` dependency cycle.

```rust
#[cfg(feature = "spawn-panel")]
#[derive(Component)]
struct WorldObjectPlacementPreview {
    sequence: Option<u32>,
    object_id: WorldObjectId,
}
```

Add preview transform helper. This applies `PlacementOffset` for display only; requests still send the un-offset `base_position`.

```rust
#[cfg(feature = "spawn-panel")]
fn preview_transform(def: &WorldObjectDef, base_position: Vec3) -> Transform {
    let offset = def
        .components
        .iter()
        .find_map(|c| c.try_downcast_ref::<PlacementOffset>())
        .map(|offset| offset.0)
        .unwrap_or(Vec3::ZERO);
    Transform::from_translation(base_position + offset)
}
```

Add hover/pending preview maintenance. The hover preview has `sequence: None`; after click, reuse/convert it to `Some(sequence)` or spawn a sequence
preview at the accepted base position.

```rust
#[cfg(feature = "spawn-panel")]
fn update_world_object_placement_preview(
    mut commands: Commands,
    ui_state: Res<SpawnPanelUi>,
    registry: Option<Res<WorldObjectDefRegistry>>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    default_material: Res<DefaultVoxModelMaterial>,
    player_query: Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    mut voxel_world: VoxelWorld,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    mut preview_query: Query<(Entity, &mut Transform, &WorldObjectPlacementPreview)>,
) {
    // Despawn hover preview when not armed or no selection.
    // Create/move a single hover preview for selected_object + current_placement_target.
    // Create/update sequence previews for pending requests.
    // Use `preview_visual_from_def`; never use `apply_object_components`.
}
```

Implementation requirements for `update_world_object_placement_preview`:

- If placement is not armed and no pending previews remain, despawn hover previews with `trace!` before expected cleanup returns.
- If armed but `WorldObjectDefRegistry` is missing, `selected_object` is `None`, or the selected object id is unknown, `trace!` and skip.
- If `current_placement_target(...)` returns `None`, `trace!` and remove/move no hover preview.
- Ensure there is at most one hover preview (`sequence: None`) for the selected object.
- Sequence previews must use `PendingWorldObjectPlacement.accepted_final_position` when present; otherwise use
  `preview_transform(def, pending.base_position)`.
- Reject handling from Phase 3 removes pending records; this system must despawn sequence previews with no matching pending record.
- Reconciled accepted placements must be removed from `SpawnPanelUi.pending` so preview maintenance cannot recreate their previews after the matching
  replicated object appears.

Add reconciliation after replicated world-object hydration. Match accepted previews by `object_id` and accepted final position; despawn only the
preview entity and remove the matching pending record.

```rust
#[cfg(feature = "spawn-panel")]
fn reconcile_placement_preview_on_replication(
    mut commands: Commands,
    mut ui_state: ResMut<SpawnPanelUi>,
    replicated_query: Query<(&WorldObjectId, &Position), Added<Replicated>>,
    preview_query: Query<(Entity, &WorldObjectPlacementPreview, &Transform)>,
) {
    for (replicated_id, replicated_position) in &replicated_query {
        let replicated_position = Vec3::from(replicated_position.0);
        for (preview_entity, preview, preview_transform) in &preview_query {
            let Some(sequence) = preview.sequence else {
                trace!("reconcile_placement_preview_on_replication: skipping hover preview");
                continue;
            };
            if &preview.object_id != replicated_id {
                trace!("reconcile_placement_preview_on_replication: preview object id does not match replicated object");
                continue;
            }
            if positions_match(preview_transform.translation, replicated_position) {
                commands.entity(preview_entity).despawn();
                ui_state.placement.pending.retain(|pending| pending.sequence != sequence);
            }
        }
    }
}

#[cfg(feature = "spawn-panel")]
fn positions_match(a: Vec3, b: Vec3) -> bool {
    a.distance_squared(b) <= 0.01 * 0.01
}
```

If child preview visuals survive parent despawn in the current Bevy hierarchy behavior, replace `despawn()` with the project's recursive despawn
convention.

Register systems in `ClientMapPlugin`:

- `update_world_object_placement_preview` in `PostUpdate` after placement input and after `TransformSystems::Propagate`.
- `reconcile_placement_preview_on_replication` after `client::world_object::on_world_object_replicated` has observed `Added<Replicated>` for the
  object. If direct ordering across modules is not available, register in `PostUpdate` and rely on matching the same `Added<Replicated>` frame after
  hydration systems; if this is not stable, add an explicit system set in `client` and order both systems in that set.

#### 3. Preview UI state use only

**File**: `crates/dev/src/panels/spawn.rs` **Action**: modify

Keep this file UI/state-only for Phase 4. Do not call `client::map` or `client::world_object` from `dev`.

Update the panel text to show accepted previews waiting for replication.

```rust
let accepted = ui_state
    .placement
    .pending
    .iter()
    .filter(|pending| pending.accepted_final_position.is_some())
    .count();
ui.label(format!("Accepted placements awaiting replication: {accepted}"));
```

#### 4. Client plugin tests for preview state

**File**: `crates/client/tests/plugin.rs` **Action**: modify

Add tests that exercise preview safety and reconciliation helpers. Keep direct ECS assertions small.

```rust
#[cfg(feature = "spawn-panel")]
#[test]
fn placement_preview_entities_are_visual_only() {
    // Spawn a preview entity through the preview helper/system.
    // Assert the preview entity has WorldObjectPlacementPreview + Transform only.
    // Assert it does not have Collider, Position, MapInstanceId, Replicated, or authoritative WorldObjectId.
}

#[cfg(feature = "spawn-panel")]
#[test]
fn replicated_object_reconciles_matching_preview_only() {
    // Create one accepted preview matching object id + position and one non-matching preview.
    // Run reconciliation.
    // Assert matching preview despawned and non-matching preview remains.
}
```

### Verification

#### Automated

- [ ] `cargo test -p client --test plugin` passes
- [ ] `cargo check -p client --features spawn-panel` passes

#### Manual

- [ ] Run `cargo server` and `cargo client`.
- [ ] Select a world object, arm placement, and confirm the preview follows terrain under the mouse.
- [ ] Click terrain and confirm there is at most one temporary accepted preview.
- [ ] Force or trigger a reject and confirm the matching preview is removed and the reject reason is shown.
- [ ] Confirm replicated hydration replaces the accepted preview without duplicate committed visuals.
- [ ] Confirm the preview has no gameplay collider/effects and cannot be interacted with as a real world object before replication.

---

## Final Verification and Documentation Check

### Automated

- [ ] `cargo check-all` passes
- [ ] `cargo test-all` passes

### Manual

- [ ] Review `README.md`; if it documents dev spawn behavior, update it to describe authoritative def-driven placement and client-local free-form
      spawning. If it does not document this area, leave it unchanged.
- [ ] Full runtime scenario: delete old saves, run `cargo server`, run `cargo client`, place a def-driven world object, restart server/client, and
      confirm the object reloads at the same final position without double-applying `PlacementOffset`.
- [ ] Confirm free-form spawn remains a dev-only client-local scratchpad and is clearly labeled that way.
