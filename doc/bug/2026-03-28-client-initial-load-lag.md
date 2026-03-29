---
date: 2026-03-28T08:53:27-07:00
researcher: Claude
git_commit: 7025d12e
branch: master
repository: bevy-lightyear-template
topic: "Client initial chunk load lag tuning"
tags: [performance, chunk-ticket, client, meshing, network]
status: investigating
last_updated: 2026-03-28
last_updated_by: Claude
---

# Performance: Client Initial Chunk Load Lag

**Date**: 2026-03-28T08:53:27-07:00
**Git Commit**: 7025d12e
**Branch**: master

## User's Prompt
Tune the chunk ticket system to reduce lag during initial load for the client.

## Summary

The client receives chunks from the server via network push. During initial load, many chunks arrive simultaneously and each is meshed synchronously on the main thread inside `handle_chunk_data_sync`. This is the primary suspected bottleneck. The full pipeline has been mapped below.

## Architecture Overview

### Client chunk pipeline (no `ChunkGenerationEnabled`)

1. **Server generates chunks** — `update_chunks` (server) runs propagator, spawns async gen tasks, polls results
2. **Server pushes to client** — `push_chunks_to_clients` sends up to **16 chunks/tick** (`MAX_CHUNK_SENDS_PER_TICK`) per player, closest first
3. **Client receives** — `handle_chunk_data_sync` (client/src/map.rs:135):
   - Drains `MessageReceiver<ChunkDataSync>` — **all** messages in a single frame
   - For each chunk: decompress palette → mesh synchronously via `mesh_chunk_greedy` → insert mesh + material → spawn entity
4. **Client remesh pipeline** — `spawn_remesh_tasks` / `poll_remesh_tasks` run with budget (4ms), but only for *edited* chunks (not initial load)
5. **Client despawn** — `despawn_out_of_range_chunks` cleans up chunks no longer in `chunk_levels`

### Key throttling constants (lifecycle.rs)

| Constant | Value | Scope |
|---|---|---|
| `CHUNK_WORK_BUDGET_MS` | 8ms | Per-map per-frame budget for gen/remesh |
| `MAX_GEN_SPAWNS_PER_FRAME` | 256 | Server gen task spawns |
| `MAX_GEN_POLLS_PER_FRAME` | 256 | Server gen task polls |
| `MAX_REMESH_SPAWNS_PER_FRAME` | 256 | Remesh task spawns |
| `MAX_REMESH_POLLS_PER_FRAME` | 256 | Remesh task polls |
| `MAX_PENDING_GEN_TASKS` | 512 | In-flight gen cap |
| `MAX_PENDING_REMESH_TASKS` | 512 | In-flight remesh cap |
| `MAX_CHUNK_SENDS_PER_TICK` | 16 | Server→client chunks/tick |
| `LOAD_LEVEL_THRESHOLD` | 20 | Max level for "loaded" column |
| Player default radius | 200 (ticket.rs:27) | Chebyshev radius in chunks |

### Server push throttling

`push_chunks_to_clients` (server/src/map.rs:680):
- Per client per tick: iterates all loaded columns, collects unsent chunks, sorts by distance, sends up to 16
- Each `ChunkDataSync` message contains the full `PalettedChunk` data
- No bandwidth/byte-size awareness — just a flat count cap

### Client receive path (the hot path for initial load)

`handle_chunk_data_sync` (client/src/map.rs:135):
- Drains ALL incoming messages in one frame (no throttling)
- Each chunk: `sync.data.to_voxels()` (palette decompression) + `mesh_chunk_greedy(&voxels)` (greedy meshing)
- Both operations are **synchronous on the main thread**
- No budget check, no cap on chunks processed per frame
- Entity spawn + mesh asset insertion per chunk

### Client-side VoxelPlugin systems (no generation)

Since client has no `ChunkGenerationEnabled`:
- `update_chunks` / `poll_chunk_tasks` — **skipped**
- `reset_chunk_budgets` — **runs** (fallback reset)
- `spawn_remesh_tasks` / `poll_remesh_tasks` — run but only for `chunks_needing_remesh` (edit-driven)

## Potential Bottlenecks (Pre-Profiler)

### ~~B1: Synchronous meshing in `handle_chunk_data_sync`~~ (Fixed by F1)
~~All received chunks are meshed on the main thread in a single frame. With 16 chunks arriving per server tick × however many frames of backlog, this is likely the primary frame spike source.~~

