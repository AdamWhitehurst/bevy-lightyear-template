pub mod network;
pub mod gameplay;

use bevy::prelude::*;
use lightyear::prelude::client::*;
use network::{ClientNetworkConfig, ClientNetworkPlugin};
use gameplay::ClientGameplayPlugin;
use protocol::*;
use render::RenderPlugin;
use ui::UiPlugin;
use std::time::Duration;

fn main() {
    let client_id = parse_client_id();

    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(SharedGameplayPlugin)
        .add_plugins(ClientNetworkPlugin {
            config: ClientNetworkConfig {
                client_id,
                ..Default::default()
            },
        })
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
