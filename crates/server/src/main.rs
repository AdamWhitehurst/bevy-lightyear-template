pub mod chunk_entities;
pub mod diagnostics;
pub mod gameplay;
pub mod map;
pub mod persistence;
pub mod transition;
pub mod world_object;

use bevy::prelude::*;
use diagnostics::ServerDiagnosticsPlugin;
use gameplay::ServerGameplayPlugin;
use map::ServerMapPlugin;
use protocol::diagnostics::SharedDiagnosticsPlugin;
use protocol::*;
use server_lightyear::{ServerNetworkConfig, ServerNetworkPlugin};
use std::time::Duration;

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(bevy::app::TerminalCtrlCHandlerPlugin)
        .add_plugins(bevy::state::app::StatesPlugin)
        .add_plugins(bevy::log::LogPlugin::default())
        .add_plugins(bevy::asset::AssetPlugin {
            file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets").to_string(),
            ..default()
        })
        .add_plugins(bevy::transform::TransformPlugin)
        .add_plugins(bevy::scene::ScenePlugin)
        // Register asset resources for voxel world mesh generation
        .add_message::<bevy::asset::AssetEvent<bevy::prelude::Mesh>>()
        .init_asset::<bevy::prelude::Mesh>()
        .init_asset::<bevy::pbr::StandardMaterial>()
        .init_asset::<bevy::shader::Shader>()
        .add_message::<bevy::asset::AssetEvent<bevy::shader::Shader>>()
        .init_asset::<bevy::image::Image>()
        .add_message::<bevy::asset::AssetEvent<bevy::image::Image>>()
        .add_plugins(lightyear::prelude::server::ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(SharedGameplayPlugin)
        .add_plugins(ServerNetworkPlugin {
            config: ServerNetworkConfig {
                cert_pem_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../certificates/cert.pem")
                    .into(),
                key_pem_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../certificates/key.pem")
                    .into(),
                ..Default::default()
            },
        })
        .add_plugins(ServerGameplayPlugin)
        .add_plugins(ServerMapPlugin)
        .add_plugins(SharedDiagnosticsPlugin)
        .add_plugins(ServerDiagnosticsPlugin)
        .run();
}
