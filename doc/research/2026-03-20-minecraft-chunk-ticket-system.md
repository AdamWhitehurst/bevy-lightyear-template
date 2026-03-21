---
date: 2026-03-20T14:41:26-07:00
researcher: claude
git_commit: a3d0a6df805a1709361535e79a316eedcc78736d
branch: bevy-lightyear-template-2
repository: bevy-lightyear-template-2
topic: "Minecraft's chunk loading ticket system and how to implement it in voxel_map_engine"
tags: [research, minecraft, chunk-loading, ticket-system, voxel-map-engine, lifecycle, optimization, async-tasks, multi-stage-generation]
status: complete
last_updated: 2026-03-21
last_updated_by: claude
last_updated_note: "Added code-level ticket system details, optimization strategies, multi-stage generation, and resolved open questions"
---

# Research: Minecraft's Chunk Loading Ticket System

**Date**: 2026-03-20T14:41:26-07:00
**Researcher**: claude
**Git Commit**: a3d0a6df805a1709361535e79a316eedcc78736d
**Branch**: bevy-lightyear-template-2
**Repository**: bevy-lightyear-template-2

## Research Question

How does Minecraft's chunk loading ticket system work, and how would it map onto this project's `voxel_map_engine` crate?

## Summary

Minecraft's ticket system (Java 1.14+) is the sole mechanism that causes chunks to load. Every loaded chunk traces back to at least one *ticket* — a data object with a **type**, a **load level** (22-44), and an optional **lifetime**. The level propagates outward from the ticket source (+1 per chunk in Chebyshev distance), creating concentric rings of decreasing load state. When multiple tickets overlap, the lowest (strongest) level wins. Chunks transition through load states: entity ticking (level ≤31), block ticking (32), border (33), inaccessible (34-44), and unloaded (45+).

The current `voxel_map_engine` has a simpler model: `ChunkTarget` components with a flat `distance` field produce a binary loaded/unloaded set. There is no concept of load levels, ticket types, expiration, or propagation — all desired chunks are equally "loaded." Mapping Minecraft's system onto the engine would mean replacing `ChunkTarget` entirely with a ticket-based system, replacing the flat `HashSet<IVec3>` desired set with a per-chunk level computed from multiple ticket sources, and gating chunk processing (generation, meshing, entity ticking) by level thresholds.

---

## Minecraft's Ticket System

### Ticket Properties

A ticket has three properties:

| Property | Description |
|---|---|
| **Type** | Source category (player, portal, forced, start, etc.) |
| **Level** | Integer 22-44; lower = stronger/more processing |
| **Lifetime** | Ticks remaining before expiration; some types never expire |

### Ticket Types

| Type | Level | Lifetime | Persists | Notes |
|---|---|---|---|---|
| Start (spawn) | 22 | Never | No | World spawn chunk. Creates largest loaded area. |
| Dragon | 24 | Never | No | Chunk (0,0) in The End during dragon fight. |
| Player Spawn | 30 | 1s (refreshed) | No | During respawn. |
| Portal | 30 | 15s (300 ticks) | Yes | Entity teleport via portal. |
| Player Loading | 31 | Never | No | Square grid around player (server render distance). |
| Forced | 31 | Never | Yes | `/forceload` command. Survives restart. |
| Ender Pearl | 31 | 2s (refreshed) | No | Follows thrown pearl. |
| Post-teleport | 32-33 | 5 ticks | No | After `/tp`, spread, etc. |

### Chunk Load States

Level determines processing:

| State | Level | Processing |
|---|---|---|
| Entity Ticking | ≤31 | Full gameplay: entity AI, spawning, block ticks, redstone |
| Block Ticking | 32 | Block mechanics (redstone, scheduled ticks), entities frozen |
| Border | 33 | No mechanics; blocks accessible to neighbors, mobs count for cap |
| Inaccessible | 34-44 | World generation pipeline runs, not accessible for gameplay |
| Unloaded | 45+ | Not loaded |

### Level Propagation

From each ticket source, the level increases by 1 per chunk outward (Chebyshev/8-neighbor distance). A chunk's effective level is the **minimum** of all levels it receives from all sources.

Example — player ticket (level 31):
- Player chunk: 31 (entity ticking)
- +1 ring: 32 (block ticking)
- +2 ring: 33 (border)
- +3 ring: 34 (inaccessible, generating)

Example — spawn ticket (level 22):
- Spawn chunk: 22 → up to +9 chunks out = 31 (entity ticking)
- +10: 32 (block ticking)
- +11: 33 (border)
- ...propagates to +22 before capping at 44

### Scheduling and Throttling

- Time-based budget: process chunk operations while `System.nanoTime() < tickDeadline` (50ms tick target)
- Chunks closer to players are prioritized via `LevelPrioritizedQueue`
- Generation progresses through a multi-stage pipeline (empty → structures → biomes → noise → surface → carvers → features → light → spawn → full)
- Each stage has a `taskMargin` / `blockStateWriteRadius` requiring neighbors at sufficient status

---

## Minecraft's Ticket System in Code

### ChunkTicketManager Data Structures

The core storage is a `Long2ObjectOpenHashMap<List<ChunkTicket>>` keyed by packed chunk position (long). Each position maps to a list of active tickets.

Key fields:
- `tickets: Long2ObjectOpenHashMap<List<ChunkTicket>>` — active tickets by packed chunk position
- `savedTickets: Long2ObjectOpenHashMap<List<ChunkTicket>>` — persisted tickets (survive restart)
- `forcedChunks: LongSet` — set of force-loaded chunk positions
- `loadingLevelUpdater: LevelUpdater` — callback for loading-phase level changes
- `simulationLevelUpdater: LevelUpdater` — callback for simulation-phase level changes

