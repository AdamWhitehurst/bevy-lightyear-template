---
date: 2026-03-09T11:55:23-07:00
researcher: claude
git_commit: 28300b3b717593e19f7b5f11f2b92e50be8e7568
branch: master
repository: bevy-lightyear-template
topic: "Minecraft-style map-as-directory saving: chunks, entities, and map metadata"
tags: [research, persistence, map-saving, chunks, entities, minecraft, voxel-world]
status: complete
last_updated: 2026-03-09
last_updated_by: claude
last_updated_note: "Added follow-up research on Minecraft in-memory chunk storage and memory reduction techniques"
---

# Research: Minecraft-style Map-as-Directory Saving

**Date**: 2026-03-09T11:55:23-07:00
**Researcher**: claude
**Git Commit**: 28300b3b717593e19f7b5f11f2b92e50be8e7568
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How Minecraft saves worlds as directories and how to implement saving the entire map terrain (per-chunk), entities in that map, and any other map-relevant data (like spawn points) — rather than just edits to the terrain.

## Summary

Minecraft saves each world as a self-contained directory containing: terrain chunks grouped into region files (32×32 chunks per `.mca` file), entities in separate region files (since 1.17), and global metadata in `level.dat`. The current project saves only a flat list of voxel modifications to a single bincode file — no per-chunk storage, no entity persistence, no map metadata. This document covers Minecraft's format in detail and maps it to the project's existing architecture to inform a directory-based save system.

## Minecraft's World Save Format

### Directory Structure

```
<worldname>/
  level.dat                  # World metadata (seed, spawn, game rules, weather)
  level.dat_old              # Backup of previous level.dat
  session.lock               # Prevents concurrent access
  region/                    # Terrain chunk data (r.x.z.mca files)
  entities/                  # Entity data (r.x.z.mca files, since 1.17)
  poi/                       # Points of interest (villager workstations, bells)
  data/                      # Scoreboards, raids, map items
  playerdata/                # Per-player data: <uuid>.dat
  DIM-1/                     # Nether (own region/, entities/, poi/)
  DIM1/                      # End (own region/, entities/, poi/)
```

Key insight: each dimension has its own `region/` and `entities/` subdirectories. The overworld's are at the root; other dimensions use named subdirectories.

### Region File Format (.mca)

Region files group 32×32 chunks (1,024 chunks) into a single file. Named `r.<rx>.<rz>.mca` where `rx = floor(chunkX / 32)`.

**Layout (4 KiB sector-aligned):**

| Offset | Size | Content |
|---|---|---|
| 0–4095 | 4 KiB | Location table (1024 × 4-byte entries) |
| 4096–8191 | 4 KiB | Timestamp table (1024 × 4-byte entries) |
| 8192+ | variable | Chunk data sectors |

**Location entry** (4 bytes, big-endian): top 3 bytes = sector offset, bottom byte = sector count. Zero = chunk not present.

**Chunk payload**: 4-byte length + 1-byte compression type (2=Zlib, 4=LZ4) + compressed NBT data.

This gives O(1) random access to any chunk: read 4 bytes from header, seek to sector, decompress.

### Chunk Data (NBT)

Each chunk contains:
- `sections` — list of 16×16×16 sub-sections, each with palette-based block storage and biome data
- `block_entities` — chests, signs, etc. (stored with the chunk, not in entities/)
- `Heightmaps` — pre-computed height data
- `Status` — generation stage (`empty` → `full`)
- `block_ticks`, `fluid_ticks` — scheduled updates
- `structures` — structure generation references

**Palette-based storage**: each section maintains a palette of unique block states and stores indices into it. Sections with one block type need zero data bits (just the single palette entry). Index bit width = max(4, ceil(log2(palette_size))).

### Entity Storage (Since 1.17)

Entities are stored in separate `.mca` region files under `entities/`, using the same sector-based format. Each chunk's entity data contains:
- `Entities` list — each with `id`, `Pos`, `Motion`, `Rotation`, `UUID`, plus type-specific tags

**Why separated**: decouples entity I/O from terrain I/O. Terrain can load/save independently of entities.

### level.dat (World Metadata)

Gzip-compressed NBT containing:
- `SpawnX/Y/Z` — world spawn coordinates
- `RandomSeed` — generation seed
- `GameType`, `Difficulty`, `hardcore`
- `Time` (total ticks), `DayTime` (time of day)
- `raining/thundering` — weather state
- `GameRules` — all game rules
- `WorldGenSettings` — generator configuration
- `DataVersion` — for format migration

