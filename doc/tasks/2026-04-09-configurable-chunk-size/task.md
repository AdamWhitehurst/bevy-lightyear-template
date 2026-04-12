# Task: Configurable Chunk Size Per Map

Make the world chunk size configurable per map instance rather than a global compile-time constant. Each map type (overworld, homebase, arena) should be able to specify its own chunk dimensions, allowing maps to use different voxel granularities (e.g., arenas might use smaller chunks for finer detail, overworld might use larger chunks for performance).
