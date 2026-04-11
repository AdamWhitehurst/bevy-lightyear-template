# Research Questions

## Context
Focus on: the persistence call sites across `server/src/persistence.rs`, `server/src/map.rs`, `server/src/chunk_entities.rs`, `voxel_map_engine/src/persistence.rs`, `voxel_map_engine/src/generation.rs`, and `voxel_map_engine/src/lifecycle.rs`. Also examine Bevy event/observer patterns used elsewhere in the codebase, the async task pipeline in `voxel_map_engine`, and crate dependency structure.

## Questions

1. How do current persistence consumers (chunk saves/loads in `voxel_map_engine/src/generation.rs`, `lifecycle.rs`, `persistence.rs`; map metadata in `server/src/persistence.rs`; chunk entities in `server/src/chunk_entities.rs`) invoke storage operations today -- are they called synchronously, from async tasks, or queued? What data flows in and out of each call site?

2. What data types are persisted (chunk voxels, chunk entities, map metadata, entity lists), what serialization formats do they use (bincode, zstd), and what filesystem assumptions do they make (atomic rename, directory listing via `list_saved_chunks`, path construction from `WorldSavePath` / `save_dir`)?

3. Trace how the "not found" case is handled today for each persistence operation -- when `load_chunk`, `load_chunk_entities`, or `load_map_meta` finds no file, what value is returned, and how does the caller decide to generate vs. error? How does the "generate once, save forever" invariant in `generation.rs` rely on this?

4. What Bevy event / observer / message patterns already exist in the codebase (e.g., `On<Add, ...>` observers, lightyear `MessageReader`/`MessageSender`, `EventWriter`/`EventReader`), and how are side-effect-producing operations triggered and scheduled today -- especially the debounced save systems in `server/src/map.rs`?

5. How does the async task pipeline in `voxel_map_engine` (`spawn_terrain_batch`, `spawn_features_task`, `poll_chunk_tasks`) interact with persistence -- are IO calls made inside `AsyncComputeTaskPool` tasks, and how do results get back to the ECS world? What constraints would a non-filesystem backend (e.g., network round-trip) impose on this pipeline?

6. Where do persistence-related types currently live across crates (`protocol/src/map/persistence.rs`, `server/src/persistence.rs`, `voxel_map_engine/src/persistence.rs`), and what are the dependency edges between these crates? Which crate could host a shared persistence trait without creating circular dependencies?
