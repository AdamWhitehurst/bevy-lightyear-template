use bevy::prelude::*;
use lightyear::netcode::{Key, NetcodeServer};
use lightyear::prelude::server;

use crate::connection::ServerNetworkConfig;

pub(crate) fn build_netcode_server(config: &ServerNetworkConfig) -> NetcodeServer {
    NetcodeServer::new(server::NetcodeConfig {
        protocol_id: config.protocol_id,
        private_key: Key::from(config.private_key),
        ..default()
    })
}
