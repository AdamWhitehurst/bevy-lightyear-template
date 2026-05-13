# Research Questions

## Context
Focus on the terrain asset pipeline in `crates/protocol/src/terrain/` and the voxel generation pipeline in `crates/voxel_map_engine/`. Also look at how chunk generation, meshing, and persistence currently fit together, what dev/authoring tooling exists today (`crates/dev`, any bin targets), and how RON-driven reflected components are loaded and consumed.

## Questions

1. Trace the full path from a `.terrain.ron` file on disk to a runtime `VoxelGenerator`: how does `TerrainDefLoader` deserialize components, how does `build_generator_from_components` decide between `FlatGenerator` and `HeightmapGenerator`, and what is the contract a type must satisfy to participate as a generator component?

2. What does the `VoxelGeneratorImpl` trait require, what data does the async chunk pipeline (`spawn_terrain_batch`, `spawn_features_task`, `spawn_mesh_task`, `build_surface_height_map`) feed into and read out of generators, and what assumptions does it make about determinism, chunk-locality, and ordering between neighboring chunks?

3. How does the engine model a single voxel and a chunk's contents — what is stored in `PalettedChunk`, how is the palette keyed, and what defines a voxel "type" or material today (sources of truth, registries, asset references)?

4. What is the difference in runtime behavior between the bounded `homebase.terrain.ron` (no generation components, fixed `MapDimensions`) and the unbounded `overworld.terrain.ron` (heightmap/biome components) — specifically how `MapDimensions`, chunk ticketing, and streaming change between the two?

5. How are randomness and seeding handled across the existing generation code — where do seeds originate (`NoiseDef`, terrain asset, runtime), how are they propagated into per-chunk work, and how is reproducibility maintained between client and server?

6. What asset categories beyond `TerrainDef` exist in the protocol crate (vox models, world objects, abilities), how are they discovered/loaded/registered, and what naming and folder conventions do they follow under `assets/`?

7. What dev-time and authoring infrastructure exists in the repo today — bin targets in `Cargo.toml`, the `crates/dev` plugin, debug rendering, gizmos, picking, editor-like camera controllers — and what would a new standalone or in-app authoring binary be able to reuse versus need fresh?

8. How does chunk persistence work (`FsChunkStore`, `FsChunkEntitiesStore`) and how does it interact with re-generation — i.e., once a chunk is generated and saved, under what conditions is the generator re-invoked, and how would a deterministic-but-bounded generation strategy fit that lifecycle?

9. What WFC, tile-graph, or constraint-solver crates are already present in `Cargo.lock` or the `git/` submodules, and what voxel-mesh primitives (block-mesh, surface-nets, height-mesh) are currently wired up that a module-based tile renderer could build on?
