# Design Discussion

## Current State

Chunk size is a workspace-wide compile-time constant `CHUNK_SIZE: u32 = 16` (`types.rs:9`), with three independent encodings of the padded size `18`: the type `PaddedChunkShape = ConstShape3u32<18, 18, 18>` (`types.rs:7`), the dead constant `PADDED_CHUNK_SIZE` (`types.rs:10`), and the file-local `PADDED_VOLUME = 18*18*18` (`palette.rs:6`). One additional hardcoded literal derives from `CHUNK_SIZE`: `SurfaceHeightMap.heights: [Option<f64>; 256]` (`config.rs:43`).

The constant feeds coordinate math (`lifecycle.rs:977,981`, `api.rs:38,44,118,124,149,155,167-169`, `instance.rs:133,145,164,166,171,178`), terrain fill loops (`terrain.rs:236,266-280`, `generation.rs:185-193`), memory allocation (`terrain.rs:231,594`), mesh bounds (`meshing.rs:12,20,22-23`), and world-height vertical extent via `DEFAULT_COLUMN_Y_MIN/MAX = -8..8` (`ticket.rs:141-142`).

`PaddedChunkShape::linearize([x, y, z])` is used as a **static** method across 8 production sites (research Addendum A). None of these sites currently hold a shape instance. The target for threading runtime dispatch is `VoxelMapInstance`, which is already reachable at every call site.

`VoxelMapConfig` carries per-map configuration (`config.rs:54-67`) and is not a replicated component. Its fields propagate to the client via `MapTransitionStart` (`transition.rs:24-30`). `ChunkDataSync` (`protocol/src/map/chunk.rs:12-16`) transmits `chunk_pos` as raw `IVec3` with no chunk-size metadata; both sides rely on the shared compile-time constant.

The meshing pipeline (`meshing.rs:11-26`) uses `greedy_quads` from `block_mesh`, which is generic over any `S: Shape<3, Coord = u32>` (research Addendum B). ndshape provides `RuntimePow2Shape<u32, 3>` which implements that trait with bit-shift-based linearization — essentially free at runtime for power-of-two dimensions.

`SurfaceHeightMap` (`config.rs:41-44`) is a per-chunk intermediate built on the main thread (`generation.rs:178`), moved by value into the async feature-placement task (`generation.rs:109-143`), and consumed by `place_features` (`terrain.rs:366-429`) for spawn-Y lookup, elevation filtering, and slope rejection. It is chunk-local with no cross-chunk state.

Pre-generated chunk save files exist under `assets/sprites/humanoid/worlds/overworld/terrain/` but are **disposable** — no backward-compat migration needed.

## Desired End State

Each `VoxelMapConfig` carries its own `chunk_size: u32` (power-of-two) and `column_y_range: (i32, i32)`. Two maps loaded simultaneously can use different chunk dimensions; coordinate math, meshing, terrain generation, persistence, and networking all dispatch on the per-map value rather than a global constant.

**Verification:**
1. `cargo check-all` passes — the compile-time constant `CHUNK_SIZE` is removed, and every prior call site resolves via a runtime value threaded through `VoxelMapInstance` or `VoxelMapConfig`.
2. `cargo server` + `cargo client` — overworld streams and meshes correctly with chunk_size=16 (baseline regression check).
3. A second map (homebase or arena) configured with chunk_size=32 loads, meshes, and renders without seams or coordinate errors.
4. Client transitions between maps with different chunk sizes via `MapTransitionStart` and reconstructs the correct config.
5. Slope-clipping at chunk edges (`terrain.rs:447`) is removed; feature placement uses the padded height map.

## Patterns to Follow

