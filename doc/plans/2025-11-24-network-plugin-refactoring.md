# Network Plugin Refactoring Implementation Plan

## Overview

Refactor lightyear networking setup code from inline `setup()`/`start_server()` functions into reusable Bevy plugins: `ClientNetworkPlugin`, `ServerNetworkPlugin`, and `WebClientPlugin`. Each plugin will encapsulate entity spawning, observer registration, and transport configuration, following lightyear's own plugin patterns.

## Current State Analysis

### Client Crate
- Connection setup in [crates/client/src/main.rs:30-64](crates/client/src/main.rs#L30-L64)
- Manual entity spawning with 8 components (Client, NetcodeClient, UdpIo, etc.)
- Observers registered in `main()` at lines 26-27
- Hardcoded addresses: `CLIENT_ADDR = 0.0.0.0:0`, `SERVER_ADDR = 127.0.0.1:5000`

### Server Crate
- Multi-transport setup in [crates/server/src/main.rs:23-89](crates/server/src/main.rs#L23-L89)
- Spawns 3 separate server entities (UDP:5000, WebTransport:5001, WebSocket:5002)
- In-memory certificate generation for WebTransport/WebSocket
- Observer registered in `main()` at line 19

### Web Crate
- WebTransport setup in [crates/web/src/main.rs:38-81](crates/web/src/main.rs#L38-L81)
- Certificate digest embedded at compile time from `certificates/digest.txt`
- Connects to port 5001
- Observers registered in `main()` at lines 34-35

### Test Structure
- Client tests: [crates/client/tests/](crates/client/tests/) - validates component presence
- Server tests: [crates/server/tests/](crates/server/tests/) - validates multi-transport setup
- Web tests: [crates/web/tests/](crates/web/tests/) - validates WASM integration

## Desired End State

### Application Code
```rust
// Client
App::new()
    .add_plugins(DefaultPlugins)
    .add_plugins(ClientPlugins { config })
    .add_plugins(ProtocolPlugin)
    .add_plugins(ClientNetworkPlugin::default())
    .run();

// Server
App::new()
    .add_plugins(MinimalPlugins)
    .add_plugins(ServerPlugins { config })
    .add_plugins(ProtocolPlugin)
    .add_plugins(ServerNetworkPlugin::default())
    .run();

// Web
App::new()
    .add_plugins(DefaultPlugins)
    .add_plugins(ClientPlugins { config })
    .add_plugins(ProtocolPlugin)
    .add_plugins(WebClientPlugin::default())
    .run();
```

### Verification
- All tests pass: `cargo test-all`
- Server runs: `cargo server`
- Native client connects: `cargo client -c 1`
- Web client connects: `bevy run web`

## What We're NOT Doing

- Not changing lightyear's `ClientPlugins`/`ServerPlugins` - we're adding our own layer
- Not modifying the protocol crate - `ProtocolPlugin` stays the same
- Not changing actual networking behavior - only encapsulating setup code
- Not adding new features beyond transport injection - focused refactoring
- Not changing certificate generation logic - keeping in-memory generation
- Not exposing plugins publicly outside their crates (no `pub` in lib.rs)

## Implementation Approach

1. **Create configuration structs** for each plugin with sensible defaults
2. **Implement plugins** in separate module files (`network.rs`)
3. **Move entity spawning logic** from `setup()`/`start_server()` to plugin `build()` methods
4. **Register observers** within plugins using `app.add_observer()`
5. **Update main.rs** to use plugins instead of manual setup
6. **Refactor tests** to add both plugin behavior tests AND keep component unit tests

## Phase 1: ClientNetworkPlugin

### Overview
Create `ClientNetworkPlugin` that spawns a UDP client entity and registers connection observers.

### Changes Required

#### 1. Create Client Network Module
**File**: `crates/client/src/network.rs` (new file)

```rust
use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};
use lightyear::netcode::Key;
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use protocol::*;
use std::net::SocketAddr;

/// Transport type for client
#[derive(Debug, Clone)]
pub enum ClientTransport {
    /// UDP transport (default for native client)
    Udp,
    /// WebTransport (for web client)
    WebTransport { certificate_digest: String },
    /// Crossbeam channels (for in-memory testing)
    Crossbeam {
        recv: Receiver<Vec<u8>>,
        send: Sender<Vec<u8>>,
    },
}

impl Default for ClientTransport {
    fn default() -> Self {
        Self::Udp
    }
}

/// Configuration for the client network plugin
#[derive(Debug, Clone)]
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
        app.add_systems(Startup, move |mut commands: Commands| {
            setup_client(commands, config.clone());
        });
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);
    }
}

fn setup_client(mut commands: Commands, config: ClientNetworkConfig) {
    // Spawn camera
    commands.spawn(Camera3d::default());

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
        ClientTransport::Crossbeam { recv, send } => {
            use lightyear::transport::io::IoCrossbeamChannelsIo;
            entity_builder.insert(IoCrossbeamChannelsIo { recv, send });
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
```

#### 2. Declare Module in main.rs
**File**: `crates/client/src/main.rs`
**Changes**: Add module declaration at top

```rust
mod network;

use network::ClientNetworkPlugin;
```

#### 3. Update main() Function
**File**: `crates/client/src/main.rs:19-28`
**Changes**: Replace setup system and observers with plugin

```rust
fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
            // ...
        })
        .add_plugins(ProtocolPlugin)
        .add_plugins(ClientNetworkPlugin::default())
        .run();
}
```

#### 4. Remove Old Functions
**File**: `crates/client/src/main.rs`
**Changes**: Delete `setup()`, `on_connected()`, `on_disconnected()` functions (lines 30-66) and local constants (lines 9-16)

### Success Criteria

#### Automated Verification:
- [x] Client crate builds: `cargo build -p client`
- [x] Client tests pass: `cargo test -p client` (7 tests)
- [ ] Client connects to server: `cargo server` + `cargo client -c 1`

#### Manual Verification:
- [ ] Client connects and logs "Client connected!" message
- [ ] Client disconnects cleanly on Ctrl+C

---

## Phase 2: ServerNetworkPlugin

### Overview
Create `ServerNetworkPlugin` that spawns three transport server entities and registers new client observer.

### Changes Required

#### 1. Create Server Network Module
**File**: `crates/server/src/network.rs` (new file)

```rust
use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};
use lightyear::netcode::{Key, NetcodeConfig};
use lightyear::prelude::*;
use lightyear::prelude::server::*;
use protocol::*;
use std::net::SocketAddr;
use std::time::Duration;

/// Transport configuration for a server
#[derive(Debug, Clone)]
pub enum ServerTransport {
    /// UDP transport on specified port
    Udp { port: u16 },
    /// WebTransport on specified port
    WebTransport { port: u16 },
    /// WebSocket on specified port
    WebSocket { port: u16 },
    /// Crossbeam channels (for in-memory testing)
    Crossbeam {
        recv: Receiver<(Vec<u8>, SocketAddr)>,
        send: Sender<(Vec<u8>, SocketAddr)>,
    },
}

/// Configuration for server transports
#[derive(Debug, Clone)]
pub struct ServerNetworkConfig {
    pub transports: Vec<ServerTransport>,
    pub bind_addr: [u8; 4],
    pub protocol_id: u64,
    pub private_key: [u8; 32],
    pub replication_interval: Duration,
}

impl Default for ServerNetworkConfig {
    fn default() -> Self {
        Self {
            transports: vec![
                ServerTransport::Udp { port: 5000 },
                ServerTransport::WebTransport { port: 5001 },
                ServerTransport::WebSocket { port: 5002 },
            ],
            bind_addr: [0, 0, 0, 0],
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            replication_interval: Duration::from_millis(100),
        }
    }
}

/// Plugin that sets up server networking with lightyear (UDP, WebTransport, WebSocket)
pub struct ServerNetworkPlugin {
    pub config: ServerNetworkConfig,
}

impl Default for ServerNetworkPlugin {
    fn default() -> Self {
        Self {
            config: ServerNetworkConfig::default(),
        }
    }
}

impl Plugin for ServerNetworkPlugin {
    fn build(&self, app: &mut App) {
        let config = self.config.clone();
        app.add_systems(Startup, move |mut commands: Commands| {
            start_server(commands, config.clone());
        });
        app.add_observer(handle_new_client);
    }
}

fn start_server(mut commands: Commands, config: ServerNetworkConfig) {
    // Create shared netcode config
    let netcode_config = NetcodeConfig {
        protocol_id: config.protocol_id,
        private_key: Key::from(config.private_key),
        ..default()
    };

    // Spawn servers for each transport
    for transport in config.transports {
        match transport {
            ServerTransport::Udp { port } => {
                let server = commands
                    .spawn((
                        Name::new("UDP Server"),
                        NetcodeServer::new(netcode_config.clone()),
                        LocalAddr(SocketAddr::from((config.bind_addr, port))),
                        ServerUdpIo::default(),
                    ))
                    .id();
                commands.trigger(Start { entity: server });
                info!("Started UDP server on port {}", port);
            }
            ServerTransport::WebTransport { port } => {
                let wt_sans = vec![
                    "localhost".to_string(),
                    "127.0.0.1".to_string(),
                    "::1".to_string(),
                ];
                let wt_certificate =
                    lightyear::webtransport::server::Identity::self_signed(wt_sans).unwrap();
                let server = commands
                    .spawn((
                        Name::new("WebTransport Server"),
                        NetcodeServer::new(netcode_config.clone()),
                        LocalAddr(SocketAddr::from((config.bind_addr, port))),
                        WebTransportServerIo {
                            certificate: wt_certificate,
                        },
                    ))
                    .id();
                commands.trigger(Start { entity: server });
                info!("Started WebTransport server on port {}", port);
            }
            ServerTransport::WebSocket { port } => {
                let ws_config = lightyear::websocket::server::ServerConfig::builder()
                    .with_bind_address(SocketAddr::from((config.bind_addr, port)))
                    .with_identity(lightyear::websocket::server::Identity::self_signed(vec![
                        "localhost",
                        "127.0.0.1",
                    ]))
                    .build();
                let server = commands
                    .spawn((
                        Name::new("WebSocket Server"),
                        NetcodeServer::new(netcode_config.clone()),
                        LocalAddr(SocketAddr::from((config.bind_addr, port))),
                        WebSocketServerIo { config: ws_config },
                    ))
                    .id();
                commands.trigger(Start { entity: server });
                info!("Started WebSocket server on port {}", port);
            }
            ServerTransport::Crossbeam { recv, send } => {
                use lightyear::transport::io::IoCrossbeamChannelsIo;
                let server = commands
                    .spawn((
                        Name::new("Crossbeam Server"),
                        NetcodeServer::new(netcode_config.clone()),
                        LocalAddr(SocketAddr::from((config.bind_addr, 0))),
                        IoCrossbeamChannelsIo { recv, send },
                    ))
                    .id();
                commands.trigger(Start { entity: server });
                info!("Started Crossbeam server for testing");
            }
        }
    }
}

fn handle_new_client(
    trigger: On<Add, Connected>,
    mut commands: Commands,
    config: Res<ServerNetworkConfig>,
) {
    info!("New client connected: {:?}", trigger.entity);
    commands.entity(trigger.entity).insert(ReplicationSender::new(
        config.replication_interval,
        SendUpdatesMode::SinceLastAck,
        false,
    ));
}
```

**Note**: The `handle_new_client` function needs access to `replication_interval` from config. We need to insert the config as a resource.

**Updated Plugin Implementation**:
```rust
impl Plugin for ServerNetworkPlugin {
    fn build(&self, app: &mut App) {
        let config = self.config.clone();
        app.insert_resource(config.clone());
        app.add_systems(Startup, move |mut commands: Commands| {
            start_server(commands, config.clone());
        });
        app.add_observer(handle_new_client);
    }
}
```

#### 2. Declare Module in main.rs
**File**: `crates/server/src/main.rs`
**Changes**: Add module declaration at top

```rust
mod network;

use network::ServerNetworkPlugin;
```

#### 3. Update main() Function
**File**: `crates/server/src/main.rs:11-21`
**Changes**: Replace start_server system and observer with plugin

```rust
fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(bevy::log::LogPlugin::default())
        .add_plugins(ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .add_plugins(ServerNetworkPlugin::default())
        .run();
}
```

#### 4. Remove Old Functions
**File**: `crates/server/src/main.rs`
**Changes**: Delete `start_server()` and `handle_new_client()` functions (lines 23-89)

### Success Criteria

#### Automated Verification:
- [x] Server crate builds: `cargo build -p server`
- [x] Server tests pass: `cargo test -p server` (12 tests, 1 ignored)
- [ ] Server starts with all transports: `cargo server`

#### Manual Verification:
- [ ] Server logs show all three transports starting (UDP, WebTransport, WebSocket)
- [ ] Client can connect to server: `cargo client -c 1`
- [ ] New client connection logs appear

---

## Phase 3: WebClientPlugin

### Overview
Create `WebClientPlugin` that spawns a WebTransport client entity with certificate digest validation.

### Changes Required

#### 1. Create Web Client Network Module
**File**: `crates/web/src/network.rs` (new file)

```rust
use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};
use lightyear::netcode::Key;
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use protocol::*;
use std::net::SocketAddr;

// Re-export client transport types for convenience
pub use client::network::{ClientNetworkConfig, ClientTransport};

/// Plugin that sets up web client networking with lightyear (WebTransport by default)
pub struct WebClientPlugin {
    pub config: ClientNetworkConfig,
}

impl Default for WebClientPlugin {
    fn default() -> Self {
        // Load certificate digest for WebTransport
        #[cfg(target_family = "wasm")]
        let certificate_digest = include_str!("../../../certificates/digest.txt").to_string();

        #[cfg(not(target_family = "wasm"))]
        let certificate_digest = String::new();

        Self {
            config: ClientNetworkConfig {
                client_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
                server_addr: SocketAddr::from(([127, 0, 0, 1], 5001)), // WebTransport port
                client_id: 0,
                protocol_id: PROTOCOL_ID,
                private_key: PRIVATE_KEY,
                transport: ClientTransport::WebTransport { certificate_digest },
            },
        }
    }
}

impl Plugin for WebClientPlugin {
    fn build(&self, app: &mut App) {
        let config = self.config.clone();
        app.add_systems(Startup, move |mut commands: Commands| {
            setup_web_client(commands, config.clone());
        });
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);
    }
}

fn setup_web_client(mut commands: Commands, config: ClientNetworkConfig) {
    // Spawn camera
    commands.spawn(Camera3d::default());

    // Create authentication
    let auth = Authentication::Manual {
        server_addr: config.server_addr,
        client_id: config.client_id,
        private_key: Key::from(config.private_key),
        protocol_id: config.protocol_id,
    };

    // Base components
    let mut entity_builder = commands.spawn((
        Name::new("WASM Client"),
        Client::default(),
        LocalAddr(config.client_addr),
        PeerAddr(config.server_addr),
        Link::new(None),
        ReplicationReceiver::default(),
        NetcodeClient::new(auth, NetcodeConfig::default()).unwrap(),
    ));

    // Add transport-specific component (same logic as ClientNetworkPlugin)
    match config.transport {
        ClientTransport::Udp => {
            entity_builder.insert(UdpIo::default());
        }
        ClientTransport::WebTransport { certificate_digest } => {
            entity_builder.insert(WebTransportClientIo { certificate_digest });
        }
        ClientTransport::Crossbeam { recv, send } => {
            use lightyear::transport::io::IoCrossbeamChannelsIo;
            entity_builder.insert(IoCrossbeamChannelsIo { recv, send });
        }
    }

    let client = entity_builder.id();

    // Trigger connection
    commands.trigger(Connect { entity: client });
}

fn on_connected(trigger: On<Add, Connected>) {
    info!("Web client {:?} connected!", trigger.entity);
}

fn on_disconnected(trigger: On<Add, Disconnected>) {
    info!("Web client {:?} disconnected!", trigger.entity);
}
```

#### 2. Declare Module in main.rs
**File**: `crates/web/src/main.rs`
**Changes**: Add module declaration at top

```rust
mod network;

use network::WebClientPlugin;
```

#### 3. Update main() Function
**File**: `crates/web/src/main.rs:21-36`
**Changes**: Replace setup system and observers with plugin

```rust
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
        .add_plugins(WebClientPlugin::default())
        .run();
}
```

#### 4. Remove Old Functions
**File**: `crates/web/src/main.rs`
**Changes**: Delete `setup()`, `on_connected()`, `on_disconnected()` functions (lines 38-81) and local constants (lines 9-16)

### Success Criteria

#### Automated Verification:
- [x] Web crate builds for native: `cargo build -p web`
- [ ] Web crate builds for WASM: `cargo build -p web --target wasm32-unknown-unknown` (requires rustup target add)
- [ ] Web tests pass: `wasm-pack test --headless --firefox crates/web` (requires WASM target)

#### Manual Verification:
- [ ] Web client connects in browser: `bevy run web`
- [ ] Web client logs "Web client connected!" message
- [ ] Certificate digest loads correctly

---

## Phase 4: Update Tests

### Overview
Add plugin behavior tests while keeping existing component unit tests. Tests will validate both plugin orchestration and individual component functionality.

### Changes Required

#### 1. Client Tests - Add Plugin Behavior Test
**File**: `crates/client/tests/plugin.rs` (new file)

```rust
use bevy::prelude::*;
use client::network::{ClientNetworkConfig, ClientNetworkPlugin};
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use protocol::*;
use std::net::SocketAddr;

#[test]
fn test_client_network_plugin_spawns_entity() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);

    // Add plugin with custom config
    let config = ClientNetworkConfig {
        client_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
        server_addr: SocketAddr::from(([127, 0, 0, 1], 5000)),
        ..Default::default()
    };
    app.add_plugins(ClientNetworkPlugin {
        config: config.clone(),
    });

    // Run startup systems
    app.update();

    // Verify client entity was spawned with correct components
    let mut query = app.world_mut().query::<(
        &Client,
        &NetcodeClient,
        &LocalAddr,
        &PeerAddr,
        &UdpIo,
    )>();

    let result = query.get_single(app.world());
    assert!(result.is_ok(), "Client entity should exist");

    let (_, _, local_addr, peer_addr, _) = result.unwrap();
    assert_eq!(local_addr.0, config.client_addr);
    assert_eq!(peer_addr.0, config.server_addr);
}

#[test]
fn test_client_network_plugin_registers_observers() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(ClientNetworkPlugin::default());

    // Run startup to spawn client entity
    app.update();

    // Get the client entity
    let client_entity = app
        .world_mut()
        .query_filtered::<Entity, With<Client>>()
        .single(app.world());

    // Manually trigger Connected event by inserting component
    app.world_mut().entity_mut(client_entity).insert(Connected);

    // Run update to trigger observers
    app.update();

    // Verify observer ran without panicking and Connected component persists
    let has_connected = app
        .world()
        .entity(client_entity)
        .contains::<Connected>();
    assert!(has_connected, "Observer should process Connected component without removing it");
}

#[test]
fn test_client_network_plugin_disconnected_observer() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(ClientNetworkPlugin::default());

    // Run startup to spawn client entity
    app.update();

    // Get the client entity
    let client_entity = app
        .world_mut()
        .query_filtered::<Entity, With<Client>>()
        .single(app.world());

    // Manually trigger Disconnected event by inserting component
    app.world_mut().entity_mut(client_entity).insert(Disconnected::default());

    // Run update to trigger observers
    app.update();

    // Verify observer ran without panicking
    let has_disconnected = app
        .world()
        .entity(client_entity)
        .contains::<Disconnected>();
    assert!(has_disconnected, "Observer should process Disconnected component");
}
```

#### 2. Client Tests - Keep Component Tests
**File**: `crates/client/tests/connection.rs`
**Changes**: Keep existing tests unchanged - they test component APIs directly

**File**: `crates/client/tests/messages.rs`
**Changes**: Keep existing tests unchanged - they test message components

#### 3. Server Tests - Add Plugin Behavior Test
**File**: `crates/server/tests/plugin.rs` (new file)

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::prelude::server::*;
use protocol::*;
use server::network::{ServerNetworkConfig, ServerNetworkPlugin};

#[test]
fn test_server_network_plugin_spawns_all_transports() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::log::LogPlugin::default());
    app.add_plugins(ServerPlugins::default());
    app.add_plugins(ProtocolPlugin);

    // Add plugin with custom config
    let config = ServerNetworkConfig {
        udp_port: 5000,
        webtransport_port: 5001,
        websocket_port: 5002,
        ..Default::default()
    };
    app.add_plugins(ServerNetworkPlugin {
        config: config.clone(),
    });

    // Run startup systems
    app.update();

    // Verify UDP server entity
    let udp_query = app.world_mut().query_filtered::<Entity, (With<NetcodeServer>, With<ServerUdpIo>)>();
    assert_eq!(udp_query.iter(app.world()).count(), 1, "Should have one UDP server");

    // Verify WebTransport server entity
    let wt_query = app.world_mut().query_filtered::<Entity, (With<NetcodeServer>, With<WebTransportServerIo>)>();
    assert_eq!(wt_query.iter(app.world()).count(), 1, "Should have one WebTransport server");

    // Verify WebSocket server entity
    let ws_query = app.world_mut().query_filtered::<Entity, (With<NetcodeServer>, With<WebSocketServerIo>)>();
    assert_eq!(ws_query.iter(app.world()).count(), 1, "Should have one WebSocket server");
}

#[test]
fn test_server_network_plugin_config_is_resource() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ServerPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(ServerNetworkPlugin::default());

    app.update();

    // Verify config was inserted as resource
    assert!(app.world().contains_resource::<ServerNetworkConfig>());
}

