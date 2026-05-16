# Implementation Plan

## Overview

The Dev plugin will expose a dedicated terrain sculpting interface for brush-based voxel editing, preview,
server-authoritative application, and acknowledged undo/redo. One logical brush operation will be represented as
world-space voxel changes while `voxel_map_engine` remains responsible for chunk lookup, dirty marking, padding updates,
and remeshing.

## Global Verification Rule

Before running any `cargo build`, `cargo check`, or `cargo test` command, first verify that no other cargo
build/check/test is already running:

```bash
pgrep -af 'cargo (build|check|test)' || true
```

If this prints an active build/check/test, wait for it to finish or stop it before running the next cargo command.

## Phase 1: Brush UI and Preview Footprint

### Implemented Deviation Notes

- Terrain brush controls are integrated into the existing `▧ World Objects` panel's Terrain tab instead of a separate
  `▦ Terrain` window. This prevents a second panel from forcing `EditingMode::Terrain` every frame and keeps mode
  switching local to the tab selector.
- Brush preview/stroke state is gated by a `Brush active` checkbox; the wireframe does not follow the cursor unless the
  brush is active.
- The rectangular brush shape is named `Rect`, not `Cube`, and exposes separate `width` and `height` controls. Width
  applies across horizontal X/Z; height applies across Y. A width of `2` covers `2x2` horizontally, `3` covers `3x3`,
  etc.
- Width, height, and material use decrement/input/increment controls (`- [value] +`) rather than bare drag inputs.

### Changes

#### 1. Brush footprint API

**File**: `crates/voxel_map_engine/src/brush.rs`  
**Action**: create

Add shared brush types and deterministic footprint helpers. Keep this module free of ECS state so client preview, client
prediction, and server application can use identical logic.

```rust
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::raycast::VoxelRaycastResult;

/// Shape used to expand a terrain brush anchor into voxel positions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Default)]
pub enum TerrainBrushShape {
    #[default]
    Rect,
    Sphere,
}

/// Editing behavior applied to a terrain brush footprint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Default)]
pub enum TerrainBrushMode {
    #[default]
    FillAir,
    ReplaceAll,
    PaintExisting,
    Remove,
}

/// Returns the world-space brush anchor implied by a raycast hit and mode.
pub fn brush_anchor(hit: &VoxelRaycastResult, mode: TerrainBrushMode) -> Option<IVec3> {
    match mode {
        TerrainBrushMode::FillAir => hit.normal.map(|normal| hit.position + normal.as_ivec3()),
        TerrainBrushMode::ReplaceAll | TerrainBrushMode::PaintExisting | TerrainBrushMode::Remove => {
            Some(hit.position)
        }
    }
}

/// Returns deterministic world-space positions covered by a brush.
pub fn brush_footprint(anchor: IVec3, shape: TerrainBrushShape, width: u32, height: u32) -> Vec<IVec3> {
    match shape {
        TerrainBrushShape::Rect => rect_footprint(anchor, width, height),
        TerrainBrushShape::Sphere => sphere_footprint(anchor, width),
    }
}

fn rect_footprint(anchor: IVec3, width: u32, height: u32) -> Vec<IVec3> { /* width on X/Z, height on Y */ }
fn sphere_footprint(anchor: IVec3, width: u32) -> Vec<IVec3> { /* width is diameter */ }
```

Add tests in this file:

- `rect_width_one_height_one_returns_anchor_only`
- `rect_width_two_height_one_returns_two_by_two_floor`
- `rect_width_two_height_three_returns_twelve_voxels`
- `sphere_excludes_cube_corners`
- `fill_air_anchor_uses_hit_normal`
- `remove_paint_replace_anchor_use_hit_position`

#### 2. Register brush module and reflect types

**File**: `crates/voxel_map_engine/src/lib.rs`  
**Action**: modify

Add module/export and type registration.

```rust
pub mod brush;
```

Inside `VoxelPlugin::build`:

```rust
app.register_type::<brush::TerrainBrushShape>();
app.register_type::<brush::TerrainBrushMode>();
```

Inside `pub mod prelude`:

```rust
pub use crate::brush::*;
```

#### 3. Terrain panel state and UI

**File**: `crates/dev/src/panels/terrain.rs`  
**Action**: create

Create terrain brush settings and controls integrated into the existing World Objects panel Terrain tab. The terrain
plugin only initializes the settings resource; it must not open a second window or force `EditingMode::Terrain` every
frame.

```rust
use crate::state::{DevInspectorState, EditingMode};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use voxel_map_engine::prelude::{TerrainBrushMode, TerrainBrushShape};

/// User-selected terrain brush settings shared by dev UI and client terrain input.
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource)]
pub struct TerrainBrushSettings {
    pub active: bool,
    pub shape: TerrainBrushShape,
    pub width: u32,
    pub height: u32,
    pub material: u8,
    pub mode: TerrainBrushMode,
}

impl Default for TerrainBrushSettings {
    fn default() -> Self {
        Self {
            active: false,
            shape: TerrainBrushShape::Rect,
            width: 1,
            height: 1,
            material: 0,
            mode: TerrainBrushMode::FillAir,
        }
    }
}

/// Initializes terrain sculpting UI resources.
pub struct TerrainPanelPlugin;

impl Plugin for TerrainPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TerrainBrushSettings>();
    }
}

/// Draws terrain brush controls inside the world-object panel's terrain tab.
pub fn draw_terrain_controls(ui: &mut egui::Ui, settings: &mut TerrainBrushSettings) {
    ui.checkbox(&mut settings.active, "Brush active");
    ui.add_enabled_ui(settings.active, |ui| {
        // Shape/mode controls omitted here for brevity.
        // Width, height, and material are rendered as: - [value] +.
    });
}
```

