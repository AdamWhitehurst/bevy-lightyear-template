pub mod network;
pub mod gameplay;

use bevy::prelude::*;
use network::ServerNetworkPlugin;
use gameplay::ServerGameplayPlugin;
use protocol::*;
use std::time::Duration;

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(bevy::log::LogPlugin::default())
        .add_plugins(bevy::asset::AssetPlugin::default())
        .add_plugins(bevy::scene::ScenePlugin)
        .add_plugins(lightyear::prelude::server::ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(SharedGameplayPlugin)
        .add_plugins(ServerNetworkPlugin::default())
        .add_plugins(ServerGameplayPlugin)
        .run();
}
