# Structure Outline

## Approach

Mirror def-driven placement: dev UI selects a replicated world-object entity, sends sequenced reliable requests, and the
server validates, mutates, replicates, and queues chunk persistence. Build in vertical slices: first deletion, then
same-chunk move preview, rotation persistence, and cross-chunk moves.

## Phase 1: Select and Delete Loaded Objects

Deliver selection from the spawn panel plus an authoritative delete request. This proves client entity targeting,
Lightyear entity mapping, server validation, replication despawn, and durable empty-chunk saves end-to-end.

**Files**: `crates/protocol/src/world_object/types.rs`, `crates/protocol/src/lib.rs`, `crates/dev/src/panels/spawn.rs`,
`crates/client/src/map.rs`, `crates/server/src/map.rs`, `crates/server/src/chunk_entities.rs`,
`crates/server/tests/world_object_edit.rs`, `crates/client/tests/plugin.rs`,
`crates/voxel_map_engine/src/persistence/mod.rs`

**Key changes**:

- `WorldObjectEditChannel` — ordered reliable bidirectional channel.
- `WorldObjectDeleteRequest { sequence: u32, target: Entity }` — new client request, `MapEntities` +
  `.add_map_entities()`.
- `WorldObjectDeleteAck { sequence: u32, target: Entity }` — new server ack, mapped entity.
- `WorldObjectEditReject { sequence: u32, reason: WorldObjectEditRejectReason }` — shared reject.
- `WorldObjectEditRejectReason::{NoControlledCharacter, TargetNotMapped, MissingTarget, NotWorldObject, ForeignMap, ChunkUnavailable}`
  — new reasons.