#### 4. Panel module registration

**File**: `crates/dev/src/panels/mod.rs`  
**Action**: modify

Add the terrain panel under the existing `spawn-panel` feature so no Cargo feature changes are needed.

```rust
#[cfg(feature = "spawn-panel")]
pub mod terrain;
```

#### 5. Dev plugin registration and export

**File**: `crates/dev/src/lib.rs`  
**Action**: modify

Register the terrain panel next to the spawn panel and re-export the settings for `client`.

```rust
#[cfg(all(feature = "inspector", feature = "spawn-panel"))]
pub use panels::terrain::{TerrainBrushSettings, TerrainPanelPlugin};
```

Inside `DevPlugin::build`:

```rust
#[cfg(feature = "spawn-panel")]
app.add_plugins(panels::terrain::TerrainPanelPlugin);
```

#### 6. Integrate terrain controls into terrain tab

**File**: `crates/dev/src/panels/spawn.rs`  
**Action**: modify

Keep the primary tab selector and render `draw_terrain_controls` in the Terrain tab, using `TerrainBrushSettings` as
panel state.

```rust
fn draw_terrain_tab(ui: &mut egui::Ui, settings: &mut TerrainBrushSettings) {
    draw_section(ui, "TERRAIN", |ui| {
        draw_terrain_controls(ui, settings);
    });
}
```

#### 7. Client preview and stroke state

**File**: `crates/client/src/map.rs`  
**Action**: modify

Import `TerrainBrushSettings` behind `spawn-panel` and add local preview/stroke state.

```rust
#[cfg(feature = "spawn-panel")]
use dev::TerrainBrushSettings;
use voxel_map_engine::prelude::{brush_anchor, brush_footprint, TerrainBrushMode};

/// Local-only terrain brush preview footprint.
#[derive(Resource, Default)]
pub struct TerrainBrushPreview {
    pub positions: Vec<IVec3>,
}

/// Tracks held brush strokes and suppresses duplicate applications at one anchor.
#[derive(Resource, Default)]
pub struct TerrainBrushStrokeState {
    pub active: bool,
    pub last_anchor: Option<IVec3>,
}
```

Initialize resources in `ClientMapPlugin`:

```rust
.init_resource::<TerrainBrushPreview>()
.init_resource::<TerrainBrushStrokeState>()
```

Add systems to the existing `PostUpdate` terrain chain before edit application in later phases:

```rust
update_terrain_brush_preview.run_if(in_editing_mode(EditingMode::Terrain)),
update_terrain_brush_stroke_state.run_if(in_editing_mode(EditingMode::Terrain)),
```

Add helpers:

```rust
fn current_terrain_brush_anchor(
    chunk_ticket: &ChunkTicket,
    voxel_world: &VoxelWorld,
    camera_query: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: &Query<&Window, With<PrimaryWindow>>,
    mode: TerrainBrushMode,
) -> Option<IVec3> {
    let Some(ray) = camera_ray(camera_query, window_query) else {
        trace!("current_terrain_brush_anchor: no camera ray");
        return None;
    };
    let Some(hit) = voxel_world.raycast(chunk_ticket.map_entity, ray, RAYCAST_MAX_DISTANCE, |v| {
        matches!(v, WorldVoxel::Solid(_))
    }) else {
        trace!("current_terrain_brush_anchor: raycast hit nothing");
        return None;
    };
    let Some(anchor) = brush_anchor(&hit, mode) else {
        trace!("current_terrain_brush_anchor: hit has no usable brush anchor");
        return None;
    };
    Some(anchor)
}
```

Preview can initially render with gizmos as wire cubes centered at each voxel position:

```rust
fn update_terrain_brush_preview(
    player_query: Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    voxel_world: VoxelWorld,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    settings: Res<TerrainBrushSettings>,
    mut preview: ResMut<TerrainBrushPreview>,
    mut gizmos: Gizmos,
) {
    let Ok(chunk_ticket) = player_query.single() else {
        trace!("update_terrain_brush_preview: no predicted player with ChunkTicket");
        preview.positions.clear();
        return;
    };
    let Some(anchor) = current_terrain_brush_anchor(
        chunk_ticket,
        &voxel_world,
        &camera_query,
        &window_query,
        settings.mode,
    ) else {
        preview.positions.clear();
        return;
    };
    preview.positions = brush_footprint(anchor, settings.shape, settings.width, settings.height);
    for pos in &preview.positions {
        gizmos.cuboid(
            Transform::from_translation(pos.as_vec3() + Vec3::splat(0.5))
                .with_scale(Vec3::splat(1.0)),
            Color::srgb(0.2, 0.9, 1.0),
        );
    }
}
```

