# Map-as-Directory Saving Implementation Plan

## Overview

Replace the single flat `world_save/voxel_world.bin` modifications file with a per-map directory structure that saves full chunk terrain data, entity state, and map metadata. Fix the unused octree bug so `VoxelMapInstance.tree` is the source of truth for loaded chunk voxel data. Bake voxel edits directly into the octree via `PalettedChunk::set` — eliminating `modified_voxels`, `write_buffer`, and the regenerate-on-edit cycle. Edits mutate chunk data in-place and trigger async remeshing (old mesh stays visible until the new one is ready). Extend persistence to support all map types (Overworld, Homebase). Replace client-side chunk generation with server-to-client chunk streaming. Add client-side block edit prediction with sequence-number acknowledgment. Add batched multi-block updates.

## Current State Analysis

- **Persistence**: Single bincode file storing `Vec<(IVec3, VoxelType)>` modifications only
- **Octree**: `VoxelMapInstance.tree: OctreeI32<Option<ChunkData>>` is declared but never read/written at runtime — this is a bug
- **Chunk pipeline**: Generates voxels → meshes → **discards voxel data**. Lookups re-invoke the generator each time
- **Edit pipeline**: `set_voxel` → `write_buffer` → `flush_write_buffer` → `modified_voxels` + chunk invalidation → full chunk regeneration with overrides → remesh. Wasteful: evicts chunk from octree and forces full regeneration just to apply one voxel change
- **Entity persistence**: None. Respawn points hardcoded at `Vec3(0.0, 5.0, 0.0)`
- **Multi-map**: Only Overworld saved. Homebases are ephemeral

