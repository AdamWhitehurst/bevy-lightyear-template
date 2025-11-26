---
date: 2025-11-24 08:06:24 PST
researcher: Adam Whitehurst
git_commit: 9b63ac901e9883984d8e6286e721bf7761c55a48
branch: master
repository: bevy-lightyear-template
topic: "Network Plugin Refactoring - Current Lightyear Connection Code Structure"
tags: [research, codebase, networking, lightyear, plugins, client, server, web, testing]
status: complete
last_updated: 2025-11-24
last_updated_by: Adam Whitehurst
---

# Research: Network Plugin Refactoring - Current Lightyear Connection Code Structure

**Date**: 2025-11-24 08:06:24 PST
**Researcher**: Adam Whitehurst
**Git Commit**: 9b63ac901e9883984d8e6286e721bf7761c55a48
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question
Document the current structure of lightyear connection code across client, server, and web crates to understand what networking logic exists and where it lives, along with existing Bevy plugin patterns and connection validation tests.

## Summary
The codebase currently implements lightyear networking directly in the `main.rs` files of each crate (client, server, web) within `setup()` or `start_server()` functions. All three crates use lightyear's `ClientPlugins` or `ServerPlugins` along with a shared `ProtocolPlugin`, but the connection setup code (transport configuration, authentication, entity spawning) is not encapsulated in custom plugins. Tests validate component setup and entity creation but do not yet test actual network message passing between components.

## Detailed Findings

### Client Crate (`/home/aw/Dev/bevy-lightyear-template/crates/client/`)

#### Current Connection Setup
**File**: `crates/client/src/main.rs`

**Application Structure** (lines 19-28):
```rust
fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .add_systems(Startup, setup)
        .add_observer(on_connected)
        .add_observer(on_disconnected)
        .run();
}
```

**Connection Logic** (lines 30-64 in `setup()` function):
- Uses UDP transport via `ClientUdpIo` component
- Manual authentication with:
  - Server address: `127.0.0.1:5000`
  - Client ID: Random 64-bit value
  - Protocol ID and private key from protocol crate
- Spawns client entity with components:
  - `Name("Client")`
  - `Client` (lightyear component)
  - `LocalAddr(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)))`
  - `ClientUdpIo`
- Registers observers for `Connected` and `Disconnected` events

#### Test Structure
**File**: `crates/client/tests/connection.rs`
- `test_client_connects_to_server()` (line 8): Validates client entity creation with all required components (Client, NetcodeClient, LocalAddr, PeerAddr, Link, ReplicationReceiver, UdpIo)
- `test_client_has_ping_manager()` (line 51): Validates PingManager component is added by ClientPlugins

**File**: `crates/client/tests/messages.rs`
- `test_message_sender_component()` (line 8): Validates MessageSender component
- `test_message_receiver_component()` (line 30): Validates MessageReceiver component

### Server Crate (`/home/aw/Dev/bevy-lightyear-template/crates/server/`)

#### Current Connection Setup
**File**: `crates/server/src/main.rs`

**Application Structure** (lines 11-21):
```rust
fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(bevy::log::LogPlugin::default())
        .add_plugins(ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .add_systems(Startup, start_server)
        .add_observer(handle_new_client)
        .run();
}
```

**Connection Logic** (lines 23-89 in `start_server()` function):
- Creates shared `NetcodeConfig` with protocol ID and private key
- Spawns **three separate server entities** for multi-transport support:

  1. **UDP Server** (port 5000):
     - Components: `Name("Server UDP")`, `NetcodeServer`, `LocalAddr`, `ServerUdpIo`

  2. **WebTransport Server** (port 5001):
     - Generates self-signed certificate with `rcgen`
     - Writes certificate digest to `assets/certificates/digest.txt`
     - Components: `Name("Server WebTransport")`, `NetcodeServer`, `LocalAddr`, `WebTransportServerIo` (with certificate)

  3. **WebSocket Server** (port 5002):
     - Uses same certificate as WebTransport
     - Components: `Name("Server WebSocket")`, `NetcodeServer`, `LocalAddr`, `WebSocketServerIo` (with TLS config)

- Registers observer `handle_new_client()` for client connection events