`update_terrain_brush_stroke_state` should only update `active`/`last_anchor`; actual sends are added in Phase 2.

### Verification

#### Automated

- [x] `pgrep -af 'cargo (build|check|test)' || true` shows no active cargo build/check/test before running tests.
- [x] `cargo test -p voxel_map_engine brush` passes.
- [x] `cargo check -p dev --features inspector,spawn-panel` passes.
- [x] `cargo check -p client` passes.

#### Manual

- [x] Run `cargo server` and `cargo client` in separate terminals.
- [x] Press `F4`, open dev panels, and confirm terrain controls appear inside the World Objects panel's Terrain tab.
- [x] Adjust shape, width, height, material, and mode; expected: controls update without affecting object placement
      tabs.
- [x] Use the `- [value] +` controls for width, height, and material; expected: decrement/increment buttons clamp at
      their min/max.
- [x] Toggle Brush active off and move over terrain; expected: no wireframe footprint follows the cursor.
- [x] Toggle Brush active on and move over terrain; expected: wireframe footprint follows the cursor.
- [x] Switch between Fill Air and Remove; expected: Fill Air preview anchors adjacent to the hit face,
      Remove/Paint/Replace anchor on the hit voxel.
- [x] Hold edit input and drag; expected: stroke state updates as the anchor changes and does not repeatedly mark the
      same anchor.

---

## Phase 2: Multi-Voxel Fill-Air/Remove Operation

### Implemented Deviation Notes

- Held terrain brush input now supports `Discrete` and `Continuous` stroke modes. `Discrete` ignores brush
  preview/anchor movement caused by newly placed voxels and only sends another request after screen-space cursor
  movement. `Continuous` repeats while held at a configurable `Every N frames` interval, defaulting to `20` frames.
- Terrain brush application uses left click for all currently implemented modes; the selected brush mode determines
  whether left click fills air or removes solids. Brush input is suppressed while egui is using pointer input so
  clicking dev-panel controls does not also edit voxels behind the panel.
- Held brush strokes lock to the initial hit face. Later cursor rays project onto that face plane instead of raycasting
  newly edited voxels, so top-face strokes move across the top plane and side-face strokes move across that side plane.
- In `Continuous` mode, if the locked-plane anchor is already edited, the client searches along the initial hit-face
  normal for the next editable layer. Holding still can therefore keep filling or removing along the clicked face normal
  without falling back to camera-biased raycasts.

### Changes

#### 1. Protocol brush request and concrete change shapes

**File**: `crates/protocol/src/map/voxel.rs`  
**Action**: modify

Add logical brush request and concrete change structs. Keep existing `VoxelEditRequest` for backward compatibility
during the transition.

```rust
use voxel_map_engine::prelude::{TerrainBrushMode, TerrainBrushShape, VoxelType};

/// One concrete voxel change accepted by the server.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
#[type_path = "protocol::map"]
pub struct VoxelChange {
    pub position: IVec3,
    pub voxel: VoxelType,
}

/// Client requests one logical terrain brush edit.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct VoxelBrushEditRequest {
    pub sequence: u32,
    pub anchor: IVec3,
    pub shape: TerrainBrushShape,
    pub width: u32,
    pub height: u32,
    pub mode: TerrainBrushMode,
    pub material: u8,
}

/// Server acknowledges a terrain edit and returns concrete accepted changes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct VoxelEditAck {
    pub sequence: u32,
    pub changes: Vec<VoxelChange>,
}
```

Update `VoxelEditReject` to support all-or-nothing brush rollback:

```rust
pub struct VoxelEditReject {
    pub sequence: u32,
    pub position: IVec3,
    pub correct_voxel: VoxelType,
}
```

Do not change reject shape yet; Phase 5 will add whole-prediction rollback helpers without requiring protocol changes.

#### 2. Register protocol messages and exports

**File**: `crates/protocol/src/lib.rs`  
**Action**: modify

Register the new client-to-server message on `VoxelChannel`:

```rust
app.register_message::<VoxelBrushEditRequest>()
    .add_direction(NetworkDirection::ClientToServer);
```

#### 3. Export new protocol types

**File**: `crates/protocol/src/map/mod.rs`  
**Action**: modify

```rust
pub use voxel::{
    SectionBlocksUpdate, VoxelBrushEditRequest, VoxelChange, VoxelChannel, VoxelEditAck,
    VoxelEditBroadcast, VoxelEditReject, VoxelEditRequest,
};
```

#### 4. Public multi-voxel engine API

**File**: `crates/voxel_map_engine/src/api.rs`  
**Action**: modify

Add a public API that preserves the existing map-entity boundary and filters `Unset` loudly.

```rust
/// Mutate multiple world-space voxels on one map. Returns the number of loaded voxels written.
pub fn set_voxels(
    &mut self,
    map: Entity,
    edits: impl IntoIterator<Item = (IVec3, WorldVoxel)>,
) -> usize {
    let Ok((mut instance, _)) = self.maps.get_mut(map) else {
        warn!("set_voxels: entity {map:?} has no VoxelMapInstance");
        return 0;
    };
    instance.set_voxels(edits)
}
```

#### 5. Instance-level multi-voxel mutation

**File**: `crates/voxel_map_engine/src/instance.rs`  
**Action**: modify

