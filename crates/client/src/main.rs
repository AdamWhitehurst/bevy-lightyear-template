pub mod network;
pub mod gameplay;

use bevy::prelude::*;
use lightyear::prelude::client::*;
use network::{ClientNetworkConfig, ClientNetworkPlugin};
use gameplay::ClientGameplayPlugin;
use protocol::*;
use render::RenderPlugin;
use ui::{UiPlugin, UiClientConfig};
use std::time::Duration;

fn main() {
    let client_id = parse_client_id();

    let network_config = ClientNetworkConfig {
        client_id,
        ..Default::default()
    };

    // Create UI config from network config to keep them in sync
    let ui_config = UiClientConfig {
        server_addr: network_config.server_addr,
        client_id: network_config.client_id,
        protocol_id: network_config.protocol_id,
        private_key: network_config.private_key,
    };

    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(SharedGameplayPlugin)
        .add_plugins(ClientNetworkPlugin {
            config: network_config,
        })
        .insert_resource(ui_config)  // Override default UiClientConfig
        .add_plugins(ClientGameplayPlugin)
        .add_plugins(RenderPlugin)
        .add_plugins(UiPlugin)
        .run();
}

fn parse_client_id() -> u64 {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "-c" || args[i] == "--client-id" {
            if let Some(id_str) = args.get(i + 1) {
                return id_str.parse().expect("Invalid client ID");
            }
        }
    }
    0
}
