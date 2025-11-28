pub mod network;

use bevy::prelude::*;
use lightyear::prelude::client::*;
use network::ClientNetworkPlugin;
use protocol::*;
use render::RenderPlugin;
use std::time::Duration;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .add_plugins(ClientNetworkPlugin::default())
        .add_plugins(RenderPlugin)
        .run();
}