Reuse `set_voxel` so dirty chunks, remesh queues, and neighbor padding remain identical to single edits.

```rust
/// Mutate multiple world-space voxels. Returns the number of loaded voxels requested for write.
pub fn set_voxels(&mut self, edits: impl IntoIterator<Item = (IVec3, WorldVoxel)>) -> usize {
    let mut written = 0;
    for (world_pos, voxel) in edits {
        debug_assert!(
            voxel != WorldVoxel::Unset,
            "set_voxels: cannot write Unset (internal sentinel)"
        );
        self.set_voxel(world_pos, voxel);
        written += 1;
    }
    written
}
```

#### 6. Engine API tests

**File**: `crates/voxel_map_engine/tests/api.rs`  
**Action**: modify

Add tests that call `VoxelWorld::set_voxels` through `run_system_once`:

```rust
#[test]
fn set_voxels_marks_each_touched_chunk_dirty_and_for_remesh() { /* edit positions in chunk 0 and chunk 1 */ }

#[test]
fn set_voxels_updates_boundary_padding_for_neighbor_chunks() { /* edit local x=15 with loaded neighbor */ }

#[test]
fn set_voxels_handles_negative_world_coordinates() { /* edit -1 and assert chunk -1 dirty */ }
```

#### 7. Client prediction shape and FillAir/Remove request sending

**File**: `crates/client/src/map.rs`  
**Action**: modify

Replace single-position predictions with multi-change predictions.

```rust
/// Tracks pending terrain edit predictions awaiting server acknowledgment.
#[derive(Resource, Default)]
pub struct VoxelPredictionState {
    pub next_sequence: u32,
    pub pending: Vec<VoxelPrediction>,
}

/// A pending terrain edit prediction awaiting server acknowledgment.
pub struct VoxelPrediction {
    pub sequence: u32,
    pub changes: Vec<PredictedVoxelChange>,
}

/// One locally predicted voxel change.
pub struct PredictedVoxelChange {
    pub position: IVec3,
    pub old_voxel: VoxelType,
    pub new_voxel: VoxelType,
}
```

Update pending-position checks in `handle_voxel_broadcasts` and `handle_section_blocks_update`:

```rust
let has_pending_prediction = prediction_state
    .pending
    .iter()
    .flat_map(|p| p.changes.iter())
    .any(|change| change.position == broadcast.position);
```

Add prediction helpers for Phase 2 modes only:

```rust
fn predict_brush_changes(
    map_entity: Entity,
    voxel_world: &VoxelWorld,
    anchor: IVec3,
    settings: &TerrainBrushSettings,
) -> Vec<PredictedVoxelChange> {
    brush_footprint(anchor, settings.shape, settings.width, settings.height)
        .into_iter()
        .filter_map(|position| {
            let old = voxel_world.get_voxel(map_entity, position);
            let new_voxel = match settings.mode {
                TerrainBrushMode::FillAir if matches!(old, WorldVoxel::Air | WorldVoxel::Unset) => {
                    VoxelType::Solid(settings.material)
                }
                TerrainBrushMode::Remove if matches!(old, WorldVoxel::Solid(_)) => VoxelType::Air,
                _ => return None,
            };
            Some(PredictedVoxelChange {
                position,
                old_voxel: old.into(),
                new_voxel,
            })
        })
        .collect()
}
```

Replace `handle_voxel_input` with `handle_terrain_brush_input` for terrain mode. It should:

1. Read `PlayerActions::PlaceVoxel` for both FillAir and Remove; the selected brush mode determines the operation.
2. Allow click-drag by using pressed state plus `TerrainBrushStrokeState`.
3. Skip if mode is PaintExisting/ReplaceAll in this phase with `trace!`.
4. Compute the anchor with `current_terrain_brush_anchor`.
5. Suppress duplicate sends when `stroke_state.active && stroke_state.last_anchor == Some(anchor)`.
6. Predict and locally apply all concrete changes with `voxel_world.set_voxels`.
7. Send one `VoxelBrushEditRequest`.

Core send path:

```rust
let sequence = prediction_state.next();
let changes = predict_brush_changes(chunk_ticket.map_entity, &voxel_world, anchor, &settings);
if changes.is_empty() {
    trace!("handle_terrain_brush_input: brush produced no changes");
    return;
}
voxel_world.set_voxels(
    chunk_ticket.map_entity,
    changes.iter().map(|change| (change.position, WorldVoxel::from(change.new_voxel))),
);
prediction_state.pending.push(VoxelPrediction { sequence, changes });
for mut sender in message_sender.iter_mut() {
    sender.send::<VoxelChannel>(VoxelBrushEditRequest {
        sequence,
        anchor,
        shape: settings.shape,
        width: settings.width,
        height: settings.height,
        mode: settings.mode,
        material: settings.material,
    });
}
stroke_state.active = true;
stroke_state.last_anchor = Some(anchor);
```

Update `handle_voxel_edit_ack` for the new ack shape:

```rust
prediction_state.pending.retain(|p| p.sequence != ack.sequence);
```

Keep old single-edit support only if compilation requires it; do not route terrain UI through it.

#### 8. Server brush request handling

**File**: `crates/server/src/map.rs`  
**Action**: modify

