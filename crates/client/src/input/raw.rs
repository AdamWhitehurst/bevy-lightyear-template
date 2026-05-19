use bevy::prelude::*;
use leafwing_input_manager::prelude::*;

/// Client-local physical input vocabulary populated by Leafwing before ownership filtering.
#[derive(Actionlike, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum RawClientActions {
    Move,
    CameraYaw,
    Jump,
    Ability1,
    Ability2,
    Ability3,
    Ability4,
    PlaceVoxel,
    RemoveVoxel,
    Delete,
}

/// Builds the client-local raw input map for routed ownership-sensitive inputs.
pub fn raw_client_input_map() -> InputMap<RawClientActions> {
    InputMap::default()
        .with(RawClientActions::Ability1, KeyCode::Digit1)
        .with(RawClientActions::Ability2, KeyCode::Digit2)
        .with(RawClientActions::Ability3, KeyCode::Digit3)
        .with(RawClientActions::Ability4, KeyCode::Digit4)
        .with(RawClientActions::PlaceVoxel, MouseButton::Left)
        .with(RawClientActions::RemoveVoxel, MouseButton::Right)
        .with(RawClientActions::Delete, KeyCode::Delete)
}