### Key Design Principles

1. **Everything serialized with one format** (NBT) then compressed
2. **Region files are simple sector allocators** — fixed header enables O(1) chunk lookup
3. **Entity/terrain separation** enables independent I/O scheduling
4. **Palette compression** for block data — uniform chunks are nearly free
5. **Data versioning** for forward migration
6. **Atomic writes** not used for regions (uses session.lock instead); level.dat uses backup copy

Sources:
- [Region file format — Minecraft Wiki](https://minecraft.wiki/w/Region_file_format)
- [Chunk format — Minecraft Wiki](https://minecraft.wiki/w/Chunk_format)
- [NBT format — Minecraft Wiki](https://minecraft.wiki/w/NBT_format)
- [Java Edition level format — Minecraft Wiki](https://minecraft.wiki/w/Java_Edition_level_format)
- [Entity format — Minecraft Wiki](https://minecraft.wiki/w/Entity_format)

## Current Project State

### What Exists Today

**Persistence**: single bincode file at `world_save/voxel_world.bin` containing only voxel modifications ([server/src/map.rs:207-213](crates/server/src/map.rs#L207-L213)):

```rust
#[derive(Serialize, Deserialize)]
struct VoxelWorldSave {
    version: u32,
    generation_seed: u64,
    generation_version: u32,
    modifications: Vec<(IVec3, VoxelType)>,
}
```

- Debounced save (1s after last edit, 5s max dirty) + save on shutdown
- Atomic write via temp file + rename
- Corrupt file recovery (backup to `.corrupt`)
- Only the Overworld is saved; Homebases are ephemeral

**Voxel data structures** ([voxel_map_engine/src/types.rs](crates/voxel_map_engine/src/types.rs)):
- `WorldVoxel` enum: `Air`, `Unset`, `Solid(u8)` — 256 material types
- `ChunkData`: `Vec<WorldVoxel>` of 18³ (padded), plus `FillType` and hash
- `VoxelType`: network-serializable mirror (no `Unset`)
- Chunks are 16³ voxels

**Map instances** ([voxel_map_engine/src/instance.rs](crates/voxel_map_engine/src/instance.rs)):
- `VoxelMapInstance` component owns `OctreeI32<Option<ChunkData>>`, `modified_voxels: HashMap<IVec3, WorldVoxel>`, `write_buffer`, `loaded_chunks`
- Three map types: `Overworld`, `Homebase { owner: u64 }`, `Arena { id: u64 }`
- `MapInstanceId` enum in protocol for network identity

**Map metadata** ([protocol/src/map.rs](crates/protocol/src/map.rs)):
- `MapWorld` resource: seed + generation_version
- `MapRegistry`: maps `MapInstanceId` → Entity
- No spawn point storage (hardcoded `Vec3::new(0.0, 5.0, 0.0)` in map transition)

**Spawn points** ([server/src/gameplay.rs](crates/server/src/gameplay.rs)):
- `spawn_respawn_points()` creates respawn point entities at fixed positions
- Not persisted, recreated each server start

**No entity persistence**: entities exist only in memory during server runtime.

### What's Missing for Map-as-Directory

1. **Per-chunk terrain saving** — currently saves only modification diffs, not full chunk data
2. **Entity persistence** — no entity save/load at all
3. **Map metadata file** — seed, spawn points, generation config not saved per-map
4. **Per-map directories** — single flat file, not one directory per map instance
5. **Multi-map persistence** — only Overworld saved; Homebases/Arenas ephemeral

## Proposed Directory Structure

Mapping Minecraft's approach to this project's architecture:

```
worlds/
  overworld/
    map.meta.bin             # MapWorld seed, generation_version, spawn points, game rules
    terrain/
      r.0.0.bin              # Region file: 8×8 chunks (or per-chunk files)
      r.1.0.bin
      ...
    entities/
      r.0.0.bin              # Entity data grouped by chunk region

  homebase-<owner_id>/
    map.meta.bin
    terrain/
      ...
    entities/
      ...

  arena-<id>/
    map.meta.bin
    terrain/
      ...
    entities/
      ...
```

## Terrain Saving Approaches

### Approach A: Save Full Chunk Data (Minecraft-style)

Save the complete voxel array for every generated chunk.

**Per-chunk file** (`terrain/chunk_<x>_<y>_<z>.bin`):
- Serialize the full `ChunkData.voxels` (18³ = 5,832 `WorldVoxel` values)
- Apply palette compression: record unique voxels, store indices
- Compress with zstd

**Region file** (Minecraft-style grouping):
- Group N×N×N chunks into one file with sector-based index header
- Reduces filesystem overhead from thousands of tiny files
- For a 3D voxel world, 8×8×8 = 512 chunks per region file is reasonable

**Pros**: complete world state on disk, no dependency on deterministic generation, supports pre-authored terrain
**Cons**: larger files, more I/O, must re-save chunks when modified

### Approach B: Save Modifications Only (Current Approach, Extended Per-Chunk)

Keep the current modifications-only model but organize per-chunk instead of one global list.

**Per-chunk modification file** (`terrain/mods_<x>_<y>_<z>.bin`):
- Only chunks with modifications get a file
- Contains `Vec<(UVec3, WorldVoxel)>` — local positions within chunk
- Terrain regenerated procedurally, modifications applied on top

**Pros**: minimal storage for sparse edits, fast saves (only dirty chunks)
**Cons**: still depends on deterministic generation, can't save pre-authored terrain

### Approach C: Hybrid — Full Chunks for Pre-authored, Mods for Procedural

- Procedural chunks: save only modifications (approach B)
- Pre-authored chunks: save full data (approach A)
- A flag per chunk indicates which type it is

**Pros**: best of both, supports the TODO items for pre-authored chunks/maps
**Cons**: two code paths

**Decision**: Approach A — save full chunk data in region files, like Minecraft. This eliminates dependency on deterministic generation, supports pre-authored terrain, and provides a complete world snapshot on disk.

### Serialization Format Recommendations

Based on benchmarks and ecosystem:

| Format | Speed | Size | Notes |
|---|---|---|---|
| **bitcode** | Fastest | Smallest | Pure Rust, newer |
| **bincode** | Fast | Small | Already used in project |
| **postcard** | Fast | Small | Good for embedded/no_std |
| **rkyv** | Zero-copy read | Medium | Best for hot-path reads |

**Compression**: zstd level 3 is the sweet spot (90%+ reduction on voxel data, fast).

Current project uses `bincode` — continuing with it avoids new dependencies. Adding `zstd` for compression is the main win.

**Decision**: bincode + zstd.

## Entity Persistence

### What Needs Saving

Entities on a map that should persist across server restarts:
- Respawn points (position, configuration)
- Placed objects / doodads (from TODO: "prefab world objects")
- Any future NPC or interactable entities

Entities that should NOT persist:
- Player characters (transient, spawned on connect)
- Projectiles, effects (transient combat state)
- Chunk entities (regenerated from terrain data)

### Entity Save Format

Each persistable entity needs:
- Position (`Vec3` or `IVec3`)
- Which chunk it belongs to (for spatial grouping)
- Component data (serde-serialized)
- Entity type identifier (for reconstruction)

Two grouping strategies:
1. **Per-chunk** (like Minecraft 1.17+): entity data grouped by chunk coordinate, stored in entity region files
2. **Per-map flat list**: all entities for the map in one file — simpler, fine for small entity counts

For early implementation, a flat list per map is simpler:

```rust
#[derive(Serialize, Deserialize)]
struct MapEntitySave {
    version: u32,
    entities: Vec<SavedEntity>,
}

#[derive(Serialize, Deserialize)]
struct SavedEntity {
    kind: String,              // e.g. "respawn_point", "doodad"
    position: Vec3,
    data: HashMap<String, Vec<u8>>,  // component name → serialized bytes
}
```

### Per-Chunk Entity Storage

Per-chunk entity storage groups entities by the chunk coordinate they occupy, stored in entity region files (separate from terrain region files). This mirrors Minecraft 1.17+'s approach.

**Region file layout** (`entities/r.<rx>.<ry>.<rz>.bin`): same sector-based format as terrain region files, but each chunk slot contains an entity list instead of voxel data.

```rust
/// Stored per chunk coordinate inside an entity region file
#[derive(Serialize, Deserialize)]
struct ChunkEntities {
    entities: Vec<SavedEntity>,
}
```

**Chunk assignment**: an entity at world position `pos` belongs to chunk `IVec3(floor(pos.x / 16), floor(pos.y / 16), floor(pos.z / 16))`.

**Save flow**:
1. Query all entities with `MapSaveTarget` marker component
2. Group by chunk coordinate (derived from `Transform.translation`)
3. For each dirty chunk region, update that region file's chunk slot

**Load flow**:
1. When a chunk is loaded (terrain generated/read), also read its entity slot from the entity region file
2. Spawn the entities with their saved components

**Advantages over flat list**:
- Entities load/unload with their chunk — no need to load all entities for a map at once
- Dirty tracking is per-chunk — only re-serialize chunks whose entities changed
- Scales to large maps with many entities without loading everything into memory
- Entities near the player load first (same spatial locality as terrain)

**When to use flat list vs per-chunk**: flat list is fine when entity counts are small (hundreds). Per-chunk becomes necessary when maps have thousands of placed objects/doodads spread across many chunks.

## Map Metadata

### What Goes in map.meta.bin

```rust
#[derive(Serialize, Deserialize)]
struct MapMeta {
    version: u32,
    map_type: MapTypeTag,       // Overworld / Homebase / Arena
    seed: u64,
    generation_version: u32,
    spawn_points: Vec<Vec3>,
    bounds: Option<IVec3>,
    // Future: weather state, time of day, game rules
}
```

This replaces the current hardcoded spawn position (`Vec3::new(0.0, 5.0, 0.0)`) and stores per-map configuration that currently only exists in code.

## Rust Ecosystem Tools

### Relevant Crates

- **[chunky-bevy](https://crates.io/crates/chunky-bevy)** — Bevy chunk management with `PerChunk` and `SuperChunk` (region-style) persistence. Young but directly relevant.
- **[bevy_save](https://github.com/hankjordan/bevy_save)** — Full-featured Bevy save/load with reflection-based snapshots, migrations, partial saves. Good for non-chunk game state.
- **[moonshine_save](https://github.com/Zeenobit/moonshine_save)** — Marker-based selective entity saving. Tag entities with `Save` component.
- **[memmap2](https://crates.io/crates/memmap2)** — Memory-mapped file I/O for region files.
- **[zstd](https://github.com/gyscos/zstd-rs)** — Best compression ratio for chunk data.
- **[lz4_flex](https://crates.io/crates/lz4_flex)** — Faster decompression, worse ratio. Pure Rust.
- **[bitcode](https://crates.io/crates/bitcode)** — Fastest + smallest serialization format.

### Bevy Scene System — Not Suitable

Bevy scenes have O(N²) component insertion, reflection overhead, and no compression. Not suitable for chunk streaming. Fine for editor/prefab workflows but not for world persistence.

### SHARD Format (Design Reference)

Alternative to Minecraft's Anvil from [scrayos.net](https://scrayos.net/justchunks-shard-format/):
- Single atomic file per region with bitmask prefix for empty sections
- Whole-file zstd compression (better ratio than per-chunk)
- Result: 500MB world → <5MB compressed

## Code References

- [crates/server/src/map.rs:207-213](crates/server/src/map.rs#L207-L213) — Current `VoxelWorldSave` struct
- [crates/server/src/map.rs:227-262](crates/server/src/map.rs#L227-L262) — `save_voxel_world_to_disk_at()` (atomic write)
- [crates/server/src/map.rs:264-344](crates/server/src/map.rs#L264-L344) — `load_voxel_world_from_disk_at()` (with corruption recovery)
- [crates/server/src/map.rs:85-113](crates/server/src/map.rs#L85-L113) — Debounced auto-save system
- [crates/voxel_map_engine/src/instance.rs:27-33](crates/voxel_map_engine/src/instance.rs#L27-L33) — `VoxelMapInstance` fields (octree, modified_voxels, loaded_chunks)
- [crates/voxel_map_engine/src/types.rs:12-17](crates/voxel_map_engine/src/types.rs#L12-L17) — `WorldVoxel` enum
- [crates/voxel_map_engine/src/types.rs:34-39](crates/voxel_map_engine/src/types.rs#L34-L39) — `ChunkData` struct
- [crates/protocol/src/map.rs:12-16](crates/protocol/src/map.rs#L12-L16) — `MapWorld` resource (seed, generation_version)
- [crates/protocol/src/map.rs:31-34](crates/protocol/src/map.rs#L31-L34) — `MapInstanceId` enum
- [crates/server/src/gameplay.rs](crates/server/src/gameplay.rs) — `spawn_respawn_points()`, hardcoded spawn positions

## Historical Context (from doc/)

- `doc/research/2026-01-17-voxel-world-save-load.md` — Original persistence research; designed the current modifications-only approach
- `doc/plans/2026-01-18-voxel-world-persistence.md` — Implementation plan for current save system
- `doc/research/2026-02-27-bonsairobo-stack-multi-instance-voxel-replacement.md` — Research on replacing bevy_voxel_world; informed current voxel_map_engine design
- `doc/plans/2026-02-28-voxel-map-engine.md` — Plan for building voxel_map_engine crate
- `TODO.md` — Lists "map as dir saving chunks, entities as files" as a pending item

## Follow-up Research: Minecraft In-Memory Chunk Storage

### Palette-Based Block Storage

Minecraft does not use flat arrays in memory. Each 16×16×16 section uses a `PalettedContainer<BlockState>` with three strategies selected at runtime:

| Strategy | Bits/Entry | When Used | Memory |
|---|---|---|---|
| Single-valued | 0 | Uniform section (all one block) | ~0 bytes (just the palette entry) |
| Indirect | 4-8 | 2-256 distinct block types | Palette array + packed long[] |
| Direct | 15 | >256 distinct types | No local palette, global registry IDs |

The block indices (4,096 per section) are packed into a `long[]` array. At 4 bits/entry, 16 entries fit per `long`, requiring 256 longs (2,048 bytes) for 4,096 blocks. An index never spans two `long` values — unused high bits are padding.

The palette is a small array mapping local indices → global `BlockState` IDs. For a section with 5 distinct blocks, the palette has 5 entries and each index is 4 bits wide.

**Key insight**: uniform sections (all air, all stone) cost essentially zero — just the single palette entry. In a typical overworld chunk, only 5-6 of 24 possible sections contain mixed blocks. The upper ~18 sections are all-air and consume near-zero memory.

Sources:
- [Minecraft Wiki — Chunk Protocol Format](https://minecraft.wiki/w/Java_Edition_protocol/Chunk_format)
- [wiki.vg — Chunk Format](https://wiki.vg/Chunk_Format)

### Chunk Memory Usage Numbers

| Component | Size | Notes |
|---|---|---|
| Block data (palette + packed) | 2-8 KiB per section | Depends on palette size |
| Sky light | 2,048 bytes per section | Nibble array (4 bits/block) |
| Block light | 2,048 bytes per section | Nibble array |
| Heightmaps | ~900 bytes per chunk column | 3 types × ~300 bytes each |
| Biomes | Small | 4×4×4 resolution, same palette container |
| **Total per section** | ~6-12 KiB | |
| **Empty overworld chunk** | ~11 KiB | Most sections are single-valued |
| **Typical overworld chunk** | ~50 KiB | 5-6 active sections |
| **Measured average** | ~170 KiB | Including entities and overhead |

At simulation distance 10: 441 chunks per player ≈ 22 MB of chunk data per player.

Sources:
- [Minecraft Forum — Memory Per Chunk](https://www.minecraftforum.net/forums/minecraft-java-edition/discussion/3120640-how-much-memory-does-it-take-to-hold-a-chunk)
- [SpigotMC — RAM Usage Per Chunk](https://www.spigotmc.org/threads/ram-usage-per-chunk.302079/)

### Chunk Ticket System (Load Level Management)

Minecraft does not use LRU or simple distance-based eviction. Instead, a **ticket system** controls which chunks stay loaded:

| Ticket Type | Level | Notes |
|---|---|---|
| Player | 31 | Moves with player, refreshed continuously |
| Forced (`/forceload`) | 31 | Persists across restarts |
| Portal | 30 | 300 ticks (15s) timeout |
| Dragon | 24 | Ender dragon fight area |

From the ticket source, the load level propagates outward (+1 per chunk hop, max 44). This creates concentric rings:

| Level | Behavior |
|---|---|
| ≤31 | Entity ticking (full AI, spawning) |
| 32 | Block ticking (entities exist but frozen) |
| 33 | Border (entities accessible, frozen, count toward mob cap) |
| 34+ | Not loaded |

When a player disconnects, chunks stay loaded ~10 seconds before unloading.

Sources:
- [Minecraft Wiki — Chunk](https://minecraft.wiki/w/Chunk)
- [Drovolon's Chunk Loading Mechanics](https://gist.github.com/Drovolon/24bfaae00d57e7a8ca64b792e14fa7c6)

### Entity Lifecycle

- Entities load/unload with chunks (since 1.17, stored in separate region files)
- Entity processing depends on chunk load level (full AI at ≤31, frozen at 32-33)
- Hostile mobs despawn instantly if >128 blocks from all players regardless of chunk state
- When a chunk unloads, entities are serialized back to the entity region file

### Light Data

Stored per section as two nibble arrays (4 bits/block):
- `SkyLight`: 2,048 bytes per section
- `BlockLight`: 2,048 bytes per section

Light is **not computed on the fly** — incrementally updated by the light engine when blocks change. Sections with no light can omit their nibble arrays.

### Comparison: Memory Approaches

| Approach | Memory/Voxel | Random Access | Best For |
|---|---|---|---|
| **Minecraft Palette** | ~0.5-2 bytes | O(1) | Mixed terrain, frequent read/write |
| **Flat Array** (current project) | 2 bytes | O(1) | Simplest, fastest |
| **Sparse Voxel Octree** | ~5 bytes/stored voxel | O(log n) | Sparse worlds |
| **DAG** | Best compression | O(log n) | Repetitive structures |
| **RLE** | Excellent for uniform runs | O(n) decompress | Layered terrain |

**Palette's advantage**: O(1) random access (critical for gameplay — block placement, physics, lighting) while being significantly better than flat arrays. Uniform sections are nearly free.

Sources:
- [Voxel Compression Documentation](https://eisenwave.github.io/voxel-compression-docs/introduction.html)
- [Ricardo Antunes — Grids vs Octrees](https://riscadoa.com/game-dev/voxel-engine-1/)

### Current Project vs Minecraft

| Aspect | Minecraft | Current Project |
|---|---|---|
| Section storage | PalettedContainer (0-15 bits/entry) | Flat `Vec<WorldVoxel>` (2 bytes/voxel) |
| Empty sections | Near-zero cost (single-valued palette) | Full 11.4 KiB allocated |
| Chunk memory | ~50 KiB typical | ~11.4 KiB (no light, no biomes) |
| Data retention | Always retained while loaded | **Discarded after meshing**  |
| Eviction | Ticket system with load levels | Distance-based `HashSet::retain` |
| Modified voxels | Baked into section data | Separate `HashMap<IVec3, WorldVoxel>` |

**Current chunk memory**: `WorldVoxel` is 2 bytes (discriminant + u8 payload). Each chunk stores 18³ = 5,832 voxels = **11,664 bytes** (~11.4 KiB). With `FillType` and `hash`, total is ~11.7 KiB per `ChunkData`.

**No compression exists**: `FillType` is defined but never read at runtime. Every chunk allocates the full Vec regardless of content.

### Applicability to This Project

The project's 16³ chunks with `WorldVoxel` (only 3 variants: Air/Unset/Solid(u8)) are much simpler than Minecraft's thousands of block states. Realistic optimizations for this project:

1. **Single-valued chunks**: If `FillType::Empty` or `FillType::Uniform`, don't allocate the Vec. This is trivially implementable since `FillType` already exists — just needs runtime use. Uniform air chunks (common above terrain) drop from 11.4 KiB to ~16 bytes.

2. **Palette compression**: With only ~257 possible voxel values (Air + 256 Solid variants), most chunks need ≤8 bits/entry. A section with 2 distinct voxels needs 1 bit/entry = 512 bytes vs 11,664 bytes — a 23× reduction. Implementation complexity is moderate.

3. **Not needed now**: At current spawning distances (2-10), flat arrays are fine. But if view distances grow or the server manages many simultaneous map instances, palette compression becomes worth it.

## Decisions

1. **Region files** for both terrain and entities (not per-chunk files). 3D regions group 8×8×8 = 512 chunks per file.
2. **Save full chunk data** — not just modifications. Eliminates proc-gen dependency, supports pre-authored terrain.
3. **`MapSaveTarget` marker component** on entities that should persist. Transient entities (projectiles, effects, players) don't get the marker.
4. **Keep current debounced save timing** (1s debounce, 5s max dirty). Add per-chunk dirty tracking so only modified chunks/regions are re-serialized.
5. **All map types persist** — Overworld, Homebases, and Arenas all get their own save directories.