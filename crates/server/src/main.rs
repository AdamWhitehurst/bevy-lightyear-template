pub mod gameplay;
pub mod map;
pub mod network;

use bevy::prelude::*;
use gameplay::ServerGameplayPlugin;
use map::ServerMapPlugin;
use network::ServerNetworkPlugin;
use protocol::*;
use std::time::Duration;

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(bevy::log::LogPlugin::default())
        .add_plugins(bevy::asset::AssetPlugin::default())
        .add_plugins(bevy::scene::ScenePlugin)
        // Register asset resources for voxel world mesh generation
        .add_message::<bevy::asset::AssetEvent<bevy::prelude::Mesh>>()
        .init_asset::<bevy::prelude::Mesh>()
        .init_asset::<bevy::shader::Shader>()
        .add_message::<bevy::asset::AssetEvent<bevy::shader::Shader>>()
        .init_asset::<bevy::image::Image>()
        .add_message::<bevy::asset::AssetEvent<bevy::image::Image>>()
        .add_plugins(lightyear::prelude::server::ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(SharedGameplayPlugin)
        .add_plugins(ServerNetworkPlugin::default())
        .add_plugins(ServerGameplayPlugin)
        .add_plugins(ServerMapPlugin)
        .run();
}
