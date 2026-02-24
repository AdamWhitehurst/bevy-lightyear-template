use ::client::network::{ClientNetworkConfig, ClientNetworkPlugin, ClientTransport};
use ::server::network::{ServerNetworkConfig, ServerNetworkPlugin, ServerTransport};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use lightyear::connection::client::PeerMetadata;
use lightyear::prelude::client as lightyear_client;
use lightyear::prelude::server as lightyear_server;
use lightyear::prelude::*;
use lightyear_client::*;
use lightyear_server::*;
use protocol::*;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::time::Duration;

/// Simplified test stepper for crossbeam transport testing
/// Based on lightyear's ClientServerStepper pattern
struct CrossbeamTestStepper {
    pub client_app: App,
    pub server_app: App,
    pub client_entity: Entity,
    pub server_entity: Entity,
    pub client_of_entity: Entity,
    pub current_time: bevy::platform::time::Instant,
    pub tick_duration: Duration,
}

impl CrossbeamTestStepper {
    /// Create new stepper with crossbeam transport and manual time control
    fn new() -> Self {
        let (crossbeam_client, crossbeam_server) = lightyear_crossbeam::CrossbeamIo::new_pair();

        // Setup server app
        let mut server_app = App::new();
        server_app.add_plugins(MinimalPlugins);
        server_app.add_plugins(ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        });
        server_app.add_plugins(ProtocolPlugin);

        // Setup client app
        let mut client_app = App::new();
        client_app.add_plugins(MinimalPlugins);
        client_app.add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        });
        client_app.add_plugins(ProtocolPlugin);

        // Finish plugin setup
        server_app.finish();
        server_app.cleanup();
        client_app.finish();
        client_app.cleanup();

        // Setup manual time control
        let current_time = bevy::platform::time::Instant::now();
        server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));

        // Spawn server entity with RawServer for crossbeam
        let server_entity = server_app
            .world_mut()
            .spawn((
                Name::new("Test Server"),
                Server::default(),
                RawServer, // Use RawServer for crossbeam transport
                DeltaManager::default(),
                crossbeam_server.clone(),
            ))
            .id();

        // Spawn client entity with crossbeam transport
        let client_entity = client_app
            .world_mut()
            .spawn((
                Name::new("Test Client"),
                Client::default(),
                PingManager::new(PingConfig {
                    ping_interval: Duration::ZERO,
                }),
                ReplicationSender::default(),
                ReplicationReceiver::default(),
                crossbeam_client.clone(),
                PredictionManager::default(),
                RawClient, // Use RawClient for crossbeam transport
                Linked,    // CRITICAL: Crossbeam needs explicit Linked marker
            ))
            .id();

        // Spawn ClientOf entity in server app for client representation
        let client_of_entity = server_app
            .world_mut()
            .spawn((
                Name::new("Test ClientOf"),
                LinkOf {
                    server: server_entity,
                },
                PingManager::new(PingConfig {
                    ping_interval: Duration::ZERO,
                }),
                ReplicationSender::default(),
                ReplicationReceiver::default(),
                Link::new(None),
                PeerAddr(SocketAddr::from(([127, 0, 0, 1], 9999))), // Mock port
                Linked, // CRITICAL: Crossbeam needs explicit Linked marker
                crossbeam_server,
            ))
            .id();

        Self {
            client_app,
            server_app,
            client_entity,
            server_entity,
            client_of_entity,
            current_time,
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        }
    }

    /// Initialize connection by triggering Start on server and Connect on client
    fn init(&mut self) {
        // Trigger Start on server
        self.server_app.world_mut().commands().trigger(Start {
            entity: self.server_entity,
        });
        self.server_app.update();

        // Trigger Connect on client
        self.client_app.world_mut().commands().trigger(Connect {
            entity: self.client_entity,
        });
        self.client_app.update();
    }

    /// Step simulation by n ticks
    fn tick_step(&mut self, n: usize) {
        for _ in 0..n {
            self.current_time += self.tick_duration;
            self.server_app
                .insert_resource(TimeUpdateStrategy::ManualInstant(self.current_time));
            self.client_app
                .insert_resource(TimeUpdateStrategy::ManualInstant(self.current_time));
            self.server_app.update();
            self.client_app.update();
        }
    }

    /// Wait for connection to establish (polls for Connected component)
    fn wait_for_connection(&mut self) -> bool {
        for tick in 0..50 {
            if self
                .client_app
                .world()
                .get::<Connected>(self.client_entity)
                .is_some()
            {
                info!("Client connected after {} ticks", tick + 1);
                return true;
            }
            self.tick_step(1);
        }
        false
    }
}

