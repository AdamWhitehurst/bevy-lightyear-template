# Implementation Plan

## Overview

Developers can use the spawn-panel dev tooling at runtime to select existing replicated world objects, preview transform
edits, submit move/rotate/delete requests, and have the server authoritatively mutate, replicate, and queue chunk
persistence. Move, rotation, and deletion must survive chunk unload/reload, including same-chunk edits, durable empty
chunks, and cross-chunk moves.

## Global Implementation Rules

- Follow the phase order exactly.
- Before each cargo command, confirm no other cargo build/check/test is running:

```bash
pgrep -af 'cargo (build|check|test|make|server|client)|rustc|rustdoc' || true
```

- If the check prints an active build/check/test process, wait for it to finish or kill it before running the next cargo
  command.
- Use existing cargo aliases where possible: `cargo check-all`, `cargo test-all`, `cargo server`, `cargo client`.
- Every expected early return added to ECS systems must include `trace!`; unexpected invalid state should use `expect`,
  `debug_assert!`, or `panic!`.
- Keep client previews local-only: no `Position`, `MapInstanceId`, `Replicated`, `Collider`, or authoritative
  `WorldObjectId` component on preview entities.
- Server ack means “ECS mutation applied and persistence save queued”; do not block on filesystem flush.

---

## Phase 1: Select and Delete Loaded Objects

### Changes

#### 1. Protocol world-object edit/delete messages

**File**: `crates/protocol/src/world_object/types.rs`  
**Action**: modify

Add `MapEntities` imports and a new ordered reliable edit channel plus delete request/ack/reject types. Keep placement
types unchanged.

```rust
use bevy::ecs::entity::{EntityMapper, MapEntities};
```

```rust
/// Ordered reliable channel for authoritative world-object edit and delete requests.
pub struct WorldObjectEditChannel;

/// Client requests deletion of an existing replicated world-object entity.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::world_object"]
pub struct WorldObjectDeleteRequest {
    pub sequence: u32,
    pub target: Entity,
}

impl MapEntities for WorldObjectDeleteRequest {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.target = entity_mapper.get_mapped(self.target);
    }
}

/// Server acknowledges that deletion was applied. The despawn arrives by replication.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::world_object"]
pub struct WorldObjectDeleteAck {
    pub sequence: u32,
    pub target: Entity,
}

impl MapEntities for WorldObjectDeleteAck {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.target = entity_mapper.get_mapped(self.target);
    }
}

/// Server rejects a world-object edit/delete request.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::world_object"]
pub struct WorldObjectEditReject {
    pub sequence: u32,
    pub reason: WorldObjectEditRejectReason,
}

/// Explicit reasons that a world-object edit/delete request can be rejected.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[type_path = "protocol::world_object"]
pub enum WorldObjectEditRejectReason {
    NoControlledCharacter,
    TargetNotMapped,
    MissingTarget,
    NotWorldObject,
    ForeignMap,
    ChunkUnavailable,
}
```

Use `TargetNotMapped` when Lightyear leaves a request target as an unmapped placeholder or when a mapping failure is
otherwise observable before entity lookup; otherwise use `MissingTarget` for entities that do not exist in the server
world.

#### 2. Register edit channel and mapped messages

**File**: `crates/protocol/src/lib.rs`  
**Action**: modify

Import the new message types via the existing `world_object::*` import path if already globbed; otherwise add explicit
imports.

After world-object placement message registration, add:

```rust
// World object edit channel
app.add_channel::<WorldObjectEditChannel>(ChannelSettings {
    mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
    ..default()
})
.add_direction(NetworkDirection::Bidirectional);

// World object edit/delete messages
app.register_message::<WorldObjectDeleteRequest>()
    .add_direction(NetworkDirection::ClientToServer)
    .add_map_entities();
app.register_message::<WorldObjectDeleteAck>()
    .add_direction(NetworkDirection::ServerToClient)
    .add_map_entities();
app.register_message::<WorldObjectEditReject>()
    .add_direction(NetworkDirection::ServerToClient);
```

#### 3. Spawn-panel selection/delete UI state

**File**: `crates/dev/src/panels/spawn.rs`  
**Action**: modify

Add edit selection state to `SpawnPanelUi`, keeping existing placement/free-form fields.

```rust
#[derive(Resource, Default)]
pub struct SpawnPanelUi {
    tab: SpawnTab,
    pub selected_object: Option<WorldObjectId>,
    pub placement: WorldObjectPlacementUi,
    pub selection: WorldObjectSelectionUi,
    selected_freeform: Vec<String>,
}

/// Client-owned world-object selection and edit request state shown by the spawn panel.
#[derive(Default)]
pub struct WorldObjectSelectionUi {
    pub selected: Option<Entity>,
    pub nearby_radius: f32,
    pub next_sequence: u32,
    pub pending_deletes: Vec<PendingWorldObjectDelete>,
    pub last_reject: Option<WorldObjectEditRejectReason>,
}

/// A pending authoritative world-object delete request.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingWorldObjectDelete {
    pub sequence: u32,
    pub target: Entity,
    pub accepted: bool,
}

impl WorldObjectSelectionUi {
    pub fn next_sequence(&mut self) -> u32 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }
}
```

Implement `Default` manually if `nearby_radius` should be non-zero:

```rust
impl Default for WorldObjectSelectionUi {
    fn default() -> Self {
        Self {
            selected: None,
            nearby_radius: 12.0,
            next_sequence: 0,
            pending_deletes: Vec::new(),
            last_reject: None,
        }
    }
}
```

Extend `draw_def_tab` with an “Existing objects” subsection. The actual nearby list is populated by client systems in
`crates/client/src/map.rs`; this panel only renders selected/pending state in Phase 1.

```rust
ui.separator();
ui.label("Existing World Object");
ui.label(match ui_state.selection.selected {
    Some(entity) => format!("Selected: {entity:?}"),
    None => "Selected: (none)".to_string(),
});
ui.add(egui::Slider::new(&mut ui_state.selection.nearby_radius, 1.0..=64.0).text("Nearby radius"));
ui.label(format!(
    "Pending delete requests: {}",
    ui_state.selection.pending_deletes.len()
));
if let Some(reason) = &ui_state.selection.last_reject {
    ui.label(format!("Last edit rejected: {reason:?}"));
}
```

The delete button should be added after client code exposes nearby selection/send intent in Phase 1; it must be disabled
when `selection.selected.is_none()`.

#### 4. Client selection, nearby list, delete send, ack/reject handling

**File**: `crates/client/src/map.rs`  
**Action**: modify

Update imports under `#[cfg(feature = "spawn-panel")]`:

```rust
use dev::panels::spawn::{
    PendingWorldObjectDelete, PendingWorldObjectPlacement, SpawnPanelUi,
};
use protocol::world_object::{
    WorldObjectDeleteAck, WorldObjectDeleteRequest, WorldObjectEditChannel,
    WorldObjectEditReject, /* existing placement imports */
};
```

Add a nearby-list helper and selection/send systems. Nearby selection uses replicated world-object entities in the
controlled predicted player’s map. Use `Position` as the distance source; do not pick local previews.

