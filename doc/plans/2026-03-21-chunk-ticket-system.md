# Chunk Ticket System Implementation Plan

## Overview

Replace the flat `ChunkTarget` + `HashSet<IVec3> loaded_chunks` model with a Minecraft-inspired ticket system. Tickets produce per-column load levels via 2D Chebyshev propagation using incremental bucket-queue BFS. Chunks transition through load states (EntityTicking → BlockTicking → Border → Inaccessible → Unloaded) based on their effective level. The existing 3D octree, persistence, and mesh entity systems remain — only the loading decision layer changes from binary set-membership to level-based. The client-request chunk protocol is replaced with server-push.

## Current State Analysis

The engine uses `ChunkTarget` components (attached to players/NPCs) with a `distance: u32` field to produce a cubic `HashSet<IVec3>` of desired positions per map. All desired chunks are equally "loaded" — no priority differentiation. Chunks outside the set are evicted. Generation spawns up to 32 tasks/frame in arbitrary `HashSet` iteration order.

### Key Discoveries:
- `ChunkTarget` defined at `chunk.rs:12-16`, used in 6 source files, 5 test files, 3 examples
- `loaded_chunks: HashSet<IVec3>` at `instance.rs:31`, referenced in ~15 files
- `update_chunks` (`lifecycle.rs:93-156`) is the scheduling brain — caches per-target desired sets, unions them per map, evicts out-of-range, spawns generation tasks
- `compute_target_desired` (`lifecycle.rs:226-240`) produces a 3D cubic volume `[-dist..=dist]` on all axes
- `spawn_missing_chunks` (`lifecycle.rs:291-314`) iterates `HashSet` in arbitrary order with `MAX_TASKS_PER_FRAME=32` cap
- No polling cap — `poll_chunk_tasks` and `poll_remesh_tasks` consume all ready tasks per frame
- No remesh spawning cap — `spawn_remesh_tasks` drains `chunks_needing_remesh` entirely
- Client uses request-pull model: `ChunkRequest`/`ChunkDataSync` in `protocol/src/map/chunk.rs`

## Desired End State

After this plan:
- `ChunkTarget` is fully replaced by `ChunkTicket` across the entire codebase
- Each map entity owns a `TicketLevelPropagator` that computes per-column effective levels via incremental bucket-queue BFS
- `VoxelMapInstance.loaded_chunks: HashSet<IVec3>` is replaced by `VoxelMapInstance.chunk_levels: HashMap<IVec2, u32>` (contains only loaded columns: level ≤ `LOAD_LEVEL_THRESHOLD`)
- Load/unload decisions use level thresholds instead of set membership
- Generation tasks spawn for columns whose level enters the loaded range, expanding to 3D `IVec3` positions via column height
- Server pushes chunk data to clients proactively — `ChunkRequest` and `request_missing_chunks` removed
- All existing tests pass with updated assertions
- New unit tests validate propagator correctness (single ticket, overlapping tickets, ticket removal, incremental updates)

### Verification:
- `cargo check-all` passes
- `cargo test-all` passes (existing + new tests)
- `cargo server` runs — chunks load around player
- `cargo client -c 1` runs — client receives chunks from server
- Visual: chunks load in concentric rings around player (nearest first)

## What We're NOT Doing

- **Priority scheduling / work caps** — separate step (Step 2 in the research doc)
- **Multi-stage generation pipeline** — separate step (Step 3)
- **Time-based work budgets** — separate step (Step 2)
- **Batched async tasks** — separate step (Step 2)
- **Ticket expiration/lifetime** — deferred; all tickets are permanent for now
- **Forced/Portal/Spawn ticket types** — only Player, NPC, MapTransition needed now

---

## Phase 1: Data Model

### Overview
Define all new types and modify `VoxelMapInstance` to use column-based level tracking. No behavior changes yet — existing systems continue to work because we add new fields alongside old ones, then migrate in Phase 3.

### Changes Required:

#### 1. New ticket module
**File**: `crates/voxel_map_engine/src/ticket.rs` (new)

```rust
use bevy::prelude::*;

/// The ticket type determines base level and semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TicketType {
    /// Full simulation around players. Base level 0.
    Player,
    /// NPC simulation. Base level 1.
    Npc,
    /// Temporary ticket for pre-loading destination during map transitions. Base level 2.
    MapTransition,
}

impl TicketType {
    /// The base load level for this ticket type. Lower = stronger.
    pub fn base_level(self) -> u32 {
        match self {
            TicketType::Player => 0,
            TicketType::Npc => 1,
            TicketType::MapTransition => 2,
        }
    }

    /// The default Chebyshev radius for this ticket type.
    pub fn default_radius(self) -> u32 {
        match self {
            TicketType::Player => 10,
            TicketType::Npc => 1,
            TicketType::MapTransition => 4,
        }
    }
}

/// Attach to entities whose `GlobalTransform` drives chunk loading for a specific map.
/// Replaces `ChunkTarget`. Local-only — not replicated over the network.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ChunkTicket {
    /// Which map this ticket loads chunks for.
    pub map_entity: Entity,
    /// Ticket type determines the base load level.
    pub ticket_type: TicketType,
    /// Radius in chunks (Chebyshev 2D) that this ticket influences.
    /// Effective level at distance d = base_level + d.
    pub radius: u32,
}

impl ChunkTicket {
    pub fn new(map_entity: Entity, ticket_type: TicketType, radius: u32) -> Self {
        debug_assert!(
            map_entity != Entity::PLACEHOLDER,
            "ChunkTicket::new called with Entity::PLACEHOLDER"
        );
        Self {
            map_entity,
            ticket_type,
            radius,
        }
    }

    /// Player ticket with default radius (10).
    pub fn player(map_entity: Entity) -> Self {
        Self::new(map_entity, TicketType::Player, TicketType::Player.default_radius())
    }

    /// NPC ticket with default radius (1).
    pub fn npc(map_entity: Entity) -> Self {
        Self::new(map_entity, TicketType::Npc, TicketType::Npc.default_radius())
    }

    /// Map transition ticket with default radius (4).
    pub fn map_transition(map_entity: Entity) -> Self {
        Self::new(map_entity, TicketType::MapTransition, TicketType::MapTransition.default_radius())
    }
}

/// A chunk column's load state, derived from its effective level.
///
/// This plan uses `LOAD_LEVEL_THRESHOLD` for loaded/unloaded decisions.
/// `LoadState` variants are used for debug display and will drive
/// simulation zone differentiation in a future plan.
// %% [SUGGESTION] Elegance — `PartialOrd, Ord` are derived but unused in this plan (LoadState is only used for debug display and `from_level`/`max_level` conversions). Consider deferring these derives until simulation zone differentiation needs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LoadState {
    /// Level 0: Full simulation — entity AI, physics, spawning.
    EntityTicking,
    /// Level 1: NPC simulation only, player entities frozen.
    BlockTicking,
    /// Level 2: Data loaded, meshed, no simulation. Available for neighbor padding.
    Border,
    /// Level 3+: Generation in progress, not accessible for gameplay.
    Inaccessible,
}

impl LoadState {
    /// Derive load state from an effective level.
    pub fn from_level(level: u32) -> Self {
        match level {
            0 => LoadState::EntityTicking,
            1 => LoadState::BlockTicking,
            2 => LoadState::Border,
            _ => LoadState::Inaccessible,
        }
    }

    /// The maximum level that produces this load state.
    pub fn max_level(self) -> u32 {
        match self {
            LoadState::EntityTicking => 0,
            LoadState::BlockTicking => 1,
            LoadState::Border => 2,
            LoadState::Inaccessible => u32::MAX,
        }
    }
}

/// The threshold level at or below which a column is considered "loaded"
/// (data in octree, mesh spawned). Columns above this level are unloaded.
///
/// Border (level 2) is the weakest loaded state — chunks at Border have data
/// and meshes but no simulation.
pub const LOAD_LEVEL_THRESHOLD: u32 = 2;

/// Maximum level value. Columns beyond this are not tracked by the propagator.
pub const MAX_LEVEL: u32 = 64;

/// Default column height range: 16 chunks vertically (Y range −8 to 7, exclusive upper bound).
pub const DEFAULT_COLUMN_Y_MIN: i32 = -8;
pub const DEFAULT_COLUMN_Y_MAX: i32 = 8;

/// Expand a 2D column position to all 3D chunk positions in the column.
/// Uses exclusive upper bound: `y_min..y_max`.
pub fn column_to_chunks(col: IVec2, y_min: i32, y_max: i32) -> impl Iterator<Item = IVec3> {
    (y_min..y_max).map(move |y| IVec3::new(col.x, y, col.y))
}

/// Convert a 3D chunk position to its 2D column (drop Y).
pub fn chunk_to_column(chunk_pos: IVec3) -> IVec2 {
    IVec2::new(chunk_pos.x, chunk_pos.z)
}
```

