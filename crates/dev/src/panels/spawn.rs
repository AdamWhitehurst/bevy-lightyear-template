//! Spawn panel. Two tabs:
//!   * **Def-driven**: pick a registered `WorldObjectId` / `AbilityId` and spawn via the
//!     existing `apply_object_components` / `apply_ability_archetype` pipelines.
//!   * **Free-form**: pick any reflected `Component` from the `AppTypeRegistry` and
//!     instantiate via `ReflectDefault`.
//! All spawns are client-local (no `Replicate`) at the world origin and carry a
//! `DevSpawned` marker.

use crate::state::DevInspectorState;
use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::prelude::ReflectDefault;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use protocol::ability::loader::apply_ability_archetype;
use protocol::ability::{AbilityAsset, AbilityDefs, AbilityId};
use protocol::map::MapInstanceId;
use protocol::world_object::{
    apply_object_components, WorldObjectDefRegistry, WorldObjectId,
};

/// Marker for any entity spawned via the dev spawn panel. Client-local; not replicated.
#[derive(Component)]
pub struct DevSpawned;

#[derive(Default, PartialEq, Eq)]
enum SpawnTab {
    #[default]
    DefDriven,
    FreeForm,
}

#[derive(Resource, Default)]
struct SpawnPanelUi {
    tab: SpawnTab,
    selected_object: Option<WorldObjectId>,
    selected_ability: Option<AbilityId>,
    selected_freeform: Vec<String>,
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
    world_objects: Option<Res<WorldObjectDefRegistry>>,
    abilities: Option<Res<AbilityDefs>>,
    type_registry: Res<AppTypeRegistry>,
    ability_assets: Res<Assets<AbilityAsset>>,
    mut commands: Commands,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("draw_spawn_panel: EguiContexts not ready, skipping frame");
        return;
    };
    egui::Window::new("Spawn (client-local)").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut ui_state.tab, SpawnTab::DefDriven, "Def-driven");
            ui.selectable_value(&mut ui_state.tab, SpawnTab::FreeForm, "Free-form");
        });
        ui.separator();
        ui.label("Spawned at world origin; client-local (no Replicate).");
        ui.separator();
        match ui_state.tab {
            SpawnTab::DefDriven => draw_def_tab(
                ui,
                &mut ui_state,
                world_objects.as_deref(),
                abilities.as_deref(),
                &type_registry,
                &ability_assets,
                &mut commands,
            ),
            SpawnTab::FreeForm => draw_freeform_tab(
                ui,
                &mut ui_state,
                &type_registry,
                &mut commands,
            ),
        }
    });
}

fn draw_def_tab(
    ui: &mut egui::Ui,
    ui_state: &mut SpawnPanelUi,
    world_objects: Option<&WorldObjectDefRegistry>,
    abilities: Option<&AbilityDefs>,
    type_registry: &AppTypeRegistry,
    ability_assets: &Assets<AbilityAsset>,
    commands: &mut Commands,
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
        if ui.button("Spawn world object").clicked() {
            if let Some(id) = ui_state.selected_object.clone() {
                if let Some(def) = reg.objects.get(&id) {
                    let entity = commands
                        .spawn((
                            id.clone(),
                            Transform::default(),
                            DevSpawned,
                            MapInstanceId::Overworld,
                            Name::new(format!("dev:{}", id.0)),
                        ))
                        .id();
                    let components = def
                        .components
                        .iter()
                        .map(|c| {
                            c.reflect_clone()
                                .expect("world object component must be cloneable")
                                .into_partial_reflect()
                        })
                        .collect();
                    apply_object_components(commands, entity, components, type_registry.0.clone());
                }
            }
        }
    } else {
        ui.label("(WorldObjectDefRegistry not yet loaded)");
    }
    ui.separator();
    ui.label("Ability");
    if let Some(defs) = abilities {
        egui::ComboBox::from_id_salt("ability_picker")
            .selected_text(
                ui_state
                    .selected_ability
                    .as_ref()
                    .map(|i| i.0.as_str())
                    .unwrap_or("(pick)"),
            )
            .show_ui(ui, |ui| {
                let mut ids: Vec<&AbilityId> = defs.abilities.keys().collect();
                ids.sort_by(|a, b| a.0.cmp(&b.0));
                for id in ids {
                    ui.selectable_value(&mut ui_state.selected_ability, Some(id.clone()), &id.0);
                }
            });
        if ui.button("Spawn ability").clicked() {
            if let Some(id) = ui_state.selected_ability.clone() {
                if let Some(handle) = defs.get(&id) {
                    if let Some(asset) = ability_assets.get(handle) {
                        let entity = commands
                            .spawn((
                                Transform::default(),
                                DevSpawned,
                                MapInstanceId::Overworld,
                                Name::new(format!("dev:{}", id.0)),
                            ))
                            .id();
                        apply_ability_archetype(
                            commands,
                            entity,
                            asset,
                            type_registry.0.clone(),
                            Vec::new(),
                        );
                    }
                }
            }
        }
    } else {
        ui.label("(AbilityDefs not yet loaded)");
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
    ui.label("Pick reflected Components (multi-select):");
    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
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