```rust
#[cfg(feature = "spawn-panel")]
pub fn nearest_world_object_in_radius(
    origin: Vec3,
    radius: f32,
    objects: &Query<(Entity, &Position, Option<&MapInstanceId>), (With<WorldObjectId>, With<Replicated>)>,
    current_map: Option<&MapInstanceId>,
) -> Option<Entity> {
    let radius_sq = radius * radius;
    objects
        .iter()
        .filter(|(_, _, object_map)| match (current_map, object_map) {
            (Some(current), Some(object_map)) => *object_map == current,
            _ => true,
        })
        .filter_map(|(entity, position, _)| {
            let dist_sq = Vec3::from(position.0).distance_squared(origin);
            (dist_sq <= radius_sq).then_some((entity, dist_sq))
        })
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(entity, _)| entity)
}
```

Add `update_world_object_nearby_selection` to select the nearest object when there is no selected entity or the selected
entity despawned. It should `trace!` and return for missing player/map state.

```rust
#[cfg(feature = "spawn-panel")]
fn update_world_object_nearby_selection(
    mut ui_state: ResMut<SpawnPanelUi>,
    player_query: Query<(&Position, &MapInstanceId), (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    object_query: Query<(Entity, &Position, Option<&MapInstanceId>), (With<WorldObjectId>, With<Replicated>)>,
) {
    let Ok((player_position, player_map)) = player_query.single() else {
        trace!("update_world_object_nearby_selection: no predicted controlled player position");
        return;
    };
    if ui_state
        .selection
        .selected
        .is_some_and(|entity| object_query.get(entity).is_ok())
    {
        return;
    }
    ui_state.selection.selected = nearest_world_object_in_radius(
        Vec3::from(player_position.0),
        ui_state.selection.nearby_radius,
        &object_query,
        Some(player_map),
    );
}
```

Add delete request input. To keep Phase 1 small, use `Delete` key while an entity is selected, and in the panel render a
“Press Delete to delete selected” label. If implementing an egui button, set a `delete_requested` bool in
`WorldObjectSelectionUi` and consume it here.

```rust
#[cfg(feature = "spawn-panel")]
fn handle_world_object_delete_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut ui_state: ResMut<SpawnPanelUi>,
    mut message_sender: Query<&mut MessageSender<WorldObjectDeleteRequest>>,
) {
    if !keys.just_pressed(KeyCode::Delete) {
        return;
    }
    let Some(target) = ui_state.selection.selected else {
        trace!("handle_world_object_delete_input: no selected world object");
        return;
    };
    let sequence = ui_state.selection.next_sequence();
    let request = WorldObjectDeleteRequest { sequence, target };

    let mut sent = false;
    for mut sender in &mut message_sender {
        sender.send::<WorldObjectEditChannel>(request.clone());
        sent = true;
    }
    if !sent {
        trace!("handle_world_object_delete_input: no WorldObjectDeleteRequest sender");
        return;
    }

    ui_state.selection.pending_deletes.push(PendingWorldObjectDelete {
        sequence,
        target,
        accepted: false,
    });
}
```

Add ack/reject handlers:

```rust
#[cfg(feature = "spawn-panel")]
fn handle_world_object_delete_ack(
    mut receivers: Query<&mut MessageReceiver<WorldObjectDeleteAck>>,
    mut ui_state: ResMut<SpawnPanelUi>,
) {
    for mut receiver in &mut receivers {
        for ack in receiver.receive() {
            if let Some(pending) = ui_state
                .selection
                .pending_deletes
                .iter_mut()
                .find(|pending| pending.sequence == ack.sequence)
            {
                pending.accepted = true;
                ui_state.selection.last_reject = None;
            } else {
                trace!("handle_world_object_delete_ack: ack seq={} had no pending delete", ack.sequence);
            }
        }
    }
}

#[cfg(feature = "spawn-panel")]
fn handle_world_object_edit_reject(
    mut receivers: Query<&mut MessageReceiver<WorldObjectEditReject>>,
    mut ui_state: ResMut<SpawnPanelUi>,
) {
    for mut receiver in &mut receivers {
        for reject in receiver.receive() {
            ui_state
                .selection
                .pending_deletes
                .retain(|pending| pending.sequence != reject.sequence);
            ui_state.selection.last_reject = Some(reject.reason);
        }
    }
}
```

Register systems in `ClientMapPlugin`:

- `Update`: add `handle_world_object_delete_ack` and `handle_world_object_edit_reject` next to placement ack/reject
  handlers.
- `PostUpdate` chain: add `update_world_object_nearby_selection` before `handle_world_object_delete_input`; add
  `handle_world_object_delete_input` before preview updates.

Also update `handle_voxel_input` so voxel input early-outs when placement is armed or an edit/delete operation is
active:

```rust
#[cfg(feature = "spawn-panel")]
if placement_ui.as_ref().is_some_and(|ui| {
    ui.placement.armed || ui.selection.selected.is_some()
}) {
    trace!("handle_voxel_input: world object edit/placement active; skipping voxel input");
    return;
}
```

#### 5. Server delete validation and request handling

**File**: `crates/server/src/map.rs`  
**Action**: modify

Update imports for new protocol messages and `ChunkEntityRef` if not already imported in this scope.

Register `handle_world_object_delete_requests` in `ServerMapPlugin::build` next to placement request handling, gated on
`WorldObjectDefRegistry` and `VoxModelRegistry` only if needed for shared query signatures; deletion itself only needs
map/chunk/store resources.

```rust
handle_world_object_delete_requests,
```

Define validation data:

```rust
pub struct ValidatedWorldObjectDelete {
    pub map_entity: Entity,
    pub chunk_pos: IVec3,
}
```

Implement validation:

```rust
pub fn validate_world_object_delete(
    target: Entity,
    player_map_entity: Entity,
    player_map_id: &MapInstanceId,
    object_query: &Query<(&WorldObjectId, &MapInstanceId, &ChunkEntityRef)>,
    map_query: &Query<&VoxelMapInstance>,
) -> Result<ValidatedWorldObjectDelete, WorldObjectEditRejectReason> {
    let Ok((_id, object_map_id, chunk_ref)) = object_query.get(target) else {
        return Err(WorldObjectEditRejectReason::MissingTarget);
    };
    if object_map_id != player_map_id || chunk_ref.map_entity != player_map_entity {
        return Err(WorldObjectEditRejectReason::ForeignMap);
    }
    let instance = map_query
        .get(player_map_entity)
        .expect("resolved map entity must have VoxelMapInstance");
    let column = voxel_map_engine::prelude::chunk_to_column(chunk_ref.chunk_pos);
    if !instance.chunk_levels.contains_key(&column) || instance.get_chunk_data(chunk_ref.chunk_pos).is_none() {
        return Err(WorldObjectEditRejectReason::ChunkUnavailable);
    }
    Ok(ValidatedWorldObjectDelete {
        map_entity: player_map_entity,
        chunk_pos: chunk_ref.chunk_pos,
    })
}
```

If the target exists but lacks `WorldObjectId`, `MapInstanceId`, or `ChunkEntityRef`, return `NotWorldObject`. Implement
this with a broader `target_exists: Query<Entity>` check before the typed object query:

```rust
if target_exists.get(target).is_ok() && object_query.get(target).is_err() {
    return Err(WorldObjectEditRejectReason::NotWorldObject);
}
```

Add request system:

