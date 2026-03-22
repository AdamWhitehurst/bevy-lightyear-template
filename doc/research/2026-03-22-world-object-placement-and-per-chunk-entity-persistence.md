---
date: "2026-03-22T09:33:05-07:00"
researcher: Claude
git_commit: 63fffb9e91e9117563b3d7e60efc21365eebe5d2
branch: master
repository: bevy-lightyear-template
topic: "World Object Feature Placement and Per-Chunk Entity Persistence"
tags: [research, codebase, world-objects, entity-persistence, chunk-lifecycle, procedural-generation, minecraft, voxel-map-engine]
status: complete
last_updated: "2026-03-22"
last_updated_by: Claude
---

# Research: World Object Feature Placement and Per-Chunk Entity Persistence

**Date**: 2026-03-22T09:33:05-07:00
**Researcher**: Claude
**Git Commit**: 63fffb9e91e9117563b3d7e60efc21365eebe5d2
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

Following the chunk ticket system refactor, how should world object feature placement on maps and per-chunk entity persistence be implemented in voxel_map_engine? How does Minecraft approach these problems, and how can we adapt them?

## Summary

The codebase has all foundational pieces in place: a ticket-based chunk loading system with load levels (`ticket.rs`), terrain generation from data-driven `.terrain.ron` definitions with biome rules, a world object definition system (`WorldObjectDef` with reflect-based component bags), and chunk persistence (`persistence.rs`). What's missing is: (1) a feature placement pipeline that spawns world objects during/after chunk generation, and (2) per-chunk entity storage that loads/unloads entities with their chunks.

Minecraft's approach provides a proven model: features are placed deterministically per-chunk using seeded RNG during a "decoration" phase after terrain generation, entities are stored in per-chunk files separate from terrain, and the ticket system's load levels control which entities tick. The key adaptation needed is that our system is simpler — 2.5D surface-based placement, no multi-chunk structures, and a "generate once, save forever" model where procedural objects are regenerated from seed on first load but persisted thereafter.

---

## Detailed Findings

### 1. How Minecraft Does Feature Placement

#### Generation Pipeline

Minecraft generates chunks through 12 sequential statuses. The relevant ones for feature placement are:

| # | Status | Purpose |
|---|--------|---------|
| 5 | `noise` | Base terrain shape via density functions |
| 6 | `surface` | Biome-specific surface blocks (grass, sand) |
| 7 | `carvers` | Cave/canyon carving |
| 8 | **`features`** | **All feature and structure placement** |
| 11 | `spawn` | Initial mob spawning |
| 12 | `full` | Chunk finalized |

Within the `features` status, there are **11 decoration steps** executed in order:
1. `raw_generation` — small end islands
2. `lakes` — lava lakes
3. `local_modifications` — geodes, icebergs
4. `underground_structures` — mineshafts, fossils
5. `surface_structures` — villages, wells
6. `strongholds`
7. `underground_ores` — ore blobs, disks
8. `underground_decoration` — infested blocks
9. `fluid_springs` — water/lava springs
10. **`vegetal_decoration`** — trees, bamboo, cacti, kelp
11. `top_layer_modification` — freeze layer

For each step, structure pieces are placed first, then features. Features are defined as **Configured Features** (what to generate) + **Placed Features** (where/how often), composed of placement modifiers applied in sequence.

#### Deterministic Seeding

Minecraft uses a layered seed hierarchy:
1. **Population seed**: `hash(worldSeed, chunkX * 16, chunkZ * 16)`
2. **Decorator seed**: `hash(populationSeed, index + 10000 * step)` where `step` is the decoration step ordinal and `index` is the feature's position within that step
3. **Region seed**: `hash(worldSeed, regionX, regionZ, salt)` for structure placement

Every feature gets a unique, deterministic random sequence.

#### Cross-Chunk Boundary Handling

Features can write blocks outside their chunk but are limited to a 3x3 chunk area. Minecraft historically offsets feature generation by +8 blocks in X/Z to create a consistent dependency direction. Structure pieces that span multiple chunks are pre-computed during `structures_starts` and split at chunk boundaries — each chunk places only its portion.

