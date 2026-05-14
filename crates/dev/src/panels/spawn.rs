//! Spawn panel. Two tabs:
//!   * **Def-driven**: pick a registered `WorldObjectId` and arm authoritative
//!     server placement from client terrain input.
//!   * **Free-form**: pick any reflected `Component` from the `AppTypeRegistry` and
//!     instantiate client-locally via `ReflectDefault`.
//! Free-form spawns are client-local (no `Replicate`) at the world origin and
//! carry a `DevSpawned` marker.

use crate::state::DevInspectorState;
use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use protocol::map::MapInstanceId;
use protocol::world_object::{
    apply_object_components, WorldObjectDefRegistry, WorldObjectEditRejectReason, WorldObjectId,
    WorldObjectPlacementRejectReason,
};

/// Marker for any entity spawned via the dev spawn panel. Client-local; not replicated.
#[derive(Component)]
pub struct DevSpawned;

#[derive(Default, PartialEq, Eq)]
pub enum SpawnTab {
    #[default]
    DefDriven,
    FreeForm,
}

#[derive(Resource, Default)]
pub struct SpawnPanelUi {
    tab: SpawnTab,
    pub selected_object: Option<WorldObjectId>,
    pub placement: WorldObjectPlacementUi,
    pub selection: WorldObjectSelectionUi,
    selected_freeform: Vec<String>,
}

/// Client-owned world-object placement request state shown by the spawn panel.
#[derive(Default)]
pub struct WorldObjectPlacementUi {
    pub armed: bool,
    pub next_sequence: u32,
    pub pending: Vec<PendingWorldObjectPlacement>,
    pub last_reject: Option<WorldObjectPlacementRejectReason>,
}

/// A pending authoritative world-object placement request.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingWorldObjectPlacement {
    pub sequence: u32,
    pub object_id: WorldObjectId,
    pub base_position: Vec3,
    pub accepted_final_position: Option<Vec3>,
}

impl WorldObjectPlacementUi {
    /// Returns the next placement sequence number and increments it.
    pub fn next_sequence(&mut self) -> u32 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }
}

/// Source used to select the current world object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldObjectSelectionSource {
    Cursor,
    NearbyList,
}

/// Client-owned world-object selection and edit request state shown by the spawn panel.
pub struct WorldObjectSelectionUi {
    pub selected: Option<Entity>,
    pub selection_source: Option<WorldObjectSelectionSource>,
    pub nearby_radius: f32,
    pub next_sequence: u32,
    pub pending_deletes: Vec<PendingWorldObjectDelete>,
    pub pending_moves: Vec<PendingWorldObjectMove>,
    pub pending_rotations: Vec<PendingWorldObjectRotation>,
    pub move_armed: bool,
    pub cursor_pick_armed: bool,
    pub rotation_degrees_y: f32,
    pub delete_requested: bool,
    pub rotate_requested: bool,
    pub last_reject: Option<WorldObjectEditRejectReason>,
}

/// A pending authoritative world-object delete request.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingWorldObjectDelete {
    pub sequence: u32,
    pub target: Entity,
    pub accepted: bool,
}

/// A pending authoritative world-object move request.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingWorldObjectMove {
    pub sequence: u32,
    pub target: Entity,
    pub final_position: Vec3,
    pub old_chunk_pos: Option<IVec3>,
    pub new_chunk_pos: Option<IVec3>,
    pub accepted: bool,
}

/// A pending authoritative world-object rotation request.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingWorldObjectRotation {
    pub sequence: u32,
    pub target: Entity,
    pub rotation: Quat,
    pub accepted: bool,
}

impl Default for WorldObjectSelectionUi {
    fn default() -> Self {
        Self {
            selected: None,
            selection_source: None,
            nearby_radius: 12.0,
            next_sequence: 0,
            pending_deletes: Vec::new(),
            pending_moves: Vec::new(),
            pending_rotations: Vec::new(),
            move_armed: false,
            cursor_pick_armed: false,
            rotation_degrees_y: 0.0,
            delete_requested: false,
            rotate_requested: false,
            last_reject: None,
        }
    }
}

impl WorldObjectSelectionUi {
    /// Returns the next edit sequence number and increments it.
    pub fn next_sequence(&mut self) -> u32 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }
}

pub struct SpawnPanelPlugin;

