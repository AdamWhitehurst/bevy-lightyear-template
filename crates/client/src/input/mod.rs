//! Client-local ownership and command routing.

pub mod ability;
pub mod control;
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
use self::control::write_filtered_control_actions;
#[cfg(feature = "spawn-panel")]
use self::editor::write_world_object_command_intents;
use self::editor::{write_dev_hotkey_intents, TerrainCommandIntent, WorldObjectCommandIntent};
use self::gestures::{update_pointer_ownership, ClientPointerGestureState};
#[cfg(feature = "spawn-panel")]
use self::ownership::capture_egui_input_ownership;
use self::ownership::{capture_editing_mode_pointer_ownership, ClientInputOwnershipSnapshot};
use self::raw::RawClientActions;
use self::schedule::ClientInputSet;
use dev::DevHotkeyIntent;

/// Routes physical client input into ownership-filtered network/local commands.
pub struct ClientInputCommandPlugin;

impl Plugin for ClientInputCommandPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InputManagerPlugin::<RawClientActions>::default())
            .init_resource::<ClientInputOwnershipSnapshot>()
            .init_resource::<ClientPointerGestureState>()
            .init_resource::<EditingMode>()
            .add_message::<TerrainCommandIntent>()
            .add_message::<WorldObjectCommandIntent>()
            .add_message::<DevHotkeyIntent>()
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
                    write_filtered_ability_actions,
                    write_filtered_control_actions,
                )
                    .in_set(ClientInputSet::WriteTransport)
                    .before(InputSystems::BufferClientInputs),
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

        #[cfg(feature = "spawn-panel")]
        app.add_systems(
            FixedPreUpdate,
            (
                update_pointer_ownership,
                write_world_object_command_intents,
                write_dev_hotkey_intents,
            )
                .chain()
                .in_set(ClientInputSet::ProduceLocalCommands),
        );

        #[cfg(not(feature = "spawn-panel"))]
        app.add_systems(
            FixedPreUpdate,
            (
                capture_editing_mode_pointer_ownership.in_set(ClientInputSet::Capture),
                update_pointer_ownership.in_set(ClientInputSet::ProduceLocalCommands),
                write_dev_hotkey_intents.in_set(ClientInputSet::ProduceLocalCommands),
            ),
        );
    }
}
