use ::server::network::{ServerNetworkConfig, ServerNetworkPlugin, ServerTransport};
use bevy::prelude::*;
use lightyear::prelude::server::*;
use protocol::*;

#[test]
fn test_server_network_plugin_spawns_all_transports() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::log::LogPlugin::default());
    app.add_plugins(ServerPlugins::default());
    app.add_plugins(ProtocolPlugin);

    // Add plugin with custom ports to avoid conflicts
    let config = ServerNetworkConfig {
        transports: vec![
            ServerTransport::Udp { port: 7000 },
            ServerTransport::WebTransport { port: 7001 },
            ServerTransport::WebSocket { port: 7002 },
        ],
        ..Default::default()
    };
    app.add_plugins(ServerNetworkPlugin { config });

    // Run startup systems
    app.update();

    // Verify UDP server entity
    let mut udp_query = app
        .world_mut()
        .query_filtered::<Entity, (With<NetcodeServer>, With<ServerUdpIo>)>();
    assert_eq!(
        udp_query.iter(app.world()).count(),
        1,
        "Should have one UDP server"
    );

    // Verify WebTransport server entity
    let mut wt_query = app
        .world_mut()
        .query_filtered::<Entity, (With<NetcodeServer>, With<WebTransportServerIo>)>();
    assert_eq!(
        wt_query.iter(app.world()).count(),
        1,
        "Should have one WebTransport server"
    );

    // Verify WebSocket server entity
    let mut ws_query = app
        .world_mut()
        .query_filtered::<Entity, (With<NetcodeServer>, With<WebSocketServerIo>)>();
    assert_eq!(
        ws_query.iter(app.world()).count(),
        1,
        "Should have one WebSocket server"
    );
}

#[test]
fn test_server_network_plugin_config_is_resource() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ServerPlugins::default());
    app.add_plugins(ProtocolPlugin);

    // Use custom ports to avoid conflicts
    let config = ServerNetworkConfig {
        transports: vec![ServerTransport::Udp { port: 7100 }],
        ..Default::default()
    };
    app.add_plugins(ServerNetworkPlugin { config });

    app.update();

    // Verify config was inserted as resource
    assert!(app.world().contains_resource::<ServerNetworkConfig>());
}

#[test]
fn test_server_network_plugin_observer_registration() {
    // This test verifies the plugin registers observers without attempting
    // to simulate a full client connection (which would require extensive setup)
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ServerPlugins::default());
    app.add_plugins(ProtocolPlugin);

    // Use custom ports to avoid conflicts
    let config = ServerNetworkConfig {
        transports: vec![ServerTransport::Udp { port: 7200 }],
        ..Default::default()
    };
    app.add_plugins(ServerNetworkPlugin { config });

    // Run startup - if observer registration is broken, this would panic
    app.update();

    // Verify the plugin was added successfully and server entities exist
    let mut server_query = app
        .world_mut()
        .query_filtered::<Entity, With<NetcodeServer>>();
    assert_eq!(
        server_query.iter(app.world()).count(),
        1,
        "Server should have spawned"
    );
}

#[test]
fn test_server_network_plugin_single_transport() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::log::LogPlugin::default());
    app.add_plugins(ServerPlugins::default());
    app.add_plugins(ProtocolPlugin);

    // Add plugin with only UDP transport
    let config = ServerNetworkConfig {
        transports: vec![ServerTransport::Udp { port: 6000 }],
        ..Default::default()
    };
    app.add_plugins(ServerNetworkPlugin { config });

    // Run startup systems
    app.update();

    // Verify only UDP server entity exists
    let mut udp_query = app
        .world_mut()
        .query_filtered::<Entity, (With<NetcodeServer>, With<ServerUdpIo>)>();
    assert_eq!(
        udp_query.iter(app.world()).count(),
        1,
        "Should have one UDP server"
    );

    // Verify no WebTransport server
    let mut wt_query = app
        .world_mut()
        .query_filtered::<Entity, (With<NetcodeServer>, With<WebTransportServerIo>)>();
    assert_eq!(
        wt_query.iter(app.world()).count(),
        0,
        "Should have no WebTransport server"
    );

    // Verify no WebSocket server
    let mut ws_query = app
        .world_mut()
        .query_filtered::<Entity, (With<NetcodeServer>, With<WebSocketServerIo>)>();
    assert_eq!(
        ws_query.iter(app.world()).count(),
        0,
        "Should have no WebSocket server"
    );
}
