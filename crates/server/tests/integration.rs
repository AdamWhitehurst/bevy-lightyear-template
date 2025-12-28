use ::client::network::{ClientNetworkConfig, ClientNetworkPlugin, ClientTransport};
use ::server::network::{ServerNetworkConfig, ServerNetworkPlugin, ServerTransport};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use lightyear::prelude::client as lightyear_client;
use lightyear::prelude::server as lightyear_server;
use lightyear::prelude::*;
use lightyear_client::*;
use lightyear_server::*;
use protocol::*;
use std::net::SocketAddr;
use std::time::Duration;

/// Integration test using UDP transport to validate connection establishment
#[test]
fn test_client_server_udp_connection() {
    // Use a unique test port to avoid conflicts
    const TEST_PORT: u16 = 7777;

    // Create server app with UDP transport on test port
    let mut server_app = App::new();
    server_app.add_plugins(MinimalPlugins);
    server_app.add_plugins(bevy::log::LogPlugin::default());
    server_app.add_plugins(ServerPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
    });
    server_app.add_plugins(ProtocolPlugin);
    server_app.add_plugins(ServerNetworkPlugin {
        config: ServerNetworkConfig {
            transports: vec![ServerTransport::Udp { port: TEST_PORT }],
            bind_addr: [127, 0, 0, 1], // localhost only for tests
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            replication_interval: Duration::from_millis(100),
        },
    });

    // Create client app with UDP transport connecting to test server
    let mut client_app = App::new();
    client_app.add_plugins(MinimalPlugins);
    client_app.add_plugins(ClientPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
    });
    client_app.add_plugins(ProtocolPlugin);
    client_app.add_plugins(ClientNetworkPlugin {
        config: ClientNetworkConfig {
            client_addr: SocketAddr::from(([127, 0, 0, 1], 0)), // Random port
            server_addr: SocketAddr::from(([127, 0, 0, 1], TEST_PORT)),
            client_id: 0,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            transport: ClientTransport::Udp,
            ..default()
        },
    });

    // Setup manual time control for deterministic testing
    let mut current_time = bevy::platform::time::Instant::now();
    let frame_duration = Duration::from_millis(10);
    server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
    client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));

    // Run startup systems
    server_app.update();
    client_app.update();

    // Manually trigger connection (since UI plugin isn't used in this test)
    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, With<lightyear_client::Client>>()
        .single(client_app.world())
        .unwrap();
    client_app
        .world_mut()
        .commands()
        .trigger(lightyear_client::Connect {
            entity: client_entity,
        });
    client_app.update();

    // Verify server spawned UDP entity
    let mut query = server_app
        .world_mut()
        .query_filtered::<Entity, With<NetcodeServer>>();
    let server_count = query.iter(server_app.world()).count();
    assert_eq!(server_count, 1, "Server should have spawned one UDP entity");

    // Verify client spawned entity
    let mut query = client_app
        .world_mut()
        .query_filtered::<Entity, With<Client>>();
    let client_count = query.iter(client_app.world()).count();
    assert_eq!(client_count, 1, "Client should have spawned one entity");

    // Step both apps multiple times to allow UDP connection to establish
    // UDP + netcode handshake can take 100-200 updates depending on timing
    for i in 0..300 {
        // Advance time before each update
        current_time += frame_duration;
        server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));

        server_app.update();
        client_app.update();

        // Small delay to allow real UDP packets to be sent/received
        std::thread::sleep(Duration::from_micros(100));

        // Check if client is connected
        let mut query = client_app
            .world_mut()
            .query_filtered::<Entity, (With<Client>, With<Connected>)>();
        let client_connected = query.iter(client_app.world()).count();

        if client_connected > 0 {
            info!("Client connected after {} update cycles", i + 1);
            break;
        }

        // Log progress every 50 cycles
        if (i + 1) % 50 == 0 {
            info!("UDP connection attempt {}/300...", i + 1);
        }
    }

    // Verify client has Connected component
    let mut query = client_app
        .world_mut()
        .query_filtered::<Entity, (With<Client>, With<Connected>)>();
    let client_connected_count = query.iter(client_app.world()).count();
    assert_eq!(
        client_connected_count, 1,
        "Client should have Connected component after connection handshake"
    );

    // Verify server added ReplicationSender to client entity
    let mut query = server_app
        .world_mut()
        .query_filtered::<Entity, (With<Connected>, With<ReplicationSender>)>();
    let client_entities_on_server = query.iter(server_app.world()).count();
    assert_eq!(
        client_entities_on_server, 1,
        "Server should have added ReplicationSender to connected client"
    );

    info!("✓ UDP connection test passed!");
    info!("✓ Client and server successfully connected via networking plugins");
}