Ticket operations:
- `addTicket(long pos, ChunkTicket)` adds to the list at that position, returns true if new
- `removeTicket(long pos, ChunkTicket)` removes from the list, returns true if present
- `getLevel(long pos, boolean forSimulation)` scans the ticket list and returns the minimum level (strongest ticket wins)
- `tick()` processes level updates by invoking the `LevelUpdater` callbacks
- `promoteToRealTickets()` transitions saved/persisted tickets into the active map on world load

The two-phase update (`loadingLevelUpdater` vs `simulationLevelUpdater`) allows the system to load chunk data before enabling simulation — loading level controls generation/data loading, simulation level controls ticking behavior.

### How Ticket Levels are Calculated

For a given chunk position, the effective level is:

```
effective_level(chunk) = min over all tickets T {
    T.level + chebyshev_distance(T.position, chunk)
}
```

Where Chebyshev distance is `max(|dx|, |dz|)` (2D in Minecraft).

The level also determines which `ChunkStatus` generation stage the chunk can reach. The formula is `ChunkStatus.byDistanceFromFull(level - 33)`:
- Level 33 (distance 0) = FULL
- Level 34 (distance 1) = LIGHT
- Level 35 (distance 2) = FEATURES
- ...
- Level 44 (distance 11) = EMPTY

### How Chunk Levels are Propagated (LevelPropagator)

The propagation uses a **priority-bucketed BFS** algorithm. The base class `LevelPropagator` (shared with light propagation) uses:

```java
LongLinkedOpenHashSet[] pendingIdUpdatesByLevel;  // one bucket per level
Long2ByteMap pendingUpdates;                       // queued level changes
int minPendingLevel;                               // lowest dirty level
```

Algorithm:
1. When a ticket is added/removed, the source chunk's level changes
2. `updateLevel(sourceId, targetId, level, decrease)` queues the change
3. `propagateLevel(sourceId, targetId, level, decrease)` pushes updates to neighbors
4. `applyPendingUpdates(maxSteps)` processes the queue in level order (lowest first), visiting neighbors and propagating level increases

The `pendingIdUpdatesByLevel` array is the key optimization: instead of a generic priority queue, it uses an array of linked hash sets indexed by level. `minPendingLevel` tracks the lowest non-empty bucket for O(1) access. This is a **bucket-queue BFS** — specialized for integer costs that differ by 1.

Propagation behavior:
- Each step outward increases level by 1 (Chebyshev neighbors)
- Propagation stops at level 44 (MAX_LEVEL)
- When tickets overlap, `recalculateLevel` recomputes from all neighbors and takes the minimum
- Both increases and decreases are handled incrementally: adding a ticket pushes lower levels outward; removing one causes levels to increase (weaken), requiring localized recalculation — not a full recompute

### ChunkHolder State Machine

Each loaded chunk position has a `ChunkHolder` that manages the chunk's lifecycle through `CompletableFuture`s.

Key fields:
- `ticketLevel: int` — current ticket level from the ticket manager
- `queueLevel: int` — scheduling priority level
- `oldTicketLevel: int` — previous level for detecting transitions
- `futures: AtomicReferenceArray<CompletableFuture<...>>` — one future per `ChunkStatus` stage

Static methods map level to state:
- `getFullChunkStatus(int level) -> FullChunkStatus` — maps level to INACCESSIBLE/BORDER/TICKING/ENTITY_TICKING
- `getStatus(int level) -> ChunkStatus` — maps level to maximum generation status achievable

`updateFutures(ChunkMap, Executor)` is called when the ticket level changes. It compares old vs new level to determine:
- **Promotion**: schedules generation tasks or enables ticking futures
- **Demotion**: cancels futures or demotes chunk state

### ChunkMap (ThreadedAnvilChunkStorage)

Maintains all `ChunkHolder`s in three maps:
- `updatingChunkMap: Long2ObjectLinkedOpenHashMap<ChunkHolder>` — chunks being modified
- `visibleChunkMap` (volatile) — chunks safe for reading from other threads
- `pendingUnloads` — chunks queued for removal

Threading via mailbox-based message passing:
- `worldgenMailbox: ProcessorHandle` — generation work queue
- `mainThreadMailbox: ProcessorHandle` — main thread operations
- `queueSorter: ChunkTaskPriorityQueueSorter` — prioritizes tasks by level

`ChunkHolder`s are created when ticket level ≤ 45, removed when level > 45.

### Client-Server Protocol

**The client does NOT run the ticket system.** The ticket system is entirely server-side. The server makes all decisions about what chunks to send.

Server sends these chunk-related packets:

| Packet | Purpose |
|---|---|
| **Set Center Chunk** | Sets center of client's loading area (chunk X, Z) |
| **Set View Distance** | Tells client the server's render distance |
| **Set Simulation Distance** | Tells client the simulation distance |
| **Chunk Data and Update Light** | Full chunk column data |
| **Unload Chunk** | Tells client to drop a chunk column |

The client's loading area is a square: `2 * server_view_distance + 7` chunks on each axis. The client ignores chunks outside this area and immediately unloads chunks that fall outside when the center changes. The client never requests chunks; the server pushes them.

This is fundamentally different from the current `voxel_map_engine` where the client computes desired chunks and requests missing ones from the server.

---

## Minecraft's Optimization Strategies

### Time-Based Tick Budget (Not Hard Count)

Minecraft does NOT use a hard integer cap on chunk operations per tick. Instead, it uses a **time-based budget**: keep processing chunk work while `System.nanoTime() < tickDeadline`. A `BooleanSupplier shouldKeepTicking` is threaded through `tick()`, `processUnloads()`, and `saveChunksEagerly()`.

