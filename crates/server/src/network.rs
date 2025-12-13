use bevy::prelude::*;
use lightyear::netcode::{Key, NetcodeServer};
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use protocol::*;
use std::net::SocketAddr;
use std::time::Duration;

/// Transport configuration for a server
#[derive(Clone)]
pub enum ServerTransport {
    /// UDP transport on specified port
    Udp { port: u16 },
    /// WebTransport on specified port
    WebTransport { port: u16 },
    /// WebSocket on specified port
    WebSocket { port: u16 },
    /// Crossbeam channels (for in-memory testing)
    Crossbeam {
        io: lightyear_crossbeam::CrossbeamIo,
    },
}

/// Configuration for server transports
#[derive(Clone, Resource)]
pub struct ServerNetworkConfig {
    pub transports: Vec<ServerTransport>,
    pub bind_addr: [u8; 4],
    pub protocol_id: u64,
    pub private_key: [u8; 32],
    pub replication_interval: Duration,
}

impl Default for ServerNetworkConfig {
    fn default() -> Self {
        Self {
            // TODO: add WebTransport and WebSocket transports
            // Use only UDP for now - multiple Server entities may confuse replication
            transports: vec![ServerTransport::Udp { port: 5000 }],
            bind_addr: [0, 0, 0, 0],
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            replication_interval: Duration::from_millis(100),
        }
    }
}

/// Plugin that sets up server networking with lightyear (UDP, WebTransport, WebSocket)
pub struct ServerNetworkPlugin {
    pub config: ServerNetworkConfig,
}

impl Default for ServerNetworkPlugin {
    fn default() -> Self {
        Self {
            config: ServerNetworkConfig::default(),
        }
    }
}

impl Plugin for ServerNetworkPlugin {
    fn build(&self, app: &mut App) {
        let config = self.config.clone();
        app.insert_resource(config.clone());
        app.add_systems(Startup, move |commands: Commands| {
            start_server(commands, config.clone());
        });
        app.add_observer(handle_new_client);
    }
}

fn start_server(mut commands: Commands, config: ServerNetworkConfig) {
    info!("Starting multi-transport server...");

    // Spawn servers for each transport
    for transport in config.transports {
        match transport {
            ServerTransport::Udp { port } => {
                let server = commands
                    .spawn((
                        Name::new("UDP Server"),
                        Server::default(),
                        NetcodeServer::new(server::NetcodeConfig {
                            protocol_id: config.protocol_id,
                            private_key: Key::from(config.private_key),
                            ..default()
                        }),
                        LocalAddr(SocketAddr::from((config.bind_addr, port))),
                        ServerUdpIo::default(),
                    ))
                    .id();
                commands.trigger(Start { entity: server });
                info!(
                    "UDP server listening on {}:{}",
                    config
                        .bind_addr
                        .iter()
                        .map(|b| b.to_string())
                        .collect::<Vec<_>>()
                        .join("."),
                    port
                );
            }
            ServerTransport::WebTransport { port } => {
                let wt_sans = vec![
                    "localhost".to_string(),
                    "127.0.0.1".to_string(),
                    "::1".to_string(),
                ];
                let wt_certificate =
                    lightyear::webtransport::prelude::Identity::self_signed(wt_sans)
                        .expect("Failed to generate WebTransport certificate");
                let server = commands
                    .spawn((
                        Name::new("WebTransport Server"),
                        Server::default(),
                        NetcodeServer::new(server::NetcodeConfig {
                            protocol_id: config.protocol_id,
                            private_key: Key::from(config.private_key),
                            ..default()
                        }),
                        LocalAddr(SocketAddr::from((config.bind_addr, port))),
                        WebTransportServerIo {
                            certificate: wt_certificate,
                        },
                    ))
                    .id();
                commands.trigger(Start { entity: server });
                info!(
                    "WebTransport server listening on {}:{}",
                    config
                        .bind_addr
                        .iter()
                        .map(|b| b.to_string())
                        .collect::<Vec<_>>()
                        .join("."),
                    port
                );
            }
            ServerTransport::WebSocket { port } => {
                let ws_config = lightyear::websocket::server::ServerConfig::builder()
                    .with_bind_address(SocketAddr::from((config.bind_addr, port)))
                    .with_identity(
                        lightyear::websocket::server::Identity::self_signed(vec![
                            "localhost".to_string(),
                            "127.0.0.1".to_string(),
                        ])
                        .expect("Failed to generate WebSocket certificate"),
                    );
                let server = commands
                    .spawn((
                        Name::new("WebSocket Server"),
                        Server::default(),
                        NetcodeServer::new(server::NetcodeConfig {
                            protocol_id: config.protocol_id,
                            private_key: Key::from(config.private_key),
                            ..default()
                        }),
                        LocalAddr(SocketAddr::from((config.bind_addr, port))),
                        WebSocketServerIo { config: ws_config },
                    ))
                    .id();
                commands.trigger(Start { entity: server });
                info!(
                    "WebSocket server listening on {}:{}",
                    config
                        .bind_addr
                        .iter()
                        .map(|b| b.to_string())
                        .collect::<Vec<_>>()
                        .join("."),
                    port
                );
            }
            ServerTransport::Crossbeam { io } => {
                let server = commands
                    .spawn((
                        Name::new("Crossbeam Server"),
                        Server::default(),
                        NetcodeServer::new(server::NetcodeConfig {
                            protocol_id: config.protocol_id,
                            private_key: Key::from(config.private_key),
                            ..default()
                        }),
                        io,
                    ))
                    .id();
                commands.trigger(Start { entity: server });
                info!("Crossbeam server started for testing");
            }
        }
    }

    info!("Server started successfully");
}

fn handle_new_client(
    trigger: On<Add, Connected>,
    mut commands: Commands,
    config: Res<ServerNetworkConfig>,
) {
    info!("New client connected: {:?}", trigger.entity);
    commands
        .entity(trigger.entity)
        .insert(ReplicationSender::new(
            config.replication_interval,
            SendUpdatesMode::SinceLastAck,
            false,
        ));
}
