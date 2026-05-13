# Design Discussion

## Current State

- The dev spawn panel's definition-driven world-object path is intentionally client-local. It spawns `WorldObjectId`, `Transform::default()`, `DevSpawned`, hardcoded `MapInstanceId::Overworld`, and applies reflected definition components directly via `apply_object_components` (`crates/dev/src/panels/spawn.rs`).
- Free-form spawning is a separate client-local tool for arbitrary reflected components. It cannot safely become authoritative because the server cannot trust arbitrary client-selected components.
- Runtime world objects are server-authored through `spawn_world_object`, which inserts `MapInstanceId`, `Replicate`, and `NetworkVisibility`; the `MapInstanceId` observer moves the entity into the matching Lightyear room.
- Generated world objects become persistent by receiving `Position` and `ChunkEntityRef { chunk_pos, map_entity }`. Existing chunk-entity eviction, periodic save, and shutdown save paths serialize `ChunkEntityRef + WorldObjectId + Position` back into per-chunk `WorldObjectSpawn` files.
- Client hydration already expects authoritative replication: `on_world_object_replicated` reacts to `Added<Replicated>`, looks up the local `WorldObjectDefRegistry`, applies definition components, inserts a collider/`Transform`, and attaches local visuals.
- Client cursor-to-world targeting already exists for voxel edits: primary-window cursor + `Camera3d` produce a `Ray3d`, raycast against the controlled player's current `ChunkTicket.map_entity`, then send an ordered reliable request.
- Voxel edits are the only existing client-originated authoritative world mutation pattern: client sends request with sequence, server derives map scope from the controlled character, validates, applies server state, and sends ack/reject.

## Desired End State

The def-driven world-object tab becomes an authoritative placement workflow:

1. User selects a registered `WorldObjectId`.
2. User arms placement mode.
3. Client shows a local preview under the mouse using current-map raycast targeting.
4. Click sends a placement request to the server with a sequence, object id, and base placement point.
5. Server derives map scope from the requesting client's controlled character, validates the request, spawns via `spawn_world_object`, inserts authoritative `Position` and `ChunkEntityRef`, and sends ack/reject.
6. Other clients see the object through normal Lightyear replication and existing client hydration.
7. The object is saved by the existing per-chunk world-object persistence systems.

Free-form spawning remains explicitly client-local and labeled as such.

## Vision Alignment

This supports the vision's Stage Editing and world-structure split: admins can edit the shared Overworld, and the same server-authored/persistent pattern can later be constrained for player Home-Base editing. It aligns with Open-World Exploration and Living Home-Base customization by making placed objects part of the real world instead of a private client illusion.

## Patterns to Follow

- **Authority boundary**: follow `handle_voxel_edit_requests`: server resolves the player's current map from controlled character state; the request must not choose authoritative map scope.
- **Replication**: reuse `spawn_world_object` and `MapInstanceId` room routing. Do not add a custom success broadcast carrying the object; replication is the source of truth.
- **Persistence**: make placed entities indistinguishable from generated entities for persistence by inserting `ChunkEntityRef` and saving through chunk-entity systems.
- **Targeting**: reuse the client voxel raycast model: primary-window cursor, camera viewport correction, controlled player's `ChunkTicket.map_entity`, solid-voxel hit, and hit normal.
- **Client lifecycle**: ack/reject only drives preview/pending UI. The final object appears only when the replicated entity hydrates.
- **Early exits**: every expected missing state path in new Rust systems uses `trace!` before `return`/`continue`.
- **Lightyear protocol contract**: Lightyear's official book describes client-server networking where the server is authoritative and the protocol defines messages, components, and channels. The design keeps placement as a message contract and committed objects as replicated components/entities.

## Design Decisions

1. **Clean cutover for def-driven world-object spawn.** Replace immediate client-local def-driven object creation with placement arming + request/preview. Keeping both paths would preserve the bug-prone behavior this task is meant to remove.

2. **Free-form spawn stays client-local.** Arbitrary reflected component spawning is a dev-only ECS scratchpad, not a safe network protocol. The UI should keep the local-only warning for that tab.

3. **Dedicated placement protocol.** Add an ordered reliable bidirectional world-object placement channel and messages, e.g. `WorldObjectPlacementRequest`, `WorldObjectPlacementAck`, and `WorldObjectPlacementReject`. Reusing `VoxelChannel` would blur a voxel-specific contract.

4. **Server-derived map scope.** The placement request carries no authoritative map id. If a map id is included for diagnostics/UI correlation, the server treats it as advisory only and derives the real map from the client's controlled character.

5. **Base-position request semantics.** The client sends the un-offset placement base point. The server applies `PlacementOffset` exactly once before inserting `Position`, matching fresh generated-object semantics. The preview may display the final visual offset for usability, but it must not send that offset-adjusted value as the base.

6. **Preview-only entity.** The client preview must not call `apply_object_components` on a normal entity. It should use only local preview marker/components and visual-only mesh/material data, avoiding authoritative gameplay components, colliders, and normal world-object hydration systems.

7. **Ack/reject does not commit objects.** Ack marks the pending preview as accepted or waiting for replication. Reject despawns the preview and records the reason. The final object is created only by server replication.

8. **Chunk persistence via normal tags.** On accepted placement, server computes the owning chunk from the placed world position and map dimensions, inserts `ChunkEntityRef`, and relies on existing periodic/eviction/shutdown save systems. Do not write a one-object chunk file immediately unless the implementation merges all current chunk objects.

9. **Fix position persistence semantics before relying on placement persistence.** Current reload detection infers "fresh versus persisted" from `persisted_components.is_empty()`, which can double-apply `PlacementOffset` for saved objects with no persisted components. Add explicit position semantics to `WorldObjectSpawn` (for example `position_kind: PlacementBase | Final`, with generated spawns defaulting to `PlacementBase` and saved entities written as `Final`) or an equivalent explicit marker. `extract_placement_offset` must use that explicit state, not component-list emptiness.

10. **Placement mode suppresses voxel editing.** Left click is already bound to voxel placement. While object placement is armed, the placement system owns the click and voxel editing must not also fire for the same input.

## What We're NOT Doing

- No authoritative free-form component spawning.
- No client-side committed world object on ack.
- No custom server-to-client placement broadcast duplicating entity replication.
- No trusting client-supplied `MapInstanceId` for authority.
- No map-level entity persistence for placed world objects; they remain per-chunk entities.
- No general player-facing editing permissions yet. This is dev tooling; production permissions are a later design.

## Open Risks

- **Preview matching.** Keeping the preview visible until replication avoids flicker, but can briefly duplicate visuals unless the client matches pending placements to hydrated replicated objects and removes the preview.
- **Old saved data.** Existing saved chunk-entity files do not encode explicit position semantics. A migration or compatibility heuristic may be needed if existing saves matter.
- **Placement validation depth.** Minimum validation is known object id, controlled player/map existence, finite position, map bounds, and loaded/reachable chunk. Collision/overlap rules are a gameplay-policy decision and can remain permissive for dev tooling.
- **Module ownership.** Targeting helpers currently live in `client::map` and visual helpers in `client::world_object`. The implementation should either expose small reusable helpers or colocate preview systems where private visual code can be reused without copying large logic.