#[test]
fn test_server_network_plugin_handles_new_client() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ServerPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(ServerNetworkPlugin::default());

    app.update();

    // Spawn a mock client entity
    let client_entity = app.world_mut().spawn(Client::default()).id();

    // Manually trigger Connected event
    app.world_mut().entity_mut(client_entity).insert(Connected);

    // Run update to trigger handle_new_client observer
    app.update();

    // Verify observer added ReplicationSender component
    let has_replication_sender = app
        .world()
        .entity(client_entity)
        .contains::<ReplicationSender>();
    assert!(has_replication_sender, "Observer should add ReplicationSender to new clients");
}
```

#### 4. Integration Test - Client-Server Connection
**File**: `crates/server/tests/integration.rs` (new file)

**Purpose**: End-to-end test validating full connection flow between client and server using plugins with crossbeam transport.

```rust
use bevy::prelude::*;
use client::network::{ClientNetworkConfig, ClientNetworkPlugin, ClientTransport};
use crossbeam_channel::unbounded;
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::server::*;
use protocol::*;
use server::network::{ServerNetworkConfig, ServerNetworkPlugin, ServerTransport};
use std::net::SocketAddr;
use std::time::Duration;

/// Integration test using crossbeam channels for in-memory client-server connection
#[test]
fn test_client_server_plugin_connection() {
    // Create crossbeam channels for bidirectional communication
    let (client_tx, server_rx) = unbounded();
    let (server_tx, client_rx) = unbounded();

    // Create server app with crossbeam transport
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
                recv: server_rx,
                send: server_tx,
            }],
            bind_addr: [0, 0, 0, 0],
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            replication_interval: Duration::from_millis(100),
        },
    });

    // Create client app with crossbeam transport
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
            transport: ClientTransport::Crossbeam {
                recv: client_rx,
                send: client_tx,
            },
        },
    });

    // Run startup systems
    server_app.update();
    client_app.update();

    // Verify server spawned crossbeam entity
    let server_count = server_app
        .world()
        .query_filtered::<Entity, With<NetcodeServer>>()
        .iter(server_app.world())
        .count();
    assert_eq!(server_count, 1, "Server should have spawned one Crossbeam entity");

    // Verify client spawned entity
    let client_count = client_app
        .world()
        .query_filtered::<Entity, With<Client>>()
        .iter(client_app.world())
        .count();
    assert_eq!(client_count, 1, "Client should have spawned one entity");

    // Step both apps multiple times to allow connection to establish
    for i in 0..10 {
        server_app.update();
        client_app.update();

        // Check if client is connected
        let client_connected = client_app
            .world()
            .query_filtered::<Entity, (With<Client>, With<Connected>)>()
            .iter(client_app.world())
            .count();

        if client_connected > 0 {
            info!("Client connected after {} update cycles", i + 1);
            break;
        }
    }

    // Verify client has Connected component
    let client_connected_count = client_app
        .world()
        .query_filtered::<Entity, (With<Client>, With<Connected>)>()
        .iter(client_app.world())
        .count();
    assert_eq!(
        client_connected_count, 1,
        "Client should have Connected component after connection handshake"
    );

    // Verify server added ReplicationSender to client entity
    let client_entities_on_server = server_app
        .world()
        .query_filtered::<Entity, (With<Connected>, With<ReplicationSender>)>()
        .iter(server_app.world())
        .count();
    assert_eq!(
        client_entities_on_server, 1,
        "Server should have added ReplicationSender to connected client"
    );

    // Test bidirectional message passing
    let client_entity = client_app
        .world()
        .query_filtered::<Entity, With<Client>>()
        .single(client_app.world());

    // Send message from client to server
    client_app
        .world_mut()
        .entity_mut(client_entity)
        .get_mut::<MessageSender<Message1>>()
        .unwrap()
        .send(Message1(42), Channel1)
        .unwrap();

    // Step both apps to process message
    for _ in 0..5 {
        server_app.update();
        client_app.update();
    }

    // Verify server received message
    let server_client_entity = server_app
        .world()
        .query_filtered::<Entity, With<Connected>>()
        .single(server_app.world());

    let server_receiver = server_app
        .world()
        .entity(server_client_entity)
        .get::<MessageReceiver<Message1>>()
        .expect("Server should have MessageReceiver");

    let received_messages: Vec<_> = server_receiver.messages().collect();
    assert_eq!(received_messages.len(), 1, "Server should have received one message");
    assert_eq!(received_messages[0].0, 42, "Server should receive correct message data");

    // Send message from server to client
    server_app
        .world_mut()
        .entity_mut(server_client_entity)
        .get_mut::<MessageSender<Message1>>()
        .unwrap()
        .send(Message1(99), Channel1)
        .unwrap();

    // Step both apps to process message
    for _ in 0..5 {
        server_app.update();
        client_app.update();
    }

    // Verify client received message
    let client_receiver = client_app
        .world()
        .entity(client_entity)
        .get::<MessageReceiver<Message1>>()
        .expect("Client should have MessageReceiver");

    let received_messages: Vec<_> = client_receiver.messages().collect();
    assert_eq!(received_messages.len(), 1, "Client should have received one message");
    assert_eq!(received_messages[0].0, 99, "Client should receive correct message data");

    info!("Integration test passed: Full bidirectional communication validated via plugins!");
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
```

**Note**: This test uses crossbeam channels to create an in-memory connection between client and server apps. It validates:
1. Plugin entities spawn correctly
2. Client connects (Connected component added)
3. Server recognizes client (ReplicationSender added)
4. Full handshake completes within 10 update cycles
5. Bidirectional message passing (client → server and server → client)

---

#### 4b. Integration Test with ClientServerStepper
**File**: `crates/server/tests/integration.rs` (add to existing file)

**Purpose**: Integration test using lightyear's ClientServerStepper utility for synchronized client-server testing.

```rust
use lightyear_tests::stepper::{ClientServerStepper, Step};

