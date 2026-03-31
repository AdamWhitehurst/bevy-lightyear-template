use ::client::map::handle_map_transition_start;
use ::client::network::{ClientNetworkConfig, ClientNetworkPlugin, ClientTransport};
use ::server::map::{
    flush_voxel_broadcasts, handle_map_switch_requests, handle_map_transition_ready,
    handle_voxel_edit_requests, PendingVoxelBroadcasts, RoomRegistry, WorldDirtyState,
};
use ::server::network::{ServerNetworkConfig, ServerNetworkPlugin, ServerTransport};
use ::server::persistence::WorldSavePath;
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimeUpdateStrategy;
use lightyear::connection::client::PeerMetadata;
use lightyear::prelude::client as lightyear_client;
use lightyear::prelude::server as lightyear_server;
use lightyear::prelude::*;
use lightyear_client::*;
use lightyear_server::*;
use protocol::map::{MapChannel, MapSwitchTarget, MapTransitionStart, PlayerMapSwitchRequest};
use protocol::*;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use ui::{ClientState, MapTransitionState};
use voxel_map_engine::prelude::{
    flat_terrain_voxels, VoxelGenerator, VoxelMapConfig, VoxelMapInstance, VoxelPlugin,
};

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
        server_app.add_plugins(lightyear::prelude::RoomPlugin);

        // Setup client app
        let mut client_app = App::new();
        client_app.add_plugins(MinimalPlugins);
        client_app.add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        });
        client_app.add_plugins(ProtocolPlugin);

        // Setup manual time control (finish/cleanup deferred to init() so tests
        // can add plugins between new() and init())
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

    /// Finalize plugin setup and initialize connection.
    fn init(&mut self) {
        self.server_app.finish();
        self.server_app.cleanup();
        self.client_app.finish();
        self.client_app.cleanup();

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
    const TEST_PORT: u16 = 7777;

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
            client_id: 0,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            transport: ClientTransport::Udp,
            ..default()
        },
    });

    let mut current_time = bevy::platform::time::Instant::now();
    let frame_duration = Duration::from_millis(10);
    server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
    client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));

    server_app.update();
    client_app.update();

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

    let mut query = server_app
        .world_mut()
        .query_filtered::<Entity, With<NetcodeServer>>();
    assert_eq!(
        query.iter(server_app.world()).count(),
        1,
        "Server should have spawned one UDP entity"
    );

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
        "Client should have Connected component after UDP handshake"
    );

    let mut query = server_app
        .world_mut()
        .query_filtered::<Entity, (With<Connected>, With<ReplicationSender>)>();
    assert_eq!(
        query.iter(server_app.world()).count(),
        1,
        "Server should have added ReplicationSender to connected client"
    );
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
            server_addr: SocketAddr::from(([127, 0, 0, 1], 5560)),
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
            ServerTransport::Udp { port: 6600 },
            ServerTransport::WebTransport { port: 6601 },
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

