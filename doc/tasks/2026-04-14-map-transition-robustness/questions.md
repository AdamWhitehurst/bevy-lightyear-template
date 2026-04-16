# Research Questions

## Context
Focus on the map transition handshake between client and server, the voxel chunk loading/meshing pipeline, lightyear room management and entity replication, the loading screen UI, and how these systems are organized across crates. Also examine the client's initial connection and first map load flow.

## Questions

1. How does `MapTransitionState` (in `ui/src/state.rs`) define the current transition sub-states, and what systems run in each state? What gates entry/exit between states?

2. Trace the full server-side room management flow during a transition in `server/src/map.rs` (`handle_map_switch_requests` / `execute_server_transition`): when exactly does the player entity leave the old room and enter the new room? Are room add/remove operations immediate or deferred (via `Commands`)? What ordering guarantees does lightyear provide for room membership changes vs. entity replication flush?

3. How does lightyear's replication pipeline interact with room membership changes? Specifically: after a `RoomTarget::RemoveSender` is issued, can entities from the old room still arrive on the client in subsequent frames? Is there any API to query whether all pending replication for a room/client has been flushed?

4. How does the client chunk pipeline work end-to-end: from `ChunkDataSync` receipt through `handle_chunk_data_sync` → `chunks_needing_remesh` → `spawn_remesh_tasks` → `poll_remesh_tasks` → spawned `VoxelChunk` entity with `Mesh3d` + collider? What data structures track each stage, and how can you determine that a specific set of chunks has completed the full pipeline (data received AND meshed AND spawned)?

5. How does the ticket/propagator system work in `voxel_map_engine` (`ticket.rs`, `propagator.rs`)? Where is `set_source` called, and why does it only happen inside `update_chunks` (which is server-only)? What would be needed for the client to have functioning propagator sources for remesh prioritization?

6. How does `check_transition_chunks_loaded` (`client/src/map.rs:584-623`) currently determine readiness? What information is available at that point (chunk counts, mesh entity counts, player position) that could be used for a stronger criterion?

7. How does `push_chunks_to_clients` (`server/src/map.rs:894-947`) select and order chunks for sending? What is the rate limit, how is distance calculated, and what does `ClientChunkVisibility` track? Could the server communicate a "manifest" of expected chunks to the client so the client knows what to wait for?

8. How are world object entities spawned on the server (`chunk_entities.rs:spawn_chunk_entities`) and replicated to the client (`client/src/world_object.rs:on_world_object_replicated`)? Is there any association between a world object entity and its parent chunk that could be used to gate world object visual setup until the corresponding terrain chunk is meshed?

9. How does the client clean up old map data during a transition (`handle_map_transition_start`, `despawn_foreign_world_objects`)? What entities/resources survive the cleanup, and what mechanism would prevent late-arriving replicated entities (from the old room) from being visually set up after cleanup has run?

10. What does the loading screen system (`setup_transition_loading_screen` in `ui/src/lib.rs`) currently display, and what transition state/progress data is accessible from the ECS that could be surfaced (e.g., chunks received count, chunks meshed count, world objects loaded, current transition phase)?

11. What client-side state is bound to the current lightyear server connection (replicated entities, connection resource, prediction history, room membership), and at which point in the current transition flow is the client most "detached" from server-specific state — i.e., old map data cleaned up but new map data not yet depended on?

12. How is transition logic currently distributed across crates (`client/src/map.rs`, `server/src/map.rs`, `ui/src/lib.rs`, `protocol/src/map/transition.rs`)? What state, systems, and messages belong exclusively to the transition flow vs. being shared with general map/chunk management? What patterns exist elsewhere in the project for encapsulating a cross-cutting concern into its own module or crate?

13. How does the client's initial connection and first map load work (`ClientState` transitions, initial room join, first chunk receipt)? What does it share with the mid-game map transition flow (physics setup, chunk loading, readiness checks, loading screen), and where do the two paths diverge?

14. What components make up a fully-loaded player entity on the client (physics, visuals, prediction, lightyear replication markers)? When a client enters a new room, how are other players already in that room replicated to the joining client — what triggers their setup, and is there a way to determine that all expected player entities have arrived and are fully initialized?
