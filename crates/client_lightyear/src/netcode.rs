use lightyear::netcode::Key;
use lightyear::prelude::client::*;
use lightyear::prelude::*;

use crate::connection::ClientNetworkConfig;

pub(crate) fn build_netcode_client(config: &ClientNetworkConfig) -> NetcodeClient {
    let auth = Authentication::Manual {
        server_addr: config.server_addr,
        client_id: config.client_id,
        private_key: Key::from(config.private_key),
        protocol_id: config.protocol_id,
    };
    let netcode_config = NetcodeConfig {
        token_expire_secs: config.token_expire_secs,
        ..Default::default()
    };
    NetcodeClient::new(auth, netcode_config).unwrap()
}
