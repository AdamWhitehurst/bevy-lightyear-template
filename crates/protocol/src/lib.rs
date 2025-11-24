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

#[cfg(feature = "test_utils")]
pub mod test_utils;

#[cfg(test)]
mod tests {
    use super::*;
    use lightyear::prelude::{AppMessageExt, ChannelRegistry};
    use lightyear_transport::channel::registry::ChannelKind;

    #[test]
    fn test_message1_serialization() {
        let msg = Message1(42);
        let serialized = serde_json::to_string(&msg).unwrap();
        let deserialized: Message1 = serde_json::from_str(&serialized).unwrap();
        assert_eq!(msg, deserialized);
    }

    #[test]
    fn test_message1_clone() {
        let msg = Message1(42);
        let cloned = msg.clone();
        assert_eq!(msg, cloned);
    }

    #[test]
    fn test_protocol_plugin_registers_message1() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(ProtocolPlugin);

        // Verify Message1 is registered
        assert!(app.is_message_registered::<Message1>(), "Message1 not registered");
    }

    #[test]
    fn test_protocol_plugin_registers_channel1() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(ProtocolPlugin);

        // Verify Channel1 is registered
        let registry = app.world().resource::<ChannelRegistry>();
        let kind = ChannelKind::of::<Channel1>();
        assert!(registry.kind_map().net_id(&kind).is_some(), "Channel1 not registered");
    }
}
