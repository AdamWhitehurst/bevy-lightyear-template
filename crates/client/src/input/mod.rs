//! Client-local ownership and command routing.

pub mod ability;
pub mod editor;
pub mod gestures;
pub mod ownership;
pub mod raw;
pub mod schedule;

use bevy::prelude::*;
use dev::EditingMode;
use leafwing_input_manager::prelude::InputManagerPlugin;
use lightyear::prelude::client::input::InputSystems;

use self::ability::write_filtered_ability_actions;
use self::editor::TerrainCommandIntent;
use self::gestures::{ClientPointerGestureState, update_pointer_ownership};
#[cfg(feature = "spawn-panel")]
use self::ownership::capture_egui_input_ownership;
use self::ownership::{ClientInputOwnershipSnapshot, capture_editing_mode_pointer_ownership};
use self::raw::RawClientActions;
use self::schedule::ClientInputSet;

/// Routes physical client input into ownership-filtered network/local commands.
pub struct ClientInputCommandPlugin;

impl Plugin for ClientInputCommandPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InputManagerPlugin::<RawClientActions>::default())
            .init_resource::<ClientInputOwnershipSnapshot>()
            .init_resource::<ClientPointerGestureState>()
            .init_resource::<EditingMode>()
            .add_message::<TerrainCommandIntent>()
            .configure_sets(
                FixedPreUpdate,
                (
                    ClientInputSet::Capture,
                    ClientInputSet::WriteTransport,
                    ClientInputSet::ProduceLocalCommands,
                )
                    .chain()
                    .before(InputSystems::BufferClientInputs),
            )
            .add_systems(
                FixedPreUpdate,
                (
                    update_pointer_ownership.in_set(ClientInputSet::ProduceLocalCommands),
                    write_filtered_ability_actions
                        .in_set(ClientInputSet::WriteTransport)
                        .before(InputSystems::BufferClientInputs),
                ),
            );

        #[cfg(feature = "spawn-panel")]
        app.add_systems(
            FixedPreUpdate,
            (
                capture_egui_input_ownership,
                capture_editing_mode_pointer_ownership,
            )
                .chain()
                .in_set(ClientInputSet::Capture),
        );

        #[cfg(not(feature = "spawn-panel"))]
        app.add_systems(
            FixedPreUpdate,
            capture_editing_mode_pointer_ownership.in_set(ClientInputSet::Capture),
        );
    }
}