#### 2. Register module and re-export
**File**: `crates/voxel_map_engine/src/lib.rs`
**Changes**: Add `pub mod ticket;` and add `pub use crate::ticket::*;` to prelude.

#### 3. Add `chunk_levels` to `VoxelMapInstance`
**File**: `crates/voxel_map_engine/src/instance.rs`
**Changes**: Add `chunk_levels: HashMap<IVec2, u32>` field alongside existing `loaded_chunks`. Both coexist temporarily — Phase 3 removes `loaded_chunks`.

```rust
use std::collections::HashMap;

pub struct VoxelMapInstance {
    pub tree: OctreeI32<Option<ChunkData>>,
    pub loaded_chunks: HashSet<IVec3>,       // kept temporarily, removed in Phase 3
    pub chunk_levels: HashMap<IVec2, u32>,   // NEW: effective level per loaded column (level ≤ LOAD_LEVEL_THRESHOLD only)
    pub dirty_chunks: HashSet<IVec3>,
    pub chunks_needing_remesh: HashSet<IVec3>,
    pub debug_colors: bool,
}
```

Initialize `chunk_levels: HashMap::new()` in `VoxelMapInstance::new()`.

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` passes
- [x] `cargo test-all` passes (all existing tests unchanged)

#### Manual Verification:
- [x] New types are accessible from the prelude

### Tests:

#### `crates/voxel_map_engine/src/ticket.rs` (unit tests)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_type_base_levels() {
        assert_eq!(TicketType::Player.base_level(), 0);
        assert_eq!(TicketType::Npc.base_level(), 1);
        assert_eq!(TicketType::MapTransition.base_level(), 2);
    }

    #[test]
    fn ticket_type_default_radii() {
        assert_eq!(TicketType::Player.default_radius(), 10);
        assert_eq!(TicketType::Npc.default_radius(), 1);
        assert_eq!(TicketType::MapTransition.default_radius(), 4);
    }

    #[test]
    fn load_state_from_level() {
        assert_eq!(LoadState::from_level(0), LoadState::EntityTicking);
        assert_eq!(LoadState::from_level(1), LoadState::BlockTicking);
        assert_eq!(LoadState::from_level(2), LoadState::Border);
        assert_eq!(LoadState::from_level(3), LoadState::Inaccessible);
        assert_eq!(LoadState::from_level(100), LoadState::Inaccessible);
    }

    #[test]
    fn column_to_chunks_produces_correct_range() {
        let col = IVec2::new(3, 5);
        let chunks: Vec<IVec3> = column_to_chunks(col, -2, 2).collect();
        assert_eq!(chunks.len(), 4); // -2, -1, 0, 1 (exclusive upper)
        assert_eq!(chunks[0], IVec3::new(3, -2, 5));
        assert_eq!(chunks[3], IVec3::new(3, 1, 5));
    }

    #[test]
    fn chunk_to_column_drops_y() {
        assert_eq!(chunk_to_column(IVec3::new(1, 99, 2)), IVec2::new(1, 2));
    }

    /// Create a dummy entity for tests (not PLACEHOLDER, which triggers debug_assert).
    fn test_entity() -> Entity {
        Entity::from_raw_u32(999).expect("valid test entity")
    }

    #[test]
    fn convenience_constructors_use_default_radii() {
        let e = test_entity();
        let p = ChunkTicket::player(e);
        assert_eq!(p.ticket_type, TicketType::Player);
        assert_eq!(p.radius, 10);
        let n = ChunkTicket::npc(e);
        assert_eq!(n.ticket_type, TicketType::Npc);
        assert_eq!(n.radius, 1);
        let t = ChunkTicket::map_transition(e);
        assert_eq!(t.ticket_type, TicketType::MapTransition);
        assert_eq!(t.radius, 4);
    }

    #[test]
    fn new_allows_custom_radius() {
        let e = test_entity();
        let t = ChunkTicket::new(e, TicketType::Player, 20);
        assert_eq!(t.radius, 20);
    }
}
```

---

## Phase 2: Level Propagation

### Overview
Implement `TicketLevelPropagator` — an incremental bucket-queue BFS that computes per-column effective levels from all tickets on a map. This is a pure data structure with no ECS dependencies, fully unit-testable.

### Changes Required:

#### 1. Propagator implementation
**File**: `crates/voxel_map_engine/src/propagator.rs` (new)

```rust
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::ticket::{LOAD_LEVEL_THRESHOLD, MAX_LEVEL};

/// Incremental bucket-queue BFS level propagator.
///
/// Computes per-column effective levels from ticket sources.
/// Propagation uses 2D Chebyshev distance (8-neighbor grid).
/// Updates are incremental: adding/removing/moving a ticket only
/// recomputes the affected region, not the entire map.
///
/// Stored as a component on map entities.
#[derive(Component)]
pub struct TicketLevelPropagator {
    /// Current effective level per column. Contains ALL columns within any
    /// ticket's radius (including levels > LOAD_LEVEL_THRESHOLD).
    /// Consumers filter by threshold when syncing to `chunk_levels`.
    levels: HashMap<IVec2, u32>,
    /// Active ticket sources.
    sources: HashMap<Entity, TicketSource>,
    /// Pending BFS updates bucketed by level. Index = level.
    /// Each bucket holds columns that need their level recalculated
    /// starting from that level.
    pending_by_level: Vec<HashSet<IVec2>>,
    // %% [SUGGESTION] Elegance — this allocates MAX_LEVEL+1=65 HashSets upfront, most empty. A `BTreeMap<u32, HashSet<IVec2>>` would only allocate for active levels while preserving ordered iteration. Alternatively, a flat `Vec<(u32, IVec2)>` sorted by level would avoid per-bucket allocation entirely.
    /// Lowest non-empty bucket index for O(1) access to highest-priority work.
    min_pending_level: usize,
    /// Whether any sources changed since last propagate().
    dirty: bool,
}

/// A ticket source contributing to level computation.
#[derive(Clone, Debug)]
struct TicketSource {
    column: IVec2,
    base_level: u32,
    radius: u32,
}

/// Diff produced by a propagation step, classified by LOAD_LEVEL_THRESHOLD.
///
/// Classification rules:
/// - `loaded`: column was absent or had level > threshold, now has level ≤ threshold
/// - `changed`: column had level ≤ threshold before AND after, but level value changed
/// - `unloaded`: column had level ≤ threshold, now absent or has level > threshold
///
/// Columns that remain above threshold (Inaccessible) in both old and new states
/// do not appear in any diff category — they are invisible to consumers.
#[derive(Debug, Default)]
pub struct LevelDiff {
    /// Columns that entered the loaded range (level ≤ LOAD_LEVEL_THRESHOLD).
    pub loaded: Vec<(IVec2, u32)>,
    /// Columns whose level changed but remained in the loaded range.
    pub changed: Vec<(IVec2, u32)>,
    /// Columns that left the loaded range (should be unloaded).
    pub unloaded: Vec<IVec2>,
}
```