#### Test Structure
**File**: `crates/server/tests/connection_flow.rs`
- `test_server_started()` (line 9): Validates server entity creation and Started component
- `test_client_server_connection()` (line 45): Documents pattern for client-server testing (references lightyear's crossbeam transport for full integration)

**File**: `crates/server/tests/multi_transport.rs`
- `test_server_creates_udp_transport()` (line 10): Validates UDP server entity
- `test_server_creates_webtransport()` (line 40): Validates WebTransport server entity with certificate
- `test_server_creates_websocket()` (line 79): Validates WebSocket server entity with TLS

**File**: `crates/server/tests/observers.rs`
- `test_server_observer_registration()` (line 29): Validates observer system registration for connection state changes

### Web Crate (`/home/aw/Dev/bevy-lightyear-template/crates/web/`)

#### Current Connection Setup
**File**: `crates/web/src/main.rs`

**Application Structure** (lines 21-36):
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
        .add_systems(Startup, setup)
        .add_observer(on_connected)
        .add_observer(on_disconnected)
        .run();
}
```

**Connection Logic** (lines 38-81 in `setup()` function):
- Uses WebTransport transport via `WebTransportClientIo` component
- Connects to port 5001 (WebTransport server)
- Loads certificate digest from `/certificates/digest.txt` using `fetch()` API
- Manual authentication similar to native client
- Spawns client entity with components:
  - `Name("Web Client")`
  - `Client`
  - `LocalAddr`
  - `WebTransportClientIo` (with certificate digest)

#### Test Structure
**File**: `crates/web/tests/wasm_integration.rs`
- `test_wasm_panic_hook()` (line 8): Validates WASM panic hook setup
- `test_protocol_imports()` (line 14): Validates protocol message imports
- `test_bevy_minimal_app()` (line 22): Validates Bevy app initialization in WASM

### Protocol Crate (`/home/aw/Dev/bevy-lightyear-template/crates/protocol/`)

#### Shared Protocol Plugin
**File**: `crates/protocol/src/lib.rs`

**Plugin Implementation** (lines 13-28):
```rust
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        // Register message
        app.register_message::<Message1>()
            .add_direction(NetworkDirection::Bidirectional);

        // Register channel
        app.add_channel::<Channel1>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);
    }
}
```

**Shared Constants**:
- `PROTOCOL_ID: u64 = 0`
- `PRIVATE_KEY: [u8; 32]` (fixed test key)
- `FIXED_TIMESTEP_HZ: f64 = 60.0`

**Test Utilities**: `crates/protocol/src/test_utils.rs`
- `test_protocol_plugin()`: Factory function for testing
- `assert_channel_registered<C>()`: Helper for channel verification
- `assert_message_registered<M>()`: Helper for message verification

## Code References

### Client Crate
- `crates/client/src/main.rs:19-28` - Application setup
- `crates/client/src/main.rs:30-64` - Connection setup in `setup()` function
- `crates/client/tests/connection.rs:8-49` - Connection validation test
- `crates/client/tests/connection.rs:51-75` - PingManager test

### Server Crate
- `crates/server/src/main.rs:11-21` - Application setup
- `crates/server/src/main.rs:23-89` - Multi-transport server setup in `start_server()` function
- `crates/server/tests/connection_flow.rs:9-42` - Server startup test
- `crates/server/tests/multi_transport.rs:10-115` - Multi-transport validation tests

### Web Crate
- `crates/web/src/main.rs:21-36` - Application setup
- `crates/web/src/main.rs:38-81` - Web client connection setup
- `crates/web/tests/wasm_integration.rs:8-35` - WASM integration tests

### Protocol Crate
- `crates/protocol/src/lib.rs:13-28` - ProtocolPlugin implementation
- `crates/protocol/src/lib.rs:30-37` - Shared constants
- `crates/protocol/src/test_utils.rs:10-35` - Test utilities

## Architecture Documentation

### Current Plugin Usage Pattern

**Plugin Groups Used**:
1. **ClientPlugins** (from lightyear) - Used by both native client and web client
   - Implements `PluginGroup` trait
   - Configurable with `tick_duration` field
   - Bundles multiple lightyear client plugins (sync, IO, connection)

2. **ServerPlugins** (from lightyear) - Used by server
   - Implements `PluginGroup` trait
   - Configurable with `tick_duration` field
   - Bundles multiple lightyear server plugins (sync, link, IO, connection)

3. **ProtocolPlugin** (custom) - Shared across all crates
   - Simple unit struct plugin
   - Registers messages and channels for networking protocol
   - Used identically in client, server, and web

### Current Connection Flow

**Native Client**:
1. App adds `DefaultPlugins`, `ClientPlugins`, `ProtocolPlugin`
2. `Startup` system `setup()` spawns client entity with UDP transport
3. Lightyear's `ClientPlugins` handles connection lifecycle
4. Observers trigger on `Connected`/`Disconnected` events

**Server**:
1. App adds `MinimalPlugins`, `LogPlugin`, `ServerPlugins`, `ProtocolPlugin`
2. `Startup` system `start_server()` spawns three server entities (UDP, WebTransport, WebSocket)
3. Lightyear's `ServerPlugins` handles connection lifecycle for all transports
4. Observer triggers on new client connections

**Web Client**:
1. App adds `DefaultPlugins`, `ClientPlugins`, `ProtocolPlugin`
2. `Startup` system `setup()` spawns client entity with WebTransport
3. Fetches certificate digest from server via HTTP
4. Lightyear's `ClientPlugins` handles connection lifecycle
5. Observers trigger on `Connected`/`Disconnected` events

### Existing Bevy Plugin Patterns Found

From lightyear dependency (`git/lightyear/`):

**Pattern 1: Simple Unit Struct Plugin**
- No configuration needed
- Direct `impl Plugin` for unit struct
- Example: `ProtocolPlugin`, `NetcodeClientPlugin`

**Pattern 2: Configurable Plugin with Fields**
- Struct holds configuration data
- Can check for existing plugins with `is_plugin_added::<T>()`
- Example: `CorePlugins` with `tick_duration` field

**Pattern 3: Plugin Group Pattern**
- Implements `PluginGroup` trait instead of `Plugin`
- Uses `PluginGroupBuilder` to compose multiple plugins
- Heavy use of `#[cfg(feature = "...")]` for conditional compilation
- Can set individual plugins in group with `.set()` method
- Examples: `ClientPlugins`, `ServerPlugins`

