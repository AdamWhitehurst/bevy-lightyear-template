# Design Discussion

## Current State

The dev plugin is feature-gated around inspector tooling: `inspector` enables egui/root menu, `world-inspector` wraps
Bevy's world inspector, and `spawn-panel` owns world-object placement UI state (`crates/dev/src/lib.rs:17-36`,
`crates/dev/src/panels/world_inspector.rs:10-23`, `crates/dev/src/panels/spawn.rs:32-55`).

Def-driven placement is already server-authoritative: the client sends
`WorldObjectPlacementRequest { sequence, object_id, base_position }`, the server resolves the controlled player's map,
validates object/chunk/bounds, spawns a replicated chunk entity, then sends ack/reject
(`crates/protocol/src/world_object/types.rs:24-58`, `crates/server/src/map.rs:756-868`).

Spawned world objects are identified by replicated `WorldObjectId`, `MapInstanceId`, replication markers, `Position`,
and `ChunkEntityRef`; clients derive visuals/colliders locally from the replicated identity and definition registry
(`crates/protocol/src/world_object/types.rs:9-14`, `crates/server/src/world_object.rs:36-43`,
`crates/server/src/world_object.rs:104-109`, `crates/client/src/world_object.rs:103-149`).

Current removal-like behavior is incidental: chunk eviction saves/despawns loaded chunk entities, client preview/stale
entities are despawned, and visual changes rebuild child visuals/colliders. There is no authoritative world-object
edit/removal request path yet (`crates/server/src/chunk_entities.rs:101-156`,
`crates/client/src/world_object.rs:228-259`).

Persistence is chunk-oriented, not entity-id-oriented: `ChunkEntityRef` connects entities to `(map_entity, chunk_pos)`,
and persisted data uses `WorldObjectSpawn` records rather than stable ECS entities
(`crates/protocol/src/map/mod.rs:34-39`, `crates/voxel_map_engine/src/config.rs:40-54`).

## Desired End State

Developers can use the spawn-panel dev tooling at runtime to:

1. select an existing replicated world object by cursor pick or nearby-object list,
2. preview moving or rotating it locally,
3. submit a move, rotate, or delete request,
4. have the server authoritatively apply the mutation and immediately queue async persistence,
5. have the mutation replicate to clients, and
6. have move, rotation, and deletion persist across chunk unload/reload, including cross-chunk moves.

Correctness is verified by server-side validation tests for edit/removal requests, client UI-state tests for selection
and pending preview operations, and persistence tests proving moved, rotated, deleted, and cross-chunk moved objects
survive chunk unload/reload correctly.

## Patterns to Follow

- Follow the existing placement request pattern: client UI state issues sequenced requests; server validates controlled
  map/chunk/object state and sends ack/reject (`crates/client/src/map.rs:432-489`, `crates/server/src/map.rs:756-824`).
- Target loaded objects by replicated `Entity` in request messages, using Bevy `MapEntities` plus Lightyear message
  `.add_map_entities()` so the client-selected entity maps to the authoritative server entity. Do not add a separate
  world-object instance identity component.
- Keep authority on the server. Client-side selection and previews are dev UX only; world-object state changes must come
  from replicated server state (`doc/tasks/2026-05-13-world-object-edit-removal-dev-plugin/research.md:1499-1503`).
- Reuse existing `trace!` early-return style for expected missing state, matching targeting/UI lifecycle paths
  (`crates/client/src/map.rs:50-76`, `doc/tasks/2026-05-13-world-object-edit-removal-dev-plugin/research.md:1505`).
- Treat client visuals/colliders as derived local state, not authoritative edit data
  (`crates/client/src/world_object.rs:103-149`, `crates/client/src/world_object.rs:228-259`).
- Persist by chunk/map ownership, not durable ECS `Entity`, because chunk persistence is already keyed through
  `ChunkEntityRef` and `WorldObjectSpawn` (`crates/protocol/src/map/mod.rs:34-39`,
  `crates/voxel_map_engine/src/config.rs:40-54`). For rotation, use the existing `persisted_components` path rather than
  adding a dedicated `WorldObjectSpawn` rotation field.