**Key insight**: Minecraft requires neighboring chunks to reach at least the `carvers` status (terrain finalized) before running features, ensuring terrain shape is stable when features query it.

#### Data-Driven Feature Definitions (1.18+)

Configured Features define the feature type and config:
```json
{
  "type": "minecraft:tree",
  "config": { "trunk_provider": { ... }, "foliage_provider": { ... } }
}
```

Placed Features wrap a configured feature with an ordered list of placement modifiers:
```json
{
  "feature": "minecraft:oak",
  "placement": [
    { "type": "minecraft:count", "count": 10 },
    { "type": "minecraft:in_square" },
    { "type": "minecraft:heightmap", "heightmap": "MOTION_BLOCKING" },
    { "type": "minecraft:biome" }
  ]
}
```

Placement modifiers include: `count`, `in_square` (random XZ spread within chunk), `heightmap` (snap to surface), `biome` (filter by biome), `rarity_filter` (1/N chance), `height_range`, `block_predicate_filter`, `noise_based_count`.

Biome definitions reference placed features in an 11-element array (one per decoration step).

### 2. How Minecraft Does Per-Chunk Entity Persistence

#### Entity Storage (Post-1.17)

Since 1.17, entities are stored **separately from terrain** in dedicated `entities/` region files:

```
world/
  region/r.0.0.mca        # terrain
  entities/r.0.0.mca      # entities for same region coords
  poi/r.0.0.mca           # points of interest (villager jobs, beds)
```

Entities are position-indexed: `chunk_x = floor(x / 16)`, `chunk_z = floor(z / 16)`. No explicit chunk reference is stored on entities — position is truth.

#### Generate Once, Save Forever

Minecraft uses a strict model: chunk generation runs **exactly once** when a chunk first reaches `full` status. After that:
- The saved state on disk is canonical
- The procedural generator is **never re-run**
- Destroyed trees/structures stay destroyed permanently
- Modified terrain stays modified permanently

The chunk format includes a `Status` field. Once it reaches `full`, generation never re-runs.

#### Entity Loading/Unloading Lifecycle

**Load**: When a chunk reaches sufficient load level → read entity data from `entities/` region file → deserialize into live entities.

**Unload**: When a chunk loses all tickets → serialize all entities in that chunk to NBT → write to `entities/` region file → despawn entities.

**Cross-chunk movement**: Entities are tracked by current position. When an entity moves to a different chunk, it is re-indexed. On save, it is written to whichever chunk it currently occupies.

#### Ticket Levels and Entity Behavior

| Load Level | State | Entity Behavior |
|---|---|---|
| ≤31 | **Entity Ticking** | Full simulation: AI, spawning, physics |
| 32 | **Block Ticking** | Redstone/blocks tick. Entities **not processed** but in memory |
| 33 | **Border** | No ticking. Entities accessible for reads only |
| 34+ | **Inaccessible** | Not loaded at all |

Entities that move from entity-ticking to block-ticking chunks become **suspended** until load level is promoted.

### 3. Current State of the Codebase

#### What Exists

**Chunk Ticket System** (`ticket.rs`): Fully implemented with `ChunkTicket`, `TicketType` (Player/Npc/MapTransition), `LoadState` (EntityTicking/BlockTicking/Border/Inaccessible), `TicketLevelPropagator`. Load levels propagate via Chebyshev distance from ticket sources.

**Terrain Generation** (`terrain.rs`, `generation.rs`): Data-driven via `.terrain.ron` with `HeightMap`, `MoistureMap`, `BiomeRules` components. `build_generator()` reads components from map entity, produces `VoxelGenerator` closure. `spawn_chunk_gen_task()` runs generation async, tries disk load first. Currently generates terrain only — no feature/object placement.

**World Object System** (`protocol/src/world_object/`): `WorldObjectDef` is a `Vec<Box<dyn PartialReflect>>` loaded from `.object.ron` files via `reflect_loader`. `apply_object_components()` inserts reflected components onto entities. Only one def exists: `tree_circle.object.ron`. Server spawns, client replicates via Lightyear.

