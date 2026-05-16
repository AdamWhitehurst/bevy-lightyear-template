# Research Questions

## Context

Focus on the dev editing UI, voxel map mutation APIs, chunk lifecycle/remeshing, protocol messages, and server/client synchronization paths. Pay attention to how voxel materials are represented, how map instances are selected, how edit history is modeled, how UI modules are organized, and how multi-voxel edits propagate through loaded chunks and network broadcasts.

## Questions

1. How does the Dev plugin structure editing modes, panel state, and panel modules, and where is terrain-specific UI or input currently routed relative to world-object placement and selection?
2. What conventions exist for keeping Dev plugin panels split across files, registering their resources/systems, and exposing panel state through `lib.rs` or `mod.rs`?
3. How do client-side systems currently create, send, acknowledge, reject, and locally track voxel edit requests, including sequence numbers and pending state?
4. How does the server resolve a client's active map, validate voxel edits, apply them to `VoxelWorld`, mark persistence state dirty, and queue broadcasts?
5. How do `VoxelWorld` and `VoxelMapInstance` locate chunk/local voxel coordinates, read existing voxel data, mutate voxel data, update boundary padding, mark chunks dirty, and schedule remeshing for single-voxel edits?
6. How do chunk lifecycle systems consume `chunks_needing_remesh`, generate replacement meshes, update spawned chunk entities, and persist dirty chunk data?
7. How are voxel material IDs and terrain definitions represented across assets, `WorldVoxel`, `VoxelType`, meshing merge values, and any UI-visible registries?
8. What semantics already exist in protocol/server code for sending multiple voxel changes from one update cycle, and how do they handle ordering, per-chunk grouping, originator exclusion, and cross-map room routing?
9. What assumptions in tests and examples cover voxel editing, remeshing, map-instance isolation, boundary padding, and multi-change broadcasts, and where are the gaps for multi-voxel area/volume edits or cross-chunk edits?
10. How could existing voxel mutation paths represent one logical edit over arbitrary world-space voxel positions, including edits that touch multiple chunks, without requiring callers to manage per-voxel chunk bookkeeping?
11. How do raycast and cursor-to-world flows identify target voxels or adjacent placement positions, and what coordinate conventions distinguish changing an existing voxel from adding a voxel next to a hit surface?
12. What undo/redo or reversible action patterns already exist in the codebase, and how do they store prior state, replay changes, and interact with authoritative server state?
13. How does this project separate authoritative server edits from client-local preview or dev tooling state, especially for admin overworld editing versus home-base editing?
