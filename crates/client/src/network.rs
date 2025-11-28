use bevy::prelude::*;
use lightyear::crossbeam::CrossbeamIo;
use lightyear::netcode::Key;
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use lightyear::webtransport::client::WebTransportClientIo;
use protocol::*;
use std::net::SocketAddr;

/// Transport type for client
#[derive(Clone)]
pub enum ClientTransport {
    /// UDP transport (default for native client)
    Udp,
    /// WebTransport (for web client)
    WebTransport { certificate_digest: String },
    /// Crossbeam channels (for in-memory testing)
    Crossbeam(CrossbeamIo),
}

impl Default for ClientTransport {
    fn default() -> Self {
        Self::Udp
    }
}

/// Configuration for the client network plugin
#[derive(Clone)]
pub struct ClientNetworkConfig {
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub client_id: u64,
    pub protocol_id: u64,
    pub private_key: [u8; 32],
    pub transport: ClientTransport,
}

impl Default for ClientNetworkConfig {
    fn default() -> Self {
        Self {
            client_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            server_addr: SocketAddr::from(([127, 0, 0, 1], 5000)),
            client_id: 0,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            transport: ClientTransport::default(),
        }
    }
}

/// Plugin that sets up client networking with lightyear
pub struct ClientNetworkPlugin {
    pub config: ClientNetworkConfig,
}

impl Default for ClientNetworkPlugin {
    fn default() -> Self {
        Self {
            config: ClientNetworkConfig::default(),
        }
    }
}

impl Plugin for ClientNetworkPlugin {
    fn build(&self, app: &mut App) {
        let config = self.config.clone();
        app.add_systems(Startup, move |commands: Commands| {
            setup_client(commands, config.clone());
        });
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);
    }
}

fn setup_client(mut commands: Commands, config: ClientNetworkConfig) {
    // Create authentication
    let auth = Authentication::Manual {
        server_addr: config.server_addr,
        client_id: config.client_id,
        private_key: Key::from(config.private_key),
        protocol_id: config.protocol_id,
    };

    // Base components (always present)
    let mut entity_builder = commands.spawn((
        Name::new("Client"),
        Client::default(),
        LocalAddr(config.client_addr),
        PeerAddr(config.server_addr),
        Link::new(None),
        ReplicationReceiver::default(),
        NetcodeClient::new(auth, NetcodeConfig::default()).unwrap(),
    ));

    // Add transport-specific component
    match config.transport {
        ClientTransport::Udp => {
            entity_builder.insert(UdpIo::default());
        }
        ClientTransport::WebTransport { certificate_digest } => {
            entity_builder.insert(WebTransportClientIo { certificate_digest });
        }
        ClientTransport::Crossbeam(crossbeam_io) => {
            entity_builder.insert(crossbeam_io);
        }
    }

    let client = entity_builder.id();

    // Trigger connection
    commands.trigger(Connect { entity: client });
}

fn on_connected(trigger: On<Add, Connected>) {
    info!("Client {:?} connected!", trigger.entity);
}

fn on_disconnected(trigger: On<Add, Disconnected>) {
    info!("Client {:?} disconnected!", trigger.entity);
}
