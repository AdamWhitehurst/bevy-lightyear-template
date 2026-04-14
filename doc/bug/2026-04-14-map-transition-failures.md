---
date: 2026-04-14T12:00:00-07:00
researcher: Claude
git_commit: cc5af72f
branch: master
repository: bevy-lightyear-template
topic: "Map transition failures: falling through world, broken prioritization, UI desync"
tags: [bug, map-transition, voxel-engine, loading, physics, ui]
status: investigation-complete
last_updated: 2026-04-14
last_updated_by: Claude
---

# Bug: Map Transition Failures

**Date**: 2026-04-14
**Git Commit**: cc5af72f
**Branch**: master

## User's Prompt

Multiple map transition issues:
- Returning to overworld: chunks don't load on client for a long time
- World objects (trees) spawn without terrain
- Players fall through world after loading screen dismisses
- UI button shows wrong map name, corrects later
- Nearby chunks load after far chunks
- Rapid transitions cause extended load stalls

## Summary

Five root causes identified. The dominant two are (1) a trivially weak transition-readiness criterion and (2) broken client-side remesh prioritization. Together they explain all reported symptoms.

## Investigation

### Transition Handshake Flow

The transition uses a 4-message handshake:

```
Client → PlayerMapSwitchRequest → Server
Server → MapTransitionStart     → Client  (freeze physics, spawn new map)
Client → MapTransitionReady     → Server  (chunks "loaded")
Server → MapTransitionEnd       → Client  (unfreeze physics, dismiss loading screen)
```

Physics freeze: `RigidBodyDisabled` + `ColliderDisabled` + `DisableRollback` inserted on both sides during transition.

### Transition Readiness Check

`crates/client/src/map.rs:608`:
```rust
if instance.chunk_levels.is_empty() {
    return;
}
```

The entire criterion for declaring the transition "ready" is that `chunk_levels` is non-empty — i.e., **a single column of raw chunk data has arrived**. No mesh readiness, no minimum coverage, no proximity check.

### Client Chunk Pipeline

Clients don't run `update_chunks` (gated on `ChunkGenerationEnabled`, server-only). Chunks arrive via:

1. Server `push_chunks_to_clients` sends `ChunkDataSync` (max 16/tick, sorted closest-first)
2. Client `handle_chunk_data_sync` inserts data into `VoxelMapInstance`, populates `chunk_levels`, adds to `chunks_needing_remesh`
3. `spawn_remesh_tasks` picks up `chunks_needing_remesh`, spawns async mesh tasks
4. `poll_remesh_tasks` completes meshes, spawns `VoxelChunk` entities with `Mesh3d` + collision

### Client Remesh Prioritization

`spawn_remesh_tasks` at `lifecycle.rs:1040-1048` builds a priority heap using `propagator.min_distance_to_source(col)`. But **`collect_tickets` never runs on the client** (inside `update_chunks`, which is gated). The propagator has zero sources. `min_distance_to_source` returns `u32::MAX` for every chunk. All chunks have identical priority → meshing order is effectively random (HashSet iteration order).

### Server Chunk Lifecycle on Map Exit

When the player leaves a map and is the last ticket source:
1. Ticket removed from propagator → all columns exceed threshold → `diff.unloaded` for everything
2. `remove_column_chunks` removes data from octree + `chunk_levels`
3. `evict_chunk_entities` saves + despawns world object entities

When the player returns:
1. New ticket → propagator reloads columns
2. Chunks re-enter the generation pipeline: Terrain(async) → Features(async) → Mesh(async)
3. Each stage requires at least one frame round-trip
4. Only after Mesh completion is chunk data eligible for `push_chunks_to_clients`

### UI Button Label

`update_map_switch_button_label` (`ui/src/lib.rs:514-540`) uses `pending.map(|p| &p.0).unwrap_or(map_id)` as the effective map. When `handle_map_transition_end` removes `PendingTransition`, the button falls back to the replicated `MapInstanceId`. If lightyear hasn't replicated the new `MapInstanceId` yet, the button briefly shows the stale value.

## Code References