### Key Discoveries:
- `WorldVoxel` already derives `Serialize, Deserialize` ([types.rs:12](crates/voxel_map_engine/src/types.rs#L12))
- `ChunkData` and `FillType` lack serde derives ([types.rs:26-39](crates/voxel_map_engine/src/types.rs#L26-L39))
- `ChunkGenResult` only carries `position` and `mesh` — voxel data is dropped ([generation.rs:14-17](crates/voxel_map_engine/src/generation.rs#L14-L17))
- `get_voxel` re-invokes generator every call, `raycast` uses a single-chunk cache ([api.rs:22-33](crates/voxel_map_engine/src/api.rs#L22-L33), [api.rs:69](crates/voxel_map_engine/src/api.rs#L69))
- OctreeI32 API: `fill_path_to_node_from_root` + `find_node` + `get_value_mut` for insert; `drop_tree` for removal
- Existing tests use `#[test]` functions with `MinimalPlugins` app or direct function calls ([voxel_persistence.rs](crates/server/tests/voxel_persistence.rs))
- `CrossbeamTestStepper` exists for client-server integration tests ([integration.rs:25-173](crates/server/tests/integration.rs#L25-L173))
- Spawn points hardcoded in 4 places: [gameplay.rs:109](crates/server/src/gameplay.rs#L109), [gameplay.rs:150](crates/server/src/gameplay.rs#L150), [gameplay.rs:192](crates/server/src/gameplay.rs#L192), [map.rs:515](crates/server/src/map.rs#L515)
- Current debounced save: 1s after last edit, 5s max dirty ([map.rs:85-113](crates/server/src/map.rs#L85-L113))

## Desired End State

```
worlds/
  overworld/
    map.meta.bin             # seed, generation_version, spawn points
    terrain/
      chunk_0_0_0.bin        # bincode + zstd compressed ChunkData
      chunk_1_0_0.bin
      ...
    entities.bin             # flat list of persistable entities

  homebase-<owner_id>/
    map.meta.bin
    terrain/
      ...
    entities.bin
```

- Full chunk voxel data saved per-chunk, not just modifications
- Chunk data retained in octree while loaded; octree used for voxel lookups (no generator re-invocation)
- Map metadata (seed, spawn points) persisted per-map
- Persistable entities (respawn points, future doodads) saved per-map
- All map types (Overworld, Homebase) persist independently
- Per-chunk dirty tracking — only modified chunks re-saved
- zstd compression on chunk files
- No `modified_voxels` or `write_buffer` — edits baked directly into octree chunk data via `PalettedChunk::set`
- No chunk regeneration on edit — mutate in-place + async remesh only
- Old mesh stays visible until async remesh completes

### Verification:
- Server saves world state to directory structure on debounced timer and shutdown
- Server loads world state from directories on startup
- Voxel modifications survive server restart
- Spawn points load from metadata (no hardcoded fallback needed after first save)
- Homebase state persists across server restarts
- Clients receive chunk data from server (no local generation)
- Block edits feel instant (client optimistic apply) with server reconciliation
- Multi-block changes in the same chunk are batched into one network message
- Integration test validates full save/load cycle with client-server setup

## What We're NOT Doing

- Region files (8x8x8 chunk grouping) — per-chunk files for now, region files can be added later for filesystem efficiency
- Arena persistence — Arenas are not in `MapInstanceId` yet
- Player data persistence (inventory, stats) — separate feature
- Pre-authored terrain import — save format supports it but no tooling
- Client-side save/load — server-only persistence
- Live migration from old save format — old `world_save/voxel_world.bin` files become incompatible (acceptable since this is pre-release)
- Light engine or light data sync — no lighting system exists yet

## Implementation Approach

Eight phases, each building on the previous:

1. **Fix Octree** — retain chunk data after generation, use for lookups (prerequisite for everything)
2. **Palette-Based Chunk Storage & Direct Mutation** — compress chunk data with palette encoding; eliminate `modified_voxels`/`write_buffer`/`flush_write_buffer`; bake edits directly into octree via `PalettedChunk::set`; add async remesh pipeline (old mesh stays until new one ready)
3. **Directory Structure & Persistence** — per-map directories, per-chunk terrain files, map metadata (server-only persistence; networking unchanged)
4. **Entity Persistence** — `MapSaveTarget` marker, respawn point save/load
5. **Server-to-Client Chunk Streaming** — server sends palette-compressed chunks to clients, client stops generating locally, remove `VoxelStateSync`/`VoxelModifications`/`MapWorld`
6. **Block Edit Prediction** — sequence-number system for client optimistic updates, server ack, room-scoped broadcasts
7. **Batched Section Updates** — accumulate per-chunk changes per tick, batch into single message
8. **Multi-Map Persistence** — homebase save/load, per-map save directories, map transition with chunk streaming

Each phase includes unit tests for all new code paths. An integration test file `crates/server/tests/world_persistence.rs` is started in Phase 3 and extended through Phase 8.

---

## Phase 1: Fix Octree — Retain ChunkData After Generation

### Overview
Fix the bug where `VoxelMapInstance.tree` is never populated. After chunk generation, store the voxel data in the octree. Use the octree for voxel lookups instead of re-invoking the generator.

### Changes Required:

#### 1. Return voxel data from chunk generation
**File**: `crates/voxel_map_engine/src/generation.rs`

Add voxels to `ChunkGenResult`:

```rust
pub struct ChunkGenResult {
    pub position: IVec3,
    pub mesh: Option<Mesh>,
    pub voxels: Vec<WorldVoxel>,
}
```

In `generate_chunk`, return the voxels:

```rust
fn generate_chunk(...) -> ChunkGenResult {
    let mut voxels = generator(position);
    apply_overrides(&mut voxels, position, overrides);
    let mesh = mesh_chunk_greedy(&voxels);
    ChunkGenResult { position, mesh, voxels }
}
```

#### 2. Add octree helper methods to VoxelMapInstance
**File**: `crates/voxel_map_engine/src/instance.rs`

```rust
use grid_tree::{NodeKey, VisitCommand};
use crate::types::ChunkData;

impl VoxelMapInstance {
    /// Insert chunk data into the octree at the given chunk position.
    pub fn insert_chunk_data(&mut self, chunk_pos: IVec3, data: ChunkData) {
        let key = NodeKey::new(0, chunk_pos);
        self.tree.fill_path_to_node_from_root(key, |_key, entry| {
            entry.or_insert_with(|| None);
            VisitCommand::Continue
        });
        let relation = self.tree.find_node(key).expect("just created path");
        *self.tree.get_value_mut(relation.child).expect("just created node") = Some(data);
    }

    /// Remove chunk data from the octree. Returns the data if it existed.
    pub fn remove_chunk_data(&mut self, chunk_pos: IVec3) -> Option<ChunkData> {
        let key = NodeKey::new(0, chunk_pos);
        let relation = self.tree.find_node(key)?;
        let value = self.tree.get_value_mut(relation.child)?;
        value.take()
    }

    /// Get a reference to chunk data in the octree.
    pub fn get_chunk_data(&self, chunk_pos: IVec3) -> Option<&ChunkData> {
        let key = NodeKey::new(0, chunk_pos);
        let relation = self.tree.find_node(key)?;
        self.tree.get_value(relation.child)?.as_ref()
    }
}
```

#### 3. Store chunk data in octree on completion
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

In `handle_completed_chunk`, after `instance.loaded_chunks.insert(result.position)`:

```rust
use crate::types::{ChunkData, FillType, WorldVoxel};

fn classify_fill_type(voxels: &[WorldVoxel]) -> FillType {
    let first = voxels.first().copied().unwrap_or(WorldVoxel::Air);
    if voxels.iter().all(|&v| v == first) {
        if first == WorldVoxel::Air { FillType::Empty } else { FillType::Uniform(first) }
    } else {
        FillType::Mixed
    }
}

fn compute_chunk_hash(voxels: &[WorldVoxel]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    voxels.hash(&mut hasher);
    hasher.finish()
}

// In handle_completed_chunk, after loaded_chunks.insert:
let fill_type = classify_fill_type(&result.voxels);
let hash = compute_chunk_hash(&result.voxels);
let chunk_data = ChunkData { voxels: result.voxels, fill_type, hash };
instance.insert_chunk_data(result.position, chunk_data);
```

#### 4. Evict chunk data from octree on unload
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Chunks are evicted from the octree when they leave the loaded range. This keeps memory bounded to only the chunks currently in view. In Phase 3+, dirty chunks will be saved to disk before eviction; in Phase 1-2, eviction simply drops the data (it can be regenerated).

In `remove_out_of_range_chunks`, when removing from `loaded_chunks`, also evict from octree:

```rust
fn remove_out_of_range_chunks(instance: &mut VoxelMapInstance, desired: &HashSet<IVec3>) {
    let removed: Vec<IVec3> = instance.loaded_chunks.iter()
        .filter(|pos| !desired.contains(pos))
        .copied()
        .collect();
    for pos in removed {
        instance.loaded_chunks.remove(&pos);
        instance.remove_chunk_data(pos);
    }
}
```

In `flush_write_buffer`, when invalidating chunks, also evict from octree (the chunk will be regenerated with the new voxel data applied):

```rust
for pos in invalidated {
    instance.loaded_chunks.remove(&pos);
    instance.remove_chunk_data(pos);
}
```

#### 5. Use octree for voxel lookups
**File**: `crates/voxel_map_engine/src/api.rs`

In `get_voxel`, check octree before falling back to generator:

```rust
pub fn get_voxel(&self, map: Entity, pos: IVec3) -> WorldVoxel {
    let Ok((instance, config)) = self.maps.get(map) else {
        warn!("get_voxel: entity {map:?} has no VoxelMapInstance");
        return WorldVoxel::Unset;
    };

    if let Some(&voxel) = instance.modified_voxels.get(&pos) {
        return voxel;
    }

    let chunk_pos = voxel_to_chunk_pos(pos);
    if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
        return lookup_voxel_in_chunk(&chunk_data.voxels, pos, chunk_pos);
    }

    evaluate_voxel_at(pos, &config.generator)
}
```

Similarly update `lookup_voxel` for raycast to check octree:

```rust
fn lookup_voxel(
    voxel_pos: IVec3,
    instance: &VoxelMapInstance,
    generator: &crate::config::VoxelGenerator,
    cached_chunk: &mut Option<(IVec3, Vec<WorldVoxel>)>,
) -> WorldVoxel {
    if let Some(&voxel) = instance.modified_voxels.get(&voxel_pos) {
        return voxel;
    }

    let chunk_pos = voxel_to_chunk_pos(voxel_pos);

    // Check octree first
    if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
        return lookup_voxel_in_chunk(&chunk_data.voxels, voxel_pos, chunk_pos);
    }

    // Fall back to generator with cache
    let needs_generate = match cached_chunk.as_ref() {
        Some((cached_pos, _)) if *cached_pos == chunk_pos => false,
        _ => true,
    };
    if needs_generate {
        *cached_chunk = Some((chunk_pos, generator(chunk_pos)));
    }

    let (_, voxels) = cached_chunk.as_ref().unwrap();
    lookup_voxel_in_chunk(voxels, voxel_pos, chunk_pos)
}
```

### Unit Tests

**File**: `crates/voxel_map_engine/src/instance.rs` (extend existing test module)

```rust
#[test]
fn insert_and_retrieve_chunk_data() {
    let mut instance = VoxelMapInstance::new(5);
    let pos = IVec3::new(1, 0, 2);
    let chunk = ChunkData::new_empty();
    instance.insert_chunk_data(pos, chunk);
    assert!(instance.get_chunk_data(pos).is_some());
    assert_eq!(instance.get_chunk_data(pos).unwrap().fill_type, FillType::Empty);
}

#[test]
fn remove_chunk_data_returns_data() {
    let mut instance = VoxelMapInstance::new(5);
    let pos = IVec3::ZERO;
    instance.insert_chunk_data(pos, ChunkData::new_empty());
    let removed = instance.remove_chunk_data(pos);
    assert!(removed.is_some());
    assert!(instance.get_chunk_data(pos).is_none());
}

#[test]
fn remove_nonexistent_chunk_returns_none() {
    let mut instance = VoxelMapInstance::new(5);
    assert!(instance.remove_chunk_data(IVec3::ZERO).is_none());
}

#[test]
fn get_nonexistent_chunk_returns_none() {
    let instance = VoxelMapInstance::new(5);
    assert!(instance.get_chunk_data(IVec3::new(99, 99, 99)).is_none());
}

#[test]
fn overwrite_chunk_data() {
    let mut instance = VoxelMapInstance::new(5);
    let pos = IVec3::ZERO;
    instance.insert_chunk_data(pos, ChunkData::new_empty());

    let mut solid_chunk = ChunkData::new_empty();
    solid_chunk.voxels[0] = WorldVoxel::Solid(1);
    solid_chunk.fill_type = FillType::Mixed;
    instance.insert_chunk_data(pos, solid_chunk);

    let data = instance.get_chunk_data(pos).unwrap();
    assert_eq!(data.fill_type, FillType::Mixed);
    assert_eq!(data.voxels[0], WorldVoxel::Solid(1));
}
```

**File**: `crates/voxel_map_engine/src/lifecycle.rs` (extend existing test module)

```rust
#[test]
fn classify_fill_type_empty() {
    let voxels = vec![WorldVoxel::Air; 100];
    assert_eq!(classify_fill_type(&voxels), FillType::Empty);
}

#[test]
fn classify_fill_type_uniform_solid() {
    let voxels = vec![WorldVoxel::Solid(5); 100];
    assert_eq!(classify_fill_type(&voxels), FillType::Uniform(WorldVoxel::Solid(5)));
}

#[test]
fn classify_fill_type_mixed() {
    let mut voxels = vec![WorldVoxel::Air; 100];
    voxels[0] = WorldVoxel::Solid(1);
    assert_eq!(classify_fill_type(&voxels), FillType::Mixed);
}
```

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client -c 1`

#### Manual Verification:
- [ ] Voxel lookups work correctly in-game (place/break blocks, walk on terrain)
- [ ] No visual regressions — chunks render identically to before
- [ ] Memory usage is reasonable (chunk data retained in octree while loaded)

---

## Phase 2: Palette-Based Chunk Storage & Direct Mutation

### Overview
Replace `ChunkData`'s flat `Vec<WorldVoxel>` with a `PalettedChunk` that stores a local palette of unique voxel types and packed bit indices. Uniform chunks (all air, all solid) store only the single palette entry with no index array. This reduces memory ~4-23× for typical chunks and produces smaller save files.

Eliminate `modified_voxels`, `write_buffer`, and `flush_write_buffer` entirely. When a voxel edit arrives, mutate the `PalettedChunk` in the octree directly via `PalettedChunk::set`, update neighbor chunk padding for boundary voxels, mark the chunk dirty, and spawn an async remesh task. The old mesh stays visible until the new one is ready. This replaces the wasteful invalidate → regenerate cycle with in-place mutation + remesh.

**Regression note**: Until Phase 3 adds persistence, edits to chunks that are later evicted from the octree are lost (no `modified_voxels` to remember them, no disk save yet). Phase 3 fixes this by saving dirty chunks before eviction.

With only 3 voxel variants (`Air`, `Unset`, `Solid(u8)`) — at most ~257 unique values — most chunks need ≤8 bits/entry. A chunk with 2 distinct voxels uses 1 bit × 4096 = 512 bytes vs 11,664 bytes flat.

### Changes Required:

#### 1. Create PalettedChunk type
**File**: `crates/voxel_map_engine/src/palette.rs` (new)

```rust
use serde::{Serialize, Deserialize};
use crate::types::{WorldVoxel, CHUNK_SIZE};

const INNER_VOLUME: usize = (CHUNK_SIZE as usize).pow(3); // 4096
const PADDED_VOLUME: usize = 18 * 18 * 18; // 5832

/// Palette-based chunk storage with two strategies.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PalettedChunk {
    /// All voxels are the same value. ~0 bytes.
    SingleValue(WorldVoxel),

    /// 2-256 distinct voxel types. Palette array + packed bit indices.
    Indirect {
        palette: Vec<WorldVoxel>,
        /// Packed indices into palette. Each index is `bits_per_entry` wide.
        /// Stored as Vec<u64> with indices packed left-to-right within each u64.
        data: Vec<u64>,
        bits_per_entry: u8,
        len: usize,
    },
}

impl PalettedChunk {
    /// Build from a flat voxel array (the 18^3 padded array from generation).
    pub fn from_voxels(voxels: &[WorldVoxel]) -> Self {
        debug_assert_eq!(voxels.len(), PADDED_VOLUME);

        // Build palette
        let mut palette = Vec::new();
        for &v in voxels {
            if !palette.contains(&v) {
                palette.push(v);
            }
        }

        if palette.len() == 1 {
            return Self::SingleValue(palette[0]);
        }

        let bits_per_entry = bits_needed(palette.len());
        let entries_per_u64 = 64 / bits_per_entry as usize;
        let num_u64s = (voxels.len() + entries_per_u64 - 1) / entries_per_u64;
        let mut data = vec![0u64; num_u64s];
        let mask = (1u64 << bits_per_entry) - 1;

        for (i, &voxel) in voxels.iter().enumerate() {
            let palette_idx = palette.iter().position(|&p| p == voxel)
                .expect("voxel must be in palette") as u64;
            let u64_index = i / entries_per_u64;
            let bit_offset = (i % entries_per_u64) * bits_per_entry as usize;
            data[u64_index] |= (palette_idx & mask) << bit_offset;
        }

        Self::Indirect { palette, data, bits_per_entry, len: voxels.len() }
    }

    /// Expand back to a flat voxel array.
    pub fn to_voxels(&self) -> Vec<WorldVoxel> {
        match self {
            Self::SingleValue(v) => vec![*v; PADDED_VOLUME],
            Self::Indirect { palette, data, bits_per_entry, len } => {
                let bpe = *bits_per_entry as usize;
                let entries_per_u64 = 64 / bpe;
                let mask = (1u64 << bpe) - 1;
                let mut voxels = Vec::with_capacity(*len);

                for i in 0..*len {
                    let u64_index = i / entries_per_u64;
                    let bit_offset = (i % entries_per_u64) * bpe;
                    let idx = ((data[u64_index] >> bit_offset) & mask) as usize;
                    debug_assert!(idx < palette.len(), "palette index {idx} out of bounds (palette len {})", palette.len());
                    voxels.push(palette[idx]);
                }
                voxels
            }
        }
    }

    /// Get a single voxel by linear index (padded array index).
    pub fn get(&self, index: usize) -> WorldVoxel {
        match self {
            Self::SingleValue(v) => *v,
            Self::Indirect { palette, data, bits_per_entry, len } => {
                debug_assert!(index < *len, "index {index} out of bounds (len {len})");
                let bpe = *bits_per_entry as usize;
                let entries_per_u64 = 64 / bpe;
                let mask = (1u64 << bpe) - 1;
                let u64_index = index / entries_per_u64;
                let bit_offset = (index % entries_per_u64) * bpe;
                let idx = ((data[u64_index] >> bit_offset) & mask) as usize;
                palette[idx]
            }
        }
    }

    /// Set a single voxel. May need to rebuild if a new voxel type is introduced
    /// or if transitioning from SingleValue to Indirect.
    pub fn set(&mut self, index: usize, voxel: WorldVoxel) {
        match self {
            Self::SingleValue(v) => {
                if *v == voxel {
                    return; // no-op
                }
                // Transition to Indirect
                let mut voxels = vec![*v; PADDED_VOLUME];
                voxels[index] = voxel;
                *self = Self::from_voxels(&voxels);
            }
            Self::Indirect { palette, data, bits_per_entry, len } => {
                // Check if voxel is already in palette
                if let Some(palette_idx) = palette.iter().position(|&p| p == voxel) {
                    let bpe = *bits_per_entry as usize;
                    let entries_per_u64 = 64 / bpe;
                    let mask = (1u64 << bpe) - 1;
                    let u64_index = index / entries_per_u64;
                    let bit_offset = (index % entries_per_u64) * bpe;
                    data[u64_index] &= !(mask << bit_offset);
                    data[u64_index] |= (palette_idx as u64 & mask) << bit_offset;
                } else {
                    // New voxel type — rebuild with expanded palette
                    let mut voxels = self.to_voxels();
                    voxels[index] = voxel;
                    *self = Self::from_voxels(&voxels);
                }
            }
        }
    }

    /// Returns true if all voxels are the same.
    pub fn is_uniform(&self) -> bool {
        matches!(self, Self::SingleValue(_))
    }

    /// Number of distinct voxel types in this chunk.
    pub fn palette_size(&self) -> usize {
        match self {
            Self::SingleValue(_) => 1,
            Self::Indirect { palette, .. } => palette.len(),
        }
    }

    /// Approximate memory usage in bytes.
    pub fn memory_usage(&self) -> usize {
        match self {
            Self::SingleValue(_) => std::mem::size_of::<WorldVoxel>(),
            Self::Indirect { palette, data, .. } => {
                palette.len() * std::mem::size_of::<WorldVoxel>()
                    + data.len() * std::mem::size_of::<u64>()
            }
        }
    }
}

/// Minimum bits needed to represent `count` distinct values.
fn bits_needed(count: usize) -> u8 {
    if count <= 1 { return 0; }
    let bits = (count as f64).log2().ceil() as u8;
    bits.max(1) // minimum 1 bit for 2 values
}
```

#### 2. Replace ChunkData.voxels with PalettedChunk
**File**: `crates/voxel_map_engine/src/types.rs`

```rust
use crate::palette::PalettedChunk;

#[derive(Clone, Serialize, Deserialize)]
pub struct ChunkData {
    pub voxels: PalettedChunk,  // was: Vec<WorldVoxel>
    pub fill_type: FillType,
    pub hash: u64,
}

impl ChunkData {
    pub fn new_empty() -> Self {
        Self {
            voxels: PalettedChunk::SingleValue(WorldVoxel::Air),
            fill_type: FillType::Empty,
            hash: 0,
        }
    }

    /// Construct from a flat voxel array (generation output).
    pub fn from_voxels(flat: &[WorldVoxel]) -> Self {
        let fill_type = classify_fill_type(flat);
        let hash = compute_chunk_hash(flat);
        let voxels = PalettedChunk::from_voxels(flat);
        Self { voxels, fill_type, hash }
    }
}
```

Move `classify_fill_type` and `compute_chunk_hash` from `lifecycle.rs` into `types.rs` as module-level functions (or `ChunkData` associated functions) since they're now used here.

#### 3. Update lifecycle.rs to construct ChunkData via from_voxels
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

In `handle_completed_chunk`, replace the manual ChunkData construction from Phase 1:

```rust
// Before (Phase 1):
let fill_type = classify_fill_type(&result.voxels);
let hash = compute_chunk_hash(&result.voxels);
let chunk_data = ChunkData { voxels: result.voxels, fill_type, hash };

// After (Phase 2):
let chunk_data = ChunkData::from_voxels(&result.voxels);
```

#### 4. Update api.rs voxel lookups to use PalettedChunk
**File**: `crates/voxel_map_engine/src/api.rs`

In `get_voxel`, remove the `modified_voxels` check (no longer exists) and use indexed PalettedChunk access:

```rust
pub fn get_voxel(&self, map: Entity, pos: IVec3) -> WorldVoxel {
    let Ok((instance, config)) = self.maps.get(map) else {
        warn!("get_voxel: entity {map:?} has no VoxelMapInstance");
        return WorldVoxel::Unset;
    };

    let chunk_pos = voxel_to_chunk_pos(pos);
    if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
        let local = pos - chunk_pos * CHUNK_SIZE as i32;
        let padded = [
            (local.x + 1) as u32,
            (local.y + 1) as u32,
            (local.z + 1) as u32,
        ];
        let index = PaddedChunkShape::linearize(padded) as usize;
        return chunk_data.voxels.get(index);
    }

    evaluate_voxel_at(pos, &config.generator)
}
```

Similarly update `lookup_voxel` for raycast — remove `modified_voxels` check, check octree first, fall back to generator with cache.

#### 5. Remove modified_voxels, write_buffer, flush_write_buffer
**File**: `crates/voxel_map_engine/src/instance.rs`

Remove `modified_voxels` and `write_buffer` fields from `VoxelMapInstance`. Add `chunks_needing_remesh` and `dirty_chunks`:

```rust
pub struct VoxelMapInstance {
    pub tree: OctreeI32<Option<ChunkData>>,
    pub loaded_chunks: HashSet<IVec3>,
    pub dirty_chunks: HashSet<IVec3>,           // NEW — unsaved changes
    pub chunks_needing_remesh: HashSet<IVec3>,  // NEW — need async remesh
    pub debug_colors: bool,
}
```

Update all constructors (`new`, `overworld`, `homebase`, `arena`) to remove `modified_voxels`/`write_buffer` and initialize the new fields as empty.

Add `get_chunk_data_mut` method:

```rust
impl VoxelMapInstance {
    /// Get a mutable reference to chunk data in the octree.
    pub fn get_chunk_data_mut(&mut self, chunk_pos: IVec3) -> Option<&mut ChunkData> {
        let key = NodeKey::new(0, chunk_pos);
        let relation = self.tree.find_node(key)?;
        self.tree.get_value_mut(relation.child)?.as_mut()
    }
}
```

**File**: `crates/voxel_map_engine/src/lifecycle.rs`

**Remove** `flush_write_buffer` system entirely.

**File**: `crates/voxel_map_engine/src/lib.rs`

Remove `flush_write_buffer` from the system chain. Add remesh systems (see step 8):

```rust
(
    lifecycle::ensure_pending_chunks,
    lifecycle::update_chunks,
    lifecycle::poll_chunk_tasks,
    lifecycle::despawn_out_of_range_chunks,
    lifecycle::spawn_remesh_tasks,   // NEW — replaces flush_write_buffer
    lifecycle::poll_remesh_tasks,    // NEW
)
    .chain(),
```

#### 6. Direct octree mutation in VoxelMapInstance
**File**: `crates/voxel_map_engine/src/instance.rs`

Add `set_voxel` method that mutates the octree in-place and updates neighbor padding:

```rust
impl VoxelMapInstance {
    /// Mutate a voxel directly in the octree. Marks the chunk dirty and
    /// queues it for async remesh. Also updates neighbor chunk padding
    /// for boundary voxels.
    ///
    /// If the chunk is not loaded, the edit is silently dropped (the chunk
    /// will be regenerated fresh when it comes into view).
    pub fn set_voxel(&mut self, world_pos: IVec3, voxel: WorldVoxel) {
        let chunk_pos = voxel_to_chunk_pos(world_pos);
        let local = world_pos - chunk_pos * CHUNK_SIZE as i32;

        // Mutate owning chunk
        {
            let Some(chunk_data) = self.get_chunk_data_mut(chunk_pos) else {
                trace!(
                    "set_voxel: chunk {chunk_pos} not loaded, edit at {world_pos} dropped"
                );
                return;
            };
            let padded = [
                (local.x + 1) as u32,
                (local.y + 1) as u32,
                (local.z + 1) as u32,
            ];
            let index = PaddedChunkShape::linearize(padded) as usize;
            chunk_data.voxels.set(index, voxel);
        } // tree borrow released

        self.dirty_chunks.insert(chunk_pos);
        self.chunks_needing_remesh.insert(chunk_pos);

        // Update neighbor chunks' padding for boundary voxels
        self.update_neighbor_padding(world_pos, chunk_pos, local, voxel);
    }

    /// For each axis, if the edited voxel sits on a chunk boundary, update the
    /// neighboring chunk's padding voxel and mark it for remesh.
    fn update_neighbor_padding(
        &mut self,
        _world_pos: IVec3,
        chunk_pos: IVec3,
        local: IVec3,
        voxel: WorldVoxel,
    ) {
        for axis in 0..3 {
            let l = local[axis];
            if l == 0 {
                // Negative boundary — neighbor's positive padding
                let mut neighbor = chunk_pos;
                neighbor[axis] -= 1;
                {
                    if let Some(nd) = self.get_chunk_data_mut(neighbor) {
                        let mut pl = local;
                        pl[axis] = CHUNK_SIZE as i32; // padding slot at far end
                        let padded = [
                            (pl.x + 1) as u32,
                            (pl.y + 1) as u32,
                            (pl.z + 1) as u32,
                        ];
                        let idx = PaddedChunkShape::linearize(padded) as usize;
                        nd.voxels.set(idx, voxel);
                    }
                }
                self.chunks_needing_remesh.insert(neighbor);
            }
            if l == CHUNK_SIZE as i32 - 1 {
                // Positive boundary — neighbor's negative padding
                let mut neighbor = chunk_pos;
                neighbor[axis] += 1;
                {
                    if let Some(nd) = self.get_chunk_data_mut(neighbor) {
                        let mut pl = local;
                        pl[axis] = -1; // padding slot at near end
                        let padded = [
                            (pl.x + 1) as u32,
                            (pl.y + 1) as u32,
                            (pl.z + 1) as u32,
                        ];
                        let idx = PaddedChunkShape::linearize(padded) as usize;
                        nd.voxels.set(idx, voxel);
                    }
                }
                self.chunks_needing_remesh.insert(neighbor);
            }
        }
    }
}
```

#### 7. Update VoxelWorld::set_voxel to use direct mutation
**File**: `crates/voxel_map_engine/src/api.rs`

Replace the old `set_voxel` (which pushed to write_buffer) with a direct call to the instance:

```rust
pub fn set_voxel(&mut self, map: Entity, pos: IVec3, voxel: WorldVoxel) {
    debug_assert!(voxel != WorldVoxel::Unset, "cannot set voxel to Unset");
    let Ok((mut instance, _)) = self.maps.get_mut(map) else {
        warn!("set_voxel: entity {map:?} has no VoxelMapInstance");
        return;
    };
    instance.set_voxel(pos, voxel);
}
```

#### 8. Async remesh pipeline
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Add types and systems for async remeshing. The old mesh stays visible until the new one is ready.

```rust
use bevy::tasks::Task;

/// A pending async remesh task for a chunk that was mutated in-place.
struct RemeshTask {
    chunk_pos: IVec3,
    task: Task<Option<Mesh>>,
}

/// Component tracking pending remesh tasks for a map instance.
#[derive(Component, Default)]
pub struct PendingRemeshes {
    tasks: Vec<RemeshTask>,
}
```

Register `PendingRemeshes` alongside `PendingChunks` in `ensure_pending_chunks`.

```rust
/// Drains `chunks_needing_remesh` and spawns async mesh tasks from existing
/// octree data. Does NOT regenerate chunks — only remeshes the data already
/// in the octree.
pub fn spawn_remesh_tasks(
    mut map_query: Query<(&mut VoxelMapInstance, &mut PendingRemeshes)>,
) {
    let pool = AsyncComputeTaskPool::get();
    for (mut instance, mut pending) in &mut map_query {
        let positions: Vec<IVec3> = instance.chunks_needing_remesh.drain().collect();

        for chunk_pos in positions {
            let Some(chunk_data) = instance.get_chunk_data(chunk_pos) else {
                continue; // chunk was unloaded between edit and remesh
            };
            // Expand to flat array for the mesher
            let voxels = chunk_data.voxels.to_voxels();
            let task = pool.spawn(async move {
                mesh_chunk_greedy(&voxels)
            });
            pending.tasks.push(RemeshTask { chunk_pos, task });
        }
    }
}

/// Polls completed remesh tasks and swaps the mesh on existing chunk entities.
/// If the chunk entity was despawned (unloaded during remesh), the result is
/// discarded. If the chunk had no entity (was all-air before) and now has a mesh,
/// a new entity is spawned.
pub fn poll_remesh_tasks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    default_material: Res<DefaultVoxelMaterial>,
    mut map_query: Query<(Entity, &VoxelMapInstance, &mut PendingRemeshes)>,
    chunk_query: Query<(Entity, &VoxelChunk, &ChildOf)>,
) {
    for (map_entity, instance, mut pending) in &mut map_query {
        let mut i = 0;
        while i < pending.tasks.len() {
            // Use the same task polling pattern as poll_chunk_tasks
            let Some(mesh_opt) = check_ready(&mut pending.tasks[i].task) else {
                i += 1;
                continue;
            };
            let remesh = pending.tasks.swap_remove(i);

            // Skip if chunk was unloaded during remesh
            if !instance.loaded_chunks.contains(&remesh.chunk_pos) {
                continue;
            }

            // Find existing chunk entity for this position under this map
            let existing = chunk_query.iter().find(|(_, vc, parent)| {
                vc.position == remesh.chunk_pos && parent.get() == map_entity
            });

            match (mesh_opt, existing) {
                (Some(mesh), Some((entity, _, _))) => {
                    // Replace mesh on existing entity — the visual swap
                    let handle = meshes.add(mesh);
                    commands.entity(entity).insert(Mesh3d(handle));
                }
                (Some(mesh), None) => {
                    // Chunk was all-air before, now has geometry — spawn new entity
                    let handle = meshes.add(mesh);
                    let offset = chunk_world_offset(remesh.chunk_pos);
                    let material = select_material(
                        instance.debug_colors, remesh.chunk_pos,
                        &mut materials, &default_material,
                    );
                    commands.entity(map_entity).with_child((
                        VoxelChunk { position: remesh.chunk_pos, lod_level: 0 },
                        Mesh3d(handle),
                        MeshMaterial3d(material),
                        Transform::from_translation(offset),
                    ));
                }
                (None, Some((entity, _, _))) => {
                    // Chunk is now all-air/solid — despawn mesh entity
                    commands.entity(entity).despawn();
                }
                (None, None) => {
                    // No mesh needed, no entity exists — nothing to do
                }
            }
        }
    }
}
```

#### 9. Remove overrides from generation pipeline
**File**: `crates/voxel_map_engine/src/generation.rs`

Remove `collect_chunk_overrides` and `apply_overrides` functions entirely. Update `spawn_chunk_gen_task` to no longer accept `modified_voxels`:

```rust
pub fn spawn_chunk_gen_task(
    pending: &mut PendingChunks,
    position: IVec3,
    generator: &VoxelGenerator,
    // modified_voxels parameter removed
) {
    let generator = Arc::clone(generator);
    let pool = AsyncComputeTaskPool::get();
    let task = pool.spawn(async move { generate_chunk(position, &generator) });
    pending.tasks.push(task);
    pending.pending_positions.insert(position);
}
```

Update `generate_chunk` — no overrides:

```rust
fn generate_chunk(position: IVec3, generator: &VoxelGenerator) -> ChunkGenResult {
    let voxels = generator(position);
    let mesh = mesh_chunk_greedy(&voxels);
    ChunkGenResult { position, mesh, voxels }
}
```

Update call site in `spawn_missing_chunks` to remove `&instance.modified_voxels` argument.

### Unit Tests

**File**: `crates/voxel_map_engine/src/palette.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WorldVoxel;

    #[test]
    fn single_value_air() {
        let voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        let p = PalettedChunk::from_voxels(&voxels);
        assert!(p.is_uniform());
        assert_eq!(p.palette_size(), 1);
        assert_eq!(p.get(0), WorldVoxel::Air);
        assert_eq!(p.get(PADDED_VOLUME - 1), WorldVoxel::Air);
    }

    #[test]
    fn single_value_solid() {
        let voxels = vec![WorldVoxel::Solid(42); PADDED_VOLUME];
        let p = PalettedChunk::from_voxels(&voxels);
        assert!(p.is_uniform());
        assert_eq!(p.get(0), WorldVoxel::Solid(42));
    }

    #[test]
    fn two_voxel_types_roundtrip() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        voxels[100] = WorldVoxel::Solid(1);
        voxels[200] = WorldVoxel::Solid(1);
        let p = PalettedChunk::from_voxels(&voxels);
        assert!(!p.is_uniform());
        assert_eq!(p.palette_size(), 2);
        let restored = p.to_voxels();
        assert_eq!(voxels, restored);
    }

    #[test]
    fn many_voxel_types_roundtrip() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        for i in 0..200 {
            voxels[i] = WorldVoxel::Solid(i as u8);
        }
        let p = PalettedChunk::from_voxels(&voxels);
        let restored = p.to_voxels();
        assert_eq!(voxels, restored);
    }

    #[test]
    fn get_single_voxel_indexed_access() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        voxels[500] = WorldVoxel::Solid(7);
        let p = PalettedChunk::from_voxels(&voxels);
        assert_eq!(p.get(0), WorldVoxel::Air);
        assert_eq!(p.get(500), WorldVoxel::Solid(7));
    }

    #[test]
    fn set_within_existing_palette() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        voxels[0] = WorldVoxel::Solid(1);
        let mut p = PalettedChunk::from_voxels(&voxels);
        p.set(10, WorldVoxel::Solid(1));
        assert_eq!(p.get(10), WorldVoxel::Solid(1));
        assert_eq!(p.palette_size(), 2); // no palette growth
    }

    #[test]
    fn set_expands_palette() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        voxels[0] = WorldVoxel::Solid(1);
        let mut p = PalettedChunk::from_voxels(&voxels);
        p.set(10, WorldVoxel::Solid(2));
        assert_eq!(p.get(10), WorldVoxel::Solid(2));
        assert_eq!(p.palette_size(), 3);
    }

    #[test]
    fn set_transitions_from_single_value() {
        let mut p = PalettedChunk::SingleValue(WorldVoxel::Air);
        p.set(0, WorldVoxel::Solid(5));
        assert!(!p.is_uniform());
        assert_eq!(p.get(0), WorldVoxel::Solid(5));
        assert_eq!(p.get(1), WorldVoxel::Air);
    }

    #[test]
    fn set_noop_on_single_value() {
        let mut p = PalettedChunk::SingleValue(WorldVoxel::Air);
        p.set(0, WorldVoxel::Air);
        assert!(p.is_uniform()); // still single-valued
    }

    #[test]
    fn memory_usage_single_value_minimal() {
        let p = PalettedChunk::SingleValue(WorldVoxel::Air);
        assert!(p.memory_usage() < 16);
    }

    #[test]
    fn memory_usage_indirect_less_than_flat() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        voxels[0] = WorldVoxel::Solid(1);
        let p = PalettedChunk::from_voxels(&voxels);
        let flat_size = PADDED_VOLUME * std::mem::size_of::<WorldVoxel>();
        assert!(p.memory_usage() < flat_size / 4); // should be much smaller
    }

    #[test]
    fn bits_needed_values() {
        assert_eq!(bits_needed(1), 0);
        assert_eq!(bits_needed(2), 1);
        assert_eq!(bits_needed(3), 2);
        assert_eq!(bits_needed(4), 2);
        assert_eq!(bits_needed(5), 3);
        assert_eq!(bits_needed(256), 8);
        assert_eq!(bits_needed(257), 9);
    }

    #[test]
    fn serde_roundtrip() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        voxels[100] = WorldVoxel::Solid(3);
        let p = PalettedChunk::from_voxels(&voxels);
        let bytes = bincode::serialize(&p).unwrap();
        let restored: PalettedChunk = bincode::deserialize(&bytes).unwrap();
        assert_eq!(p.to_voxels(), restored.to_voxels());
    }

    #[test]
    fn serde_single_value_roundtrip() {
        let p = PalettedChunk::SingleValue(WorldVoxel::Solid(10));
        let bytes = bincode::serialize(&p).unwrap();
        let restored: PalettedChunk = bincode::deserialize(&bytes).unwrap();
        assert!(restored.is_uniform());
        assert_eq!(restored.get(0), WorldVoxel::Solid(10));
    }
}
```

**File**: `crates/voxel_map_engine/src/instance.rs` — extend test module for direct mutation

```rust
#[test]
fn set_voxel_mutates_octree_in_place() {
    let mut instance = VoxelMapInstance::new(5);
    let chunk_pos = IVec3::ZERO;
    // Insert a chunk with all air
    let voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
    let chunk_data = ChunkData::from_voxels(&voxels);
    instance.insert_chunk_data(chunk_pos, chunk_data);
    instance.loaded_chunks.insert(chunk_pos);

    // Edit a voxel
    let world_pos = IVec3::new(5, 5, 5);
    instance.set_voxel(world_pos, WorldVoxel::Solid(42));

    // Verify the edit is in the octree
    let data = instance.get_chunk_data(chunk_pos).unwrap();
    let local = world_pos - chunk_pos * CHUNK_SIZE as i32;
    let padded = [(local.x + 1) as u32, (local.y + 1) as u32, (local.z + 1) as u32];
    let index = PaddedChunkShape::linearize(padded) as usize;
    assert_eq!(data.voxels.get(index), WorldVoxel::Solid(42));

    // Verify dirty + remesh tracking
    assert!(instance.dirty_chunks.contains(&chunk_pos));
    assert!(instance.chunks_needing_remesh.contains(&chunk_pos));
}

#[test]
fn set_voxel_on_boundary_updates_neighbor_padding() {
    let mut instance = VoxelMapInstance::new(5);
    let chunk_a = IVec3::ZERO;
    let chunk_b = IVec3::new(1, 0, 0);

    // Insert both chunks with all air
    for &pos in &[chunk_a, chunk_b] {
        let voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        instance.insert_chunk_data(pos, ChunkData::from_voxels(&voxels));
        instance.loaded_chunks.insert(pos);
    }

    // Edit voxel at x=15 (boundary of chunk_a, padding of chunk_b)
    let world_pos = IVec3::new(CHUNK_SIZE as i32 - 1, 5, 5);
    instance.set_voxel(world_pos, WorldVoxel::Solid(7));

    // Verify chunk_b's padding was updated
    let neighbor_data = instance.get_chunk_data(chunk_b).unwrap();
    // The voxel at chunk_a's x=15 corresponds to chunk_b's padding at x=-1 → padded index x=0
    let padded = [0u32, (5 + 1) as u32, (5 + 1) as u32];
    let idx = PaddedChunkShape::linearize(padded) as usize;
    assert_eq!(neighbor_data.voxels.get(idx), WorldVoxel::Solid(7));

    // Both chunks marked for remesh
    assert!(instance.chunks_needing_remesh.contains(&chunk_a));
    assert!(instance.chunks_needing_remesh.contains(&chunk_b));
}

#[test]
fn set_voxel_on_unloaded_chunk_is_dropped() {
    let mut instance = VoxelMapInstance::new(5);
    // Don't insert any chunks — the edit target is not loaded
    instance.set_voxel(IVec3::new(5, 5, 5), WorldVoxel::Solid(1));
    assert!(instance.dirty_chunks.is_empty());
    assert!(instance.chunks_needing_remesh.is_empty());
}

#[test]
fn multiple_edits_same_chunk_single_remesh() {
    let mut instance = VoxelMapInstance::new(5);
    let chunk_pos = IVec3::ZERO;
    let voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
    instance.insert_chunk_data(chunk_pos, ChunkData::from_voxels(&voxels));
    instance.loaded_chunks.insert(chunk_pos);

    // Multiple edits to the same chunk
    instance.set_voxel(IVec3::new(1, 1, 1), WorldVoxel::Solid(1));
    instance.set_voxel(IVec3::new(2, 2, 2), WorldVoxel::Solid(2));
    instance.set_voxel(IVec3::new(3, 3, 3), WorldVoxel::Solid(3));

    // chunks_needing_remesh contains just the one chunk (HashSet deduplicates)
    assert_eq!(instance.chunks_needing_remesh.len(), 1);
    assert!(instance.chunks_needing_remesh.contains(&chunk_pos));
}
```

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client -c 1`

#### Manual Verification:
- [ ] Voxel edits work in-game (place/break blocks) — instant visual feedback
- [ ] Old mesh stays visible until remesh completes (no flicker)
- [ ] No visual regressions — chunks render identically to before
- [ ] Boundary edits (voxels at chunk edges) correctly update neighboring chunk meshes
- [ ] Uniform chunks (air above terrain) consume near-zero memory in the octree

---

## Phase 3: Directory Structure, Map Metadata, and Per-Chunk Terrain Persistence

### Overview
Create the `worlds/<map>/` directory structure. Save map metadata to `map.meta.bin`. Save full chunk terrain data to per-chunk files with zstd compression. Replace the old single-file save system.

Note: Per-chunk dirty tracking (`dirty_chunks`, `chunks_needing_remesh`) was already added to `VoxelMapInstance` in Phase 2. `VoxelMapInstance.set_voxel()` marks chunks dirty on edit. This phase adds the persistence layer that saves dirty chunks to disk (on debounce timer, on eviction, on shutdown).

### Changes Required:

#### 1. Add dependencies
**File**: `crates/server/Cargo.toml`

```toml
zstd = "0.13"
```

**File**: `crates/voxel_map_engine/Cargo.toml`

```toml
bincode = "1.3"
zstd = "0.13"
```

#### 2. Add serde derives to ChunkData and FillType
**File**: `crates/voxel_map_engine/src/types.rs`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FillType { ... }

#[derive(Clone, Serialize, Deserialize)]
pub struct ChunkData { ... }
```

#### 3. Dirty tracking — already done in Phase 2

`dirty_chunks` and `chunks_needing_remesh` were added to `VoxelMapInstance` in Phase 2 (step 5). `VoxelMapInstance::set_voxel()` marks chunks dirty on every edit. No `flush_write_buffer` exists — it was removed in Phase 2. No changes needed here.

The persistence systems added below read from `instance.dirty_chunks` to determine which chunks need saving.

#### 4. Add `save_dir` to VoxelMapConfig
**File**: `crates/voxel_map_engine/src/config.rs`

```rust
use std::path::PathBuf;

#[derive(Component)]
pub struct VoxelMapConfig {
    pub seed: u64,
    pub generation_version: u32,    // NEW — moved from MapWorld resource
    pub spawning_distance: u32,
    pub bounds: Option<IVec3>,
    pub tree_height: u32,
    pub generator: VoxelGenerator,
    pub save_dir: Option<PathBuf>,  // NEW — None means no persistence
}
```

Update `VoxelMapConfig::new` to accept and store `save_dir` and `generation_version`. The engine crate doesn't use them directly — they're read by the server crate's persistence systems and by `spawn_chunk_gen_task` (see step 9).

Update all call sites in `VoxelMapInstance::overworld()`, `homebase()`, `arena()` to pass `save_dir: None` and `generation_version: 0` by default. The server crate overrides these when spawning maps.

#### 5. Create persistence module in voxel_map_engine
**File**: `crates/voxel_map_engine/src/persistence.rs` (new)

Low-level chunk I/O with compression. The server orchestrates when to call these functions.

```rust
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::fs;
use bevy::prelude::*;
use serde::{Serialize, Deserialize};
use crate::types::ChunkData;

const CHUNK_SAVE_VERSION: u32 = 1;
const ZSTD_COMPRESSION_LEVEL: i32 = 3;

/// Versioned envelope wrapping chunk data for on-disk persistence.
#[derive(Serialize, Deserialize)]
struct ChunkFileEnvelope {
    version: u32,
    data: ChunkData,
}

/// Build the file path for a chunk at the given position within a map directory.
pub fn chunk_file_path(map_dir: &Path, chunk_pos: IVec3) -> PathBuf {
    map_dir.join("terrain").join(format!(
        "chunk_{}_{}_{}.bin",
        chunk_pos.x, chunk_pos.y, chunk_pos.z
    ))
}

/// Save a single chunk's data to disk (bincode + zstd). Atomic via tmp+rename.
pub fn save_chunk(map_dir: &Path, chunk_pos: IVec3, data: &ChunkData) -> Result<(), String> {
    let path = chunk_file_path(map_dir, chunk_pos);
    fs::create_dir_all(path.parent().expect("chunk path has parent"))
        .map_err(|e| format!("mkdir terrain: {e}"))?;

    let envelope = ChunkFileEnvelope { version: CHUNK_SAVE_VERSION, data: data.clone() };
    let bytes = bincode::serialize(&envelope).map_err(|e| format!("serialize chunk: {e}"))?;

    let tmp_path = path.with_extension("bin.tmp");
    let file = fs::File::create(&tmp_path).map_err(|e| format!("create tmp: {e}"))?;
    let mut encoder = zstd::Encoder::new(file, ZSTD_COMPRESSION_LEVEL)
        .map_err(|e| format!("zstd encoder: {e}"))?;
    encoder.write_all(&bytes).map_err(|e| format!("write chunk: {e}"))?;
    encoder.finish().map_err(|e| format!("zstd finish: {e}"))?;

    fs::rename(&tmp_path, &path).map_err(|e| format!("atomic rename: {e}"))?;
    Ok(())
}

/// Load a single chunk's data from disk. Returns None if no saved file exists.
pub fn load_chunk(map_dir: &Path, chunk_pos: IVec3) -> Result<Option<ChunkData>, String> {
    let path = chunk_file_path(map_dir, chunk_pos);
    if !path.exists() {
        return Ok(None);
    }

    let file = fs::File::open(&path).map_err(|e| format!("open chunk: {e}"))?;
    let mut decoder = zstd::Decoder::new(file).map_err(|e| format!("zstd decoder: {e}"))?;
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).map_err(|e| format!("read chunk: {e}"))?;

    let envelope: ChunkFileEnvelope = bincode::deserialize(&bytes)
        .map_err(|e| format!("deserialize chunk: {e}"))?;

    if envelope.version != CHUNK_SAVE_VERSION {
        return Err(format!(
            "chunk version mismatch: expected {CHUNK_SAVE_VERSION}, got {}",
            envelope.version
        ));
    }

    Ok(Some(envelope.data))
}

/// Delete a chunk file if it exists.
pub fn delete_chunk(map_dir: &Path, chunk_pos: IVec3) -> Result<(), String> {
    let path = chunk_file_path(map_dir, chunk_pos);
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("delete chunk: {e}"))?;
    }
    Ok(())
}