The propagator exposes these methods:

```rust
impl TicketLevelPropagator {
    pub fn new() -> Self { ... }

    /// Set or update a ticket source. Marks dirty for recompute.
    pub fn set_source(&mut self, entity: Entity, column: IVec2, base_level: u32, radius: u32) { ... }

    /// Remove a ticket source. Marks dirty for recompute.
    pub fn remove_source(&mut self, entity: Entity) { ... }

    /// Run incremental propagation. Returns the diff of level changes
    /// classified by LOAD_LEVEL_THRESHOLD.
    pub fn propagate(&mut self) -> LevelDiff { ... }

    /// Get the current effective level for a column. None = not tracked.
    pub fn get_level(&self, col: IVec2) -> Option<u32> { ... }

    /// Get the full levels map (read-only). Includes all levels, not just loaded.
    pub fn levels(&self) -> &HashMap<IVec2, u32> { ... }

    /// Check if any sources have changed since last propagate().
    pub fn is_dirty(&self) -> bool { ... }
}
```

**Incremental bucket-queue BFS propagation algorithm:**

The propagator uses Minecraft's `LevelPropagator` pattern: an array of `HashSet`s indexed by level, processed lowest-first.

**On `set_source` (add or move a ticket):**
1. If updating an existing source, queue invalidation of old region (see removal).
2. Store the new source.
3. For each column within the source's Chebyshev radius, compute `candidate_level = base_level + max(|dx|, |dz|)`.
4. If `candidate_level < current_level_or_MAX`, queue the column into `pending_by_level[candidate_level]`.
5. Update `min_pending_level`.

**On `remove_source`:**
1. Remove the source from the map.
2. For each column the removed source influenced (within its radius), queue recalculation: insert into `pending_by_level` at the column's current level.
3. Update `min_pending_level`.

**On `propagate`:**
1. If not dirty, return empty diff.
2. Save a snapshot of old levels (only columns that were ≤ threshold, for diff classification).
3. Process pending updates:
   - Starting from `min_pending_level`, iterate buckets.
   - For each column in the bucket, recompute its effective level by scanning all sources: `min { source.base_level + chebyshev(source.column, col) }` for sources where the column is within radius.
   - If the recomputed level differs from the stored level, update `levels` and queue 8 Chebyshev neighbors into appropriate buckets (since their levels may now change).
   - If recomputed level would be > MAX_LEVEL or no source covers this column, remove from `levels`.
4. Build `LevelDiff` by comparing old snapshot to new levels:
   - **loaded**: was absent or > threshold, now ≤ threshold
   - **changed**: was ≤ threshold and still ≤ threshold, but level value changed
   - **unloaded**: was ≤ threshold, now absent or > threshold
5. Clear dirty flag, reset pending buckets.

**Amortization**: The `propagate` method processes all pending updates to completion. For a player ticket with radius 10, a move across one chunk boundary invalidates and recomputes ~40 columns on the boundary (the ring that enters/exits range). This is fast enough to run in a single frame. If future profiling shows spikes (e.g., teleportation), add a `max_steps` parameter to spread work across frames.

