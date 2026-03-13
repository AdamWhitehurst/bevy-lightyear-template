//! Test utilities for protocol testing
//!
//! Enable with the `test_utils` feature flag.

use bevy::prelude::App;
use lightyear::prelude::{AppMessageExt, Message};

/// Create a test protocol plugin with default settings
pub fn test_protocol_plugin() -> crate::ProtocolPlugin {
    crate::ProtocolPlugin
}

/// Verify that a message type is registered
pub fn assert_message_registered<M: Message>(app: &App) {
    assert!(
        app.is_message_registered::<M>(),
        "Message {} not registered",
        std::any::type_name::<M>()
    );
}
