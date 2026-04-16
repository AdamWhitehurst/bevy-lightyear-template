# Design Discussion

## Current State

The map transition is a loose handshake across four files with a trivially satisfied readiness check and multiple race conditions.

**Transition flow today (mid-game):**
1. UI button press sets `MapTransitionState::Transitioning`, sends `PlayerMapSwitchRequest` (`ui/src/lib.rs:484-510`)
2. Server resolves target, sends `MapTransitionStart`, moves player between rooms atomically at command-flush (`server/src/map.rs:1145-1221`)
3. Client receives `MapTransitionStart`, despawns old maps/world objects, spawns new `VoxelMapInstance`, freezes player (`client/src/map.rs:418-485`)
4. Client waits for `!chunk_levels.is_empty()` — passes after ONE column arrives (`client/src/map.rs:608`), sends `MapTransitionReady`
5. Server receives ready, sends `MapTransitionEnd` (`server/src/map.rs:1379-1413`)
6. Client unfreezes player, sets `MapTransitionState::Playing` (`client/src/map.rs:625-652`)

**Initial connection flow (separate path):**
1. `NetcodeClient` connect → `ClientState::InGame` (`ui/src/lib.rs:94-138`)
2. Server spawns character in overworld room (`server/src/gameplay.rs:352-418`)
3. Client spawns overworld `VoxelMapInstance` independently from `OnEnter(AppState::Ready)` (`client/src/map.rs:98-128`)
4. No freeze, no readiness check, no handshake. Player falls through unmeshed terrain.

**Known bugs (from `doc/bug/2026-04-14-map-transition-failures.md`):**
- H1: Readiness fires after one column — player unfreezes into unmeshed void
- H2: Client propagator has zero sources — remesh order is arbitrary, distant chunks mesh before nearby ones
- H3: Server regeneration latency not tolerated — client can wait indefinitely
- H5: Entities from new room arrive before terrain is ready
- H6: Old-room entities leak through after `RemoveSender` — no guard on visual setup handlers

## Desired End State

Two cleanly separated concerns, encapsulated in their own module:

**Client Map Transition** — changing which map a client loads and observes. Heavy, handshake-driven:
1. **Gates on spatial readiness** — chunk terrain within a radius is fully meshed and collidered before signaling ready
2. **Conditionally gates on entity arrival** — if entities were relocated as part of this transition (e.g. the character), waits for them before dismissing loading screen
3. **Prioritizes nearby chunks** — client propagator has functioning sources so `spawn_remesh_tasks` meshes closest chunks first
4. **Rejects stale entities** — old-room entities that arrive after cleanup are caught by per-handler checks and a safety-net cleanup system
5. **Unifies both paths** — initial connection and mid-game transition flow through the same state machine, sharing readiness logic and handshake
6. **Cleans up prediction state** — `VoxelPredictionState.pending` is cleared on transition start
7. **Reports progress** — loading screen shows current phase, chunks received/meshed counts
8. **Annotates server-switch seam** — code comments mark where a full server disconnect/reconnect could be inserted

**Entity Relocation** — moving any entity between maps. Lightweight, no handshake:
- Reusable server-side operation for any entity with `MapInstanceId` (characters, world objects, NPCs)
- Update `MapInstanceId`, `RemoveEntity` from old room, `AddEntity` to new room
- Lightyear handles replication changes automatically
- Can be invoked as part of a client map transition OR independently (e.g. NPC patrol, projectile crossing maps)

**Terminology:**
- **Character entity** — the gameplay entity (`CharacterMarker`, `CharacterType`, `Predicted`/`Interpolated`, physics via `CharacterPhysicsBundle`, visuals via `SpriteRig`). May or may not relocate during a client transition.
- **Client entity** — the lightyear connection entity (`ReplicationReceiver`, `PredictionManager`, `MessageSender`). Persists across transitions.
- **Client map transition** — the full handshake process: `RemoveSender`/`AddSender`, chunk loading, readiness, loading screen.
- **Entity relocation** — moving an entity between rooms/maps. Orthogonal to client transitions.