**Chunk Persistence** (`persistence.rs`): Per-chunk terrain files at `worlds/{map}/terrain/chunk_X_Y_Z.bin`. Bincode + zstd compression, atomic write via tmp+rename. Versioned envelope. Dirty chunks saved on eviction.

**Entity Persistence** (`server/src/persistence.rs`): Flat `entities.bin` per map. Only `SavedEntityKind::RespawnPoint` supported. `MapSaveTarget` marker component. No spatial partitioning.

#### What's Missing

1. **No feature placement pipeline**: `VoxelGenerator` returns only voxels. No mechanism to return "also spawn these entities." No `PlacementRules` handling (the component type exists in terrain.rs but is **not** currently defined or used — only `HeightMap`, `MoistureMap`, `BiomeRules` exist).
2. **No per-chunk entity storage**: All entities saved in one flat file per map. No chunk-entity association. No load/unload with chunks.
3. **No `PlacementRules` or `PlacementRule` types**: Referenced in prior research but not yet implemented in terrain.rs.
4. **No chunk generation status tracking**: Chunks are binary loaded/unloaded. No "generated but not decorated" intermediate state.
5. **LoadState enum exists** but is only used for debug display — not yet driving entity ticking behavior.

### 4. Relevant Existing Patterns

#### PlacementRules Design (From Prior Research)

The procedural generation research (`doc/research/2026-03-18`) designed but did not implement:

```rust
#[derive(Component, Clone, Serialize, Deserialize, Reflect)]
#[reflect(Component, Serialize, Deserialize)]
pub struct PlacementRules(pub Vec<PlacementRule>);

#[derive(Clone, Serialize, Deserialize, Reflect)]
pub struct PlacementRule {
    pub object_id: String,
    pub allowed_biomes: Vec<String>,
    pub density: f64,
    pub min_spacing: f64,
    pub slope_max: Option<f64>,
}
```

Placement technique: Poisson disk sampling with density modulation from noise. Per-chunk deterministic generation via `hash(map_seed, chunk_pos)`.

#### Per-Chunk Entity Persistence Design (From Prior Research)

