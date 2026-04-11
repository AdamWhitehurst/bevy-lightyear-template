# Research Questions

## Context
Focus on the `voxel_map_engine` crate (types, config, lifecycle, generation, meshing, placement, API) and how chunk coordinates flow through the protocol crate's map/chunk networking. Also examine the terrain definition system and how per-map ECS components are applied.

## Questions

1. Where is `CHUNK_SIZE` (and derived constants like `PADDED_CHUNK_SIZE`, `PaddedChunkShape`) referenced across the entire workspace? For each usage site, is the value used for coordinate math, memory allocation, mesh generation, or something else? Which usages are in hot paths vs. one-time setup?

2. How does `PalettedChunk` allocate and index its voxel storage? Trace from chunk creation through terrain fill to voxel read/write — where does the storage size come from, and is it derived from `CHUNK_SIZE` or from the ndshape type parameter?

3. How does the chunk lifecycle system (`lifecycle.rs`) convert between world-space positions and chunk coordinates? Trace the full flow from `ChunkTicket` position through column expansion to individual chunk `IVec3` positions. Where does chunk size enter each conversion?

4. How does the meshing pipeline (`meshing.rs`) use chunk dimensions? Trace from `ChunkData` input through greedy meshing to final `Mesh` output — where are chunk size assumptions embedded in vertex positions, UV coordinates, or buffer sizes?

5. How does the terrain generation pipeline (`terrain.rs`, `generation.rs`) use chunk size? Look at noise coordinate scaling, surface height map dimensions, and feature placement bounds.

6. How does the chunk networking system (`ChunkDataSync`, `UnloadColumn`) encode and decode chunk positions? Does the client need to know the chunk size of a remote map to correctly interpret positions?

7. How does `VoxelMapConfig` get created and applied to map entities? Trace the flow from map spawn (server) through terrain def application to client-side config replication. What fields does it carry, and how would a new field propagate?

8. How does the `VoxelWorld` API (`api.rs`) resolve voxel positions to chunk coordinates and local offsets? What would break if two simultaneously-loaded maps used different chunk sizes?

9. How does chunk collider generation use chunk dimensions? Trace from voxel data through collider shape creation — are collider bounds derived from the chunk size constant or from the mesh geometry?
