use bevy::prelude::*;
use lightyear::netcode::Key;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use lightyear::webtransport::prelude::Identity;
use protocol::*;
use std::net::SocketAddr;
use std::time::Duration;

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(bevy::log::LogPlugin::default())
        .add_plugins(ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .add_systems(Startup, start_server)
        .add_observer(handle_new_client)
        .run();
}

fn start_server(mut commands: Commands) {
    info!("Starting multi-transport server...");

    let netcode_config = NetcodeConfig {
        protocol_id: PROTOCOL_ID,
        private_key: Key::from(PRIVATE_KEY),
        ..default()
    };

    // UDP Server (port 5000)
    let udp_server = commands.spawn((
        Name::new("UDP Server"),
        NetcodeServer::new(netcode_config.clone()),
        LocalAddr(SocketAddr::from(([0, 0, 0, 0], 5000))),
        ServerUdpIo::default(),
    )).id();
    commands.trigger(Start { entity: udp_server });
    info!("UDP server listening on 0.0.0.0:5000");

    // WebTransport Server (port 5001)
    let wt_sans = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    let wt_certificate = Identity::self_signed(wt_sans).expect("Failed to generate WebTransport certificate");
    let wt_server = commands.spawn((
        Name::new("WebTransport Server"),
        NetcodeServer::new(netcode_config.clone()),
        LocalAddr(SocketAddr::from(([0, 0, 0, 0], 5001))),
        WebTransportServerIo {
            certificate: wt_certificate,
        },
    )).id();
    commands.trigger(Start { entity: wt_server });
    info!("WebTransport server listening on 0.0.0.0:5001");

    // WebSocket Server (port 5002)
    let ws_config = lightyear::websocket::server::ServerConfig::builder()
        .with_bind_address(SocketAddr::from(([0, 0, 0, 0], 5002)))
        .with_identity(lightyear::websocket::server::Identity::self_signed(vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
        ]).expect("Failed to generate WebSocket certificate"));
    let ws_server = commands.spawn((
        Name::new("WebSocket Server"),
        NetcodeServer::new(netcode_config),
        LocalAddr(SocketAddr::from(([0, 0, 0, 0], 5002))),
        WebSocketServerIo { config: ws_config },
    )).id();
    commands.trigger(Start { entity: ws_server });
    info!("WebSocket server listening on 0.0.0.0:5002");

    info!("Server started successfully");
}

fn handle_new_client(
    trigger: On<Add, Connected>,
    mut commands: Commands,
) {
    info!("New client connected: {:?}", trigger.entity);
    commands.entity(trigger.entity).insert(ReplicationSender::new(
        Duration::from_millis(100),
        SendUpdatesMode::SinceLastAck,
        false,
    ));
}