/// List all chunk positions that have saved files in the terrain/ subdirectory.
pub fn list_saved_chunks(map_dir: &Path) -> Result<Vec<IVec3>, String> {
    let terrain_dir = map_dir.join("terrain");
    if !terrain_dir.exists() {
        return Ok(Vec::new());
    }

    let mut positions = Vec::new();
    for entry in fs::read_dir(&terrain_dir).map_err(|e| format!("read_dir: {e}"))? {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(pos) = parse_chunk_filename(&name) {
            positions.push(pos);
        }
    }
    Ok(positions)
}

/// Parse "chunk_X_Y_Z.bin" filename into IVec3. Handles negative coordinates.
pub fn parse_chunk_filename(name: &str) -> Option<IVec3> {
    let name = name.strip_prefix("chunk_")?.strip_suffix(".bin")?;
    // Split from the right to handle negative numbers (e.g. "chunk_-1_0_2.bin")
    // Use splitn(3, '_') but negative signs make this tricky.
    // Instead, parse by finding the last two '_' separators.
    let last_sep = name.rfind('_')?;
    let z: i32 = name[last_sep + 1..].parse().ok()?;
    let rest = &name[..last_sep];
    let mid_sep = rest.rfind('_')?;
    let y: i32 = rest[mid_sep + 1..].parse().ok()?;
    let x: i32 = rest[..mid_sep].parse().ok()?;
    Some(IVec3::new(x, y, z))
}
```

Register module in `crates/voxel_map_engine/src/lib.rs`:

```rust
pub mod persistence;
```

And add to the prelude re-exports.

#### 6. Create MapMeta type and persistence
**File**: `crates/server/src/persistence.rs` (new module)

```rust
use std::fs;
use std::path::{Path, PathBuf};
use bevy::prelude::*;
use protocol::map::MapInstanceId;
use serde::{Serialize, Deserialize};

const META_VERSION: u32 = 1;

/// Metadata for a single map instance, saved to map.meta.bin.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MapMeta {
    pub version: u32,
    pub seed: u64,
    pub generation_version: u32,
    pub spawn_points: Vec<Vec3>,
}

/// Resource holding the base save directory path.
#[derive(Resource)]
pub struct WorldSavePath(pub PathBuf);

impl Default for WorldSavePath {
    fn default() -> Self {
        Self(PathBuf::from("worlds"))
    }
}

/// Resolve the save directory for a MapInstanceId within the base save path.
pub fn map_save_dir(base: &Path, map_id: &MapInstanceId) -> PathBuf {
    match map_id {
        MapInstanceId::Overworld => base.join("overworld"),
        MapInstanceId::Homebase { owner } => base.join(format!("homebase-{owner}")),
    }
}

/// Save map metadata to map.meta.bin. Atomic via tmp+rename.
pub fn save_map_meta(map_dir: &Path, meta: &MapMeta) -> Result<(), String> {
    fs::create_dir_all(map_dir).map_err(|e| format!("mkdir map_dir: {e}"))?;
    let path = map_dir.join("map.meta.bin");
    let bytes = bincode::serialize(meta).map_err(|e| format!("serialize meta: {e}"))?;
    let tmp_path = path.with_extension("bin.tmp");
    fs::write(&tmp_path, &bytes).map_err(|e| format!("write meta tmp: {e}"))?;
    fs::rename(&tmp_path, &path).map_err(|e| format!("rename meta: {e}"))?;
    Ok(())
}

/// Load map metadata from map.meta.bin. Returns None if file doesn't exist.
pub fn load_map_meta(map_dir: &Path) -> Result<Option<MapMeta>, String> {
    let path = map_dir.join("map.meta.bin");
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|e| format!("read meta: {e}"))?;
    let meta: MapMeta = bincode::deserialize(&bytes).map_err(|e| format!("deserialize meta: {e}"))?;
    if meta.version != META_VERSION {
        return Err(format!("meta version mismatch: expected {META_VERSION}, got {}", meta.version));
    }
    Ok(Some(meta))
}
```

Register in `crates/server/src/lib.rs`:

```rust
pub mod persistence;
```

#### 7. Replace old save system (persistence only — networking unchanged)
**File**: `crates/server/src/map.rs`

**Remove** these save-system types and functions:
- `VoxelWorldSave` struct (line 207-213)
- `VoxelDirtyState` resource (line 187-202)
- `VoxelSavePath` resource (line 218-225)
- `save_voxel_world_to_disk_at` function (line 227-262)
- `load_voxel_world_from_disk_at` function (line 264-344)
- `save_voxel_world_debounced` system (line 85-113)
- `save_voxel_world_on_shutdown` system (line 115-135)
- `load_voxel_world` system (line 63-83)

**Keep until Phase 5** (still needed for client sync during active sessions):
- `VoxelModifications` resource — populated at runtime from edits (NOT loaded from disk), used by `send_initial_voxel_state` to sync late-joining clients with edits made during the current session. After a server restart, this is empty — clients won't see pre-restart edits until Phase 5 adds chunk streaming.
- `send_initial_voxel_state` observer — still sends runtime modifications to connecting clients
- `MapWorld` resource — client uses `seed` for local generation until Phase 5 replaces it
- `VoxelStateSync` message type and channel registration

**Add** constants to `crates/server/src/map.rs`:

```rust
/// Default seed for a new overworld (replaces MapWorld.seed default).
const DEFAULT_OVERWORLD_SEED: u64 = 999;

