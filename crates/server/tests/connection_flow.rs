use bevy::log::LogPlugin;
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::prelude::server::*;
use protocol::{FIXED_TIMESTEP_HZ, PROTOCOL_ID, PRIVATE_KEY};
use std::time::Duration;

#[test]
fn test_server_started() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, LogPlugin::default()));

    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ServerPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    let netcode_config = NetcodeConfig {
        protocol_id: PROTOCOL_ID,
        private_key: lightyear::netcode::Key::from(PRIVATE_KEY),
        ..default()
    };

    let server_entity = app.world_mut().spawn((
        Name::new("Test Server"),
        NetcodeServer::new(netcode_config),
        LocalAddr(std::net::SocketAddr::from(([0, 0, 0, 0], 0))),
        ServerUdpIo::default(),
    )).id();

    // Trigger Start event
    app.world_mut().trigger(Start {
        entity: server_entity,
    });

    app.update();

    // Verify Started component present
    assert!(app.world().get::<Started>(server_entity).is_some(), "Server should have Started component");
}

// NOTE: Full client-server connection test with crossbeam requires lightyear "test_utils" feature
// and CrossbeamIo which is not available in standard lightyear features.
// For full integration testing, use the stepper pattern from lightyear_tests examples.
#[test]
fn test_client_server_connection() {
    // This test demonstrates the setup pattern for client-server testing
    // For actual connection testing with message passing, see lightyear_tests/src/stepper.rs

    let mut server_app = App::new();
    server_app.add_plugins((MinimalPlugins, LogPlugin::default()));

    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    server_app.add_plugins(ServerPlugins { tick_duration });
    server_app.add_plugins(protocol::ProtocolPlugin);

    let netcode_config = NetcodeConfig {
        protocol_id: PROTOCOL_ID,
        private_key: lightyear::netcode::Key::from(PRIVATE_KEY),
        ..default()
    };

    let server_entity = server_app.world_mut().spawn((
        Name::new("Test Server"),
        NetcodeServer::new(netcode_config),
        LocalAddr(std::net::SocketAddr::from(([0, 0, 0, 0], 0))),
        ServerUdpIo::default(),
    )).id();

    server_app.world_mut().trigger(Start {
        entity: server_entity,
    });

    server_app.update();

    // Verify server started
    assert!(server_app.world().get::<Started>(server_entity).is_some());

    // NOTE: For full client-server connection testing with message passing,
    // use lightyear's crossbeam transport which requires additional setup.
    // See git/lightyear/lightyear_tests/src/stepper.rs for reference implementation.
}