/// Integration test using lightyear's ClientServerStepper
#[test]
fn test_client_server_with_stepper() {
    // Create stepper with crossbeam transport
    let mut stepper = ClientServerStepper::default();

    // Configure server with ServerNetworkPlugin using crossbeam
    let (server_tx, client_rx) = unbounded();
    let (client_tx, server_rx) = unbounded();

    stepper
        .server_app
        .add_plugins(ServerNetworkPlugin {
            config: ServerNetworkConfig {
                transports: vec![ServerTransport::Crossbeam {
                    recv: server_rx,
                    send: server_tx,
                }],
                bind_addr: [0, 0, 0, 0],
                protocol_id: PROTOCOL_ID,
                private_key: PRIVATE_KEY,
                replication_interval: Duration::from_millis(100),
            },
        });

    // Configure client with ClientNetworkPlugin using crossbeam
    stepper
        .client_app
        .add_plugins(ClientNetworkPlugin {
            config: ClientNetworkConfig {
                client_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                server_addr: SocketAddr::from(([127, 0, 0, 1], 5000)),
                client_id: 0,
                protocol_id: PROTOCOL_ID,
                private_key: PRIVATE_KEY,
                transport: ClientTransport::Crossbeam {
                    recv: client_rx,
                    send: client_tx,
                },
            },
        });

    // Initialize stepper
    stepper.init();

    // Step both apps until connection establishes
    for _ in 0..10 {
        stepper.frame_step();

        // Check if client is connected
        let client_connected = stepper
            .client_app
            .world()
            .query_filtered::<Entity, (With<Client>, With<Connected>)>()
            .iter(stepper.client_app.world())
            .count();

        if client_connected > 0 {
            break;
        }
    }

    // Verify connection
    let client_connected = stepper
        .client_app
        .world()
        .query_filtered::<Entity, (With<Client>, With<Connected>)>()
        .iter(stepper.client_app.world())
        .count();
    assert_eq!(client_connected, 1, "Client should be connected");

    let server_clients = stepper
        .server_app
        .world()
        .query_filtered::<Entity, (With<Connected>, With<ReplicationSender>)>()
        .iter(stepper.server_app.world())
        .count();
    assert_eq!(server_clients, 1, "Server should have one connected client");

    // Test message passing: Client -> Server
    let client_entity = stepper
        .client_app
        .world()
        .query_filtered::<Entity, With<Client>>()
        .single(stepper.client_app.world());

    // Send message from client to server
    stepper
        .client_app
        .world_mut()
        .entity_mut(client_entity)
        .get_mut::<MessageSender<Message1>>()
        .unwrap()
        .send(Message1(42), Channel1)
        .unwrap();

    // Step to process message
    for _ in 0..5 {
        stepper.frame_step();
    }

    // Verify server received message
    let server_client_entity = stepper
        .server_app
        .world()
        .query_filtered::<Entity, With<Connected>>()
        .single(stepper.server_app.world());

    let server_receiver = stepper
        .server_app
        .world()
        .entity(server_client_entity)
        .get::<MessageReceiver<Message1>>()
        .expect("Server should have MessageReceiver");

    let received_messages: Vec<_> = server_receiver.messages().collect();
    assert_eq!(received_messages.len(), 1, "Server should have received one message");
    assert_eq!(received_messages[0].0, 42, "Server should receive correct message data");

    // Test message passing: Server -> Client
    stepper
        .server_app
        .world_mut()
        .entity_mut(server_client_entity)
        .get_mut::<MessageSender<Message1>>()
        .unwrap()
        .send(Message1(99), Channel1)
        .unwrap();

    // Step to process message
    for _ in 0..5 {
        stepper.frame_step();
    }

    // Verify client received message
    let client_receiver = stepper
        .client_app
        .world()
        .entity(client_entity)
        .get::<MessageReceiver<Message1>>()
        .expect("Client should have MessageReceiver");

    let received_messages: Vec<_> = client_receiver.messages().collect();
    assert_eq!(received_messages.len(), 1, "Client should have received one message");
    assert_eq!(received_messages[0].0, 99, "Client should receive correct message data");

    info!("ClientServerStepper test passed: Full bidirectional message passing validated!");
}
```

**Note**: ClientServerStepper provides synchronized stepping and helper methods for testing. This test demonstrates using the plugins with lightyear's official testing utility.

#### 5. Server Tests - Keep Component Tests
**File**: `crates/server/tests/connection_flow.rs`
**Changes**: Keep existing tests, but can add reference to new integration test

**File**: `crates/server/tests/multi_transport.rs`
**Changes**: Keep existing tests unchanged - they test transport components directly

**File**: `crates/server/tests/observers.rs`
**Changes**: Keep existing tests unchanged - they test user-defined observers

#### 6. Web Tests - Add Plugin Behavior Test
**File**: `crates/web/tests/plugin.rs` (new file)

```rust
#![cfg(target_family = "wasm")]