**Verification:** A client map transition is correct when:
- All chunks within radius N of spawn have `VoxelChunk` + `Mesh3d` + `Collider`
- All entities from `MapTransitionEntity` messages have resolved in `RemoteEntityMap` and exist in the `World`
- No entities with stale `MapInstanceId` exist
- Loading screen is dismissed
- `VoxelPredictionState.pending` is empty

## Patterns to Follow

**Plugin encapsulation — `protocol/src/ability/` pattern:**
- Dedicated directory with `types.rs`, per-concern system files, `mod.rs` for re-exports, `plugin.rs` for registration (`protocol/src/ability/plugin.rs:30-120`)
- Shared run-conditions bound to `let` variables and reused across `add_systems` calls (`plugin.rs:79`)
- Apply this pattern for a `transition/` module under `protocol/src/`

**Message protocol — existing `MapChannel` pattern:**
- `MapChannel` is `OrderedReliable` (`protocol/src/lib.rs:100-148`), appropriate for transition handshake messages
- `MapTransitionStart` already carries map metadata (`transition.rs:22-32`) — extend rather than replace

**Chunk pipeline completion check:**
- Full completion = absent from `chunks_needing_remesh` AND absent from `ChunkWorkTracker.remeshing` AND child `VoxelChunk` entity has `Mesh3d` + `Collider` (`voxel_map_engine/src/lifecycle.rs:1019-1193`, `protocol/src/map/colliders.rs:11-40`)
- No single flag exists; the readiness check must consult all three data structures

**Client map transition vs entity relocation — two distinct operations:**
- **Client map transition** controls what a client observes: `RemoveSender(client)` / `AddSender(client)` changes room membership, triggering chunk loading and the full handshake. This is the heavy path with loading screen and readiness gates.
- **Entity relocation** moves entities between maps: `RemoveEntity(entity)` / `AddEntity(entity)` changes which room replicates the entity. Lightyear sends `SpawnAction::Despawn` via `ActionsChannel` (`lightyear_replication/src/receive.rs:921-933`) then `SpawnAction::Spawn` when the entity enters a room visible to a client. Reusable for any entity type.
- These compose: a client transition may include zero or more entity relocations. The typical case relocates the character, but spectator/death/lobby scenarios may not.
- Current code fuses all four room ops atomically (`server/src/map.rs:1171-1186`). New design splits them: client room changes (`RemoveSender`/`AddSender`) are the transition; entity room changes (`RemoveEntity`/`AddEntity`) are relocations orchestrated by the transition when requested.
- When a character IS relocated: it despawns on client, then respawns as fresh `Predicted` + `Replicated` when `AddEntity` + `AddSender` happen in Phase 2. Setup observers fire naturally (`handle_new_character` → physics, `resolve_character_rig` → visuals, `attach_chunk_ticket_to_player` → chunk ticket). Freeze components (`RigidBodyDisabled`, `ColliderDisabled`, `DisableRollback`) are not replicated — fresh character arrives physics-enabled.
- Chunk data flows via `ChunkChannel` independent of room membership (`push_chunks_to_clients` at `server/src/map.rs:894-948` checks only `ChunkTicket`, not rooms). The only room-gated send is `VoxelEditBroadcast` (`server/src/map.rs:845-850`), which is acceptable to miss during transition.

**Anti-pattern — do NOT follow:**
- `spawn_overworld` (`client/src/map.rs:98-128`) spawns a map client-side independently of server state. The unified flow should be server-driven.
- `check_transition_chunks_loaded` (`client/src/map.rs:608`) using `chunk_levels.is_empty()` as sole criterion. Replace entirely.

## Design Decisions

1. **Spatial radius readiness (Q1):** Gate on all chunks within N columns of spawn having `VoxelChunk` + `Collider`. Aligns with server's closest-first send order (`server/src/map.rs:1001`). Avoids manifest complexity — the server's send-set is unstable (columns enter `chunk_levels` over many frames as async generation completes, `voxel_map_engine/src/lifecycle.rs:367`). The readiness radius is sent by the server in `MapTransitionStart`, computed from the player's default chunk loading radius. Client uses this value to determine its spatial readiness threshold.