Specific budgets:
- `CHUNK_SAVED_PER_TICK` and `CHUNK_SAVED_EAGERLY_PER_TICK` — I/O write caps
- `MAX_ACTIVE_CHUNK_WRITES` — `AtomicInteger`-tracked concurrent async I/O writes
- Unload budget: Paper processes "minimum 50 chunks or 5% of unload queue, whichever is larger" per tick
- Empirically ~50 chunk operations per tick at the 50ms tick target

### Priority Scheduling

`ChunkTaskScheduler` with a `LevelPrioritizedQueue`:
- Tasks indexed by chunk position, priority derived from chunk's **load level** (lower = higher priority)
- `LEVELS` constant defines discrete priority tiers
- `updateLevel()` re-prioritizes tasks when a chunk's level changes

Paper's improvements (commit `3dc5ad3`) add distance-and-direction-aware scheduling:
- **Front-facing chunks** (in player movement direction): 10-20 tick delays
- **Back-facing chunks**: 15-40 tick delays
- **Very near chunks** (distance < 5-6 blocks): load immediately, skip throttle
- Priority `URGENT` (value 2) for blocking loads; chunks at priority < 5 bypass throttling

### Async/Threading Model

Vanilla `ChunkMap` (1.21+):
- `worldgenTaskDispatcher` — dispatches generation tasks to worker thread pool
- `lightTaskDispatcher` — separate dispatcher for lighting calculations
- `mainThreadExecutor` — server tick thread for non-thread-safe state
- `PrioritizedConsecutiveExecutor` — ensures tasks for the same chunk run in sequence while different chunks parallelize

Worker allocation: defaults to `max(1, physicalCores/2)` threads for chunk generation.

What happens where:
- **Worker threads**: noise generation, surface placement, carving, feature placement, structure generation, light calculation, chunk serialization/deserialization
- **Main thread**: final chunk promotion to FULL, entity spawning, block entity ticking, chunk event callbacks, ticket manager updates, level propagation

### Throttling and Backpressure

When a player teleports far away:
1. Post-teleport ticket (level 32-33, 5-tick lifetime) created at destination
2. New chunks flood the pending queue, but the time-budget `BooleanSupplier` prevents stalling
3. Paper adds **load delays by distance**: distant chunks intentionally delayed 20-40 ticks
4. `ThrottledChunkTaskScheduler` rate-limits task submission to prevent overwhelming the worker pool
5. Old-location chunks enter `pendingUnloads`, processed gradually (50 or 5% per tick)
6. `EAGER_CHUNK_SAVE_COOLDOWN_IN_MILLIS` prevents saving a chunk too frequently

### Batching and Contention Reduction

- `pendingGenerationTasks` list — tasks accumulated, drained via `runGenerationTasks()` during tick
- `acquireGeneration(pos)` / `releaseGeneration(holder)` — reference counting prevents concurrent modification of the same chunk
- `PrioritizedConsecutiveExecutor` — same-chunk tasks sequential, cross-chunk tasks parallel
- Paper abandoned `ConcurrentHashMap` for lock-free data structures to reduce contention
- `Long2ByteMap chunkTypeCache` — caches chunk state lookups during batch processing

### Multi-Stage Generation Pipeline

Stages (1.21+, ordered):

| # | Status | Description | taskMargin |
|---|---|---|---|
| 0 | EMPTY | Initial allocation | -1 (no requirements) |
| 1 | STRUCTURE_STARTS | Place structure start points | 0 |
| 2 | STRUCTURE_REFERENCES | Link neighboring chunks to structures | 8 (structure search radius) |
| 3 | BIOMES | Determine and store biome data | 0 |
| 4 | NOISE | Base terrain shape, liquid bodies | 0 |
| 5 | SURFACE | Biome-dependent surface blocks | 0 |
| 6 | CARVERS | Cave carving | 0 |
| 7 | FEATURES | Feature placement, structures, heightmaps | 1 (blockStateWriteRadius=1) |
| 8 | INITIALIZE_LIGHT | Initialize lighting engine | 0 |
| 9 | LIGHT | Calculate light levels | 1 |
| 10 | SPAWN | Mob spawning preparation | 0 |
| 11 | FULL | Promotion to LevelChunk | 0 |

`taskMargin` / `blockStateWriteRadius` defines how many chunks outward must be at the previous status. FEATURES has `blockStateWriteRadius=1` because features can write blocks in a 3x3 chunk area.

**How taskMargin interacts with ticket levels**: A chunk at ticket level N can reach `ChunkStatus.byDistanceFromFull(N - 33)`. To generate a chunk to FULL, surrounding rings must be at progressively earlier stages. Generating one FULL chunk may require partially generating chunks up to 11 chunks out.

The `ChunkGenerationSteps.Builder` constructs the pipeline fluently:
```java
builder.then(STRUCTURE_STARTS, step -> step.task(...))
       .then(STRUCTURE_REFERENCES, step -> step.dependsOn(STRUCTURE_STARTS, 8).task(...))
       .then(FEATURES, step -> step.blockStateWriteRadius(1).task(...))
       ...
```

### Incremental Level Updates

When a ticket is added/removed, the `LevelPropagator` does NOT recompute all levels from scratch. Instead:
1. The removed source's contributions are invalidated
2. An incremental update propagates outward from the change point
3. For each affected chunk, check if another source provides an equal or lower level
4. If not, the chunk's level increases (weakens), which propagates further
5. The number of affected chunks is bounded by the ticket's radius of influence

---

## Current `voxel_map_engine` Architecture

### Chunk Loading Model

The engine uses `ChunkTarget` components to drive loading:

```
ChunkTarget { map_entity: Entity, distance: u32 }
```