- **Config propagation path** (`server/src/map.rs:97-126`, `transition.rs:24-30`, `client/src/map.rs:499-527`): New `VoxelMapConfig` fields travel server spawn → terrain def application → `MapTransitionStart` → client reconstruction. Chunk size and column Y range follow this established path.
- **Padding convention** (research cross-cutting #5): `padded = chunk + 2`, world coord formula `chunk_pos * chunk_size + padded_index - 1`. This invariant is preserved; every `- 1` stays.
- **Negative-correct coordinate math** (`api.rs:165-170`): `div_euclid` for voxel → chunk, `floor` for float → chunk. Both generalize trivially to runtime divisors.
- **ndshape runtime shape** (Addendum B): `RuntimePow2Shape::<u32, 3>::new([padded, padded, padded])` implements `Shape<3, Coord = u32>` directly and passes to `greedy_quads` unchanged. Stored once per map instance (see Design Decision #2).
- **Boxed heavy fields moved into async tasks** (`generation.rs:109-143`): async task payloads that own large buffers should be heap-allocated before the move to avoid a stack memcpy of the full struct. `SurfaceHeightMap.heights` adopts `Box<[Option<f64>]>` for this reason.
- **Single source of truth for derived constants**: The current three independent `18` literals are an anti-pattern (research cross-cutting #1). Every derived size (`padded_size`, `padded_volume`, mesh max bounds) must be computed from the one stored `chunk_size` or the one stored shape instance.

**Pattern to NOT follow:** `PaddedChunkShape::linearize(p)` static-method dispatch. Every call site moves to `shape.linearize(p)` instance-method dispatch with the shape read from `VoxelMapInstance`.

## Design Decisions

1. **Runtime `u32` on `VoxelMapConfig`, not const generic.** — Option A from Q1. Simpler implementation, supports two maps with different chunk sizes loaded simultaneously, and `RuntimePow2Shape` recovers the const-shape linearize performance via bit shifts. Const generic propagation through `ndshape::ConstShape3u32<N, N, N>` with computed expressions is not reliably supported by current stable Rust.

2. **`VoxelMapInstance` owns the shape instance.** — `VoxelMapInstance` is accessible at every static-dispatch call site (research Addendum A) and already travels alongside chunk data. It stores `chunk_size: u32`, `padded_size: u32`, and `shape: RuntimePow2Shape<u32, 3>` (constructed once at map spawn). The shape is cheap to copy (3× u32) and can be cloned into async tasks that need it for meshing or terrain fill.

3. **Cubic chunks only (single `u32`).** — Option A from Q2. Task phrasing "chunk dimensions" is satisfied by a scalar; non-cubic adds `PaddedChunkShape` construction complexity and changes `SurfaceHeightMap` indexing without clear user benefit. Revisitable if a future map needs non-cubic.

4. **Powers of 2 only, validated at `VoxelMapConfig::new`.** — Enables `RuntimePow2Shape` and bit-shift `div_euclid`. `debug_assert!(chunk_size.is_power_of_two() && chunk_size >= 8)` in the constructor. Minimum 8 is a sanity floor (smaller chunks would spend disproportionate time on padding borders).

5. **`SurfaceHeightMap.heights: Box<[Option<f64>]>`, padded.** — Boxed slice sized `padded_size * padded_size = (chunk_size + 2)²` at construction. No `MAX_CHUNK_SIZE` cap, no waste. Iterating the full padded XZ range in `build_surface_height_map` produces heights at the 1-voxel border for free (voxel data is already padded). This eliminates the edge-clipping in `exceeds_slope` (`terrain.rs:447`). Candidate indexing becomes `(local_x + 1, local_z + 1)` via a `heights.at(x, z)` helper.

6. **Column Y bounds become per-map `(i32, i32)` on `VoxelMapConfig`.** — World height scales with chunk size; a map using chunk_size=32 gets double the vertical extent of chunk_size=16 if Y bounds stay at `-8..8`. Store `column_y_range: (i32, i32)` on `VoxelMapConfig`, defaulting to `(-8, 8)`. Replaces the `DEFAULT_COLUMN_Y_MIN/MAX` consts in `ticket.rs:141-142`. `column_to_chunks` takes the range from the map's config at call time.

7. **Network: transmit chunk_size in both `MapTransitionStart` and `ChunkDataSync`.** — Option B from Q4. Adding to `MapTransitionStart` is required for client config reconstruction; adding to `ChunkDataSync` lets the client validate chunk-size agreement per chunk (a mismatched value is a hard error, not silent corruption). `column_y_range` also rides `MapTransitionStart`.

8. **Bump `CHUNK_SAVE_VERSION` and gate load on matching chunk_size.** — The serialized `PalettedChunk::Indirect.len` bakes in the padded volume. New saves embed the current chunk size; loading a save with a mismatched chunk size is a hard error (save is invalidated per your "disposable maps" guidance).

## What We're NOT Doing

- **Non-cubic chunks** (e.g. 16x32x16). Single scalar only. Revisitable later.
- **Runtime mutability** of a live map's chunk size. Chunk size is set at map spawn and immutable for the map's lifetime.
- **Migration of existing save files.** `worlds/overworld/terrain/*.bin` becomes unreadable; acceptable per your Q1 response.
- **Replacing `div_euclid` with manual bit shifts** for coordinate math. The compiler-generated code for `i32::div_euclid` with a power-of-two runtime divisor is already tight; premature optimization.
- **Per-chunk (rather than per-map) chunk size.** One size per map instance, not per chunk. Prevents combinatorial explosion in the coordinate math and preserves one-to-one shape dispatch.
- **Fixing the Y-boundary slope check gap.** The padding fix covers X/Z neighbors within the same Y chunk; the Y-boundary case (slope check across vertically adjacent chunks) is pre-existing and out of scope.
- **Cross-chunk feature placement.** Features remain chunk-local; chunk-corner edge cases remain as-is.

## Open Risks

1. **PalettedChunk serialization format changes implicitly.** `PalettedChunk::Indirect.len` and `data: Vec<u64>` length depend on padded volume. Mixing serialized chunks across chunk-size boundaries will corrupt silently if the save version gate fails. The per-`ChunkDataSync` chunk_size field is the belt-and-braces check; version bump is the authoritative gate.

2. **Shape instance lifetime in async tasks.** Meshing and terrain fill tasks currently capture no shape. They'll need to capture a cloned `RuntimePow2Shape<u32, 3>` (12 bytes) from `VoxelMapInstance` before spawning. Any task that outlives its source map's despawn needs careful audit (in-flight tasks should complete with the captured shape, not the now-gone map's).

3. **Feature-placement behavior change at chunk edges.** Padding the slope check is strictly more correct, but candidates previously surviving a clipped check may now be rejected. Worlds regenerate differently — acceptable given disposable-world guidance.

4. **`PaddedChunkShape` type alias removal breadth.** Every test file referencing `PaddedChunkShape::USIZE` for `vec![Air; …]` allocation needs updating. Research flagged "many test files" — expect broad mechanical changes.

5. **Homebase and arena spawn sites.** Currently only overworld is fully wired up. Introducing chunk_size variance means any existing homebase/arena spawn path (`server/src/map.rs:1048`) needs config updates too.