The same research designed:

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChunkEntity {
    pub procedural_id: Option<u64>,
    pub object_id: WorldObjectId,
    pub position: Vec3,
    pub destroyed: bool,
    pub component_overrides: HashMap<String, String>,
}
```

Stored at `entities/chunk_X_Y_Z.entities.bin`. Chunk load sequence: generate procedural objects → load entity file → merge (destroyed = skip, overrides = apply, no match = spawn fresh). Chunk unload: serialize current state → write entity file → despawn.

### 5. Minecraft Patterns Applied to Our System

#### What We Should Adopt

**Generate-once, save-forever model**: Like Minecraft, run procedural placement exactly once per chunk. The first time a chunk is generated, produce both voxels and entity spawn positions. Save the entity list to disk. On subsequent loads, restore from disk — never re-run placement.

This is simpler than the "mutation overlay" approach in the prior research (where we'd always re-run the generator and diff against saved state). It means:
- No `procedural_id` or `destroyed` fields needed
- No merge logic between generated and saved entities
- Chunk entity file is the single source of truth after first generation
- Player modifications (destroyed trees, moved objects) are naturally persisted

**Per-chunk entity files**: Separate from terrain data, stored in `entities/` subdirectory. Same bincode + zstd + atomic write pattern.

**Deterministic seeded placement**: Per-chunk seed derived from `hash(map_seed, chunk_pos)`. Poisson disk sampling for spacing. Biome-filtered placement rules.

**Ticket-level-driven entity ticking**: Use the existing `LoadState` to control entity behavior:
- `EntityTicking` (level 0): Full simulation
- `BlockTicking` (level 1): Entities loaded but frozen
- `Border` (level 2): Entities not loaded
- `Inaccessible` (3+): Not relevant

#### What We Skip / Simplify

**No multi-chunk features/structures**: Our world objects fit within a single chunk. No need for structure starts/references, bounding box systems, or cross-chunk piece splitting. Trees and objects are single-entity spawns.

**No decoration steps**: Minecraft's 11-step system handles complex ordering of caves → ores → structures → vegetation. We have one placement pass: "spawn objects on the surface after terrain is generated."

**No 8-block offset**: Minecraft's offset trick manages cross-chunk feature spillover. Since our objects don't modify terrain (they're entities placed on top), no offset needed.

**No intermediate chunk statuses**: Minecraft needs `structures_starts` → `structures_references` → `features` because structures span chunks. Our generation is: terrain → place entities → done.

**No POI system**: Minecraft's POI tracks villager job sites and pathfinding. Not relevant to our entity types.

### 6. Integration Points

#### ChunkGenResult Extension

Currently:
```rust
pub struct ChunkGenResult {
    pub position: IVec3,
    pub mesh: Option<Mesh>,
    pub chunk_data: ChunkData,
    pub from_disk: bool,
}
```

Needs a field for spawned entities: world object placements determined during generation. These are only generated when the chunk has no saved entity file (first generation).

#### Generation Flow

1. `spawn_chunk_gen_task` runs async
2. Try disk load for terrain → if found, load terrain from disk
3. Try disk load for entities → if found, entities come from disk (no generation)
4. If no entity file exists AND `PlacementRules` present: run placement algorithm using terrain heightmap data + biome data from the just-generated chunk
5. Return both terrain and entity spawn list in `ChunkGenResult`

#### Entity Spawning in `poll_chunk_tasks`

After `handle_completed_chunk` inserts terrain data:
1. If entity spawn list is non-empty, iterate and call `spawn_world_object` for each
2. Tag each spawned entity with `ChunkEntityRef { chunk_pos, map_entity }`
3. Mark entities for network replication via Lightyear rooms

#### Entity Unloading in Chunk Eviction

In `remove_column_chunks`:
1. Query entities with `ChunkEntityRef` matching the unloading chunk
2. Serialize to `Vec<ChunkEntity>`
3. Write to `entities/chunk_X_Y_Z.entities.bin`
4. Despawn the ECS entities

#### Existing `entities.bin` Coexistence

The flat per-map `entities.bin` continues to handle map-global entities (respawn points). The two systems are orthogonal:
- `entities.bin`: loaded once at map spawn, saved on map save. `MapSaveTarget` component.
- `entities/chunk_*.entities.bin`: loaded/saved with chunk lifecycle. `ChunkEntityRef` component.

### 7. Placement Algorithm Detail

#### Poisson Disk Sampling Per-Chunk

For a chunk at position `chunk_pos` with `PlacementRules`:

```
for each rule in placement_rules:
    rng = seeded_rng(hash(map_seed, chunk_pos, rule.object_id))
    candidates = poisson_disk_sample(chunk_bounds_xz, rule.min_spacing, rng)
    for each candidate (x, z):
        height = sample_terrain_height(x, z)  // from just-generated voxels
        biome = sample_biome(x, z)            // from moisture/height
        if biome not in rule.allowed_biomes: skip
        if slope_at(x, z) > rule.slope_max: skip
        if rng.random() > rule.density: skip
        emit WorldObjectSpawn { object_id, position: (x, height, z) }