#### 2. Register module
**File**: `crates/voxel_map_engine/src/lib.rs`
**Changes**: Add `pub mod propagator;` and `pub use crate::propagator::*;` to prelude.

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` passes
- [x] `cargo test-all` passes
- [x] All propagator unit tests pass

### Tests:

#### `crates/voxel_map_engine/src/propagator.rs` (unit tests)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::LOAD_LEVEL_THRESHOLD;

    /// Distinct entities needed for multi-source tests — can't use PLACEHOLDER for all.
    fn entity(id: u32) -> Entity {
        Entity::from_raw(id)
    }
    // %% [VIOLATION] Coherence — `Entity::from_raw(id)` does not exist in Bevy 0.18. The API is `Entity::from_raw_u32(id)` which returns `Option<Entity>`. Phase 1 tests correctly use `Entity::from_raw_u32(999).expect("valid test entity")`. This should be `Entity::from_raw_u32(id).expect("valid test entity")`.

    #[test]
    fn single_player_ticket_produces_concentric_levels() {
        let mut prop = TicketLevelPropagator::new();
        prop.set_source(entity(1), IVec2::ZERO, 0, 5);
        let diff = prop.propagate();

        // Center column should be level 0
        assert_eq!(prop.get_level(IVec2::ZERO), Some(0));
        // 1 step away: level 1
        assert_eq!(prop.get_level(IVec2::new(1, 0)), Some(1));
        assert_eq!(prop.get_level(IVec2::new(1, 1)), Some(1)); // Chebyshev
        // 2 steps away: level 2
        assert_eq!(prop.get_level(IVec2::new(2, 0)), Some(2));
        // 5 steps away: level 5
        assert_eq!(prop.get_level(IVec2::new(5, 0)), Some(5));
        // 6 steps away: outside radius, not tracked
        assert_eq!(prop.get_level(IVec2::new(6, 0)), None);

        // Columns at level ≤ LOAD_LEVEL_THRESHOLD appear in diff.loaded
        let loaded_count = diff.loaded.len();
        // radius 5, threshold 2: loaded columns = (2*2+1)² = 25
        assert_eq!(loaded_count, 25);
    }

    #[test]
    fn npc_ticket_starts_at_level_1() {
        let mut prop = TicketLevelPropagator::new();
        prop.set_source(entity(1), IVec2::ZERO, 1, 3);
        prop.propagate();

        // Center: base_level=1 + distance=0 = 1
        assert_eq!(prop.get_level(IVec2::ZERO), Some(1));
        // 1 step: 1+1=2
        assert_eq!(prop.get_level(IVec2::X), Some(2));
        // 2 steps: 1+2=3
        assert_eq!(prop.get_level(IVec2::new(2, 0)), Some(3));
    }

    #[test]
    fn overlapping_tickets_take_minimum_level() {
        let mut prop = TicketLevelPropagator::new();
        // Player at (0,0) level 0 radius 5
        prop.set_source(entity(1), IVec2::ZERO, 0, 5);
        // NPC at (3,0) level 1 radius 3
        prop.set_source(entity(2), IVec2::new(3, 0), 1, 3);
        prop.propagate();

        // Column (3,0): player contributes 0+3=3, NPC contributes 1+0=1 → min=1
        assert_eq!(prop.get_level(IVec2::new(3, 0)), Some(1));
        // Column (0,0): player contributes 0, NPC contributes 1+3=4 → min=0
        assert_eq!(prop.get_level(IVec2::ZERO), Some(0));
        // Column (2,0): player contributes 0+2=2, NPC contributes 1+1=2 → min=2
        assert_eq!(prop.get_level(IVec2::new(2, 0)), Some(2));
    }

    #[test]
    fn ticket_removal_unloads_exclusive_columns() {
        let mut prop = TicketLevelPropagator::new();
        prop.set_source(entity(1), IVec2::ZERO, 0, 2);
        prop.propagate();

        // (2,0) is at level 2 (loaded)
        assert_eq!(prop.get_level(IVec2::new(2, 0)), Some(2));

        prop.remove_source(entity(1));
        let diff = prop.propagate();

        // Everything should be removed
        assert_eq!(prop.get_level(IVec2::ZERO), None);
        // All 25 columns (5x5) were loaded, now unloaded
        assert_eq!(diff.unloaded.len(), 25);
    }

    #[test]
    fn ticket_move_updates_levels_incrementally() {
        let mut prop = TicketLevelPropagator::new();
        prop.set_source(entity(1), IVec2::ZERO, 0, 3);
        prop.propagate();

        assert_eq!(prop.get_level(IVec2::ZERO), Some(0));

        // Move ticket to (5,0) — no overlap with old position's loaded range
        prop.set_source(entity(1), IVec2::new(5, 0), 0, 3);
        let diff = prop.propagate();

        // Old center should be removed
        assert_eq!(prop.get_level(IVec2::ZERO), None);
        // New center should be level 0
        assert_eq!(prop.get_level(IVec2::new(5, 0)), Some(0));

        // Diff should contain both loads and unloads
        assert!(!diff.loaded.is_empty());
        assert!(!diff.unloaded.is_empty());
    }

    #[test]
    fn ticket_move_one_step_has_minimal_diff() {
        let mut prop = TicketLevelPropagator::new();
        prop.set_source(entity(1), IVec2::ZERO, 0, 3);
        prop.propagate();

        // Move one chunk east
        prop.set_source(entity(1), IVec2::X, 0, 3);
        let diff = prop.propagate();

        // Only the boundary columns should change, not the entire area
        // The bulk of the loaded area overlaps — most columns stay loaded
        let total_changes = diff.loaded.len() + diff.changed.len() + diff.unloaded.len();
        // Full area is 7x7=49 columns. A 1-step move should affect much fewer.
        assert!(total_changes < 49, "expected incremental diff, got {total_changes} changes");
    }

    #[test]
    fn no_propagation_when_clean() {
        let mut prop = TicketLevelPropagator::new();
        prop.set_source(entity(1), IVec2::ZERO, 0, 2);
        prop.propagate();

        // Second propagate without changes
        let diff = prop.propagate();
        assert!(diff.loaded.is_empty());
        assert!(diff.changed.is_empty());
        assert!(diff.unloaded.is_empty());
    }

    #[test]
    fn chebyshev_distance_correct_for_diagonals() {
        let mut prop = TicketLevelPropagator::new();
        prop.set_source(entity(1), IVec2::ZERO, 0, 5);
        prop.propagate();

        // Diagonal (3,3): Chebyshev distance = max(3,3) = 3, so level = 0+3 = 3
        assert_eq!(prop.get_level(IVec2::new(3, 3)), Some(3));
        // (2,4): Chebyshev = max(2,4) = 4
        assert_eq!(prop.get_level(IVec2::new(2, 4)), Some(4));
    }

    #[test]
    fn diff_classifies_by_load_threshold() {
        let mut prop = TicketLevelPropagator::new();
        // Player at origin, radius 3. Loaded columns: level 0,1,2 (distance ≤ 2)
        prop.set_source(entity(1), IVec2::ZERO, 0, 3);
        let diff = prop.propagate();

        // First propagate: loaded columns are those at level ≤ 2 (distance ≤ 2)
        // That's a 5x5 square = 25 columns
        assert_eq!(diff.loaded.len(), 25);
        assert!(diff.changed.is_empty());
        assert!(diff.unloaded.is_empty());

        // Columns at distance 3 (level 3) should NOT be in diff.loaded
        assert!(!diff.loaded.iter().any(|(col, _)| *col == IVec2::new(3, 0)));
        // But they should exist in the propagator's internal levels
        assert_eq!(prop.get_level(IVec2::new(3, 0)), Some(3));
    }

    #[test]
    fn overlapping_ticket_strengthens_column_produces_changed() {
        let mut prop = TicketLevelPropagator::new();
        prop.set_source(entity(1), IVec2::ZERO, 0, 3);
        prop.propagate();

        // Column (2,0) is level 2 (Border, loaded)
        assert_eq!(prop.get_level(IVec2::new(2, 0)), Some(2));

        // Add NPC at (2,0) with base_level=0 — strengthens column to level 0
        prop.set_source(entity(2), IVec2::new(2, 0), 0, 2);
        let diff = prop.propagate();

        // (2,0) changed from level 2 to level 0 — both ≤ threshold → "changed"
        assert!(diff.changed.iter().any(|(col, lvl)| *col == IVec2::new(2, 0) && *lvl == 0));
    }

    #[test]
    fn column_crossing_threshold_is_loaded_or_unloaded() {
        let mut prop = TicketLevelPropagator::new();
        // NPC (base_level=1) at origin, radius 3
        prop.set_source(entity(1), IVec2::ZERO, 1, 3);
        prop.propagate();

        // Distance 2: level = 1+2 = 3 (above threshold) — not in diff.loaded
        assert_eq!(prop.get_level(IVec2::new(2, 0)), Some(3));

        // Add player (base_level=0) at same position — strengthens distance-2 to level 2
        prop.set_source(entity(2), IVec2::ZERO, 0, 3);
        let diff = prop.propagate();

        // (2,0) went from level 3 (above threshold) to level 2 (at threshold) → "loaded"
        assert!(diff.loaded.iter().any(|(col, _)| *col == IVec2::new(2, 0)));
    }

    #[test]
    fn large_radius_column_count() {
        let mut prop = TicketLevelPropagator::new();
        prop.set_source(entity(1), IVec2::ZERO, 0, 10);
        prop.propagate();

        // Chebyshev radius 10: (2*10+1)² = 441 columns total
        let count = prop.levels().len();
        assert_eq!(count, 441);
    }

    #[test]
    fn removal_with_overlapping_ticket_preserves_shared_columns() {
        let mut prop = TicketLevelPropagator::new();
        prop.set_source(entity(1), IVec2::ZERO, 0, 3);
        prop.set_source(entity(2), IVec2::new(2, 0), 0, 3);
        prop.propagate();

        // Remove entity 1. Column (0,0) was level 0 from entity 1.
        // Entity 2 at (2,0) contributes level 0+2=2 to (0,0).
        prop.remove_source(entity(1));
        let diff = prop.propagate();

        // (0,0) should still be loaded at level 2 (from entity 2)
        assert_eq!(prop.get_level(IVec2::ZERO), Some(2));
        // It changed from level 0 to level 2 → "changed" (both ≤ threshold)
        assert!(diff.changed.iter().any(|(col, _)| *col == IVec2::ZERO));
        // Columns exclusively covered by entity 1 should be unloaded
        assert!(diff.unloaded.iter().any(|col| *col == IVec2::new(-3, 0)));
    }
}
```

---

## Phase 3: Lifecycle Rewrite

### Overview
Replace the internals of the lifecycle system chain to use tickets and the propagator instead of `ChunkTarget` and `HashSet<IVec3>` desired sets. This is the largest phase — it touches the core scheduling logic.

### Key Design Decision: `chunk_levels` Scope

`VoxelMapInstance.chunk_levels` stores **only loaded columns** (effective level ≤ `LOAD_LEVEL_THRESHOLD`). The propagator internally tracks all levels, but when syncing to `chunk_levels`, only loaded columns are inserted. This means:
- `spawn_missing_chunks` can iterate `chunk_levels` without threshold guards
- `despawn_out_of_range_chunks` uses `chunk_levels.contains_key()` directly
- `poll_remesh_tasks` uses `chunk_levels.contains_key()` directly
- `diff.unloaded` columns are removed from `chunk_levels`
- `diff.loaded` columns are inserted into `chunk_levels`

### Changes Required:

#### 1. New system: `collect_tickets`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Replaces `purge_stale_targets` + `update_target_caches_for_map`. Reads all `ChunkTicket` components, updates each map's `TicketLevelPropagator` sources.

