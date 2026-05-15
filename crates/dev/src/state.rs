//! Runtime toggle state for `DevPlugin` debug panels.

use bevy::prelude::*;

/// Active dev editing mode used to route terrain and world-object input.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditingMode {
    #[default]
    Terrain,
    PlaceDefinition,
    PlaceFreeForm,
    SelectEdit,
}

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
}