### B2: Server send rate (`MAX_CHUNK_SENDS_PER_TICK = 16`)
16 chunks/tick may be too slow OR too fast depending on client's ability to process. If server sends faster than client can mesh, messages accumulate in the receive buffer and cause increasingly large frame spikes.

### ~~B3: No client-side receive budget~~ (Fixed by F1)
~~Unlike gen/remesh which have `ChunkWorkBudget` (4ms), `handle_chunk_data_sync` has no budget — it processes everything available.~~
Receive is now cheap (data insertion only). Meshing is budget-throttled via existing `spawn_remesh_tasks`.

### B4: Player default radius = 200 chunks
This means up to ~400×400 columns = 160,000 columns × 16 Y slices = 2.56M chunks total. Initial load of this radius is enormous.
**Note**: `LOAD_LEVEL_THRESHOLD = 20` limits which columns are actually loaded, so effective radius is `min(radius, LOAD_LEVEL_THRESHOLD) = 20` columns → ~41×41 = 1,681 columns × 16 Y = ~26,896 chunks.

### B5: Palette decompression + meshing cost per chunk (Partially fixed by F1)
~~Each chunk goes through `to_voxels()` (bit-unpacking) then `mesh_chunk_greedy()`.~~ Meshing is now async. However, `to_voxels()` + `ChunkData::from_voxels()` (decompress-then-recompress) still runs synchronously in the receive handler. Much cheaper than meshing but worth monitoring.

### ~~B6: Entity spawn overhead~~ (Fixed by F1)
~~Each chunk becomes a Bevy entity with Mesh3d + MeshMaterial3d + Transform. Spawning thousands of entities in one frame may cause ECS overhead.~~
Entity spawning now happens in `poll_remesh_tasks`, throttled by `MAX_REMESH_POLLS_PER_FRAME` (32).

## Hypotheses

### H1: Synchronous meshing in `handle_chunk_data_sync` causes frame spikes during initial load

**Hypothesis:** The dominant source of client initial-load lag is `handle_chunk_data_sync` performing `to_voxels()` (palette decompression) and `mesh_chunk_greedy()` synchronously on the main thread for every chunk received in a frame. With the server sending up to 16 chunks/tick, and the client draining all buffered messages in one frame, this creates frame spikes proportional to the number of chunks received.

**Prediction:** Profiler will show `handle_chunk_data_sync` consuming a large fraction (>50%) of frame time during initial load, with `mesh_chunk_greedy` as the dominant sub-call. Frame time will correlate with number of chunks processed that frame.

**Test:** Check Tracy/profiler output for:
1. Total time spent in `handle_chunk_data_sync` per frame during initial load
2. Per-chunk cost breakdown: `to_voxels()` vs `mesh_chunk_greedy()` vs entity spawn
3. Number of chunks processed per frame (count of `ChunkDataSync` messages drained)

**Proposed fix (if validated):** Replace inline meshing with async pipeline:
- `handle_chunk_data_sync` inserts chunk data + adds position to `chunks_needing_remesh`
- Existing `spawn_remesh_tasks` / `poll_remesh_tasks` handles meshing async, budget-throttled, distance-prioritized
- Net code change: ~15 lines removed from `handle_chunk_data_sync`, ~2 lines added

**Decision:** Approved — profiler confirms mean 23ms/frame, P99 149ms, total 3.67s in handle_chunk_data_sync during initial load

## Fixes

### F1: Move client chunk meshing to async remesh pipeline

**Root Cause:** `handle_chunk_data_sync` performs palette decompression and greedy meshing synchronously on the main thread for every received chunk. Mean 23ms/frame, P99 149ms.

**Fix:** `handle_chunk_data_sync` should only insert chunk data and mark the position for async meshing. The existing `spawn_remesh_tasks` / `poll_remesh_tasks` pipeline (already running on client) handles the rest — async, budget-throttled (4ms), distance-prioritized.

**Changes:**
- `client/src/map.rs` `handle_chunk_data_sync`: Remove inline `to_voxels()`, `mesh_chunk_greedy()`, entity spawn. Add `instance.chunks_needing_remesh.insert(sync.chunk_pos)`.
- `lifecycle.rs` `spawn_remesh_tasks`: Needs to also spawn the chunk entity (not just swap mesh on existing), since initial-load chunks have no pre-existing entity. Already handles this case — the `(Some(mesh), None)` branch in `poll_remesh_tasks` spawns a new entity.

**Risk:** Chunks appear 1-2 frames after data arrives instead of same-frame. During initial load this is strictly better (no spike). For single-chunk edits mid-game, remesh is already async so no behavioral change there.

**Decision:** Approved

## Solutions

*Pending.*