/// Test that a client can disconnect and reconnect multiple times to the same
/// persistent server. Each cycle creates a fresh crossbeam channel pair and
/// client app while the server app (and its state) persists across reconnections.
#[test]
fn test_crossbeam_reconnection() {
    const RECONNECT_COUNT: usize = 3;

    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    let mut current_time = bevy::platform::time::Instant::now();

    // Persistent server app — survives across all reconnection cycles
    let mut server_app = App::new();
    server_app.add_plugins(MinimalPlugins);
    server_app.add_plugins(ServerPlugins { tick_duration });
    server_app.add_plugins(ProtocolPlugin);
    server_app.add_plugins(lightyear::prelude::RoomPlugin);
    server_app.finish();
    server_app.cleanup();
    server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));

    for iteration in 0..RECONNECT_COUNT {
        // Fresh crossbeam channel pair for this connection
        let (crossbeam_client, crossbeam_server) = lightyear_crossbeam::CrossbeamIo::new_pair();

        // Spawn server + client_of entities for this connection
        let server_entity = server_app
            .world_mut()
            .spawn((
                Name::new(format!("Server {iteration}")),
                Server::default(),
                RawServer,
                DeltaManager::default(),
                crossbeam_server.clone(),
            ))
            .id();

        let client_of_entity = server_app
            .world_mut()
            .spawn((
                Name::new(format!("ClientOf {iteration}")),
                LinkOf {
                    server: server_entity,
                },
                PingManager::new(PingConfig {
                    ping_interval: Duration::ZERO,
                }),
                ReplicationSender::default(),
                ReplicationReceiver::default(),
                Link::new(None),
                PeerAddr(SocketAddr::from(([127, 0, 0, 1], 9990 + iteration as u16))),
                Linked,
                crossbeam_server,
            ))
            .id();

        // Trigger Start on server
        server_app.world_mut().commands().trigger(Start {
            entity: server_entity,
        });
        server_app.update();

        // Fresh client app for this connection
        let mut client_app = App::new();
        client_app.add_plugins(MinimalPlugins);
        client_app.add_plugins(ClientPlugins { tick_duration });
        client_app.add_plugins(ProtocolPlugin);
        client_app.finish();
        client_app.cleanup();
        client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));

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
                crossbeam_client,
                PredictionManager::default(),
                RawClient,
                Linked,
            ))
            .id();

        // Trigger Connect on client
        client_app.world_mut().commands().trigger(Connect {
            entity: client_entity,
        });
        client_app.update();

        // Wait for connection
        let mut connected = false;
        for _ in 0..50 {
            current_time += tick_duration;
            server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
            client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
            server_app.update();
            client_app.update();

            if client_app.world().get::<Connected>(client_entity).is_some() {
                connected = true;
                break;
            }
        }

        assert!(connected, "Client should connect on iteration {iteration}");
        assert!(
            server_app
                .world()
                .get::<Connected>(client_of_entity)
                .is_some(),
            "Server should have Connected on ClientOf entity on iteration {iteration}"
        );

        // Simulate disconnect: drop client app, then clean up server-side entities
        // before stepping to avoid sending on disconnected crossbeam channels
        drop(client_app);
        server_app.world_mut().despawn(client_of_entity);
        server_app.world_mut().despawn(server_entity);

        // Step server to flush despawns and process cleanup
        for _ in 0..10 {
            current_time += tick_duration;
            server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
            server_app.update();
        }
    }
}

