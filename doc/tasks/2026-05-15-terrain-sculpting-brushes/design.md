# Design Discussion

## Current State

Dev terrain editing exists as an editing mode, but not as a usable terrain tool. `EditingMode::Terrain` is the default shared mode, and client terrain input is routed only while that mode is active (`crates/dev/src/state.rs:5-13`, `crates/client/src/map.rs:165-185`). The Terrain tab currently renders an empty body inside the spawn panel (`crates/dev/src/panels/spawn.rs:227-229`).

Voxel editing is single-voxel end to end: client prediction, `VoxelEditRequest`, server validation/application, `VoxelWorld::set_voxel`, and `VoxelMapInstance::set_voxel` all carry one position and one voxel value (`crates/client/src/map.rs:105-118`, `crates/client/src/map.rs:401-430`, `crates/server/src/map.rs:1352-1402`, `crates/voxel_map_engine/src/api.rs:52-65`, `crates/voxel_map_engine/src/instance.rs:95-118`).

Chunk-local bookkeeping, dirty marking, padding updates, and remesh scheduling are already encapsulated below the public voxel API (`crates/voxel_map_engine/src/instance.rs:95-148`). Server broadcasts already group multiple accepted concrete changes by chunk and can send `SectionBlocksUpdate` for 2+ changes in one chunk (`crates/protocol/src/map/voxel.rs:41-47`, `crates/server/src/map.rs:751-759`, `crates/server/src/map.rs:1430-1471`).

Voxel material selection is numeric today: `WorldVoxel::Solid(u8)` / `VoxelType::Solid(u8)`, terrain assets use numeric biome material IDs, and no UI-visible material registry was found (`crates/voxel_map_engine/src/types.rs:6-12`, `crates/voxel_map_engine/src/types.rs:126-148`, `crates/voxel_map_engine/src/terrain.rs:75-84`, `assets/terrain/overworld.terrain.ron:18-26`).

No general undo/redo stack exists. The closest pattern is client prediction storing old/new voxel state and authoritative reject rollback (`crates/client/src/map.rs:112-118`, `crates/protocol/src/map/voxel.rs:32-39`).

Vision fit: this is a dev/admin world-building tool for editable home-base/overworld/stage content, aligning with Stage Editing and the Home-Base / Overworld / Instanced Stages separation in `VISION.md`.

## Desired End State

The Dev plugin exposes a dedicated terrain sculpting interface with:

- Brush shape, brush size, selected voxel material, and mode controls.
- Brush mode: add or remove voxels in the selected brush volume.
- Paint mode: change only existing solid voxels; never create new solid voxels.
- A mouse-following wireframe brush preview showing the exact edit footprint before applying.
- Undo/redo for acknowledged terrain edits.
- A separate terrain panel module rather than expanding `dev/src/panels/spawn.rs`.
- A voxel-map operation API that applies one logical edit over many world-space voxel positions, including positions spanning multiple chunks, while preserving dirty/remesh/padding behavior.

Correctness is verified when a single brush stroke can affect multiple chunks, all affected chunks remesh/persist/broadcast correctly, paint mode does not add voxels, undo/redo restores prior voxel state, and the preview footprint matches the applied operation.

## Patterns to Follow

- Keep panel modules under `crates/dev/src/panels/`, feature-gated in `panels/mod.rs`, with each panel registering its own resource/systems through a plugin (`crates/dev/src/panels/mod.rs:1-8`, `crates/dev/src/panels/spawn.rs:151-162`).
- Re-export shared dev state from `crates/dev/src/lib.rs` when other crates need it (`crates/dev/src/lib.rs:8-12`).
- Route terrain input through `EditingMode::Terrain`; do not let terrain tools compete with object placement/selection modes (`crates/client/src/map.rs:165-185`).
- Keep map identity explicit at boundaries: `MapInstanceId` for protocol/server semantics, local map `Entity` for voxel APIs (`crates/protocol/src/map/types.rs:9-34`, `crates/voxel_map_engine/src/api.rs:10-16`).
- Preserve existing raycast conventions: removing/painting targets the hit voxel; adding targets `hit.position + normal.as_ivec3()` (`crates/client/src/map.rs:394-405`, `crates/client/src/map.rs:86-100`).
- Reuse server-side chunk grouping for concrete broadcasts rather than making clients understand chunk batching (`crates/server/src/map.rs:751-759`, `crates/server/src/map.rs:1452-1471`).

Patterns not to follow:

- Do not keep terrain-specific UI embedded in `spawn.rs`; the current empty Terrain tab is a placeholder, not the target architecture (`crates/dev/src/panels/spawn.rs:227-229`).
- Do not expose multi-voxel editing as caller-managed repeated chunk/local mutations; current single-position APIs hide chunk bookkeeping, and the new logical operation should preserve that boundary (`crates/voxel_map_engine/src/api.rs:52-65`, `crates/voxel_map_engine/src/instance.rs:95-118`).
- Do not invent labeled material definitions for this task; research found only numeric material IDs and terrain definitions, not a material registry (`crates/voxel_map_engine/src/types.rs:6-12`, `crates/protocol/src/terrain/registry.rs:7-20`).

## Design Decisions

1. **Terrain UI boundary**: create a dedicated terrain panel module — matches panel conventions and prevents `spawn.rs` from becoming a mixed object/terrain editor.
2. **Brush model**: shape + size + selected numeric voxel material + mode — covers required controls while staying within existing `u8` material representation.
3. **Preview model**: mouse-following wireframe footprint — users must see the affected brush volume before committing an edit.
4. **Mutation boundary**: add a logical multi-voxel operation API to `voxel_map_engine` — callers provide world-space changes; the engine owns chunk lookup, dirty marking, padding, and remesh scheduling.
5. **Network authority**: client sends one logical edit operation; server validates, applies, acknowledges/rejects, and broadcasts accepted concrete changes grouped by chunk — keeps server authority and uses existing batching semantics.
6. **Prediction/history model**: client predicts the whole operation and records acknowledged before/after voxel changes for dev-only undo/redo — undo/redo replays inverse logical operations instead of mutating local state only.

## What We're NOT Doing

- Building a full material registry, material names, thumbnails, or terrain palette asset system.
- Adding player-facing terrain editing permissions or economy rules.
- Redesigning chunk generation, chunk persistence, or remesh scheduling beyond supporting multi-voxel operations.
- Adding non-voxel terrain deformation, smoothing, falloff, or continuous sculpting semantics.
- Making undo/redo global across world objects or unrelated dev tools.
- Supporting speculative local-only terrain edits that bypass server authority.

## Open Risks

- Large brush strokes may generate many concrete changes; validation, prediction memory, and network payload size may need limits.
- Undo/redo semantics depend on authoritative acknowledgments; rejected or partially invalid operations need explicit all-or-nothing behavior.
- Brush footprint generation must match preview and application exactly, especially at chunk boundaries and negative coordinates.
- Numeric material IDs are not user-friendly, but adding labels is intentionally out of scope.
- Server validation is currently permissive; future admin/player permission checks may affect the operation message shape.