```rust
/// Collect all ChunkTicket components and update each map's propagator sources.
fn collect_tickets(
    map_query: &Query<&GlobalTransform, With<VoxelMapInstance>>,
    ticket_query: &Query<(Entity, &ChunkTicket, &GlobalTransform)>,
    propagators: &mut Query<&mut TicketLevelPropagator>,
    // Passed from update_chunks's Local<HashMap<Entity, CachedTicket>> —
    // collect_tickets is a helper, not a standalone system.
    ticket_cache: &mut HashMap<Entity, CachedTicket>,
) {
    let _span = info_span!("collect_tickets").entered();
    // 1. Detect removed tickets: any entity in cache not in ticket_query → remove_source
    let active: HashSet<Entity> = ticket_query.iter().map(|(e, _, _)| e).collect();
    // %% [SUGGESTION] Elegance — allocating a HashSet of all active entities every frame just to detect removals is wasteful. Instead, mark each cache entry during the update loop below, then sweep unmarked entries. This avoids the per-frame allocation.
    let stale: Vec<Entity> = ticket_cache.keys().filter(|e| !active.contains(e)).copied().collect();
    for entity in stale {
        if let Some(cached) = ticket_cache.remove(&entity) {
            if let Ok(mut prop) = propagators.get_mut(cached.map_entity) {
                prop.remove_source(entity);
            }
        }
    }

    // 2. For each ticket:
    //    a. Transform GlobalTransform to map-local space
    //    b. Compute 2D column position (drop Y)
    //    c. Compare against cache — if column, type, or radius changed, call set_source
    for (ticket_entity, ticket, transform) in ticket_query.iter() {
        let Ok(map_transform) = map_query.get(ticket.map_entity) else {
            trace!(
                "collect_tickets: ticket {ticket_entity:?} references non-existent map {:?}, \
                 expected during deferred command application",
                ticket.map_entity
            );
            continue;
        };
        let map_inv = map_transform.affine().inverse();
        let local_pos = map_inv.transform_point3(transform.translation());
        let column = world_to_column_pos(local_pos);

        let needs_update = match ticket_cache.get(&ticket_entity) {
            Some(cached) => {
                cached.column != column
                    || cached.map_entity != ticket.map_entity
                    || cached.ticket_type != ticket.ticket_type
                    || cached.radius != ticket.radius
            }
            None => true,
        };

        if needs_update {
            if let Ok(mut prop) = propagators.get_mut(ticket.map_entity) {
                prop.set_source(
                    ticket_entity,
                    column,
                    ticket.ticket_type.base_level(),
                    ticket.radius,
                );
            }
            ticket_cache.insert(ticket_entity, CachedTicket {
                column,
                map_entity: ticket.map_entity,
                ticket_type: ticket.ticket_type,
                radius: ticket.radius,
            });
        }
    }
}
```

`CachedTicket` replaces `TargetCache`:
```rust
struct CachedTicket {
    column: IVec2,
    map_entity: Entity,
    ticket_type: TicketType,
    radius: u32,
}
```

Helper:
```rust
fn world_to_column_pos(translation: Vec3) -> IVec2 {
    let chunk = (translation / CHUNK_SIZE as f32).floor().as_ivec3();
    IVec2::new(chunk.x, chunk.z)
}
```

#### 2. Modify `update_chunks`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

The main `update_chunks` system changes its query to use `ChunkTicket` instead of `ChunkTarget`. Flow becomes:

1. Collect tickets → update propagator sources (`info_span!("collect_tickets")`)
2. For each map: call `propagator.propagate()` to get `LevelDiff` (`info_span!("propagate_ticket_levels")`, `plot!("bfs_steps_this_frame")`, `plot!("propagator_dirty_columns")`)
3. Sync `chunk_levels` on `VoxelMapInstance` from diff:
   - For `diff.loaded` columns: insert into `chunk_levels` with their level
   - For `diff.changed` columns: update level in `chunk_levels`
   - For `diff.unloaded` columns: remove from `chunk_levels`, expand to 3D, evict data
4. `spawn_missing_chunks` iterates `chunk_levels` — all entries are loaded columns

System signature changes:
```rust
pub fn update_chunks(
    mut map_query: Query<(
        Entity,
        &mut VoxelMapInstance,
        &VoxelMapConfig,
        &VoxelGenerator,
        &mut PendingChunks,
        &mut TicketLevelPropagator,
        &GlobalTransform,
    )>,
    ticket_query: Query<(Entity, &ChunkTicket, &GlobalTransform)>,
    map_transforms: Query<&GlobalTransform, With<VoxelMapInstance>>,
    mut tick: Local<u32>,
    mut ticket_cache: Local<HashMap<Entity, CachedTicket>>,
)
```

#### 3. Modify `remove_out_of_range_chunks`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Changes from set-membership check to level-based. Operates on `diff.unloaded` columns:
```rust
fn remove_out_of_range_chunks(
    instance: &mut VoxelMapInstance,
    unloaded_columns: &[IVec2],
    save_dir: Option<&std::path::Path>,
    y_min: i32,
    y_max: i32,
) {
    let _span = info_span!("remove_out_of_range_chunks").entered();
    for &col in unloaded_columns {
        for chunk_pos in column_to_chunks(col, y_min, y_max) {
            if instance.dirty_chunks.remove(&chunk_pos) {
                if let Some(dir) = save_dir {
                    if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
                        // Spawn async save task to avoid blocking the main thread.
                        // Clone data before removal below; fire-and-forget.
                        let data = chunk_data.clone();
                        let dir = dir.to_path_buf();
                        // Step 2: Push to pending save queue instead of spawning task here
                        AsyncComputeTaskPool::get().spawn(async move {
                            if let Err(e) = crate::persistence::save_chunk(&dir, chunk_pos, &data) {
                                error!("Failed to save evicted dirty chunk at {chunk_pos}: {e}");
                            }
                        }).detach();
                    }
                }
            }
            instance.remove_chunk_data(chunk_pos);
        }
        instance.chunk_levels.remove(&col);
    }
}
```

#### 4. Modify `spawn_missing_chunks`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Iterates `chunk_levels` (which only contains loaded columns), expands to 3D positions, spawns gen tasks for positions not yet in the octree:
```rust
fn spawn_missing_chunks(
    instance: &VoxelMapInstance,
    pending: &mut PendingChunks,
    config: &VoxelMapConfig,
    generator: &VoxelGenerator,
    y_min: i32,
    y_max: i32,
) {
    let _span = info_span!("spawn_missing_chunks").entered();
    let mut spawned = 0;
    // Sort by level (lowest first) so nearest columns generate before distant ones.
    // This implements concentric-ring loading for free.
    let mut cols: Vec<_> = instance.chunk_levels.iter().collect();
    cols.sort_by_key(|(_, &lvl)| lvl);
    for (&col, &_level) in cols {
        // chunk_levels only contains loaded columns (level ≤ LOAD_LEVEL_THRESHOLD),
        // so no threshold filter needed here.
        for chunk_pos in column_to_chunks(col, y_min, y_max) {
            if spawned >= MAX_TASKS_PER_FRAME { return; }
            if instance.get_chunk_data(chunk_pos).is_some() { continue; }
            if pending.pending_positions.contains(&chunk_pos) { continue; }
            spawn_chunk_gen_task(pending, chunk_pos, generator, config.save_dir.clone());
            spawned += 1;
        }
    }
    plot!(plot_name!("gen_spawned_this_frame"), spawned as f64);
    plot!(plot_name!("gen_tasks_in_flight"), pending.tasks.len() as f64);
}
```

#### 5. Remove old target infrastructure
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Delete `compute_target_desired`, `union_desired_from_cache`, `TargetCache`, `purge_stale_targets`, `update_target_caches_for_map`, and the `desired_cache` local. The propagator replaces all of this.

#### 6. Modify `despawn_out_of_range_chunks`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Change from `loaded_chunks.contains` to `chunk_levels.contains_key`:
```rust
// chunk_levels only contains loaded columns, so this is a clean loaded check
if !instance.chunk_levels.contains_key(&chunk_to_column(chunk.position)) {
    commands.entity(entity).despawn();
}
```

#### 7. Modify `poll_remesh_tasks`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Change the loaded check from `loaded_chunks.contains` to `chunk_levels.contains_key(&chunk_to_column(...))`.

#### 8. Modify `poll_chunk_tasks` → `handle_completed_chunk`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Remove `instance.loaded_chunks.insert(result.position)` — loaded state is now determined by `chunk_levels`, not a separate set.