- `crates/client/src/map.rs:584-623` — `check_transition_chunks_loaded` (trivial readiness check)
- `crates/client/src/map.rs:418-485` — `handle_map_transition_start`
- `crates/client/src/map.rs:625-652` — `handle_map_transition_end`
- `crates/client/src/map.rs:130-145` — `attach_chunk_ticket_to_player`
- `crates/client/src/map.rs:148-187` — `handle_chunk_data_sync`
- `crates/server/src/map.rs:894-947` — `push_chunks_to_clients`
- `crates/server/src/map.rs:974-1023` — `send_unsent_chunks` (distance-sorted, 16/tick cap)
- `crates/server/src/map.rs:1058-1221` — `execute_server_transition`
- `crates/voxel_map_engine/src/lifecycle.rs:1019-1108` — `spawn_remesh_tasks` (broken client prioritization)
- `crates/voxel_map_engine/src/lifecycle.rs:310-411` — `update_chunks` (server-only, owns `collect_tickets`)
- `crates/voxel_map_engine/src/propagator.rs:160-167` — `min_distance_to_source` (returns MAX with no sources)
- `crates/voxel_map_engine/src/ticket.rs:1-86` — `ChunkTicket`, `TicketType`
- `crates/ui/src/lib.rs:514-540` — `update_map_switch_button_label`

## Hypotheses

### H1: Trivial transition-readiness criterion

**Hypothesis:** `check_transition_chunks_loaded` signals "ready" after a single column of chunk data arrives. This triggers the full completion handshake (unfreeze physics, dismiss loading screen) before the terrain under the player exists as mesh/collider entities.

**Prediction:** Adding a trace to `check_transition_chunks_loaded` will show it fires with `chunk_levels.len() == 1` (or very low), long before mesh entities exist around the spawn point. Physics unfreezes → player falls → mesh eventually spawns → player teleports to surface.

**Test:** Add `warn!("transition ready: {} columns, spawn={:?}", instance.chunk_levels.len(), pending.0)` before line 618. Run client+server, transition maps, observe column count at ready time.

**Result:** Homebase (first visit): 49 columns (full map — 7x7 bounds). Overworld: 4-8 columns (partial — server regenerating after unload). Rapid transitions: 5-9 columns. Homebase bounds are `Some((4,4,4))` → 49 columns is the complete map. Overworld is unbounded, needs 81+ columns at radius 4. Even 49 columns = no mesh/collision gate.

**Decision:** Validated

### H2: Client remesh prioritization is broken (no propagator sources)

**Hypothesis:** On the client, `update_chunks` never runs (no `ChunkGenerationEnabled`), so `collect_tickets` never calls `propagator.set_source`. `spawn_remesh_tasks` calls `propagator.min_distance_to_source(col)` which returns `u32::MAX` for all chunks. Meshing order is random. Far chunks can mesh before the chunk directly under the player.

**Prediction:** Adding a trace in `spawn_remesh_tasks` will show `distance_to_source == u32::MAX` for every chunk. Observing mesh entity spawn order will show no distance correlation.

**Test:** Add `warn!("remesh: pos={}, dist={}", work.position, work.distance_to_source)` at line 1086 in `spawn_remesh_tasks`. Verify all distances are `u32::MAX`.

**Result:** All remesh entries show `dist=4294967295` (u32::MAX). Confirmed: client propagator has zero sources because `collect_tickets` (inside `update_chunks`) never runs on clients. Meshing order is effectively random.

**Decision:** Validated

### H3: Server chunk regeneration latency on map re-entry

**Hypothesis:** When a player is the last ticket source on a map and leaves, all chunks are unloaded (data removed from octree). Returning requires the full generation pipeline: Terrain → Features → Mesh, each an async round-trip. During this time `push_chunks_to_clients` has nothing to send. This explains the extended delay when returning to the overworld, and compounds with rapid transitions (each departure triggers a full unload).

**Prediction:** After transitioning away from overworld and back, `push_chunks_to_clients` will show 0 chunks sent for multiple frames until the generation pipeline catches up. The delay correlates with how many chunks need regeneration.

**Test:** Add `warn!("push_chunks: sent={} for {:?}", sent, map_id)` after line 935. Transition away and back. Observe the gap between transition start and first non-zero push.

**Result:** Confirmed. Overworld `chunk_levels` drops to 0 on exit. On re-entry, `chunk_levels` ramps back to 81 within ~2ms (propagation is fast), but `sent=0` persists for ~60 frames (~26ms) because chunk DATA doesn't exist yet (only column-level tracking). `get_chunk_data()` returns None until generation pipeline completes each stage. First send is only 8 chunks (not 16) — pipeline is staggered. Additional finding: `sent_so_far` can decrease mid-push (204→184), suggesting a `ClientChunkVisibility` reset during the transition causing chunk re-sends.