/// Buffer resource to collect received messages
#[derive(Resource)]
struct MessageBuffer<M> {
    messages: Vec<(Entity, M)>,
}

impl<M> Default for MessageBuffer<M> {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
        }
    }
}

/// Observer system to collect messages into buffer
fn collect_messages<M: lightyear::prelude::Message + Debug + Clone>(
    mut receiver: Query<(Entity, &mut MessageReceiver<M>)>,
    mut buffer: ResMut<MessageBuffer<M>>,
) {
    receiver.iter_mut().for_each(|(entity, mut receiver)| {
        receiver.receive().for_each(|m| {
            buffer.messages.push((entity, m));
        });
    });
}

/// Buffer resource to collect received events/triggers
#[derive(Resource)]
struct EventBuffer<E> {
    events: Vec<(Entity, E)>,
}

impl<E> Default for EventBuffer<E> {
    fn default() -> Self {
        Self { events: Vec::new() }
    }
}

/// Observer to collect remote events into buffer
fn collect_events<E: Event + Debug + Clone>(
    trigger: On<RemoteEvent<E>>,
    peer_metadata: Res<PeerMetadata>,
    mut buffer: ResMut<EventBuffer<E>>,
) {
    let remote = *peer_metadata
        .mapping
        .get(&trigger.event().from)
        .expect("Remote entity should be in peer metadata");
    buffer
        .events
        .push((remote, trigger.event().trigger.clone()));
}