```rust
#[allow(clippy::too_many_arguments)]
pub fn handle_world_object_delete_requests(
    mut receivers: Query<(Entity, &mut MessageReceiver<WorldObjectDeleteRequest>)>,
    mut ack_senders: Query<&mut MessageSender<WorldObjectDeleteAck>>,
    mut reject_senders: Query<&mut MessageSender<WorldObjectEditReject>>,
    controlled_query: Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: Res<MapRegistry>,
    map_query: Query<&VoxelMapInstance>,
    target_exists: Query<Entity>,
    object_query: Query<(&WorldObjectId, &MapInstanceId, &ChunkEntityRef)>,
    entity_save_query: Query<(&ChunkEntityRef, &WorldObjectId, &Position, Option<&ActiveTransformation>, Option<&protocol::Health>)>,
    mut store_query: Query<(&StoreBackend<IVec3, Vec<WorldObjectSpawn>, FsChunkEntitiesStore>, &mut PendingStoreOps<IVec3, Vec<WorldObjectSpawn>>)>,
    mut commands: Commands,
) {
    for (client_entity, mut receiver) in &mut receivers {
        for request in receiver.receive() {
            let Some((map_entity, map_id)) = resolve_player_map(client_entity, &controlled_query, &map_registry) else {
                send_world_object_edit_reject(client_entity, request.sequence, WorldObjectEditRejectReason::NoControlledCharacter, &mut reject_senders);
                continue;
            };
            let result = validate_world_object_delete(request.target, map_entity, &map_id, &object_query, &map_query);
            let Ok(validated) = result else {
                send_world_object_edit_reject(client_entity, request.sequence, result.unwrap_err(), &mut reject_senders);
                continue;
            };

            commands.entity(request.target).despawn();
            crate::chunk_entities::save_chunk_entities_now_or_queue(
                validated.map_entity,
                validated.chunk_pos,
                Some(request.target),
                &entity_save_query,
                &mut store_query,
            );
            send_world_object_delete_ack(client_entity, WorldObjectDeleteAck { sequence: request.sequence, target: request.target }, &mut ack_senders);
        }
    }
}
```

Because `commands.entity(target).despawn()` is deferred, pass `Some(deleted_entity)` to the persistence helper so the
just-deleted entity is excluded from the saved chunk snapshot immediately.

Add send helpers:

```rust
fn send_world_object_edit_reject(
    client_entity: Entity,
    sequence: u32,
    reason: WorldObjectEditRejectReason,
    reject_senders: &mut Query<&mut MessageSender<WorldObjectEditReject>>,
) { /* same shape as send_placement_reject, using WorldObjectEditChannel */ }

fn send_world_object_delete_ack(
    client_entity: Entity,
    ack: WorldObjectDeleteAck,
    ack_senders: &mut Query<&mut MessageSender<WorldObjectDeleteAck>>,
) { /* same shape as send_placement_ack, using WorldObjectEditChannel */ }
```

#### 6. Immediate chunk entity save helper, including empty chunks

**File**: `crates/server/src/chunk_entities.rs`  
**Action**: modify

Make collection helpers reusable and add an explicit chunk-save helper that can exclude a pending-deleted entity.
Include doc comments.

```rust
pub type ChunkEntitySaveQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static ChunkEntityRef,
        &'static WorldObjectId,
        &'static Position,
        Option<&'static ActiveTransformation>,
        Option<&'static protocol::Health>,
    ),
>;
```

If the alias is awkward with Bevy lifetimes, keep the explicit query type at call sites.

Add helper:

```rust
/// Queues an immediate save for one loaded chunk, writing an empty entity file when no objects remain.
pub fn save_chunk_entities_now_or_queue(
    map_entity: Entity,
    chunk_pos: IVec3,
    exclude_entity: Option<Entity>,
    entity_query: &Query<(
        Entity,
        &ChunkEntityRef,
        &WorldObjectId,
        &Position,
        Option<&ActiveTransformation>,
        Option<&protocol::Health>,
    )>,
    store_query: &mut Query<(
        &StoreBackend<IVec3, Vec<WorldObjectSpawn>, FsChunkEntitiesStore>,
        &mut PendingStoreOps<IVec3, Vec<WorldObjectSpawn>>,
    )>,
) {
    let spawns = collect_chunk_entity_spawns(map_entity, chunk_pos, exclude_entity, entity_query);
    let Ok((store, mut ops)) = store_query.get_mut(map_entity) else {
        trace!("save_chunk_entities_now_or_queue: map entity {map_entity:?} has no chunk entity store");
        return;
    };
    ops.spawn_save(&store.0, chunk_pos, spawns);
}
```

Add reusable collection for a single chunk:

```rust
pub fn collect_chunk_entity_spawns(
    map_entity: Entity,
    chunk_pos: IVec3,
    exclude_entity: Option<Entity>,
    entity_query: &Query<(
        Entity,
        &ChunkEntityRef,
        &WorldObjectId,
        &Position,
        Option<&ActiveTransformation>,
        Option<&protocol::Health>,
    )>,
) -> Vec<WorldObjectSpawn> {
    entity_query
        .iter()
        .filter(|(entity, chunk_ref, _, _, _, _)| {
            Some(*entity) != exclude_entity
                && chunk_ref.map_entity == map_entity
                && chunk_ref.chunk_pos == chunk_pos
        })
        .map(|(_, _, obj_id, pos, active_transform, health)| WorldObjectSpawn {
            object_id: obj_id.0.clone(),
            position: pos.0,
            position_kind: WorldObjectPositionKind::Final,
            persisted_components: serialize_persisted(active_transform, health),
        })
        .collect()
}
```

Change existing `collect_chunk_entities`, `evict_chunk_entities`, `save_chunk_entities_periodic`, and
`save_all_chunk_entities_on_exit` query shapes to include `Entity` only where needed; do not change semantics except to
reuse `collect_chunk_entity_spawns` where helpful.

#### 7. Preserve missing-vs-empty entity files during feature generation

**File**: `crates/voxel_map_engine/src/persistence/mod.rs`  
**Action**: modify

No schema bump is required if `EntityFileEnvelope` bytes are unchanged. Add/keep tests proving empty saved entity files
are distinguishable from missing files.

```rust
#[test]
fn empty_entities_file_is_authoritative_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = test_entity_store(dir.path());
    store.save(&IVec3::ZERO, &Vec::new()).unwrap();
    let loaded = store.load(&IVec3::ZERO).unwrap();
    assert_eq!(loaded, Some(Vec::new()));
}
```

#### 8. Generation path: use missing-vs-empty result

**File**: `crates/voxel_map_engine/src/generation.rs`  
**Action**: modify

This file was not listed in `structure.md`; the user approved adding it because deletion persistence cannot work while
generation collapses missing and empty saves.

Change `load_chunk_entities_from_store` to return `Option<Vec<WorldObjectSpawn>>`, where `None` means no file/error and
`Some(vec![])` means authoritative empty file.

```rust
fn load_chunk_entities_from_store(
    store: &Option<FsChunkEntitiesStore>,
    pos: IVec3,
) -> Option<Vec<WorldObjectSpawn>> {
    let Some(store) = store else {
        return None;
    };
    use persistence::Store;
    match store.load(&pos) {
        Ok(spawns) => spawns,
        Err(e) => {
            bevy::log::warn!("Failed to load entities at {pos}: {e}");
            None
        }
    }
}
```

Update disk terrain load branch:

```rust
let entity_spawns = load_chunk_entities_from_store(&entity_store, pos).unwrap_or_default();
```

Update features stage branch:

