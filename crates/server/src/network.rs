use async_compat::Compat;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use lightyear::netcode::{Key, NetcodeServer};
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use protocol::*;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

const CERT_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../certificates/cert.pem");
const KEY_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../certificates/key.pem");
const REPLICATION_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Resource)]
pub struct ServerNetworkConfig {
    pub bind_addr: IpAddr,
    pub port: u16,
    pub protocol_id: u64,
    pub private_key: [u8; 32],
    pub replication_interval: Duration,
}

impl Default for ServerNetworkConfig {
    fn default() -> Self {
        Self {
            bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: 5001,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            replication_interval: REPLICATION_INTERVAL,
        }
    }
}

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
        app.register_required_components_with::<ClientOf, ReplicationSender>(|| {
            ReplicationSender::new(REPLICATION_INTERVAL, SendUpdatesMode::SinceLastAck, false)
        });
        app.add_systems(Startup, move |commands: Commands| {
            start_server(commands, config.clone());
        });
    }
}

fn load_webtransport_identity() -> lightyear::webtransport::prelude::Identity {
    IoTaskPool::get()
        .scope(|s| {
            s.spawn(Compat::new(async {
                lightyear::webtransport::prelude::Identity::load_pemfiles(CERT_PEM, KEY_PEM)
                    .await
                    .expect("Failed to load WebTransport certificates")
            }));
        })
        .pop()
        .unwrap()
}

fn start_server(mut commands: Commands, config: ServerNetworkConfig) {
    let wt_certificate = load_webtransport_identity();
    let digest = wt_certificate.certificate_chain().as_slice()[0].hash();
    info!("WebTransport certificate digest: {}", digest);

    let server = commands
        .spawn((
            Name::new("WebTransport Server"),
            Server::default(),
            NetcodeServer::new(server::NetcodeConfig {
                protocol_id: config.protocol_id,
                private_key: Key::from(config.private_key),
                ..default()
            }),
            LocalAddr(SocketAddr::from((config.bind_addr, config.port))),
            WebTransportServerIo {
                certificate: wt_certificate,
            },
        ))
        .id();
    commands.trigger(Start { entity: server });
    info!(
        "WebTransport server listening on {}:{}",
        config.bind_addr, config.port
    );
}