Each target generates a cubic volume of desired chunk positions (`-dist..=dist` on all three axes). All targets for a map are unioned into a `HashSet<IVec3>` of desired positions. Chunks in the set are loaded; chunks outside are unloaded. There is no level differentiation.

### Key Components and Flow

| Component | File | Role |
|---|---|---|
| `ChunkTarget` | `chunk.rs:12-16` | Drives loading; attached to player entities |
| `VoxelMapConfig` | `config.rs:16-29` | Per-map settings (seed, bounds, generates_chunks, spawning_distance) |
| `VoxelGenerator` | `config.rs:12-13` | `Arc<dyn Fn(IVec3) -> Vec<WorldVoxel>>` generation function |
| `VoxelMapInstance` | `instance.rs:28-37` | Owns octree, loaded_chunks set, dirty/remesh sets |
| `PendingChunks` | `generation.rs:22-27` | In-flight async generation tasks + pending_positions HashSet |
| `PendingRemeshes` | `lifecycle.rs:33-43` | In-flight async remesh tasks |

### Lifecycle System Chain (lib.rs:32-41)

```
ensure_pending_chunks → update_chunks → poll_chunk_tasks
→ despawn_out_of_range_chunks → spawn_remesh_tasks → poll_remesh_tasks
```

1. **`ensure_pending_chunks`** (lifecycle.rs:48-75): Inserts `PendingChunks` and `PendingRemeshes` on map entities that have a `VoxelGenerator` but lack them.
2. **`update_chunks`** (lifecycle.rs:93-156): Transforms target positions into map-local chunk coords, builds cubic desired set per target (cached until chunk boundary crossing), unions per map, evicts out-of-range chunks (saving dirty ones to disk), spawns up to `MAX_TASKS_PER_FRAME` generation tasks.
3. **`poll_chunk_tasks`** (lifecycle.rs:321-358): Polls all ready async tasks (no per-frame limit on polling), inserts `ChunkData` into octree, spawns mesh entities.
4. **`despawn_out_of_range_chunks`** (lifecycle.rs:417-446): Despawns mesh entities for unloaded chunks.
5. **`spawn_remesh_tasks`** (lifecycle.rs:449-471): Drains `chunks_needing_remesh` entirely — no throttle. Spawns async mesh tasks for each.
6. **`poll_remesh_tasks`** (lifecycle.rs:474-536): Polls all ready remesh tasks (no per-frame limit), replaces/spawns/despawns mesh entities.

### Async Task Patterns

Both generation and remesh use `AsyncComputeTaskPool::get().spawn(async move { ... })`:
- Generation tasks: `generation.rs:30-72` — tries disk load, falls back to generator closure + greedy mesh
- Remesh tasks: `lifecycle.rs:449-471` — expands paletted data, runs `mesh_chunk_greedy`
- Polling via `check_ready()` (Bevy's zero-cost noop-waker poll)
- Task handles stored in `Vec<Task<T>>`, polled with index-based while loop + `swap_remove`

### Throttling

- `MAX_TASKS_PER_FRAME = 32` (lifecycle.rs:14) — caps new generation task **spawns** per frame
- No cap on total in-flight generation tasks
- No cap on polling — all ready tasks consumed each frame
- No cap on remesh task spawning — all `chunks_needing_remesh` drained at once
- No prioritization — `HashSet` iteration order is arbitrary
- Target caching: desired sets only recomputed when a target crosses a chunk boundary (lifecycle.rs:198-205)

### Server vs. Client

- **Server**: `generates_chunks = true`. `ChunkTarget` on player entities (distance=10) and dummy target (distance=1) drive generation. Also handles `ChunkRequest` messages from clients.
- **Client**: `generates_chunks = false`. `ChunkTarget` on predicted player (distance=10 steady state, distance=4 during transitions). Sends `ChunkRequest` to server for missing chunks, receives `ChunkDataSync` responses. `update_chunks` still runs to compute desired set and evict out-of-range chunks.

### Current Distance Values

| Location | Distance | File |
|---|---|---|
| Server player spawn | 10 | `server/src/gameplay.rs:317` |
| Server dummy NPC | 1 | `server/src/gameplay.rs:82` |
| Server map transition | 10 | `server/src/map.rs:806` |
| Client auto-attach | 10 | `client/src/map.rs:130` |
| Client map transition | 4 | `client/src/map.rs:525` |

### What's Missing vs. Minecraft

| Minecraft Feature | Current Engine |
|---|---|
| Multiple ticket types with different levels | Single `ChunkTarget` with flat distance |
| Load levels (entity ticking, block ticking, border) | Binary loaded/unloaded |
| Level propagation (+1 per chunk) | Flat radius, all chunks equal |
| Ticket expiration/lifetime | No expiration |
| Lowest-level-wins overlap resolution | Union of all desired positions |
| Priority scheduling (closer chunks first) | Arbitrary HashSet iteration order |
| Multi-stage generation pipeline | Single-step: generate → mesh |
| Server-push chunk protocol | Client-request protocol |
| Time-based tick budget | Hard count per frame |
| Per-chunk generation reference counting | No contention protection |
| Incremental level propagation | Full recompute each change |
%% [SUGGESTION] Elegance — This comparison table is useful but conflates "ticket system features" with "optimization features" and "networking protocol changes." These are three separate concerns. A planner would benefit from them being separated: (1) core ticket system (tickets, levels, propagation, load states), (2) scheduling/throttling optimizations (priority queue, caps, batching), (3) networking protocol change (server-push). This separation would make phased implementation clearer.

---

## Mapping Minecraft's System to `voxel_map_engine`

### Concept Mapping

`ChunkTarget` should be replaced entirely with a ticket-based system closer to Minecraft's implementation.
%% [SUGGESTION] Elegance — This section proposes replacing ChunkTarget "entirely" but doesn't describe a migration strategy. A planner needs to know: can this be done incrementally (ticket system wrapping ChunkTarget initially, then replacing it), or must it be a big-bang rewrite? What's the minimum viable ticket system that provides value before all features are implemented?
%% Just rewrite, not incremental transition unless needed by plan phasing

| Minecraft Concept | Engine Equivalent |
|---|---|
| Ticket | New `ChunkTicket` component (replaces `ChunkTarget`) |
| Ticket type | `TicketType` enum field |
| Ticket level | Starting level integer |
| Ticket lifetime | Optional tick counter, decremented each frame |
| Effective chunk level | Computed per-chunk via propagation, stored in a map |
| `ChunkTicketManager` | New system that computes effective levels |
| Entity ticking zone | Chunks with effective level ≤ threshold |
| Block ticking zone | Chunks with level = threshold + 1 |
| Border/generation zone | Chunks with level = threshold + 2..N |
| `loaded_chunks: HashSet<IVec3>` | `chunk_levels: HashMap<IVec3, u32>` |

### Data Structures

**ChunkTicket** (replaces `ChunkTarget`):
```rust
struct ChunkTicket {
    ticket_type: TicketType,
    level: u32,            // starting level (lower = stronger)
    lifetime: Option<u32>, // ticks remaining, None = permanent
}

enum TicketType {
    Player,   // follows entity position
    Forced,   // static position, persists across restarts
    Portal,   // temporary, created on teleport
    Spawn,    // world spawn area
}
```

**Per-chunk level map** (replaces `loaded_chunks: HashSet<IVec3>`):
```rust
// In VoxelMapInstance:
chunk_levels: HashMap<IVec3, ChunkLevel>,

struct ChunkLevel {
    effective_level: u32,
    load_state: LoadState,
}

enum LoadState {
    EntityTicking,  // full gameplay: entity AI, physics, spawning
    BlockTicking,   // block mechanics, entities frozen
    Border,         // data loaded, accessible to neighbors, no ticking
    Inaccessible,   // generation pipeline stages in progress
}
```

### Load States

Use Minecraft's full model to support future levels of simulation differentiation:

| Level | State | What Happens |
|---|---|---|
| ≤ base | EntityTicking | Server: entity AI, physics, spawning. Client: rendered, collidable, animated. |
| base+1 | BlockTicking | Block mechanics run, entities frozen. |
| base+2 | Border | Data in octree, meshed. Accessible to neighbors for padding. No ticking. |
| base+3..N | Inaccessible | Generation pipeline stages in progress. Not accessible for gameplay. |
| N+1+ | Unloaded | Not loaded. |

Where `base` is the ticket's starting level. The exact numeric range (Minecraft's 22-44 vs a simpler 0-based range) is an implementation detail.
%% [SUGGESTION] Coherence — This says load states map relative to `base`, but the Minecraft system uses absolute levels (31 = entity ticking, 32 = block ticking, etc.) not relative. The concept mapping table above also uses absolute thresholds. This paragraph contradicts that by implying relative levels. A planner needs a clear decision: are load state thresholds absolute or relative to the ticket's starting level? Recommend resolving in this doc.

