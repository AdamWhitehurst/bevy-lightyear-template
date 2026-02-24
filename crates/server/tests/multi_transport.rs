use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use lightyear::webtransport::prelude::Identity;
use protocol::{FIXED_TIMESTEP_HZ, PRIVATE_KEY, PROTOCOL_ID};
use std::time::Duration;

#[test]
fn test_server_creates_udp_transport() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ServerPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    // Spawn UDP server
    let netcode_config = NetcodeConfig {
        protocol_id: PROTOCOL_ID,
        private_key: lightyear::netcode::Key::from(PRIVATE_KEY),
        ..default()
    };

    let server_id = app
        .world_mut()
        .spawn((
            Name::new("UDP Server"),
            NetcodeServer::new(netcode_config),
            LocalAddr(std::net::SocketAddr::from(([0, 0, 0, 0], 5000))),
            ServerUdpIo::default(),
        ))
        .id();

    // Verify server entity exists
    assert!(app.world().get_entity(server_id).is_ok());

    // Verify server has NetcodeServer component
    assert!(app.world().get::<NetcodeServer>(server_id).is_some());
}

#[test]
fn test_server_creates_webtransport() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ServerPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    // Spawn WebTransport server
    let netcode_config = NetcodeConfig {
        protocol_id: PROTOCOL_ID,
        private_key: lightyear::netcode::Key::from(PRIVATE_KEY),
        ..default()
    };

    let wt_sans = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    let wt_certificate =
        Identity::self_signed(wt_sans).expect("Failed to generate WebTransport certificate");

    let server_id = app
        .world_mut()
        .spawn((
            Name::new("WebTransport Server"),
            NetcodeServer::new(netcode_config),
            LocalAddr(std::net::SocketAddr::from(([0, 0, 0, 0], 5001))),
            WebTransportServerIo {
                certificate: wt_certificate,
            },
        ))
        .id();

    // Verify server entity exists
    assert!(app.world().get_entity(server_id).is_ok());

    // Verify server has NetcodeServer component
    assert!(app.world().get::<NetcodeServer>(server_id).is_some());
}

#[test]
fn test_server_creates_websocket() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ServerPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    // Spawn WebSocket server
    let netcode_config = NetcodeConfig {
        protocol_id: PROTOCOL_ID,
        private_key: lightyear::netcode::Key::from(PRIVATE_KEY),
        ..default()
    };

    let ws_config = lightyear::websocket::server::ServerConfig::builder()
        .with_bind_address(std::net::SocketAddr::from(([0, 0, 0, 0], 5002)))
        .with_identity(
            lightyear::websocket::server::Identity::self_signed(vec![
                "localhost".to_string(),
                "127.0.0.1".to_string(),
            ])
            .expect("Failed to generate WebSocket certificate"),
        );

    let server_id = app
        .world_mut()
        .spawn((
            Name::new("WebSocket Server"),
            NetcodeServer::new(netcode_config),
            LocalAddr(std::net::SocketAddr::from(([0, 0, 0, 0], 5002))),
            WebSocketServerIo { config: ws_config },
        ))
        .id();

    // Verify server entity exists
    assert!(app.world().get_entity(server_id).is_ok());

    // Verify server has NetcodeServer component
    assert!(app.world().get::<NetcodeServer>(server_id).is_some());
}
