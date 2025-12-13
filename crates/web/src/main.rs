use bevy::prelude::*;
use client::gameplay::ClientGameplayPlugin;
use lightyear::prelude::client::*;
use protocol::*;
use render::RenderPlugin;
use ui::UiPlugin;
use std::time::Duration;

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
        .add_plugins(ClientGameplayPlugin)
        .add_plugins(RenderPlugin)
        .add_plugins(UiPlugin)
        .run();
}