- ````WorldObjectSelectionUi {
    selected: Option<Entity>,
    nearby_radius: f32,
    next_sequence: u32,
    pending_deletes: Vec<PendingWorldObjectDelete>,
    last_reject: Option<WorldObjectEditRejectReason>,
  }```
        — new panel state.
  ````

- `handle_world_object_delete_requests(...)` — server validates selected entity, despawns it, and queues a save for its
  chunk.
- `save_chunk_entities_now_or_queue(map_entity: Entity, chunk_pos: IVec3, ...)` — helper that writes current chunk
  object state, including empty vectors.

**Verify**: Before running cargo, confirm no other cargo build/check/test is running. Then run
`cargo test -p server --test world_object_edit delete` and
`cargo test -p client --features spawn-panel --test plugin selection`; manually enable F6, select an existing object
from the nearby list, click delete, and confirm it disappears and does not return after chunk unload/reload.

---

## Phase 2: Move Preview and Same-Chunk Move

Add local-only move previews and an authoritative move request for objects that remain in their current chunk. This
makes edit previews useful while keeping server state as the only source of truth.

**Files**: `crates/protocol/src/world_object/types.rs`, `crates/dev/src/panels/spawn.rs`, `crates/client/src/map.rs`,
`crates/client/src/world_object.rs`, `crates/server/src/map.rs`, `crates/server/src/chunk_entities.rs`,
`crates/server/tests/world_object_edit.rs`, `crates/client/tests/plugin.rs`

**Key changes**:

- `WorldObjectMoveRequest { sequence: u32, target: Entity, final_position: Vec3 }` — new mapped request.
- `WorldObjectMoveAck { sequence: u32, target: Entity, final_position: Vec3 }` — accepted mutation ack.
- `PendingWorldObjectMove { sequence: u32, target: Entity, final_position: Vec3, accepted: bool }` — pending UI state.
- `WorldObjectEditPreview { sequence: Option<u32>, target: Entity, object_id: WorldObjectId }` — local-only preview
  marker.
- `current_world_object_move_target(...): Option<Vec3>` — cursor terrain target for move placement.
- `validate_world_object_move(...): Result<ValidatedWorldObjectMove, WorldObjectEditRejectReason>` — finite, bounds,
  same-map, loaded-chunk validation.
- `apply_world_object_move(entity: Entity, final_position: Vec3, chunk_pos: IVec3, ...)` — mutates `Position`, leaves
  `ChunkEntityRef` unchanged for same-chunk moves, queues persistence.
- `reconcile_edit_preview_on_transform_replication(...)` — despawns matching preview when replicated state matches ack.

**Verify**: Run `cargo test -p server --test world_object_edit move_same_chunk` and
`cargo test -p client --features spawn-panel --test plugin edit_preview`; manually select an object, preview a new
same-chunk position, submit move, and confirm the preview is replaced by replicated object state.

---

## Phase 3: Rotate Objects and Persist Rotation

Add rotation editing through the same request/preview path. Persist rotation through `persisted_components` rather than
extending `WorldObjectSpawn`.

**Files**: `crates/protocol/src/world_object/types.rs`, `crates/dev/src/panels/spawn.rs`, `crates/client/src/map.rs`,
`crates/server/src/map.rs`, `crates/server/src/chunk_entities.rs`, `crates/server/tests/world_object_edit.rs`,
`crates/voxel_map_engine/src/persistence/mod.rs`

**Key changes**:

- `WorldObjectRotateRequest { sequence: u32, target: Entity, rotation: Quat }` — new mapped request.
- `WorldObjectRotateAck { sequence: u32, target: Entity, rotation: Quat }` — accepted mutation ack.
- `PendingWorldObjectRotation { sequence: u32, target: Entity, rotation: Quat, accepted: bool }` — pending UI state.
- `WorldObjectRotationSnapshot(pub Quat)` or reflected `Rotation` persistence support — serialized into
  `WorldObjectSpawn.persisted_components`.
- `serialize_persisted(..., rotation: Option<&Rotation>) -> Vec<PersistedComponent>` — includes rotation snapshot.
- `restore_persisted(...)` — restores persisted rotation after chunk reload.
- `validate_world_object_rotation(...): Result<Quat, WorldObjectEditRejectReason>` — rejects non-finite/invalid
  rotations.

**Verify**: Run `cargo test -p server --test world_object_edit rotate` and
`cargo test -p voxel_map_engine chunk_entities`; manually rotate an object, unload/reload its chunk, and confirm
replicated orientation persists.

---

## Phase 4: Cross-Chunk Move Persistence

Extend move to support crossing chunk boundaries. The server updates ownership, removes the object from the old chunk
save, appends it to the new chunk save, and rejects moves into unavailable destination chunks.

**Files**: `crates/server/src/map.rs`, `crates/server/src/chunk_entities.rs`,
`crates/server/tests/world_object_edit.rs`, `crates/dev/src/panels/spawn.rs`, `crates/client/src/map.rs`

**Key changes**:

- `ValidatedWorldObjectMove { map_entity: Entity, old_chunk_pos: IVec3, new_chunk_pos: IVec3, final_position: Vec3 }` —
  validation result expanded for chunk transfer.
- `apply_world_object_move(...)` — updates `ChunkEntityRef` when `old_chunk_pos != new_chunk_pos`.
- `queue_world_object_move_persistence(map_entity: Entity, old_chunk_pos: IVec3, new_chunk_pos: IVec3, moved_entity: Entity, ...)`
  — queues saves for both affected chunks.
- `WorldObjectEditRejectReason::DestinationChunkUnavailable` — explicit cross-chunk rejection.
- `PendingWorldObjectMove` UI display — shows source/destination chunk and rejection reason.

**Verify**: Run `cargo test -p server --test world_object_edit cross_chunk`; manually move an object into an adjacent
loaded chunk, unload/reload both chunks, and confirm the source stays empty and destination contains the moved object.

---

## Phase 5: Cursor Picking Polish and Runtime QA

Add cursor-based object selection on top of the nearby list, and do runtime validation across delete, move, rotate,
reject cleanup, target despawn, and delayed replication.

**Files**: `crates/dev/src/panels/spawn.rs`, `crates/client/src/map.rs`, `crates/client/src/world_object.rs`,
`crates/client/tests/plugin.rs`, `README.md`

**Key changes**:

- `current_world_object_pick(...): Option<Entity>` — maps cursor ray to nearest replicated world object, with `trace!`
  early-outs when unavailable.
- `WorldObjectSelectionSource::{Cursor, NearbyList}` — records how current selection was chosen.
- `cleanup_stale_world_object_edit_previews(...)` — removes previews for rejects, despawned targets, or completed
  reconciliation.
- `draw_world_object_edit_tab(...)` — final panel UI for select/delete/move/rotate status.

**Verify**: Run `cargo check-all` and `cargo test-all`; manually test F6 panel selection by cursor and nearby list,
invalid target rejection, delete/move/rotate persistence after reload, and update `README.md` if user-facing dev tooling
docs mention spawn panel behavior.

## Testing Checkpoints

- After Phase 1: a selected replicated object can be deleted authoritatively, its chunk save can be empty, and deleted
  generated objects do not return.
- After Phase 2: same-chunk moves use local previews, server mutation, ack/reject cleanup, replication reconciliation,
  and persistence.
- After Phase 3: rotations replicate and survive chunk unload/reload through persisted components.
- After Phase 4: cross-chunk moves save both source and destination chunks and reject unavailable destinations.
- After Phase 5: both selection modes work in the dev panel, stale previews are cleaned up, full cargo checks/tests
  pass, and runtime QA covers the complete edit workflow.