2. **Client propagator sourcing (Q2):** Decouple `collect_tickets` from `update_chunks` in `voxel_map_engine` (`lifecycle.rs:488-496`) so it runs independently of chunk generation. Currently gated behind `generation_enabled` (`lib.rs:50`) which is server-only. The engine should support "ticket collection without generation" — register `collect_tickets` as a standalone system that both client and server run, while generation-specific systems remain server-gated. This activates distance-based priority in `spawn_remesh_tasks`'s `BinaryHeap<ChunkWork>` ordering (`lifecycle.rs:1019-1108`) on both sides.

3. **Late-arrival entity guard (Q3):** The split-phase room change (decision 8) eliminates most stale entity races by design — new-room entities don't replicate until after client spatial readiness, and the round-trip between phases lets in-flight old-room packets drain. The character entity despawns cleanly via lightyear. However, in-flight old-room update packets (unreliable `UpdatesChannel`) can still arrive after `RemoveSender`. Two defensive layers:
   - **Primary:** Per-handler `MapInstanceId` check in `on_world_object_replicated` (`client/src/world_object.rs:30`) and `handle_new_character` (`client/src/gameplay.rs:66`). Query `MapRegistry` for active map, compare, despawn + early return on mismatch.
   - **Safety net:** A system each frame that queries `Replicated` + `MapInstanceId` entities and despawns any whose map doesn't match the active map. Catches future handler omissions.

4. **Full path unification (Q4):** Both paths are structurally identical from the client's perspective:
   - Loading screen visible
   - `MapTransitionStart` received → client spawns `VoxelMapInstance` for new map
   - Chunks arrive via `ChunkChannel`, mesh with server-driven closest-first ordering
   - Spatial readiness met (chunks within radius have `Collider`) → send `MapTransitionReady`
   - Server does `AddSender(client)` to new room. If entities were relocated, `AddEntity` for each. Sends `MapTransitionEnd`.
   - `Complete` gate: `MapTransitionEnd` received + all `MapTransitionEntity` server entities resolved in `RemoteEntityMap` → dismiss loading screen
   Initial connect: server spawns character (with `ChunkTicket`, `server/src/gameplay.rs:409`) but defers room addition until `MapTransitionReady`, sends `MapTransitionStart` after `handle_connected`. Mid-game with character: server removes character + client from old room, later adds both to new room. Mid-game without character (future: spectator/death): server only changes client room membership. `spawn_overworld` is removed.

5. **Clear prediction state (Q5):** `VoxelPredictionState.pending.clear()` during transition start, before map cleanup. Old-map predictions cannot interfere with new-map voxel state. No `MapInstanceId` tagging needed — predictions don't survive transitions.

6. **Module structure:** New `protocol/src/transition/` directory following the ability plugin pattern.
   - `types.rs` — transition types and messages:
     - `MapTransitionStart` (extended with `readiness_radius`) — server→client, begins transition
     - `MapTransitionReady` — client→server, spatial readiness met
     - `MapTransitionEntity { unmapped_entity: Entity }` — server→client, carries a raw server-side entity ID (NOT remapped via `MapEntities`). Zero or more, one per relocated entity. Client stores these and polls `RemoteEntityMap::get_local` until all resolve.
     - `MapTransitionEnd` — server→client, terminal message. OrderedReliable on `MapChannel` guarantees all `MapTransitionEntity` messages arrive before this, sealing the entity list.
     - `TransitionPhase` enum for the client state machine.
   - `relocation.rs` — the `relocate_entity` helper and related server-side logic. Reusable outside the transition flow.
   - `client.rs` — client-side transition state machine systems.
   - `server.rs` — server-side Phase 1/Phase 2 orchestration, composing `relocate_entity` as needed.
   - `plugin.rs` — `TransitionPlugin` registration.
   - `mod.rs` — re-exports.
   The client and server crate `map.rs` files lose their transition-specific functions. `MapTransitionState` moves from `ui/src/state.rs` to this module.