```rust
let saved = load_chunk_entities_from_store(&entity_store, position);
let entity_spawns = match saved {
    Some(spawns) => spawns,
    None => generator.place_features(position, &height_map),
};
```

#### 9. Server delete tests

**File**: `crates/server/tests/world_object_edit.rs`  
**Action**: create

Mirror setup from `crates/server/tests/world_object_placement.rs`: `App`, `MinimalPlugins`, `ReplicationSendPlugin`
where necessary, test registry/defs, `MapDimensions`, loaded `VoxelMapInstance`, `ChunkEntityRef`, `Position`, and
`WorldObjectId`.

Minimum tests:

```rust
#[test]
fn delete_validation_accepts_loaded_world_object_on_player_map() { /* validate_world_object_delete Ok */ }

#[test]
fn delete_validation_rejects_missing_or_non_world_object_target() { /* MissingTarget + NotWorldObject */ }

#[test]
fn delete_validation_rejects_foreign_map_and_unloaded_chunk() { /* ForeignMap + ChunkUnavailable */ }

#[test]
fn delete_save_writes_empty_chunk_file() { /* save_chunk_entities_now_or_queue excludes deleted entity and store loads Some(empty) */ }
```

The persistence test can call `save_chunk_entities_now_or_queue`, then `ops.flush()`, then
`FsChunkEntitiesStore::load(&chunk_pos)` and assert `Some(vec![])`.

#### 10. Client selection tests

**File**: `crates/client/tests/plugin.rs`  
**Action**: modify

Add tests behind `#[cfg(feature = "spawn-panel")]`:

```rust
#[test]
fn world_object_selection_ui_sequences_and_pending_delete_ack() {
    let mut ui = WorldObjectSelectionUi::default();
    let target = Entity::from_raw(42);
    assert_eq!(ui.next_sequence(), 0);
    ui.pending_deletes.push(PendingWorldObjectDelete { sequence: 0, target, accepted: false });
    ui.pending_deletes[0].accepted = true;
    assert!(ui.pending_deletes[0].accepted);
}

#[test]
fn nearest_world_object_in_radius_chooses_closest_replicated_object() { /* spawn two replicated WorldObjectId + Position entities and assert closest */ }
```

#### 11. README check

**File**: `README.md`  
**Action**: review; modify only if Phase 1 user-facing spawn panel behavior is documented

If editing, minimally extend “Dev Inspector”:

```markdown
With the spawn panel enabled, the existing-object edit section can select nearby replicated world objects and request
authoritative deletion from the server.
```

### Verification

#### Automated

- [x] Confirm no cargo build/check/test is running:
      `pgrep -af 'cargo (build|check|test|make|server|client)|rustc|rustdoc' || true`
- [x] `cargo test -p server --test world_object_edit delete` passes
- [x] Confirm no cargo build/check/test is running again
- [x] `cargo test -p client --features spawn-panel --test plugin selection` passes
- [x] Confirm no cargo build/check/test is running again
- [x] `cargo test -p voxel_map_engine persistence::tests::empty_entities_file_is_authoritative_empty` passes

#### Manual

- [x] Run `cargo server`, then `cargo client`; press `F4`, then `F6`.
- [x] Select an existing replicated world object from the nearby selection state.
- [x] Press `Delete` or click the delete button if implemented.
- [x] Confirm the selected object disappears on the client by server replication, not local-only despawn.
- [x] Unload and reload the chunk containing the deleted object.
- [x] Confirm the deleted/generated object does not return.

---

## Phase 2: Move Preview and Same-Chunk Move

### Changes

#### 1. Protocol move messages

**File**: `crates/protocol/src/world_object/types.rs`  
**Action**: modify

Add mapped move request and ack. Reuse `WorldObjectEditChannel` and `WorldObjectEditReject`.

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::world_object"]
pub struct WorldObjectMoveRequest {
    pub sequence: u32,
    pub target: Entity,
    pub final_position: Vec3,
}

impl MapEntities for WorldObjectMoveRequest {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.target = entity_mapper.get_mapped(self.target);
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::world_object"]
pub struct WorldObjectMoveAck {
    pub sequence: u32,
    pub target: Entity,
    pub final_position: Vec3,
}

impl MapEntities for WorldObjectMoveAck {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.target = entity_mapper.get_mapped(self.target);
    }
}
```

#### 2. Register move messages

**File**: `crates/protocol/src/lib.rs`  
**Action**: modify

Add to edit message registration:

```rust
app.register_message::<WorldObjectMoveRequest>()
    .add_direction(NetworkDirection::ClientToServer)
    .add_map_entities();
app.register_message::<WorldObjectMoveAck>()
    .add_direction(NetworkDirection::ServerToClient)
    .add_map_entities();
```

#### 3. Pending move UI state

**File**: `crates/dev/src/panels/spawn.rs`  
**Action**: modify

Extend `WorldObjectSelectionUi`:

```rust
pub pending_moves: Vec<PendingWorldObjectMove>,
pub move_armed: bool,
```

Add pending type:

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct PendingWorldObjectMove {
    pub sequence: u32,
    pub target: Entity,
    pub final_position: Vec3,
    pub accepted: bool,
}
```

Update `draw_def_tab` existing-object section:

```rust
if ui.button("Arm move").clicked() && ui_state.selection.selected.is_some() {
    ui_state.selection.move_armed = true;
    ui_state.selection.last_reject = None;
}
if ui_state.selection.move_armed && ui.button("Cancel move").clicked() {
    ui_state.selection.move_armed = false;
}
ui.label(format!("Pending moves: {}", ui_state.selection.pending_moves.len()));
```

#### 4. Local edit preview marker and preview spawning

**File**: `crates/client/src/map.rs`  
**Action**: modify

Add marker:

```rust
#[cfg(feature = "spawn-panel")]
#[derive(Component)]
pub struct WorldObjectEditPreview {
    pub sequence: Option<u32>,
    pub target: Entity,
    pub object_id: WorldObjectId,
}
```

Add terrain target for moves. This differs from placement by using the final world position directly.

```rust
#[cfg(feature = "spawn-panel")]
pub fn current_world_object_move_target(
    player_query: &Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    voxel_world: &mut VoxelWorld,
    camera_query: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: &Query<&Window, With<PrimaryWindow>>,
) -> Option<Vec3> {
    current_placement_target(player_query, voxel_world, camera_query, window_query)
        .map(|target| target.base_position)
}
```

Add preview spawn helper reusing `preview_visual_from_def`:

```rust
#[cfg(feature = "spawn-panel")]
pub fn spawn_world_object_edit_preview(
    commands: &mut Commands,
    sequence: Option<u32>,
    target: Entity,
    object_id: WorldObjectId,
    transform: Transform,
    def: &WorldObjectDef,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    default_material: &DefaultVoxModelMaterial,
) -> Entity {
    let entity = commands
        .spawn((
            WorldObjectEditPreview { sequence, target, object_id },
            transform,
            Visibility::default(),
            Name::new("world-object-edit-preview"),
        ))
        .id();
    preview_visual_from_def(commands, entity, def, vox_registry, vox_assets, default_material);
    entity
}
```

Add `handle_world_object_move_input`: when `selection.move_armed`, `PlaceVoxel` just pressed, selected target exists,
and terrain target exists, send `WorldObjectMoveRequest`, push `PendingWorldObjectMove`, and clear last reject.