```

**Neighboring chunk consideration**: Poisson disk sampling needs to check objects in adjacent chunks for minimum spacing. Two approaches:
1. **Regenerate neighbors**: Since placement is deterministic, we can re-run the placement algorithm for the 8 neighboring chunks to check spacing. Cheap if we only sample positions (no entity spawning).
2. **Accept boundary artifacts**: For simplicity, ignore cross-chunk spacing. Objects near chunk borders may cluster slightly. Acceptable for trees/scenery.

Approach 2 is simpler and matches the game's visual style (2.5D brawler, not a survival builder where exact tree spacing matters).

#### Height Sampling from Voxels

After terrain generation, the chunk's voxel data is available. Surface height at (x, z) within a chunk = highest y where `voxels[x, y, z] != Air`. This avoids re-sampling noise and handles any terrain modifications.

### 8. Networking Considerations

World objects spawned by the server are replicated to clients via Lightyear rooms. The existing replication flow (`spawn_world_object` → `Replicate::to_clients` → client `on_world_object_replicated`) handles this. Per-chunk entity persistence is server-only — clients don't persist entities.

Chunk entity load/unload maps naturally to Lightyear's room system: when a chunk loads, its entities are spawned and added to the map's room. When it unloads, they're despawned (and Lightyear handles replication cleanup).

---

## Code References

### Voxel Map Engine
- `crates/voxel_map_engine/src/ticket.rs` — ChunkTicket, TicketType, LoadState, column_to_chunks
- `crates/voxel_map_engine/src/generation.rs` — ChunkGenResult, PendingChunks, spawn_chunk_gen_task
- `crates/voxel_map_engine/src/lifecycle.rs` — update_chunks, poll_chunk_tasks, despawn_out_of_range_chunks, remove_column_chunks
- `crates/voxel_map_engine/src/instance.rs` — VoxelMapInstance, chunk_levels, dirty_chunks
- `crates/voxel_map_engine/src/persistence.rs` — save_chunk, load_chunk, chunk_file_path
- `crates/voxel_map_engine/src/config.rs` — VoxelGenerator, VoxelMapConfig
- `crates/voxel_map_engine/src/terrain.rs` — HeightMap, MoistureMap, BiomeRules, build_generator

### World Object System
- `crates/protocol/src/world_object/types.rs:46` — WorldObjectDef (Vec<Box<dyn PartialReflect>>)
- `crates/protocol/src/world_object/spawn.rs:8` — apply_object_components
- `crates/protocol/src/world_object/loader.rs` — WorldObjectLoader (custom AssetLoader with TypeRegistry)
- `crates/protocol/src/reflect_loader.rs` — deserialize_component_map (TypedReflectDeserializer)
- `crates/server/src/world_object.rs:21` — spawn_world_object
- `crates/client/src/world_object.rs:30` — on_world_object_replicated

### Entity Persistence
- `crates/protocol/src/map/persistence.rs:1-19` — SavedEntity, SavedEntityKind, MapSaveTarget
- `crates/server/src/persistence.rs:67-107` — save_entities, load_entities (flat entities.bin)
- `crates/server/src/persistence.rs:21-28` — WorldSavePath resource

### Server Map
- `crates/server/src/map.rs:94-123` — spawn_overworld (terrain def application)
- `crates/server/src/map.rs:129-148` — apply_terrain_defs
- `crates/server/src/gameplay.rs:236` — spawn_test_tree (only current object spawn)

### Terrain Assets
- `assets/terrain/overworld.terrain.ron` — Full terrain with heightmap, moisture, biomes
- `assets/terrain/homebase.terrain.ron` — Empty (flat terrain)
- `assets/terrain/arena_hills.terrain.ron` — Heightmap only, single biome
- `assets/objects/tree_circle.object.ron` — Only world object definition

---

## Architecture Documentation

### Current Generation Flow

```
ChunkTicket (player position) → TicketLevelPropagator → column levels
    → spawn_missing_chunks → spawn_chunk_gen_task (async)
        → try disk load, fallback to VoxelGenerator closure
        → ChunkGenResult { voxels, mesh }
    → poll_chunk_tasks → insert voxel data + spawn mesh entity
```

### Proposed Generation Flow (With Feature Placement)

```
ChunkTicket → TicketLevelPropagator → column levels
    → spawn_missing_chunks → spawn_chunk_gen_task (async)
        → try disk load for terrain
        → try disk load for entities (entities/chunk_X_Y_Z.entities.bin)
        → if no entity file: run PlacementRules → WorldObjectSpawn list
        → ChunkGenResult { voxels, mesh, entity_spawns }
    → poll_chunk_tasks
        → insert voxel data + spawn mesh entity
        → for each entity_spawn: spawn_world_object + tag ChunkEntityRef
    → on chunk eviction:
        → serialize ChunkEntityRef entities → write entity file
        → despawn entities
```

### Persistence Layout

```
worlds/overworld/
  map.meta.bin                          # map metadata (existing)
  entities.bin                          # map-global entities: respawn points (existing)
  terrain/
    chunk_0_0_0.bin                     # voxel data (existing)
    chunk_1_0_2.bin
  entities/                             # NEW: per-chunk entity files
    chunk_0_0_0.entities.bin
    chunk_1_0_2.entities.bin
