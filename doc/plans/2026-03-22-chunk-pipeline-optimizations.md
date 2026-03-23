# Chunk Pipeline Optimizations Implementation Plan

## Overview

Optimize the `voxel_map_engine` chunk pipeline with time-based work budgets, priority queues, batched async generation, propagation amortization, reference counting, data structure improvements, and Tracy instrumentation. These are independent of the ticket system (Step 1) and networking (Step 4) — they improve the existing spawning/polling/saving pipeline.

## Current State Analysis

The ticket system (Step 1) and server-push networking (Step 4) are implemented. The pipeline has these bottlenecks:

| Area | Current | Problem |
|---|---|---|
| Gen spawning | 32/frame hard cap, sorted by level | No time awareness, no distance tiebreaker |
| Gen polling | All ready tasks consumed | Unbounded main-thread work |
| Remesh spawning | Entire `chunks_needing_remesh` drained | Unbounded; mass edits spike |
| Remesh polling | All ready tasks consumed | Unbounded main-thread work |
| In-flight tasks | No cap | Memory grows under sustained load |
| Chunk saving | Detached async tasks, no cap | I/O spike on teleport/mass unload |
| Propagation | Incremental BFS, runs to completion | Large moves (teleport) stall a frame |
| Task granularity | 1 chunk per async task | Pool overhead |
| Gen/remesh overlap | `pending_positions` dedup only | No cross-phase protection |
| `pending_by_level` | `Vec<HashSet<IVec2>>` (65 buckets) | Most buckets empty, linear scan |
| Observability | `info_span!` only | No numeric plots for tuning |

### Key Discoveries:
- `lifecycle.rs:19` — `MAX_TASKS_PER_FRAME = 32` is the only throttle
- `propagator.rs:180-196` — `process_pending_updates` has no `max_steps`; runs full BFS
- `lifecycle.rs:290-291` — columns sorted by level but no distance tiebreaker
- `lifecycle.rs:468-490` — `spawn_remesh_tasks` drains `chunks_needing_remesh` with zero throttle
- `lifecycle.rs:330-354` — `poll_chunk_tasks` consumes all ready tasks, no budget check
- `generation.rs:24-27` — `PendingChunks.tasks` is unbounded `Vec<Task<_>>`
- `lifecycle.rs:263-269` — `remove_column_chunks` uses `.detach()` on save tasks: no tracking, no cap, I/O spikes on mass unload
- `propagator.rs:34` — `pending_by_level: Vec<HashSet<IVec2>>` has 65 entries (0..=MAX_LEVEL)
- All budget-gated work is main-thread only; async tasks run independently on `AsyncComputeTaskPool`

## Desired End State

After this plan:
- All chunk pipeline work (spawn, poll, remesh) is governed by a per-frame time budget (~4ms default)
- Generation and remesh tasks are spawned in priority order (level ASC, distance to nearest ticket source ASC)
- Total in-flight async tasks are capped (128 gen, 64 remesh) — this is the backpressure mechanism for async work that runs to completion on `AsyncComputeTaskPool`
- Chunk saves are queued and drained at a bounded rate (no more detached fire-and-forget I/O)
- Large level propagation spreads across frames (256 BFS steps/frame default)
- Generation tasks batch 4 chunks per async task
- A `ChunkWorkTracker` prevents concurrent gen+remesh on the same chunk
- `pending_by_level` uses `BTreeMap` instead of a 65-element `Vec`
- Tracy plots expose queue depths, budget usage, BFS steps, in-flight counts, **and which cap caused work to stop** (time budget, hard cap, or in-flight cap)

### Verification

Build and run: `cargo server`, `cargo client -c 1`. Walk around, teleport, verify chunks load smoothly. Enable Tracy (`cargo run -p server --features tracy`) and confirm plots appear.

## What We're NOT Doing

