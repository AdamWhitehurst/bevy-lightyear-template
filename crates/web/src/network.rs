use bevy::prelude::*;
use protocol::*;
use std::net::SocketAddr;

// Re-export client network types
pub use client::network::{ClientNetworkConfig, ClientNetworkPlugin, ClientTransport};

/// Plugin that sets up web client networking with WebTransport
pub struct WebClientPlugin;

impl Default for WebClientPlugin {
    fn default() -> Self {
        Self
    }
}

impl Plugin for WebClientPlugin {
    fn build(&self, app: &mut App) {
        // Load certificate digest for WebTransport
        #[cfg(target_family = "wasm")]
        let certificate_digest = include_str!("../../../certificates/digest.txt").to_string();

        #[cfg(not(target_family = "wasm"))]
        let certificate_digest = String::new();

        // Configure for WebTransport on port 5001
        let config = ClientNetworkConfig {
            client_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            server_addr: SocketAddr::from(([127, 0, 0, 1], 5001)),
            client_id: 0,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            transport: ClientTransport::WebTransport { certificate_digest },
        };

        // Reuse ClientNetworkPlugin with WebTransport config
        app.add_plugins(ClientNetworkPlugin { config });
    }
}
