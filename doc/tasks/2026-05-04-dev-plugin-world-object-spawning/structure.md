# Structure Outline

## Approach

Cleanly cut over def-driven dev spawning from client-local entity creation to server-authoritative placement. First make saved world-object positions explicit, then add the placement protocol/server path, then wire the dev UI and preview around authoritative replication.

## Phase 1: Explicit World-Object Position Semantics

Make generated and saved world-object positions unambiguous so `PlacementOffset` is applied exactly once. This is a prerequisite slice: it crosses spawn generation, per-chunk persistence, reload, and eviction, but intentionally has no UI because placement is not safe to rely on until persistence semantics are fixed.

**Files**: `crates/voxel_map_engine/src/config.rs`, `crates/voxel_map_engine/src/persistence/mod.rs`, `crates/server/src/chunk_entities.rs`

**Key changes**:
- `enum WorldObjectPositionKind { PlacementBase, Final }` — new persisted position meaning.
- `WorldObjectSpawn { object_id: String, position: Vec3, position_kind: WorldObjectPositionKind, persisted_components: Vec<PersistedComponent> }` — modified storage boundary.
- `fn extract_placement_offset(def: &WorldObjectDef, position_kind: WorldObjectPositionKind) -> Vec3` — replace `persisted_components.is_empty()` reload inference.
- `fn collect_chunk_entities(...) -> HashMap<(Entity, IVec3), Vec<WorldObjectSpawn>>` — write saved entities as `Final`.

**Verify**: `cargo test -p voxel_map_engine persistence::tests`; `cargo test -p server --test voxel_persistence`; manually confirm old/default generated spawns still load as `PlacementBase` while saved/evicted entities persist final positions.

---

## Phase 2: Authoritative Placement Protocol and Server Commit

Add a dedicated ordered reliable placement request/ack/reject contract and a server handler that derives map scope from the controlled character. A direct server-side test can place an object without any dev UI and verify the spawned entity is replicated, map-scoped, chunk-tagged, and persistable.

**Files**: `crates/protocol/src/lib.rs`, `crates/protocol/src/world_object/mod.rs`, `crates/protocol/src/world_object/types.rs`, `crates/server/src/map.rs`, `crates/server/src/chunk_entities.rs`, `crates/server/src/world_object.rs`, `crates/server/tests/world_object_placement.rs`

**Key changes**:
- `struct WorldObjectPlacementChannel;` — new bidirectional ordered reliable channel.
- `WorldObjectPlacementRequest { sequence: u32, object_id: WorldObjectId, base_position: Vec3 }` — client-to-server request; no authoritative map id.
- `WorldObjectPlacementAck { sequence: u32, object_id: WorldObjectId, final_position: Vec3 }` — server-to-client correlation only, not object commit.
- `WorldObjectPlacementReject { sequence: u32, reason: WorldObjectPlacementRejectReason }` — server-to-client failure.
- `enum WorldObjectPlacementRejectReason { NoControlledCharacter, UnknownObject, NonFinitePosition, OutOfBounds, ChunkUnavailable }` — explicit validation failures.
- `pub fn handle_world_object_placement_requests(...)` — receive, validate, spawn, ack/reject.
- `fn spawn_placed_world_object(..., object_id: WorldObjectId, base_position: Vec3, map_entity: Entity, map_id: MapInstanceId) -> Entity` — inserts `Position` and `ChunkEntityRef` after `spawn_world_object`.

**Verify**: `cargo test -p server --test world_object_placement`; manually inspect that successful placement uses `Replicate`, `NetworkVisibility`, `MapInstanceId`, `Position`, and `ChunkEntityRef`, and that rejection does not spawn an entity.

---

## Phase 3: Dev Panel Placement Cutover

Replace the def-driven "spawn at origin" button with arm/cancel placement mode, current-map cursor targeting, click-to-send, and ack/reject UI state. Free-form spawning remains client-local and labeled that way.

**Files**: `crates/dev/src/panels/spawn.rs`, `crates/client/src/map.rs`, `crates/client/src/lib.rs`, `crates/client/tests/plugin.rs`

**Key changes**:
- `SpawnPanelUi { selected_object: Option<WorldObjectId>, placement: WorldObjectPlacementUi, ... }` — track armed/pending/rejected state.
- `WorldObjectPlacementUi { armed: bool, next_sequence: u32, pending: Vec<PendingWorldObjectPlacement>, last_reject: Option<WorldObjectPlacementRejectReason> }` — new UI state.
- `PendingWorldObjectPlacement { sequence: u32, object_id: WorldObjectId, base_position: Vec3 }` — pending client request.
- `pub struct PlacementTarget { base_position: Vec3, hit_normal: IVec3 }` — reusable cursor target result.
- `pub fn current_placement_target(...) -> Option<PlacementTarget>` — camera ray + current `ChunkTicket.map_entity` raycast helper.
- `fn handle_voxel_input(...)` / `fn handle_world_object_placement_input(...)` — two small systems gated by edit mode so exactly one owns each click.
- `fn handle_world_object_placement_ack(...)` / `fn handle_world_object_placement_reject(...)` — update pending UI only.

**Verify**: `cargo test -p client --test plugin`; manually run `cargo server` + `cargo client`, select a world object, arm placement, click terrain, and confirm the final object appears by replication rather than local spawn; also confirm free-form still spawns client-local only.

---

## Phase 4: Visual-Only Preview and Replication Reconciliation

Add a local preview that follows the mouse and never receives authoritative gameplay components or colliders. Keep accepted previews until matching replication arrives, then remove the preview to avoid duplicate visuals.

**Files**: `crates/dev/src/panels/spawn.rs`, `crates/client/src/world_object.rs`, `crates/client/src/map.rs`, `crates/client/tests/plugin.rs`

**Key changes**:
- `#[derive(Component)] struct WorldObjectPlacementPreview { sequence: Option<u32>, object_id: WorldObjectId }` — local-only preview marker.
- `#[derive(Component)] struct PlacementPreviewVisual;` — local visual child marker.
- `pub fn preview_visual_from_def(...) -> Option<Entity>` — visual-only helper; does not call `apply_object_components`.
- `fn update_world_object_placement_preview(...)` — move/create/despawn hover preview from `PlacementTarget`.
- `fn reconcile_placement_preview_on_replication(...)` — remove accepted preview when matching replicated `WorldObjectId`/position hydrates.

**Verify**: `cargo test -p client --test plugin`; manually verify preview follows terrain, click leaves at most one temporary preview, reject removes preview, and replicated hydration replaces the preview without duplicate committed objects.

---

## Testing Checkpoints

- After Phase 1: generated, evicted, saved, and reloaded chunk world objects have explicit base/final position semantics; no offset is inferred from persisted component count.
- After Phase 2: a client placement message can be accepted/rejected by the server; accepted objects are normal replicated chunk entities and use existing persistence tags.
- After Phase 3: the dev def-driven tab is no longer client-local; arming and clicking sends authoritative placement requests, while free-form remains local-only.
- After Phase 4: placement has a visual-only preview, ack/reject controls pending UI, and final committed objects appear only through Lightyear replication.