7. **Transition state machine phases and gates:**

   The state machine governs the **client map transition** — the client's loading experience. Entity relocations are orchestrated by the server at the appropriate phase but are not hard-coded into the state machine itself.

   - `Idle` — no transition in progress.
     **Gate → Cleanup:** `MapTransitionStart` received from server.

   - `Cleanup` — loading screen shown. Old map despawned (mid-game) or no map yet (initial connect). `VoxelPredictionState.pending` cleared. `VoxelMapInstance` for new map spawned from `MapTransitionStart` metadata (bounds, seed, chunk_size, column_y_range). If entities were in the old room, they despawn asynchronously via lightyear — not gated on, handled by entity guards.
     **Gate → Loading:** Client-side map cleanup complete + new `VoxelMapInstance` exists in `MapRegistry`.

   - `Loading` — receiving chunks via `ChunkChannel`. Server-side `ChunkTicket` (on whatever entity drives chunk sending — typically the character, updated to target map at `server/src/map.rs:1201`) + `Position` (warped to spawn) drives `push_chunks_to_clients` (`server/src/map.rs:895`). Client doesn't need a local `ChunkTicket` to receive — `handle_chunk_data_sync` processes incoming `ChunkDataSync` into the `VoxelMapInstance`. Server sends closest-first (`server/src/map.rs:1001`).
     **Gate → Ready:** Spatial readiness — all chunks within readiness radius (from `MapTransitionStart`) of spawn position (from `MapTransitionStart.spawn_position`) have `VoxelChunk` + `Mesh3d` + `Collider`.

   - `Ready` — spatial readiness met, `MapTransitionReady` sent. Server receives this, performs Phase 2: `AddSender(client)` to new room (always), plus `AddEntity` for any relocated entities. Sends zero or more `MapTransitionEntity { unmapped_entity }` followed by `MapTransitionEnd` — all on `MapChannel` (OrderedReliable), so entity messages are guaranteed to arrive before `MapTransitionEnd`.
     **Gate → Complete:** `MapTransitionEnd` received AND all `MapTransitionEntity` server entities resolve via `RemoteEntityMap::get_local` (`lightyear_serde/src/entity_map.rs:112`) to local entities that exist in the `World`. If zero `MapTransitionEntity` messages were received, `MapTransitionEnd` alone suffices. A polling system on the client checks resolution each frame via `Query<&MessageManager, With<Client>>` → `entity_mapper.get_local(unmapped_entity)`. No timing dependency on spawn action arrival order — the poll naturally retries until the mapping exists.

   - `Complete` — dismiss loading screen → `Idle`. No client-side "unfreeze" needed — fresh entities arrive without freeze components (not replicated, per `protocol/src/lib.rs:153-204`).

   Both initial connect and mid-game transition enter at `Cleanup`. The client-side flow is identical regardless of whether entities are relocated.

8. **Split-phase server orchestration:** Server splits `execute_server_transition` (`server/src/map.rs:1145-1221`) into two phases. The client transition (`RemoveSender`/`AddSender`) is always present. Entity relocations (`RemoveEntity`/`AddEntity`) are composed in when requested.

   - **Phase 1 — on transition request:**
     - Always: `RemoveSender(client_entity)` from old room. Send `MapTransitionStart`. Update `ChunkTicket` on the chunk-driving entity to target map. Chunks start flowing via `ChunkChannel`.
     - If relocating character: `RemoveEntity(character_entity)` from old room. Update character's `MapInstanceId`, `Position`. Insert freeze components server-side.
     - If relocating other entities: call `relocate_entity` for each (same helper, different entity).
   - **Phase 1 — on initial connect:**
     - Spawn character entity with all components (not yet in any room). Set `MapInstanceId`, `ChunkTicket`. Send `MapTransitionStart`. Chunks start flowing.
   - **Phase 2 — on `MapTransitionReady` from client (all paths):**
     - `AddSender(client_entity)` to target room. If relocating entities: remove freeze components, `AddEntity` for each to target room. World objects and remote players in the room begin replicating. If no entities relocated: only `AddSender`.
     - For each relocated entity, send `MapTransitionEntity { unmapped_entity: entity }` on `MapChannel` (raw server-side entity ID, no `MapEntities` remapping). Then send `MapTransitionEnd`. All on `MapChannel` (OrderedReliable) — entity messages guaranteed to arrive before `MapTransitionEnd`.
     - No frame delay needed. The client stores the raw server entity IDs and polls `RemoteEntityMap::get_local` each frame until all resolve. The spawn actions arrive independently on `ActionsChannel` and populate `RemoteEntityMap` whenever they arrive — the polling handles any ordering naturally.