Import new protocol types and add a brush request handler next to `handle_voxel_edit_requests`.

```rust
pub fn handle_voxel_brush_edit_requests(
    mut receivers: Query<(Entity, &mut MessageReceiver<VoxelBrushEditRequest>)>,
    mut ack_senders: Query<&mut MessageSender<VoxelEditAck>>,
    mut reject_senders: Query<&mut MessageSender<VoxelEditReject>>,
    mut pending_broadcasts: ResMut<PendingVoxelBroadcasts>,
    mut dirty_state: ResMut<WorldDirtyState>,
    time: Res<Time>,
    mut voxel_world: VoxelWorld,
    controlled_query: Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: Res<MapRegistry>,
) { /* resolve map, validate, compute concrete changes, apply, ack, queue */ }
```

Concrete changes helper for FillAir/Remove:

```rust
fn concrete_brush_changes(
    request: &VoxelBrushEditRequest,
    map_entity: Entity,
    voxel_world: &VoxelWorld,
) -> Vec<VoxelChange> {
    brush_footprint(request.anchor, request.shape, request.width, request.height)
        .into_iter()
        .filter_map(|position| {
            let old = voxel_world.get_voxel(map_entity, position);
            let voxel = match request.mode {
                TerrainBrushMode::FillAir if matches!(old, WorldVoxel::Air | WorldVoxel::Unset) => {
                    VoxelType::Solid(request.material)
                }
                TerrainBrushMode::Remove if matches!(old, WorldVoxel::Solid(_)) => VoxelType::Air,
                _ => return None,
            };
            Some(VoxelChange { position, voxel })
        })
        .collect()
}
```

Apply/ack/queue helper:

```rust
fn apply_voxel_changes(
    changes: &[VoxelChange],
    map_entity: Entity,
    voxel_world: &mut VoxelWorld,
    dirty_state: &mut WorldDirtyState,
    time: &Time,
) {
    voxel_world.set_voxels(
        map_entity,
        changes.iter().map(|change| (change.position, WorldVoxel::from(change.voxel))),
    );
    let now = time.elapsed_secs_f64();
    if !dirty_state.is_dirty {
        dirty_state.first_dirty_time = Some(now);
    }
    dirty_state.is_dirty = true;
    dirty_state.last_edit_time = now;
}

fn send_edit_ack(
    client_entity: Entity,
    sequence: u32,
    changes: Vec<VoxelChange>,
    ack_senders: &mut Query<&mut MessageSender<VoxelEditAck>>,
) {
    if let Ok(mut sender) = ack_senders.get_mut(client_entity) {
        sender.send::<VoxelChannel>(VoxelEditAck { sequence, changes });
    } else {
        warn!("send_edit_ack: no ack sender for {client_entity:?}");
    }
}
```

For each accepted `VoxelChange`, call existing `queue_edit_broadcast(PendingVoxelEdit { ... })`; existing
`flush_voxel_broadcasts` will batch by chunk.

Register `handle_voxel_brush_edit_requests` in the server map plugin wherever `handle_voxel_edit_requests` is
registered.

### Verification

#### Automated

- [x] `pgrep -af 'cargo (build|check|test)' || true` shows no active cargo build/check/test before running tests.
- [x] `cargo test -p voxel_map_engine api` passes.
- [x] `cargo test -p client prediction` passes after updating prediction tests for multi-change predictions.
- [x] `cargo test -p server different_chunks_produce_separate_entries` passes.
- [x] `cargo check -p protocol` passes.
- [x] `cargo check -p client` passes.
- [x] `cargo check -p server` passes.

#### Manual

- [x] Run `cargo server` and two `cargo client` instances.
- [x] Select Fill Air, width/height > 1, click once; expected: multiple voxels are added through server authority.
- [x] Select Remove, width/height > 1, left-click once; expected: multiple solid voxels are removed through server
      authority.
- [x] In Discrete stroke mode, press and hold without moving the cursor; expected: only one brush request is applied
      even if the preview anchor moves because newly placed voxels are closer to the camera.
- [x] In Discrete stroke mode, click-drag across the screen; expected: repeated brush applications occur only after
      screen-space cursor movement.
- [x] In Continuous stroke mode, press and hold; expected: brush applications repeat according to `Every N frames`
      (default `20`), and increasing the value slows the repeat rate. Fill Air grows outward along the initial hit-face
      normal and Remove digs inward along the opposite normal while the cursor remains still.
- [x] Start strokes from a top face and from side faces; expected: held-stroke movement follows the initial hit face
      instead of flipping between vertical and horizontal planes mid-stroke.
- [x] Hold or drag Fill Air from an oblique camera angle; expected: the stroke follows the initial hit-face plane
      instead of stepping diagonally toward the camera as new voxels are placed.
- [x] Click and drag controls in the dev panel while terrain editing and Brush active are enabled; expected: UI controls
      respond, but no terrain brush edits are sent behind the panel.
- [x] Drag across a chunk boundary; expected: both chunks visually remesh and later persist after server save debounce.
- [x] Observe second client; expected: it receives the same terrain updates, with same-chunk multi-edits arriving
      through batched section updates.

---

## Phase 3: Replacement Mode Semantics

### Changes

#### 1. Enable all modes in the terrain panel