#### 9. Modify `ensure_pending_chunks`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Also auto-insert `TicketLevelPropagator` on map entities that lack one. Keep the `With<VoxelGenerator>` gate. `TicketLevelPropagator` should derive `Default` (delegating to `new()`) for consistency with `PendingChunks` and `PendingRemeshes`.

#### 10. Remove `loaded_chunks` from `VoxelMapInstance`
**File**: `crates/voxel_map_engine/src/instance.rs`

Remove the `loaded_chunks: HashSet<IVec3>` field entirely. All consumers now use `chunk_levels: HashMap<IVec2, u32>` (for column-level loaded checks) or `get_chunk_data(pos).is_some()` (for 3D chunk-level existence checks).

The `set_voxel` method (`instance.rs:134`) needs no change — it already guards on `get_chunk_data_mut(chunk_pos)` returning `None`.

#### 11. Remove `ChunkTarget` from `chunk.rs`
**File**: `crates/voxel_map_engine/src/chunk.rs`

Delete the `ChunkTarget` struct, its `impl`, and its test. Keep `VoxelChunk`.

#### 12. Update imports
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Replace `use crate::chunk::ChunkTarget` with `use crate::ticket::ChunkTicket` and add `use crate::propagator::TicketLevelPropagator` and `use crate::ticket::{chunk_to_column, column_to_chunks}`.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` passes
- [ ] All voxel_map_engine unit tests pass
- [ ] All voxel_map_engine integration tests pass (after Phase 4 updates)

#### Manual Verification:
- [ ] `cargo server` — chunks generate around player position
- [ ] `cargo client -c 1` — client receives and renders chunks

### Tests:

#### `crates/voxel_map_engine/tests/lifecycle.rs` (updated)

The test helper `spawn_target` becomes `spawn_ticket`:
```rust
fn spawn_ticket(world: &mut World, map_entity: Entity, ticket_type: TicketType, radius: u32) -> Entity {
    world.spawn((
        ChunkTicket::new(map_entity, ticket_type, radius),
        Transform::default(),
        GlobalTransform::default(),
    )).id()
}
```

All 9 existing tests adapted to use `ChunkTicket` and `chunk_levels` instead of `ChunkTarget` and `loaded_chunks`.

New tests:
```rust
#[test]
fn column_loading_expands_to_3d_chunks() {
    // A ticket at a position should cause chunks at multiple Y levels to generate.
    // Set up map with ticket radius 0 (only center column).
    // After ticking, verify that chunk data exists at multiple Y positions
    // in the column (e.g., IVec3(0, -8, 0) through IVec3(0, 7, 0)).
}

#[test]
fn ticket_level_affects_loaded_columns() {
    // Player ticket (level 0, radius 3):
    // Column at distance 0 → level 0 (EntityTicking, loaded)
    // Column at distance 2 → level 2 (Border, loaded)
    // Column at distance 3 → level 3 (Inaccessible, NOT in chunk_levels)
    // Verify that chunk_levels contains only the 25 columns at distance ≤ 2.
    // Verify that no chunk data exists for distance-3 columns.
}

#[test]
fn npc_ticket_produces_weaker_levels_than_player() {
    // NPC ticket (level 1, radius 1):
    // Center column → level 1 (BlockTicking)
    // Distance 1 → level 2 (Border)
    // Verify chunk_levels has these levels.
    // Player ticket (level 0, radius 1):
    // Center → level 0 (EntityTicking)
    // Verify player center has lower level than NPC center.
}

#[test]
fn unloaded_columns_evict_chunk_data() {
    // Set up map with ticket, tick until chunks load.
    // Move ticket far away so old columns exceed threshold.
    // Tick again. Verify old columns removed from chunk_levels
    // and chunk data removed from octree.
}
```

---

## Phase 4: Consumer Migration

### Overview
Update all consumers of `ChunkTarget` and `loaded_chunks` outside `voxel_map_engine` to use `ChunkTicket` and `chunk_levels`. The client keeps its `ChunkRequest` mechanism temporarily — Phase 5 replaces it with server-push.

### Changes Required:

#### 1. Server: player spawn
**File**: `crates/server/src/gameplay.rs:317`
```rust
// Before:
ChunkTarget::new(registry.get(&MapInstanceId::Overworld), 10)
// After:
ChunkTicket::player(registry.get(&MapInstanceId::Overworld))
```

#### 2. Server: NPC spawn
**File**: `crates/server/src/gameplay.rs:82`
```rust
// Before:
ChunkTarget::new(registry.get(&MapInstanceId::Overworld), 1)
// After:
let overworld = registry.get(&MapInstanceId::Overworld);
ChunkTicket::npc(overworld)
```

#### 3. Server: map transition
**File**: `crates/server/src/map.rs:805-806`
```rust
// Before:
ChunkTarget::new(map_entity, 10)
// After:
ChunkTicket::player(map_entity)
```

#### 4. Server: chunk request handler
**File**: `crates/server/src/map.rs:640-685`

Change `instance.loaded_chunks.contains(...)` checks to `instance.get_chunk_data(chunk_pos).is_some()` (data existence in octree). For the all-air fallback, check `instance.chunk_levels.contains_key(&chunk_to_column(chunk_pos))`.

This system is removed entirely in Phase 5.

#### 5. Client: auto-attach ticket
**File**: `crates/client/src/map.rs:117-132`
```rust
// Before:
ChunkTarget::new(map_entity, 10)
// After:
ChunkTicket::player(map_entity)
```

Query filter changes from `Without<ChunkTarget>` to `Without<ChunkTicket>`.

#### 6. Client: request_missing_chunks (interim)
**File**: `crates/client/src/map.rs:135-202`

Change `target: &ChunkTarget` to `ticket: &ChunkTicket` in the query. The desired set computation changes from a 3D cube using `target.distance` to a 2D column expansion using `ticket.radius`:

```rust
// Compute desired columns as 2D Chebyshev radius from player chunk position
let chunk_pos = world_to_chunk_pos(player_pos);
let col = IVec2::new(chunk_pos.x, chunk_pos.z);
let radius = ticket.radius as i32;