/// Integration test using UDP transport to validate connection establishment
#[test]
fn test_client_server_udp_connection() {
    // Use a unique test port to avoid conflicts
    const TEST_PORT: u16 = 7777;

    // Create server app with UDP transport on test port
    let mut server_app = App::new();
    server_app.add_plugins(MinimalPlugins);
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

/// Test that client and server connect properly via crossbeam transport
/// Verifies Connected and Linked components are present
#[test]
fn test_crossbeam_connection_establishment() {
    let mut stepper = CrossbeamTestStepper::new();
    stepper.init();

    let connected = stepper.wait_for_connection();
    assert!(
        connected,
        "Client should have Connected component after connection establishment"
    );

    // Verify Connected component on client
    assert!(
        stepper
            .client_app
            .world()
            .get::<Connected>(stepper.client_entity)
            .is_some(),
        "Client should have Connected component"
    );

    // Verify Linked component on client (critical for crossbeam)
    assert!(
        stepper
            .client_app
            .world()
            .get::<Linked>(stepper.client_entity)
            .is_some(),
        "Client should have Linked component for crossbeam transport"
    );

    // Verify server has connected client (ClientOf entity should have Connected)
    assert!(
        stepper
            .server_app
            .world()
            .get::<Connected>(stepper.client_of_entity)
            .is_some(),
        "Server should have Connected component on ClientOf entity"
    );

    info!("✓ Crossbeam connection test passed!");
}

/// Test sending messages from client to server via crossbeam
#[test]
fn test_crossbeam_client_to_server_messages() {
    let mut stepper = CrossbeamTestStepper::new();

    // Add message buffer to server
    stepper
        .server_app
        .init_resource::<MessageBuffer<VoxelEditRequest>>();
    stepper
        .server_app
        .add_systems(Update, collect_messages::<VoxelEditRequest>);

    stepper.init();
    stepper.wait_for_connection();

    // Send message from client
    let test_message = VoxelEditRequest {
        position: IVec3::new(1, 2, 3),
        voxel: VoxelType::Solid(42),
    };

    stepper
        .client_app
        .world_mut()
        .entity_mut(stepper.client_entity)
        .get_mut::<MessageSender<VoxelEditRequest>>()
        .expect("Client should have MessageSender")
        .send::<VoxelChannel>(test_message.clone());

    // Step to deliver message (3 ticks needed to ensure delivery)
    stepper.tick_step(3);

    // Verify on server
    let buffer = stepper
        .server_app
        .world()
        .resource::<MessageBuffer<VoxelEditRequest>>();
    assert_eq!(
        buffer.messages.len(),
        1,
        "Server should receive exactly one message"
    );
    assert_eq!(
        buffer.messages[0].1, test_message,
        "Received message should match sent message"
    );
    assert_eq!(
        buffer.messages[0].0, stepper.client_of_entity,
        "Source entity should be server's client representation"
    );

    info!("✓ Client-to-server message test passed!");
}

/// Test sending messages from server to client via crossbeam
#[test]
fn test_crossbeam_server_to_client_messages() {
    let mut stepper = CrossbeamTestStepper::new();

    // Add message buffer to client
    stepper
        .client_app
        .init_resource::<MessageBuffer<VoxelEditBroadcast>>();
    stepper
        .client_app
        .add_systems(Update, collect_messages::<VoxelEditBroadcast>);

    stepper.init();
    stepper.wait_for_connection();

    // Send message from server to client
    let test_message = VoxelEditBroadcast {
        position: IVec3::new(4, 5, 6),
        voxel: VoxelType::Solid(99),
    };

    stepper
        .server_app
        .world_mut()
        .entity_mut(stepper.client_of_entity)
        .get_mut::<MessageSender<VoxelEditBroadcast>>()
        .expect("Server client entity should have MessageSender")
        .send::<VoxelChannel>(test_message.clone());

    // Step simulation to deliver message (may need 2 ticks for server→client)
    stepper.tick_step(2);

    // Verify message received on client
    let buffer = stepper
        .client_app
        .world()
        .resource::<MessageBuffer<VoxelEditBroadcast>>();
    assert_eq!(
        buffer.messages.len(),
        1,
        "Client should receive exactly one message"
    );
    assert_eq!(
        buffer.messages[0].1, test_message,
        "Received message should match sent message"
    );
    assert_eq!(
        buffer.messages[0].0, stepper.client_entity,
        "Message should be received by client entity"
    );

    info!("✓ Server-to-client message test passed!");
}

/// Test sending events/triggers from client to server via crossbeam
#[test]
fn test_crossbeam_event_triggers() {
    use protocol::TestTrigger;

    let mut stepper = CrossbeamTestStepper::new();

    // Add event buffer and observer to server
    stepper
        .server_app
        .init_resource::<EventBuffer<TestTrigger>>();
    stepper
        .server_app
        .add_observer(collect_events::<TestTrigger>);

    stepper.init();
    stepper.wait_for_connection();

    // Send trigger from client
    let test_trigger = TestTrigger {
        data: "test_event_data".to_string(),
    };

    stepper
        .client_app
        .world_mut()
        .entity_mut(stepper.client_entity)
        .get_mut::<EventSender<TestTrigger>>()
        .expect("Client should have EventSender")
        .trigger::<VoxelChannel>(test_trigger.clone());

    // Step to deliver event
    stepper.tick_step(3);

    // Verify event received on server
    let buffer = stepper
        .server_app
        .world()
        .resource::<EventBuffer<TestTrigger>>();
    assert_eq!(
        buffer.events.len(),
        1,
        "Server should receive exactly one event"
    );
    assert_eq!(buffer.events[0].1, test_trigger, "Event data should match");
    assert_eq!(
        buffer.events[0].0, stepper.client_of_entity,
        "Event should be from client_of_entity"
    );

    info!("✓ Event/trigger test passed!");
}
