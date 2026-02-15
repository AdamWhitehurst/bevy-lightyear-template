use avian3d::prelude::PhysicsDebugPlugin;
use bevy::prelude::*;
use client::gameplay::ClientGameplayPlugin;
use client::map::ClientMapPlugin;
use lightyear::prelude::client::*;
use protocol::*;
use render::RenderPlugin;
use std::time::Duration;
use ui::{UiClientConfig, UiPlugin};

pub mod network;
use network::WebClientPlugin;

fn main() {
    #[cfg(target_family = "wasm")]
    console_error_panic_hook::set_once();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Lightyear WASM Client".to_string(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(SharedGameplayPlugin)
        .add_plugins(WebClientPlugin::default())
        .insert_resource(UiClientConfig {
            server_addr: std::net::SocketAddr::from(([127, 0, 0, 1], 5001)),
            client_id: 0,
            protocol_id: protocol::PROTOCOL_ID,
            private_key: protocol::PRIVATE_KEY,
        })
        .add_plugins(ClientGameplayPlugin)
        .add_plugins(ClientMapPlugin)
        .add_plugins(RenderPlugin)
        .add_plugins(UiPlugin)
        .add_plugins(PhysicsDebugPlugin::default())
        .run();
}
