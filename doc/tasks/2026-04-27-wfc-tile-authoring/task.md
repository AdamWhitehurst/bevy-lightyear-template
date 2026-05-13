# Task

Add Wave Function Collapse (WFC) as a new procedural map-generation technique alongside the existing flat and heightmap generators, and ship a 3D tile authoring tool used to define the WFC tile set (modules, sockets/adjacency rules, weights, rotations).

The terrain asset format must expose generation type as an explicit, selectable choice so a `.terrain.ron` can opt into WFC and reference an authored tile set.
