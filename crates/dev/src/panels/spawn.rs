//! Spawn panel. Selects between terrain editing, authoritative definition-driven
//! world-object placement, free-form client-local spawning, and existing world-object
//! selection/editing.
//!
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
enum SpawnPanelMode {
    #[default]
    Terrain,
    PlaceDefinition,
    PlaceFreeForm,
    SelectEdit,
}

#[derive(Resource, Default)]
pub struct SpawnPanelUi {
    mode: SpawnPanelMode,
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
    pub nearby_scan_requested: bool,
    pub nearby_objects: Vec<NearbyWorldObject>,
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

/// A nearby world object entry shown by the selection UI.
#[derive(Clone, Debug, PartialEq)]
pub struct NearbyWorldObject {
    pub entity: Entity,
    pub object_id: WorldObjectId,
    pub distance: f32,
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
            nearby_radius: 64.0,
            nearby_scan_requested: false,
            nearby_objects: Vec::new(),
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
    egui::Window::new("▧ World Objects")
        .default_width(130.0)
        .show(ctx, |ui| {
            draw_primary_tabs(ui, &mut ui_state.mode);
            ui.add_space(4.0);
            match ui_state.mode {
                SpawnPanelMode::Terrain => draw_terrain_tab(ui),
                SpawnPanelMode::PlaceDefinition => {
                    draw_definition_placement(ui, &mut ui_state, world_objects.as_deref())
                }
                SpawnPanelMode::PlaceFreeForm => {
                    draw_freeform_tab(ui, &mut ui_state, &type_registry, &mut commands)
                }
                SpawnPanelMode::SelectEdit => draw_world_object_edit_tab(ui, &mut ui_state),
            }
            ui.add_space(4.0);
            draw_status_section(ui, &ui_state);
        });
}

fn draw_primary_tabs(ui: &mut egui::Ui, mode: &mut SpawnPanelMode) {
    ui.horizontal(|ui| {
        ui.selectable_value(mode, SpawnPanelMode::Terrain, "▦  Terrain");
        ui.selectable_value(mode, SpawnPanelMode::PlaceDefinition, "▧  Place");
        ui.selectable_value(mode, SpawnPanelMode::PlaceFreeForm, "□  Free");
        ui.selectable_value(mode, SpawnPanelMode::SelectEdit, "↖  Edit");
    });
}

fn draw_section(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).strong());
            ui.add_space(3.0);
            add_contents(ui);
        });
    });
}

fn draw_terrain_tab(ui: &mut egui::Ui) {
    draw_section(ui, "TERRAIN", |_| {});
}

fn draw_definition_placement(
    ui: &mut egui::Ui,
    ui_state: &mut SpawnPanelUi,
    world_objects: Option<&WorldObjectDefRegistry>,
) {
    let Some(reg) = world_objects else {
        ui.label("World object definitions are still loading.");
        return;
    };

    egui::Grid::new("definition_placement_grid")
        .num_columns(2)
        .spacing([4.0, 2.0])
        .show(ui, |ui| {
            ui.label("Object Type");
            egui::ComboBox::from_id_salt("world_object_picker")
                .width(70.0)
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
            ui.end_row();
        });

    ui.add_space(4.0);
    let has_selection = ui_state.selected_object.is_some();
    let button_label = if ui_state.placement.armed {
        "Cancel Placement"
    } else {
        "⌖  Place Object"
    };
    if ui
        .add_enabled(
            has_selection,
            egui::Button::new(button_label).min_size(egui::vec2(55.0, 7.0)),
        )
        .clicked()
    {
        ui_state.placement.armed = !ui_state.placement.armed;
        ui_state.placement.last_reject = None;
    }
    if let Some(reason) = &ui_state.placement.last_reject {
        ui.label(format!("Last placement rejected: {reason:?}"));
    }
}