for dx in -radius..=radius {
    for dz in -radius..=radius {
        let target_col = col + IVec2::new(dx, dz);
        let distance = dx.abs().max(dz.abs()) as u32;
        let level = ticket.ticket_type.base_level() + distance;
        if level > LOAD_LEVEL_THRESHOLD { continue; }
        for chunk_pos in column_to_chunks(target_col, y_min, y_max) {
            if instance.get_chunk_data(chunk_pos).is_some() { continue; }
            if state.pending_requests.contains(&chunk_pos) { continue; }
            // send ChunkRequest
        }
    }
}
```

This is removed entirely in Phase 5.

#### 7. Client: handle_chunk_data_sync
**File**: `crates/client/src/map.rs:205-277`

Replace `instance.loaded_chunks.insert(chunk_pos)` with:
```rust
instance.chunk_levels.entry(chunk_to_column(chunk_pos)).or_insert(0);
```

#### 8. Client: voxel operation systems
**File**: `crates/client/src/map.rs` (multiple systems)

Systems that read `chunk_target.map_entity` switch to reading `chunk_ticket.map_entity`:
- `handle_voxel_broadcasts` (~line 280)
- `handle_section_blocks_update` (~line 320)
- `handle_voxel_input` (~line 347)
- `handle_voxel_edit_reject` (~line 453)

#### 9. Client: map transition
**File**: `crates/client/src/map.rs:523-526`
```rust
// Before:
ChunkTarget::new(map_entity, 4)
// After:
ChunkTicket::map_transition(map_entity)
```

#### 10. Client: check_transition_chunks_loaded
**File**: `crates/client/src/map.rs:583`

Change `instance.loaded_chunks.is_empty()` to `instance.chunk_levels.is_empty()`.

#### 11. Update imports across server and client
Replace all `ChunkTarget` imports with `ChunkTicket`.

#### 12. Update tests

**`crates/voxel_map_engine/tests/api.rs`** (all 9 tests):
- `spawn_target` helper → `spawn_ticket` using `ChunkTicket::player`
- `has_loaded_chunk` helper → check `instance.get_chunk_data(pos).is_some()` or `instance.chunk_levels.contains_key(&chunk_to_column(pos))`

**`crates/server/tests/integration.rs`**:
- Line 928: `ChunkTarget::new(overworld_map, 4)` → `ChunkTicket::new(overworld_map, TicketType::Player, 4)`
- Line 1340: `instance.loaded_chunks.insert(chunk_pos)` → `instance.chunk_levels.insert(chunk_to_column(chunk_pos), 0)` + `instance.insert_chunk_data(chunk_pos, ChunkData::new_empty())`
- Line 1452: same pattern

**`crates/server/tests/voxel_persistence.rs`**:
- Lines 17, 35, 90, 97, 108: `loaded_chunks.insert/remove/contains` → `chunk_levels.insert/remove/contains_key` with column conversion

**`crates/server/tests/world_persistence.rs`**:
- Line 114: `loaded_chunks.insert` → `chunk_levels.insert`

**`crates/client/tests/map_transition.rs`**:
- Line 58: `ChunkTarget::new(map, 0)` → `ChunkTicket::new(map, TicketType::Player, 0)`
- Lines 105-110: `loaded_chunks.insert` → `chunk_levels.insert`

**`crates/protocol/tests/physics_isolation.rs`**:
- Lines 170, 203, 256: `ChunkTarget { map_entity, distance }` → `ChunkTicket::new(target_map, TicketType::Player, 0)`
- Lines 186-196, 213-226: `loaded_chunks.len()` → `chunk_levels.len()`

#### 13. Update examples

**`crates/voxel_map_engine/examples/terrain.rs`**:
- Line 32: `ChunkTarget { map_entity, distance: 5 }` → `ChunkTicket::new(map_entity, TicketType::Player, 5)`

**`crates/voxel_map_engine/examples/editing.rs`**:
- Line 39: same pattern

**`crates/voxel_map_engine/examples/multi_instance.rs`**:
- Lines 37-40: `ChunkTarget { ... }` → `ChunkTicket::new(...)`
- Line 129: query `&mut ChunkTicket` instead of `&mut ChunkTarget`
- Line 167: `target.map_entity = map_entity` → `ticket.map_entity = map_entity`

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` passes
- [x] `cargo test-all` passes — all tests across all crates
- [ ] `cargo server` builds and runs
- [ ] `cargo client -c 1` builds and runs

#### Manual Verification:
- [ ] Server: chunks generate around player in concentric rings
- [ ] Client: receives chunks from server, renders correctly
- [ ] Map transition: client transitions between maps, loads chunks on new map
- [ ] Voxel editing: place/remove voxels works correctly
- [ ] Multiple maps: chunks isolated between overworld/homebase/arena
- [ ] NPC: dummy NPC generates chunks around itself

---

## Phase 5: Server-Push Networking

### Overview
Replace the client-request chunk protocol (`ChunkRequest`/`request_missing_chunks`) with server-push. The server monitors per-player chunk visibility from ticket levels and proactively sends chunk data. The client becomes a passive receiver — it never requests chunks.

This resolves the architectural mismatch where the client independently computed desired chunks. Now only the server runs the propagator and makes all loading decisions.

### Changes Implemented:

#### 1. `ChunkGenerationEnabled` resource gate
**File**: `crates/voxel_map_engine/src/lib.rs`

Added `ChunkGenerationEnabled` marker resource. `VoxelPlugin` gates `update_chunks` and `poll_chunk_tasks` behind `run_if(resource_exists::<ChunkGenerationEnabled>)`. The server inserts this resource; the client does not. This prevents the client-side propagator from running and fighting with server-pushed data.

Systems that still run on client (needed for mesh cleanup and voxel edit remeshing):
- `ensure_pending_chunks`
- `despawn_out_of_range_chunks`
- `spawn_remesh_tasks` / `poll_remesh_tasks`

#### 2. New server system: `push_chunks_to_clients`
**File**: `crates/server/src/map.rs`

Tracks sent state at the **individual chunk level** (IVec3), not column level. This is critical because chunk generation is async — chunks within a column generate at different times. Each frame, the system iterates all columns in the player's loaded range and sends any chunks that have data but haven't been sent yet. This allows newly generated chunks to be sent as they become available.

`ClientChunkVisibility` component tracks:
- `sent_chunks: HashSet<IVec3>` — individual chunks whose data has been sent
- `sent_columns: HashSet<IVec2>` — columns the client believes are loaded (for UnloadColumn)

When columns leave the loaded range, sends `UnloadColumn` and clears per-chunk tracking.

#### 3. New protocol message: `UnloadColumn`
**File**: `crates/protocol/src/map/chunk.rs`

Server → client message telling client to drop all chunks in a column. Registered on `ChunkChannel` as `ServerToClient`.

#### 4. `ChunkChannel` direction changed to `ServerToClient`
**File**: `crates/protocol/src/lib.rs`

Was `Bidirectional` (needed for `ChunkRequest`). Now `ServerToClient` only.

#### 5. `ClientChunkVisibility` inserted on player spawn
**File**: `crates/server/src/gameplay.rs`

#### 6. Removed `ChunkRequest`, `handle_chunk_requests`, `request_missing_chunks`, `ClientChunkState`
- `ChunkRequest` struct deleted from protocol
- `handle_chunk_requests` system deleted from server
- `request_missing_chunks` system deleted from client
- `ClientChunkState` component deleted from client
- All registrations and imports cleaned up

#### 7. `handle_unload_column` on client
**File**: `crates/client/src/map.rs`

Handles `UnloadColumn` messages by removing chunk data from octree and `chunk_levels` entry. Mesh entity cleanup handled by existing `despawn_out_of_range_chunks`. Gracefully handles missing player (during transitions) with trace + early return, draining messages to prevent queue buildup.

#### 8. `ChunkDataSyncBuffer` on client
**File**: `crates/client/src/map.rs`

Lightyear clears `MessageReceiver` buffers every frame in `Last`, regardless of whether `receive()` was called. Chunks arriving before the predicted player entity has a `ChunkTicket` would be lost. `handle_chunk_data_sync` drains messages into a `Local<ChunkDataSyncBuffer>` and replays them once the player is ready.

#### 9. Simplified client `handle_chunk_data_sync`
Removed `ClientChunkState`/`pending_requests` tracking. Added `ChunkDataSyncBuffer` parameter for early-arrival buffering.

#### 10. Updated tests
- Removed `test_client_requests_chunk_and_receives_data` from integration tests
- Removed `ClientChunkState` from `map_transition.rs` test helpers
- Added `ChunkGenerationEnabled` to all test apps and examples that use `VoxelPlugin`

### Bugs Found During Implementation:

1. **Client propagator fighting server push** — `VoxelPlugin` registered `update_chunks` on both server and client. On the client, the propagator called `remove_column_chunks` which deleted octree data and `chunk_levels` entries that the server had just pushed. Fixed by gating with `ChunkGenerationEnabled`.

2. **Lightyear message loss** — `MessageReceiver` is cleared every frame in `Last`. Chunks arriving before the predicted player had a `ChunkTicket` were silently dropped. Fixed with `ChunkDataSyncBuffer`.

