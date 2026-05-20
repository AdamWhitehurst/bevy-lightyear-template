//! World-tree inspector panel. Wraps `bevy_inspector_egui::quick::WorldInspectorPlugin`
//! with a runtime toggle.

use crate::state::DevInspectorState;
use crate::DevHotkeyIntent;
use bevy::prelude::*;
use bevy_inspector_egui::quick::WorldInspectorPlugin;

pub struct WorldInspectorPanelPlugin;

impl Plugin for WorldInspectorPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(WorldInspectorPlugin::new().run_if(world_inspector_enabled))
            .add_message::<DevHotkeyIntent>()
            .add_systems(Update, toggle_world_inspector);
    }
}

fn world_inspector_enabled(state: Res<DevInspectorState>) -> bool {
    state.enabled && state.panels.world_inspector
}

fn toggle_world_inspector(
    mut intents: MessageReader<DevHotkeyIntent>,
    mut state: ResMut<DevInspectorState>,
) {
    for intent in intents.read() {
        if *intent == DevHotkeyIntent::ToggleWorldInspector {
            state.panels.world_inspector = !state.panels.world_inspector;
        }
    }
}