**File**: `crates/dev/src/panels/terrain.rs`  
**Action**: modify

Ensure `PaintExisting` and `ReplaceAll` controls are enabled, not placeholder-disabled. No new UI state is needed.

#### 2. Protocol supports all modes

**File**: `crates/protocol/src/map/voxel.rs`  
**Action**: modify

No new structs are needed if Phase 2 derives serde/reflect for `TerrainBrushMode`. Add tests if protocol tests exist;
otherwise rely on `cargo check -p protocol`.

#### 3. Shared footprint semantics stay unchanged

**File**: `crates/voxel_map_engine/src/brush.rs`  
**Action**: modify

Add tests proving footprint output is independent of mode; only anchor selection differs.

```rust
#[test]
fn footprint_is_mode_independent_after_anchor_selection() { /* compare same anchor */ }
```

#### 4. Client prediction for replacement modes

**File**: `crates/client/src/map.rs`  
**Action**: modify

Extend `predict_brush_changes`:

```rust
let new_voxel = match settings.mode {
    TerrainBrushMode::FillAir if matches!(old, WorldVoxel::Air | WorldVoxel::Unset) => {
        VoxelType::Solid(settings.material)
    }
    TerrainBrushMode::Remove if matches!(old, WorldVoxel::Solid(_)) => VoxelType::Air,
    TerrainBrushMode::PaintExisting if matches!(old, WorldVoxel::Solid(_)) => {
        VoxelType::Solid(settings.material)
    }
    TerrainBrushMode::ReplaceAll => VoxelType::Solid(settings.material),
    _ => return None,
};
```

Update input mapping so PaintExisting, ReplaceAll, and Remove all use the place action (`PlayerActions::PlaceVoxel`); the selected mode determines the operation.

#### 5. Server concrete changes for replacement modes

**File**: `crates/server/src/map.rs`  
**Action**: modify

Extend `concrete_brush_changes` with the same filtering rules as client prediction:

```rust
let voxel = match request.mode {
    TerrainBrushMode::FillAir if matches!(old, WorldVoxel::Air | WorldVoxel::Unset) => {
        VoxelType::Solid(request.material)
    }
    TerrainBrushMode::Remove if matches!(old, WorldVoxel::Solid(_)) => VoxelType::Air,
    TerrainBrushMode::PaintExisting if matches!(old, WorldVoxel::Solid(_)) => {
        VoxelType::Solid(request.material)
    }
    TerrainBrushMode::ReplaceAll => VoxelType::Solid(request.material),
    _ => return None,
};
```

#### 6. All-or-nothing validation skeleton

**File**: `crates/server/src/map.rs`  
**Action**: modify

Add validation hook now, with permissive rules except the brush footprint-size limit added in Phase 5.

```rust
fn validate_voxel_brush_edit(
    request: &VoxelBrushEditRequest,
    _map_entity: Entity,
    _voxel_world: &VoxelWorld,
) -> bool {
    request.width > 0 && request.height > 0
}
```

Use this before computing/applying changes. If false, reject the whole request.

### Verification

#### Automated

- [x] `pgrep -af 'cargo (build|check|test)' || true` shows no active cargo build/check/test before running tests.
- [x] `cargo test -p voxel_map_engine brush` passes.
- [x] `cargo test -p client paint` passes after adding client prediction tests for PaintExisting and ReplaceAll.
- [x] `cargo test -p server paint` passes after adding server concrete-change tests.
- [x] `cargo check -p protocol` passes.
- [x] `cargo check -p client` passes.
- [x] `cargo check -p server` passes.

#### Manual

- [ ] Run `cargo server` and `cargo client`.
- [ ] Select Paint Existing over mixed air/solid terrain; expected: solid voxels change material and air remains air.
- [ ] Select Replace All over mixed air/solid terrain; expected: every voxel in the footprint becomes the selected
      material.
- [ ] Confirm preview footprint is identical to the applied footprint for PaintExisting and ReplaceAll.
- [ ] Confirm other clients observe PaintExisting and ReplaceAll edits.

---

## Phase 4: Undo/Redo for Acknowledged Brush Edits

### Changes

#### 1. Concrete edit request protocol

**File**: `crates/protocol/src/map/voxel.rs`  
**Action**: modify

Add a concrete request for undo/redo replay, where brush parameters are no longer sufficient to describe the inverse.

```rust
/// Client requests a concrete authoritative voxel edit, used by terrain undo/redo.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct VoxelConcreteEditRequest {
    pub sequence: u32,
    pub changes: Vec<VoxelChange>,
}
```

#### 2. Register and export concrete request

**File**: `crates/protocol/src/lib.rs`  
**Action**: modify

```rust
app.register_message::<VoxelConcreteEditRequest>()
    .add_direction(NetworkDirection::ClientToServer);
```

**File**: `crates/protocol/src/map/mod.rs`  
**Action**: modify

Export `VoxelConcreteEditRequest`.

#### 3. Terrain history resource and UI controls

**File**: `crates/dev/src/panels/terrain.rs`  
**Action**: modify

Add client-dev terrain edit history resource and button request flags. This resource is globally unique dev UI state, so
ECS `Resource` is appropriate.