Add `update_world_object_edit_preview`: maintain one hover move preview and one preview per pending move. Use selected
target’s `WorldObjectId` and definition. Despawn stale previews whose sequence no longer exists. `trace!` expected
missing registry/target/terrain state.

Add `handle_world_object_move_ack`: mark matching pending move `accepted = true` and clear last reject.

Extend `handle_world_object_edit_reject`: also remove pending moves with the rejected sequence and clear `move_armed`
only if desired; do not clear selection.

Register in `ClientMapPlugin`:

- `Update`: `handle_world_object_move_ack`.
- `PostUpdate` chain: `handle_world_object_move_input`, `update_world_object_edit_preview`,
  `reconcile_edit_preview_on_transform_replication`.

#### 5. Client world-object transform replication reconciliation

**File**: `crates/client/src/world_object.rs`  
**Action**: modify only if current replication does not update `Transform` on changed `Position`

Add or expose helper to keep `Transform` aligned when replicated `Position` changes after initial replication:

```rust
pub fn on_world_object_position_changed(
    mut query: Query<(&Position, Option<&Rotation>, &mut Transform), (With<WorldObjectId>, Changed<Position>)>,
) {
    for (pos, rot, mut transform) in &mut query {
        *transform = transform_from_physics(Some(pos), rot);
    }
}
```

Register this in the client world-object plugin after replication/physics updates, not in `map.rs`, if no equivalent
system exists.

#### 6. Server move validation and apply helper

**File**: `crates/server/src/map.rs`  
**Action**: modify

Extend reject enum in Phase 1 if not already done with placement-style reasons needed for move:

```rust
// in WorldObjectEditRejectReason
NonFinitePosition,
OutOfBounds,
```

Add validation result:

```rust
pub struct ValidatedWorldObjectMove {
    pub map_entity: Entity,
    pub old_chunk_pos: IVec3,
    pub new_chunk_pos: IVec3,
    pub final_position: Vec3,
}
```

Add same-chunk validation in Phase 2. Cross-chunk acceptance remains Phase 4; reject cross-chunk moves with
`ChunkUnavailable` or `OutOfBounds` until Phase 4 explicitly supports them.

```rust
pub fn validate_world_object_move(
    request: &WorldObjectMoveRequest,
    player_map_entity: Entity,
    player_map_id: &MapInstanceId,
    object_query: &Query<(&WorldObjectId, &MapInstanceId, &ChunkEntityRef)>,
    map_query: &Query<(&VoxelMapInstance, &MapDimensions)>,
) -> Result<ValidatedWorldObjectMove, WorldObjectEditRejectReason> {
    if !request.final_position.is_finite() {
        return Err(WorldObjectEditRejectReason::NonFinitePosition);
    }
    let Ok((_id, object_map_id, chunk_ref)) = object_query.get(request.target) else {
        return Err(WorldObjectEditRejectReason::MissingTarget);
    };
    if object_map_id != player_map_id || chunk_ref.map_entity != player_map_entity {
        return Err(WorldObjectEditRejectReason::ForeignMap);
    }
    let (instance, dimensions) = map_query
        .get(player_map_entity)
        .expect("resolved map entity must have VoxelMapInstance and MapDimensions");
    let new_chunk_pos = crate::chunk_entities::chunk_pos_for_world_position(request.final_position, dimensions.chunk_size);
    if !placement_chunk_in_bounds(new_chunk_pos, dimensions) {
        return Err(WorldObjectEditRejectReason::OutOfBounds);
    }
    if new_chunk_pos != chunk_ref.chunk_pos {
        return Err(WorldObjectEditRejectReason::ChunkUnavailable);
    }
    let column = voxel_map_engine::prelude::chunk_to_column(new_chunk_pos);
    if !instance.chunk_levels.contains_key(&column) || instance.get_chunk_data(new_chunk_pos).is_none() {
        return Err(WorldObjectEditRejectReason::ChunkUnavailable);
    }
    Ok(ValidatedWorldObjectMove {
        map_entity: player_map_entity,
        old_chunk_pos: chunk_ref.chunk_pos,
        new_chunk_pos,
        final_position: request.final_position,
    })
}
```

Add apply helper:

```rust
pub fn apply_world_object_move(
    entity: Entity,
    validated: &ValidatedWorldObjectMove,
    commands: &mut Commands,
) {
    commands.entity(entity).insert(Position(validated.final_position));
}
```

Add `handle_world_object_move_requests` mirroring delete:

- Resolve player map.
- Validate request.
- Insert new `Position`.
- Queue `save_chunk_entities_now_or_queue(validated.map_entity, validated.old_chunk_pos, None, ...)`.
- Send `WorldObjectMoveAck` on `WorldObjectEditChannel`.

Register in `ServerMapPlugin::build` next to delete.

#### 7. Chunk persistence support for moved same-chunk objects

**File**: `crates/server/src/chunk_entities.rs`  
**Action**: modify

No new helper is required if `save_chunk_entities_now_or_queue` from Phase 1 collects current `Position`. Ensure the
move request queues save after `commands.entity(entity).insert(Position(...))`; because `Commands` are deferred, either:

- update persistence collection by passing an override position for the moved entity, or
- mutate the entity immediately with `World` access in a command and queue persistence in a following system.

Preferred minimal approach: extend helper to accept an optional override:

```rust
pub struct ChunkEntitySaveOverride {
    pub entity: Entity,
    pub position: Option<Vec3>,
    pub chunk_pos: Option<IVec3>,
}
```

Use the override when collecting the moved entity so the queued save contains the accepted `final_position` in the same
frame.

#### 8. Server move tests

**File**: `crates/server/tests/world_object_edit.rs`  
**Action**: modify

Add:

```rust
#[test]
fn move_same_chunk_validation_accepts_loaded_target() { /* final_position finite, same chunk */ }

#[test]
fn move_same_chunk_validation_rejects_non_finite_out_of_bounds_and_unloaded() { /* reject reasons */ }

#[test]
fn same_chunk_move_save_uses_new_final_position() { /* queue save with override and assert loaded spawn.position */ }
```

#### 9. Client preview tests

**File**: `crates/client/tests/plugin.rs`  
**Action**: modify

Add:

```rust
#[test]
fn edit_preview_is_local_only() { /* assert WorldObjectEditPreview + Transform, no Position/MapInstanceId/Replicated/Collider/WorldObjectId */ }

#[test]
fn edit_preview_reconciles_when_replicated_transform_matches_accepted_move() { /* pending move accepted, target Position matches, preview despawned */ }
```

### Verification

#### Automated

- [x] Confirm no cargo build/check/test is running:
      `pgrep -af 'cargo (build|check|test|make|server|client)|rustc|rustdoc' || true`
- [x] `cargo test -p server --test world_object_edit move_same_chunk` passes
- [x] Confirm no cargo build/check/test is running again
- [x] `cargo test -p client --features spawn-panel --test plugin edit_preview` passes

#### Manual

- [ ] Run server/client, press `F4`, then `F6`.
- [ ] Select an existing object by nearby list.
- [ ] Arm move and hover over a same-chunk terrain position.
- [ ] Confirm a local-only preview appears.
- [ ] Submit move.
- [ ] Confirm the preview is replaced by the replicated object at the new position.
- [ ] Unload/reload the chunk and confirm the object remains at the moved position.

---

## Phase 3: Rotate Objects and Persist Rotation

### Changes

#### 1. Protocol rotate messages