### Level Computation

Use Minecraft's bucket-queue BFS with incremental updates:
1. Maintain a `LevelPropagator` using an array of `HashSet`s indexed by level, with a `min_pending_level` tracker.
2. When tickets are added/removed, queue the change and propagate incrementally — not a full recompute.
3. For each chunk, effective level = minimum across all contributing ticket sources.
4. Map effective level to `LoadState` via thresholds.
5. Diff against previous frame's levels to determine transitions.

Use Chebyshev distance (`max(|dx|, |dy|, |dz|)`) for 3D propagation, extending Minecraft's 2D model.
%% We need to simplify to using only 2d and loading chunks as columns like minecraft does

### Multi-Stage Generation Pipeline

The generation pipeline should become multi-stage, mirroring Minecraft's approach:

| # | Status | Description | Neighbor Requirement |
|---|---|---|---|
| 0 | Empty | Initial allocation | None |
| 1 | Terrain | Base terrain shape (noise, biomes) | None |
| 2 | Features | Trees, ores, structures | 1-ring at Terrain |
| 3 | Light | Lighting calculation | 1-ring at Features |
| 4 | Mesh | Greedy meshing | 1-ring at Light (for padding) |
| 5 | Full | Promoted to gameplay-ready | None |

Each stage has a `neighbor_requirement` defining how many surrounding chunks must be at the previous status. FEATURES needs a 1-chunk ring at TERRAIN because feature placement can affect neighboring chunks (trees crossing boundaries, etc.).

The ticket level determines maximum achievable status: chunks deep in the inaccessible range only advance to early stages, providing neighbor data for closer chunks.

### Client Interaction

Following Minecraft's model: the server controls all chunk loading decisions. The client should NOT compute its own desired chunks or request missing ones. Instead:

1. Server computes effective levels via the ticket system
2. For chunks reaching sufficient level (Border or better), server sends chunk data to client
3. Server sends a "center chunk" update when the player moves
4. Server sends "unload chunk" when chunks leave the sending radius
5. Client renders based on received chunks — passive receiver

This replaces the current `ChunkRequest`/`ChunkDataSync` protocol with a server-push model.
%% [SUGGESTION] Coherence — The server-push model is a major networking change that interacts with Lightyear. How does this integrate with Lightyear's replication/message system? Does Lightyear support server-push chunk streaming natively, or does this need custom channels? A planner needs to know the Lightyear integration surface.

### Priority Scheduling