**Pattern 4: Plugin with Systems and Observers**
- Systems defined as associated functions on plugin struct
- Uses `configure_sets()` for system ordering with `.chain()`
- Registers systems to specific schedules (PreUpdate, PostUpdate)
- Adds observers for lifecycle events
- Example: `NetcodeClientPlugin` (`git/lightyear/lightyear_netcode/src/client_plugin.rs:23-248`)

### Test Infrastructure

**Current Testing Approach**:
- Tests validate component presence and entity creation
- Use `MinimalPlugins` for lightweight testing
- Test each crate independently
- Do not yet test actual network message passing between crates

**Test Patterns Found**:
```rust
// Pattern A: Component validation
let mut app = App::new();
app.add_plugins(MinimalPlugins);
app.add_plugins(ClientPlugins::default());
// ... spawn entity, update app
assert!(query.get_single().is_ok());

// Pattern B: Message registration validation
let mut app = App::new();
app.add_plugins(MinimalPlugins);
app.add_plugins(ProtocolPlugin);
assert!(app.is_message_registered::<Message1>());
```

**Advanced Integration Testing**:
- Lightyear provides `ClientServerStepper` (`git/lightyear/lightyear_tests/src/stepper.rs:36`)
- Uses crossbeam channels for in-memory client-server testing
- Allows synchronized stepping of client and server apps
- Can test actual message passing (referenced in `crates/server/tests/connection_flow.rs:78-80`)

## Historical Context (from thoughts/)

No existing research documents found for this topic. This is the initial research document for network plugin refactoring.

## Related Research

None yet - this is the first research document on this topic.

## Open Questions

1. **Plugin Encapsulation Scope**: What functionality should be included in each plugin?
   - Connection setup (entity spawning, component configuration)?
   - Observer registration?
   - Certificate generation/loading?
   - Address configuration?

2. **Plugin Configuration**: What parameters should be configurable?
   - Server addresses/ports?
   - Protocol ID and private key?
   - Transport type selection?
   - Certificate handling?

3. **Test Migration**: How should tests be updated?
   - Should plugins expose test utilities?
   - Should tests use the new plugins or continue using direct setup?
   - How to test actual message passing between client/server/web?

4. **Code Location**: Where should the new plugins be defined?
   - In the same `main.rs` file?
   - In separate module files (`client/src/network.rs`, etc.)?
   - Should they be public or private to the crate?

5. **Shared Code**: Should there be shared networking utilities?
   - Common authentication setup?
   - Shared observer patterns?
   - Reusable certificate handling?