**File**: `crates/protocol/src/world_object/types.rs`  
**Action**: modify

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::world_object"]
pub struct WorldObjectRotateRequest {
    pub sequence: u32,
    pub target: Entity,
    pub rotation: Quat,
}

impl MapEntities for WorldObjectRotateRequest {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.target = entity_mapper.get_mapped(self.target);
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::world_object"]
pub struct WorldObjectRotateAck {
    pub sequence: u32,
    pub target: Entity,
    pub rotation: Quat,
}

impl MapEntities for WorldObjectRotateAck {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.target = entity_mapper.get_mapped(self.target);
    }
}

/// Persisted rotation snapshot for chunk entity saves.
#[derive(Component, Reflect, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[reflect(Component, Persist)]
pub struct WorldObjectRotationSnapshot(pub Quat);
```

Add reject reason if missing:

```rust
InvalidRotation,
```

#### 2. Register rotate messages and rotation snapshot type

**File**: `crates/protocol/src/lib.rs`  
**Action**: modify

```rust
app.register_message::<WorldObjectRotateRequest>()
    .add_direction(NetworkDirection::ClientToServer)
    .add_map_entities();
app.register_message::<WorldObjectRotateAck>()
    .add_direction(NetworkDirection::ServerToClient)
    .add_map_entities();
```

**File**: `crates/protocol/src/world_object/plugin.rs`  
**Action**: modify

Register the persisted snapshot type:

```rust
app.register_type::<super::types::WorldObjectRotationSnapshot>();
```

#### 3. Rotation UI state

**File**: `crates/dev/src/panels/spawn.rs`  
**Action**: modify

Extend `WorldObjectSelectionUi`:

```rust
pub rotation_degrees_y: f32,
pub pending_rotations: Vec<PendingWorldObjectRotation>,
```

Add pending type:

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct PendingWorldObjectRotation {
    pub sequence: u32,
    pub target: Entity,
    pub rotation: Quat,
    pub accepted: bool,
}
```

Extend UI:

```rust
ui.add(egui::Slider::new(&mut ui_state.selection.rotation_degrees_y, -180.0..=180.0).text("Yaw"));
if ui.button("Rotate selected").clicked() && ui_state.selection.selected.is_some() {
    ui_state.selection.rotate_requested = true;
}
ui.label(format!("Pending rotations: {}", ui_state.selection.pending_rotations.len()));
```

If using a button, add `rotate_requested: bool` to state and consume it in client map system.

#### 4. Client rotate request and preview

**File**: `crates/client/src/map.rs`  
**Action**: modify

Add `handle_world_object_rotate_input`:

- Requires selected target.
- Consumes `rotate_requested` from UI or uses a key such as `KeyR` only when selection exists.
- Builds `Quat::from_rotation_y(ui_state.selection.rotation_degrees_y.to_radians())`.
- Sends `WorldObjectRotateRequest` on `WorldObjectEditChannel`.
- Pushes `PendingWorldObjectRotation`.

Update edit preview transform to include pending/hover rotation:

```rust
let transform = Transform {
    translation: current_or_pending_position,
    rotation: pending_rotation_or_current_rotation,
    ..default()
};
```

Add `handle_world_object_rotate_ack`; extend `handle_world_object_edit_reject` to remove pending rotations by sequence.

#### 5. Server rotation validation/application

**File**: `crates/server/src/map.rs`  
**Action**: modify

Add validation:

```rust
pub fn validate_world_object_rotation(
    request: &WorldObjectRotateRequest,
    player_map_entity: Entity,
    player_map_id: &MapInstanceId,
    object_query: &Query<(&WorldObjectId, &MapInstanceId, &ChunkEntityRef)>,
    map_query: &Query<&VoxelMapInstance>,
) -> Result<Quat, WorldObjectEditRejectReason> {
    if !request.rotation.is_finite() || request.rotation.length_squared() <= f32::EPSILON {
        return Err(WorldObjectEditRejectReason::InvalidRotation);
    }
    let Ok((_id, object_map_id, chunk_ref)) = object_query.get(request.target) else {
        return Err(WorldObjectEditRejectReason::MissingTarget);
    };
    if object_map_id != player_map_id || chunk_ref.map_entity != player_map_entity {
        return Err(WorldObjectEditRejectReason::ForeignMap);
    }
    let instance = map_query
        .get(player_map_entity)
        .expect("resolved map entity must have VoxelMapInstance");
    let column = voxel_map_engine::prelude::chunk_to_column(chunk_ref.chunk_pos);
    if !instance.chunk_levels.contains_key(&column) || instance.get_chunk_data(chunk_ref.chunk_pos).is_none() {
        return Err(WorldObjectEditRejectReason::ChunkUnavailable);
    }
    Ok(request.rotation.normalize())
}
```

Add `handle_world_object_rotate_requests`:

- Resolve player map.
- Validate target and rotation.
- Insert `Rotation(rotation)` on target.
- Queue chunk save with a rotation override, or use immediate world mutation before collection.
- Send `WorldObjectRotateAck`.

Register in `ServerMapPlugin::build` next to move/delete.

#### 6. Persist rotation snapshots

**File**: `crates/server/src/chunk_entities.rs`  
**Action**: modify

Add `Rotation` to all chunk entity save queries:

```rust
Option<&Rotation>,
```

Change `serialize_persisted` signature:

```rust
fn serialize_persisted(
    active_transform: Option<&ActiveTransformation>,
    health: Option<&protocol::Health>,
    rotation: Option<&Rotation>,
) -> Vec<PersistedComponent>
```

Serialize rotation snapshot:

```rust
if let Some(rotation) = rotation {
    let snapshot = protocol::world_object::WorldObjectRotationSnapshot(rotation.0);
    if let Ok(ron_data) = ron::to_string(&snapshot) {
        result.push(PersistedComponent {
            type_path: std::any::type_name::<protocol::world_object::WorldObjectRotationSnapshot>().to_string(),
            ron_data,
        });
    }
}
```

Update `restore_persisted`:

```rust
let rotation_type = std::any::type_name::<protocol::world_object::WorldObjectRotationSnapshot>();
let mut persisted_rotation: Option<protocol::world_object::WorldObjectRotationSnapshot> = None;
// parse in loop
if let Some(rotation) = persisted_rotation {
    commands.entity(entity).insert(Rotation(rotation.0));
}
```

Ensure rotation restore happens after any transformation application so persisted rotation wins.

#### 7. Persistence tests for rotation payload shape

**File**: `crates/voxel_map_engine/src/persistence/mod.rs`  
**Action**: modify

Add test that `WorldObjectSpawn.persisted_components` with a rotation snapshot round-trips through
`FsChunkEntitiesStore`. No schema bump required because `persisted_components` already exists.

```rust
#[test]
fn chunk_entities_preserve_rotation_persisted_component() { /* save/load and compare type_path + ron_data */ }
```

#### 8. Server rotation tests

**File**: `crates/server/tests/world_object_edit.rs`  
**Action**: modify

Add:

```rust
#[test]
fn rotate_validation_accepts_normalized_finite_rotation() { /* assert normalized returned */ }

#[test]
fn rotate_validation_rejects_invalid_rotation_and_unavailable_chunk() { /* InvalidRotation + ChunkUnavailable */ }

#[test]
fn rotation_persists_through_chunk_entity_save_restore_payload() { /* save/load persisted component */ }
```

### Verification

#### Automated