Patterns not to follow:

- Do not mutate replicated entities directly from the client; that would bypass the existing placement authority
  boundary (`doc/tasks/2026-05-13-world-object-edit-removal-dev-plugin/research.md:1499-1500`).
- Do not use arbitrary reflected component editing for this task; the chosen scope is transform-only editing plus
  deletion, leaving component mutation to existing world-inspector/free-form reflection tooling.
- Do not rely on ECS entity IDs as durable object identity across unload/reload.

## Design Decisions

1. **Editing scope**: transform-only editing plus deletion — developers can move/rotate/delete world objects; component
   editing and definition replacement are out of scope.
2. **Authority model**: client request, server mutation — mirrors placement and keeps replication/persistence coherent.
3. **Selection model**: cursor selection and nearby-object list are both in scope — cursor selection supports spatial
   editing; list selection keeps tooling usable when picking is unavailable or ambiguous.
4. **Target identity**: edit/delete requests carry the selected replicated `Entity` and use Lightyear entity mapping;
   the server rejects unmapped, missing, stale, foreign-map, or non-world-object targets.
5. **Local previews**: edit previews are in scope and should reuse placement-preview infrastructure: local-only visual
   preview entities, visual children from `preview_visual_from_def`, sequenced pending state, reject cleanup, and
   reconciliation when replicated server state matches the accepted mutation.
6. **Persistence semantics**: ack means the server applied the ECS mutation and queued async persistence, not that the
   filesystem write completed. Async persistence failures are logged only.
7. **Deletion persistence**: durable chunk-level deletion — deletion must save the chunk entity file even when empty so
   deleted/generated objects do not return after unload/reload. Missing entity file means generate features; existing
   empty entity file means authoritative empty saved state.
8. **Cross-chunk movement**: moving across chunk boundaries is in scope. The server updates `ChunkEntityRef`, removes
   the object from the old chunk's saved state, adds it to the new chunk's saved state, and queues saves for both
   chunks. If the destination chunk is unavailable, the server rejects the request with a placement-style reject reason.

## System Boundaries

- **Dev UI/client tooling** owns panel state, selection state, pending edit/remove operations, and local previews under
  the existing dev/plugin gate.
- **Client targeting** maps cursor/list selection to a candidate replicated world object and sends intent, not mutation.
- **Protocol** defines edit/remove request, ack, and reject messages on a reliable channel analogous to placement. Edit
  and remove request types that contain target `Entity` fields must implement `MapEntities` and register with
  `.add_map_entities()`.
- **Server world-object lifecycle** validates the target belongs to the controlled player's current map and loaded
  chunk, then mutates `Position`/`Rotation` or despawns the object. Cross-chunk moves update chunk ownership and reject
  unavailable destination chunks.
- **Chunk persistence layer** records durable move/rotation/delete state for affected chunks so mutations survive future
  reloads/generation.

## What We're NOT Doing

- No arbitrary reflected component editing.
- No changing a placed object's `WorldObjectId`/definition.
- No player-facing home-base/overworld editing permissions; this remains dev tooling.
- No full map editor UI, undo stack, multi-select, copy/paste, or gizmo framework.
- No new durable world-object instance identity component; loaded-object requests use mapped replicated entities, and
  persistence remains chunk-record based.

## Open Risks

- Entity picking API details need implementation-time research, but nearby-list selection is also in scope and should
  prevent picking details from blocking the feature.
- The generation path must preserve the difference between missing entity files and existing empty entity files; current
  loading code collapses both to an empty vector and would regenerate deleted objects unless changed.
- Cross-chunk persistence must save both the source and destination chunks in one accepted operation; tests should cover
  source chunk emptied, destination chunk appended, and destination chunk unavailable rejection.
- Replicated `Position`/`Rotation` mutation behavior should be verified in runtime tests, since placement currently
  creates objects but does not edit existing ones.
- Preview reconciliation for edits must avoid hiding a real object forever if a reject arrives, replication is delayed,
  or the target despawns before the matching replicated transform is observed.
