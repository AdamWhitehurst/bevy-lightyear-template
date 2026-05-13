# Research Questions

## Context

Focus on the development tooling in `crates/dev`, the shared world-object definition and spawn code in `crates/protocol/src/world_object/`, and the server/client map and world-object lifecycle in `crates/server`, `crates/client`, and `crates/voxel_map_engine`. Also examine the existing input-to-world-coordinate, replication, map-scoping, and persistence flows that connect these systems.

## Questions

1. Trace the current dev spawn panel flow from UI interaction to entity creation: which plugins, resources, feature gates, marker components, object registries, and helper functions are involved, and how does the resulting entity differ from ordinary runtime world objects?
2. How are world-object definitions loaded, registered, represented, and applied at runtime — including `WorldObjectId`, reflected components, `PlacementOffset`, spawn-only versus persistent component markers, and the contract of `apply_object_components`?
3. Trace the generated world-object lifecycle from terrain placement rules through `WorldObjectSpawn`, pending chunk entity queues, `spawn_world_object`, replication setup, client hydration, and eventual despawn or transformation.
4. How does the protocol layer register world-object and map-related components/messages for networking, and what existing client-to-server request/ack/reject patterns are used for authoritative world mutations such as voxel edits?
5. How does the server determine map scope, room visibility, and authority for map mutations and replicated entities — including `MapInstanceId`, `RoomRegistry`, chunk visibility, transitions, and validation of client-originated requests?
6. How does the client convert mouse or cursor state into world-space targets today, including camera ray construction, voxel raycasts, input actions, prediction state, rollback/rejection handling, and any existing pointer/gizmo rendering conventions?
7. How are replicated world objects hydrated on the client, and which components or child entities are local-only visuals/colliders versus authoritative replicated state?
8. How are world objects persisted and restored across chunk eviction, periodic saves, shutdown saves, and map reloads, and what distinguishes per-chunk entity files from map-level entity persistence?
9. What automated tests or reusable test harnesses currently cover dev plugins, world-object replication, voxel/map persistence, chunk sync, map transitions, and multi-client behavior, and what scenarios do they exercise?
