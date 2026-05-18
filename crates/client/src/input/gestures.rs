use bevy::prelude::*;

use super::ownership::{ClientInputOwnershipSnapshot, PointerInputOwner};

/// Pointer owner latched for an active press or drag.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientPointerGestureState {
    pub owner: Option<PointerInputOwner>,
    pub active_button: Option<MouseButton>,
}

impl ClientPointerGestureState {
    /// Returns the owner for the active gesture, or the current snapshot owner when idle.
    pub fn effective_owner(&self, snapshot_owner: PointerInputOwner) -> PointerInputOwner {
        self.owner.unwrap_or(snapshot_owner)
    }

    /// Clears the active pointer gesture.
    pub fn clear(&mut self) {
        self.owner = None;
        self.active_button = None;
    }
}

/// Latches pointer ownership from press until release.
pub fn update_pointer_ownership(
    buttons: Res<ButtonInput<MouseButton>>,
    snapshot: Res<ClientInputOwnershipSnapshot>,
    mut gesture: ResMut<ClientPointerGestureState>,
) {
    if gesture.owner.is_none() {
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            if buttons.just_pressed(button) {
                gesture.owner = Some(snapshot.pointer);
                gesture.active_button = Some(button);
                return;
            }
        }
    }

    let Some(button) = gesture.active_button else {
        trace!("update_pointer_ownership: no active pointer gesture");
        return;
    };
    if buttons.just_released(button) {
        gesture.clear();
    }
}
