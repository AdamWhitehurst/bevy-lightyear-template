use bevy::prelude::*;
#[cfg(feature = "spawn-panel")]
use bevy_egui::input::EguiWantsInput;
use dev::{DevInspectorState, EditingMode};

/// Client-local input ownership visible to fixed-tick transport writers.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientInputOwnershipSnapshot {
    pub keyboard: KeyboardInputOwner,
    pub pointer: PointerInputOwner,
}

/// Client-local keyboard input owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyboardInputOwner {
    #[default]
    Gameplay,
    Ui,
    Text,
}

impl KeyboardInputOwner {
    /// Returns whether ability commands may be emitted.
    pub fn allows_ability_commands(self) -> bool {
        matches!(self, Self::Gameplay)
    }

    /// Returns whether keyboard-origin world-object commands may be emitted.
    pub fn allows_world_object_commands(self) -> bool {
        matches!(self, Self::Gameplay)
    }

    /// Returns whether locomotion commands may be emitted.
    pub fn allows_locomotion(self) -> bool {
        matches!(self, Self::Gameplay)
    }

    /// Returns whether jump commands may be emitted.
    pub fn allows_jump(self) -> bool {
        matches!(self, Self::Gameplay)
    }

    /// Returns whether keyboard-origin camera controls may be emitted.
    pub fn allows_camera_control(self) -> bool {
        matches!(self, Self::Gameplay)
    }

    /// Returns whether developer hotkeys may be emitted.
    pub fn allows_dev_hotkeys(self) -> bool {
        matches!(self, Self::Gameplay)
    }
}

/// Client-local pointer input owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerInputOwner {
    #[default]
    World,
    Ui,
    TerrainBrush,
    WorldObject,
}

impl PointerInputOwner {
    /// Returns whether terrain commands may be emitted.
    pub fn allows_terrain(self) -> bool {
        matches!(self, Self::TerrainBrush)
    }

    /// Returns whether world-object commands may be emitted.
    pub fn allows_world_object(self) -> bool {
        matches!(self, Self::WorldObject)
    }

    /// Returns whether camera control may update networked movement yaw.
    pub fn allows_camera_control(self) -> bool {
        matches!(self, Self::World)
    }
}

/// Applies egui focus state to the client input ownership snapshot.
pub fn apply_egui_ownership_state(
    ownership: &mut ClientInputOwnershipSnapshot,
    wants_keyboard_input: bool,
    wants_pointer_input: bool,
) {
    ownership.keyboard = if wants_keyboard_input {
        KeyboardInputOwner::Text
    } else {
        KeyboardInputOwner::Gameplay
    };
    ownership.pointer = if wants_pointer_input {
        PointerInputOwner::Ui
    } else {
        PointerInputOwner::World
    };
}

/// Applies the active dev editing mode to non-UI pointer ownership.
pub fn apply_editing_mode_pointer_ownership(
    ownership: &mut ClientInputOwnershipSnapshot,
    editing_mode: EditingMode,
) {
    if ownership.pointer == PointerInputOwner::Ui {
        trace!("apply_editing_mode_pointer_ownership: preserving UI pointer ownership");
        return;
    }

    ownership.pointer = if editing_mode.wants_terrain_pointer() {
        PointerInputOwner::TerrainBrush
    } else if editing_mode.wants_world_object_pointer() {
        PointerInputOwner::WorldObject
    } else {
        PointerInputOwner::World
    };
}

/// Captures active editing mode pointer ownership for fixed-tick command routing.
pub fn capture_editing_mode_pointer_ownership(
    editing_mode: Res<EditingMode>,
    inspector_state: Option<Res<DevInspectorState>>,
    mut ownership: ResMut<ClientInputOwnershipSnapshot>,
) {
    let editing_panel_active = inspector_state
        .as_deref()
        .is_some_and(|state| state.enabled && state.panels.spawn_panel);
    if editing_panel_active {
        apply_editing_mode_pointer_ownership(&mut ownership, *editing_mode);
        return;
    }
    if ownership.pointer == PointerInputOwner::Ui {
        trace!("capture_editing_mode_pointer_ownership: preserving UI pointer ownership");
        return;
    }
    ownership.pointer = PointerInputOwner::World;
}

/// Captures egui input ownership for fixed-tick command routing.
#[cfg(feature = "spawn-panel")]
pub fn capture_egui_input_ownership(
    mut ownership: ResMut<ClientInputOwnershipSnapshot>,
    egui_wants_input: Option<Res<EguiWantsInput>>,
) {
    let Some(egui_wants_input) = egui_wants_input else {
        trace!("capture_egui_input_ownership: EguiWantsInput not ready; preserving existing owner");
        return;
    };

    apply_egui_ownership_state(
        &mut ownership,
        egui_wants_input.wants_keyboard_input(),
        egui_wants_input.wants_any_pointer_input(),
    );
}