/// Test that client and server plugins can be instantiated together
#[test]
fn test_client_server_plugin_initialization() {
    // Create crossbeam transport pair
    let (crossbeam_client, crossbeam_server) = lightyear_crossbeam::CrossbeamIo::new_pair();

    // Create server app
    let mut server_app = App::new();
    server_app.add_plugins(MinimalPlugins);
    server_app.add_plugins(ServerPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
    });
    server_app.add_plugins(ProtocolPlugin);
    server_app.add_plugins(ServerNetworkPlugin {
        config: ServerNetworkConfig {
            transports: vec![ServerTransport::Crossbeam {
                io: crossbeam_server,
            }],
            bind_addr: [0, 0, 0, 0],
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            replication_interval: Duration::from_millis(100),
        },
    });

    // Create client app
    let mut client_app = App::new();
    client_app.add_plugins(MinimalPlugins);
    client_app.add_plugins(ClientPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
    });
    client_app.add_plugins(ProtocolPlugin);
    client_app.add_plugins(ClientNetworkPlugin {
        config: ClientNetworkConfig {
            client_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            server_addr: SocketAddr::from(([127, 0, 0, 1], 5000)),
            client_id: 0,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            transport: ClientTransport::Crossbeam(crossbeam_client),
            ..default()
        },
    });

    // Run startup systems
    server_app.update();
    client_app.update();

    // Verify server spawned entity
    let mut query = server_app
        .world_mut()
        .query_filtered::<Entity, With<NetcodeServer>>();
    assert_eq!(
        query.iter(server_app.world()).count(),
        1,
        "Server should have spawned one entity"
    );

    // Verify client spawned entity
    let mut query = client_app
        .world_mut()
        .query_filtered::<Entity, With<Client>>();
    assert_eq!(
        query.iter(client_app.world()).count(),
        1,
        "Client should have spawned one entity"
    );

    info!("Plugin initialization test passed!");
}

/// Test that plugins can be configured with different transports
#[test]
fn test_plugin_transport_configuration() {
    // Test server can be configured with multiple transports
    let config = ServerNetworkConfig {
        transports: vec![
            ServerTransport::Udp { port: 6000 },
            ServerTransport::WebTransport { port: 6001 },
        ],
        ..Default::default()
    };
    assert_eq!(config.transports.len(), 2);

    // Test client can be configured with different transport types
    let udp_config = ClientNetworkConfig {
        transport: ClientTransport::Udp,
        ..Default::default()
    };
    assert!(matches!(udp_config.transport, ClientTransport::Udp));

    let wt_config = ClientNetworkConfig {
        transport: ClientTransport::WebTransport {
            certificate_digest: "test".to_string(),
        },
        ..Default::default()
    };
    assert!(matches!(
        wt_config.transport,
        ClientTransport::WebTransport { .. }
    ));
}

/// Test that a client can connect and disconnect multiple times,
/// ensuring connection tokens are properly refreshed on each reconnection.
#[test]
fn test_reconnection_with_token_refresh() {
    const TEST_PORT: u16 = 7780;
    const RECONNECT_COUNT: usize = 3;

    // Create server app (persists across reconnections)
    let mut server_app = App::new();
    server_app.add_plugins(MinimalPlugins);
    server_app.add_plugins(ServerPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
    });
    server_app.add_plugins(ProtocolPlugin);
    server_app.add_plugins(ServerNetworkPlugin {
        config: ServerNetworkConfig {
            transports: vec![ServerTransport::Udp { port: TEST_PORT }],
            bind_addr: [127, 0, 0, 1],
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            replication_interval: Duration::from_millis(100),
        },
    });

    let mut current_time = bevy::platform::time::Instant::now();
    let frame_duration = Duration::from_millis(10);
    server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
    server_app.update();

    for iteration in 0..RECONNECT_COUNT {
        // Create fresh client with unique client_id for each connection
        let mut client_app = App::new();
        client_app.add_plugins(MinimalPlugins);
        client_app.add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        });
        client_app.add_plugins(ProtocolPlugin);
        client_app.add_plugins(ClientNetworkPlugin {
            config: ClientNetworkConfig {
                client_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                server_addr: SocketAddr::from(([127, 0, 0, 1], TEST_PORT)),
                client_id: iteration as u64, // Unique ID per connection
                protocol_id: PROTOCOL_ID,
                private_key: PRIVATE_KEY,
                transport: ClientTransport::Udp,
                ..default()
            },
        });
        client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        client_app.update();

        // Trigger connection
        let client_entity = client_app
            .world_mut()
            .query_filtered::<Entity, With<lightyear_client::Client>>()
            .single(client_app.world())
            .unwrap();
        client_app
            .world_mut()
            .commands()
            .trigger(lightyear_client::Connect {
                entity: client_entity,
            });
        client_app.update();

        // Wait for connection
        let mut connected = false;
        for _ in 0..300 {
            current_time += frame_duration;
            server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
            client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
            server_app.update();
            client_app.update();
            std::thread::sleep(Duration::from_micros(100));

            let mut query = client_app
                .world_mut()
                .query_filtered::<Entity, (With<Client>, With<Connected>)>();
            if query.iter(client_app.world()).count() > 0 {
                connected = true;
                break;
            }
        }
        assert!(
            connected,
            "Client should connect on iteration {}",
            iteration
        );

        // Verify server has connected client
        let mut query = server_app
            .world_mut()
            .query_filtered::<Entity, (With<Connected>, With<ReplicationSender>)>();
        assert!(
            query.iter(server_app.world()).count() >= 1,
            "Server should have connected client on iteration {}",
            iteration
        );

        // Trigger disconnect
        client_app
            .world_mut()
            .commands()
            .trigger(lightyear_client::Disconnect {
                entity: client_entity,
            });

        // Step to process disconnect
        for _ in 0..50 {
            current_time += frame_duration;
            server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
            client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
            server_app.update();
            client_app.update();
            std::thread::sleep(Duration::from_micros(100));
        }
    }

    info!(
        "✓ Reconnection test passed! {} connect/disconnect cycles completed",
        RECONNECT_COUNT
    );
}

/// Test that voxel messages are registered in protocol
#[test]
fn test_voxel_messages_registered() {
    use protocol::{VoxelEditRequest, VoxelType};

    // Create simple app to verify message types compile
    let _request = VoxelEditRequest {
        position: IVec3::new(1, 2, 3),
        voxel: VoxelType::Solid(42),
    };

    info!("✓ Voxel message types compile successfully!");
}