/// Test that voxel messages are registered in protocol
#[test]
fn test_voxel_messages_registered() {
    use protocol::{VoxelEditRequest, VoxelType};

    // Create simple app to verify message types compile
    let _request = VoxelEditRequest {
        position: IVec3::new(1, 2, 3),
        voxel: VoxelType::Solid(42),
        sequence: 0,
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
        sequence: 0,
    };

    stepper
        .client_app
        .world_mut()
        .entity_mut(stepper.client_entity)
        .get_mut::<MessageSender<VoxelEditRequest>>()
        .expect("Client should have MessageSender")
        .send::<VoxelChannel>(test_message.clone());

    // Poll until server receives the message
    let mut received = false;
    for _ in 0..30 {
        stepper.tick_step(1);
        if !stepper
            .server_app
            .world()
            .resource::<MessageBuffer<VoxelEditRequest>>()
            .messages
            .is_empty()
        {
            received = true;
            break;
        }
    }
    assert!(received, "Server should receive the message");

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

    // Poll until client receives the message
    let mut received = false;
    for _ in 0..30 {
        stepper.tick_step(1);
        if !stepper
            .client_app
            .world()
            .resource::<MessageBuffer<VoxelEditBroadcast>>()
            .messages
            .is_empty()
        {
            received = true;
            break;
        }
    }
    assert!(received, "Client should receive the message");

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

    // Poll until server receives the event
    let mut received = false;
    for _ in 0..30 {
        stepper.tick_step(1);
        if !stepper
            .server_app
            .world()
            .resource::<EventBuffer<TestTrigger>>()
            .events
            .is_empty()
        {
            received = true;
            break;
        }
    }
    assert!(received, "Server should receive the event");

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

fn add_server_map_systems(stepper: &mut CrossbeamTestStepper) {
    stepper.server_app.init_resource::<MapRegistry>();
    stepper.server_app.init_resource::<RoomRegistry>();
    stepper.server_app.init_resource::<WorldSavePath>();
    stepper.server_app.add_systems(
        Update,
        (handle_map_switch_requests, handle_map_transition_ready),
    );
}

fn register_overworld_on_server(stepper: &mut CrossbeamTestStepper) -> Entity {
    let map = stepper
        .server_app
        .world_mut()
        .spawn((
            VoxelMapInstance::new(3),
            VoxelMapConfig::new(0, 0, 1, None, 3),
            VoxelGenerator(Arc::new(flat_terrain_voxels)),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();
    stepper
        .server_app
        .world_mut()
        .resource_mut::<MapRegistry>()
        .insert(MapInstanceId::Overworld, map);
    map
}

fn spawn_server_character(stepper: &mut CrossbeamTestStepper, client_of_entity: Entity) -> Entity {
    stepper
        .server_app
        .world_mut()
        .spawn((
            CharacterMarker,
            MapInstanceId::Overworld,
            ControlledBy {
                owner: client_of_entity,
                lifetime: Default::default(),
            },
        ))
        .id()
}

/// Verify the C2S→S2C map switch roundtrip: client sends `PlayerMapSwitchRequest`,
/// server processes it, client receives `MapTransitionStart`.
#[test]
fn map_switch_request_triggers_transition_start() {
    let mut stepper = CrossbeamTestStepper::new();

    // Add map systems and resources before plugin init completes
    add_server_map_systems(&mut stepper);
    stepper
        .client_app
        .init_resource::<MessageBuffer<MapTransitionStart>>();
    stepper
        .client_app
        .add_systems(Update, collect_messages::<MapTransitionStart>);

    stepper.init();
    stepper.wait_for_connection();

    let overworld_map = register_overworld_on_server(&mut stepper);

    // Add RemoteId to client_of_entity so handle_map_switch_requests can resolve the target map id
    let client_of = stepper.client_of_entity;
    stepper
        .server_app
        .world_mut()
        .entity_mut(client_of)
        .insert(RemoteId(PeerId::Netcode(42)));

    let character = spawn_server_character(&mut stepper, client_of);

    // Add character to overworld room via RoomRegistry
    let overworld_room = stepper
        .server_app
        .world_mut()
        .resource_mut::<RoomRegistry>()
        .0
        .get(&MapInstanceId::Overworld)
        .copied();
    if let Some(room) = overworld_room {
        stepper.server_app.world_mut().trigger(RoomEvent {
            room,
            target: RoomTarget::AddEntity(character),
        });
    }

    // Give character a ChunkTicket pointing at the overworld map
    stepper.server_app.world_mut().entity_mut(character).insert(
        voxel_map_engine::prelude::ChunkTicket::map_transition(overworld_map),
    );

    // Client sends map switch request (ClientToServer direction)
    let client_entity = stepper.client_entity;
    stepper
        .client_app
        .world_mut()
        .entity_mut(client_entity)
        .get_mut::<MessageSender<PlayerMapSwitchRequest>>()
        .expect("client entity must have MessageSender<PlayerMapSwitchRequest>")
        .send::<MapChannel>(PlayerMapSwitchRequest {
            target: MapSwitchTarget::Homebase,
        });

    // Poll until character gets PendingTransition (message delivery is async via crossbeam ticks)
    let mut got_transitioning = false;
    for _ in 0..30 {
        stepper.tick_step(1);
        if stepper
            .server_app
            .world()
            .get::<PendingTransition>(character)
            .is_some()
        {
            got_transitioning = true;
            break;
        }
    }
    assert!(
        got_transitioning,
        "Character should have PendingTransition marker after request"
    );

    // Poll until client receives MapTransitionStart
    let mut got_message = false;
    for _ in 0..10 {
        stepper.tick_step(1);
        if stepper
            .client_app
            .world()
            .resource::<MessageBuffer<MapTransitionStart>>()
            .messages
            .len()
            >= 1
        {
            got_message = true;
            break;
        }
    }
    assert!(got_message, "Client should receive MapTransitionStart");

    let buffer = stepper
        .client_app
        .world()
        .resource::<MessageBuffer<MapTransitionStart>>();
    assert_eq!(
        buffer.messages.len(),
        1,
        "Client should receive exactly one MapTransitionStart"
    );
    assert!(
        matches!(buffer.messages[0].1.target, MapInstanceId::Homebase { .. }),
        "Transition target should be a Homebase"
    );
}

/// Verify the server ignores a second map switch request when the player is already transitioning.
#[test]
fn duplicate_switch_request_ignored() {
    let mut stepper = CrossbeamTestStepper::new();

    add_server_map_systems(&mut stepper);
    stepper
        .client_app
        .init_resource::<MessageBuffer<MapTransitionStart>>();
    stepper
        .client_app
        .add_systems(Update, collect_messages::<MapTransitionStart>);

    stepper.init();
    stepper.wait_for_connection();

    register_overworld_on_server(&mut stepper);

    let client_of = stepper.client_of_entity;
    stepper
        .server_app
        .world_mut()
        .entity_mut(client_of)
        .insert(RemoteId(PeerId::Netcode(42)));

    let client_entity = stepper.client_entity;
    let character = spawn_server_character(&mut stepper, client_of);

    // First request — should be processed (ClientToServer direction)
    stepper
        .client_app
        .world_mut()
        .entity_mut(client_entity)
        .get_mut::<MessageSender<PlayerMapSwitchRequest>>()
        .expect("client entity must have MessageSender<PlayerMapSwitchRequest>")
        .send::<MapChannel>(PlayerMapSwitchRequest {
            target: MapSwitchTarget::Homebase,
        });

    // Poll until PendingTransition is applied — ensures deferred commands are flushed
    // before sending the second request, so the guard check is reliable
    let mut transitioning = false;
    for _ in 0..30 {
        stepper.tick_step(1);
        if stepper
            .server_app
            .world()
            .get::<PendingTransition>(character)
            .is_some()
        {
            transitioning = true;
            break;
        }
    }
    assert!(
        transitioning,
        "Character must have PendingTransition before sending duplicate request"
    );

    // Second request while already transitioning — should be ignored
    stepper
        .client_app
        .world_mut()
        .entity_mut(client_entity)
        .get_mut::<MessageSender<PlayerMapSwitchRequest>>()
        .expect("client entity must have MessageSender<PlayerMapSwitchRequest>")
        .send::<MapChannel>(PlayerMapSwitchRequest {
            target: MapSwitchTarget::Homebase,
        });

    stepper.tick_step(10);

    let buffer = stepper
        .client_app
        .world()
        .resource::<MessageBuffer<MapTransitionStart>>();
    assert_eq!(
        buffer.messages.len(),
        1,
        "Client should receive only one MapTransitionStart; duplicate request must be ignored"
    );
}

/// Both the server and the client App spawn homebase map entities through their real systems.
/// Server: handle_map_switch_requests → ensure_map_exists → VoxelMapInstance::homebase()
/// Client: handle_map_transition_start → spawn_map_instance
/// Then verify both produce identical VoxelMapConfig (seed, bounds, tree_height).
#[test]
fn server_and_client_spawn_matching_homebase_configs() {
    const TEST_CLIENT_ID: u64 = 42;

    let (crossbeam_client, crossbeam_server) = lightyear_crossbeam::CrossbeamIo::new_pair();

    let mut server_app = App::new();
    server_app.add_plugins(MinimalPlugins);
    server_app.add_plugins(ServerPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
    });
    server_app.add_plugins(ProtocolPlugin);
    server_app.add_plugins(lightyear::prelude::RoomPlugin);
    server_app.init_resource::<MapRegistry>();
    server_app.init_resource::<RoomRegistry>();
    server_app.init_resource::<WorldSavePath>();
    server_app.add_systems(
        Update,
        (handle_map_switch_requests, handle_map_transition_ready),
    );
    server_app.finish();
    server_app.cleanup();

    let mut client_app = App::new();
    client_app.add_plugins(MinimalPlugins);
    client_app.add_plugins(StatesPlugin);
    client_app.add_plugins(ClientPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
    });
    client_app.add_plugins(ProtocolPlugin);
    client_app.insert_state(ClientState::InGame);
    client_app.add_sub_state::<MapTransitionState>();
    client_app.init_resource::<MapRegistry>();
    client_app.add_systems(Update, handle_map_transition_start);
    client_app.finish();
    client_app.cleanup();

    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    let mut current_time = bevy::platform::time::Instant::now();
    server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
    client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));

    let server_entity = server_app
        .world_mut()
        .spawn((
            Name::new("Test Server"),
            Server::default(),
            RawServer,
            DeltaManager::default(),
            crossbeam_server.clone(),
        ))
        .id();

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
            RawClient,
            Linked,
        ))
        .id();

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
            PeerAddr(SocketAddr::from(([127, 0, 0, 1], 9999))),
            Linked,
            crossbeam_server,
        ))
        .id();

    server_app.world_mut().commands().trigger(Start {
        entity: server_entity,
    });
    server_app.update();
    client_app.world_mut().commands().trigger(Connect {
        entity: client_entity,
    });
    client_app.update();

    for _ in 0..50 {
        current_time += tick_duration;
        server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        server_app.update();
        client_app.update();
        if client_app.world().get::<Connected>(client_entity).is_some() {
            break;
        }
    }
    assert!(
        client_app.world().get::<Connected>(client_entity).is_some(),
        "Client must connect"
    );

    let overworld = server_app
        .world_mut()
        .spawn((
            VoxelMapInstance::new(3),
            VoxelMapConfig::new(0, 0, 1, None, 3),
            VoxelGenerator(Arc::new(flat_terrain_voxels)),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();
    server_app
        .world_mut()
        .resource_mut::<MapRegistry>()
        .insert(MapInstanceId::Overworld, overworld);

    let character = server_app
        .world_mut()
        .spawn((
            CharacterMarker,
            MapInstanceId::Overworld,
            ControlledBy {
                owner: client_of_entity,
                lifetime: Default::default(),
            },
        ))
        .id();

    server_app
        .world_mut()
        .entity_mut(client_of_entity)
        .insert(RemoteId(PeerId::Netcode(TEST_CLIENT_ID)));

    // Predicted player required by handle_map_transition_start
    // Must include MapInstanceId to match the system's query
    client_app.world_mut().spawn((
        CharacterMarker,
        Predicted,
        Controlled,
        MapInstanceId::Overworld,
    ));

    client_app
        .world_mut()
        .entity_mut(client_entity)
        .get_mut::<MessageSender<PlayerMapSwitchRequest>>()
        .expect("client entity must have MessageSender<PlayerMapSwitchRequest>")
        .send::<MapChannel>(PlayerMapSwitchRequest {
            target: MapSwitchTarget::Homebase,
        });

    let _ = character; // used above for spawn; server systems find it via ControlledBy query

    for _ in 0..40 {
        current_time += tick_duration;
        server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
        server_app.update();
        client_app.update();

        let client_has_homebase = client_app
            .world()
            .resource::<MapRegistry>()
            .0
            .keys()
            .any(|id| matches!(id, MapInstanceId::Homebase { .. }));
        if client_has_homebase {
            break;
        }
    }

    let server_homebase_entity = server_app
        .world()
        .resource::<MapRegistry>()
        .0
        .iter()
        .find(|(id, _)| matches!(id, MapInstanceId::Homebase { .. }))
        .map(|(_, &e)| e)
        .expect("Server must have spawned homebase");

    let client_homebase_entity = client_app
        .world()
        .resource::<MapRegistry>()
        .0
        .iter()
        .find(|(id, _)| matches!(id, MapInstanceId::Homebase { .. }))
        .map(|(_, &e)| e)
        .expect("Client must have spawned homebase");

    let server_config = server_app
        .world()
        .get::<VoxelMapConfig>(server_homebase_entity)
        .expect("Server homebase must have VoxelMapConfig");
    let client_config = client_app
        .world()
        .get::<VoxelMapConfig>(client_homebase_entity)
        .expect("Client homebase must have VoxelMapConfig");

    assert_eq!(server_config.seed, client_config.seed, "seed must match");
    assert_eq!(
        server_config.bounds, client_config.bounds,
        "bounds must match"
    );
    assert_eq!(
        server_config.tree_height, client_config.tree_height,
        "tree_height must match"
    );
}

/// Test that client sends `VoxelEditRequest` and receives `VoxelEditAck` from server.
#[test]
fn test_voxel_edit_ack_received() {
    use voxel_map_engine::prelude::{ChunkData, ChunkStatus, FillType, PalettedChunk, WorldVoxel};

    let mut stepper = CrossbeamTestStepper::new();

    add_server_map_systems(&mut stepper);
    stepper.server_app.init_resource::<WorldDirtyState>();
    stepper.server_app.init_resource::<PendingVoxelBroadcasts>();
    stepper.server_app.add_systems(
        Update,
        (handle_voxel_edit_requests, flush_voxel_broadcasts).chain(),
    );

    stepper
        .client_app
        .init_resource::<MessageBuffer<VoxelEditAck>>();
    stepper
        .client_app
        .add_systems(Update, collect_messages::<VoxelEditAck>);

    stepper.init();
    stepper.wait_for_connection();

    // Spawn overworld with a loaded chunk at origin so the edit succeeds
    let chunk_pos = IVec3::ZERO;
    let mut instance = VoxelMapInstance::new(3);
    instance.insert_chunk_data(
        chunk_pos,
        ChunkData {
            voxels: PalettedChunk::SingleValue(WorldVoxel::Solid(1)),
            fill_type: FillType::Uniform(WorldVoxel::Solid(1)),
            hash: 0,
            status: ChunkStatus::Full,
        },
    );
    instance
        .chunk_levels
        .insert(voxel_map_engine::prelude::chunk_to_column(chunk_pos), 0);

    let overworld = stepper
        .server_app
        .world_mut()
        .spawn((
            instance,
            VoxelMapConfig::new(0, 0, 1, None, 3),
            VoxelGenerator(Arc::new(flat_terrain_voxels)),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();
    stepper
        .server_app
        .world_mut()
        .resource_mut::<MapRegistry>()
        .insert(MapInstanceId::Overworld, overworld);

    let client_of = stepper.client_of_entity;
    let _character = spawn_server_character(&mut stepper, client_of);

    // Client sends VoxelEditRequest
    let client_entity = stepper.client_entity;
    stepper
        .client_app
        .world_mut()
        .entity_mut(client_entity)
        .get_mut::<MessageSender<VoxelEditRequest>>()
        .expect("client must have MessageSender<VoxelEditRequest>")
        .send::<VoxelChannel>(VoxelEditRequest {
            position: IVec3::new(5, 5, 5),
            voxel: VoxelType::Air,
            sequence: 0,
        });

    // Poll until client receives VoxelEditAck
    let mut got_ack = false;
    for _ in 0..30 {
        stepper.tick_step(1);
        let buffer = stepper
            .client_app
            .world()
            .resource::<MessageBuffer<VoxelEditAck>>();
        if !buffer.messages.is_empty() {
            got_ack = true;
            break;
        }
    }

    assert!(got_ack, "Client should receive VoxelEditAck from server");
    let buffer = stepper
        .client_app
        .world()
        .resource::<MessageBuffer<VoxelEditAck>>();
    assert_eq!(buffer.messages.len(), 1, "Should receive exactly one ack");
    assert_eq!(
        buffer.messages[0].1.sequence, 0,
        "Ack sequence should match request"
    );
}

/// Set up the server app with VoxelPlugin and ChunkGenerationEnabled so that
/// chunk lifecycle systems (propagator, generation, despawn) run as they do in
/// the real server. `push_chunks_to_clients` is registered separately since
/// `ServerMapPlugin` includes too much unrelated setup for a focused test.
fn add_voxel_server_systems(stepper: &mut CrossbeamTestStepper) {
    use ::server::map::push_chunks_to_clients;
    use voxel_map_engine::prelude::ChunkGenerationEnabled;

    stepper
        .server_app
        .add_plugins(bevy::transform::TransformPlugin);
    stepper.server_app.init_resource::<Assets<Mesh>>();
    stepper
        .server_app
        .init_resource::<Assets<StandardMaterial>>();
    stepper.server_app.add_plugins(VoxelPlugin);
    stepper.server_app.insert_resource(ChunkGenerationEnabled);
    stepper
        .server_app
        .add_systems(Update, push_chunks_to_clients);
}

/// Verify the server pushes `ChunkDataSync` to a client without any client request,
/// purely based on `ClientChunkVisibility` and loaded chunk data.
#[test]
fn test_server_pushes_chunks_without_request() {
    use ::server::map::ClientChunkVisibility;
    use avian3d::prelude::Position;
    use voxel_map_engine::prelude::{
        chunk_to_column, ChunkData, ChunkStatus, ChunkTicket, FillType, PalettedChunk, TicketType,
        WorldVoxel,
    };

    let mut stepper = CrossbeamTestStepper::new();

    add_voxel_server_systems(&mut stepper);
    stepper
        .client_app
        .init_resource::<MessageBuffer<ChunkDataSync>>();
    stepper
        .client_app
        .add_systems(Update, collect_messages::<ChunkDataSync>);

    stepper.init();
    assert!(
        stepper.wait_for_connection(),
        "Client must connect before test proceeds"
    );

    add_server_map_systems(&mut stepper);

    // Spawn overworld map with a chunk at IVec3::ZERO
    let chunk_pos = IVec3::ZERO;
    let chunk_voxels = PalettedChunk::SingleValue(WorldVoxel::Solid(1));
    let mut instance = VoxelMapInstance::new(3);
    instance.insert_chunk_data(
        chunk_pos,
        ChunkData {
            voxels: chunk_voxels.clone(),
            fill_type: FillType::Uniform(WorldVoxel::Solid(1)),
            hash: 0,
            status: ChunkStatus::Full,
        },
    );
    instance.chunk_levels.insert(chunk_to_column(chunk_pos), 0);

    let map_entity = stepper
        .server_app
        .world_mut()
        .spawn((
            instance,
            VoxelMapConfig::new(0, 0, 1, None, 3),
            VoxelGenerator(Arc::new(flat_terrain_voxels)),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();
    stepper
        .server_app
        .world_mut()
        .resource_mut::<MapRegistry>()
        .insert(MapInstanceId::Overworld, map_entity);

    // Spawn character with Position, ChunkTicket, and ClientChunkVisibility
    let client_of = stepper.client_of_entity;
    stepper.server_app.world_mut().spawn((
        CharacterMarker,
        MapInstanceId::Overworld,
        ControlledBy {
            owner: client_of,
            lifetime: Default::default(),
        },
        Position(Vec3::new(0.0, 5.0, 0.0)),
        ChunkTicket::new(map_entity, TicketType::Player, 10),
        ClientChunkVisibility::default(),
    ));

    // Tick until client receives ChunkDataSync
    let mut received = false;
    for _ in 0..50 {
        stepper.tick_step(1);
        let buffer = stepper
            .client_app
            .world()
            .resource::<MessageBuffer<ChunkDataSync>>();
        if !buffer.messages.is_empty() {
            received = true;
            break;
        }
    }

    assert!(
        received,
        "Client should receive ChunkDataSync without sending any request"
    );

    let buffer = stepper
        .client_app
        .world()
        .resource::<MessageBuffer<ChunkDataSync>>();
    let sync = &buffer.messages[0].1;
    assert_eq!(
        sync.chunk_pos, chunk_pos,
        "ChunkDataSync chunk_pos must match the inserted chunk"
    );
    assert_eq!(
        sync.data, chunk_voxels,
        "ChunkDataSync data must match what was inserted"
    );
}

/// Verify the server sends `UnloadColumn` when a player moves far away from
/// previously-sent chunks.
#[test]
fn test_server_sends_unload_column_when_out_of_range() {
    use ::server::map::ClientChunkVisibility;
    use avian3d::prelude::Position;
    use voxel_map_engine::prelude::{
        chunk_to_column, ChunkData, ChunkStatus, ChunkTicket, FillType, PalettedChunk, TicketType,
        WorldVoxel,
    };

    let mut stepper = CrossbeamTestStepper::new();

    add_voxel_server_systems(&mut stepper);
    stepper
        .client_app
        .init_resource::<MessageBuffer<ChunkDataSync>>();
    stepper
        .client_app
        .add_systems(Update, collect_messages::<ChunkDataSync>);
    stepper
        .client_app
        .init_resource::<MessageBuffer<UnloadColumn>>();
    stepper
        .client_app
        .add_systems(Update, collect_messages::<UnloadColumn>);

    stepper.init();
    assert!(
        stepper.wait_for_connection(),
        "Client must connect before test proceeds"
    );

    add_server_map_systems(&mut stepper);

    // Spawn overworld with chunk at origin
    let chunk_pos = IVec3::ZERO;
    let mut instance = VoxelMapInstance::new(3);
    instance.insert_chunk_data(
        chunk_pos,
        ChunkData {
            voxels: PalettedChunk::SingleValue(WorldVoxel::Solid(1)),
            fill_type: FillType::Uniform(WorldVoxel::Solid(1)),
            hash: 0,
            status: ChunkStatus::Full,
        },
    );
    instance.chunk_levels.insert(chunk_to_column(chunk_pos), 0);

    let map_entity = stepper
        .server_app
        .world_mut()
        .spawn((
            instance,
            VoxelMapConfig::new(0, 0, 1, None, 3),
            VoxelGenerator(Arc::new(flat_terrain_voxels)),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();
    stepper
        .server_app
        .world_mut()
        .resource_mut::<MapRegistry>()
        .insert(MapInstanceId::Overworld, map_entity);

    // Spawn character near origin
    let client_of = stepper.client_of_entity;
    let character = stepper
        .server_app
        .world_mut()
        .spawn((
            CharacterMarker,
            MapInstanceId::Overworld,
            ControlledBy {
                owner: client_of,
                lifetime: Default::default(),
            },
            Position(Vec3::ZERO),
            ChunkTicket::new(map_entity, TicketType::Player, 10),
            ClientChunkVisibility::default(),
        ))
        .id();

    // Tick until initial ChunkDataSync is received
    let mut got_initial = false;
    for _ in 0..50 {
        stepper.tick_step(1);
        let buffer = stepper
            .client_app
            .world()
            .resource::<MessageBuffer<ChunkDataSync>>();
        if !buffer.messages.is_empty() {
            got_initial = true;
            break;
        }
    }
    assert!(got_initial, "Client must receive initial ChunkDataSync");

    // Move player far away so origin column leaves range
    stepper
        .server_app
        .world_mut()
        .entity_mut(character)
        .insert(Position(Vec3::new(10000.0, 0.0, 10000.0)));

    // Tick until UnloadColumn is received
    let mut got_unload = false;
    for _ in 0..50 {
        stepper.tick_step(1);
        let buffer = stepper
            .client_app
            .world()
            .resource::<MessageBuffer<UnloadColumn>>();
        if !buffer.messages.is_empty() {
            got_unload = true;
            break;
        }
    }

    assert!(
        got_unload,
        "Client should receive UnloadColumn after player moves out of range"
    );

    let buffer = stepper
        .client_app
        .world()
        .resource::<MessageBuffer<UnloadColumn>>();
    assert_eq!(
        buffer.messages[0].1.column,
        IVec2::ZERO,
        "Unloaded column should be the origin column"
    );
}
