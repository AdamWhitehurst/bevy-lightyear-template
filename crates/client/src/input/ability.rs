use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::Controlled;
use protocol::NetworkedPlayerActions;

use super::ownership::ClientInputOwnershipSnapshot;
use super::raw::RawClientActions;

const ABILITY_ACTION_MAP: [(RawClientActions, NetworkedPlayerActions); 4] = [
    (RawClientActions::Ability1, NetworkedPlayerActions::Ability1),
    (RawClientActions::Ability2, NetworkedPlayerActions::Ability2),
    (RawClientActions::Ability3, NetworkedPlayerActions::Ability3),
    (RawClientActions::Ability4, NetworkedPlayerActions::Ability4),
];

/// Copies ownership-filtered raw ability slot buttons into the networked input state.
pub fn write_filtered_ability_actions(
    ownership: Res<ClientInputOwnershipSnapshot>,
    mut query: Query<
        (
            &ActionState<RawClientActions>,
            &mut ActionState<NetworkedPlayerActions>,
        ),
        With<Controlled>,
    >,
) {
    for (raw_actions, mut networked_actions) in &mut query {
        for (_, networked_action) in ABILITY_ACTION_MAP {
            networked_actions.release(&networked_action);
        }

        if !ownership.keyboard.allows_ability_commands() {
            trace!(owner = ?ownership.keyboard, "ability keyboard input suppressed");
            continue;
        }

        for (raw_action, networked_action) in ABILITY_ACTION_MAP {
            if raw_actions.just_pressed(&raw_action) {
                networked_actions.press(&networked_action);
            }
        }
    }
}
