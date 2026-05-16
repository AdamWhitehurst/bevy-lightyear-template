# Structure Outline

## Approach

Add terrain sculpting as a dev-tool vertical path: shared brush footprint/types drive preview, click-drag stroke input, client prediction, server validation/application, broadcasts, and undo/redo. Keep voxel chunk bookkeeping inside `voxel_map_engine`; callers submit logical world-space operations, not chunk-local edits.

## Phase 1: Brush UI and Preview Footprint

Deliver a dedicated terrain panel with brush controls and a mouse-following wireframe preview that uses the same footprint builder later used by editing. Establish stroke input state so holding the edit input and dragging can repeatedly apply the brush as the cursor crosses new anchors.

**Files**: `crates/dev/src/panels/terrain.rs`, `crates/dev/src/panels/mod.rs`, `crates/dev/src/lib.rs`, `crates/client/src/map.rs`, `crates/voxel_map_engine/src/brush.rs`, `crates/voxel_map_engine/src/lib.rs`

**Key changes**:

- `pub struct TerrainPanelPlugin;` — new panel plugin registered by `DevPlugin`
- `pub struct TerrainBrushSettings { shape: TerrainBrushShape, size: u32, material: u8, mode: TerrainBrushMode }` — shared dev resource
- `pub enum TerrainBrushShape { Cube, Sphere }` — brush footprint selector
- `pub enum TerrainBrushMode { FillAir, ReplaceAll, PaintExisting, Remove }` — selected edit behavior
- `pub fn brush_anchor(hit: &VoxelRaycastResult, mode: TerrainBrushMode) -> Option<IVec3>` — fill-air uses adjacent voxel; replace-all/paint/remove use hit voxel
- `pub fn brush_footprint(anchor: IVec3, shape: TerrainBrushShape, size: u32) -> Vec<IVec3>` — deterministic preview/application footprint
- `pub struct TerrainBrushPreview { positions: Vec<IVec3> }` — local-only preview state or marker data
- `pub struct TerrainBrushStrokeState { active: bool, last_anchor: Option<IVec3> }` — tracks click-drag stroke continuity and suppresses duplicate applications at the same anchor

**Verify**: `cargo test -p voxel_map_engine brush_footprint`; run client, open Terrain tab, adjust shape/size/material/mode, and confirm preview follows cursor, matches fill-air/remove anchor conventions, and updates while holding edit input and dragging.

---

## Phase 2: Multi-Voxel Fill-Air/Remove Operation

Make one brush application apply end-to-end through prediction, protocol, server authority, voxel engine mutation, dirty chunks, remesh, persistence, and broadcast, then repeat those applications during click-drag strokes as the cursor reaches new brush anchors. This phase supports FillAir and Remove only.

**Files**: `crates/protocol/src/map/voxel.rs`, `crates/voxel_map_engine/src/api.rs`, `crates/voxel_map_engine/src/instance.rs`, `crates/voxel_map_engine/tests/api.rs`, `crates/client/src/map.rs`, `crates/server/src/map.rs`

**Key changes**:

- `pub struct VoxelBrushEditRequest { sequence: u32, anchor: IVec3, shape: TerrainBrushShape, size: u32, mode: TerrainBrushMode, material: u8 }` — client-to-server logical operation
- `pub struct VoxelChange { position: IVec3, voxel: VoxelType }` — concrete accepted change
- `pub struct VoxelEditAck { sequence: u32, changes: Vec<VoxelChange> }` — acknowledge concrete result for prediction/history
- `pub struct VoxelPrediction { sequence: u32, changes: Vec<PredictedVoxelChange> }` — replace single-position prediction
- `pub struct PredictedVoxelChange { position: IVec3, old_voxel: VoxelType, new_voxel: VoxelType }`
- `pub fn set_voxels(&mut self, map: Entity, edits: impl IntoIterator<Item = (IVec3, WorldVoxel)>) -> usize` — public multi-edit API
- `pub fn set_voxels(&mut self, edits: impl IntoIterator<Item = (IVec3, WorldVoxel)>) -> usize` — instance-level implementation preserving dirty/remesh/padding behavior
- `fn concrete_brush_changes(..., voxel_world: &VoxelWorld) -> Vec<VoxelChange>` — computes no-op-filtered FillAir/Remove changes
- `fn handle_terrain_brush_input(...)` — sends one logical request on click and additional requests while dragging over new anchors

**Verify**: `cargo test -p voxel_map_engine api`; `cargo test -p server map::tests::different_chunks_produce_separate_entries`; manually click-drag a brush stroke across a chunk boundary and confirm repeated applications occur, both chunks remesh, and other clients receive updates.