Replace `HashSet` iteration with a priority queue:
- Primary sort: effective level (lower first — entity-ticking chunks before border chunks)
- Secondary sort: distance to nearest player (closer first)
- Urgent priority bypass for chunks needed by blocked main-thread operations

---

## Optimization Strategies for `voxel_map_engine`

### Current Bottlenecks

| Area | Current Behavior | Problem |
|---|---|---|
| Generation spawning | 32 tasks/frame, arbitrary order | No priority; distant chunks may load before near ones |
| Generation polling | All ready tasks consumed per frame | Unbounded main-thread work per frame |
| Remesh spawning | All `chunks_needing_remesh` drained at once | Unbounded task spawning; mass voxel edits spike |
| Remesh polling | All ready tasks consumed per frame | Unbounded main-thread work per frame |
| Total in-flight tasks | No cap | Memory grows unbounded under sustained load |
| Level computation | N/A (flat model) | Full recompute of desired set on chunk boundary crossing |

### Strategy 1: Per-Frame Work Caps

Limit main-thread work per frame across all phases:

```rust
const MAX_GEN_SPAWNS_PER_FRAME: usize = 32;    // existing, keep
const MAX_GEN_POLLS_PER_FRAME: usize = 16;      // NEW: cap octree insertions + mesh entity spawns
const MAX_REMESH_SPAWNS_PER_FRAME: usize = 16;  // NEW: cap remesh task spawning
const MAX_REMESH_POLLS_PER_FRAME: usize = 16;   // NEW: cap mesh replacements
```

Unprocessed work stays in the queue for next frame. This bounds worst-case frame time from chunk operations.

### Strategy 2: Total In-Flight Task Caps

Limit total concurrent async tasks to prevent memory growth:

```rust
const MAX_PENDING_GEN_TASKS: usize = 128;       // don't spawn new gen if this many in-flight
const MAX_PENDING_REMESH_TASKS: usize = 64;      // don't spawn new remesh if this many in-flight
```

Check `pending.tasks.len()` before spawning. This provides backpressure — if tasks complete slower than they're spawned, spawning pauses until the pool drains.

### Strategy 3: Priority Queue for Generation

Replace `HashSet` iteration with a `BinaryHeap` sorted by:
1. Effective level (lower = higher priority)
2. Distance to nearest ticket source (closer = higher priority)

```rust
struct ChunkWork {
    position: IVec3,
    effective_level: u32,
    distance_to_source: u32,
}

impl Ord for ChunkWork {
    fn cmp(&self, other: &Self) -> Ordering {
        self.effective_level.cmp(&other.effective_level)
            .then(self.distance_to_source.cmp(&other.distance_to_source))
    }
}
```

This ensures entity-ticking chunks load before border chunks, and near chunks load before far ones.

### Strategy 4: Batched Async Work

Instead of spawning one task per chunk, batch multiple chunks into a single async task to reduce task overhead and pool contention:

```rust
const BATCH_SIZE: usize = 4;

// Collect up to BATCH_SIZE positions, spawn one task that generates all of them
let batch: Vec<IVec3> = work_queue.drain(..BATCH_SIZE.min(work_queue.len())).collect();
let task = pool.spawn(async move {
    batch.iter().map(|&pos| generate_chunk(pos, &*generator)).collect::<Vec<_>>()
});
```

Trade-off: larger batches reduce overhead but increase latency for individual chunks. 4-8 chunks per batch is a reasonable starting point.

### Strategy 5: Deferred Level Propagation

Level propagation (BFS from ticket sources) should NOT run every frame. Instead:

1. **Dirty flag**: only recompute when tickets are added, removed, moved, or expired
2. **Incremental updates**: use Minecraft's bucket-queue BFS to update only affected chunks, not the entire map
3. **Amortized propagation**: if the dirty region is large (e.g., teleport), spread the BFS across multiple frames using a `max_steps` parameter (Minecraft's `applyPendingUpdates(maxSteps)` pattern)

```rust
struct TicketLevelPropagator {
    pending_updates_by_level: [HashSet<IVec3>; MAX_LEVEL],
    min_pending_level: usize,
    is_dirty: bool,
}

impl TicketLevelPropagator {
    fn apply_pending_updates(&mut self, max_steps: usize) -> usize { ... }
}
```

### Strategy 6: Remesh Throttling

Instead of draining `chunks_needing_remesh` all at once:

1. Cap remesh spawns per frame (`MAX_REMESH_SPAWNS_PER_FRAME`)
2. Prioritize remesh by proximity to player (visible chunks first)
3. Coalesce rapid mutations: if a chunk is modified multiple times in quick succession, only remesh once after a short delay

```rust
// In spawn_remesh_tasks:
let mut spawned = 0;
let mut positions: Vec<IVec3> = instance.chunks_needing_remesh.iter().copied().collect();
positions.sort_by_key(|pos| distance_to_nearest_player(pos)); // closest first
for chunk_pos in positions {
    if spawned >= MAX_REMESH_SPAWNS_PER_FRAME { break; }
    instance.chunks_needing_remesh.remove(&chunk_pos);
    // ... spawn task ...
    spawned += 1;
}
// Remaining positions stay in chunks_needing_remesh for next frame
```

### Strategy 7: Generation Reference Counting

Prevent concurrent modification of the same chunk (Minecraft's `acquireGeneration` / `releaseGeneration` pattern). The current `pending_positions: HashSet<IVec3>` partially serves this role but only for generation. Extend to cover all phases:

```rust
struct ChunkWorkTracker {
    generating: HashSet<IVec3>,
    remeshing: HashSet<IVec3>,
}
```

Skip spawning a remesh task if the chunk is still generating, and vice versa.

### Comparison: Current vs. Proposed

| Aspect | Current | Proposed |
|---|---|---|
| Gen spawn cap | 32/frame | 32/frame (keep) |
| Gen poll cap | Unlimited | 16/frame |
| Remesh spawn cap | Unlimited | 16/frame |
| Remesh poll cap | Unlimited | 16/frame |
| Total in-flight gen | Unlimited | 128 max |
| Total in-flight remesh | Unlimited | 64 max |
| Spawn order | Random (HashSet) | Priority queue (level, distance) |
| Level propagation | N/A | Incremental BFS, amortized |
| Task granularity | 1 chunk/task | 4-8 chunks/task (batched) |
| Contention protection | pending_positions dedup | Full reference counting |

---

## Code References

- `crates/voxel_map_engine/src/chunk.rs:12-16` — `ChunkTarget` definition
- `crates/voxel_map_engine/src/lifecycle.rs:93-156` — `update_chunks` system
- `crates/voxel_map_engine/src/lifecycle.rs:226-240` — `compute_target_desired` (cubic distance)
- `crates/voxel_map_engine/src/lifecycle.rs:265-289` — `remove_out_of_range_chunks`
- `crates/voxel_map_engine/src/lifecycle.rs:291-314` — `spawn_missing_chunks` (no priority)
- `crates/voxel_map_engine/src/lifecycle.rs:14` — `MAX_TASKS_PER_FRAME = 32`
- `crates/voxel_map_engine/src/lifecycle.rs:321-358` — `poll_chunk_tasks` (no poll cap)
- `crates/voxel_map_engine/src/lifecycle.rs:449-471` — `spawn_remesh_tasks` (no spawn cap)
- `crates/voxel_map_engine/src/lifecycle.rs:474-536` — `poll_remesh_tasks` (no poll cap)
- `crates/voxel_map_engine/src/lifecycle.rs:33-43` — `PendingRemeshes`
- `crates/voxel_map_engine/src/instance.rs:28-37` — `VoxelMapInstance` fields
- `crates/voxel_map_engine/src/instance.rs:130-183` — `set_voxel` and `update_neighbor_padding`
- `crates/voxel_map_engine/src/config.rs:12-29` — `VoxelGenerator`, `VoxelMapConfig`
- `crates/voxel_map_engine/src/generation.rs:22-27` — `PendingChunks`
- `crates/voxel_map_engine/src/generation.rs:30-72` — `spawn_chunk_gen_task`
- `crates/server/src/gameplay.rs:313-318` — Server player ChunkTarget insertion
- `crates/client/src/map.rs:117-132` — Client ChunkTarget auto-attach
- `crates/client/src/map.rs:135-202` — Client `request_missing_chunks`

## Historical Context (from doc/)

- `doc/research/2026-03-11-minecraft-world-sync-protocol.md` — Minecraft's chunk data transfer protocol (paletted sections, block change packets). Directly related: describes how chunks are sent to clients.
- `doc/research/2026-01-03-server-chunk-visibility-determination.md` — Early research on per-client chunk visibility. Predates current ChunkTarget system.
- `doc/research/2026-01-03-bevy-ecs-chunk-visibility-patterns.md` — ECS patterns for chunk visibility that informed current design.
- `doc/plans/2026-02-28-voxel-map-engine.md` — Original voxel_map_engine implementation plan.
- `doc/plans/2026-01-04-transform-based-chunk-visibility.md` — Plan that introduced transform-based (vs camera-based) chunk loading.

## Related Research

- `doc/research/2026-03-18-procedural-map-generation.md` — Terrain generation system
- `doc/research/2026-03-20-performance-profiling-tools.md` — Tracy profiling for terrain performance

## Sources

- [Chunk — Minecraft Wiki](https://minecraft.wiki/w/Chunk)
- [Spawn chunk — Minecraft Wiki](https://minecraft.wiki/w/Spawn_chunk)
- [/forceload — Minecraft Wiki](https://minecraft.wiki/w/Commands/forceload)
- [Chunk loading overview (empirical 1.14.4 testing) — GitHub Gist](https://gist.github.com/Drovolon/24bfaae00d57e7a8ca64b792e14fa7c6)
- [ChunkTicketManager — Fabric Yarn API](https://maven.fabricmc.net/docs/yarn-22w17a+build.3/net/minecraft/server/world/ChunkTicketManager.html)
- [Chunk Loading — Technical Minecraft Wiki](https://techmcdocs.github.io/pages/GameMechanics/ChunkLoading/)
- [ChunkTicketManager — Yarn 1.21.5 API](https://maven.fabricmc.net/docs/yarn-1.21.5-pre3+build.1/net/minecraft/server/world/ChunkTicketManager.html)
- [ChunkTicketType — Yarn 1.21.5 API](https://maven.fabricmc.net/docs/yarn-1.21.5+build.1/net/minecraft/server/world/ChunkTicketType.html)
- [LevelPropagator — Yarn 1.17-pre1 API](https://maven.fabricmc.net/docs/yarn-1.17-pre1+build.1/net/minecraft/world/chunk/light/LevelPropagator.html)
- [ChunkHolder — Forge 1.17.1 JavaDocs](https://nekoyue.github.io/ForgeJavaDocs-NG/javadoc/1.17.1/net/minecraft/server/level/ChunkHolder.html)
- [ChunkMap — Forge 1.17.1 JavaDocs](https://nekoyue.github.io/ForgeJavaDocs-NG/javadoc/1.17.1/net/minecraft/server/level/ChunkMap.html)
- [ChunkStatus — Yarn 1.19 API](https://maven.fabricmc.net/docs/yarn-1.19-pre1+build.6/net/minecraft/world/chunk/ChunkStatus.html)
- [ChunkStatus.java decompiled 1.13.2 — Akarin Project](https://github.com/Akarin-project/Minecraft/blob/master/1.13.2/spigot/net/minecraft/server/ChunkStatus.java)
- [PaperMC World and Chunk Management — DeepWiki](https://deepwiki.com/PaperMC/Paper/3.3-world-and-chunk-management)
- [Fabric API PR #4541 — CHUNK_LEVEL_TYPE_CHANGE event](https://github.com/FabricMC/fabric-api/pull/4541)
- [Java Edition Protocol — Minecraft Wiki](https://minecraft.wiki/w/Java_Edition_protocol/Packets)
- [PaperMC chunk priority improvements — commit 3dc5ad3](https://github.com/PaperMC/Paper/commit/3dc5ad343fbcd025a13592d137e40abe6ca4ab5b)
- [PaperMC chunk system rewrite — PR #8177](https://github.com/PaperMC/Paper/pull/8177)
- [ChunkTaskScheduler — Yarn API 1.21.2](https://maven.fabricmc.net/docs/yarn-1.21.2+build.1/net/minecraft/server/world/ChunkTaskScheduler.html)
- [ChunkGenerationStep.Builder — Yarn API 1.21.2](https://maven.fabricmc.net/docs/yarn-1.21.2+build.1/net/minecraft/world/chunk/ChunkGenerationStep.Builder.html)
- [C2ME Concurrent Chunk Management Engine — Modrinth](https://modrinth.com/mod/c2me-fabric)

## Resolved Questions

1. **How many load states?** Use Minecraft's full model (EntityTicking, BlockTicking, Border, Inaccessible) to support future simulation differentiation. Decision: match Minecraft.

2. **ECS components vs data structure?** Tickets should be ECS components for query ergonomics. Level propagation and the resulting `chunk_levels` map live on `VoxelMapInstance` (or a dedicated `ChunkTicketManager` resource per map). The propagator runs as a system that reads ticket components and writes to the level map. This separates the ECS-friendly ticket interface from the computationally intensive propagation, which can be deferred/amortized.

3. **Client interaction?** Follow Minecraft: server-push model. Server computes levels, sends chunk data to clients for chunks at sufficient level. Client is a passive receiver. Replaces current `ChunkRequest`/`ChunkDataSync` protocol.

4. **Distance metric?** Chebyshev (`max(|dx|, |dy|, |dz|)`). Matches Minecraft and the current cubic volume approach.

5. **Multi-stage generation?** Yes. Stages: Empty → Terrain → Features → Light → Mesh → Full. Each stage has neighbor requirements enabling the inaccessible level range.

## Open Questions

1. **Exact level range?** Should we use Minecraft's 22-44 range, or a simpler 0-based range? A 0-based range is cleaner but loses compatibility with Minecraft documentation references.
%% 0-based
2. **Batch size tuning?** The optimal chunk batch size (Strategy 4) depends on generation cost per chunk and task pool overhead. Needs profiling with Tracy.
3. **Amortization budget?** How many BFS steps per frame for level propagation (Strategy 5)? Needs profiling to determine the right balance between responsiveness and frame budget.
4. **3D propagation cost?** Minecraft propagates in 2D (4/8 neighbors). Our 3D world adds a third axis (6/26 neighbors), increasing propagation volume cubically. May need a tighter max level or cylindrical propagation (2D horizontal + limited vertical).
%% Use 2d like minecraft
%% [VIOLATION] Coherence — Open question #4 is a planning blocker. The choice between 6-neighbor vs 26-neighbor vs cylindrical propagation fundamentally changes the algorithm, data structures, and performance characteristics. A planner cannot write a concrete implementation plan without this resolved. Recommend: resolve with a decision and rationale (cylindrical is the most likely winner given that vertical render distance is typically much smaller than horizontal).
%% [SUGGESTION] Coherence — Open question #1 is also a planning blocker. The level range determines every threshold constant in the system. Recommend: just use 0-based — there is no value in Minecraft compatibility for an unrelated game engine.
%% [SUGGESTION] Coherence — The doc presents the full target state but doesn't suggest implementation phases. A planner needs to know: what's the minimum viable ticket system? Suggested phasing: (a) ticket system + level propagation replacing ChunkTarget, (b) priority scheduling, (c) multi-stage generation, (d) server-push networking. Each is independently valuable. Do not plan out full work, make a short section on it.


## Missing Information for Planning

%% [VIOLATION] Coherence — The following gaps would prevent a planner from writing an actionable implementation plan:

%% 1. **Integration with Multi-map instances** — The doc does not consider how multiple map instances will be supported in this system. Needs further research, consideration and elaboration.
%% 2. **Which ticket types does this project actually need now?** The doc maps all of Minecraft's types but doesn't prioritize. The project has players and map transitions — does it need Portal/Forced/Spawn tickets today, or just Player tickets? A planner needs a "must have vs nice to have" breakdown. Just Players, map transitions, npcs for now

%% 3. **What do load states actually DO in this project?** Minecraft's EntityTicking/BlockTicking/Border map to specific Minecraft mechanics (redstone, mob AI, etc.). This project has none of those. The doc doesn't define what each load state means for THIS game's mechanics. Without that, load states are unused complexity. A planner needs concrete behavior per state. EntityTicking should be used for full simulation where there are players. BlockTicking should be for chunks with only NPCs, or other non-player mechanics. Border is the same

%% 4. **Integration with existing systems**: Which systems currently depend on `loaded_chunks: HashSet<IVec3>` or `ChunkTarget`? The doc references code locations but doesn't enumerate all consumers. A planner needs the full dependency graph to understand the blast radius.

%% 5. **Testing/verification strategy**: How will the planner verify that the ticket system produces correct level maps? No mention of unit tests for the propagator, integration tests for ticket lifecycle, or runtime verification approach.
