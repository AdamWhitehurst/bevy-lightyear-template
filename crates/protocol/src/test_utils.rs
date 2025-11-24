//! Test utilities for protocol testing
//!
//! Enable with the `test_utils` feature flag.

use bevy::prelude::App;
use lightyear::prelude::{AppMessageExt, Channel, ChannelRegistry, Message};
use lightyear_transport::channel::registry::ChannelKind;

/// Create a test protocol plugin with default settings
pub fn test_protocol_plugin() -> crate::ProtocolPlugin {
    crate::ProtocolPlugin
}

/// Verify that a channel is registered correctly
pub fn assert_channel_registered<C: Channel>(app: &App) {
    let registry = app.world().resource::<ChannelRegistry>();
    let kind = ChannelKind::of::<C>();
    assert!(
        registry.kind_map().net_id(&kind).is_some(),
        "Channel {} not registered",
        std::any::type_name::<C>()
    );
}

/// Verify that a message type is registered
pub fn assert_message_registered<M: Message>(app: &App) {
    assert!(
        app.is_message_registered::<M>(),
        "Message {} not registered",
        std::any::type_name::<M>()
    );
}
