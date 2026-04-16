# Structure Outline

## Approach

Replace the scattered transition code with a unified `protocol/src/transition/` module that implements a five-phase client state machine (Idle→Cleanup→Loading→Ready→Complete→Idle), split-phase server orchestration, spatial-radius readiness gating, and client-side propagator activation — serving both initial connection and mid-game transitions through the same path.

## Phase 1: Foundation Types and Module Scaffold

Stand up the `transition/` module with all shared types, messages, and the plugin shell. No behavior changes — existing code keeps working.

**Files**:
- `protocol/src/transition/` (new directory): `mod.rs`, `types.rs`, `plugin.rs`
- `protocol/src/map/transition.rs` (modify — re-export from new module)
- `protocol/src/lib.rs` (register `TransitionPlugin`)

**Key changes**:
- `TransitionPhase` enum: `Idle`, `Cleanup`, `Loading`, `Ready`, `Complete`
- `ClientTransitionState` resource wrapping `TransitionPhase` + `readiness_radius: u32` + `spawn_position: Vec3` + `pending_entities: Vec<Entity>` + `end_received: bool`
- `MapTransitionStart` extended: add `readiness_radius: u32`, `spawn_position: Vec3`
- `MapTransitionEntity { unmapped_entity: Entity }` — new message, `MapChannel`
- `TransitionPlugin` — registers messages and channels, no systems yet
- `protocol/src/map/transition.rs` becomes thin re-exports from `transition/types.rs`

**Verify**: `cargo check-all` passes. No runtime behavior change.

---

## Phase 2: Client Propagator Activation

Decouple `collect_tickets` from `update_chunks` so it runs on both client and server. This enables distance-based remesh priority on the client.

**Files**:
- `voxel_map_engine/src/lifecycle.rs` (extract `collect_tickets` to `pub fn`)
- `voxel_map_engine/src/lib.rs` (register `collect_tickets` as standalone system, unconditional)

**Key changes**:
- `pub fn collect_tickets(...)` — extracted as standalone system with own query params (currently private, called inside `update_chunks`)
- `update_chunks` loses its `collect_tickets` call
- `VoxelMapEnginePlugin` registers `collect_tickets` in `Update` unconditionally (not gated on `generation_enabled`), ordered before `spawn_remesh_tasks`
- `update_chunks` + `poll_chunk_tasks` remain gated on `generation_enabled`

**Verify**: `cargo check-all` passes. `cargo test-all` passes. Run `cargo client` + `cargo server` — chunks should still mesh closest-first on client (propagator now has sources).

---

## Phase 3: Spatial Readiness + Client State Machine

Replace `check_transition_chunks_loaded` with spatial-radius readiness. Wire the client-side state machine (Cleanup→Loading→Ready→Complete). Loading screen shows phase info.

**Files**:
- `protocol/src/transition/client.rs` (new — client state machine systems)
- `protocol/src/transition/plugin.rs` (register client systems)
- `client/src/map.rs` (remove `check_transition_chunks_loaded`, `handle_map_transition_end`; keep `handle_map_transition_start` temporarily)
- `ui/src/lib.rs` (update loading screen to show phase + chunk counts)
- `ui/src/state.rs` (`MapTransitionState` kept for now, driven by new state machine)

**Key changes**:
- `fn check_spatial_readiness(registry, map_query, state) -> bool` — queries all chunk positions within `readiness_radius` of `spawn_position`, checks each has child `VoxelChunk` with `Mesh3d` + `Collider`
- `fn update_transition_state(...)` — `Update` system: evaluates gates per phase, advances `ClientTransitionState.phase`
- `fn handle_transition_entities(...)` — accumulates `MapTransitionEntity` messages, polls `RemoteEntityMap::get_local` each frame
- `fn on_transition_complete(...)` — dismisses loading screen, sets `MapTransitionState::Playing`
- Loading screen text updated to show: phase name, chunks received/meshed counts

**Verify**: `cargo check-all` passes. `cargo client` + `cargo server` — mid-game transition: loading screen stays up until nearby terrain is meshed+collidered. Player prompt for manual test.

---

## Phase 4: Split-Phase Server Orchestration + Entity Relocation

Replace `execute_server_transition` with Phase 1/Phase 2 split. Extract `relocate_entity`. Server sends `MapTransitionEntity` + `MapTransitionEnd` in Phase 2.

**Files**:
- `protocol/src/transition/server.rs` (new — server orchestration)
- `protocol/src/transition/relocation.rs` (new — `relocate_entity` helper)
- `protocol/src/transition/plugin.rs` (register server systems)
- `server/src/map.rs` (remove `execute_server_transition`, `handle_map_transition_ready`; keep `handle_map_switch_requests` temporarily as caller)
- `server/src/gameplay.rs` (modify `handle_connected` — defer room addition)

