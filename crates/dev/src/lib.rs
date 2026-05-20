//! Development-only tooling: physics debug rendering, runtime toggles, and
//! optional `bevy-inspector-egui` panels (behind the `inspector` Cargo feature).

use avian3d::prelude::{PhysicsDebugPlugin, PhysicsGizmos};
use bevy::gizmos::config::GizmoConfigStore;
use bevy::prelude::*;

mod state;
pub use state::{DevInspectorState, EditingMode, PanelFlags};

/// Client-routed developer hotkey commands.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevHotkeyIntent {
    /// Toggles physics debug gizmos.
    TogglePhysicsDebug,
    /// Toggles the dev inspector root menu.
    ToggleDevInspector,
    /// Toggles the world inspector panel.
    ToggleWorldInspector,
    /// Toggles the spawn panel.
    ToggleSpawnPanel,
}

#[cfg(feature = "inspector")]
pub mod panels;

#[cfg(all(feature = "inspector", feature = "spawn-panel"))]
pub use panels::terrain::{
    AcknowledgedVoxelChange, TerrainBrushSettings, TerrainBrushStrokeMode, TerrainEditHistory,
    TerrainEditRecord, TerrainPanelPlugin,
};

/// Adds physics debug rendering, runtime debug toggles, and optional inspector panels.
pub struct DevPlugin;

impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsDebugPlugin)
            .init_resource::<DevInspectorState>()
            .init_resource::<EditingMode>()
            .add_message::<DevHotkeyIntent>()
            .add_systems(Startup, hide_physics_debug)
            .add_systems(Update, (toggle_physics_debug, toggle_dev_inspector));

        #[cfg(feature = "inspector")]
        {
            use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
            app.add_plugins(EguiPlugin::default()).add_systems(
                EguiPrimaryContextPass,
                draw_root_menu.run_if(inspector_enabled),
            );

            #[cfg(feature = "world-inspector")]
            app.add_plugins(panels::world_inspector::WorldInspectorPanelPlugin);

            #[cfg(feature = "spawn-panel")]
            app.add_plugins((panels::spawn::SpawnPanelPlugin, TerrainPanelPlugin));
        }
    }
}

/// Hides the avian physics debug gizmos by default; press F3 at runtime to reveal.
fn hide_physics_debug(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<PhysicsGizmos>();
    config.enabled = false;
}

/// Toggles the avian physics debug gizmos from routed dev hotkey intents.
fn toggle_physics_debug(
    mut intents: MessageReader<DevHotkeyIntent>,
    mut store: ResMut<GizmoConfigStore>,
) {
    for intent in intents.read() {
        if *intent == DevHotkeyIntent::TogglePhysicsDebug {
            let (config, _) = store.config_mut::<PhysicsGizmos>();
            config.enabled = !config.enabled;
        }
    }
}

/// Toggles the dev inspector root menu from routed dev hotkey intents.
fn toggle_dev_inspector(
    mut intents: MessageReader<DevHotkeyIntent>,
    mut state: ResMut<DevInspectorState>,
) {
    for intent in intents.read() {
        if *intent == DevHotkeyIntent::ToggleDevInspector {
            state.enabled = !state.enabled;
        }
    }
}

#[cfg(feature = "inspector")]
fn inspector_enabled(state: Res<DevInspectorState>) -> bool {
    state.enabled
}

#[cfg(feature = "inspector")]
fn draw_root_menu(mut state: ResMut<DevInspectorState>, mut contexts: bevy_egui::EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else {
        // EguiContexts not yet attached to the primary window.
        trace!("draw_root_menu: EguiContexts not ready, skipping frame");
        return;
    };
    bevy_egui::egui::TopBottomPanel::top("dev_inspector_root").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Dev Inspector");
            ui.separator();
            #[cfg(feature = "world-inspector")]
            ui.checkbox(&mut state.panels.world_inspector, "World");
            #[cfg(feature = "spawn-panel")]
            ui.checkbox(&mut state.panels.spawn_panel, "Spawn");
            let _ = &mut state; // silence unused-mut when no panel features are enabled
        });
    });
}