9. **Entity relocation as reusable operation:** Extract a server-side `relocate_entity` helper that works for any entity with `MapInstanceId`:
   ```
   relocate_entity(entity, old_room, new_room, target_map_id, spawn_position?)
   ```
   - `RemoveEntity(entity)` from old room
   - Update entity's `MapInstanceId`
   - `AddEntity(entity)` to new room
   - Optionally update `Position`
   This is callable from the transition system (Phase 1/2) or independently for non-transition scenarios (NPC patrol, projectile crossing maps, world object relocation). The transition system orchestrates *when* relocations happen; the relocation itself is entity-type agnostic.

## What We're NOT Doing

- **Chunk manifest protocol** — the server's send-set is unstable due to async generation. Spatial radius readiness avoids this entirely.
- **World object readiness gating** — not gating on world objects being loaded. They arrive asynchronously and pop in, which is acceptable. Only terrain chunks gate readiness.
- **Remote player arrival gating** — no mechanism exists to know expected player count (`research.md Q14`). Players appear as they replicate, which is fine.
- **Server-switch implementation** — only annotating the seam point where disconnect/reconnect would slot in (between `Cleanup` and `Loading` phases).
- **Non-character transition scenarios** — the design supports optional entity relocation (zero `MapTransitionEntity` messages before `MapTransitionEnd`), but this task only implements the character-relocation path. Spectator/death/lobby transitions are a future extension with no entity messages.
- **Streaming/progressive loading screen** — the loading screen shows phase + counts for debugging, but no progress bar or percentage. We don't know total expected counts.
- **Reworking lightyear replication internals** — no flush query API exists. We work around it with client-side guards.

## Open Risks

1. **Observer ordering for safety-net cleanup** — the cleanup system runs per-frame, not per-observer-trigger. Stale entities may exist for one frame before cleanup catches them. Per-handler checks are the real defense; cleanup is belt-and-suspenders.

2. **Initial connection unification scope** — making initial join server-driven means `spawn_overworld` goes away and the client must wait for `MapTransitionStart` before spawning ANY map. If the server is slow to send this (e.g., overworld not yet generated), the client sits at loading screen longer than today. This is correct behavior. (Accepted.)

3. **Readiness radius tuning** — too small and the player sees void at edges; too large and loading takes noticeably longer. This is a runtime-tunable const, but getting the initial value right matters. Will need playtesting.

4. **`ChunkTicket` and propagator timing** — `attach_chunk_ticket_to_player` (`client/src/map.rs:132-143`) fires on `Added<Predicted>`, and `collect_tickets` feeds the propagator. Under the new flow, the character entity only appears (via `AddSender` to new room) after spatial readiness — meaning all chunks within the readiness radius are already meshed. The propagator activates too late to help with the critical initial batch. This is acceptable: the server sends closest-first anyway (`server/src/map.rs:1001`), and the readiness-radius chunks are the ones that matter most. The propagator benefits chunks beyond the readiness radius that mesh after the loading screen dismisses.

5. **`OverworldMap` resource** — confirmed dead code (`research.md`), but removing it is a separate cleanup. Don't let it confuse the implementation.

6. **`RemoteEntityMap` as public API dependency** — the polling approach depends on `MessageManager.entity_mapper` (`lightyear_messages/src/lib.rs:84`) and `RemoteEntityMap::get_local` (`lightyear_serde/src/entity_map.rs:112`), both currently `pub` and used in lightyear's own tests (`lightyear_tests/src/client_server/replication.rs:88-94`). If a future lightyear version changes this API, the polling system would need updating. Low risk — entity mapping is fundamental to lightyear's architecture.

## Server-Switch Seam Annotation

The maximum detachment point is between `Cleanup` and `Loading` phases: old maps despawned, character entity despawned on client, prediction state cleared, loading screen visible, no `Predicted` character entity, no dependency on current server's replication state. A server-switch would:
1. Complete `Cleanup` phase on current server
2. **[SEAM]** Disconnect from current server, connect to new server, re-authenticate
3. New server spawns character entity (not yet in room) + sends `MapTransitionStart` for its map
4. Resume at `Loading` phase — identical to initial connect from here

Code should annotate this seam with comments at the phase transition boundary.