**Key changes**:
- `fn relocate_entity(commands, entity, old_room, new_room, target_map_id, spawn_position?)` — `RemoveEntity` from old, update `MapInstanceId`, optionally update `Position`, `AddEntity` to new
- `fn server_transition_phase1(...)` — `RemoveSender(client)` from old room, send `MapTransitionStart` (with `readiness_radius`, `spawn_position`), update `ChunkTicket`. If relocating: `RemoveEntity(character)`, update `MapInstanceId`/`Position`, insert freeze components
- `fn server_transition_phase2(...)` — on `MapTransitionReady`: `AddSender(client)` to new room. If entities relocated: remove freeze, `AddEntity` each. Send `MapTransitionEntity` per relocated entity, then `MapTransitionEnd`. All on `MapChannel`
- `handle_connected` modified: spawn character (not in room), send `MapTransitionStart`, defer `AddSender`+`AddEntity` to Phase 2

**Verify**: `cargo check-all` + `cargo test-all` pass. `cargo client` + `cargo server` — mid-game transition uses split phases. Initial connect deferred to Phase 5. Player prompt for manual test.

---

## Phase 5: Unified Initial Connection Path

Route initial connection through the same transition state machine. Remove `spawn_overworld`. Both paths are now identical from client perspective.

**Files**:
- `client/src/map.rs` (remove `spawn_overworld`, migrate remaining `handle_map_transition_start` logic into `transition/client.rs`)
- `protocol/src/transition/client.rs` (absorb cleanup logic from old `handle_map_transition_start`)
- `ui/src/lib.rs` (remove `on_client_connected` direct `InGame` transition; connection enters transition flow)
- `ui/src/state.rs` (simplify — `MapTransitionState` may become internal to transition module)

**Key changes**:
- `fn on_transition_start_received(...)` — handles `MapTransitionStart` for both paths: show loading screen, despawn old maps (if any), clear `VoxelPredictionState.pending`, spawn new `VoxelMapInstance`, advance to `Cleanup` phase
- `spawn_overworld` removed — client no longer spawns maps independently
- Initial connect flow: server sends `MapTransitionStart` after `handle_connected` → client enters `Cleanup` → same flow as mid-game
- `VoxelPredictionState.pending.clear()` on every transition start

**Verify**: `cargo check-all` + `cargo test-all` pass. `cargo client` + `cargo server` — fresh connect goes through loading screen with readiness gating. Mid-game transition still works. Player prompt for manual test.

---

## Phase 6: Entity Guards and Stale Entity Cleanup

Add per-handler `MapInstanceId` checks and safety-net cleanup system. Close the late-arrival vulnerability.

**Files**:
- `client/src/world_object.rs` (add `MapInstanceId` check to `on_world_object_replicated`)
- `client/src/gameplay.rs` (add `MapInstanceId` check to `handle_new_character`)
- `protocol/src/transition/client.rs` (add safety-net cleanup system)
- `protocol/src/transition/plugin.rs` (register cleanup system)

**Key changes**:
- `on_world_object_replicated`: query `MapRegistry` for active map, compare entity's `MapInstanceId`, despawn + early return on mismatch
- `handle_new_character`: same guard pattern
- `fn cleanup_stale_map_entities(...)` — per-frame system: query all `Replicated` + `MapInstanceId` entities, despawn any whose map doesn't match active map in `MapRegistry`. Safety net for future handler omissions

**Verify**: `cargo check-all` + `cargo test-all` pass. `cargo client` + `cargo server` — transition repeatedly between maps, check no stale entities visible. Player prompt for manual test.

---

## Phase 7: Cleanup and Polish

Remove dead code, migrate `MapTransitionState` ownership, add server-switch seam annotations.

**Files**:
- `client/src/map.rs` (remove remaining transition-specific dead code)
- `server/src/map.rs` (remove remaining transition-specific dead code)
- `ui/src/state.rs` (clean up `MapTransitionState` if moved)
- `protocol/src/transition/client.rs` (add `// [SERVER-SWITCH SEAM]` comment at Cleanup→Loading boundary)
- `README.md` (update if transition architecture is documented)

**Key changes**:
- Remove dead transition functions from `client/src/map.rs` and `server/src/map.rs` that were replaced in earlier phases
- `OverworldMap` resource: leave as-is (separate cleanup per design)
- Seam annotation: `// [SERVER-SWITCH SEAM] — disconnect current server, connect to new, re-auth, resume at Loading phase`
- README updates if applicable

**Verify**: `cargo check-all` + `cargo test-all` pass. Full manual test: fresh connect, mid-game transition, rapid transitions. No regressions.

## Testing Checkpoints

After each phase, this should be true cumulatively:

| Phase | Checkpoint |
|-------|-----------|
| 1 | New `transition/` module exists, compiles, no behavior change |
| 2 | Client propagator has sources; `spawn_remesh_tasks` orders by distance on client |
| 3 | Mid-game transition waits for spatial readiness (chunks within radius have Mesh3d+Collider) before sending Ready |
| 4 | Server splits room changes into Phase 1 (remove) and Phase 2 (add after Ready). `MapTransitionEntity` messages sent. Entity relocation is a reusable helper |
| 5 | Initial connection and mid-game transition use identical client-side flow. `spawn_overworld` is gone. `VoxelPredictionState.pending` cleared on every transition |
| 6 | Old-room entities arriving after transition are caught by per-handler checks and safety-net cleanup |
| 7 | No dead transition code remains in `client/src/map.rs` or `server/src/map.rs`. Seam annotated |