impl Plugin for SpawnPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnPanelUi>()
            .add_systems(Update, toggle_spawn_panel)
            .add_systems(
                EguiPrimaryContextPass,
                draw_spawn_panel.run_if(spawn_panel_enabled),
            );
    }
}

fn spawn_panel_enabled(state: Res<DevInspectorState>) -> bool {
    state.enabled && state.panels.spawn_panel
}

fn toggle_spawn_panel(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<DevInspectorState>) {
    if keys.just_pressed(KeyCode::F6) {
        state.panels.spawn_panel = !state.panels.spawn_panel;
    }
}

fn draw_spawn_panel(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<SpawnPanelUi>,
    // Optional because definitions load during startup; the panel renders a loading label until ready.
    world_objects: Option<Res<WorldObjectDefRegistry>>,
    type_registry: Res<AppTypeRegistry>,
    mut commands: Commands,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("draw_spawn_panel: EguiContexts not ready, skipping frame");
        return;
    };
    egui::Window::new("Spawn").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut ui_state.tab, SpawnTab::DefDriven, "Def-driven");
            ui.selectable_value(&mut ui_state.tab, SpawnTab::FreeForm, "Free-form");
        });
        ui.separator();
        ui.label(
            "Def-driven placement is server-authoritative; free-form spawning is client-local.",
        );
        ui.separator();
        match ui_state.tab {
            SpawnTab::DefDriven => draw_def_tab(ui, &mut ui_state, world_objects.as_deref()),
            SpawnTab::FreeForm => {
                draw_freeform_tab(ui, &mut ui_state, &type_registry, &mut commands)
            }
        }
    });
}

fn draw_def_tab(
    ui: &mut egui::Ui,
    ui_state: &mut SpawnPanelUi,
    world_objects: Option<&WorldObjectDefRegistry>,
) {
    ui.label("World Object");
    if let Some(reg) = world_objects {
        egui::ComboBox::from_id_salt("world_object_picker")
            .selected_text(
                ui_state
                    .selected_object
                    .as_ref()
                    .map(|i| i.0.as_str())
                    .unwrap_or("(pick)"),
            )
            .show_ui(ui, |ui| {
                let mut ids: Vec<&WorldObjectId> = reg.objects.keys().collect();
                ids.sort_by(|a, b| a.0.cmp(&b.0));
                for id in ids {
                    ui.selectable_value(&mut ui_state.selected_object, Some(id.clone()), &id.0);
                }
            });
        let has_selection = ui_state.selected_object.is_some();
        if ui
            .add_enabled(
                has_selection && !ui_state.placement.armed,
                egui::Button::new("Arm placement"),
            )
            .clicked()
        {
            ui_state.placement.armed = true;
            ui_state.placement.last_reject = None;
        }

        if ui_state.placement.armed && ui.button("Cancel placement").clicked() {
            ui_state.placement.armed = false;
        }

        ui.label(if ui_state.placement.armed {
            "Placement armed: click terrain to request server placement."
        } else {
            "Select an object and arm placement."
        });
        ui.label(format!(
            "Pending placement requests: {}",
            ui_state.placement.pending.len()
        ));
        let accepted = ui_state
            .placement
            .pending
            .iter()
            .filter(|pending| pending.accepted_final_position.is_some())
            .count();
        ui.label(format!(
            "Accepted placements awaiting replication: {accepted}"
        ));
        if let Some(reason) = &ui_state.placement.last_reject {
            ui.label(format!("Last placement rejected: {reason:?}"));
        }

        ui.separator();
        draw_world_object_edit_tab(ui, ui_state);
    } else {
        ui.label("(WorldObjectDefRegistry not yet loaded)");
    }
}

