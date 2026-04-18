//! Runtime toggle state for `DevPlugin` debug panels.

use bevy::prelude::*;

/// Master toggle + per-panel toggles. F4 flips `enabled`; per-panel F-keys
/// flip the matching field in `panels`.
#[derive(Resource, Default)]
pub struct DevInspectorState {
    pub enabled: bool,
    pub panels: PanelFlags,
}

/// One `bool` per debug panel. A panel is drawn iff `state.enabled && state.panels.<field>`.
#[derive(Default)]
pub struct PanelFlags {
    pub world_inspector: bool,
    pub spawn_panel: bool,
    pub netviz: bool,
    pub chunk_debugger: bool,
    pub ability_editor: bool,
    pub world_object_editor: bool,
}