/// Current terrain generation algorithm version. Bump when generation
/// code changes to invalidate saved chunks.
const GENERATION_VERSION: u32 = 0;
```

**Replace with** these new resources and systems:

```rust
use crate::persistence::{WorldSavePath, MapMeta, map_save_dir, save_map_meta, load_map_meta};
use voxel_map_engine::persistence as chunk_persist;

/// Tracks debounced save timing. Replaces VoxelDirtyState.
#[derive(Resource)]
pub struct WorldDirtyState {
    pub is_dirty: bool,
    pub last_edit_time: f64,
    pub first_dirty_time: Option<f64>,
}

const SAVE_DEBOUNCE_SECONDS: f64 = 1.0;
const MAX_DIRTY_SECONDS: f64 = 5.0;
```

**New system: `mark_world_dirty`** — called from `handle_voxel_edit_requests` when a voxel is edited. Updates `WorldDirtyState` timestamps (same logic as the old `VoxelDirtyState` update in `handle_voxel_edit_requests`).

**New system: `save_dirty_chunks_debounced`** — replaces `save_voxel_world_debounced`. Runs in `Update`:

```rust
/// Debounced save of all dirty chunks across all map instances.
fn save_dirty_chunks_debounced(
    time: Res<Time>,
    mut dirty_state: ResMut<WorldDirtyState>,
    save_path: Res<WorldSavePath>,
    mut map_query: Query<(&mut VoxelMapInstance, &VoxelMapConfig, &MapInstanceId)>,
) {
    if !dirty_state.is_dirty {
        return;
    }

    let now = time.elapsed_secs_f64();
    let time_since_edit = now - dirty_state.last_edit_time;
    let time_since_first_dirty = dirty_state.first_dirty_time
        .map(|t| now - t)
        .unwrap_or(0.0);

    let should_save = time_since_edit >= SAVE_DEBOUNCE_SECONDS
        || time_since_first_dirty >= MAX_DIRTY_SECONDS;

    if !should_save {
        return;
    }

    for (mut instance, config, map_id) in &mut map_query {
        let Some(map_dir) = config.save_dir.as_deref() else {
            panic!("save_dir not set on map instance {map_id:?}");
        };

        save_dirty_chunks_for_instance(&mut instance, map_dir);

        // Also save metadata
        let meta = MapMeta {
            version: 1,
            seed: config.seed,
            generation_version: config.generation_version,
            spawn_points: vec![], // Phase 3 will populate this from RespawnPoint entities
        };
        if let Err(e) = save_map_meta(map_dir, &meta) {
            error!("Failed to save map meta for {map_id:?}: {e}");
        }
    }

    dirty_state.is_dirty = false;
    dirty_state.first_dirty_time = None;
}

/// Save all dirty chunks for a single map instance.
fn save_dirty_chunks_for_instance(instance: &mut VoxelMapInstance, map_dir: &Path) {
    let dirty: Vec<IVec3> = instance.dirty_chunks.drain().collect();
    for chunk_pos in dirty {
        if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
            if let Err(e) = chunk_persist::save_chunk(map_dir, chunk_pos, chunk_data) {
                error!("Failed to save chunk at {chunk_pos}: {e}");
                // Re-insert into dirty set so we retry next cycle
                instance.dirty_chunks.insert(chunk_pos);
            }
        } else {
            // Chunk was evicted from octree — it was already saved during eviction
            debug_assert!(
                !instance.loaded_chunks.contains(&chunk_pos),
                "dirty chunk {chunk_pos} in loaded_chunks but missing from octree"
            );
        }
    }
}
```

**New system: `save_world_on_shutdown`** — replaces `save_voxel_world_on_shutdown`. Runs in `Last`:

```rust
/// Save all dirty chunks and metadata on graceful shutdown.
fn save_world_on_shutdown(
    mut exit_events: EventReader<AppExit>,
    mut map_query: Query<(&mut VoxelMapInstance, &VoxelMapConfig, &MapInstanceId)>,
    dirty_state: Res<WorldDirtyState>,
) {
    if exit_events.read().next().is_none() {
        return;
    }
    if !dirty_state.is_dirty {
        return;
    }

    for (mut instance, config, map_id) in &mut map_query {
        let Some(map_dir) = config.save_dir.as_deref() else { continue };
        save_dirty_chunks_for_instance(&mut instance, map_dir);

        let meta = MapMeta {
            version: 1,
            seed: config.seed,
            generation_version: config.generation_version,
            spawn_points: vec![], // Phase 3 populates
        };
        if let Err(e) = save_map_meta(map_dir, &meta) {
            error!("Failed to save meta on shutdown for {map_id:?}: {e}");
        }
    }
    info!("World saved on shutdown");
}
```

**New system: `save_evicted_dirty_chunks`** — integrated into the chunk eviction path. In `remove_out_of_range_chunks` (Phase 1 already handles eviction from octree), add a save step for dirty chunks:

```rust
fn remove_out_of_range_chunks(instance: &mut VoxelMapInstance, desired: &HashSet<IVec3>, save_dir: Option<&Path>) {
    let removed: Vec<IVec3> = instance.loaded_chunks.iter()
        .filter(|pos| !desired.contains(pos))
        .copied()
        .collect();
    for pos in removed {
        // Save dirty chunk to disk before evicting from octree
        if instance.dirty_chunks.contains(&pos) {
            if let Some(dir) = save_dir {
                if let Some(chunk_data) = instance.get_chunk_data(pos) {
                    if let Err(e) = chunk_persist::save_chunk(dir, pos, chunk_data) {
                        error!("Failed to save evicted dirty chunk at {pos}: {e}");
                    }
                }
            }
            instance.dirty_chunks.remove(&pos);
        }
        instance.loaded_chunks.remove(&pos);
        instance.remove_chunk_data(pos);
    }
}
```

This requires `update_chunks` (which calls `remove_out_of_range_chunks`) to pass `config.save_dir.as_deref()` through. Update the `update_chunks` system signature to include `VoxelMapConfig` (it already queries it) and pass `config.save_dir.as_deref()` to `remove_out_of_range_chunks`.

#### 8. Load saved chunks during generation
**File**: `crates/voxel_map_engine/src/generation.rs`

Modify `spawn_chunk_gen_task` to accept an optional save directory. The async task tries loading from disk first, falling back to generation. Note: `modified_voxels` parameter was removed in Phase 2 — no overrides needed since edits are baked into the octree and saved to disk directly.

```rust
pub fn spawn_chunk_gen_task(
    pending: &mut PendingChunks,
    position: IVec3,
    generator: &VoxelGenerator,
    save_dir: Option<PathBuf>,
) {
    let generator = Arc::clone(generator);
    let pool = AsyncComputeTaskPool::get();

    let task = pool.spawn(async move {
        // Try loading saved chunk from disk
        if let Some(ref dir) = save_dir {
            match crate::persistence::load_chunk(dir, position) {
                Ok(Some(chunk_data)) => {
                    let voxels = chunk_data.voxels.to_voxels();
                    let mesh = mesh_chunk_greedy(&voxels);
                    return ChunkGenResult {
                        position, mesh, voxels, from_disk: true,
                    };
                }
                Ok(None) => {} // No saved file, fall through to generation
                Err(e) => {
                    bevy::log::warn!("Failed to load chunk at {position}: {e}, regenerating");
                }
            }
        }

        // Generate from scratch (no overrides — edits are baked into chunk data)
        generate_chunk(position, &generator)
    });

    pending.tasks.push(task);
    pending.pending_positions.insert(position);
}
```

Note: `chunk_data.voxels.to_voxels()` is needed because after Phase 2, `ChunkData.voxels` is a `PalettedChunk`, and the mesher requires a flat `Vec<WorldVoxel>`.

Add `from_disk: bool` field to `ChunkGenResult`:

```rust
pub struct ChunkGenResult {
    pub position: IVec3,
    pub mesh: Option<Mesh>,
    pub voxels: Vec<WorldVoxel>,
    pub from_disk: bool,  // true if loaded from saved file, false if generated
}
```

In `generate_chunk`, set `from_disk: false`.

Dirty tracking for completed chunks:
- `from_disk: true` → clean (already saved)
- `from_disk: false` → clean (can be regenerated from the generator)
- Dirty is only set by `VoxelMapInstance::set_voxel()` when player edits arrive, never in `handle_completed_chunk`

**Update call site** in `lifecycle.rs` `spawn_missing_chunks`:

```rust
fn spawn_missing_chunks(
    instance: &VoxelMapInstance,
    pending: &mut PendingChunks,
    config: &VoxelMapConfig,
    desired: &HashSet<IVec3>,
) {
    let mut spawned = 0;
    for &pos in desired {
        if spawned >= MAX_TASKS_PER_FRAME { break; }
        if instance.loaded_chunks.contains(&pos) { continue; }
        if is_already_pending(pending, pos) { continue; }

        spawn_chunk_gen_task(
            pending, pos, &config.generator,
            config.save_dir.clone(),  // load from disk if available
        );
        spawned += 1;
    }
}
```

#### 9. Update handle_voxel_edit_requests
**File**: `crates/server/src/map.rs`

Replace the old `VoxelModifications.modifications.push(...)` and `VoxelDirtyState` update with `WorldDirtyState` update:

```rust
fn handle_voxel_edit_requests(
    // ... existing params ...
    mut dirty_state: ResMut<WorldDirtyState>,
    time: Res<Time>,
) {
    for (entity, mut receiver) in &mut edit_receivers {
        for edit in receiver.drain::<VoxelEditRequest>() {
            // Set voxel in VoxelWorld — this calls VoxelMapInstance::set_voxel()
            // which mutates the octree in-place and marks the chunk dirty
            voxel_world.set_voxel(overworld_entity, edit.position, edit.voxel.into());

            // Update dirty state for debounced save
            let now = time.elapsed_secs_f64();
            if !dirty_state.is_dirty {
                dirty_state.first_dirty_time = Some(now);
            }
            dirty_state.is_dirty = true;
            dirty_state.last_edit_time = now;

            // Append to runtime VoxelModifications for client sync (kept until Phase 5)
            modifications.modifications.push((edit.position, edit.voxel));

            // Broadcast to all clients (unchanged)
            // ...
        }
    }
}
```

Note: `VoxelModifications` is kept as a runtime-only append log for client sync (connecting clients receive it via `send_initial_voxel_state`). It is NOT persisted to disk — edits are baked into chunk data in the octree and saved per-chunk. After server restart, `VoxelModifications` starts empty. Phase 5 removes it entirely (replaced by chunk streaming).

#### 10. Networking — unchanged in this phase

Networking changes (`VoxelStateSync` removal, client-side generation removal, room-scoped broadcasts, client prediction, batched updates) are handled in Phases 5-7. Phase 3 focuses purely on server-side persistence.

#### 11. Replace hardcoded spawn points
**File**: `crates/server/src/gameplay.rs`

Currently `spawn_respawn_points` (line 110-112) spawns a single respawn point at `Vec3(0.0, 5.0, 0.0)` with just `(RespawnPoint, Position(...))`. Update to:

```rust
fn spawn_respawn_points(
    mut commands: Commands,
    save_path: Res<WorldSavePath>,
) {
    let map_dir = map_save_dir(&save_path.0, &MapInstanceId::Overworld);
    let spawn_points = match load_map_meta(&map_dir) {
        Ok(Some(meta)) if !meta.spawn_points.is_empty() => meta.spawn_points,
        _ => vec![Vec3::new(0.0, 5.0, 0.0)], // default for first run
    };
    for pos in spawn_points {
        commands.spawn((
            RespawnPoint,
            Position(pos),
            MapInstanceId::Overworld,
        ));
    }
}
```

Also update the `nearest_respawn_pos` fallback (line 150) and the map transition spawn position (map.rs line 515) to use a constant:

```rust
pub const DEFAULT_SPAWN_POS: Vec3 = Vec3::new(0.0, 5.0, 0.0);
```

Update `handle_connected` in `crates/server/src/gameplay.rs` to use respawn points instead of the hardcoded circular spread:

```rust
fn handle_connected(
    // ... existing params ...
    respawn_query: Query<(&Position, &MapInstanceId), With<RespawnPoint>>,
) {
    // ... existing connection setup ...

    // Use a respawn point for the player's starting position
    let spawn_pos = respawn_query.iter()
        .filter(|(_, mid)| **mid == MapInstanceId::Overworld)
        .map(|(pos, _)| pos.0)
        .next()
        .unwrap_or(DEFAULT_SPAWN_POS);

    // Spawn character at the respawn point (elevated for safe landing)
    let start_pos = Vec3::new(spawn_pos.x, spawn_pos.y + 25.0, spawn_pos.z);
    // ... spawn character with Position(start_pos) ...
}
```

#### 12. Update ServerMapPlugin registration
**File**: `crates/server/src/map.rs`

In `ServerMapPlugin::build`, replace old system registrations:

```rust
// Remove:
//   .chain((spawn_overworld, load_voxel_world))  // Startup
//   save_voxel_world_debounced                      // Update
//   save_voxel_world_on_shutdown                    // Last
//   send_initial_voxel_state observer (uses VoxelModifications)

// Replace with:
app.init_resource::<WorldSavePath>()
   .insert_resource(WorldDirtyState {
       is_dirty: false,
       last_edit_time: 0.0,
       first_dirty_time: None,
   });

// Startup (chained):
app.add_systems(Startup, spawn_overworld);

// Update:
app.add_systems(Update, (
    handle_voxel_edit_requests,
    save_dirty_chunks_debounced,
    handle_map_switch_requests,
    handle_map_transition_ready,
    attach_chunk_colliders,
));

// Last:
app.add_systems(Last, save_world_on_shutdown);

// Observer — send initial voxel state from runtime VoxelModifications (kept until Phase 5):
app.add_observer(send_initial_voxel_state);
```

#### 13. Set save_dir on overworld spawn
**File**: `crates/server/src/map.rs`

In `spawn_overworld` (line 46-61), after creating the `VoxelMapConfig`, set `save_dir`:

```rust
fn spawn_overworld(
    mut commands: Commands,
    mut map_registry: ResMut<MapRegistry>,
    save_path: Res<WorldSavePath>,
) {
    let map_dir = map_save_dir(&save_path.0, &MapInstanceId::Overworld);

    // Load seed from existing metadata, or use default for first run
    let (seed, generation_version) = match load_map_meta(&map_dir) {
        Ok(Some(meta)) => (meta.seed, meta.generation_version),
        _ => (DEFAULT_OVERWORLD_SEED, GENERATION_VERSION),
    };

    let instance = VoxelMapInstance::new(5);
    let mut config = VoxelMapConfig::new(
        seed, 2, None, 5,
        Arc::new(flat_terrain_voxels),
    );
    config.generation_version = generation_version;
    config.save_dir = Some(map_dir);

    let map = commands.spawn((
        instance, config,
        Overworld,
        MapInstanceId::Overworld,
        Transform::default(),
    )).id();
    commands.insert_resource(OverworldMap(map)); // OverworldMap defined at server/src/map.rs:43-44
    map_registry.insert(MapInstanceId::Overworld, map);
}
```

### Unit Tests

**File**: `crates/voxel_map_engine/src/persistence.rs` — `#[cfg(test)] mod tests`

```rust
#[test]
fn save_load_chunk_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let pos = IVec3::new(1, -2, 3);
    let mut chunk = ChunkData::new_empty();
    chunk.voxels[100] = WorldVoxel::Solid(5);
    chunk.fill_type = FillType::Mixed;

    save_chunk(dir.path(), pos, &chunk).unwrap();
    let loaded = load_chunk(dir.path(), pos).unwrap().expect("chunk should exist");
    assert_eq!(loaded.voxels, chunk.voxels);
    assert_eq!(loaded.fill_type, chunk.fill_type);
}

#[test]
fn load_nonexistent_chunk_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_chunk(dir.path(), IVec3::ZERO).unwrap().is_none());
}

#[test]
fn save_chunk_creates_directories() {
    let dir = tempfile::tempdir().unwrap();
    let map_dir = dir.path().join("deep/nested/map");
    save_chunk(&map_dir, IVec3::ZERO, &ChunkData::new_empty()).unwrap();
    assert!(map_dir.join("terrain").exists());
}

#[test]
fn corrupt_chunk_file_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = chunk_file_path(dir.path(), IVec3::ZERO);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"not valid data").unwrap();
    assert!(load_chunk(dir.path(), IVec3::ZERO).is_err());
}

#[test]
fn parse_chunk_filename_valid() {
    assert_eq!(parse_chunk_filename("chunk_1_2_3.bin"), Some(IVec3::new(1, 2, 3)));
    assert_eq!(parse_chunk_filename("chunk_0_0_0.bin"), Some(IVec3::ZERO));
}

#[test]
fn parse_chunk_filename_negative_coords() {
    assert_eq!(parse_chunk_filename("chunk_-1_0_2.bin"), Some(IVec3::new(-1, 0, 2)));
    assert_eq!(parse_chunk_filename("chunk_-10_-20_-30.bin"), Some(IVec3::new(-10, -20, -30)));
}

#[test]
fn parse_chunk_filename_invalid() {
    assert_eq!(parse_chunk_filename("not_a_chunk.bin"), None);
    assert_eq!(parse_chunk_filename("chunk_1_2.bin"), None);
    assert_eq!(parse_chunk_filename("chunk_a_b_c.bin"), None);
    assert_eq!(parse_chunk_filename("chunk_1_2_3.txt"), None);
}

#[test]
fn list_saved_chunks_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    assert!(list_saved_chunks(dir.path()).unwrap().is_empty());
}

#[test]
fn list_saved_chunks_with_files() {
    let dir = tempfile::tempdir().unwrap();
    let positions = [IVec3::new(0, 0, 0), IVec3::new(1, -1, 2)];
    for &pos in &positions {
        save_chunk(dir.path(), pos, &ChunkData::new_empty()).unwrap();
    }
    let mut found = list_saved_chunks(dir.path()).unwrap();
    found.sort_by_key(|p| (p.x, p.y, p.z));
    let mut expected = positions.to_vec();
    expected.sort_by_key(|p| (p.x, p.y, p.z));
    assert_eq!(found, expected);
}

#[test]
fn delete_chunk_removes_file() {
    let dir = tempfile::tempdir().unwrap();
    save_chunk(dir.path(), IVec3::ZERO, &ChunkData::new_empty()).unwrap();
    assert!(chunk_file_path(dir.path(), IVec3::ZERO).exists());
    delete_chunk(dir.path(), IVec3::ZERO).unwrap();
    assert!(!chunk_file_path(dir.path(), IVec3::ZERO).exists());
}

#[test]
fn delete_nonexistent_chunk_is_ok() {
    let dir = tempfile::tempdir().unwrap();
    delete_chunk(dir.path(), IVec3::ZERO).unwrap(); // should not error
}

#[test]
fn chunk_data_zstd_compression_reduces_size() {
    let dir = tempfile::tempdir().unwrap();
    let chunk = ChunkData::new_empty(); // all Air — highly compressible
    save_chunk(dir.path(), IVec3::ZERO, &chunk).unwrap();

    let path = chunk_file_path(dir.path(), IVec3::ZERO);
    let compressed_size = fs::metadata(&path).unwrap().len();
    let raw_size = bincode::serialize(&ChunkFileEnvelope {
        version: CHUNK_SAVE_VERSION, data: chunk
    }).unwrap().len() as u64;

    assert!(compressed_size < raw_size / 2, "compressed {compressed_size} should be < half of raw {raw_size}");
}
```

