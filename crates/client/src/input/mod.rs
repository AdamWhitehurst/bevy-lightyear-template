//! Client-local ownership and command routing.

pub mod ability;
pub mod ownership;
pub mod raw;
pub mod schedule;

use bevy::prelude::*;
use leafwing_input_manager::prelude::InputManagerPlugin;
use lightyear::prelude::client::input::InputSystems;

use self::ability::write_filtered_ability_actions;
#[cfg(feature = "spawn-panel")]
use self::ownership::capture_egui_input_ownership;
use self::ownership::ClientInputOwnershipSnapshot;
use self::raw::RawClientActions;
use self::schedule::ClientInputSet;

/// Routes physical client input into ownership-filtered network/local commands.
pub struct ClientInputCommandPlugin;

impl Plugin for ClientInputCommandPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InputManagerPlugin::<RawClientActions>::default())
            .init_resource::<ClientInputOwnershipSnapshot>()
            .configure_sets(
                FixedPreUpdate,
                (ClientInputSet::Capture, ClientInputSet::WriteTransport)
                    .chain()
                    .before(InputSystems::BufferClientInputs),
            )
            .add_systems(
                FixedPreUpdate,
                write_filtered_ability_actions
                    .in_set(ClientInputSet::WriteTransport)
                    .before(InputSystems::BufferClientInputs),
            );

        #[cfg(feature = "spawn-panel")]
        app.add_systems(
            FixedPreUpdate,
            capture_egui_input_ownership.in_set(ClientInputSet::Capture),
        );
    }
}