- [x] Confirm no cargo build/check/test is running:
      `pgrep -af 'cargo (build|check|test|make|server|client)|rustc|rustdoc' || true`
- [x] `cargo test -p server --test world_object_edit rotate` passes
- [x] Confirm no cargo build/check/test is running again
- [x] `cargo test -p voxel_map_engine chunk_entities` passes

#### Manual

- [x] Run server/client, select a replicated world object in the F6 panel.
- [x] Rotate the object using the yaw control.
- [x] Confirm orientation changes by server replication.
- [x] Unload/reload the chunk.
- [x] Confirm replicated orientation persists after reload.

---

## Phase 4: Cross-Chunk Move Persistence

### Changes

#### 1. Accept cross-chunk validation and explicit destination rejection

**File**: `crates/protocol/src/world_object/types.rs`  
**Action**: modify

Add reject reason:

```rust
DestinationChunkUnavailable,
```

#### 2. Expand move validation result and acceptance

**File**: `crates/server/src/map.rs`  
**Action**: modify

Update `validate_world_object_move` from Phase 2 to allow `new_chunk_pos != old_chunk_pos` when destination chunk is in
bounds, column loaded, and chunk data exists. Return `DestinationChunkUnavailable` for an unavailable destination chunk.

```rust
if !instance.chunk_levels.contains_key(&column) || instance.get_chunk_data(new_chunk_pos).is_none() {
    return Err(WorldObjectEditRejectReason::DestinationChunkUnavailable);
}
```

Keep `ValidatedWorldObjectMove` as:

```rust
pub struct ValidatedWorldObjectMove {
    pub map_entity: Entity,
    pub old_chunk_pos: IVec3,
    pub new_chunk_pos: IVec3,
    pub final_position: Vec3,
}
```

Update `apply_world_object_move`:

```rust
pub fn apply_world_object_move(
    entity: Entity,
    validated: &ValidatedWorldObjectMove,
    commands: &mut Commands,
) {
    commands.entity(entity).insert(Position(validated.final_position));
    if validated.old_chunk_pos != validated.new_chunk_pos {
        commands.entity(entity).insert(ChunkEntityRef {
            map_entity: validated.map_entity,
            chunk_pos: validated.new_chunk_pos,
        });
    }
}
```

#### 3. Queue source and destination persistence

**File**: `crates/server/src/chunk_entities.rs`  
**Action**: modify

Add a helper for cross-chunk moves:

```rust
/// Queues saves for both chunks affected by a moved world object.
pub fn queue_world_object_move_persistence(
    map_entity: Entity,
    old_chunk_pos: IVec3,
    new_chunk_pos: IVec3,
    moved_entity: Entity,
    final_position: Vec3,
    entity_query: &Query<(
        Entity,
        &ChunkEntityRef,
        &WorldObjectId,
        &Position,
        Option<&ActiveTransformation>,
        Option<&protocol::Health>,
        Option<&Rotation>,
    )>,
    store_query: &mut Query<(
        &StoreBackend<IVec3, Vec<WorldObjectSpawn>, FsChunkEntitiesStore>,
        &mut PendingStoreOps<IVec3, Vec<WorldObjectSpawn>>,
    )>,
) {
    save_chunk_entities_now_or_queue(map_entity, old_chunk_pos, Some(moved_entity), entity_query, store_query);
    save_chunk_entities_now_or_queue_with_override(
        map_entity,
        new_chunk_pos,
        None,
        Some(ChunkEntitySaveOverride {
            entity: moved_entity,
            position: Some(final_position),
            chunk_pos: Some(new_chunk_pos),
        }),
        entity_query,
        store_query,
    );
}
```

If Phase 2 already introduced an override-capable helper, implement this helper as a thin wrapper around it.

#### 4. Use cross-chunk persistence from move request handling

**File**: `crates/server/src/map.rs`  
**Action**: modify

In `handle_world_object_move_requests`, replace the single old-chunk save with:

```rust
crate::chunk_entities::queue_world_object_move_persistence(
    validated.map_entity,
    validated.old_chunk_pos,
    validated.new_chunk_pos,
    request.target,
    validated.final_position,
    &entity_save_query,
    &mut store_query,
);
```

Ensure the source chunk save excludes `request.target` even though the deferred `ChunkEntityRef` update has not applied
yet.

#### 5. UI displays cross-chunk move status

**File**: `crates/dev/src/panels/spawn.rs`  
**Action**: modify

Extend `PendingWorldObjectMove`:

```rust
pub old_chunk_pos: Option<IVec3>,
pub new_chunk_pos: Option<IVec3>,
```

Render when present:

```rust
for pending in &ui_state.selection.pending_moves {
    if let (Some(old), Some(new)) = (pending.old_chunk_pos, pending.new_chunk_pos) {
        ui.label(format!("Move {}: {old:?} -> {new:?}", pending.sequence));
    }
}
```

#### 6. Client computes source/destination display data

**File**: `crates/client/src/map.rs`  
**Action**: modify

When pushing `PendingWorldObjectMove`, compute chunk positions if local `ChunkTicket` / map dimensions are available. If
unavailable, store `None` and `trace!`.

```rust
let new_chunk_pos = voxel_map_engine::lifecycle::world_to_chunk_pos(final_position, dimensions.chunk_size);
```

Do not use these client-side chunk values for authority; they are UI-only.

#### 7. Cross-chunk tests

**File**: `crates/server/tests/world_object_edit.rs`  
**Action**: modify

Add:

```rust
#[test]
fn cross_chunk_move_validation_accepts_loaded_destination() { /* old != new, both loaded */ }

#[test]
fn cross_chunk_move_rejects_unavailable_destination() { /* DestinationChunkUnavailable */ }

#[test]
fn cross_chunk_move_saves_empty_source_and_destination_with_moved_object() { /* source load Some(empty), destination load contains moved object */ }
```

### Verification

#### Automated

- [x] Confirm no cargo build/check/test is running:
      `pgrep -af 'cargo (build|check|test|make|server|client)|rustc|rustdoc' || true`
- [x] `cargo test -p server --test world_object_edit cross_chunk` passes

#### Manual

- [x] Run server/client and select an object in a loaded chunk.
- [x] Move it into an adjacent loaded chunk.
- [x] Confirm the move is accepted and replicated.
- [x] Unload/reload both source and destination chunks.
- [x] Confirm the source chunk stays empty and the destination chunk contains the moved object.
- [x] Attempt moving into an unloaded destination chunk and confirm the UI shows `DestinationChunkUnavailable`.

---

## Phase 5: Cursor Picking Polish and Runtime QA

### Changes

#### 1. Cursor picking source state and final edit tab UI

**File**: `crates/dev/src/panels/spawn.rs`  
**Action**: modify

