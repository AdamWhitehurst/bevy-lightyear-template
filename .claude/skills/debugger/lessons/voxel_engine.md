# Voxel Engine Debugging Lessons

## ChunkWorkBudget Requires Reset Each Frame

`ChunkWorkBudget` stores an `Instant` and a duration. `has_time()` checks elapsed time since that instant. If `reset()` is never called, the budget permanently expires after ~4ms from creation.

`update_chunks` (gated on `ChunkGenerationEnabled`) is the primary reset site. Any system gated behind `ChunkGenerationEnabled` that touches the budget will not run on clients. The remesh pipeline (`spawn_remesh_tasks`, `poll_remesh_tasks`) runs unconditionally but checks `budget.has_time()` — so a missing reset silently starves it.

**Pattern:** When gating a system that owns shared per-frame state (budgets, counters), ensure the state is still maintained for ungated consumers. Use `run_if(not(condition))` for a fallback reset.

## Stale Async Task Results After Column Unload

`remove_column_chunks` removes chunk data from the octree but does NOT cancel in-flight async tasks or clear `tracker.generating`. When a Features task completes after its column was unloaded, `handle_completed_chunk` receives a result with `chunk_data: None` for a chunk that no longer exists.

**Root cause example:** The DummyTarget NPC has physics and spawns before terrain meshes are ready. It drifts across a column boundary due to lack of collision geometry, causing `set_source` → `queue_invalidation` → `diff.unloaded` for old columns while Features tasks are still in-flight.

**Fix:** `handle_completed_chunk` gracefully handles missing chunk data for Features results (returns early with a trace log). The same race can affect Mesh stage results but those carry `chunk_data: Some(...)` so they insert data for an unloaded column — currently harmless but worth noting.

**Pattern:** Any async task pipeline with in-place updates (no data payload in result) must guard against the target being removed while the task runs. Either cancel tasks on unload, or handle missing targets at poll time.

## Gated Systems Can Starve Shared Infrastructure Consumers

When a system populates shared state (propagator sources, caches, counters) but is gated behind a resource/condition, ungated consumers of that state will see empty/uninitialized values. Check whether each consumer of shared per-frame state has its data source active in all execution contexts (server vs client, different app states, etc).

## Lightyear Room Removal Is Not Instantaneous

Deferred `commands.trigger(RoomEvent)` for room membership changes means lightyear may replicate entities from the old room in the same or subsequent frames after the removal is triggered. Any client-side cleanup of foreign entities must account for late arrivals, not just entities present at cleanup time.