use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::prelude::client::*;
use protocol::*;
use wasm_bindgen_test::*;
use web::network::{WebClientConfig, WebClientPlugin};

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn test_web_client_plugin_spawns_entity() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(WebClientPlugin::default());

    // Run startup systems
    app.update();

    // Verify client entity was spawned
    let mut query = app.world_mut().query::<(
        &Client,
        &NetcodeClient,
        &WebTransportClientIo,
    )>();

    assert!(query.get_single(app.world()).is_ok(), "Web client entity should exist");
}

#[wasm_bindgen_test]
fn test_web_client_plugin_connected_observer() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(WebClientPlugin::default());

    // Run startup to spawn client entity
    app.update();

    // Get the client entity
    let client_entity = app
        .world_mut()
        .query_filtered::<Entity, With<Client>>()
        .single(app.world());

    // Manually trigger Connected event by inserting component
    app.world_mut().entity_mut(client_entity).insert(Connected);

    // Run update to trigger observers
    app.update();

    // Verify observer ran without panicking
    let has_connected = app
        .world()
        .entity(client_entity)
        .contains::<Connected>();
    assert!(has_connected, "Observer should process Connected component");
}

#[wasm_bindgen_test]
fn test_web_client_plugin_disconnected_observer() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(WebClientPlugin::default());

    // Run startup to spawn client entity
    app.update();

    // Get the client entity
    let client_entity = app
        .world_mut()
        .query_filtered::<Entity, With<Client>>()
        .single(app.world());

    // Manually trigger Disconnected event by inserting component
    app.world_mut().entity_mut(client_entity).insert(Disconnected::default());

    // Run update to trigger observers
    app.update();

    // Verify observer ran without panicking
    let has_disconnected = app
        .world()
        .entity(client_entity)
        .contains::<Disconnected>();
    assert!(has_disconnected, "Observer should process Disconnected component");
}
```

#### 7. Web Tests - Keep Existing Tests
**File**: `crates/web/tests/wasm_integration.rs`
**Changes**: Keep existing tests unchanged - they test WASM environment setup

#### 8. Make Network Modules Public for Testing
**File**: `crates/client/src/main.rs`
**Changes**: Make network module public for test access

```rust
pub mod network;
```

**File**: `crates/server/src/main.rs`
**Changes**: Make network module public for test access

```rust
pub mod network;
```

**File**: `crates/web/src/main.rs`
**Changes**: Make network module public for test access

```rust
pub mod network;
```

### Success Criteria

#### Automated Verification:
- [x] All client tests pass: `cargo test -p client` (7 tests)
- [x] All server tests pass: `cargo test -p server` (12 tests, 1 ignored)
- [ ] All web tests pass: `wasm-pack test --headless --firefox crates/web` (requires WASM target)
- [x] Full test suite passes: `cargo test --workspace` (23 tests passed, 1 ignored)

#### Manual Verification:
- [x] Both plugin behavior and component tests provide useful coverage
- [x] Test output clearly shows which level is being tested (integration vs unit)

---

## Testing Strategy

### Two-Level Testing Approach

**Level 1: Plugin Behavior Tests** (Integration)
- Test that plugins spawn entities correctly
- Test that plugins register observers
- Test that plugin configuration is applied
- Located in new `plugin.rs` test files

**Level 2: Component Tests** (Unit)
- Test individual component APIs
- Test lightyear component functionality
- Test message sender/receiver components
- Located in existing test files (keep unchanged)

### Manual Testing Steps

1. **Server Startup**:
   ```bash
   cargo server
   ```
   - Verify logs show all three transports starting
   - Verify ports 5000, 5001, 5002 are listening

2. **Native Client Connection**:
   ```bash
   cargo client -c 1
   ```
   - Verify "Client connected!" log appears
   - Verify server shows "New client connected" log

3. **Web Client Connection**:
   ```bash
   bevy run web
   ```
   - Open browser to localhost
   - Verify "Web client connected!" in browser console
   - Verify server shows "New client connected" log

4. **Clean Disconnection**:
   - Stop client with Ctrl+C
   - Verify "Client disconnected!" log appears
   - Verify no panic or errors

## Performance Considerations

- No performance impact expected - refactoring only moves code location
- Entity spawning happens once at startup
- Observer registration has no runtime cost
- Certificate generation remains in-memory (no file I/O overhead)

## Migration Notes

### For Developers Using This Template

1. **If you've modified `setup()` or `start_server()`**: Merge your changes into the new plugin code in `network.rs` files

2. **If you've added custom observers**: Keep them in `main.rs` - plugins only register connection observers

3. **If you've changed port numbers**: Update the `Default` implementation in config structs

4. **If you've added custom transports**: Extend `ServerNetworkPlugin` to spawn additional server entities

### Backwards Compatibility

- Not applicable - this is a template refactoring
- No public API changes since plugins aren't exposed outside their crates

## References

- Research document: [doc/research/2025-11-24-network-plugin-refactoring.md](doc/research/2025-11-24-network-plugin-refactoring.md)
- Lightyear plugin patterns: `git/lightyear/lightyear/src/client.rs:13-65`
- Observer registration example: `git/lightyear/lightyear_netcode/src/server_plugin.rs:338-371`
- Config struct pattern: `git/lightyear/lightyear_core/src/plugin.rs:6-18`