fn draw_world_object_edit_tab(ui: &mut egui::Ui, ui_state: &mut SpawnPanelUi) {
    ui.label("Existing World Object");
    ui.label(match ui_state.selection.selected {
        Some(entity) => format!("Selected: {entity:?}"),
        None => "Selected: (none)".to_string(),
    });
    ui.label(match ui_state.selection.selection_source {
        Some(source) => format!("Selection source: {source:?}"),
        None => "Selection source: (none)".to_string(),
    });
    if ui
        .add_enabled(
            !ui_state.selection.cursor_pick_armed,
            egui::Button::new("Arm cursor pick"),
        )
        .clicked()
    {
        ui_state.selection.cursor_pick_armed = true;
        ui_state.selection.last_reject = None;
    }
    if ui_state.selection.cursor_pick_armed && ui.button("Cancel cursor pick").clicked() {
        ui_state.selection.cursor_pick_armed = false;
    }
    ui.label(if ui_state.selection.cursor_pick_armed {
        "Cursor pick armed: click an existing world object in-game."
    } else {
        "Arm cursor pick, then click an existing world object in-game."
    });
    ui.add(
        egui::Slider::new(&mut ui_state.selection.nearby_radius, 1.0..=64.0).text("Nearby radius"),
    );
    if ui
        .add_enabled(
            ui_state.selection.selected.is_some(),
            egui::Button::new("Delete selected"),
        )
        .clicked()
    {
        ui_state.selection.delete_requested = true;
        ui_state.selection.last_reject = None;
    }
    ui.label("Press Delete to delete selected.");
    if ui
        .add_enabled(
            ui_state.selection.selected.is_some() && !ui_state.selection.move_armed,
            egui::Button::new("Arm move"),
        )
        .clicked()
    {
        ui_state.selection.move_armed = true;
        ui_state.selection.last_reject = None;
    }
    if ui_state.selection.move_armed && ui.button("Cancel move").clicked() {
        ui_state.selection.move_armed = false;
    }
    ui.label(if ui_state.selection.move_armed {
        "Move armed: click terrain to request server move."
    } else {
        "Arm move to preview moving the selected object."
    });
    ui.label(format!(
        "Pending delete requests: {}",
        ui_state.selection.pending_deletes.len()
    ));
    ui.add(
        egui::Slider::new(&mut ui_state.selection.rotation_degrees_y, -180.0..=180.0).text("Yaw"),
    );
    if ui
        .add_enabled(
            ui_state.selection.selected.is_some(),
            egui::Button::new("Rotate selected"),
        )
        .clicked()
    {
        ui_state.selection.rotate_requested = true;
        ui_state.selection.last_reject = None;
    }
    ui.label(format!(
        "Pending move requests: {}",
        ui_state.selection.pending_moves.len()
    ));
    for pending in &ui_state.selection.pending_moves {
        if let (Some(old), Some(new)) = (pending.old_chunk_pos, pending.new_chunk_pos) {
            ui.label(format!("Move {}: {old:?} -> {new:?}", pending.sequence));
        }
    }
    ui.label(format!(
        "Pending rotation requests: {}",
        ui_state.selection.pending_rotations.len()
    ));
    if let Some(reason) = &ui_state.selection.last_reject {
        ui.label(format!("Last edit rejected: {reason:?}"));
    }
}

fn draw_freeform_tab(
    ui: &mut egui::Ui,
    ui_state: &mut SpawnPanelUi,
    type_registry: &AppTypeRegistry,
    commands: &mut Commands,
) {
    let registry = type_registry.read();
    let mut component_paths: Vec<String> = registry
        .iter()
        .filter(|reg| reg.data::<ReflectComponent>().is_some())
        .map(|reg| reg.type_info().type_path().to_string())
        .collect();
    component_paths.sort();
    ui.label("Pick reflected Components (multi-select, client-local):");
    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            for path in &component_paths {
                let mut checked = ui_state.selected_freeform.iter().any(|p| p == path);
                if ui.checkbox(&mut checked, path).changed() {
                    if checked {
                        ui_state.selected_freeform.push(path.clone());
                    } else {
                        ui_state.selected_freeform.retain(|p| p != path);
                    }
                }
            }
        });
    let spawn_clicked = ui.button("Spawn with selected components").clicked();
    if spawn_clicked && !ui_state.selected_freeform.is_empty() {
        let mut components: Vec<Box<dyn bevy::reflect::PartialReflect>> = Vec::new();
        for path in &ui_state.selected_freeform {
            let Some(reg) = registry.get_with_type_path(path) else {
                warn!("freeform spawn: type {path} not in registry, skipping");
                continue;
            };
            let Some(default) = reg.data::<ReflectDefault>() else {
                warn!("freeform spawn: type {path} has no ReflectDefault, skipping");
                continue;
            };
            components.push(default.default().into_partial_reflect());
        }
        drop(registry);
        let entity = commands
            .spawn((
                Transform::default(),
                DevSpawned,
                MapInstanceId::Overworld,
                Name::new("dev:freeform"),
            ))
            .id();
        apply_object_components(commands, entity, components, type_registry.0.clone());
    }
}
