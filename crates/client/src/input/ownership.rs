use bevy::prelude::*;
#[cfg(feature = "spawn-panel")]
use bevy_egui::input::EguiWantsInput;

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
}

/// Client-local pointer input owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerInputOwner {
    #[default]
    World,
    Ui,
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
