---
date: 2025-12-31 09:46:58 PST
researcher: Claude Sonnet 4.5
git_commit: c94a8558ce5c4169bf6efb10fec5c31336193686
branch: master
repository: bevy-lightyear-template
topic: "Crossbeam IO Transport and ClientServerStepper Integration"
tags: [research, codebase, networking, testing, lightyear, crossbeam, integration-tests]
status: complete
last_updated: 2025-12-31
last_updated_by: Claude Sonnet 4.5
last_updated_note: "Added implementation decisions based on project requirements"
---

# Research: Crossbeam IO Transport and ClientServerStepper Integration

**Date**: 2025-12-31 09:46:58 PST
**Researcher**: Claude Sonnet 4.5
**Git Commit**: c94a8558ce5c4169bf6efb10fec5c31336193686
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to finish integration of lightyear's Crossbeam IO transport and ClientServerStepper. Update the integration tests validating that crossbeam transport to setup client and server connection using ClientServerStepper and ensure that lightyear messages can be passed between them. The existing crossbeam tests are NOT complete - they don't check that the client properly connects nor do they test message passing.

## Summary

The codebase has partial crossbeam transport integration with incomplete tests. The project has crossbeam transport defined in both client and server network modules ([client/src/network.rs](crates/client/src/network.rs), [server/src/network.rs](crates/server/src/network.rs)) and a basic plugin initialization test ([server/tests/integration.rs:151-219](crates/server/tests/integration.rs#L151-L219)), but lacks proper connection verification and message passing tests.

Lightyear provides `ClientServerStepper` in `git/lightyear/lightyear_tests/src/stepper.rs` which creates separate Bevy apps for client/server connected via crossbeam channels. The stepper handles initialization, connection establishment, timeline synchronization, and provides helper methods for stepping simulation and accessing entities.

To complete the integration, tests must verify `Connected` component on client/server entities, `IsSynced<InputTimeline>` and `IsSynced<InterpolationTimeline>` on clients, and validate bidirectional message passing using `MessageSender`/`MessageReceiver` components with buffer observer pattern.

## Detailed Findings

### Current Crossbeam Transport Implementation

#### Client Network Module
**Location**: [crates/client/src/network.rs](crates/client/src/network.rs)

The client transport enum includes crossbeam:
```rust
// Line 17-19
pub enum ClientTransport {
    Udp,
    WebTransport { certificate_digest: String },
    Crossbeam(CrossbeamIo),  // Line 18
}
```

Setup function handles crossbeam case:
```rust
// Lines 114-116
ClientTransport::Crossbeam(crossbeam_io) => {
    entity_builder.insert(crossbeam_io);
}
```

**Key Import**: `use lightyear::crossbeam::CrossbeamIo;` (line 2)

#### Server Network Module
**Location**: [crates/server/src/network.rs](crates/server/src/network.rs)

Server transport enum:
```rust
// Lines 18-22
pub enum ServerTransport {
    Udp { port: u16 },
    WebTransport { port: u16 },
    WebSocket { port: u16 },
    Crossbeam { io: lightyear_crossbeam::CrossbeamIo },  // Line 21
}
```

Server spawning logic:
```rust
// Lines 175-190
ServerTransport::Crossbeam { io } => {
    let server = commands
        .spawn((
            Name::new("Crossbeam Server"),
            Server::default(),
            NetcodeServer::new(server::NetcodeConfig {
                protocol_id: config.protocol_id,
                private_key: Key::from(config.private_key),
                ..default()
            }),
            io,
        ))
        .id();
    commands.trigger(Start { entity: server });
    info!("Crossbeam server started for testing");
}
```

**Key Import**: `use lightyear_crossbeam::CrossbeamIo;` (implied from usage)

#### Existing Integration Test
**Location**: [crates/server/tests/integration.rs:151-219](crates/server/tests/integration.rs#L151-L219)

Current test (`test_client_server_plugin_initialization`):
- Creates crossbeam channel pair via `lightyear_crossbeam::CrossbeamIo::new_pair()` (line 154)
- Sets up server app with `ServerTransport::Crossbeam` (lines 163-173)
- Sets up client app with `ClientTransport::Crossbeam` (lines 182-192)
- Runs startup systems once (lines 195-196)
- Verifies entity spawning only (lines 199-216)

**Gaps Identified**:
1. No `Connect` trigger on client entity
2. No `Start` trigger on server entity
3. No verification of `Connected` component
4. No verification of `Linked` component (critical for crossbeam)
5. No timeline synchronization verification
6. No message passing tests

**Note in connection_flow.rs**: [crates/server/tests/connection_flow.rs:41-43](crates/server/tests/connection_flow.rs#L41-L43) states:
```rust
// NOTE: Full client-server connection test with crossbeam requires lightyear "test_utils" feature
// and CrossbeamIo which is not available in standard lightyear features.
// For full integration testing, use the stepper pattern from lightyear_tests examples.
```

### Lightyear ClientServerStepper Implementation

#### Architecture Overview
**Location**: [git/lightyear/lightyear_tests/src/stepper.rs](git/lightyear/lightyear_tests/src/stepper.rs)

`ClientServerStepper` struct (lines 36-47):
```rust
pub struct ClientServerStepper {
    pub client_apps: Vec<App>,
    pub server_app: App,
    pub client_entities: Vec<Entity>,
    pub server_entity: Entity,
    pub client_of_entities: Vec<Entity>,
    pub host_client_entity: Option<Entity>,
    pub frame_duration: Duration,
    pub tick_duration: Duration,
    pub current_time: bevy::platform::time::Instant,
    pub avian_mode: AvianReplicationMode,
}
```

Configuration types:
- `StepperConfig` (lines 66-73): Defines frame/tick duration, client types, server type, auto-init flag
- `ClientType` (lines 50-56): Host, Raw, Netcode, Steam
- `ServerType` (lines 59-63): Raw, Netcode, Steam

#### Crossbeam Channel Pair Creation
**Location**: [git/lightyear/lightyear_crossbeam/src/lib.rs:49-54](git/lightyear/lightyear_crossbeam/src/lib.rs#L49-L54)

```rust
pub fn new_pair() -> (Self, Self) {
    let (sender1, receiver1) = crossbeam_channel::unbounded();
    let (sender2, receiver2) = crossbeam_channel::unbounded();
    (Self { sender: sender1, receiver: receiver2 },
     Self { sender: sender2, receiver: receiver1 })
}
```

Establishes bidirectional channel where:
- First instance: sends via `sender1`, receives via `receiver2`
- Second instance: sends via `sender2`, receives via `receiver1`

#### Server Initialization Flow
**Location**: [git/lightyear/lightyear_tests/src/stepper.rs:142-196](git/lightyear/lightyear_tests/src/stepper.rs#L142-L196)

Steps:
1. Create new `App` (line 148)
2. Add minimal plugins (lines 150-155): `MinimalPlugins`, `StatesPlugin`, `InputPlugin`, `LogPlugin`
3. Add `ServerPlugins` with tick duration (line 160)
4. Add `ProtocolPlugin` (line 162)
5. Spawn server entity with `DeltaManager::default()` (line 163)
6. Insert transport component based on `ServerType` (lines 165-182)

For crossbeam testing, server typically uses `ServerType::Netcode` which inserts `NetcodeServer` component.

#### Client Initialization Flow
**Location**: [git/lightyear/lightyear_tests/src/stepper.rs:198-312](git/lightyear/lightyear_tests/src/stepper.rs#L198-L312)

Steps for regular client (not Host):
1. Create new client `App` (line 199)
2. Add minimal plugins (lines 200-205)
3. Add `ClientPlugins` with tick duration (lines 211-213)
4. Add `ProtocolPlugin` (lines 215-217)
5. Call `app.finish()` and `app.cleanup()` (lines 218-219)
6. Create crossbeam pair: `let (crossbeam_client, crossbeam_server) = CrossbeamIo::new_pair()` (line 221)
7. Setup `Authentication::Manual` (lines 223-228)
8. Spawn client entity (line 253) with:
   - `Client::default()`
   - `PingManager` with zero interval (lines 256-258)
   - `ReplicationSender::default()` and `ReplicationReceiver::default()` (lines 259-260)
   - `crossbeam_client` (line 261)
   - `PredictionManager::default()` (line 264)
   - Transport component based on `ClientType` (lines 266-280)
9. Spawn "ClientOf" entity in server app (lines 282-309) with:
   - `LinkOf { server: self.server_entity }` (lines 286-288)
   - `PingManager` (lines 290-292)
   - `ReplicationSender` and `ReplicationReceiver` (lines 294-295)
   - `Link::new(None)` (line 297)
   - `PeerAddr` using client index as mock port (lines 298-301)
   - **CRITICAL**: `Linked` marker component (line 303)
   - `crossbeam_server` (line 304)

**Critical Requirement**: Crossbeam needs explicit `Linked` component on both client and ClientOf entities. Other transports auto-add via ServerLink observer, but crossbeam has immediate link.

#### Connection Initialization
**Location**: [git/lightyear/lightyear_tests/src/stepper.rs:411-452](git/lightyear/lightyear_tests/src/stepper.rs#L411-L452)

The `init()` method:
1. Finish plugin setup on server app (lines 412-418)
2. Initialize time with `Instant::now()` (lines 421-422)
3. Update server's `Time<Real>` resource (lines 423-427)
4. **Trigger `Start` event on server entity** (lines 428-430)
5. **Flush server world** (line 433) - critical for HostServer
6. Initialize client apps' time (lines 435-439)
7. **Trigger `Connect` event on all client entities** (lines 440-442)
8. Trigger `Connect` on host client if exists (lines 444-448)
9. Call `wait_for_connection()` (line 450)
10. Call `wait_for_sync()` (line 451)

#### Connection Establishment Verification
**Location**: [git/lightyear/lightyear_tests/src/stepper.rs:455-465](git/lightyear/lightyear_tests/src/stepper.rs#L455-L465)

`wait_for_connection()`:
```rust
pub fn wait_for_connection(&mut self) {
    for _ in 0..50 {
        if (0..self.client_entities.len())
            .all(|client_id| self.client(client_id).contains::<Connected>())
        {
            info!("Clients are all connected");
            break;
        }
        self.tick_step(1);
    }
}
```

Loops up to 50 ticks checking for `Connected` component on all clients.

#### Synchronization Verification
**Location**: [git/lightyear/lightyear_tests/src/stepper.rs:468-481](git/lightyear/lightyear_tests/src/stepper.rs#L468-L481)

`wait_for_sync()`:
```rust
pub fn wait_for_sync(&mut self) {
    for _ in 0..50 {
        if (0..self.client_entities.len()).all(|client_id| {
            self.client(client_id).contains::<IsSynced<InputTimeline>>()
                && self.client(client_id).contains::<IsSynced<InterpolationTimeline>>()
        }) {
            info!("Clients are all synced");
            break;
        }
        self.tick_step(1);
    }
}
```

Verifies both timeline types are synchronized.

#### Time Management
**Location**: [git/lightyear/lightyear_tests/src/stepper.rs:483-494](git/lightyear/lightyear_tests/src/stepper.rs#L483-L494)

Manual time control via `TimeUpdateStrategy::ManualInstant`:
```rust
pub fn advance_time(&mut self, duration: Duration) {
    self.current_time += duration;
    self.client_apps.iter_mut().for_each(|client_app| {
        client_app.insert_resource(TimeUpdateStrategy::ManualInstant(self.current_time));
    });
    self.server_app.insert_resource(TimeUpdateStrategy::ManualInstant(self.current_time));
}
```

Step methods:
- `frame_step(n)` (lines 504-523): Advance by n * frame_duration
- `tick_step(n)` (lines 547-566): Advance by n * tick_duration

### Message Passing Test Patterns

#### Buffer Observer Pattern
**Location**: [git/lightyear/lightyear_tests/src/client_server/messages.rs:13-30](git/lightyear/lightyear_tests/src/client_server/messages.rs#L13-L30)

Standard pattern for collecting messages:
```rust
#[derive(Resource)]
struct Buffer<M>(Vec<(Entity, M)>);

impl<M> Default for Buffer<M> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

fn count_messages_observer<M: Message + Debug>(
    mut receiver: Query<(Entity, &mut MessageReceiver<M>)>,
    mut buffer: ResMut<Buffer<M>>,
) {
    receiver.iter_mut().for_each(|(entity, mut receiver)| {
        receiver.receive().for_each(|m| buffer.0.push((entity, m)));
    })
}
```

#### Client-to-Server Message Test
**Location**: [git/lightyear/lightyear_tests/src/client_server/messages.rs:32-62](git/lightyear/lightyear_tests/src/client_server/messages.rs#L32-L62)

Setup:
```rust
let mut stepper = ClientServerStepper::from_config(StepperConfig::single());
stepper.server_app.init_resource::<Buffer<StringMessage>>();
stepper.server_app.add_systems(Update, count_messages_observer::<StringMessage>);
```

Send from client:
```rust
let send_message = StringMessage("Hello".to_string());
stepper.client_mut(0)
    .get_mut::<MessageSender<StringMessage>>()
    .unwrap()
    .send::<Channel1>(send_message.clone());

stepper.frame_step(1);
```

Verify on server:
```rust
let received_messages = stepper.server_app.world().resource::<Buffer<StringMessage>>();
assert_eq!(
    &received_messages.0,
    &vec![(stepper.client_of_entities[0], send_message)]
);
```

**Key Points**:
- `client_mut(0)` gets mutable access to client entity
- `MessageSender<T>` component sends messages
- `Channel1` specifies the channel type
- `frame_step(1)` processes one simulation frame
- `client_of_entities[0]` is the server-side entity representing the client

#### Server-to-Client Message Test
**Location**: [git/lightyear/lightyear_tests/src/client_server/messages.rs:64-79](git/lightyear/lightyear_tests/src/client_server/messages.rs#L64-L79)

Setup on client:
```rust
stepper.client_app().init_resource::<Buffer<StringMessage>>();
stepper.client_app().add_systems(Update, count_messages_observer::<StringMessage>);
```

Send from server:
```rust
let send_message = StringMessage("World".to_string());
stepper.client_of_mut(0)
    .get_mut::<MessageSender<StringMessage>>()
    .unwrap()
    .send::<Channel1>(send_message.clone());

stepper.frame_step(2);
```

Verify on client:
```rust
let received_messages = stepper.client_apps[0].world().resource::<Buffer<StringMessage>>();
assert_eq!(
    &received_messages.0,
    &vec![(stepper.client_entities[0], send_message)]
);
```

**Key Points**:
- `client_of_mut(0)` gets server-side client representation
- Server sends via ClientOf entity's `MessageSender`
- May need 2 frame steps for bidirectional exchange timing

#### Crossbeam Communication Systems
**Location**: [git/lightyear/lightyear_crossbeam/src/lib.rs:87-117](git/lightyear/lightyear_crossbeam/src/lib.rs#L87-L117)

Send system (lines 87-98):
```rust
fn send(mut query: Query<(&mut Link, &CrossbeamIo), With<Linked>>) {
    query.iter_mut().for_each(|(mut link, crossbeam_io)| {
        link.send.drain(..).for_each(|payload| {
            crossbeam_io.sender.try_send(payload.into()).unwrap();
        });
    });
}
```

Receive system (lines 100-117):
```rust
fn receive(mut query: Query<(&mut Link, &CrossbeamIo), With<Linked>>) {
    query.iter_mut().for_each(|(mut link, crossbeam_io)| {
        loop {
            match crossbeam_io.receiver.try_recv() {
                Ok(data) => {
                    link.recv.push(data.into());
                }
                Err(TryRecvError::Empty) => {
                    break;
                }
                Err(TryRecvError::Disconnected) => {
                    break;
                }
            }
        }
    });
}
```

System scheduling:
- `receive` in `PreUpdate::LinkReceiveSystems::BufferToLink` (lines 126-129)
- `send` in `PostUpdate::LinkSystems::Send` (line 130)

### Connection Verification Requirements

Based on lightyear test patterns, proper connection requires checking:

1. **`Connected` Component** - Transport-level connection established
   - On client entity in client app
   - On ClientOf entity in server app

2. **`Linked` Component** - Link layer ready
   - CRITICAL for crossbeam (must add manually)
   - Auto-added by other transports via ServerLink

3. **Timeline Sync Components**:
   - `IsSynced<InputTimeline>` on client entity
   - `IsSynced<InterpolationTimeline>` on client entity

4. **Additional Components** (optional verification):
   - `Transport` - Transport layer initialized
   - `LocalAddr`, `PeerAddr` - Address information
   - `LocalId`, `RemoteId` - Entity IDs
   - `ReplicationSender`, `ReplicationReceiver` - For entity replication

### Current Test Gaps

Comparing [crates/server/tests/integration.rs:151-219](crates/server/tests/integration.rs#L151-L219) to lightyear patterns:

**Missing**:
1. Time management setup (`TimeUpdateStrategy::ManualInstant`)
2. `Start` trigger on server entity
3. `Connect` trigger on client entity
4. Connection establishment loop checking `Connected`
5. Synchronization loop checking `IsSynced` components
6. `Linked` component verification (critical for crossbeam)
7. Message passing tests with buffer observer pattern
8. Bidirectional message exchange verification

**Present**:
- Crossbeam pair creation ✓
- Server app setup with crossbeam transport ✓
- Client app setup with crossbeam transport ✓
- Basic entity spawning verification ✓

## Code References

### Implementation Files
- [crates/client/src/network.rs:2](crates/client/src/network.rs#L2) - CrossbeamIo import
- [crates/client/src/network.rs:18](crates/client/src/network.rs#L18) - ClientTransport::Crossbeam variant
- [crates/client/src/network.rs:114-116](crates/client/src/network.rs#L114-L116) - Crossbeam client setup
- [crates/server/src/network.rs:19-22](crates/server/src/network.rs#L19-L22) - ServerTransport enum with Crossbeam
- [crates/server/src/network.rs:175-190](crates/server/src/network.rs#L175-L190) - Crossbeam server spawning

### Test Files
- [crates/server/tests/integration.rs:151-219](crates/server/tests/integration.rs#L151-L219) - Current plugin initialization test
- [crates/server/tests/connection_flow.rs:41-43](crates/server/tests/connection_flow.rs#L41-L43) - Note about stepper pattern

### Lightyear Reference Implementation
- [git/lightyear/lightyear_tests/src/stepper.rs:36-47](git/lightyear/lightyear_tests/src/stepper.rs#L36-L47) - ClientServerStepper struct
- [git/lightyear/lightyear_tests/src/stepper.rs:142-196](git/lightyear/lightyear_tests/src/stepper.rs#L142-L196) - Server initialization
- [git/lightyear/lightyear_tests/src/stepper.rs:198-312](git/lightyear/lightyear_tests/src/stepper.rs#L198-L312) - Client initialization
- [git/lightyear/lightyear_tests/src/stepper.rs:411-452](git/lightyear/lightyear_tests/src/stepper.rs#L411-L452) - Connection init flow
- [git/lightyear/lightyear_tests/src/stepper.rs:455-465](git/lightyear/lightyear_tests/src/stepper.rs#L455-L465) - Connection verification
- [git/lightyear/lightyear_tests/src/stepper.rs:468-481](git/lightyear/lightyear_tests/src/stepper.rs#L468-L481) - Sync verification
- [git/lightyear/lightyear_tests/src/client_server/messages.rs:13-30](git/lightyear/lightyear_tests/src/client_server/messages.rs#L13-L30) - Buffer observer pattern
- [git/lightyear/lightyear_tests/src/client_server/messages.rs:32-62](git/lightyear/lightyear_tests/src/client_server/messages.rs#L32-L62) - Client-to-server test
- [git/lightyear/lightyear_tests/src/client_server/messages.rs:64-79](git/lightyear/lightyear_tests/src/client_server/messages.rs#L64-L79) - Server-to-client test

### Crossbeam Transport Implementation
- [git/lightyear/lightyear_crossbeam/src/lib.rs:38-54](git/lightyear/lightyear_crossbeam/src/lib.rs#L38-L54) - CrossbeamIo struct and new_pair
- [git/lightyear/lightyear_crossbeam/src/lib.rs:87-117](git/lightyear/lightyear_crossbeam/src/lib.rs#L87-L117) - Send/receive systems
- [git/lightyear/lightyear_crossbeam/src/lib.rs:120-132](git/lightyear/lightyear_crossbeam/src/lib.rs#L120-L132) - Plugin registration

## Architecture Documentation

### Crossbeam Transport Flow

1. **Channel Creation**: `CrossbeamIo::new_pair()` creates two unbounded bidirectional channels
2. **Entity Setup**:
   - Client app spawns client entity with one half of channel pair
   - Server app spawns ClientOf entity with other half of channel pair
3. **Link Establishment**: Both entities require `Linked` component for crossbeam
4. **Packet Flow**:
   - Lightyear systems write to `Link.send` buffer
   - `CrossbeamPlugin::send()` drains buffer, pushes to crossbeam sender channel
   - `CrossbeamPlugin::receive()` pulls from crossbeam receiver channel, pushes to `Link.recv`
   - Lightyear systems read from `Link.recv` buffer

### ClientServerStepper Test Architecture

1. **Separate Apps**: Client and server run in separate Bevy `App` instances
2. **Manual Time**: `TimeUpdateStrategy::ManualInstant` for deterministic stepping
3. **Entity Relationships**:
   - `client_entities[i]` - client entity in client app
   - `client_of_entities[i]` - server-side representation in server app
   - `server_entity` - main server entity
4. **Stepping**: `frame_step(n)` or `tick_step(n)` advances simulation deterministically
5. **Access Helpers**:
   - `client(i)` / `client_mut(i)` - access client entity
   - `client_of(i)` / `client_of_mut(i)` - access server's client representation
   - `server()` / `server_mut()` - access server entity

### Message Testing Pattern

1. **Setup Phase**:
   - Create stepper with config
   - Add buffer resource to receiving app
   - Add observer system to collect messages
2. **Send Phase**:
   - Get entity with `client_mut(i)` or `client_of_mut(i)`
   - Get `MessageSender<T>` component
   - Call `send::<ChannelType>(message)`
3. **Process Phase**:
   - Call `frame_step(1)` or `frame_step(2)` for bidirectional
4. **Verify Phase**:
   - Access buffer resource
   - Assert received messages match expected

## Implementation Decisions

Based on project requirements, the following decisions have been made for completing the crossbeam integration:

1. **Test Utils Feature**: **YES** - Enable lightyear's `test_utils` feature for crossbeam integration tests. This provides necessary test utilities and mocking capabilities.

2. **LocalAddr Component**: **NO** - ClientOf entities do not need `LocalAddr` component. Follow the stepper pattern which only adds `PeerAddr` to ClientOf entities.

3. **Auto Init**: **YES** - Use `StepperConfig { init: true }` for automatic connection and synchronization. This simplifies test setup and follows the standard pattern.

4. **Client Count**: **SINGLE-CLIENT** - Focus on single-client scenarios for crossbeam tests. Multi-client testing can be added later if needed.

5. **Entity Replication**: **NOT NOW** - Focus tests on message passing and connection verification. Entity replication testing over crossbeam transport is deferred.

6. **Timeline Verification**: **SPECIFIC SCENARIOS ONLY** - Check both `InputTimeline` and `InterpolationTimeline` sync only when relevant to the specific test case, not in all tests.

7. **Connection Timeout**: **5-TICK PREFERRED** - Use 5-tick timeout where possible for faster test execution. May extend to 50 ticks if connection takes longer in practice.

8. **Trigger/Event Tests**: **YES** - Include tests for event/trigger passing using `EventSender` and `On<RemoteEvent<T>>` pattern in addition to message passing tests.