---

## Phase 3: Replacement Mode Semantics

Add replacement modes as a full vertical slice: UI selection, preview, predicted request, server computation, application, ack, and broadcast. `PaintExisting` changes only existing solid voxels and never creates new solids; `ReplaceAll` overwrites every voxel in the footprint with the selected material.

**Files**: `crates/dev/src/panels/terrain.rs`, `crates/client/src/map.rs`, `crates/server/src/map.rs`, `crates/voxel_map_engine/src/brush.rs`, `crates/protocol/src/map/voxel.rs`

**Key changes**:

- `TerrainBrushMode::PaintExisting` and `TerrainBrushMode::ReplaceAll` — enabled in panel and protocol reflection/serialization
- `fn concrete_brush_changes(...) -> Vec<VoxelChange>` — `PaintExisting` filters to `matches!(old, WorldVoxel::Solid(_))`; `ReplaceAll` writes the whole footprint
- `fn predict_brush_changes(..., mode: TerrainBrushMode) -> Vec<PredictedVoxelChange>` — client prediction mirrors server filtering/overwrite semantics
- `fn validate_voxel_brush_edit(request: &VoxelBrushEditRequest, map_entity: Entity, voxel_world: &VoxelWorld) -> bool` — all-or-nothing request validation and size limits

**Verify**: `cargo test -p server paint`; manually test `PaintExisting` over mixed air/solid terrain and confirm air remains air; test `ReplaceAll` and confirm air and solids both become the selected material.

---

## Phase 4: Undo/Redo for Acknowledged Brush Edits

Record acknowledged concrete brush edits and expose undo/redo buttons/shortcuts that submit inverse logical concrete operations through the same authoritative path.

**Files**: `crates/dev/src/panels/terrain.rs`, `crates/client/src/map.rs`, `crates/protocol/src/map/voxel.rs`, `crates/server/src/map.rs`

**Key changes**:

- `pub struct TerrainEditHistory { undo: Vec<TerrainEditRecord>, redo: Vec<TerrainEditRecord> }` — client dev resource
- `pub struct TerrainEditRecord { changes: Vec<AcknowledgedVoxelChange> }`
- `pub struct AcknowledgedVoxelChange { position: IVec3, old_voxel: VoxelType, new_voxel: VoxelType }`
- `pub struct VoxelConcreteEditRequest { sequence: u32, changes: Vec<VoxelChange> }` — used for undo/redo replay when brush params no longer describe the inverse
- `fn handle_terrain_undo_redo_input(...)` — sends inverse/reapply requests, not local-only mutations
- `fn handle_voxel_edit_ack(...)` — moves acknowledged records into history and clears redo on new edits

**Verify**: `cargo test -p client voxel_prediction`; manually make FillAir, ReplaceAll, PaintExisting, and Remove strokes, undo/redo each, and confirm other clients observe the reverted/reapplied terrain.

---

## Phase 5: Limits, Regression Tests, and Runtime Verification

Harden the feature with request limits, cross-chunk tests, prediction reject tests, and documented manual verification steps.

**Files**: `crates/server/src/map.rs`, `crates/client/src/map.rs`, `crates/voxel_map_engine/tests/api.rs`, `README.md` if dev terrain workflow documentation changes

**Key changes**:

- `const MAX_BRUSH_VOXELS: usize = ...` — validation guard for memory/network payload size
- `fn reject_brush_edit(sequence: u32, prediction: &VoxelPrediction)` — rollback all predicted changes for rejected operations
- Tests for negative coordinates, chunk-boundary padding/remesh, multi-chunk broadcasts, click-drag duplicate suppression, fill-air-only, replace-all, paint-existing-no-create, and undo/redo replay

**Verify**: `cargo check-all` and `cargo test-all` pass; manually run client/server, test click-drag cross-chunk FillAir/ReplaceAll/PaintExisting/Remove and undo/redo, and inspect logs for expected dirty/remesh/broadcast behavior.

## Testing Checkpoints

- After Phase 1: Terrain tab owns sculpting controls; preview footprint is deterministic and cursor-following; stroke state tracks held input and anchor changes.
- After Phase 2: FillAir/Remove click-drag brush strokes mutate multiple voxels through server authority and remesh/persist/broadcast correctly, including across chunks.
- After Phase 3: PaintExisting changes only existing solid voxels and cannot fill air; ReplaceAll overwrites every voxel in the footprint.
- After Phase 4: Undo/redo works only for acknowledged edits and routes through the server.
- After Phase 5: Size limits and regression coverage protect cross-chunk edits, rejects, prediction cleanup, and preview/application parity.