3. **Column-level vs chunk-level tracking** — Original design tracked sent *columns* (IVec2), but chunk generation is async per-chunk (IVec3). A column marked "sent" when partially generated would never get remaining chunks re-sent. Fixed by tracking individual chunks.

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` passes
- [x] `cargo test-all` passes
- [x] `cargo server` builds and runs
- [x] `cargo client -c 1` builds and runs

#### Manual Verification:
- [x] Client receives chunks automatically upon connecting — no manual request
- [x] Walking to edge of loaded area: new chunks arrive from server automatically
- [ ] Map transition: chunks for new map arrive without client requesting
- [x] Verify no `ChunkRequest` references remain in codebase (grep)

---

## Testing Strategy

### Unit Tests (Phase 1-2):
- Ticket type base levels and default radii
- LoadState derivation from levels
- Column ↔ chunk coordinate conversion
- Convenience constructors use hardcoded radii
- Custom radius via `new()`
- Propagator: single ticket concentric levels
- Propagator: NPC ticket starts at level 1
- Propagator: overlapping tickets minimum-wins
- Propagator: ticket removal unloads exclusive columns
- Propagator: ticket move (far) produces loads + unloads
- Propagator: ticket move (1 step) has minimal diff
- Propagator: clean propagator returns empty diff
- Propagator: Chebyshev diagonal distance correctness
- Propagator: diff classifies by LOAD_LEVEL_THRESHOLD
- Propagator: overlapping ticket strengthens column → "changed"
- Propagator: column crossing threshold is "loaded" or "unloaded"
- Propagator: large radius column count
- Propagator: removal with overlap preserves shared columns

### Integration Tests (Phase 3-4):
- Column loading expands to 3D chunks
- Ticket level affects which columns get chunk data
- NPC ticket produces weaker levels than player
- Unloaded columns evict chunk data
- Ticket on map A doesn't load chunks on map B
- Switching ticket between maps unloads old, loads new
- Removing ticket entity unloads all its chunks
- Bounded maps respect bounds during column expansion
- All 8 existing lifecycle.rs tests adapted
- All 9 existing api.rs tests adapted

### Phase 5 Tests:
- Server pushes chunks to connected client without request
- Server sends UnloadColumn when columns leave loaded range
- Client handles UnloadColumn by removing data and despawning meshes
- Map transition works end-to-end with server-push

### Existing Tests (adapted in Phase 4):
- `lifecycle.rs`: 8 tests updated to use `ChunkTicket` (%% same count error — actually 9 tests)
- `api.rs`: 9 tests updated
- `integration.rs`: 3 tests updated (1 removed in Phase 5)
- `voxel_persistence.rs`: 5 tests updated (lines 17, 35, 90, 97, 108)
- `world_persistence.rs`: 1 test updated
- `map_transition.rs`: 2 tests updated
- `physics_isolation.rs`: 3 tests updated

### Manual Testing Steps:
1. `cargo server` — observe chunks loading around spawn position
2. `cargo client -c 1` — connect, verify chunks render around player
3. Walk to edge of loaded area — verify new chunks load, distant ones unload
4. Place/remove voxels — verify edits work and mesh updates
5. Trigger map transition — verify old map unloads, new map loads
6. Run examples: `cargo run --example terrain`, `cargo run --example editing`, `cargo run --example multi_instance`
7. (Phase 5) Verify client receives chunks without sending requests — grep for `ChunkRequest` should return zero hits in source code

## Tracy Instrumentation

Added in Phase 3 alongside the lifecycle rewrite. Uses the existing `bevy/trace_tracy` infrastructure (already active via `--features bevy/trace_tracy`). Spans and plots are included inline in the pseudocode above — every new system has `info_span!` and numeric `plot!` calls.

### Workspace Wiring

```toml
# crates/voxel_map_engine/Cargo.toml
[features]
tracy = ["tracy-client/enable"]

[dependencies]
tracy-client = { version = "0.18", default-features = false }
```

```toml
# Root Cargo.toml
[workspace.dependencies]
tracy-client = { version = "0.18", default-features = false }

# App crate features (e.g., server/Cargo.toml)
[features]
tracy = ["bevy/trace_tracy", "voxel_map_engine/tracy"]
```

Without the `enable` feature, all `tracy-client` macros compile to no-ops (zero cost).

### Key Metrics to Monitor

| Metric | What It Tells You | Tune |
|---|---|---|
| `bfs_steps_this_frame` | Level propagation cost per frame | Amortization budget |
| `gen_tasks_in_flight` | Backpressure health | `MAX_TASKS_PER_FRAME` |
| `gen_tasks_polled_this_frame` | Main-thread cost of chunk insertion | Poll cap (future Step 2) |
| `gen_queue_depth` | How far behind generation is | `MAX_TASKS_PER_FRAME`, batch size |
| `remesh_tasks_in_flight` | Remesh backpressure | Remesh cap (future Step 2) |
| `chunks_pushed_this_frame` | Server→client bandwidth pressure | Send throttling |

Run: `cargo run -p server --features tracy`

---

## Performance Considerations

- **3D→2D volume change**: The current system loads a 3D cubic volume: with distance=10, that's (21)³ = 9,261 chunks. The new system loads 2D columns × height. With `LOAD_LEVEL_THRESHOLD=2` and a player ticket (base_level=0, radius=10), only columns at Chebyshev distance ≤ 2 get chunk data: (5)² × 16 = 400 chunks. This is a 96% reduction in loaded volume. The loaded area is 80×80 blocks — small for an open-world game but sufficient for initial development and testing. `LOAD_LEVEL_THRESHOLD` can be raised (e.g., to 10 for 21×21 columns) to match current behavior if needed. The outer ring (distance 3-10) exists in the propagator but doesn't generate chunk data — it provides level metadata for future priority scheduling and multi-stage generation.
- **Incremental BFS cost**: Moving one chunk boundary invalidates ~40 columns (the ring entering/exiting radius). Recomputing their levels scans all sources (typically 1-3 tickets per map). This is O(boundary_size × num_sources) ≈ O(40 × 3) = O(120) — negligible per frame.
- **Full recompute fallback**: On first load or teleportation, the propagator processes all columns within radius. For radius 10: 441 columns × ~3 sources = ~1,300 iterations. Still fast (<1ms).
- **Column expansion**: Each loaded column expands to 16 chunk positions (Y range −8 to 7). Generation tasks are still capped at `MAX_TASKS_PER_FRAME=32`.
- **No regression on propagator frequency**: The propagator only runs when tickets move (player crosses chunk boundary), same as the current target cache invalidation.

## Migration Notes

- `ChunkTarget` is fully removed — no deprecation wrapper. All consumers switch at once in Phase 4.
- `loaded_chunks: HashSet<IVec3>` is removed in Phase 3. The replacement `chunk_levels: HashMap<IVec2, u32>` tracks columns, not individual chunks, and only contains loaded columns (level ≤ `LOAD_LEVEL_THRESHOLD`). Code that checked per-chunk membership should use either `chunk_levels.contains_key(&chunk_to_column(pos))` (is this column loaded?) or `get_chunk_data(pos).is_some()` (does this specific chunk have data?).
- Tests that manually insert into `loaded_chunks` must instead insert into `chunk_levels` and potentially also insert chunk data into the octree.
- `ChunkRequest` is removed in Phase 5. The server pushes chunks proactively.
- The client does not run the propagator. In Phase 4 it computes desired columns from ticket radius. In Phase 5 it is a passive receiver.

## References

- Research: `doc/research/2026-03-20-minecraft-chunk-ticket-system.md`
- Original voxel_map_engine plan: `doc/plans/2026-02-28-voxel-map-engine.md`
- Key source files:
  - `crates/voxel_map_engine/src/lifecycle.rs` — system chain
  - `crates/voxel_map_engine/src/chunk.rs:13-16` — ChunkTarget (to be removed)
  - `crates/voxel_map_engine/src/instance.rs:28-37` — VoxelMapInstance
  - `crates/voxel_map_engine/src/generation.rs:22-27` — PendingChunks
  - `crates/server/src/gameplay.rs:82,317` — server ticket insertion points
  - `crates/client/src/map.rs:117-132` — client ticket attachment
  - `crates/server/src/map.rs:640-685` — chunk request handler (removed in Phase 5)
  - `crates/client/src/map.rs:135-202` — chunk request sender (removed in Phase 5)
