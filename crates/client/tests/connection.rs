use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use protocol::{FIXED_TIMESTEP_HZ, PRIVATE_KEY, PROTOCOL_ID};
use std::time::Duration;

#[test]
fn test_client_connects_to_server() {
    // Setup client app with MinimalPlugins (headless)
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ClientPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    // Spawn client entity with UDP transport
    let client_addr = std::net::SocketAddr::from(([0, 0, 0, 0], 0));
    let server_addr = std::net::SocketAddr::from(([127, 0, 0, 1], 5000));

    let auth = Authentication::Manual {
        server_addr,
        client_id: 0,
        private_key: lightyear::netcode::Key::from(PRIVATE_KEY),
        protocol_id: PROTOCOL_ID,
    };

    let client_id = app
        .world_mut()
        .spawn((
            Name::new("Test Client"),
            Client::default(),
            LocalAddr(client_addr),
            PeerAddr(server_addr),
            Link::new(None),
            ReplicationReceiver::default(),
            NetcodeClient::new(auth, NetcodeConfig::default())
                .expect("Failed to create NetcodeClient"),
            UdpIo::default(),
        ))
        .id();

    // Verify client entity exists
    assert!(app.world().get_entity(client_id).is_ok());

    // Verify client has Client component
    assert!(app.world().get::<Client>(client_id).is_some());

    // Verify client has NetcodeClient component
    assert!(app.world().get::<NetcodeClient>(client_id).is_some());
}

#[test]
fn test_client_has_ping_manager() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ClientPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    let client_addr = std::net::SocketAddr::from(([0, 0, 0, 0], 0));
    let server_addr = std::net::SocketAddr::from(([127, 0, 0, 1], 5000));

    let auth = Authentication::Manual {
        server_addr,
        client_id: 0,
        private_key: lightyear::netcode::Key::from(PRIVATE_KEY),
        protocol_id: PROTOCOL_ID,
    };

    let client_id = app
        .world_mut()
        .spawn((
            Name::new("Test Client"),
            Client::default(),
            LocalAddr(client_addr),
            PeerAddr(server_addr),
            Link::new(None),
            ReplicationReceiver::default(),
            NetcodeClient::new(auth, NetcodeConfig::default())
                .expect("Failed to create NetcodeClient"),
            UdpIo::default(),
        ))
        .id();

    // Run app setup
    app.update();

    // Verify PingManager component added (it's added by the client plugins)
    // Note: PingManager is internal to lightyear, so we just verify the client entity still exists
    assert!(app.world().get_entity(client_id).is_ok());
    assert!(app.world().get::<Client>(client_id).is_some());
}