**File**: `crates/server/src/persistence.rs` — `#[cfg(test)] mod tests`

```rust
#[test]
fn save_load_map_meta_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let meta = MapMeta {
        version: 1,
        seed: 42,
        generation_version: 3,
        spawn_points: vec![Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)],
    };
    save_map_meta(dir.path(), &meta).unwrap();
    let loaded = load_map_meta(dir.path()).unwrap().expect("meta should exist");
    assert_eq!(loaded.seed, 42);
    assert_eq!(loaded.generation_version, 3);
    assert_eq!(loaded.spawn_points.len(), 2);
}

#[test]
fn load_map_meta_missing_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_map_meta(dir.path()).unwrap().is_none());
}

#[test]
fn map_save_dir_overworld() {
    let base = Path::new("worlds");
    assert_eq!(map_save_dir(base, &MapInstanceId::Overworld), PathBuf::from("worlds/overworld"));
}

#[test]
fn map_save_dir_homebase() {
    let base = Path::new("worlds");
    assert_eq!(
        map_save_dir(base, &MapInstanceId::Homebase { owner: 42 }),
        PathBuf::from("worlds/homebase-42")
    );
}
```

**File**: `crates/server/tests/voxel_persistence.rs` — update existing tests

Replace the 4 existing tests with tests for the new system:

```rust
#[test]
fn dirty_chunks_saved_on_debounce() {
    // 1. Create a VoxelMapInstance with save_dir pointing at a temp directory
    // 2. Insert chunk data into octree
    // 3. Mark chunk as dirty in dirty_chunks
    // 4. Call save_dirty_chunks_for_instance
    // 5. Verify chunk file exists on disk
    // 6. Verify dirty_chunks is now empty
}

#[test]
fn clean_chunks_not_saved() {
    // 1. Create instance with chunk data in octree but NOT in dirty_chunks
    // 2. Call save_dirty_chunks_for_instance
    // 3. Verify no chunk files written to disk
}

#[test]
fn evicted_dirty_chunk_saved_before_removal() {
    // 1. Create instance with a dirty chunk in octree
    // 2. Call remove_out_of_range_chunks with desired set that excludes the chunk
    // 3. Verify chunk file was saved to disk before eviction
    // 4. Verify chunk is no longer in octree or dirty_chunks
}

#[test]
fn initial_voxel_state_from_runtime_modifications() {
    // Verify send_initial_voxel_state reads from runtime VoxelModifications
    // (populated by handle_voxel_edit_requests, not loaded from disk)
}
```

### Integration Test (start)

**File**: `crates/server/tests/world_persistence.rs` (new)

```rust
use std::path::Path;
use tempfile::TempDir;
use bevy::prelude::*;
use voxel_map_engine::prelude::*;
use server::persistence::{WorldSavePath, load_map_meta};
use voxel_map_engine::persistence as chunk_persist;

/// Helper: create a minimal server app with map systems and persistence.
fn create_test_server_app(save_dir: &Path) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(VoxelPlugin);
    // Add server map systems, WorldSavePath, WorldDirtyState, etc.
    app.insert_resource(WorldSavePath(save_dir.to_path_buf()));
    // ... register systems ...
    app
}

#[test]
fn terrain_persists_across_server_restart() {
    let tmp = TempDir::new().unwrap();
    let map_dir = tmp.path().join("overworld");

    // First run: save chunk data and metadata
    {
        // Create and save chunk data directly (bypasses needing a full Bevy app)
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
        voxels[100] = WorldVoxel::Solid(42);
        let chunk_data = ChunkData::from_voxels(&voxels);
        let chunk_pos = IVec3::new(0, 0, 0);

        chunk_persist::save_chunk(&map_dir, chunk_pos, &chunk_data)
            .expect("save chunk");

        let meta = MapMeta {
            version: 1,
            seed: 999,
            generation_version: 0,
            spawn_points: vec![Vec3::new(0.0, 5.0, 0.0)],
        };
        save_map_meta(&map_dir, &meta).expect("save meta");
    }

    // Second run: verify data loads correctly
    {
        let chunk_pos = IVec3::new(0, 0, 0);
        let loaded = chunk_persist::load_chunk(&map_dir, chunk_pos)
            .expect("load chunk")
            .expect("chunk should exist");

        let loaded_voxels = loaded.voxels.to_voxels();
        assert_eq!(loaded_voxels[100], WorldVoxel::Solid(42));
        assert_eq!(loaded_voxels[0], WorldVoxel::Air);

        let meta = load_map_meta(&map_dir)
            .expect("load meta")
            .expect("meta should exist");
        assert_eq!(meta.seed, 999);
        assert_eq!(meta.spawn_points.len(), 1);
    }
}
```

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client -c 1`

#### Manual Verification:
- [ ] Start server, place blocks, stop server → `worlds/overworld/terrain/` contains chunk files
- [ ] Restart server → placed blocks are still there
- [ ] `worlds/overworld/map.meta.bin` exists with correct seed
- [ ] Chunk files are zstd compressed (smaller than raw bincode)
- [ ] Newly connecting client still receives correct terrain (VoxelStateSync still works — unchanged)
- [ ] No `world_save/voxel_world.bin` created (old system removed)

---

## Phase 4: Entity Persistence

### Overview
Add a `MapSaveTarget` marker component. Entities with this marker are serialized to `entities.bin` per map. Respawn points are the first entity type to persist. The save/load integrates with the debounced save system from Phase 3.

### Changes Required:

#### 1. Define persistence types
**File**: `crates/protocol/src/map.rs`

```rust
/// Marker: this entity should be saved with its map.
#[derive(Component, Clone, Debug)]
pub struct MapSaveTarget;

/// Identifies the type of a saved entity for reconstruction.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SavedEntityKind {
    RespawnPoint,
    // Future variants: Doodad, NPC, Portal, etc.
}

/// A single entity serialized for persistence.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SavedEntity {
    pub kind: SavedEntityKind,
    pub position: Vec3,
}
```

These types live in `protocol` (shared crate) so both server and tests can use them. `MapSaveTarget` is server-only at runtime but the type definition is shared.

#### 2. Add entity save/load to persistence module
**File**: `crates/server/src/persistence.rs`

```rust
use protocol::map::{SavedEntity, SavedEntityKind};

const ENTITY_SAVE_VERSION: u32 = 1;

/// Versioned envelope wrapping entity data for on-disk persistence.
#[derive(Serialize, Deserialize)]
struct EntityFileEnvelope {
    version: u32,
    entities: Vec<SavedEntity>,
}

/// Save entities to entities.bin in the map directory. Atomic via tmp+rename.
pub fn save_entities(map_dir: &Path, entities: &[SavedEntity]) -> Result<(), String> {
    fs::create_dir_all(map_dir).map_err(|e| format!("mkdir: {e}"))?;
    let path = map_dir.join("entities.bin");
    let envelope = EntityFileEnvelope {
        version: ENTITY_SAVE_VERSION,
        entities: entities.to_vec(),
    };
    let bytes = bincode::serialize(&envelope).map_err(|e| format!("serialize entities: {e}"))?;
    let tmp_path = path.with_extension("bin.tmp");
    fs::write(&tmp_path, &bytes).map_err(|e| format!("write entities tmp: {e}"))?;
    fs::rename(&tmp_path, &path).map_err(|e| format!("rename entities: {e}"))?;
    Ok(())
}

/// Load entities from entities.bin. Returns empty vec if file doesn't exist.
pub fn load_entities(map_dir: &Path) -> Result<Vec<SavedEntity>, String> {
    let path = map_dir.join("entities.bin");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(&path).map_err(|e| format!("read entities: {e}"))?;
    let envelope: EntityFileEnvelope = bincode::deserialize(&bytes)
        .map_err(|e| format!("deserialize entities: {e}"))?;
    if envelope.version != ENTITY_SAVE_VERSION {
        return Err(format!(
            "entity version mismatch: expected {ENTITY_SAVE_VERSION}, got {}",
            envelope.version
        ));
    }
    Ok(envelope.entities)
}
```

#### 3. Add entity collection and save system
**File**: `crates/server/src/map.rs`

**No duplication**: `collect_and_save_entities` performs a full snapshot — it queries ALL `MapSaveTarget` entities, builds the complete entity list, and overwrites the entire `entities.bin`. Each save is a full replacement, not an append. No UUIDs needed.

```rust
use protocol::map::{MapSaveTarget, SavedEntity, SavedEntityKind};

/// Collect all persistable entities grouped by map and save to disk.
fn collect_and_save_entities(
    save_path: &WorldSavePath,
    entity_query: &Query<(&MapSaveTarget, &MapInstanceId, &Position, Option<&RespawnPoint>)>,
) {
    // Group entities by MapInstanceId
    let mut by_map: HashMap<MapInstanceId, Vec<SavedEntity>> = HashMap::new();

    for (_marker, map_id, position, respawn) in entity_query.iter() {
        let kind = if respawn.is_some() {
            SavedEntityKind::RespawnPoint
        } else {
            // This should never happen — all MapSaveTarget entities should have a recognized kind.
            // If hit, it means a new entity type was added with MapSaveTarget but not handled here.
            debug_assert!(false, "Entity with MapSaveTarget has no recognized SavedEntityKind");
            continue;
        };

        by_map.entry(map_id.clone())
            .or_default()
            .push(SavedEntity { kind, position: position.0 });
    }

    for (map_id, entities) in &by_map {
        let map_dir = map_save_dir(&save_path.0, map_id);
        if let Err(e) = save_entities(&map_dir, entities) {
            error!("Failed to save entities for {map_id:?}: {e}");
        }
    }
}
```

Integrate this into the existing save paths:
- In `save_dirty_chunks_debounced`, after saving chunks, call `collect_and_save_entities`
- In `save_world_on_shutdown`, after saving chunks, call `collect_and_save_entities`

Also update `save_dirty_chunks_debounced` to populate `meta.spawn_points` from `RespawnPoint` entities:

```rust
// In save_dirty_chunks_debounced, when building MapMeta:
let spawn_points: Vec<Vec3> = respawn_query.iter()
    .filter(|(_, map_id)| *map_id == current_map_id)
    .map(|(pos, _)| pos.0)
    .collect();
let meta = MapMeta {
    version: 1,
    seed: config.seed,
    generation_version: config.generation_version,
    spawn_points,
};
```

#### 4. Add entity load system
**File**: `crates/server/src/map.rs`

```rust
/// Load entities from disk for a map and spawn them in the ECS.
fn load_map_entities(
    commands: &mut Commands,
    save_path: &WorldSavePath,
    map_id: &MapInstanceId,
) -> usize {
    let map_dir = map_save_dir(&save_path.0, map_id);
    let entities = match load_entities(&map_dir) {
        Ok(entities) => entities,
        Err(e) => {
            warn!("Failed to load entities for {map_id:?}: {e}");
            return 0;
        }
    };

    let count = entities.len();
    for saved in entities {
        match saved.kind {
            SavedEntityKind::RespawnPoint => {
                commands.spawn((
                    RespawnPoint,
                    MapSaveTarget,
                    Position(saved.position),
                    map_id.clone(),
                ));
            }
            // Future variants handled here
        }
    }
    count
}
```

Call this during startup, after `spawn_overworld`:

```rust
fn load_overworld_entities(
    mut commands: Commands,
    save_path: Res<WorldSavePath>,
) {
    let count = load_map_entities(&mut commands, &save_path, &MapInstanceId::Overworld);
    if count > 0 {
        info!("Loaded {count} entities for overworld");
    }
}
```

`load_map_entities` is generic — it works for any `MapInstanceId`. For overworld, call it at startup. For homebases, call it in `ensure_map_exists` (Phase 8). A `Startup` system wraps it for the overworld:

```rust
fn load_startup_entities(
    mut commands: Commands,
    save_path: Res<WorldSavePath>,
) {
    load_map_entities(&mut commands, &save_path, &MapInstanceId::Overworld);
}
```

Register in `Startup` chain: `spawn_overworld` → `load_startup_entities` → `validate_respawn_points`.

#### 5. Replace spawn_respawn_points with validate_respawn_points
**File**: `crates/server/src/gameplay.rs`

Replace `spawn_respawn_points` with a validation function. Every map must have at least one respawn point — if none were loaded from disk (first run), spawn the default. This runs after entity loading.

```rust
/// Validates that every map has at least one respawn point.
/// On first run (no save), spawns a default. On subsequent runs, loaded from disk.
fn validate_respawn_points(
    mut commands: Commands,
    existing: Query<(&RespawnPoint, &MapInstanceId)>,
    map_registry: Res<MapRegistry>,
) {
    for (map_id, _entity) in map_registry.0.iter() {
        let has_respawn = existing.iter().any(|(_, mid)| mid == map_id);
        if !has_respawn {
            info!("Map {map_id:?} has no respawn points — spawning default");
            commands.spawn((
                RespawnPoint,
                MapSaveTarget,
                Position(DEFAULT_SPAWN_POS),
                map_id.clone(),
            ));
        }
    }
}
```

#### 6. Add MapSaveTarget to RespawnPoint requires
**File**: `crates/protocol/src/lib.rs`

Consider adding `#[require(MapSaveTarget)]` to `RespawnPoint` so it's always saved:

```rust
#[derive(Component, Clone, Debug)]
#[require(MapSaveTarget)]
pub struct RespawnPoint;
```

This ensures any entity with `RespawnPoint` automatically gets `MapSaveTarget`. If `require` isn't appropriate (some respawn points shouldn't be saved), add `MapSaveTarget` explicitly at spawn sites instead.

### Unit Tests

**File**: `crates/server/src/persistence.rs` — extend test module

```rust
#[test]
fn save_load_entities_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let entities = vec![
        SavedEntity { kind: SavedEntityKind::RespawnPoint, position: Vec3::new(1.0, 2.0, 3.0) },
        SavedEntity { kind: SavedEntityKind::RespawnPoint, position: Vec3::new(4.0, 5.0, 6.0) },
    ];
    save_entities(dir.path(), &entities).unwrap();
    let loaded = load_entities(dir.path()).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].kind, SavedEntityKind::RespawnPoint);
    assert_eq!(loaded[0].position, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(loaded[1].position, Vec3::new(4.0, 5.0, 6.0));
}

#[test]
fn load_entities_missing_file_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = load_entities(dir.path()).unwrap();
    assert!(loaded.is_empty());
}

#[test]
fn save_entities_creates_directory() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("deep/nested");
    save_entities(&nested, &[]).unwrap();
    assert!(nested.join("entities.bin").exists());
}

#[test]
fn save_entities_overwrites_previous() {
    let dir = tempfile::tempdir().unwrap();
    let v1 = vec![SavedEntity { kind: SavedEntityKind::RespawnPoint, position: Vec3::ZERO }];
    save_entities(dir.path(), &v1).unwrap();

    let v2 = vec![
        SavedEntity { kind: SavedEntityKind::RespawnPoint, position: Vec3::ONE },
        SavedEntity { kind: SavedEntityKind::RespawnPoint, position: Vec3::NEG_ONE },
    ];
    save_entities(dir.path(), &v2).unwrap();

    let loaded = load_entities(dir.path()).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].position, Vec3::ONE);
}

#[test]
fn corrupt_entities_file_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("entities.bin"), b"garbage data").unwrap();
    assert!(load_entities(dir.path()).is_err());
}

#[test]
fn entity_kind_serialization_roundtrip() {
    let kind = SavedEntityKind::RespawnPoint;
    let bytes = bincode::serialize(&kind).unwrap();
    let back: SavedEntityKind = bincode::deserialize(&bytes).unwrap();
    assert_eq!(back, SavedEntityKind::RespawnPoint);
}
```

### Integration Test (extend)

**File**: `crates/server/tests/world_persistence.rs`

Add a test (or extend the Phase 3 test) that validates entity persistence:

```rust
#[test]
fn entities_persist_across_server_restart() {
    let tmp = TempDir::new().unwrap();

    // First run: spawn respawn points, save
    {
        let mut app = create_test_server_app(tmp.path());
        // Run startup systems (spawns overworld, load_overworld_entities finds nothing,
        //   spawn_respawn_points creates default at DEFAULT_SPAWN_POS)
        app.update();

        // Verify a RespawnPoint entity exists
        let world = app.world();
        let respawn_count = world.query::<&RespawnPoint>().iter(world).count();
        assert_eq!(respawn_count, 1);

        // Manually add a second respawn point
        app.world_mut().spawn((
            RespawnPoint,
            MapSaveTarget,
            Position(Vec3::new(10.0, 20.0, 30.0)),
            MapInstanceId::Overworld,
        ));

        // Trigger save (directly call collect_and_save_entities or advance time)
        // ...

        // Verify entities.bin exists
        assert!(tmp.path().join("overworld/entities.bin").exists());
    }

    // Second run: verify entities loaded from disk
    {
        let mut app = create_test_server_app(tmp.path());
        app.update(); // runs startup: spawn_overworld → load_overworld_entities → spawn_respawn_points

        let world = app.world();
        let positions: Vec<Vec3> = world.query::<(&RespawnPoint, &Position)>()
            .iter(world)
            .map(|(_, pos)| pos.0)
            .collect();

        assert_eq!(positions.len(), 2);
        assert!(positions.contains(&DEFAULT_SPAWN_POS));
        assert!(positions.contains(&Vec3::new(10.0, 20.0, 30.0)));
    }
}
```

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client -c 1`

#### Manual Verification:
- [ ] `worlds/overworld/entities.bin` exists after server run
- [ ] Respawn points persist across server restart
- [ ] Death/respawn uses loaded respawn points correctly (nearest_respawn_pos finds them)
- [ ] `worlds/overworld/map.meta.bin` contains correct spawn_points list

**Note:** Phase 3 manual verification items also apply and should be re-verified:
- [ ] Start server, place blocks, stop server → `worlds/overworld/terrain/` contains chunk files
- [ ] Restart server → placed blocks are still there

---

## Phase 5: Server-to-Client Chunk Streaming

### Overview
Replace client-side chunk generation with server-to-client chunk streaming. The server sends palette-compressed chunk data to clients as they enter view range. Clients store received chunks in their `VoxelMapInstance` (retained for optimistic updates, raycasting, and remeshing). Remove `VoxelStateSync`, `VoxelModifications`, `MapWorld`, and client-side `VoxelGenerator` usage.

### Changes Required:

#### 1. Define chunk sync message types
**File**: `crates/protocol/src/map.rs`

Add `Reflect` derive to `PalettedChunk` (in `crates/voxel_map_engine/src/palette.rs`):

```rust
#[derive(Clone, Debug, Serialize, Deserialize, Reflect)]
pub enum PalettedChunk { ... }
```

Then use `PalettedChunk` directly in the network message — no separate `ChunkDataPayload` type needed. `WorldVoxel::Unset` should never appear in a generated/saved chunk (it's only used during generation as a sentinel), so it won't go over the wire.

```rust
/// Server sends a full chunk's palette-compressed data to a client.
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
pub struct ChunkDataSync {
    pub chunk_pos: IVec3,
    pub data: PalettedChunk,
}