- Multi-stage generation pipeline (Step 3 — separate plan)
- Changes to networking protocol
- Changes to ticket types or level thresholds
- Distance-and-direction-aware scheduling (Paper's front-facing optimization) — future refinement

## Implementation Approach

Tracy first (measure before optimizing), then budget (foundation), then priority queue + caps (builds on budget), then save queue (same async backpressure pattern), then amortization, batching, ref counting, and data structures (independent of each other).

---

## Phase 1: Tracy Instrumentation

### Overview
Add `tracy-client` dependency with a feature gate. Add numeric plots to all chunk pipeline functions. This is the foundation — every subsequent phase adds its own plots as part of the implementation.

### Changes Required:

#### 1. Workspace dependency
**File**: `Cargo.toml` (workspace root)
```toml
[workspace.dependencies]
tracy-client = { version = "0.18", default-features = false }
```

#### 2. Crate feature + dependency
**File**: `crates/voxel_map_engine/Cargo.toml`
```toml
[features]
tracy = ["tracy-client/enable"]

[dependencies]
tracy-client = { workspace = true }
```

#### 3. Server/client feature forwarding
**File**: `crates/server/Cargo.toml`
```toml
[features]
tracy = ["bevy/trace_tracy", "voxel_map_engine/tracy"]
```

#### 4. Tracy plots in existing functions
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Add plots at the end of each function body (these are no-ops without the `tracy` feature):

```rust
use tracy_client::{plot, plot_name};
```

In `spawn_missing_chunks` — after the loop:
```rust
plot!(plot_name!("gen_spawned_this_frame"), spawned as f64);
plot!(plot_name!("gen_queue_depth"), instance.chunk_levels.len() as f64);
```

In `poll_chunk_tasks` — track polled count, emit:
```rust
plot!(plot_name!("gen_tasks_in_flight"), pending.tasks.len() as f64);
plot!(plot_name!("gen_tasks_polled_this_frame"), polled as f64);
```

In `spawn_remesh_tasks` — track spawned count:
```rust
plot!(plot_name!("remesh_spawned_this_frame"), spawned as f64);
plot!(plot_name!("remesh_tasks_in_flight"), pending.tasks.len() as f64);
```

In `poll_remesh_tasks` — track polled count:
```rust
plot!(plot_name!("remesh_polled_this_frame"), polled as f64);
```

#### 5. Tracy plots in propagator
**File**: `crates/voxel_map_engine/src/propagator.rs`

In `process_pending_updates` — count steps:
```rust
let mut steps: usize = 0;
// ... inside loop: steps += columns.len();
plot!(plot_name!("bfs_steps_this_frame"), steps as f64);
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all`
- [x] `cargo test -p voxel_map_engine`
- [ ] `cargo server` builds and runs
- [ ] `cargo client -c 1` builds and runs

#### Manual Verification:
- [ ] `cargo run -p server --features tracy` — Tracy plots appear for gen_spawned, gen_tasks_in_flight, remesh_spawned, bfs_steps

---

## Phase 2: Time-Based Work Budget

### Overview
Replace hard `MAX_TASKS_PER_FRAME` with an `Instant`-based budget. The budget is a Component on map entities, reset each frame by `update_chunks`, consumed by all downstream systems in the chain.

### Changes Required:

#### 1. Budget component
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

```rust
/// Per-frame time budget for chunk pipeline work on a single map.
/// Reset at the start of each frame by `update_chunks`.
/// All downstream systems check `has_time()` before doing work.
#[derive(Component)]
pub struct ChunkWorkBudget {
    start: std::time::Instant,
    budget: std::time::Duration,
}

/// Default budget: ~25% of a 16ms frame at 60fps.
const CHUNK_WORK_BUDGET_MS: u64 = 4;

/// Safety caps — even within budget, don't exceed these per frame.
const MAX_GEN_SPAWNS_PER_FRAME: usize = 64;
const MAX_GEN_POLLS_PER_FRAME: usize = 32;
const MAX_REMESH_SPAWNS_PER_FRAME: usize = 32;
const MAX_REMESH_POLLS_PER_FRAME: usize = 32;

impl ChunkWorkBudget {
    fn reset(&mut self) {
        self.start = std::time::Instant::now();
    }

    /// Returns true if there is time remaining in the budget.
    pub fn has_time(&self) -> bool {
        self.start.elapsed() < self.budget
    }
}

impl Default for ChunkWorkBudget {
    fn default() -> Self {
        Self {
            start: std::time::Instant::now(),
            budget: std::time::Duration::from_millis(CHUNK_WORK_BUDGET_MS),
        }
    }
}
```

#### 2. Auto-insert budget + reset each frame
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Add `ChunkWorkBudget` to `ensure_pending_chunks` (same pattern as `TicketLevelPropagator`).

In `update_chunks`: add `&mut ChunkWorkBudget` to the map query. Call `budget.reset()` at the top of the per-map loop body.

#### 3. Thread budget through systems and helpers

`spawn_missing_chunks` is a helper called from `update_chunks`, not a standalone system — it receives the budget as a function parameter. The actual ECS systems (`poll_chunk_tasks`, `spawn_remesh_tasks`, `poll_remesh_tasks`) add `&ChunkWorkBudget` to their map queries directly.

**`update_chunks`**: Add `&mut ChunkWorkBudget` to the map query. Pass `&budget` to `spawn_missing_chunks`.

**`spawn_missing_chunks`**: Accept `budget: &ChunkWorkBudget` parameter. Check `budget.has_time()` in the loop alongside `spawned >= MAX_GEN_SPAWNS_PER_FRAME`. Remove old `MAX_TASKS_PER_FRAME` constant.

**`poll_chunk_tasks`**: Add budget + counter check:
```rust
let mut polled = 0;
while i < pending.tasks.len() && budget.has_time() && polled < MAX_GEN_POLLS_PER_FRAME {
```

**`spawn_remesh_tasks`**: Budget + counter:
```rust
let mut spawned = 0;
for chunk_pos in positions {
    if !budget.has_time() || spawned >= MAX_REMESH_SPAWNS_PER_FRAME { break; }
    // ... spawn task ...
    spawned += 1;
}
// Remaining positions go back into chunks_needing_remesh for next frame
```

**`poll_remesh_tasks`**: Budget + counter:
```rust
let mut polled = 0;
while i < pending.tasks.len() && budget.has_time() && polled < MAX_REMESH_POLLS_PER_FRAME {
```

#### 4. Stop-reason instrumentation

Every throttled loop must emit a Tracy plot indicating **why** it stopped. This is critical for tuning — knowing whether the time budget, hard cap, or in-flight cap is the bottleneck tells us which constant to adjust.

Use an enum-to-float mapping: `0.0` = completed all work, `1.0` = time budget exhausted, `2.0` = hard cap hit, `3.0` = in-flight cap hit.

```rust
/// Why a throttled loop stopped processing. Emitted as a Tracy plot for tuning.
/// Values are f64 for Tracy plot compatibility.
#[repr(u8)]
enum StopReason {
    /// All available work was processed.
    Completed = 0,
    /// Time budget exhausted.
    TimeBudget = 1,
    /// Per-frame hard cap reached.
    HardCap = 2,
    /// Total in-flight task cap reached.
    InFlightCap = 3,
}
```

In each throttled function, track the reason:

**`spawn_missing_chunks`**:
```rust
let stop_reason = if pending.tasks.len() >= MAX_PENDING_GEN_TASKS {
    StopReason::InFlightCap
} else if spawned >= MAX_GEN_SPAWNS_PER_FRAME {
    StopReason::HardCap
} else if !budget.has_time() {
    StopReason::TimeBudget
} else {
    StopReason::Completed
};
plot!(plot_name!("gen_spawn_stop_reason"), stop_reason as u8 as f64);
```

**`poll_chunk_tasks`**:
```rust
plot!(plot_name!("gen_poll_stop_reason"), stop_reason as u8 as f64);
```

**`spawn_remesh_tasks`**:
```rust
plot!(plot_name!("remesh_spawn_stop_reason"), stop_reason as u8 as f64);
```

**`poll_remesh_tasks`**:
```rust
plot!(plot_name!("remesh_poll_stop_reason"), stop_reason as u8 as f64);
```

Also emit the budget remaining:
```rust
plot!(plot_name!("chunk_work_budget_remaining_us"),
      budget.budget.saturating_sub(budget.start.elapsed()).as_micros() as f64);
```

#### 5. Remesh leftover handling

Currently `spawn_remesh_tasks` drains `chunks_needing_remesh` into a local `Vec` then iterates. With throttling, unprocessed positions must stay in the set. Change to: iterate `chunks_needing_remesh` directly, remove only the ones we spawn tasks for:

```rust
let positions: Vec<IVec3> = instance.chunks_needing_remesh.iter().copied().collect();
for chunk_pos in positions {
    if !budget.has_time() || spawned >= MAX_REMESH_SPAWNS_PER_FRAME { break; }
    instance.chunks_needing_remesh.remove(&chunk_pos);
    // ... spawn task ...
    spawned += 1;
}
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all`
- [x] `cargo test -p voxel_map_engine`
- [ ] `cargo server` — chunks still load around player
- [ ] `cargo client -c 1` — chunks still render

#### Manual Verification:
- [ ] Tracy: `chunk_work_budget_remaining_us` shows budget being consumed but not always exhausted
- [ ] Tracy: `gen_spawn_stop_reason` / `gen_poll_stop_reason` / `remesh_spawn_stop_reason` / `remesh_poll_stop_reason` plots visible and show which cap is active
- [ ] Walking around feels responsive (budget not too tight)
- [ ] Mass voxel edit doesn't spike frame time (remesh throttled)

---

## Phase 3: Priority Queue + In-Flight Caps

### Overview
Replace `Vec` sort with `BinaryHeap` for gen and remesh spawning. Add distance-to-nearest-source as tiebreaker. Cap total in-flight tasks.

### Changes Required:

#### 1. ChunkWork priority type
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

```rust
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// A chunk position with priority metadata for the work queue.
struct ChunkWork {
    position: IVec3,
    effective_level: u32,
    distance_to_source: u32,
}

impl Eq for ChunkWork {}
impl PartialEq for ChunkWork {
    fn eq(&self, other: &Self) -> bool {
        self.effective_level == other.effective_level
            && self.distance_to_source == other.distance_to_source
    }
}

/// Min-heap: lower level first, then closer distance first.
/// BinaryHeap is a max-heap, so we reverse the ordering.
impl Ord for ChunkWork {
    fn cmp(&self, other: &Self) -> Ordering {
        other.effective_level.cmp(&self.effective_level)
            .then(other.distance_to_source.cmp(&self.distance_to_source))
    }
}
impl PartialOrd for ChunkWork {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
```

#### 2. Build priority queue in `spawn_missing_chunks`

Replace current sort+iterate with:
```rust
let mut heap = BinaryHeap::new();
for (&col, &level) in &instance.chunk_levels {
    for chunk_pos in column_to_chunks(col, y_min, y_max) {
        if instance.get_chunk_data(chunk_pos).is_some() { continue; }
        if is_already_pending(pending, chunk_pos) { continue; }
        if !is_within_bounds(chunk_pos, config.bounds) { continue; }
        heap.push(ChunkWork {
            position: chunk_pos,
            effective_level: level,
            distance_to_source: propagator.min_distance_to_source(col),
        });
    }
}
while let Some(work) = heap.pop() {
    if !budget.has_time() || spawned >= MAX_GEN_SPAWNS_PER_FRAME { break; }
    // ... spawn task ...
}
```

#### 3. Add `min_distance_to_source` to propagator
**File**: `crates/voxel_map_engine/src/propagator.rs`

```rust
/// Returns the minimum Chebyshev distance from this column to any ticket source.
pub fn min_distance_to_source(&self, col: IVec2) -> u32 {
    self.sources.values()
        .map(|s| chebyshev_distance(s.column, col))
        .min()
        .unwrap_or(u32::MAX)
}
```

#### 4. Priority for remesh spawning
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Remesh uses the same `BinaryHeap` pattern as gen. Reuse `ChunkWork` (remesh doesn't have a level, so use 0 — distance is the only sort key):

```rust
let mut heap = BinaryHeap::new();
for &pos in instance.chunks_needing_remesh.iter() {
    let col = chunk_to_column(pos);
    heap.push(ChunkWork {
        position: pos,
        effective_level: 0,
        distance_to_source: propagator.min_distance_to_source(col),
    });
}
while let Some(work) = heap.pop() {
    if !budget.has_time() || spawned >= MAX_REMESH_SPAWNS_PER_FRAME { break; }
    instance.chunks_needing_remesh.remove(&work.position);
    // ... spawn task ...
    spawned += 1;
}
```

Add `&TicketLevelPropagator` and `&ChunkWorkBudget` to the `spawn_remesh_tasks` query.

#### 5. In-flight task caps (async backpressure)
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Async tasks on `AsyncComputeTaskPool` run to completion — they cannot be cancelled or budget-gated. The **only** backpressure mechanism is refusing to spawn new tasks when too many are in-flight. Without this, sustained load (teleport, mass edits) causes unbounded memory growth as tasks queue faster than they complete.

```rust
const MAX_PENDING_GEN_TASKS: usize = 128;
const MAX_PENDING_REMESH_TASKS: usize = 64;
```

Check in-flight cap **before entering the spawn loop** (early out):

In `spawn_missing_chunks`:
```rust
if pending.tasks.len() >= MAX_PENDING_GEN_TASKS {
    plot!(plot_name!("gen_spawn_stop_reason"), StopReason::InFlightCap as u8 as f64);
    return;
}
```

In `spawn_remesh_tasks`:
```rust
if pending.tasks.len() >= MAX_PENDING_REMESH_TASKS {
    plot!(plot_name!("remesh_spawn_stop_reason"), StopReason::InFlightCap as u8 as f64);
    return;
}
```

Also check inside the loop — the in-flight count grows as we spawn within a single frame:
```rust
// Inside spawn_missing_chunks loop:
if pending.tasks.len() >= MAX_PENDING_GEN_TASKS { break; }
```

> **Note**: Phase 6 (Batched Generation) changes `PendingChunks` to use `batch_tasks`. At that point, in-flight checks become `pending.batch_tasks.len() >= MAX_PENDING_GEN_TASKS`.

#### 6. Tracy plots
```rust
plot!(plot_name!("gen_queue_depth"), heap.len() as f64);
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all`
- [ ] `cargo test -p voxel_map_engine`
- [ ] `cargo server` && `cargo client -c 1`

#### Manual Verification:
- [ ] Chunks near the player load before distant chunks (visible spiral-out pattern)
- [ ] Tracy: `gen_tasks_in_flight` stays under 128, `remesh_tasks_in_flight` under 64
- [ ] Tracy: under sustained load (teleport), `gen_spawn_stop_reason` shows `InFlightCap (3.0)` confirming backpressure is active
- [ ] After mass edit, remesh processes closest chunks first

---

## Phase 4: PendingSaves Queue

### Overview
Replace detached fire-and-forget save tasks with a bounded `PendingSaves` queue. Currently `remove_column_chunks` (lifecycle.rs:263-269) calls `.detach()` on save tasks — no tracking, no cap, no backpressure. A teleport unloading hundreds of columns fires hundreds of concurrent I/O tasks. This phase adds a tracked queue with per-frame drain rate and in-flight cap, matching Minecraft's `CHUNK_SAVED_PER_TICK` and `MAX_ACTIVE_CHUNK_WRITES` pattern.

### Changes Required:

#### 1. PendingSaves component
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

```rust
use bevy::tasks::Task;

use std::collections::VecDeque;

/// Queued chunk saves awaiting async I/O.
#[derive(Component, Default)]
pub struct PendingSaves {
    /// Chunks waiting to be saved (not yet spawned as tasks). FIFO order.
    queue: VecDeque<PendingSave>,
    /// In-flight async save tasks.
    tasks: Vec<Task<()>>,
}

struct PendingSave {
    position: IVec3,
    data: ChunkData,
    save_dir: PathBuf,
}

/// Maximum save tasks drained from queue per frame.
const MAX_SAVE_SPAWNS_PER_FRAME: usize = 16;

/// Maximum concurrent in-flight save tasks.
const MAX_PENDING_SAVE_TASKS: usize = 32;
```

#### 2. Auto-insert on map entities
Add `PendingSaves` to `ensure_pending_chunks` alongside the other components.

#### 3. Replace detached saves in `remove_column_chunks`

Change `remove_column_chunks` to accept `&mut PendingSaves` and enqueue instead of detaching:

```rust
fn remove_column_chunks(
    instance: &mut VoxelMapInstance,
    pending_saves: &mut PendingSaves,
    col: IVec2,
    save_dir: Option<&std::path::Path>,
    y_min: i32,
    y_max: i32,
) {
    let _span = info_span!("remove_column_chunks").entered();
    for chunk_pos in column_to_chunks(col, y_min, y_max) {
        if instance.dirty_chunks.remove(&chunk_pos) {
            if let Some(dir) = save_dir {
                if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
                    pending_saves.queue.push_back(PendingSave {
                        position: chunk_pos,
                        data: chunk_data.clone(),
                        save_dir: dir.to_path_buf(),
                    });
                }
            }
        }
        instance.remove_chunk_data(chunk_pos);
    }
    instance.chunk_levels.remove(&col);
}
```

#### 4. New system: `drain_pending_saves`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

Add a new system to the chain that drains the save queue at a bounded rate:

```rust
/// Drain pending save queue: poll completed tasks, spawn new ones within budget.
pub fn drain_pending_saves(
    mut map_query: Query<&mut PendingSaves>,
) {
    let pool = AsyncComputeTaskPool::get();
    for mut pending in &mut map_query {
        // Poll completed save tasks
        let mut i = 0;
        while i < pending.tasks.len() {
            if check_ready(&mut pending.tasks[i]).is_some() {
                pending.tasks.swap_remove(i);
            } else {
                i += 1;
            }
        }

        // Spawn new save tasks from queue
        let mut spawned = 0;
        while !pending.queue.is_empty()
            && pending.tasks.len() < MAX_PENDING_SAVE_TASKS
            && spawned < MAX_SAVE_SPAWNS_PER_FRAME
        {
            let save = pending.queue.pop_front().unwrap();
            let task = pool.spawn(async move {
                if let Err(e) = crate::persistence::save_chunk(
                    &save.save_dir, save.position, &save.data
                ) {
                    error!("Failed to save chunk at {:?}: {e}", save.position);
                }
            });
            pending.tasks.push(task);
            spawned += 1;
        }

        plot!(plot_name!("save_queue_depth"), pending.queue.len() as f64);
        plot!(plot_name!("save_tasks_in_flight"), pending.tasks.len() as f64);
        plot!(plot_name!("saves_spawned_this_frame"), spawned as f64);

        let stop_reason = if pending.tasks.len() >= MAX_PENDING_SAVE_TASKS {
            StopReason::InFlightCap
        } else if spawned >= MAX_SAVE_SPAWNS_PER_FRAME {
            StopReason::HardCap
        } else {
            StopReason::Completed
        };
        plot!(plot_name!("save_spawn_stop_reason"), stop_reason as u8 as f64);
    }
}
```

Note: `drain_pending_saves` does NOT consume the `ChunkWorkBudget`. Save I/O is independent of the chunk generation pipeline — saves happen even when the generation budget is exhausted. Saves use their own separate caps.

#### 5. Add to system chain
**File**: `crates/voxel_map_engine/src/lib.rs`

Add `drain_pending_saves` to the system chain. Save enqueuing happens in `remove_column_chunks`, called from `update_chunks` during level diff processing. `drain_pending_saves` runs later in the chain to process the accumulated queue:

```rust
app.add_systems(
    Update,
    (
        lifecycle::ensure_pending_chunks,
        (lifecycle::update_chunks, lifecycle::poll_chunk_tasks).run_if(generation_enabled),
        lifecycle::despawn_out_of_range_chunks,
        lifecycle::drain_pending_saves,
        lifecycle::spawn_remesh_tasks,
        lifecycle::poll_remesh_tasks,
    )
        .chain(),
);
```

#### 6. Update `update_chunks` to pass PendingSaves

Add `&mut PendingSaves` to the `update_chunks` map query and pass it to `remove_column_chunks`:

```rust
for &col in &diff.unloaded {
    remove_column_chunks(&mut instance, &mut pending_saves, col, config.save_dir.as_deref(), y_min, y_max);
}
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all`
- [ ] `cargo test -p voxel_map_engine`
- [ ] `cargo server` && `cargo client -c 1`

#### Manual Verification:
- [ ] Teleport far away: no I/O spike (saves drain gradually over subsequent frames)
- [ ] Tracy: `save_tasks_in_flight` stays under 32, `save_queue_depth` drains to 0 over time
- [ ] Tracy: `save_spawn_stop_reason` shows `InFlightCap` during mass unload, then `Completed` as queue empties
- [ ] Dirty chunks are actually saved (edit voxel, walk away, reload — edit persists)

---

## Phase 5: Propagation Amortization

### Overview
Add a `max_steps` parameter to `process_pending_updates` so large BFS (teleport, mass ticket changes) spreads across frames instead of stalling.

### Changes Required:

#### 1. Amortized BFS
**File**: `crates/voxel_map_engine/src/propagator.rs`

```rust
/// Maximum BFS steps per propagation call. A player ticket at radius 10
/// affects ~441 columns. 256 handles most single-ticket changes in one frame.
const MAX_BFS_STEPS_PER_FRAME: usize = 256;
```

Change `process_pending_updates` signature and behavior:
```rust
/// Processes up to `max_steps` pending columns. Returns the number of
/// steps actually processed. If work remains, `dirty` stays true.
fn process_pending_updates(&mut self, max_steps: usize) -> usize {
    let mut steps = 0;
    let mut level_idx = self.min_pending_level;
    while level_idx < self.pending_by_level.len() && steps < max_steps {
        if self.pending_by_level[level_idx].is_empty() {
            level_idx += 1;
            continue;
        }
        let mut columns: Vec<IVec2> = self.pending_by_level[level_idx].drain().collect();
        let mut processed = 0;
        for &col in &columns {
            if steps >= max_steps { break; }
            self.recompute_column(col);
            steps += 1;
            processed += 1;
        }
        // Re-insert unprocessed columns back into the bucket
        if processed < columns.len() {
            self.pending_by_level[level_idx].extend(columns.drain(processed..));
        }
        level_idx = self.find_min_pending_from(0);
    }
    self.min_pending_level = self.find_min_pending_from(0);
    steps
}
```

#### 2. Update `propagate()` to use amortized processing

```rust
pub fn propagate(&mut self) -> LevelDiff {
    if !self.dirty {
        trace!("propagate: not dirty, returning empty diff");
        return LevelDiff::default();
    }

    let old_loaded = self.snapshot_loaded_columns();
    let steps = self.process_pending_updates(MAX_BFS_STEPS_PER_FRAME);

    // Only mark clean if all pending work is done
    let has_remaining = self.find_min_pending_from(0) < self.pending_by_level.len();
    if !has_remaining {
        self.dirty = false;
    }

    plot!(plot_name!("bfs_steps_this_frame"), steps as f64);
    plot!(plot_name!("bfs_remaining_dirty"), if has_remaining { 1.0 } else { 0.0 });
    plot!(plot_name!("bfs_hit_step_cap"), if has_remaining && steps >= MAX_BFS_STEPS_PER_FRAME { 1.0 } else { 0.0 });

    self.build_diff(&old_loaded)
}
```

#### 3. Caller handles partial propagation

`update_chunks` already calls `propagator.propagate()` each frame. With amortization, it may return partial diffs across multiple frames. The existing diff application logic (`loaded`, `changed`, `unloaded`) is already additive — partial diffs apply correctly. No caller changes needed.

#### 4. Update tests

Existing tests call `propagate()` and expect full completion. For tests, either:
- Use a large `MAX_BFS_STEPS_PER_FRAME` in `#[cfg(test)]` (e.g., 10000), or
- Call `propagate()` in a loop until `!is_dirty()`:

```rust
/// Runs propagation to completion across multiple amortized calls.
/// Returns the final loaded set (not accumulated partial diffs, which
/// can contain duplicates — e.g., a column "loaded" in diff 1 then
/// "changed" in diff 2).
fn propagate_fully(prop: &mut TicketLevelPropagator) -> LevelDiff {
    while prop.is_dirty() {
        prop.propagate();
    }
    // Build diff from final state vs empty (everything is "loaded")
    let mut diff = LevelDiff::default();
    for (&col, &level) in prop.levels() {
        if level <= LOAD_LEVEL_THRESHOLD {
            diff.loaded.push((col, level));
        }
    }
    diff
}
```

Prefer the test helper approach — it validates the amortization logic produces correct final state. Tests that need intermediate diff semantics should call `propagate()` directly.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all`
- [ ] `cargo test -p voxel_map_engine` — all propagator tests pass with amortized BFS
- [ ] `cargo server` && `cargo client -c 1`

#### Manual Verification:
- [ ] Teleport far away: chunks load progressively over 2-3 frames instead of one stall
- [ ] Tracy: `bfs_steps_this_frame` ≤ 256, `bfs_remaining_dirty` drops to 0 within a few frames

---

## Phase 6: Batched Async Generation

### Overview
Group multiple chunks into a single async task to reduce pool overhead and contention.

### Changes Required:

#### 1. Batch size constant
**File**: `crates/voxel_map_engine/src/generation.rs`

```rust
/// Number of chunks to generate per async task.
/// Larger batches reduce pool overhead but increase latency for individual chunks.
const GEN_BATCH_SIZE: usize = 4;
```

#### 2. Batch spawn function
**File**: `crates/voxel_map_engine/src/generation.rs`

```rust
/// Spawn an async task that generates a batch of chunks.
pub fn spawn_chunk_gen_batch(
    pending: &mut PendingChunks,
    positions: Vec<IVec3>,
    generator: &VoxelGenerator,
    save_dir: Option<PathBuf>,
) {
    let generator = Arc::clone(&generator.0);
    let pool = AsyncComputeTaskPool::get();

    for &pos in &positions {
        pending.pending_positions.insert(pos);
    }

    let task = pool.spawn(async move {
        let _span = info_span!("chunk_gen_batch").entered();
        positions.into_iter().map(|pos| {
            if let Some(ref dir) = save_dir {
                match crate::persistence::load_chunk(dir, pos) {
                    Ok(Some(chunk_data)) => {
                        let mesh = if chunk_data.fill_type == FillType::Empty {
                            None
                        } else {
                            let voxels = chunk_data.voxels.to_voxels();
                            mesh_chunk_greedy(&voxels)
                        };
                        return ChunkGenResult { position: pos, mesh, chunk_data, from_disk: true };
                    }
                    Ok(None) => {}
                    Err(e) => {
                        bevy::log::warn!("Failed to load chunk at {pos}: {e}, regenerating");
                    }
                }
            }
            generate_chunk(pos, &*generator)
        }).collect::<Vec<_>>()
    });

    pending.batch_tasks.push(task);
}
```

#### 3. Replace PendingChunks task storage
**File**: `crates/voxel_map_engine/src/generation.rs`

Replace the single-chunk `tasks` field with batch-only storage. A "batch" of 1 is just a batch — no need for dual-path polling.

```rust
#[derive(Component, Default)]
pub struct PendingChunks {
    pub tasks: Vec<Task<Vec<ChunkGenResult>>>,
    pub pending_positions: HashSet<IVec3>,
}
```

Remove `spawn_chunk_gen_task` — all spawning goes through `spawn_chunk_gen_batch`. Callers that previously spawned single chunks now pass a `vec![position]`.

#### 4. Update `spawn_missing_chunks` to batch

Collect up to `GEN_BATCH_SIZE` positions from the heap before spawning:
```rust
let mut batch = Vec::with_capacity(GEN_BATCH_SIZE);
while let Some(work) = heap.pop() {
    if !budget.has_time() || spawned >= MAX_GEN_SPAWNS_PER_FRAME { break; }
    if pending.tasks.len() >= MAX_PENDING_GEN_TASKS { break; }
    batch.push(work.position);
    spawned += 1;
    if batch.len() >= GEN_BATCH_SIZE {
        spawn_chunk_gen_batch(&mut pending, std::mem::take(&mut batch), generator, config.save_dir.clone());
    }
}
if !batch.is_empty() {
    spawn_chunk_gen_batch(&mut pending, batch, generator, config.save_dir.clone());
}
```

#### 5. Update `poll_chunk_tasks` for batch tasks

Since `tasks` is now `Vec<Task<Vec<ChunkGenResult>>>`, polling yields a `Vec` of results per task:
```rust
let mut i = 0;
while i < pending.tasks.len() && budget.has_time() && polled < MAX_GEN_POLLS_PER_FRAME {
    if let Some(results) = check_ready(&mut pending.tasks[i]) {
        let _ = pending.tasks.swap_remove(i);
        for result in results {
            pending.pending_positions.remove(&result.position);
            handle_completed_chunk(/* ... */, result);
            polled += 1;
        }
    } else {
        i += 1;
    }
}
```

Note: `polled` counts individual chunk results, not batch tasks. A single batch completion may yield up to `GEN_BATCH_SIZE` results, each incrementing `polled`. Both `polled < MAX_GEN_POLLS_PER_FRAME` and `budget.has_time()` are checked per outer iteration, so a batch that pushes past the hard cap won't start processing the next batch.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all`
- [ ] `cargo test -p voxel_map_engine`
- [ ] `cargo server` && `cargo client -c 1`

#### Manual Verification:
- [ ] Chunks still load correctly (same visual result)
- [ ] Tracy: fewer total async tasks spawned (batches of 4), same throughput

---

## Phase 7: Generation Reference Counting

### Overview
Prevent concurrent gen+remesh on the same chunk with a `ChunkWorkTracker` component.

### Changes Required:

#### 1. ChunkWorkTracker component
**File**: `crates/voxel_map_engine/src/lifecycle.rs`

```rust
/// Tracks which chunks have in-flight work to prevent overlapping gen/remesh.
#[derive(Component, Default)]
pub struct ChunkWorkTracker {
    pub generating: HashSet<IVec3>,
    pub remeshing: HashSet<IVec3>,
}
```

#### 2. Auto-insert on map entities
Add to `ensure_pending_chunks` alongside `PendingChunks`, etc.

#### 3. Guard spawn_missing_chunks
Skip spawning gen if chunk is in `tracker.remeshing`:
```rust
if tracker.generating.contains(&chunk_pos) || tracker.remeshing.contains(&chunk_pos) { continue; }
// after spawn:
tracker.generating.insert(chunk_pos);
```

#### 4. Guard spawn_remesh_tasks
Skip remesh if chunk is in `tracker.generating`:
```rust
if tracker.generating.contains(&chunk_pos) || tracker.remeshing.contains(&chunk_pos) {
    // Leave in chunks_needing_remesh for next frame
    continue;
}
// after spawn:
tracker.remeshing.insert(chunk_pos);
```

#### 5. Clear on task completion
In `poll_chunk_tasks`:
```rust
tracker.generating.remove(&result.position);
```

In `poll_remesh_tasks`:
```rust
tracker.remeshing.remove(&remesh.chunk_pos);
```

#### 6. Replace PendingChunks.pending_positions

`PendingChunks.pending_positions` is now redundant — it duplicates `tracker.generating`. Replace all `pending.pending_positions` usage with `tracker.generating`. Remove `pending_positions` from `PendingChunks`.

> **Cross-phase dependency**: Phase 6's `spawn_chunk_gen_batch` uses `pending.pending_positions.insert(pos)`. This phase replaces those calls with `tracker.generating.insert(pos)`. Phase 6's code must be updated retroactively when this phase is implemented.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all`
- [ ] `cargo test -p voxel_map_engine`
- [ ] `cargo server` && `cargo client -c 1`

#### Manual Verification:
- [ ] Edit a chunk that's still generating — remesh deferred, no panic/corruption
- [ ] Tracy: no overlapping gen+remesh on same position

---

## Phase 8: Data Structure Optimizations

### Overview
Replace `pending_by_level: Vec<HashSet<IVec2>>` (65 pre-allocated buckets, most empty) with `BTreeMap<u32, HashSet<IVec2>>`. This is a clean swap — the BTreeMap provides ordered iteration by level (same as the Vec index scan) with no empty-bucket overhead.

### Changes Required:

#### 1. Replace Vec with BTreeMap
**File**: `crates/voxel_map_engine/src/propagator.rs`

```rust
use std::collections::BTreeMap;

pub struct TicketLevelPropagator {
    levels: HashMap<IVec2, u32>,
    sources: HashMap<Entity, TicketSource>,
    pending_by_level: BTreeMap<u32, HashSet<IVec2>>,
    dirty: bool,
}
```

Remove `min_pending_level` field — `BTreeMap::first_key_value()` provides O(log n) access to the minimum key.

#### 2. Update insert_pending
```rust
fn insert_pending(&mut self, level: u32, col: IVec2) {
    if level <= MAX_LEVEL {
        self.pending_by_level.entry(level).or_default().insert(col);
    }
}
```

#### 3. Update process_pending_updates
```rust
fn process_pending_updates(&mut self, max_steps: usize) -> usize {
    let mut steps = 0;
    while steps < max_steps {
        let Some((&level, _)) = self.pending_by_level.first_key_value() else {
            break;
        };
        let mut columns: Vec<IVec2> = self.pending_by_level.remove(&level).unwrap().into_iter().collect();
        let mut processed = 0;
        for &col in &columns {
            if steps >= max_steps { break; }
            self.recompute_column(col);
            steps += 1;
            processed += 1;
        }
        // Re-insert unprocessed columns back into the BTreeMap
        if processed < columns.len() {
            self.pending_by_level.entry(level).or_default()
                .extend(columns.drain(processed..));
        }
    }
    // Clean up empty entries
    self.pending_by_level.retain(|_, set| !set.is_empty());
    steps
}
```

#### 4. Update find_min_pending_from

Remove this method entirely — it's replaced by `BTreeMap::first_key_value()`.

#### 5. Update constructor
```rust
pub fn new() -> Self {
    Self {
        levels: HashMap::new(),
        sources: HashMap::new(),
        pending_by_level: BTreeMap::new(),
        dirty: false,
    }
}
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all`
- [ ] `cargo test -p voxel_map_engine` — all propagator tests pass unchanged
- [ ] `cargo server` && `cargo client -c 1`

#### Manual Verification:
- [ ] Behavior identical to before (level computation results unchanged)

---

## Testing Strategy

### Unit Tests:
- `ChunkWorkBudget`: verify `has_time()` returns false after budget elapsed
- `ChunkWork` ordering: verify min-heap produces level ASC, distance ASC
- `ChunkWorkTracker`: verify gen/remesh mutual exclusion
- `PendingSaves`: verify queue drain respects caps, verify enqueue/dequeue ordering
- `TicketLevelPropagator` amortized: verify `propagate_fully` helper produces same results as unbounded BFS
- `BTreeMap` propagator: all existing propagator tests pass without modification

### Integration Tests:
- Budget system integration: spawn map with tickets, advance multiple frames, verify chunks load progressively
- Batch generation: verify all chunks in a batch complete and insert into octree
- Save queue: dirty chunk eviction enqueues saves, `drain_pending_saves` processes them over frames

### Manual Testing Steps:
1. Start server + client, walk around — chunks load smoothly
2. Teleport far away — chunks load progressively (no frame stall), saves drain gradually
3. Mass voxel edit — remesh spreads across frames
4. Edit voxels, walk away, restart server — edits persist (save queue flushed)
5. Enable Tracy, verify all plots appear and update (including save_queue_depth, save_tasks_in_flight)

## Performance Considerations

- Both gen and remesh use `BinaryHeap` for priority ordering — consistent pattern, and both benefit from early exit without sorting the full set
- `BinaryHeap` rebuild per frame in `spawn_missing_chunks` and `spawn_remesh_tasks` — acceptable: gen heap shrinks as chunks load, remesh heap is typically small
- `min_distance_to_source` iterates all sources per column — fine with <100 sources (tickets). If sources grow large, add a spatial index later
- Batch size 4 is conservative — tune with Tracy data
- BFS amortization 256 steps handles 1 player ticket in 1-2 frames. Multiple concurrent teleports may take more frames — acceptable

## References

- Research: `doc/research/2026-03-20-minecraft-chunk-ticket-system.md`
  - Strategy 1 (time budget): lines 840-897
  - Strategy 2 (in-flight caps): lines 901-910
  - Strategy 3 (priority queue): lines 912-933
  - Strategy 4 (batching): lines 935-949
  - Strategy 5 (amortization): lines 951-969
  - Strategy 7 (ref counting): lines 993-1004
  - Tracy integration: lines 1250-1345
- Current implementation: `crates/voxel_map_engine/src/lifecycle.rs`, `propagator.rs`, `generation.rs`