fn draw_world_object_edit_tab(ui: &mut egui::Ui, ui_state: &mut SpawnPanelUi) {
    draw_section(ui, "SELECTION", |ui| {
        ui.horizontal(|ui| {
            ui.group(|ui| {
                ui.label("Selected Object");
                ui.label(match ui_state.selection.selected {
                    Some(entity) => format!("{entity:?}"),
                    None => "(none)".to_string(),
                });
                ui.label(match ui_state.selection.selection_source {
                    Some(source) => format!("↖  Source: {source:?}"),
                    None => "↖  Source: (none)".to_string(),
                });
            });
            ui.vertical(|ui| {
                let pick_label = if ui_state.selection.cursor_pick_armed {
                    "Cancel Pick"
                } else {
                    "⌖  Pick from Scene"
                };
                if ui
                    .add(egui::Button::new(pick_label).min_size(egui::vec2(55.0, 7.0)))
                    .clicked()
                {
                    ui_state.selection.cursor_pick_armed = !ui_state.selection.cursor_pick_armed;
                    ui_state.selection.last_reject = None;
                }
                ui.horizontal(|ui| {
                    let has_selection = ui_state.selection.selected.is_some();
                    let move_label = if ui_state.selection.move_armed {
                        "Cancel Move"
                    } else {
                        "↔  Move"
                    };
                    if ui
                        .add_enabled(has_selection, egui::Button::new(move_label))
                        .clicked()
                    {
                        ui_state.selection.move_armed = !ui_state.selection.move_armed;
                        ui_state.selection.last_reject = None;
                    }
                    if ui
                        .add_enabled(has_selection, egui::Button::new("⟳  Rotate"))
                        .clicked()
                    {
                        ui_state.selection.rotate_requested = true;
                        ui_state.selection.last_reject = None;
                    }
                    if ui
                        .add_enabled(has_selection, egui::Button::new("🗑  Delete"))
                        .clicked()
                    {
                        ui_state.selection.delete_requested = true;
                        ui_state.selection.last_reject = None;
                    }
                });
            });
        });
        ui.horizontal(|ui| {
            ui.label(format!(
                "Nearby Objects ({})",
                ui_state.selection.nearby_objects.len()
            ));
            if ui.button("⟳").clicked() {
                ui_state.selection.nearby_scan_requested = true;
            }
        });
        egui::ScrollArea::vertical()
            .max_height(80.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for nearby in &ui_state.selection.nearby_objects {
                    let selected = ui_state.selection.selected == Some(nearby.entity);
                    let label = format!(
                        "{}  {}  {:.1}m",
                        nearby.entity, nearby.object_id.0, nearby.distance
                    );
                    if ui
                        .add_sized(
                            [ui.available_width(), ui.spacing().interact_size.y],
                            egui::Button::new(label).selected(selected),
                        )
                        .clicked()
                    {
                        ui_state.selection.selected = Some(nearby.entity);
                        ui_state.selection.selection_source =
                            Some(WorldObjectSelectionSource::NearbyList);
                    }
                }
            });
        if let Some(reason) = &ui_state.selection.last_reject {
            ui.label(format!("Last edit rejected: {reason:?}"));
        }
    });

    ui.add_space(4.0);
    draw_section(ui, "TRANSFORM", |ui| {
        ui.label("⟳  Yaw");
        ui.horizontal(|ui| {
            ui.add(
                egui::Slider::new(&mut ui_state.selection.rotation_degrees_y, -180.0..=180.0)
                    .show_value(false),
            );
            ui.label(format!("{:.0}°", ui_state.selection.rotation_degrees_y));
        });
        for pending in &ui_state.selection.pending_moves {
            if let (Some(old), Some(new)) = (pending.old_chunk_pos, pending.new_chunk_pos) {
                ui.label(format!("Move {}: {old:?} -> {new:?}", pending.sequence));
            }
        }
    });
}

fn draw_status_section(ui: &mut egui::Ui, ui_state: &SpawnPanelUi) {
    egui::CollapsingHeader::new("STATUS")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("world_object_status_grid")
                .num_columns(4)
                .spacing([2.0, 1.5])
                .show(ui, |ui| {
                    draw_status_item(
                        ui,
                        "↧  Pending placement requests",
                        ui_state.placement.pending.len(),
                    );
                    draw_status_item(
                        ui,
                        "↔  Pending move requests",
                        ui_state.selection.pending_moves.len(),
                    );
                    ui.end_row();
                    let accepted = ui_state
                        .placement
                        .pending
                        .iter()
                        .filter(|pending| pending.accepted_final_position.is_some())
                        .count();
                    draw_status_item(ui, "✓  Accepted awaiting replication", accepted);
                    draw_status_item(
                        ui,
                        "⟳  Pending rotation requests",
                        ui_state.selection.pending_rotations.len(),
                    );
                    ui.end_row();
                    draw_status_item(
                        ui,
                        "🗑  Pending delete requests",
                        ui_state.selection.pending_deletes.len(),
                    );
                    ui.end_row();
                });
        });
}

fn draw_status_item(ui: &mut egui::Ui, label: &str, value: usize) {
    ui.label(label);
    ui.label(value.to_string());
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