/// Client requests a chunk from the server.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct ChunkRequest {
    pub chunk_pos: IVec3,
}

/// Server tells client to discard a chunk (left view range).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct ChunkUnload {
    pub chunk_pos: IVec3,
}
```

#### 2. Register chunk sync channel and messages
**File**: `crates/protocol/src/lib.rs`

```rust
/// Channel for chunk data streaming.
pub struct ChunkChannel;

// In ProtocolPlugin::build:
app.add_channel::<ChunkChannel>(ChannelSettings {
    mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
    ..default()
})
.add_direction(NetworkDirection::Bidirectional);

app.register_message::<ChunkDataSync>()
    .add_direction(NetworkDirection::ServerToClient);
app.register_message::<ChunkRequest>()
    .add_direction(NetworkDirection::ClientToServer);
app.register_message::<ChunkUnload>()
    .add_direction(NetworkDirection::ServerToClient);
```

Use `UnorderedReliable` — chunks must arrive but order doesn't matter (each chunk is independent).

#### 3. Client-driven chunk requests
**File**: `crates/client/src/map.rs`

The client determines which chunks it needs (from `ChunkTarget` distance) and requests them from the server. The server doesn't track per-client chunk state — it just serves requests.

```rust
/// Tracks which chunks the client has requested/received.
#[derive(Component, Default)]
pub struct ClientChunkState {
    /// Chunks we've received from the server.
    pub received: HashSet<IVec3>,
    /// Chunks we've requested but not yet received.
    pub pending_requests: HashSet<IVec3>,
}
```

New system: `request_missing_chunks` — replaces `spawn_missing_chunks` on client:

```rust
/// Requests chunks from server that the client needs but doesn't have.
fn request_missing_chunks(
    mut chunk_state: Query<&mut ClientChunkState>,
    chunk_targets: Query<(&ChunkTarget, &Position), (With<Predicted>, With<CharacterMarker>)>,
    map_query: Query<&VoxelMapConfig>,
    mut senders: Query<&mut MessageSender<ChunkRequest>>,
) {
    // Predicted player doesn't exist until first replication arrives post-connect
    let Ok((target, pos)) = chunk_targets.single() else { return };
    let config = map_query.get(target.map_entity)
        .expect("ChunkTarget.map_entity must have VoxelMapConfig");
    let mut state = chunk_state.get_mut(target.map_entity)
        .expect("ChunkTarget.map_entity must have ClientChunkState");

    let desired = collect_desired_positions_for(pos, config);

    // Request chunks we don't have and haven't requested yet
    for &chunk_pos in &desired {
        if state.received.contains(&chunk_pos) { continue; }
        if state.pending_requests.contains(&chunk_pos) { continue; }

        for mut sender in senders.iter_mut() {
            sender.send::<ChunkChannel>(ChunkRequest { chunk_pos });
        }
        state.pending_requests.insert(chunk_pos);
    }

    // Discard chunks that left view range
    let to_discard: Vec<IVec3> = state.received.iter()
        .filter(|pos| !desired.contains(pos))
        .copied()
        .collect();
    for chunk_pos in to_discard {
        state.received.remove(&chunk_pos);
        // loaded_chunks removal + mesh despawn handled by existing lifecycle systems
    }
    state.pending_requests.retain(|pos| desired.contains(pos));
}
```

#### 3b. Server: handle chunk requests
**File**: `crates/server/src/map.rs`

The server simply responds to chunk requests. No per-client tracking needed.

```rust
/// Handles client chunk requests — sends chunk data if available.
fn handle_chunk_requests(
    mut receivers: Query<(Entity, &mut MessageReceiver<ChunkRequest>)>,
    mut senders: Query<&mut MessageSender<ChunkDataSync>>,
    controlled_query: Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: Res<MapRegistry>,
    map_query: Query<&VoxelMapInstance>,
) {
    for (client_entity, mut receiver) in &mut receivers {
        for request in receiver.receive() {
            // Find which map this client is on
            let player_map_id = controlled_query.iter()
                .find(|(ctrl, _)| ctrl.owner == client_entity)
                .map(|(_, mid)| mid)
                .expect("requesting client must have a character with MapInstanceId");

            let map_entity = map_registry.get(player_map_id);
            // Map instance not yet spawned — skip until ready
            let Ok(instance) = map_query.get(map_entity) else { continue };

            // Send chunk if it exists in the octree
            if let Some(chunk_data) = instance.get_chunk_data(request.chunk_pos) {
                if let Ok(mut sender) = senders.get_mut(client_entity) {
                    sender.send::<ChunkChannel>(ChunkDataSync {
                        chunk_pos: request.chunk_pos,
                        data: chunk_data.voxels.clone(),
                    });
                }
            } else {
                // Chunk not generated yet — client will re-request next frame
                // since it remains in the desired set
                trace!("Chunk {} not yet available for request", request.chunk_pos);
            }
        }
    }
}
```

#### 4. Client: receive chunks and store in VoxelMapInstance
**File**: `crates/client/src/map.rs`

Replace the client's chunk generation pipeline. The client no longer runs `VoxelGenerator` — instead it receives chunk data from the server and stores it in `VoxelMapInstance`.

```rust
/// Receives chunk data from server and inserts into the local VoxelMapInstance.
fn handle_chunk_data_sync(
    mut receivers: Query<&mut MessageReceiver<ChunkDataSync>>,
    mut map_query: Query<(&mut VoxelMapInstance, &mut PendingChunks)>,
    map_registry: Res<MapRegistry>,
    // Need to know which map this client is viewing
    player_query: Query<&ChunkTarget, (With<Predicted>, With<CharacterMarker>)>,
) {
    // Predicted player doesn't exist until first replication arrives post-connect
    let Ok(chunk_target) = player_query.single() else { return };
    let (mut instance, mut pending) = map_query.get_mut(chunk_target.map_entity)
        .expect("ChunkTarget must reference a valid VoxelMapInstance");

    for mut receiver in &mut receivers {
        for sync in receiver.receive() {
            let chunk_data = ChunkData::from_voxels(&sync.data.to_voxels());
            let mesh = mesh_chunk_greedy(&voxels);

            // Store in octree
            instance.insert_chunk_data(sync.chunk_pos, chunk_data);
            instance.loaded_chunks.insert(sync.chunk_pos);

            // Spawn mesh entity (same as handle_completed_chunk)
            if let Some(mesh) = mesh {
                // Spawn VoxelChunk child entity with mesh
            }
        }
    }
}

/// Handles chunk unload messages from server.
fn handle_chunk_unload(
    mut receivers: Query<&mut MessageReceiver<ChunkUnload>>,
    mut map_query: Query<&mut VoxelMapInstance>,
    player_query: Query<&ChunkTarget, (With<Predicted>, With<CharacterMarker>)>,
) {
    // Predicted player doesn't exist until first replication arrives post-connect
    let Ok(chunk_target) = player_query.single() else { return };
    // Map entity may not exist yet during transition
    let Ok(mut instance) = map_query.get_mut(chunk_target.map_entity) else { return };

    for mut receiver in &mut receivers {
        for unload in receiver.receive() {
            instance.loaded_chunks.remove(&unload.chunk_pos);
            instance.remove_chunk_data(unload.chunk_pos);
            // despawn_out_of_range_chunks will clean up the mesh entity
        }
    }
}
```

#### 5. Remove client-side chunk generation
**File**: `crates/client/src/map.rs`

- Remove `VoxelGenerator` usage from client's `spawn_overworld` — client no longer needs a generator function
- Remove `spawn_map_instance`'s generator parameter on client side
- Client's `VoxelMapConfig` no longer needs `generator` field — set to a no-op or make it `Option`
- Keep `VoxelMapInstance` (needed for octree storage, voxel lookups, optimistic edits)

**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Client no longer runs `spawn_missing_chunks` / `spawn_chunk_gen_task`. Two approaches:
1. **Feature gate**: `#[cfg(feature = "server")]` on chunk generation systems
2. **Config flag**: Add `generates_chunks: bool` to `VoxelMapConfig`, skip generation when false

Approach 2 is simpler — client sets `generates_chunks: false`, server sets `true`. The `update_chunks` system skips `spawn_missing_chunks` when `generates_chunks` is false. The `remove_out_of_range_chunks` and `despawn_out_of_range_chunks` systems still run on client (to clean up unloaded chunks).

#### 6. Remove VoxelStateSync, VoxelModifications, MapWorld
**File**: `crates/protocol/src/map.rs`

- Remove `VoxelStateSync` message type
- Remove `MapWorld` resource

**File**: `crates/protocol/src/lib.rs`

- Remove `VoxelStateSync` channel registration
- Remove `MapWorld` re-export / init_resource

**File**: `crates/server/src/map.rs`

- Remove `VoxelModifications` resource (was kept as runtime-only sync in Phase 3, now fully replaced by chunk streaming)
- Remove `send_initial_voxel_state` observer
- Remove `VoxelModifications.push()` from `handle_voxel_edit_requests`

**File**: `crates/client/src/map.rs`

- Remove `handle_state_sync` system
- Remove `init_resource::<MapWorld>()`
- Remove `Res<MapWorld>` from `spawn_overworld` — seed now comes from `MapTransitionStart` or is set when creating the client's `VoxelMapInstance`

#### 7. Update MapTransitionStart to carry chunk streaming context

The client needs to know which map to associate incoming chunks with. `MapTransitionStart` already carries `target: MapInstanceId`, `seed`, `bounds`, etc. The client uses this to create a new `VoxelMapInstance` (with `generates_chunks: false`), then incoming `ChunkDataSync` messages populate it.

No changes needed to `MapTransitionStart` — the existing fields suffice.

#### 8. Initial connect flow

On first connect, the client receives chunks via `ChunkDataSync` messages (the server's `sync_client_chunks` system handles this automatically once the client's `ChunkTarget` is set up). No special initial sync message needed — the same chunk streaming path handles both initial load and movement-driven loading.

### Unit Tests

**File**: `crates/voxel_map_engine/src/palette.rs` — test `PalettedChunk` serde with `Reflect`

```rust
#[test]
fn paletted_chunk_network_serde_roundtrip() {
    let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME];
    voxels[100] = WorldVoxel::Solid(5);
    let paletted = PalettedChunk::from_voxels(&voxels);
    let bytes = bincode::serialize(&paletted).unwrap();
    let restored: PalettedChunk = bincode::deserialize(&bytes).unwrap();
    assert_eq!(paletted.to_voxels(), restored.to_voxels());
}

#[test]
fn paletted_chunk_single_value_network_roundtrip() {
    let paletted = PalettedChunk::SingleValue(WorldVoxel::Air);
    let bytes = bincode::serialize(&paletted).unwrap();
    let restored: PalettedChunk = bincode::deserialize(&bytes).unwrap();
    assert!(restored.is_uniform());
    assert_eq!(restored.get(0), WorldVoxel::Air);
}
```

### Integration Test

**File**: `crates/server/tests/integration.rs` — extend with chunk request/response test

```rust
#[test]
fn test_client_requests_chunk_and_receives_data() {
    let mut stepper = CrossbeamTestStepper::new();

    stepper.client_app.init_resource::<MessageBuffer<ChunkDataSync>>();
    stepper.client_app.add_systems(Update, collect_messages::<ChunkDataSync>);

    stepper.init();
    stepper.wait_for_connection();

    // Run enough ticks for server to generate chunks
    stepper.tick_step(10);

    // Client sends a chunk request
    stepper.client_app.world_mut()
        .entity_mut(stepper.client_entity)
        .get_mut::<MessageSender<ChunkRequest>>()
        .expect("Client should have MessageSender")
        .send::<ChunkChannel>(ChunkRequest { chunk_pos: IVec3::ZERO });

    stepper.tick_step(5);

    // Client should receive chunk data
    let buffer = stepper.client_app.world().resource::<MessageBuffer<ChunkDataSync>>();
    assert!(!buffer.messages.is_empty(), "Client should receive chunk data from server");
}
```

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client -c 1`

#### Manual Verification:
- [x] Client renders terrain received from server (no local generation)
- [x] Moving to new areas loads new chunks from server
- [ ] Chunks unload when leaving view range
- [ ] Map transitions work — new map's chunks stream from server
- [ ] Existing voxel edits are visible (baked into chunk data)
- [ ] No `VoxelGenerator` invoked on client side
- [ ] Performance: chunk streaming is fast enough for smooth gameplay

---

## Phase 6: Block Edit Prediction & Acknowledgment

### Overview
Add client-side prediction for block edits using a sequence-number system (matching Minecraft's approach). The client optimistically applies edits locally for instant feedback, the server validates and acknowledges with a sequence number, and the client reconciles predictions with authoritative state.

### Changes Required:

#### 1. Add sequence number to VoxelEditRequest
**File**: `crates/protocol/src/map.rs`

```rust
/// Client requests a voxel edit with a prediction sequence number.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct VoxelEditRequest {
    pub position: IVec3,
    pub voxel: VoxelType,
    pub sequence: u32,
}

/// Server acknowledges a block edit up to this sequence number.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct VoxelEditAck {
    pub sequence: u32,
}
```

Register `VoxelEditAck` in `ProtocolPlugin::build`:

```rust
app.register_message::<VoxelEditAck>()
    .add_direction(NetworkDirection::ServerToClient);
```

#### 2. Client: optimistic apply with sequence tracking
**File**: `crates/client/src/map.rs`

```rust
/// Tracks pending predictions for block edits.
#[derive(Resource, Default)]
pub struct VoxelPredictionState {
    pub next_sequence: u32,
    /// Predictions not yet acknowledged. Keyed by sequence number.
    pub pending: Vec<VoxelPrediction>,
}

/// A single pending block edit prediction awaiting server acknowledgment.
pub struct VoxelPrediction {
    pub sequence: u32,
    pub position: IVec3,
    pub old_voxel: VoxelType,
    pub new_voxel: VoxelType,
}

impl VoxelPredictionState {
    pub fn next(&mut self) -> u32 {
        let seq = self.next_sequence;
        self.next_sequence += 1;
        seq
    }
}
```

Update `send_voxel_edit` to optimistically apply and track prediction:

```rust
pub fn send_voxel_edit(
    position: IVec3,
    voxel: VoxelType,
    mut message_sender: Query<&mut MessageSender<VoxelEditRequest>>,
    mut prediction_state: ResMut<VoxelPredictionState>,
    mut voxel_world: VoxelWorld,
    player_query: Query<&ChunkTarget, (With<Predicted>, With<CharacterMarker>)>,
) {
    let chunk_target = player_query.single()
        .expect("send_voxel_edit called without a predicted player");
    let sequence = prediction_state.next();

    // Record old voxel for potential rollback
    let old_voxel = voxel_world.get_voxel(chunk_target.map_entity, position).into();

    // Optimistically apply locally — instant feedback
    voxel_world.set_voxel(chunk_target.map_entity, position, WorldVoxel::from(voxel));

    prediction_state.pending.push(VoxelPrediction {
        sequence,
        position,
        old_voxel,
        new_voxel: voxel,
    });

    for mut sender in message_sender.iter_mut() {
        sender.send::<VoxelChannel>(VoxelEditRequest { position, voxel, sequence });
    }
}
```

#### 3. Client: handle server ack and reconcile
**File**: `crates/client/src/map.rs`

```rust
/// Handles VoxelEditAck — clears acknowledged predictions.
fn handle_voxel_edit_ack(
    mut receivers: Query<&mut MessageReceiver<VoxelEditAck>>,
    mut prediction_state: ResMut<VoxelPredictionState>,
) {
    for mut receiver in &mut receivers {
        for ack in receiver.receive() {
            // Remove all predictions up to and including this sequence
            prediction_state.pending.retain(|p| p.sequence > ack.sequence);
        }
    }
}
```

**Invalid edit handling**: When the server rejects an edit (e.g., out of range, invalid block, anti-cheat), it sends a `VoxelEditReject` message instead of an ack. The client rolls back the prediction by restoring the old voxel:

```rust
/// Server rejects a block edit — client must roll back.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct VoxelEditReject {
    pub sequence: u32,
    pub position: IVec3,
    pub correct_voxel: VoxelType, // what the voxel actually is on the server
}
```

Client handler:

```rust
fn handle_voxel_edit_reject(
    mut receivers: Query<&mut MessageReceiver<VoxelEditReject>>,
    mut prediction_state: ResMut<VoxelPredictionState>,
    mut voxel_world: VoxelWorld,
    player_query: Query<&ChunkTarget, (With<Predicted>, With<CharacterMarker>)>,
) {
    // Predicted player doesn't exist until first replication arrives post-connect
    let Ok(chunk_target) = player_query.single() else { return };

    for mut receiver in &mut receivers {
        for reject in receiver.receive() {
            // Roll back: set the voxel to the server's authoritative value
            voxel_world.set_voxel(
                chunk_target.map_entity,
                reject.position,
                WorldVoxel::from(reject.correct_voxel),
            );
            // Clear the rejected prediction
            prediction_state.pending.retain(|p| p.sequence != reject.sequence);
        }
    }
}
```

Register `VoxelEditReject` in `ProtocolPlugin::build`:
```rust
app.register_message::<VoxelEditReject>()
    .add_direction(NetworkDirection::ServerToClient);