```rust
/// Client-side history of acknowledged terrain edits.
#[derive(Resource, Default, Clone, Debug)]
pub struct TerrainEditHistory {
    pub undo: Vec<TerrainEditRecord>,
    pub redo: Vec<TerrainEditRecord>,
    pub undo_requested: bool,
    pub redo_requested: bool,
}

/// One acknowledged terrain operation.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainEditRecord {
    pub changes: Vec<AcknowledgedVoxelChange>,
}

/// One acknowledged voxel change with old and new values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AcknowledgedVoxelChange {
    pub position: IVec3,
    pub old_voxel: VoxelType,
    pub new_voxel: VoxelType,
}
```

Initialize in `TerrainPanelPlugin` and draw Undo/Redo controls:

```rust
app.init_resource::<TerrainEditHistory>();
```

```rust
ui.horizontal(|ui| {
    if ui.button("Undo").clicked() {
        history.undo_requested = true;
    }
    if ui.button("Redo").clicked() {
        history.redo_requested = true;
    }
});
```

#### 4. Client imports and history handling

**File**: `crates/client/src/map.rs`  
**Action**: modify

Import `TerrainEditHistory`, `TerrainEditRecord`, and `AcknowledgedVoxelChange` from `dev`.

Extend pending prediction records with source operation kind if needed:

```rust
pub enum TerrainPredictionKind {
    NewEdit,
    Undo,
    Redo,
}

pub struct VoxelPrediction {
    pub sequence: u32,
    pub changes: Vec<PredictedVoxelChange>,
    pub kind: TerrainPredictionKind,
}
```

Update `handle_voxel_edit_ack` to:

1. Find the matching pending prediction by sequence.
2. Convert pending `PredictedVoxelChange` values into `TerrainEditRecord`.
3. For `NewEdit`, push to `history.undo` and clear `history.redo`.
4. For `Undo`, push the undone record to `history.redo` only after ack.
5. For `Redo`, push the redone record back to `history.undo` only after ack.
6. Remove the pending prediction.

Core record conversion:

```rust
fn acknowledged_record(prediction: &VoxelPrediction) -> TerrainEditRecord {
    TerrainEditRecord {
        changes: prediction
            .changes
            .iter()
            .map(|change| AcknowledgedVoxelChange {
                position: change.position,
                old_voxel: change.old_voxel,
                new_voxel: change.new_voxel,
            })
            .collect(),
    }
}
```

Add undo/redo input system:

```rust
fn handle_terrain_undo_redo_input(
    mut history: ResMut<TerrainEditHistory>,
    mut prediction_state: ResMut<VoxelPredictionState>,
    mut voxel_world: VoxelWorld,
    player_query: Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    mut message_sender: Query<&mut MessageSender<VoxelConcreteEditRequest>>,
) { /* pop requested record, predict locally, send inverse/reapply */ }
```

Inverse/reapply helpers:

```rust
fn inverse_changes(record: &TerrainEditRecord) -> Vec<VoxelChange> {
    record
        .changes
        .iter()
        .map(|change| VoxelChange { position: change.position, voxel: change.old_voxel })
        .collect()
}

fn reapply_changes(record: &TerrainEditRecord) -> Vec<VoxelChange> {
    record
        .changes
        .iter()
        .map(|change| VoxelChange { position: change.position, voxel: change.new_voxel })
        .collect()
}
```

Register `handle_terrain_undo_redo_input.run_if(in_editing_mode(EditingMode::Terrain))` in the terrain `PostUpdate`
chain.

#### 5. Server concrete request handling

**File**: `crates/server/src/map.rs`  
**Action**: modify

Add handler for `VoxelConcreteEditRequest`. It should resolve the active player map, validate all changes, apply them
through `apply_voxel_changes`, ack with the exact accepted changes, and queue broadcasts for every change.

```rust
pub fn handle_voxel_concrete_edit_requests(
    mut receivers: Query<(Entity, &mut MessageReceiver<VoxelConcreteEditRequest>)>,
    mut ack_senders: Query<&mut MessageSender<VoxelEditAck>>,
    mut reject_senders: Query<&mut MessageSender<VoxelEditReject>>,
    mut pending_broadcasts: ResMut<PendingVoxelBroadcasts>,
    mut dirty_state: ResMut<WorldDirtyState>,
    time: Res<Time>,
    mut voxel_world: VoxelWorld,
    controlled_query: Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: Res<MapRegistry>,
) { /* same apply/ack/queue path as brush requests */ }
```

Register it in the server map plugin next to the brush request handler.

### Verification

#### Automated

- [ ] `pgrep -af 'cargo (build|check|test)' || true` shows no active cargo build/check/test before running tests.
- [ ] `cargo test -p client prediction` passes after adding history/ack tests.
- [ ] `cargo test -p client undo` passes after adding inverse/reapply tests.
- [ ] `cargo test -p server concrete` passes after adding concrete request tests.
- [ ] `cargo check -p protocol` passes.
- [ ] `cargo check -p dev --features inspector,spawn-panel` passes.
- [ ] `cargo check -p client` passes.
- [ ] `cargo check -p server` passes.

#### Manual