```

### Entity Ownership

```
Map-global entities (existing):
  RespawnPoint + MapSaveTarget → entities.bin (load once at map spawn)

Chunk-bound entities (new):
  WorldObject + ChunkEntityRef → entities/chunk_X_Y_Z.entities.bin (load/unload with chunk)
```

---

## Historical Context (from doc/)

- `doc/research/2026-03-18-procedural-map-generation.md` — Comprehensive research on terrain generation, noise configuration, placement algorithms, and per-chunk entity persistence design. Most of the architectural decisions for feature placement originate here.
- `doc/research/2026-03-13-world-object-ron-assets.md` — World object definition system design, reflect-based component bags, asset loading pipeline.
- `doc/research/2026-03-20-minecraft-chunk-ticket-system.md` — Minecraft's ticket system research that informed the chunk ticket implementation.
- `doc/plans/2026-03-21-chunk-ticket-system.md` — The recently completed chunk ticket system plan.
- `doc/plans/2026-03-19-procedural-map-generation.md` — Procedural generation plan (terrain implemented, placement not yet).
- `doc/research/2026-03-09-minecraft-style-map-directory-saving.md` — Map-as-directory persistence pattern.

---

## Related Research

- [2026-03-18-procedural-map-generation.md](2026-03-18-procedural-map-generation.md) — Prior research covering terrain gen + placement design
- [2026-03-13-world-object-ron-assets.md](2026-03-13-world-object-ron-assets.md) — World object system design
- [2026-03-20-minecraft-chunk-ticket-system.md](2026-03-20-minecraft-chunk-ticket-system.md) — Minecraft ticket system
- [2026-03-09-minecraft-style-map-directory-saving.md](2026-03-09-minecraft-style-map-directory-saving.md) — Map persistence

---

## Open Questions

1. **Entity file format**: Should per-chunk entity files use the same `ChunkEntity` format from prior research (with `procedural_id`, `destroyed`, `component_overrides`), or a simpler format given the "generate once, save forever" model? The simpler model only needs `object_id`, `position`, and serialized component state.

2. **Placement during generation vs. post-generation**: Should the placement algorithm run inside the async generation task (has access to voxel data for height sampling) or as a separate system after terrain is inserted? Running inside the task is more efficient (single async pass) but couples placement to the voxel engine. Running as a separate system keeps concerns separated but requires an extra frame and re-reading voxel data.

3. **Cross-chunk spacing**: Accept boundary artifacts (objects may cluster at chunk borders) or regenerate neighbor positions for spacing checks? The former is simpler; the latter produces better visual results but is more complex and slower.

4. **Entity ticking zones**: When should `LoadState` start driving entity behavior? It could be part of this plan (entities freeze at `BlockTicking`, unload at `Border`) or deferred to a separate plan. This determines whether `ChunkEntityRef` entities need tick-control logic.

5. **Client-side entity prediction**: Should clients spawn placeholder entities when chunks load (before server replication arrives) or wait for server authority? Current approach: server-authoritative, clients wait for Lightyear replication.

## Sources

- [World generation — Minecraft Wiki](https://minecraft.wiki/w/World_generation)
- [Placed feature — Minecraft Wiki](https://minecraft.wiki/w/Placed_feature)
- [Configured feature — Minecraft Wiki](https://minecraft.wiki/w/Configured_feature)
- [Structure set — Minecraft Wiki](https://minecraft.wiki/w/Structure_set)
- [Chunk format — Minecraft Wiki](https://minecraft.wiki/w/Chunk_format)
- [Entity format — Minecraft Wiki](https://minecraft.wiki/w/Entity_format)
- [Java Edition level format — Minecraft Wiki](https://minecraft.wiki/w/Java_Edition_level_format)
- [Point of Interest format — Minecraft Wiki](https://minecraft.wiki/w/Point_of_Interest_format)
- [PersistentEntitySectionManager — NeoForge Javadoc](https://nekoyue.github.io/ForgeJavaDocs-NG/javadoc/1.21.x-neoforge/net/minecraft/world/level/entity/PersistentEntitySectionManager.html)
- [Misode's Placed Features Guide](https://misode.github.io/guides/placed-features/)
