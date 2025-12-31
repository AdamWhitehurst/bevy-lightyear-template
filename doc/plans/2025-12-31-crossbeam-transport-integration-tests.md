# Crossbeam Transport Integration Tests Implementation Plan

## Overview

Complete the integration testing for lightyear's Crossbeam IO transport by implementing proper connection verification and bidirectional message passing tests. The current tests only verify basic plugin initialization but lack connection establishment checks, timeline synchronization, and message exchange validation.

## Current State Analysis

### Existing Infrastructure
- Crossbeam transport is defined in both client ([crates/client/src/network.rs:18](crates/client/src/network.rs#L18)) and server ([crates/server/src/network.rs:19-22](crates/server/src/network.rs#L19-L22)) network modules
- Basic plugin initialization test exists at [crates/server/tests/integration.rs:151-219](crates/server/tests/integration.rs#L151-L219)
- Test dependencies already include `lightyear_crossbeam = "0.25.5"` and `protocol = { features = ["test_utils"] }`
- Protocol defines voxel messages: `VoxelEditRequest`, `VoxelEditBroadcast`, `VoxelStateSync` via `VoxelChannel`

### Missing Components
1. Connection establishment verification (`Connected` component checks)
2. Link layer verification (`Linked` component checks - critical for crossbeam)
3. Timeline synchronization checks (`IsSynced<InputTimeline>`, `IsSynced<InterpolationTimeline>`)
4. Message passing tests with buffer observer pattern
5. Bidirectional message exchange validation
6. Trigger/event passing tests

### Key Discoveries
- Crossbeam requires explicit `Linked` component (other transports auto-add via ServerLink)
- Lightyear provides `ClientServerStepper` pattern in reference tests ([git/lightyear/lightyear_tests/src/stepper.rs](git/lightyear/lightyear_tests/src/stepper.rs))
- Buffer observer pattern is standard for message collection ([git/lightyear/lightyear_tests/src/client_server/messages.rs:13-30](git/lightyear/lightyear_tests/src/client_server/messages.rs#L13-L30))
- Connection timeout should be 5 ticks (preferred) but may extend to 50 ticks if needed

## Desired End State

After this plan is complete:

1. **Connection Test**: A test that verifies crossbeam client-server connection establishment with proper `Connected` and `Linked` component checks
2. **Message Passing Test**: Tests validating bidirectional message exchange (client→server and server→client)
3. **Event/Trigger Test**: Tests validating event/trigger passing using `EventSender` and `On<RemoteEvent<T>>` pattern

### Verification
- Run `cargo test --package server --test integration` - all tests pass
- Tests verify connection in 5 ticks (may extend to 50 if necessary)
- Message passing works bidirectionally
- Events/triggers propagate correctly

## What We're NOT Doing

- Multi-client testing scenarios (deferred - single client only)
- Entity replication tests over crossbeam transport (deferred)
- Timeline sync verification in every test (only where relevant)
- ClientServerStepper custom implementation (use lightyear's pattern as reference)
- LocalAddr component on ClientOf entities (not needed per stepper pattern)

## Implementation Approach

Enhance the existing `crates/server/tests/integration.rs` file by adding three new comprehensive tests that follow lightyear's testing patterns. Each test builds on the previous, starting with connection verification, then message passing, then event/trigger handling.

Use the buffer observer pattern from lightyear tests to collect and verify messages. Follow the manual time control pattern with `TimeUpdateStrategy::ManualInstant` for deterministic test execution.

## Phase 1: Add Connection Verification Test

### Overview
Create a test that properly verifies crossbeam client-server connection establishment, checking for `Connected` and `Linked` components with timeline synchronization.

### Changes Required

#### 1. Test Helper Types
**File**: `crates/server/tests/integration.rs`
**Location**: After the existing imports, before tests
**Changes**: Add helper types for test support

```rust
use bevy::time::TimeUpdateStrategy;
use lightyear::prelude::client::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use std::time::Duration;

/// Helper to create deterministic time control
fn setup_manual_time(app: &mut App) -> bevy::platform::time::Instant {
    let now = bevy::platform::time::Instant::now();
    app.insert_resource(TimeUpdateStrategy::ManualInstant(now));
    now
}

/// Advance time by duration in both apps
fn advance_time(
    server_app: &mut App,
    client_app: &mut App,
    current_time: &mut bevy::platform::time::Instant,
    duration: Duration,
) {
    *current_time += duration;
    server_app.insert_resource(TimeUpdateStrategy::ManualInstant(*current_time));
    client_app.insert_resource(TimeUpdateStrategy::ManualInstant(*current_time));
}

/// Wait for connection with configurable timeout (in ticks)
fn wait_for_connection(
    server_app: &mut App,
    client_app: &mut App,
    current_time: &mut bevy::platform::time::Instant,
    client_entity: Entity,
    max_ticks: usize,
) -> bool {
    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);

    for tick in 0..max_ticks {
        advance_time(server_app, client_app, current_time, tick_duration);
        server_app.update();
        client_app.update();

        if client_app
            .world()
            .get::<Connected>(client_entity)
            .is_some()
        {
            info!("Client connected after {} ticks", tick + 1);
            return true;
        }
    }
    false
}
```

#### 2. Connection Verification Test
**File**: `crates/server/tests/integration.rs`
**Location**: After existing tests
**Changes**: Add new test function

```rust
/// Test that client and server connect properly via crossbeam transport
/// Verifies Connected and Linked components are present
#[test]
fn test_crossbeam_connection_establishment() {
    // Create crossbeam transport pair
    let (crossbeam_client, crossbeam_server) = lightyear_crossbeam::CrossbeamIo::new_pair();

    // Setup server app
    let mut server_app = App::new();
    server_app.add_plugins(MinimalPlugins);
    server_app.add_plugins(bevy::log::LogPlugin::default());
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

    // Setup client app
    let mut client_app = App::new();
    client_app.add_plugins(MinimalPlugins);
    client_app.add_plugins(bevy::log::LogPlugin::default());
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

    // Setup manual time control
    let mut current_time = setup_manual_time(&mut server_app);
    setup_manual_time(&mut client_app);

    // Run startup systems
    server_app.update();
    client_app.update();

    // Get entity references
    let server_entity = server_app
        .world_mut()
        .query_filtered::<Entity, With<NetcodeServer>>()
        .single(server_app.world())
        .expect("Server entity not found");

    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, With<Client>>()
        .single(client_app.world())
        .expect("Client entity not found");

    // Trigger Start on server
    server_app
        .world_mut()
        .commands()
        .trigger(Start {
            entity: server_entity,
        });
    server_app.update();

    // Trigger Connect on client
    client_app
        .world_mut()
        .commands()
        .trigger(Connect {
            entity: client_entity,
        });
    client_app.update();

    // Wait for connection (try 5 ticks first, extend to 50 if needed)
    let connected = wait_for_connection(
        &mut server_app,
        &mut client_app,
        &mut current_time,
        client_entity,
        5,
    );

    // If not connected in 5 ticks, try up to 50
    let connected = if !connected {
        warn!("Connection not established in 5 ticks, extending to 50 ticks");
        wait_for_connection(
            &mut server_app,
            &mut client_app,
            &mut current_time,
            client_entity,
            45, // Already ran 5, so run 45 more
        )
    } else {
        connected
    };

    assert!(
        connected,
        "Client should have Connected component after connection establishment"
    );

    // Verify Linked component (critical for crossbeam)
    assert!(
        client_app
            .world()
            .get::<Linked>(client_entity)
            .is_some(),
        "Client should have Linked component for crossbeam transport"
    );

    // Verify server has connected client
    let server_client_entities: Vec<Entity> = server_app
        .world_mut()
        .query_filtered::<Entity, With<Connected>>()
        .iter(server_app.world())
        .collect();
    assert!(
        !server_client_entities.is_empty(),
        "Server should have at least one connected client entity"
    );

    // Verify ReplicationSender was added by server observer
    let has_replication_sender = server_client_entities.iter().any(|&entity| {
        server_app
            .world()
            .get::<ReplicationSender>(entity)
            .is_some()
    });
    assert!(
        has_replication_sender,
        "Server should add ReplicationSender to connected client"
    );

    info!("✓ Crossbeam connection test passed!");
}
```

### Success Criteria

**Implementation Note**: Used `CrossbeamTestStepper` pattern (based on lightyear's stepper) instead of manual setup. Required `RawServer`/`RawClient` and explicit `Linked` components.

#### Automated Verification:
- [x] Test compiles: `cargo test --package server --test integration --no-run`
- [x] Test passes: `cargo test --package server --test integration test_crossbeam_connection_establishment`
- [x] All existing tests still pass: `cargo test --package server --test integration`

#### Manual Verification:
- [x] Test completes in under 1 second
- [x] Connection establishes within 5 ticks (actually 1 tick with stepper)
- [x] No panics or errors in test output

---

## Phase 2: Add Message Passing Tests

### Overview
Add tests for bidirectional message passing (client→server and server→client) using the buffer observer pattern.

### Changes Required

#### 1. Message Buffer Helper Types
**File**: `crates/server/tests/integration.rs`
**Location**: After Phase 1 helpers, before tests
**Changes**: Add buffer observer pattern types

```rust
use bevy::ecs::system::SystemId;

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
fn collect_messages<M: Message + Debug + Clone>(
    mut receiver: Query<(Entity, &mut MessageReceiver<M>)>,
    mut buffer: ResMut<MessageBuffer<M>>,
) {
    receiver.iter_mut().for_each(|(entity, mut receiver)| {
        receiver.receive().for_each(|m| {
            buffer.messages.push((entity, m));
        });
    });
}
```

#### 2. Client-to-Server Message Test
**File**: `crates/server/tests/integration.rs`
**Location**: After Phase 1 test
**Changes**: Add client→server message test

```rust
/// Test sending messages from client to server via crossbeam
#[test]
fn test_crossbeam_client_to_server_messages() {
    // Setup apps with crossbeam transport (same as connection test)
    let (crossbeam_client, crossbeam_server) = lightyear_crossbeam::CrossbeamIo::new_pair();

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

    // Add message buffer to server
    server_app.init_resource::<MessageBuffer<VoxelEditRequest>>();
    server_app.add_systems(Update, collect_messages::<VoxelEditRequest>);

    // Setup time and connect
    let mut current_time = setup_manual_time(&mut server_app);
    setup_manual_time(&mut client_app);

    server_app.update();
    client_app.update();

    let server_entity = server_app
        .world_mut()
        .query_filtered::<Entity, With<NetcodeServer>>()
        .single(server_app.world())
        .unwrap();

    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, With<Client>>()
        .single(client_app.world())
        .unwrap();

    server_app.world_mut().commands().trigger(Start {
        entity: server_entity,
    });
    server_app.update();

    client_app.world_mut().commands().trigger(Connect {
        entity: client_entity,
    });
    client_app.update();

    // Wait for connection
    let connected = wait_for_connection(
        &mut server_app,
        &mut client_app,
        &mut current_time,
        client_entity,
        50,
    );
    assert!(connected, "Client must be connected before sending messages");

    // Send message from client
    let test_message = VoxelEditRequest {
        position: IVec3::new(1, 2, 3),
        voxel: VoxelType::Solid(42),
    };

    client_app
        .world_mut()
        .entity_mut(client_entity)
        .get_mut::<MessageSender<VoxelEditRequest>>()
        .expect("Client should have MessageSender")
        .send::<VoxelChannel>(test_message.clone());

    // Step simulation to deliver message
    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    advance_time(&mut server_app, &mut client_app, &mut current_time, tick_duration);
    client_app.update();
    server_app.update();

    // Verify message received on server
    let buffer = server_app.world().resource::<MessageBuffer<VoxelEditRequest>>();
    assert_eq!(
        buffer.messages.len(),
        1,
        "Server should receive exactly one message"
    );

    let (source_entity, received_message) = &buffer.messages[0];
    assert_eq!(
        received_message, &test_message,
        "Received message should match sent message"
    );

    // Verify source entity is the server's client representation
    assert!(
        server_app.world().get::<Connected>(*source_entity).is_some(),
        "Source entity should be a connected client on server"
    );

    info!("✓ Client-to-server message test passed!");
}
```

#### 3. Server-to-Client Message Test
**File**: `crates/server/tests/integration.rs`
**Location**: After client-to-server test
**Changes**: Add server→client message test

```rust
/// Test sending messages from server to client via crossbeam
#[test]
fn test_crossbeam_server_to_client_messages() {
    // Setup apps (same pattern as previous test)
    let (crossbeam_client, crossbeam_server) = lightyear_crossbeam::CrossbeamIo::new_pair();

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

    // Add message buffer to client
    client_app.init_resource::<MessageBuffer<VoxelEditBroadcast>>();
    client_app.add_systems(Update, collect_messages::<VoxelEditBroadcast>);

    // Setup time and connect
    let mut current_time = setup_manual_time(&mut server_app);
    setup_manual_time(&mut client_app);

    server_app.update();
    client_app.update();

    let server_entity = server_app
        .world_mut()
        .query_filtered::<Entity, With<NetcodeServer>>()
        .single(server_app.world())
        .unwrap();

    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, With<Client>>()
        .single(client_app.world())
        .unwrap();

    server_app.world_mut().commands().trigger(Start {
        entity: server_entity,
    });
    server_app.update();

    client_app.world_mut().commands().trigger(Connect {
        entity: client_entity,
    });
    client_app.update();

    // Wait for connection
    let connected = wait_for_connection(
        &mut server_app,
        &mut client_app,
        &mut current_time,
        client_entity,
        50,
    );
    assert!(connected, "Client must be connected before receiving messages");

    // Get server's client representation entity
    let server_client_entity = server_app
        .world_mut()
        .query_filtered::<Entity, With<Connected>>()
        .iter(server_app.world())
        .next()
        .expect("Server should have connected client entity");

    // Send message from server to client
    let test_message = VoxelEditBroadcast {
        position: IVec3::new(4, 5, 6),
        voxel: VoxelType::Solid(99),
    };

    server_app
        .world_mut()
        .entity_mut(server_client_entity)
        .get_mut::<MessageSender<VoxelEditBroadcast>>()
        .expect("Server client entity should have MessageSender")
        .send::<VoxelChannel>(test_message.clone());

    // Step simulation to deliver message (may need 2 ticks for server→client)
    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    for _ in 0..2 {
        advance_time(&mut server_app, &mut client_app, &mut current_time, tick_duration);
        server_app.update();
        client_app.update();
    }

    // Verify message received on client
    let buffer = client_app.world().resource::<MessageBuffer<VoxelEditBroadcast>>();
    assert_eq!(
        buffer.messages.len(),
        1,
        "Client should receive exactly one message"
    );

    let (source_entity, received_message) = &buffer.messages[0];
    assert_eq!(
        received_message, &test_message,
        "Received message should match sent message"
    );
    assert_eq!(
        *source_entity, client_entity,
        "Message should be received by client entity"
    );

    info!("✓ Server-to-client message test passed!");
}
```

### Success Criteria

**Implementation Note**: Used stepper pattern. Added `PartialEq` to voxel message types for assertions. Delivery takes 2-3 ticks.

#### Automated Verification:
- [x] Tests compile: `cargo test --package server --test integration --no-run`
- [x] Client-to-server test passes: `cargo test --package server --test integration test_crossbeam_client_to_server_messages`
- [x] Server-to-client test passes: `cargo test --package server --test integration test_crossbeam_server_to_client_messages`
- [x] All tests pass: `cargo test --package server --test integration`

#### Manual Verification:
- [x] Messages are delivered within 1-2 ticks (actually 2-3 ticks)
- [x] Message content is preserved exactly
- [x] No message duplication occurs
- [x] Tests complete quickly (under 2 seconds each)

---

## Phase 3: Add Event/Trigger Tests

### Overview
Add test for event/trigger passing using `EventSender` and `On<RemoteEvent<T>>` observer pattern to verify events propagate correctly over crossbeam transport.

### Changes Required

#### 1. Test Event Type
**File**: `crates/protocol/src/lib.rs`
**Location**: After existing component definitions, before `ProtocolPlugin`
**Changes**: Add test event type (conditionally compiled)

```rust
#[cfg(feature = "test_utils")]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Reflect, Event)]
pub struct TestTrigger {
    pub data: String,
}
```

#### 2. Register Test Event in Protocol
**File**: `crates/protocol/src/lib.rs`
**Location**: In `ProtocolPlugin::build()` method, after channel registration
**Changes**: Register test trigger conditionally

```rust
#[cfg(feature = "test_utils")]
app.add_message::<TestTrigger>(NetworkDirection::Bidirectional);
```

#### 3. Event Buffer Helper
**File**: `crates/server/tests/integration.rs`
**Location**: After message buffer helpers
**Changes**: Add event buffer type

```rust
use lightyear::prelude::PeerMetadata;

/// Buffer resource to collect received events/triggers
#[derive(Resource)]
struct EventBuffer<E> {
    events: Vec<(Entity, E)>,
}

impl<E> Default for EventBuffer<E> {
    fn default() -> Self {
        Self {
            events: Vec::new(),
        }
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
        .get(&trigger.from)
        .expect("Remote entity should be in peer metadata mapping");
    buffer.events.push((remote, trigger.trigger.clone()));
}
```

#### 4. Event/Trigger Passing Test
**File**: `crates/server/tests/integration.rs`
**Location**: After message tests
**Changes**: Add event trigger test

```rust
use protocol::TestTrigger;

/// Test sending events/triggers from client to server via crossbeam
#[test]
fn test_crossbeam_event_triggers() {
    // Setup apps
    let (crossbeam_client, crossbeam_server) = lightyear_crossbeam::CrossbeamIo::new_pair();

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

    // Add event buffer and observer to server
    server_app.init_resource::<EventBuffer<TestTrigger>>();
    server_app.add_observer(collect_events::<TestTrigger>);

    // Setup time and connect
    let mut current_time = setup_manual_time(&mut server_app);
    setup_manual_time(&mut client_app);

    server_app.update();
    client_app.update();

    let server_entity = server_app
        .world_mut()
        .query_filtered::<Entity, With<NetcodeServer>>()
        .single(server_app.world())
        .unwrap();

    let client_entity = client_app
        .world_mut()
        .query_filtered::<Entity, With<Client>>()
        .single(client_app.world())
        .unwrap();

    server_app.world_mut().commands().trigger(Start {
        entity: server_entity,
    });
    server_app.update();

    client_app.world_mut().commands().trigger(Connect {
        entity: client_entity,
    });
    client_app.update();

    // Wait for connection
    let connected = wait_for_connection(
        &mut server_app,
        &mut client_app,
        &mut current_time,
        client_entity,
        50,
    );
    assert!(connected, "Client must be connected before sending events");

    // Get server's client entity for verification
    let server_client_entity = server_app
        .world_mut()
        .query_filtered::<Entity, With<Connected>>()
        .iter(server_app.world())
        .next()
        .expect("Server should have connected client entity");

    // Send trigger from client
    let test_trigger = TestTrigger {
        data: "test_event_data".to_string(),
    };

    client_app
        .world_mut()
        .entity_mut(client_entity)
        .get_mut::<EventSender<TestTrigger>>()
        .expect("Client should have EventSender")
        .trigger::<VoxelChannel>(test_trigger.clone());

    // Step simulation to deliver event
    let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    advance_time(&mut server_app, &mut client_app, &mut current_time, tick_duration);
    client_app.update();
    server_app.update();

    // Verify event received on server
    let buffer = server_app.world().resource::<EventBuffer<TestTrigger>>();
    assert_eq!(
        buffer.events.len(),
        1,
        "Server should receive exactly one event"
    );

    let (source_entity, received_event) = &buffer.events[0];
    assert_eq!(
        received_event, &test_trigger,
        "Received event should match sent event"
    );
    assert_eq!(
        *source_entity, server_client_entity,
        "Event should be from server's client entity"
    );

    info!("✓ Event/trigger test passed!");
}
```

### Success Criteria

**Implementation Note**: Used `register_event()` API with `On<RemoteEvent<E>>` observer pattern. Events delivered in 3 ticks.

#### Automated Verification:
- [x] Protocol compiles with test event: `cargo test --package protocol --no-run`
- [x] Test compiles: `cargo test --package server --test integration --no-run`
- [x] Event test passes: `cargo test --package server --test integration test_crossbeam_event_triggers`
- [x] All integration tests pass: `cargo test --package server --test integration`

#### Manual Verification:
- [x] Events are delivered within 1 tick (actually 3 ticks)
- [x] Event data is preserved exactly
- [x] Observer pattern works correctly with PeerMetadata
- [x] Test completes quickly (under 2 seconds)

---

## Testing Strategy

### Unit Tests
Not applicable - these are integration tests

### Integration Tests
All tests in `crates/server/tests/integration.rs`:
- `test_crossbeam_connection_establishment` - Connection verification
- `test_crossbeam_client_to_server_messages` - Client→server messaging
- `test_crossbeam_server_to_client_messages` - Server→client messaging
- `test_crossbeam_event_triggers` - Event/trigger propagation

### Manual Testing Steps
1. Run all integration tests: `cargo test --package server --test integration`
2. Verify test output shows connection timing (should be ≤5 ticks normally)
3. Check that no warnings or errors appear in logs
4. Confirm tests complete quickly (all tests < 10 seconds total)

## Performance Considerations

- Tests use manual time control (`TimeUpdateStrategy::ManualInstant`) for deterministic execution
- 5-tick timeout preferred for fast test execution
- Extended to 50 ticks if connection takes longer (with warning log)
- Single-client scenarios keep test complexity low
- Crossbeam channels are unbounded, so no backpressure concerns in tests

## Migration Notes

Not applicable - adding new tests, not migrating existing code.

## References

- Original research: `doc/research/2025-12-31-crossbeam-clientserverstepper-integration.md`
- Lightyear stepper implementation: `git/lightyear/lightyear_tests/src/stepper.rs`
- Lightyear message tests: `git/lightyear/lightyear_tests/src/client_server/messages.rs`
- Current integration tests: `crates/server/tests/integration.rs:151-219`
- Client network module: `crates/client/src/network.rs:114-116`
- Server network module: `crates/server/src/network.rs:175-190`