- [ ] Run `cargo server` and two `cargo client` instances.
- [ ] Make FillAir, Remove, PaintExisting, and ReplaceAll strokes; expected: each acknowledged edit enables Undo.
- [ ] Undo each edit; expected: terrain reverts only after server ack and other clients observe the revert.
- [ ] Redo each edit; expected: terrain reapplies only after server ack and other clients observe the reapply.
- [ ] Make a new edit after undo; expected: redo stack clears.

---

## Phase 5: Limits, Regression Tests, and Runtime Verification

### Changes

#### 1. Server brush footprint and payload limits

**File**: `crates/server/src/map.rs`  
**Action**: modify

Add a concrete limit and enforce it before generating/applying changes.

```rust
const MAX_BRUSH_VOXELS: usize = 4096;

fn validate_voxel_brush_edit(
    request: &VoxelBrushEditRequest,
    _map_entity: Entity,
    _voxel_world: &VoxelWorld,
) -> bool {
    if request.width == 0 || request.height == 0 {
        return false;
    }
    brush_footprint(request.anchor, request.shape, request.width, request.height).len() <= MAX_BRUSH_VOXELS
}

fn validate_concrete_voxel_edit(request: &VoxelConcreteEditRequest) -> bool {
    !request.changes.is_empty() && request.changes.len() <= MAX_BRUSH_VOXELS
}
```

Use `trace!`/`warn!` on rejects and send one reject for the sequence.

#### 2. Whole-prediction reject rollback

**File**: `crates/client/src/map.rs`  
**Action**: modify

Add a rollback helper and use it in `handle_voxel_edit_reject`.

```rust
fn reject_brush_edit(
    sequence: u32,
    prediction_state: &mut VoxelPredictionState,
    voxel_world: &mut VoxelWorld,
    map_entity: Entity,
) {
    let Some(index) = prediction_state.pending.iter().position(|p| p.sequence == sequence) else {
        trace!("reject_brush_edit: no pending prediction for seq={sequence}");
        return;
    };
    let prediction = prediction_state.pending.remove(index);
    voxel_world.set_voxels(
        map_entity,
        prediction
            .changes
            .iter()
            .map(|change| (change.position, WorldVoxel::from(change.old_voxel))),
    );
}
```

Keep applying `reject.correct_voxel` for old single-edit compatibility only if needed, but brush/concrete predictions
should roll back all predicted changes by sequence.

#### 3. Client duplicate-suppression tests

**File**: `crates/client/src/map.rs`  
**Action**: modify

Add pure tests for stroke duplicate suppression and reject rollback:

```rust
#[test]
fn stroke_state_suppresses_duplicate_anchor_while_active() { /* same anchor => no second send */ }

#[test]
fn reject_rolls_back_all_changes_in_prediction() { /* multi-change rollback */ }
```

#### 4. Engine regression tests

**File**: `crates/voxel_map_engine/tests/api.rs`  
**Action**: modify

Add/confirm tests for:

- negative coordinate multi-edit chunk mapping
- boundary padding/remesh after a multi-edit touches local `0` and `chunk_size - 1`
- dirty chunks include every touched loaded chunk
- unloaded chunks are traced/skipped without panicking

#### 5. Server regression tests

**File**: `crates/server/src/map.rs`  
**Action**: modify

Add tests under the existing test module for:

- brush limit rejects oversized requests
- FillAir only writes air/unset positions
- Remove only writes solid positions
- ReplaceAll writes every footprint position
- PaintExisting never creates solids in air
- concrete request limit rejects oversized undo/redo payloads
- multi-chunk accepted changes queue separate broadcast buckets

#### 6. README update if workflow changed

**File**: `README.md`  
**Action**: modify if needed

Update the Dev Inspector section only if the implemented UI changes documented user workflow. Add a concise sentence
such as:

```markdown
The Terrain tab provides dev/admin brush sculpting controls for Fill Air, Remove, Paint Existing, Replace All, preview,
and acknowledged undo/redo while in Terrain editing mode.
```

### Verification

#### Automated

- [ ] `pgrep -af 'cargo (build|check|test)' || true` shows no active cargo build/check/test before running tests.
- [ ] `cargo check-all` passes.
- [ ] `cargo test-all` passes.
- [ ] `cargo test -p voxel_map_engine brush` passes.
- [ ] `cargo test -p voxel_map_engine api` passes.
- [ ] `cargo test -p client prediction` passes.
- [ ] `cargo test -p client undo` passes.
- [ ] `cargo test -p server brush` passes.
- [ ] `cargo test -p server concrete` passes.

#### Manual

- [ ] Run `cargo server` and two `cargo client` instances.
- [ ] FillAir click-drag across a chunk boundary; expected: all affected chunks remesh, persist, and broadcast.
- [ ] Remove click-drag across a chunk boundary; expected: all affected chunks remesh, persist, and broadcast.
- [ ] PaintExisting over mixed air/solid voxels; expected: air remains air.
- [ ] ReplaceAll over mixed air/solid voxels; expected: all footprint voxels become selected material.
- [ ] Undo and redo each mode; expected: changes route through server authority and second client observes both
      operations.
- [ ] Try an oversized brush if UI allows it; expected: server rejects, client rolls back predicted changes, and no
      partial terrain edit remains.
- [ ] Inspect server/client logs; expected: no unexpected warnings beyond intentional reject tests, and
      dirty/remesh/broadcast traces align with edited chunks.
