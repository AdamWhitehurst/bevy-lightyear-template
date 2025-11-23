use bevy::prelude::*;
use lightyear::netcode::Key;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use protocol::*;
use std::net::SocketAddr;
use std::time::Duration;

const SERVER_ADDR: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
    5001, // WebTransport server port
);

const CLIENT_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0)), 0);

fn main() {
    #[cfg(target_family = "wasm")]
    console_error_panic_hook::set_once();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Lightyear WASM Client".to_string(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .add_systems(Startup, setup)
        .add_observer(on_connected)
        .add_observer(on_disconnected)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera3d::default());

    info!("WASM Client: Connecting to server at {}...", SERVER_ADDR);

    let auth = Authentication::Manual {
        server_addr: SERVER_ADDR,
        client_id: 0,
        private_key: Key::from(PRIVATE_KEY),
        protocol_id: PROTOCOL_ID,
    };

    // Load certificate digest at compile time
    let certificate_digest = {
        #[cfg(target_family = "wasm")]
        {
            include_str!("../../../certificates/digest.txt").to_string()
        }
        #[cfg(not(target_family = "wasm"))]
        {
            String::new()
        }
    };

    info!("Using certificate digest: {}", certificate_digest);

    let client = commands
        .spawn((
            Name::new("WASM Client"),
            Client::default(),
            LocalAddr(CLIENT_ADDR),
            PeerAddr(SERVER_ADDR),
            Link::new(None),
            ReplicationReceiver::default(),
            NetcodeClient::new(auth, NetcodeConfig::default())
                .expect("Failed to create NetcodeClient"),
            #[cfg(target_family = "wasm")]
            WebTransportClientIo { certificate_digest },
        ))
        .id();

    commands.trigger(Connect { entity: client });
}

fn on_connected(trigger: On<Add, Connected>) {
    info!(
        "WASM Client: Successfully connected to server! Entity: {:?}",
        trigger.entity
    );
}

fn on_disconnected(trigger: On<Add, Disconnected>) {
    info!(
        "WASM Client: Disconnected from server. Entity: {:?}",
        trigger.entity
    );
}