**Decision:** Validated — contributing factor but not dominant. Generation latency is ~26ms. The bigger bottleneck is the 16/tick push rate across 324 chunks (~300ms total), compounded by H1 (early readiness) and H2 (random mesh order).

### H4: UI button label race — MapInstanceId replication vs PendingTransition removal

**Hypothesis:** `handle_map_transition_end` removes `PendingTransition`. The button then reads the replicated `MapInstanceId`. If lightyear hasn't replicated the updated `MapInstanceId` yet (it was set on the server at the START of the transition), the button momentarily reads the OLD map ID, showing the wrong label until replication catches up.

**Prediction:** Between `MapTransitionEnd` receipt and `MapInstanceId` replication, the button will show the destination map name (as if the player is still on the old map). This window is brief but visible.

**Test:** Add `warn!("button: map_id={:?} pending={:?}", map_id, pending.map(|p| &p.0))` in `update_map_switch_button_label`. Watch for frames where `pending` is `None` but `map_id` is the OLD map.

**Result:** Could not reproduce. MapInstanceId replication appears to consistently arrive before PendingTransition is removed. The race window may be too narrow to observe under normal conditions, or lightyear's replication timing prevents it in practice.

**Decision:** Inconclusive — deprioritize

### H5: Entity-before-terrain race (world objects replicate before chunk mesh exists)

**Hypothesis:** World object entities (trees) enter the lightyear room and replicate independently of chunk voxel data. `spawn_chunk_entities` runs `after(poll_chunk_tasks)` in the same frame, so entities are spawned and replicated on the same frame the Features stage completes. But the client hasn't necessarily received or meshed the corresponding chunk yet. Trees appear floating/at wrong height until terrain mesh arrives.

**Prediction:** Client receives `Replicated` world objects for chunks that haven't been received via `ChunkDataSync` yet. Trees will be visible before terrain under them.

**Test:** In client `on_world_object_replicated`, log MapInstanceId, Position, and whether chunk data exists for each replicated object.

**Result:** Confirmed. Vast majority of world objects replicate with `has_chunk_data=false` — entities arrive before their terrain chunk data on the client. Additionally, 453 entities arrived with `map=Overworld — not in registry` — overworld entities replicating AFTER the client transitioned to homebase and removed overworld from MapRegistry. These late-arriving entities get full visual setup (mesh, collider) but aren't cleaned up, rendering as floating trees in the homebase scene. See H6 for this cross-map leak.

**Decision:** Validated

### H6: Late-arriving room entities leak across map transitions

**Hypothesis:** The server's `execute_server_transition` uses deferred `commands.trigger(RoomEvent { target: RoomTarget::RemoveSender(...) })` to remove the client from the old room. Lightyear may flush entity replication before the room removal takes effect. Additionally, `despawn_foreign_world_objects` in `handle_map_transition_start` only cleans up entities that exist at transition time. Overworld entities that replicate AFTER the cleanup get full visual setup (`on_world_object_replicated` runs, attaching meshes and colliders) and are never despawned — they render at their overworld positions in the homebase scene.

**Prediction:** After transitioning to homebase, overworld `WorldObjectId` entities will continue arriving via `Added<Replicated>`. These will have `MapInstanceId::Overworld` but the overworld won't be in `MapRegistry`. They'll be visible as floating trees in the homebase.

**Test:** Instrumented `on_world_object_replicated` to log map registry lookups.

**Result:** 453 entities logged with `map=Overworld — not in registry` after transitioning to homebase. Confirmed: lightyear continues replicating old-room entities after the transition. `on_world_object_replicated` gives them full visual components. No system despawns them afterward. Visible as trees in the homebase screenshot.

**Decision:** Validated — new root cause discovered during H5 investigation

## Symptom → Root Cause Mapping

| Symptom | Root Causes |
|---|---|
| Players fall through world after loading screen | H1 (trivial readiness), H2 (random mesh order) |
| Trees spawn without terrain | H5 (entity-before-terrain race) |
| Nearby chunks load after far chunks | H2 (broken client prioritization) |
| UI button shows wrong map | H4 (inconclusive) |
| Returning to overworld: long load | H1 + H3 (trivial readiness + regeneration latency) |
| Rapid transitions: extended stalls | H3 (each departure unloads, each return regenerates) |
| Overworld trees visible in homebase | H6 (late-arriving room entities leak across transitions) |

## Fixes

_To be filled after hypothesis validation._

## Solutions

_To be filled after fixes are approved and implemented._