```

The client also receives `VoxelEditBroadcast` from other players. Since the client already has its own predictions applied, it should skip broadcasts for positions it has pending predictions on:

```rust
fn handle_voxel_broadcasts(
    mut receiver: Query<&mut MessageReceiver<VoxelEditBroadcast>>,
    mut voxel_world: VoxelWorld,
    prediction_state: Res<VoxelPredictionState>,
    player_query: Query<&ChunkTarget, (With<Predicted>, With<CharacterMarker>)>,
) {
    // Predicted player doesn't exist until first replication arrives post-connect
    let Ok(chunk_target) = player_query.single() else { return };

    for mut message_receiver in receiver.iter_mut() {
        for broadcast in message_receiver.receive() {
            // Skip broadcasts for positions with pending local predictions —
            // the prediction will be reconciled via ack/reject instead
            let has_prediction = prediction_state.pending.iter()
                .any(|p| p.position == broadcast.position);
            if has_prediction { continue; }

            voxel_world.set_voxel(
                chunk_target.map_entity,
                broadcast.position,
                WorldVoxel::from(broadcast.voxel),
            );
        }
    }
}
```

#### 4. Server: validate, apply, ack, and broadcast
**File**: `crates/server/src/map.rs`

Update `handle_voxel_edit_requests` to:
1. Determine which map the edit is for (via player's `MapInstanceId`)
2. Validate — reject if invalid
3. Apply if valid
4. Send `VoxelEditAck` to the originator (or `VoxelEditReject` on failure)
5. Broadcast `VoxelEditBroadcast` to other clients on the same map (room-scoped)

```rust
fn handle_voxel_edit_requests(
    mut receivers: Query<(Entity, &mut MessageReceiver<VoxelEditRequest>)>,
    mut ack_senders: Query<&mut MessageSender<VoxelEditAck>>,
    mut reject_senders: Query<&mut MessageSender<VoxelEditReject>>,
    mut pending_broadcasts: ResMut<PendingVoxelBroadcasts>,
    mut dirty_state: ResMut<WorldDirtyState>,
    time: Res<Time>,
    mut voxel_world: VoxelWorld,
    controlled_query: Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: Res<MapRegistry>,
) {
    for (client_entity, mut receiver) in &mut receivers {
        for request in receiver.receive() {
            let map_entity = resolve_player_map(
                client_entity, &controlled_query, &map_registry,
            );

            if !is_edit_valid(
                &request, map_entity, client_entity,
                &voxel_world, &mut reject_senders,
            ) {
                continue;
            }

            apply_voxel_edit(
                &request, map_entity, &mut voxel_world,
                &mut dirty_state, &time,
            );

            send_edit_ack(client_entity, request.sequence, &mut ack_senders);

            queue_edit_broadcast(
                request.position, request.voxel, &mut pending_broadcasts,
            );
        }
    }
}

/// Resolves which map entity a client's character is on.
fn resolve_player_map(
    client_entity: Entity,
    controlled_query: &Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: &MapRegistry,
) -> Entity {
    let player_map_id = controlled_query.iter()
        .find(|(ctrl, _)| ctrl.owner == client_entity)
        .map(|(_, mid)| mid)
        .expect("editing client must have a character with MapInstanceId");
    map_registry.get(player_map_id)
}

/// Validates the edit and sends a reject if invalid. Returns `true` if edit is valid.
fn is_edit_valid(
    request: &VoxelEditRequest,
    map_entity: Entity,
    client_entity: Entity,
    voxel_world: &VoxelWorld,
    reject_senders: &mut Query<&mut MessageSender<VoxelEditReject>>,
) -> bool {
    if validate_voxel_edit(request, map_entity, voxel_world) {
        return true;
    }
    let current_voxel = voxel_world.get_voxel(map_entity, request.position);
    if let Ok(mut sender) = reject_senders.get_mut(client_entity) {
        sender.send::<VoxelChannel>(VoxelEditReject {
            sequence: request.sequence,
            position: request.position,
            correct_voxel: current_voxel.into(),
        });
    }
    false
}

/// Applies the voxel edit and marks the world dirty.
fn apply_voxel_edit(
    request: &VoxelEditRequest,
    map_entity: Entity,
    voxel_world: &mut VoxelWorld,
    dirty_state: &mut WorldDirtyState,
    time: &Time,
) {
    voxel_world.set_voxel(map_entity, request.position, WorldVoxel::from(request.voxel));
    let now = time.elapsed_secs_f64();
    if !dirty_state.is_dirty {
        dirty_state.first_dirty_time = Some(now);
    }
    dirty_state.is_dirty = true;
    dirty_state.last_edit_time = now;
}

/// Sends an edit acknowledgment to the originating client.
fn send_edit_ack(
    client_entity: Entity,
    sequence: u32,
    ack_senders: &mut Query<&mut MessageSender<VoxelEditAck>>,
) {
    if let Ok(mut sender) = ack_senders.get_mut(client_entity) {
        sender.send::<VoxelChannel>(VoxelEditAck { sequence });
    }
}

/// Queues a voxel edit for batched broadcast (Phase 7).
fn queue_edit_broadcast(
    position: IVec3,
    voxel: VoxelType,
    pending: &mut PendingVoxelBroadcasts,
) {
    let chunk_pos = voxel_to_chunk_pos(position);
    pending.per_chunk.entry(chunk_pos).or_default().push((position, voxel));
}

/// Validates a voxel edit request against server-side rules.
/// Returns `false` if the edit should be rejected (out of bounds, invalid block, etc.).
fn validate_voxel_edit(
    request: &VoxelEditRequest,
    map_entity: Entity,
    voxel_world: &VoxelWorld,
) -> bool {
    // TODO: Add validation rules as needed:
    // - Position within map bounds
    // - Player within editing range
    // - Block type is valid
    // - Anti-cheat checks
    true // Accept all edits for now
}
```

The broadcast iterates clients in the room and sends individually, ensuring edits only reach clients on the same map. The originator is skipped (they already have the prediction applied).

Note: The exact `Room` API for iterating clients depends on lightyear's `Room` component structure. If `Room` doesn't expose a `clients()` method directly, use the room membership query pattern from lightyear's API.

### Unit Tests

```rust
#[test]
fn prediction_state_sequence_increments() {
    let mut state = VoxelPredictionState::default();
    assert_eq!(state.next(), 0);
    assert_eq!(state.next(), 1);
    assert_eq!(state.next(), 2);
}

#[test]
fn ack_clears_predictions_up_to_sequence() {
    let mut state = VoxelPredictionState::default();
    for i in 0..5 {
        state.pending.push(VoxelPrediction {
            sequence: i,
            position: IVec3::ZERO,
            old_voxel: VoxelType::Air,
            new_voxel: VoxelType::Solid(1),
        });
    }
    // Ack sequence 2 — clears 0, 1, 2
    state.pending.retain(|p| p.sequence > 2);
    assert_eq!(state.pending.len(), 2);
    assert_eq!(state.pending[0].sequence, 3);
}
```

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client -c 1`

#### Manual Verification:
- [ ] Block placement/breaking feels instant (no visible delay)
- [ ] Other players see edits from the editing player
- [ ] Invalid edits are rolled back (server rejection)
- [ ] Rapid edits under latency don't cause ghost blocks
- [ ] Edits on different maps are correctly scoped

---

## Phase 7: Batched Section Updates

### Overview
When multiple block changes happen in the same chunk on the same server tick, batch them into a single `SectionBlocksUpdate` message instead of sending individual `VoxelEditBroadcast` messages. This reduces packet count for multi-block operations (e.g., explosions, fill commands).

### Changes Required:

#### 1. Define batched update message
**File**: `crates/protocol/src/map.rs`

```rust
/// Batched block changes for a single chunk, sent when 2+ changes happen in one tick.
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
pub struct SectionBlocksUpdate {
    pub chunk_pos: IVec3,
    pub changes: Vec<(IVec3, VoxelType)>,
}
```

Register in `ProtocolPlugin::build`:

```rust
app.register_message::<SectionBlocksUpdate>()
    .add_direction(NetworkDirection::ServerToClient);
```

#### 2. Server: accumulate and batch per tick
**File**: `crates/server/src/map.rs`

Add a resource to accumulate edits per tick:

```rust
/// Accumulates voxel edits per chunk during a tick for batching.
#[derive(Resource, Default)]
pub struct PendingVoxelBroadcasts {
    /// chunk_pos → list of (world_pos, voxel) changes
    pub per_chunk: HashMap<IVec3, Vec<(IVec3, VoxelType)>>,
}
```

Split `handle_voxel_edit_requests` — it still applies edits immediately but defers broadcasting:

```rust
// In handle_voxel_edit_requests, replace the sender.send broadcast with:
let chunk_pos = voxel_to_chunk_pos(request.position);
pending_broadcasts.per_chunk
    .entry(chunk_pos)
    .or_default()
    .push((request.position, request.voxel));
```

New system: `flush_voxel_broadcasts` — runs after `handle_voxel_edit_requests`:

```rust
/// Flushes accumulated voxel edits as either individual or batched messages.
fn flush_voxel_broadcasts(
    mut pending: ResMut<PendingVoxelBroadcasts>,
    mut sender: ServerMultiMessageSender,
    server: Single<&Server>,
) {
    if pending.per_chunk.is_empty() { return; }

    let server_ref = server.into_inner();
    for (chunk_pos, changes) in pending.per_chunk.drain() {
        if changes.len() == 1 {
            // Single change — send individual broadcast
            let (pos, voxel) = changes[0];
            sender.send::<_, VoxelChannel>(
                &VoxelEditBroadcast { position: pos, voxel },
                server_ref,
                &NetworkTarget::All,
            ).ok();
        } else {
            // Multiple changes — send batched update
            sender.send::<_, VoxelChannel>(
                &SectionBlocksUpdate { chunk_pos, changes },
                server_ref,
                &NetworkTarget::All,
            ).ok();
        }
    }
}
```

#### 3. Client: handle batched updates
**File**: `crates/client/src/map.rs`

```rust
/// Handles batched block updates from server.
fn handle_section_blocks_update(
    mut receivers: Query<&mut MessageReceiver<SectionBlocksUpdate>>,
    mut voxel_world: VoxelWorld,
    prediction_state: Res<VoxelPredictionState>,
    player_query: Query<&ChunkTarget, (With<Predicted>, With<CharacterMarker>)>,
) {
    // Predicted player doesn't exist until first replication arrives post-connect
    let Ok(chunk_target) = player_query.single() else { return };

    for mut receiver in &mut receivers {
        for update in receiver.receive() {
            for (pos, voxel) in &update.changes {
                // Skip positions with pending local predictions —
                // reconciled via ack/reject instead
                let has_prediction = prediction_state.pending.iter()
                    .any(|p| p.position == *pos);
                if has_prediction { continue; }

                voxel_world.set_voxel(
                    chunk_target.map_entity,
                    *pos,
                    WorldVoxel::from(*voxel),
                );
            }
        }
    }
}
```

### Unit Tests

Phase 7 unit tests require a Bevy `App` with `flush_voxel_broadcasts` wired up and message collection, since the behavior under test is "which message type gets sent." Pure data-structure tests don't verify the branching logic. Use the `CrossbeamTestStepper` pattern or a minimal `App` with a mock sender to capture sent messages.

```rust
#[test]
fn single_change_sends_individual_broadcast() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<PendingVoxelBroadcasts>();
    app.init_resource::<MessageBuffer<VoxelEditBroadcast>>();
    app.init_resource::<MessageBuffer<SectionBlocksUpdate>>();
    app.add_systems(Update, (flush_voxel_broadcasts, collect_sent_messages).chain());

    // Insert a single change
    app.world_mut().resource_mut::<PendingVoxelBroadcasts>()
        .per_chunk.entry(IVec3::ZERO).or_default()
        .push((IVec3::new(1, 2, 3), VoxelType::Solid(1)));

    app.update();

    let broadcasts = app.world().resource::<MessageBuffer<VoxelEditBroadcast>>();
    let batched = app.world().resource::<MessageBuffer<SectionBlocksUpdate>>();
    assert_eq!(broadcasts.messages.len(), 1, "single change should send individual broadcast");
    assert!(batched.messages.is_empty(), "single change should not send batched update");
}

#[test]
fn multiple_changes_in_same_chunk_sends_batched_update() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<PendingVoxelBroadcasts>();
    app.init_resource::<MessageBuffer<VoxelEditBroadcast>>();
    app.init_resource::<MessageBuffer<SectionBlocksUpdate>>();
    app.add_systems(Update, (flush_voxel_broadcasts, collect_sent_messages).chain());

    {
        let mut pending = app.world_mut().resource_mut::<PendingVoxelBroadcasts>();
        let chunk = IVec3::ZERO;
        pending.per_chunk.entry(chunk).or_default()
            .push((IVec3::new(1, 2, 3), VoxelType::Solid(1)));
        pending.per_chunk.entry(chunk).or_default()
            .push((IVec3::new(4, 5, 6), VoxelType::Air));
    }

    app.update();

    let broadcasts = app.world().resource::<MessageBuffer<VoxelEditBroadcast>>();
    let batched = app.world().resource::<MessageBuffer<SectionBlocksUpdate>>();
    assert!(broadcasts.messages.is_empty(), "multi-change should not send individual broadcasts");
    assert_eq!(batched.messages.len(), 1, "multi-change should send one batched update");
    assert_eq!(batched.messages[0].changes.len(), 2);
}

#[test]
fn different_chunks_produce_separate_messages() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<PendingVoxelBroadcasts>();
    app.init_resource::<MessageBuffer<VoxelEditBroadcast>>();
    app.init_resource::<MessageBuffer<SectionBlocksUpdate>>();
    app.add_systems(Update, (flush_voxel_broadcasts, collect_sent_messages).chain());

    {
        let mut pending = app.world_mut().resource_mut::<PendingVoxelBroadcasts>();
        pending.per_chunk.entry(IVec3::ZERO).or_default()
            .push((IVec3::new(1, 2, 3), VoxelType::Solid(1)));
        pending.per_chunk.entry(IVec3::ONE).or_default()
            .push((IVec3::new(17, 18, 19), VoxelType::Solid(2)));
    }

    app.update();

    let broadcasts = app.world().resource::<MessageBuffer<VoxelEditBroadcast>>();
    assert_eq!(broadcasts.messages.len(), 2, "each single-change chunk gets its own broadcast");
}

#[test]
fn pending_cleared_after_flush() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<PendingVoxelBroadcasts>();
    app.add_systems(Update, flush_voxel_broadcasts);

    app.world_mut().resource_mut::<PendingVoxelBroadcasts>()
        .per_chunk.entry(IVec3::ZERO).or_default()
        .push((IVec3::new(1, 2, 3), VoxelType::Solid(1)));

    app.update();

    let pending = app.world().resource::<PendingVoxelBroadcasts>();
    assert!(pending.per_chunk.is_empty(), "pending should be drained after flush");
}
```

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client -c 1`

#### Manual Verification:
- [ ] Single block edits still work normally
- [ ] Multi-block operations (if any exist) are batched correctly
- [ ] No regressions in edit latency
- [ ] Network traffic reduced for multi-block changes (observable in logs)

---

## Phase 8: Multi-Map Persistence

### Overview
Extend persistence to save/load Homebase maps. Each map type gets its own subdirectory under `worlds/`. All maps share the same debounced save timer — only dirty chunks are written. Homebases load from disk when a player transitions to their homebase. Chunk streaming (Phase 5) handles sending homebase chunk data to clients.

### Changes Required:

#### 1. Set save_dir on homebase creation
**File**: `crates/server/src/map.rs`

In `ensure_map_exists` (line 538-570), when creating a new homebase instance, set `save_dir` and load existing metadata if available:

```rust
/// Ensures a map instance exists in the registry, creating it if needed.
/// Panics if called for Overworld (must be spawned at startup).
fn ensure_map_exists(
    commands: &mut Commands,
    map_registry: &mut MapRegistry,
    map_id: &MapInstanceId,
    save_path: &WorldSavePath,
) -> Entity {
    if let Some(&entity) = map_registry.0.get(map_id) {
        return entity;
    }

    match map_id {
        MapInstanceId::Homebase { owner } => {
            spawn_homebase(commands, map_registry, *owner, map_id, save_path)
        }
        MapInstanceId::Overworld => {
            panic!("Overworld should be spawned at startup, not lazily");
        }
    }
}

/// Creates a new homebase map instance, loading metadata from disk if available.
fn spawn_homebase(
    commands: &mut Commands,
    map_registry: &mut MapRegistry,
    owner: u64,
    map_id: &MapInstanceId,
    save_path: &WorldSavePath,
) -> Entity {
    let map_dir = map_save_dir(&save_path.0, map_id);
    let seed = load_homebase_seed(&map_dir, owner);
    let bounds = IVec3::new(4, 4, 4);

    let (instance, mut config, homebase) =
        VoxelMapInstance::homebase(owner, bounds, flat_terrain_voxels_arc());
    config.save_dir = Some(map_dir);
    config.seed = seed;

    let entity = commands
        .spawn((instance, config, homebase, map_id.clone()))
        .id();
    map_registry.insert(map_id.clone(), entity);
    entity
}

/// Loads the homebase seed from saved metadata, or derives from owner ID for new homebases.
fn load_homebase_seed(map_dir: &Path, owner: u64) -> u64 {
    match load_map_meta(map_dir) {
        Ok(Some(meta)) => {
            info!("Loading homebase-{owner} from saved metadata (seed={})", meta.seed);
            meta.seed
        }
        _ => {
            let seed = seed_from_id(owner);
            info!("Creating new homebase-{owner} (seed={seed})");
            seed
        }
    }
}
```

Update `execute_server_transition` to pass `save_path` to `ensure_map_exists`.

#### 2. Load homebase entities on creation
**File**: `crates/server/src/map.rs`

After `ensure_map_exists` creates a homebase, load its entities from disk:

```rust
// In execute_server_transition, after ensure_map_exists:
let map_entity = ensure_map_exists(&mut commands, &mut map_registry, &target_map_id, &save_path);

