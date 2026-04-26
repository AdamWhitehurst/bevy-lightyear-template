use bevy::prelude::*;
use client_lightyear::{ClientNetworkConfig, ClientNetworkPlugin};
use protocol::*;
use std::net::SocketAddr;

pub struct WebClientPlugin;

impl Default for WebClientPlugin {
    fn default() -> Self {
        Self
    }
}

impl Plugin for WebClientPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(target_family = "wasm")]
        let certificate_digest = include_str!("../../../certificates/digest.txt").to_string();
        #[cfg(not(target_family = "wasm"))]
        let certificate_digest = String::new();

        let config = ClientNetworkConfig {
            client_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            server_addr: SocketAddr::from(([127, 0, 0, 1], 5001)),
            client_id: 0,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            certificate_digest,
            ..default()
        };

        app.add_plugins(ClientNetworkPlugin { config });
    }
}
