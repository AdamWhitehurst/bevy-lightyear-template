use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

// Message definitions
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Message1(pub usize);

// Channel marker
pub struct Channel1;

// Protocol registration plugin
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        // Register message
        app.register_message::<Message1>()
            .add_direction(NetworkDirection::Bidirectional);

        // Register channel
        app.add_channel::<Channel1>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);
    }
}

// Shared constants
pub const PROTOCOL_ID: u64 = 0;
pub const PRIVATE_KEY: [u8; 32] = [0; 32];
pub const FIXED_TIMESTEP_HZ: f64 = 64.0;