// If this map was just created (wasn't in registry before), load its entities
// The load_map_entities function is idempotent — it only spawns if entities.bin exists
load_map_entities(&mut commands, &save_path, &target_map_id);
```

To avoid loading entities multiple times, track whether entities have been loaded for each map. Add a marker component:

```rust
/// Marker indicating entities for this map have been loaded from disk.
/// Prevents duplicate loading on repeated map transitions.
#[derive(Component)]
struct EntitiesLoaded;
```

Check for this before loading:

```rust
if !commands.entity(map_entity).contains::<EntitiesLoaded>() {
    load_map_entities(&mut commands, &save_path, &target_map_id);
    commands.entity(map_entity).insert(EntitiesLoaded);
}
```

#### 3. Save all maps on the same debounce timer
**File**: `crates/server/src/map.rs`

The `save_dirty_chunks_debounced` system from Phase 3 already iterates ALL map instances (`Query<(&mut VoxelMapInstance, &VoxelMapConfig, &MapInstanceId)>`). This naturally covers homebases — when a player edits voxels in their homebase, the chunks are marked dirty, and the debounce timer saves them.

No additional system needed for homebase saves. The existing `save_dirty_chunks_debounced` and `save_world_on_shutdown` handle all maps uniformly.

Verify that `handle_voxel_edit_requests` correctly identifies which map entity the edit belongs to. Currently (map.rs line 359-363) it always targets the overworld entity. This needs to be updated to use the player's current `MapInstanceId`:

```rust
fn handle_voxel_edit_requests(
    // ... existing params ...
    controlled_by_query: Query<&ControlledBy>,
    character_query: Query<&MapInstanceId, With<CharacterMarker>>,
    map_registry: Res<MapRegistry>,
    room_registry: Res<RoomRegistry>,
) {
    for (client_entity, mut receiver) in &mut edit_receivers {
        for edit in receiver.drain::<VoxelEditRequest>() {
            // Find which map the editing player is on.
            // The client entity has a ControlledBy component pointing to the character entity,
            // which has a MapInstanceId component.
            let player_map_id = controlled_by_query.get(client_entity)
                .ok()
                .and_then(|controlled| character_query.get(controlled.entity()).ok())
                .expect("editing client must have a character with MapInstanceId");
            let map_entity = map_registry.get(player_map_id);

            voxel_world.set_voxel(map_entity, edit.position, edit.voxel.into());

            // ... dirty state update, broadcast ...
        }
    }
}
```

This is a correctness fix: without it, homebase edits would be applied to the overworld.

#### 4. Chunk streaming on map transition

When a player transitions to a homebase, the server streams chunk data to the client via `ChunkDataSync` messages (Phase 5). The client's `ChunkTarget` is reassigned to the new map entity, and `sync_client_chunks` (Phase 5) automatically handles sending the new map's chunks.

No explicit initial sync needed — the same chunk streaming path handles transitions. The client's `ClientChunkState` is reset when it transitions to a new map, causing `request_missing_chunks` to request fresh chunks for the new map.

#### 5. Handle homebase unloading (save before despawn)
**File**: `crates/server/src/map.rs`

Currently homebases are never despawned — they persist in memory once created. If we add homebase unloading in the future (to reclaim memory when no players are present), we need a pre-despawn save. For now, this is not needed since:
- `save_world_on_shutdown` saves all maps on graceful shutdown
- `save_dirty_chunks_debounced` saves dirty chunks periodically
- Evicted dirty chunks are saved during eviction

If homebase unloading is added later, add a system that:
1. Detects when a homebase has no `ChunkTarget` entities pointing at it
2. Saves all dirty chunks and entities
3. Despawns the map entity and its children
4. Removes from `MapRegistry`

This is explicitly out of scope for now.

#### 6. Update MapTransitionStart for chunk streaming context
**File**: `crates/server/src/map.rs`

`MapTransitionStart` still carries `seed`, `bounds`, and `spawn_position`. With chunk streaming (Phase 5), the client uses `seed` only for display purposes (not generation). Verify that `execute_server_transition` reads `config.seed` from saved metadata:

```rust
// In execute_server_transition:
let (_, config) = map_query.get(target_map_entity).unwrap();
sender.send(&MapTransitionStart {
    seed: config.seed,
    generation_version: config.generation_version,
    bounds: config.bounds,
    spawn_position: DEFAULT_SPAWN_POS,
}).expect("send map transition start");
```

The client resets its `ClientChunkState` when receiving `MapTransitionStart`, so `request_missing_chunks` will request the new map's chunks:

```rust
// In handle_map_transition_start on client, after creating the new VoxelMapInstance:
if let Ok(mut state) = chunk_state.get_mut(new_map_entity) {
    state.received.clear();
    state.pending_requests.clear();
}
```

### Unit Tests

**File**: `crates/server/src/persistence.rs` — extend test module

```rust
#[test]
fn map_save_dir_different_homebases_are_isolated() {
    let base = Path::new("worlds");
    let dir1 = map_save_dir(base, &MapInstanceId::Homebase { owner: 1 });
    let dir2 = map_save_dir(base, &MapInstanceId::Homebase { owner: 2 });
    assert_ne!(dir1, dir2);
    assert_eq!(dir1, PathBuf::from("worlds/homebase-1"));
    assert_eq!(dir2, PathBuf::from("worlds/homebase-2"));
}

#[test]
fn overworld_and_homebase_dirs_are_isolated() {
    let base = Path::new("worlds");
    let ow = map_save_dir(base, &MapInstanceId::Overworld);
    let hb = map_save_dir(base, &MapInstanceId::Homebase { owner: 1 });
    assert_ne!(ow, hb);
}
```

**File**: `crates/server/tests/voxel_persistence.rs` — add multi-map tests

```rust
#[test]
fn multiple_maps_save_independently() {
    let tmp = TempDir::new().unwrap();
    let ow_dir = map_save_dir(tmp.path(), &MapInstanceId::Overworld);
    let hb_dir = map_save_dir(tmp.path(), &MapInstanceId::Homebase { owner: 42 });

    // Save different chunk data to each map
    let mut ow_chunk = ChunkData::new_empty();
    ow_chunk.voxels[0] = WorldVoxel::Solid(1);
    chunk_persist::save_chunk(&ow_dir, IVec3::ZERO, &ow_chunk).unwrap();

    let mut hb_chunk = ChunkData::new_empty();
    hb_chunk.voxels[0] = WorldVoxel::Solid(99);
    chunk_persist::save_chunk(&hb_dir, IVec3::ZERO, &hb_chunk).unwrap();

    // Load each independently
    let ow_loaded = chunk_persist::load_chunk(&ow_dir, IVec3::ZERO).unwrap().unwrap();
    let hb_loaded = chunk_persist::load_chunk(&hb_dir, IVec3::ZERO).unwrap().unwrap();
    assert_eq!(ow_loaded.voxels[0], WorldVoxel::Solid(1));
    assert_eq!(hb_loaded.voxels[0], WorldVoxel::Solid(99));
}

#[test]
fn homebase_metadata_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let hb_dir = map_save_dir(tmp.path(), &MapInstanceId::Homebase { owner: 123 });

    let meta = MapMeta {
        version: 1,
        seed: 123, // seed_from_id(123)
        generation_version: 0,
        spawn_points: vec![Vec3::new(0.0, 5.0, 0.0)],
    };
    save_map_meta(&hb_dir, &meta).unwrap();

    let loaded = load_map_meta(&hb_dir).unwrap().expect("meta should exist");
    assert_eq!(loaded.seed, 123);
}

#[test]
fn homebase_entities_saved_separately() {
    let tmp = TempDir::new().unwrap();
    let ow_dir = map_save_dir(tmp.path(), &MapInstanceId::Overworld);
    let hb_dir = map_save_dir(tmp.path(), &MapInstanceId::Homebase { owner: 1 });

    save_entities(&ow_dir, &[
        SavedEntity { kind: SavedEntityKind::RespawnPoint, position: Vec3::ZERO },
    ]).unwrap();
    save_entities(&hb_dir, &[
        SavedEntity { kind: SavedEntityKind::RespawnPoint, position: Vec3::ONE },
        SavedEntity { kind: SavedEntityKind::RespawnPoint, position: Vec3::NEG_ONE },
    ]).unwrap();

    assert_eq!(load_entities(&ow_dir).unwrap().len(), 1);
    assert_eq!(load_entities(&hb_dir).unwrap().len(), 2);
}
```

### Integration Test (complete)

**File**: `crates/server/tests/world_persistence.rs`

The final integration test validates the full feature end-to-end:

```rust
#[test]
fn full_world_persistence_cycle() {
    let tmp = TempDir::new().unwrap();

    // === First run ===
    {
        let mut app = create_test_server_app(tmp.path());
        app.update(); // startup: overworld spawned, default respawn point created

        // Generate some overworld chunks (run several frames with a ChunkTarget)
        for _ in 0..10 { app.update(); }

        // Edit an overworld voxel
        // (access VoxelWorld system param and call set_voxel)
        {
            let world = app.world_mut();
            let overworld_entity = world.resource::<MapRegistry>()
                .get(&MapInstanceId::Overworld);
            // ... set_voxel(overworld_entity, IVec3::new(0, 0, 0), WorldVoxel::Solid(42))
        }
        app.update(); // set_voxel mutates octree + marks chunk dirty

        // Create a homebase
        {
            let world = app.world_mut();
            let save_path = world.resource::<WorldSavePath>().clone();
            let mut map_registry = world.resource_mut::<MapRegistry>();
            let hb_id = MapInstanceId::Homebase { owner: 999 };
            ensure_map_exists(&mut world.commands(), &mut map_registry, &hb_id, &save_path);
        }
        app.update();

        // Edit a homebase voxel
        {
            let world = app.world_mut();
            let hb_entity = world.resource::<MapRegistry>()
                .get(&MapInstanceId::Homebase { owner: 999 });
            // ... set_voxel(hb_entity, IVec3::new(1, 1, 1), WorldVoxel::Solid(7))
        }
        app.update(); // flush

        // Trigger save (advance time past debounce threshold or call directly)
        // ... save_dirty_chunks_for_instance for each map ...

        // Verify files exist
        assert!(tmp.path().join("overworld/map.meta.bin").exists());
        assert!(tmp.path().join("overworld/entities.bin").exists());
        assert!(!chunk_persist::list_saved_chunks(&tmp.path().join("overworld")).unwrap().is_empty());
        assert!(tmp.path().join("homebase-999/map.meta.bin").exists());
    }

    // === Second run ===
    {
        let mut app = create_test_server_app(tmp.path());
        app.update(); // startup: overworld loaded, respawn points loaded from disk

        // Verify overworld
        {
            let world = app.world();
            // Check overworld voxel edit persisted
            // ... get_voxel(overworld_entity, IVec3::new(0, 0, 0)) == WorldVoxel::Solid(42)

            // Check respawn points loaded
            let respawn_count = world.query::<&RespawnPoint>().iter(world).count();
            assert!(respawn_count >= 1);

            // Check metadata
            let meta = load_map_meta(&tmp.path().join("overworld")).unwrap().unwrap();
            assert_eq!(meta.seed, 999); // overworld seed
        }

        // Trigger homebase creation (simulating a player transition)
        {
            let world = app.world_mut();
            let save_path = world.resource::<WorldSavePath>().clone();
            let mut map_registry = world.resource_mut::<MapRegistry>();
            let hb_id = MapInstanceId::Homebase { owner: 999 };
            ensure_map_exists(&mut world.commands(), &mut map_registry, &hb_id, &save_path);
        }
        app.update();

        // Verify homebase loaded from saved metadata
        {
            let world = app.world();
            let hb_entity = world.resource::<MapRegistry>()
                .get(&MapInstanceId::Homebase { owner: 999 });
            let config = world.get::<VoxelMapConfig>(hb_entity).unwrap();
            assert_eq!(config.seed, 999); // seed_from_id(999)

            // Homebase voxel edit should load when chunks are generated
            // (chunks load from disk via save_dir in spawn_chunk_gen_task)
        }
    }
}
```

### Success Criteria:

#### Automated Verification:
- [ ] All tests pass: `cargo test-all`
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client -c 1`

#### Manual Verification:
- [ ] `worlds/overworld/` and `worlds/homebase-<id>/` directories created
- [ ] Modify homebase terrain, restart server, transition to homebase → terrain persists
- [ ] Different players' homebases are isolated (player A's edits don't appear in player B's homebase)
- [ ] Voxel edits in homebase are correctly scoped (don't affect overworld)
- [ ] Client connecting to a homebase receives chunks via streaming (not local generation)
- [ ] No performance regression during saves

---

## Testing Strategy

### Unit Tests (per phase):
- Phase 1: Octree insert/get/remove, fill type classification, chunk hash, overwrite behavior
- Phase 2: PalettedChunk from_voxels/to_voxels roundtrip, single-value optimization, get/set indexed access, set expanding palette, set transitioning from single-value, memory_usage, bits_needed, serde roundtrip. Direct mutation: set_voxel mutates octree in-place, boundary edits update neighbor padding, unloaded chunk edits are dropped, multiple edits to same chunk produce single remesh entry
- Phase 3: Chunk file save/load roundtrip, zstd compression, filename parsing (positive/negative coords), metadata save/load, directory creation, corrupt file handling, chunk deletion, listing
- Phase 4: Entity save/load roundtrip, missing file returns empty, overwrite behavior, corrupt file handling, kind serialization roundtrip
- Phase 5: PalettedChunk network serde roundtrip, ChunkRequest/ChunkDataSync integration, client chunk state management
- Phase 6: Prediction sequence increment, ack clears predictions up to sequence, prediction filtering on broadcasts
- Phase 7: Single change sends individual broadcast, multiple changes batched, different chunks sent separately
- Phase 8: Map directory isolation (overworld vs homebase vs different homebases), multi-map chunk save independence, homebase metadata roundtrip, cross-map entity isolation

### Integration Tests:
- `crates/server/tests/world_persistence.rs` — persistence-focused (MinimalPlugins, no networking):
  - Built incrementally: Phase 3 adds `terrain_persists_across_server_restart`, Phase 4 adds `entities_persist_across_server_restart`, Phase 8 adds `full_world_persistence_cycle`
  - Final test validates: terrain persistence, entity persistence, metadata persistence, multi-map isolation, homebase load-from-disk
- `crates/server/tests/integration.rs` — networking-focused (CrossbeamTestStepper):
  - Phase 5 adds `test_chunk_data_streamed_to_client`
  - Phase 6 adds `test_voxel_edit_ack_received` (client sends edit, receives ack)
  - Phase 7 adds `test_batched_updates_received` (server sends batch for multi-edit)

### Existing Test Updates:
- `crates/server/tests/voxel_persistence.rs` — the 4 existing tests (`test_save_load_cycle`, `test_corrupt_file_recovery`, `test_generation_metadata_mismatch`, `test_shutdown_save`) are all based on the old `VoxelWorldSave`/`VoxelModifications` system and must be rewritten for the new directory-based system. Specifically:
  - `test_save_load_cycle` → replaced by `save_load_chunk_roundtrip` + `save_load_map_meta_roundtrip`
  - `test_corrupt_file_recovery` → replaced by `corrupt_chunk_file_returns_error` + `corrupt_entities_file_returns_error`
  - `test_generation_metadata_mismatch` → replaced by metadata version check in `load_map_meta`
  - `test_shutdown_save` → replaced by `dirty_chunks_saved_on_debounce` + integration test

## Performance Considerations

- **No regeneration on edit**: Direct octree mutation + async remesh eliminates the old regenerate-from-scratch cycle. A single voxel edit triggers only a `PalettedChunk::set` (microseconds) + async remesh (same cost as initial meshing, ~1ms off main thread). The old approach evicted the chunk, re-ran the generator with all overrides, then remeshed — substantially more expensive.
- **Old mesh stays visible**: No visual gap during remesh. The old mesh renders until the async remesh task completes and swaps the mesh handle. This is a significant UX improvement over the old approach where the chunk entity was despawned during regeneration.
- **zstd level 3**: Fast compression, ~90% size reduction on voxel data. Level 3 is the sweet spot.
- **Per-chunk dirty tracking**: Only modified chunks are re-saved. Most chunks save once (on generation) and never again unless a player edits them.
- **Atomic writes**: tmp file + rename prevents partial writes on crash.
- **Memory — octree eviction**: Chunk data is evicted from the octree when chunks leave the loaded range (same lifetime as the mesh entities). Only currently-loaded chunks occupy memory. At spawning_distance=2 (current overworld default), that's a 5x5x5 = 125 chunk cube per player × ~46KB/chunk ≈ ~6MB per player — negligible. Even at spawning_distance=10, eviction keeps memory bounded to the loaded set rather than accumulating indefinitely.
- **Eviction + persistence**: When a dirty chunk is evicted (Phase 3+), it must be saved to disk first. The save happens synchronously during eviction. This is acceptable because chunk saves are fast (bincode + zstd of palette-compressed data takes <1ms) and eviction only affects chunks at the edge of the loaded range (a few per frame at most).
- **Palette compression**: Uniform chunks (all air, all stone) cost ~2 bytes. Mixed chunks with 2-5 voxel types use 1-3 bits/entry ≈ 0.7-2.2 KiB vs 11.4 KiB flat — a 5-16× reduction. This benefits both in-memory footprint and save file size (less data to compress with zstd).

## Migration Notes

- The old `world_save/voxel_world.bin` format is abandoned. No migration path — players must start fresh. Acceptable for pre-release.
- **Phase 2**: `modified_voxels`, `write_buffer`, `flush_write_buffer`, `collect_chunk_overrides`, `apply_overrides` are removed. Voxel edits are baked directly into the octree via `PalettedChunk::set`. Async remesh pipeline replaces chunk regeneration on edit. **Temporary regression**: edits lost on chunk eviction until Phase 3 adds persistence.
- **Phase 3**: `VoxelWorldSave`, `VoxelDirtyState`, `VoxelSavePath` resources are removed. `VoxelModifications` is kept as runtime-only append log for client sync (NOT loaded from disk). `VoxelStateSync`, `MapWorld` kept temporarily for networking.
- **Phase 5**: `VoxelModifications`, `VoxelStateSync`, `MapWorld`, `send_initial_voxel_state` are removed. Replaced by chunk streaming (`ChunkDataSync` messages).
- The `MapWorld` resource's `seed` field is replaced by `DEFAULT_OVERWORLD_SEED` constant (overworld) or `seed_from_id(owner)` (homebases), stored per-map in `VoxelMapConfig.seed`. Its `generation_version` field moves to `VoxelMapConfig.generation_version`, persisted per-map in `MapMeta`.
- Client-side chunk generation is removed in Phase 5. Clients receive all chunk data from the server.
- `VoxelEditRequest` gains a `sequence` field in Phase 6. `VoxelEditAck` is a new server→client message.

## References

- Research — map saving: [doc/research/2026-03-09-minecraft-style-map-directory-saving.md](doc/research/2026-03-09-minecraft-style-map-directory-saving.md)
- Research — world sync protocol: [doc/research/2026-03-11-minecraft-world-sync-protocol.md](doc/research/2026-03-11-minecraft-world-sync-protocol.md)
- Original persistence research: [doc/research/2026-01-17-voxel-world-save-load.md](doc/research/2026-01-17-voxel-world-save-load.md)
- Current save system: [crates/server/src/map.rs:207-344](crates/server/src/map.rs#L207-L344)
- Current networking: [crates/server/src/map.rs:346-403](crates/server/src/map.rs#L346-L403)
- Client map systems: [crates/client/src/map.rs](crates/client/src/map.rs)
- Octree API: [git/grid-tree-rs/src/tree.rs](git/grid-tree-rs/src/tree.rs)
- Existing persistence tests: [crates/server/tests/voxel_persistence.rs](crates/server/tests/voxel_persistence.rs)
- Integration test harness: [crates/server/tests/integration.rs:25-173](crates/server/tests/integration.rs#L25-L173)
- Lightyear message patterns: [crates/protocol/src/lib.rs:159-193](crates/protocol/src/lib.rs#L159-L193)
