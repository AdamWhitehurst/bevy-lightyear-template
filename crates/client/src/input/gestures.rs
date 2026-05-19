use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::Controlled;

use super::ownership::{ClientInputOwnershipSnapshot, PointerInputOwner};
use super::raw::RawClientActions;

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
    action_query: Query<&ActionState<RawClientActions>, With<Controlled>>,
    snapshot: Res<ClientInputOwnershipSnapshot>,
    mut gesture: ResMut<ClientPointerGestureState>,
) {
    let Ok(action_state) = action_query.single() else {
        trace!("update_pointer_ownership: no controlled raw action state");
        return;
    };

    if gesture.owner.is_none() {
        if action_state.just_pressed(&RawClientActions::PlaceVoxel) {
            gesture.owner = Some(snapshot.pointer);
            gesture.active_button = Some(MouseButton::Left);
            return;
        }
        if action_state.just_pressed(&RawClientActions::RemoveVoxel) {
            gesture.owner = Some(snapshot.pointer);
            gesture.active_button = Some(MouseButton::Right);
            return;
        }
    }

    let Some(button) = gesture.active_button else {
        trace!("update_pointer_ownership: no active pointer gesture");
        return;
    };
    let released = match button {
        MouseButton::Left => action_state.just_released(&RawClientActions::PlaceVoxel),
        MouseButton::Right => action_state.just_released(&RawClientActions::RemoveVoxel),
        other => {
            trace!(
                ?other,
                "update_pointer_ownership: unsupported active pointer button"
            );
            false
        }
    };
    if released {
        gesture.clear();
    }
}