Add selection source:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldObjectSelectionSource {
    Cursor,
    NearbyList,
}
```

Extend `WorldObjectSelectionUi`:

```rust
pub selection_source: Option<WorldObjectSelectionSource>,
pub cursor_pick_requested: bool,
pub delete_requested: bool,
```

Factor existing-object UI into a self-describing function:

```rust
fn draw_world_object_edit_tab(ui: &mut egui::Ui, ui_state: &mut SpawnPanelUi) {
    // selected entity, source, nearby radius, delete/move/rotate controls, pending counts, last reject
}
```

Call it from `draw_def_tab` below placement controls. Do not add arbitrary component editing.

#### 2. Cursor object picking

**File**: `crates/client/src/map.rs`  
**Action**: modify

Add `current_world_object_pick`. It maps the cursor ray to the nearest replicated world object by distance from the ray.
Keep it simple: use each object’s `Position` and choose the closest point within a small radius threshold; do not
introduce a new picking dependency.

```rust
#[cfg(feature = "spawn-panel")]
pub fn current_world_object_pick(
    camera_query: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: &Query<&Window, With<PrimaryWindow>>,
    object_query: &Query<(Entity, &Position), (With<WorldObjectId>, With<Replicated>)>,
) -> Option<Entity> {
    let Some(ray) = camera_ray(camera_query, window_query) else {
        trace!("current_world_object_pick: no camera ray");
        return None;
    };
    object_query
        .iter()
        .filter_map(|(entity, position)| {
            let point = Vec3::from(position.0);
            let to_point = point - ray.origin;
            let t = to_point.dot(*ray.direction);
            if !(0.0..=RAYCAST_MAX_DISTANCE).contains(&t) {
                return None;
            }
            let closest = ray.origin + *ray.direction * t;
            let dist_sq = point.distance_squared(closest);
            (dist_sq <= 2.0 * 2.0).then_some((entity, t, dist_sq))
        })
        .min_by(|(_, t_a, d_a), (_, t_b, d_b)| t_a.total_cmp(t_b).then(d_a.total_cmp(d_b)))
        .map(|(entity, _, _)| entity)
}
```

Add `handle_world_object_cursor_pick_input`:

- Consume `cursor_pick_requested` or a key/mouse shortcut only when spawn panel is active.
- Set `selection.selected` and `selection.selection_source = Some(Cursor)`.
- `trace!` if no pick found.

Update nearby selection to set `selection_source = Some(NearbyList)`.

#### 3. Stale preview cleanup

**File**: `crates/client/src/map.rs`  
**Action**: modify

Add cleanup system:

```rust
#[cfg(feature = "spawn-panel")]
fn cleanup_stale_world_object_edit_previews(
    mut commands: Commands,
    ui_state: Res<SpawnPanelUi>,
    target_query: Query<Entity, (With<WorldObjectId>, With<Replicated>)>,
    preview_query: Query<(Entity, &WorldObjectEditPreview)>,
) {
    for (preview_entity, preview) in &preview_query {
        let target_exists = target_query.get(preview.target).is_ok();
        let pending_move = ui_state.selection.pending_moves.iter().any(|p| p.sequence == preview.sequence.unwrap_or(u32::MAX));
        let pending_rotation = ui_state.selection.pending_rotations.iter().any(|p| p.sequence == preview.sequence.unwrap_or(u32::MAX));
        let hover = preview.sequence.is_none() && ui_state.selection.move_armed;
        if !target_exists || (!pending_move && !pending_rotation && !hover) {
            trace!("cleanup_stale_world_object_edit_previews: despawning stale preview {preview_entity:?}");
            commands.entity(preview_entity).despawn();
        }
    }
}
```

Register after ack/reject handlers and before preview creation in the PostUpdate chain.

#### 4. Keep changed transforms aligned on client

**File**: `crates/client/src/world_object.rs`  
**Action**: modify

Ensure `on_world_object_position_changed` and a matching rotation changed path are registered:

```rust
pub fn on_world_object_rotation_changed(
    mut query: Query<(&Position, &Rotation, &mut Transform), (With<WorldObjectId>, Changed<Rotation>)>,
) {
    for (pos, rot, mut transform) in &mut query {
        *transform = transform_from_physics(Some(pos), Some(rot));
    }
}
```

If `transform_from_physics` is private and accepts `Option<&Position>, Option<&Rotation>`, reuse it directly in the same
module.

#### 5. Client plugin tests for cursor pick and cleanup

**File**: `crates/client/tests/plugin.rs`  
**Action**: modify

Add tests:

```rust
#[test]
fn stale_edit_preview_is_removed_when_target_despawns() { /* target absent -> preview despawn */ }

#[test]
fn cursor_pick_prefers_nearest_object_along_ray() { /* unit-level helper test if camera setup is too heavy */ }
```

If full Bevy camera/window setup is brittle, extract ray-object selection into a pure helper:

```rust
pub fn pick_world_object_from_ray(ray: Ray3d, objects: impl Iterator<Item = (Entity, Vec3)>) -> Option<Entity>
```

and test that helper.

#### 6. README dev tooling docs

**File**: `README.md`  
**Action**: modify

Update the existing “Dev Inspector” paragraph to include final behavior:

```markdown
Press `F4` to toggle the dev inspector root menu. With the spawn panel enabled, press `F6` or use the root menu to open
it. Def-driven world-object placement is server-authoritative: select an object, arm placement, preview the terrain
target, then click terrain in-game. The same panel can select existing replicated world objects by cursor pick or nearby
list and request authoritative delete, move, or yaw rotation edits that persist across chunk reloads. Free-form spawning
remains client-local.
```

### Verification

#### Automated

- [ ] Confirm no cargo build/check/test is running:
      `pgrep -af 'cargo (build|check|test|make|server|client)|rustc|rustdoc' || true`
- [ ] `cargo check-all` passes
- [ ] Confirm no cargo build/check/test is running again
- [ ] `cargo test-all` passes
- [ ] Confirm no cargo build/check/test is running again
- [ ] `cargo test -p client --features spawn-panel --test plugin cursor_pick` passes if not already covered by
      `cargo test-all`
- [ ] Confirm no cargo build/check/test is running again
- [ ] `cargo test -p client --features spawn-panel --test plugin cleanup` passes if not already covered by
      `cargo test-all`

#### Manual

- [ ] Run `cargo server` and `cargo client`.
- [ ] Press `F4`, then `F6`.
- [ ] Select by nearby list and by cursor pick; confirm UI records the correct selection source.
- [ ] Delete selected object; confirm despawn replicates and deleted object stays gone after chunk reload.
- [ ] Move selected object within the same chunk; confirm preview cleanup and persisted reload.
- [ ] Move selected object across chunks; confirm source/destination persistence after both chunks reload.
- [ ] Rotate selected object; confirm orientation replication and persisted reload.
- [ ] Force rejection cases: no selected target, stale/despawned target, foreign-map target if test setup allows,
      unavailable destination chunk.
- [ ] Confirm rejects remove pending previews and keep real replicated objects visible.

---

## Testing Checkpoints

- [x] After Phase 1: a selected replicated object can be deleted authoritatively, its chunk save can be empty, and
      deleted generated objects do not return.
- [ ] After Phase 2: same-chunk moves use local previews, server mutation, ack/reject cleanup, replication
      reconciliation, and persistence.
- [x] After Phase 3: rotations replicate and survive chunk unload/reload through persisted components.
- [x] After Phase 4: cross-chunk moves save both source and destination chunks and reject unavailable destinations.
- [ ] After Phase 5: both selection modes work in the dev panel, stale previews are cleaned up, full cargo checks/tests
      pass, and runtime QA covers the complete edit workflow.

## Approved Deviation From Structure Outline

- Added `crates/voxel_map_engine/src/generation.rs` to Phase 1. Reason: durable deletion requires distinguishing missing
  entity files from existing empty entity files during feature generation; the current regeneration decision is
  implemented in `generation.rs`, not only `persistence/mod.rs`. User approved this deviation before plan writing.
