//! Development-only tooling: physics debug rendering and runtime debug toggles.

use avian3d::prelude::{PhysicsDebugPlugin, PhysicsGizmos};
use bevy::gizmos::config::GizmoConfigStore;
use bevy::prelude::*;

/// Adds physics debug rendering and keybindings for toggling debug views at runtime.
pub struct DevPlugin;

impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsDebugPlugin::default())
            .add_systems(Startup, hide_physics_debug)
            .add_systems(Update, toggle_physics_debug);
    }
}

/// Hides the avian physics debug gizmos by default; press F3 at runtime to reveal.
fn hide_physics_debug(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<PhysicsGizmos>();
    config.enabled = false;
}

/// Toggles the avian physics debug gizmos when F3 is pressed.
fn toggle_physics_debug(keys: Res<ButtonInput<KeyCode>>, mut store: ResMut<GizmoConfigStore>) {
    if keys.just_pressed(KeyCode::F3) {
        let (config, _) = store.config_mut::<PhysicsGizmos>();
        config.enabled = !config.enabled;
    }
}
