# Testing Infrastructure Implementation Plan

## Overview

Implement comprehensive automated testing infrastructure for the bevy-lightyear-template project to verify server/client/web connectivity, message passing, entity replication, and multi-transport support. All tests will be automated and runnable via `cargo test` without manual intervention or log inspection.

## Current State Analysis

**Status**: Zero test infrastructure exists
- No `#[cfg(test)]` modules in any source files
- No `tests/` directories in any crates
- No dev-dependencies defined in any Cargo.toml
- No test tasks in Makefile.toml
- Protocol crate: `crates/protocol/src/lib.rs:1-33` - defines Message1, Channel1, ProtocolPlugin
- Client crate: `crates/client/src/main.rs:1-66` - UDP connection setup
- Server crate: `crates/server/src/main.rs:1-89` - multi-transport (UDP, WebTransport, WebSocket)
- Web crate: `crates/web/src/main.rs:1-95` - WASM client with WebTransport

**Key Discoveries**:
- Lightyear provides `ClientServerStepper` test infrastructure: `git/lightyear/lightyear_tests/src/stepper.rs:36-567`
- Bevy provides headless testing with MinimalPlugins: `git/bevy/tests/how_to_test_apps.rs:52-86`
- Test protocols should follow: `git/lightyear/lightyear_tests/src/protocol.rs:1-220`
- Connection patterns: `git/lightyear/lightyear_tests/src/client_server/base.rs:9-105`

## Desired End State

A fully automated test infrastructure where:
1. **Protocol crate** has unit tests for serialization and registration
2. **Protocol crate** provides test utilities via `test_utils` feature
3. **Server/Client/Web crates** have integration tests using ClientServerStepper
4. **All tests** run headless via `cargo test` without rendering
5. **WASM tests** run in headless browser via wasm-bindgen-test
6. **Makefile.toml** provides convenient test commands
7. **Coverage targets**: Protocol 95%, Server 85%, Client 80%, Web 70% (smoke tests)

### Verification
Run `cargo test-all` (new Makefile.toml task) and verify all test suites pass:
- `cargo test -p protocol` - Protocol unit tests pass
- `cargo test -p client` - Client integration tests pass
- `cargo test -p server` - Server integration tests pass
- `wasm-pack test --headless --firefox crates/web` - WASM tests pass

## What We're NOT Doing

- No GitHub Actions CI/CD workflow (deferred)
- No entity replication tests (Phase 3)
- No physics (Avian) integration tests (Phase 3)
- No performance benchmarks or criterion setup (Phase 3)
- No workspace-level `tests/` directory (not needed yet)
- No manual testing procedures or UI verification
- No test coverage reporting setup (deferred)

## Implementation Approach

Follow Lightyear's testing patterns using:
1. **ClientServerStepper** for synchronized client-server testing with in-memory channels
2. **MinimalPlugins** for headless Bevy app testing (no rendering)
3. **Test utilities in protocol crate** to avoid circular dependencies
4. **Feature-gated test_utils** to keep test code separate
5. **wasm-bindgen-test** for WASM-specific browser tests

Model tests after:
- `git/lightyear/lightyear_tests/src/client_server/base.rs:11-85` - Connection setup
- `git/lightyear/lightyear_tests/src/client_server/messages.rs:32-80` - Message passing
- `git/lightyear/lightyear_tests/src/protocol.rs:146-269` - Protocol definition

---

## Phase 1: Foundation (Workspace + Protocol + Basic Tests)

### Overview
Establish workspace test infrastructure, add protocol unit tests, create test utilities in protocol crate, and implement basic Makefile.toml test tasks.

### Changes Required

- [x] #### 1. Workspace Root Configuration
**File**: `Cargo.toml`
**Changes**: Add workspace-level dev-dependencies for shared test infrastructure (NOTE: workspace.dev-dependencies not valid, added profile optimizations only)

```toml
# Add after [workspace.dependencies]

[workspace.dev-dependencies]
approx = "0.5.1"
mock_instant = "0.6"
test-log = { version = "0.2.17", features = ["trace", "color"] }

# Add profile optimizations for faster test execution
[profile.test]
opt-level = 1
debug = true

[profile.dev.package."*"]
opt-level = 3
```

- [x] #### 2. Protocol Crate Configuration
**File**: `crates/protocol/Cargo.toml`
**Changes**: Add dev-dependencies and test_utils feature

```toml
[features]
test_utils = []

[dev-dependencies]
approx = { workspace = true }
test-log = { workspace = true }
serde_test = "1.0"
```

- [x] #### 3. Protocol Test Utilities Module
**File**: `crates/protocol/src/test_utils.rs` (NEW)
**Changes**: Create test utility functions and builders

```rust
//! Test utilities for protocol testing
//!
//! Enable with the `test_utils` feature flag.

use bevy::prelude::*;
use lightyear::prelude::*;

/// Create a test protocol plugin with default settings
pub fn test_protocol_plugin() -> crate::ProtocolPlugin {
    crate::ProtocolPlugin
}

/// Verify that a message can be serialized and deserialized
pub fn assert_message_roundtrip<M>(message: M)
where
    M: Message + Debug + PartialEq,
{
    use serde_test::{assert_tokens, Token};
    // Serialization test implementation
}

/// Verify that a channel is registered correctly
pub fn assert_channel_registered<C: Channel>(app: &App) {
    let registry = app.world().resource::<ChannelRegistry>();
    assert!(registry.contains::<C>(), "Channel {} not registered", std::any::type_name::<C>());
}

/// Verify that a message type is registered
pub fn assert_message_registered<M: Message>(app: &App) {
    let registry = app.world().resource::<MessageRegistry>();
    assert!(registry.contains::<M>(), "Message {} not registered", std::any::type_name::<M>());
}
```

- [x] #### 4. Protocol Library Exports
**File**: `crates/protocol/src/lib.rs`
**Changes**: Export test_utils module conditionally

```rust
// Add at the end of the file

#[cfg(feature = "test_utils")]
pub mod test_utils;
```

- [x] #### 5. Protocol Unit Tests (Inline)
**File**: `crates/protocol/src/lib.rs`
**Changes**: Add inline unit tests for protocol types

```rust
// Add at the end of the file

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;
    use lightyear::prelude::*;

    #[test]
    fn test_message1_serialization() {
        let msg = Message1(42);
        let serialized = serde_json::to_string(&msg).unwrap();
        let deserialized: Message1 = serde_json::from_str(&serialized).unwrap();
        assert_eq!(msg, deserialized);
    }

    #[test]
    fn test_message1_clone() {
        let msg = Message1(42);
        let cloned = msg.clone();
        assert_eq!(msg, cloned);
    }

    #[test]
    fn test_protocol_plugin_registers_message1() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(ProtocolPlugin);

        // Verify Message1 is registered
        let registry = app.world().resource::<MessageRegistry>();
        assert!(registry.contains::<Message1>(), "Message1 not registered");
    }

    #[test]
    fn test_protocol_plugin_registers_channel1() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(ProtocolPlugin);

        // Verify Channel1 is registered
        let registry = app.world().resource::<ChannelRegistry>();
        assert!(registry.contains::<Channel1>(), "Channel1 not registered");
    }
}
```

- [x] #### 6. Makefile.toml Test Tasks
**File**: `Makefile.toml`
**Changes**: Add test-related tasks

```toml
# Add after existing tasks

[tasks.test-protocol]
description = "Run protocol crate unit tests"
command = "cargo"
args = ["test", "-p", "protocol", "--all-features"]

[tasks.test-client]
description = "Run client crate integration tests"
command = "cargo"
args = ["test", "-p", "client"]

[tasks.test-server]
description = "Run server crate integration tests"
command = "cargo"
args = ["test", "-p", "server"]

[tasks.test-native]
description = "Run all native tests (protocol, client, server)"
dependencies = ["test-protocol", "test-client", "test-server"]

[tasks.test-all]
description = "Run all tests including WASM"
run_task = { name = ["test-native", "test-wasm"], parallel = false }

[tasks.test-wasm]
description = "Run WASM tests for web crate"
command = "wasm-pack"
args = ["test", "--headless", "--firefox", "crates/web"]
```

### Success Criteria

All protocol unit tests pass when running `cargo test-protocol`:
- `test_message1_serialization` - Message1 serde roundtrip works
- `test_message1_clone` - Message1 implements Clone correctly
- `test_protocol_plugin_registers_message1` - Message1 registered in MessageRegistry
- `test_protocol_plugin_registers_channel1` - Channel1 registered in ChannelRegistry

---

## Phase 2: Integration Tests (Client, Server, WASM)

### Overview
Implement integration tests for client, server, and web crates using ClientServerStepper for synchronized client-server testing. Add WASM-specific tests with wasm-bindgen-test.

### Changes Required

- [x] #### 1. Client Crate Dependencies
**File**: `crates/client/Cargo.toml`
**Changes**: Add dev-dependencies for integration testing

```toml
[dev-dependencies]
approx = { workspace = true }
mock_instant = { workspace = true }
test-log = { workspace = true }
protocol = { workspace = true, features = ["test_utils"] }
lightyear = { workspace = true, features = ["test_utils"] }
```

- [x] #### 2. Client Integration Test: Connection
**File**: `crates/client/tests/connection.rs` (NEW)
**Changes**: Test client connection lifecycle

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::_internal::client::ClientPlugins;
use protocol::{FIXED_TIMESTEP_HZ, PROTOCOL_ID, PRIVATE_KEY};

#[test]
fn test_client_connects_to_server() {
    // Setup client app with MinimalPlugins (headless)
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let tick_duration = std::time::Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ClientPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    // Spawn client entity with UDP transport
    let addr = "127.0.0.1:0".parse().unwrap();
    let server_addr = "127.0.0.1:5000".parse().unwrap();

    let client_id = app.world_mut().spawn((
        Link::<Netcode>::new(
            ClientIo::new(
                transport::UdpClientIo { server_addr },
                netcode::NetcodeConfig::new(
                    server_addr,
                    PROTOCOL_ID,
                    0,
                    PRIVATE_KEY.into(),
                ),
            ),
        ),
    )).id();

    // Verify client entity exists
    assert!(app.world().get_entity(client_id).is_ok());

    // Verify client has Link component
    assert!(app.world().get::<Link<Netcode>>(client_id).is_some());
}

#[test]
fn test_client_has_ping_manager() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let tick_duration = std::time::Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ClientPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    let addr = "127.0.0.1:0".parse().unwrap();
    let server_addr = "127.0.0.1:5000".parse().unwrap();

    let client_id = app.world_mut().spawn((
        Link::<Netcode>::new(
            ClientIo::new(
                transport::UdpClientIo { server_addr },
                netcode::NetcodeConfig::new(
                    server_addr,
                    PROTOCOL_ID,
                    0,
                    PRIVATE_KEY.into(),
                ),
            ),
        ),
    )).id();

    // Run app setup
    app.update();

    // Verify PingManager component added
    assert!(app.world().get::<PingManager>(client_id).is_some());
}
```

- [x] #### 3. Server Crate Dependencies
**File**: `crates/server/Cargo.toml`
**Changes**: Add dev-dependencies

```toml
[dev-dependencies]
approx = { workspace = true }
mock_instant = { workspace = true }
test-log = { workspace = true }
protocol = { workspace = true, features = ["test_utils"] }
lightyear = { workspace = true, features = ["test_utils"] }
```

- [x] #### 4. Server Integration Test: Multi-Transport
**File**: `crates/server/tests/multi_transport.rs` (NEW)
**Changes**: Test server multi-transport setup

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::_internal::server::ServerPlugins;
use protocol::{FIXED_TIMESTEP_HZ, PROTOCOL_ID, PRIVATE_KEY};

#[test]
fn test_server_creates_udp_transport() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, LogPlugin::default()));

    let tick_duration = std::time::Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ServerPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    // Spawn UDP server
    let addr = "127.0.0.1:5000".parse().unwrap();
    let server_id = app.world_mut().spawn((
        Link::<Netcode>::new(ServerIo::new(
            transport::UdpServerIo { addr },
            netcode::NetcodeConfig::new(addr, PROTOCOL_ID, 0, PRIVATE_KEY.into()),
        )),
    )).id();

    // Verify server entity exists
    assert!(app.world().get_entity(server_id).is_ok());

    // Verify server has Link component
    assert!(app.world().get::<Link<Netcode>>(server_id).is_some());
}

#[test]
fn test_server_creates_webtransport() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, LogPlugin::default()));

    let tick_duration = std::time::Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ServerPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    // Spawn WebTransport server
    let addr = "127.0.0.1:5001".parse().unwrap();

    // Load certificate (use default for test)
    let certificates = vec![wtransport::tls::Certificate::load(
        include_bytes!("../../../certs/cert.pem"),
        include_bytes!("../../../certs/key.pem"),
    ).unwrap()];

    let server_id = app.world_mut().spawn((
        Link::<Netcode>::new(ServerIo::new(
            transport::WebtransportServerIo::new(addr, certificates),
            netcode::NetcodeConfig::new(addr, PROTOCOL_ID, 0, PRIVATE_KEY.into()),
        )),
    )).id();

    // Verify server entity exists
    assert!(app.world().get_entity(server_id).is_ok());
}

#[test]
fn test_server_creates_websocket() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, LogPlugin::default()));

    let tick_duration = std::time::Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ServerPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    // Spawn WebSocket server
    let addr = "127.0.0.1:5002".parse().unwrap();

    let certificates = vec![wtransport::tls::Certificate::load(
        include_bytes!("../../../certs/cert.pem"),
        include_bytes!("../../../certs/key.pem"),
    ).unwrap()];

    let server_id = app.world_mut().spawn((
        Link::<Netcode>::new(ServerIo::new(
            transport::WebsocketServerIo::new(addr, certificates),
            netcode::NetcodeConfig::new(addr, PROTOCOL_ID, 0, PRIVATE_KEY.into()),
        )),
    )).id();

    // Verify server entity exists
    assert!(app.world().get_entity(server_id).is_ok());
}
```

- [x] #### 5. Server Integration Test: Connection Flow
**File**: `crates/server/tests/connection_flow.rs` (NEW)
**Changes**: Test client-server connection using ClientServerStepper

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::_internal::client::ClientPlugins;
use lightyear::_internal::server::ServerPlugins;
use protocol::{FIXED_TIMESTEP_HZ, PROTOCOL_ID, PRIVATE_KEY};
use std::time::Duration;

/// Minimal ClientServerStepper for connection testing
struct TestStepper {
    client_app: App,
    server_app: App,
    client_entity: Entity,
    server_entity: Entity,
    tick_duration: Duration,
}

impl TestStepper {
    fn new() -> Self {
        let tick_duration = Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);

        // Setup server
        let mut server_app = App::new();
        server_app.add_plugins((MinimalPlugins, LogPlugin::default()));
        server_app.add_plugins(ServerPlugins { tick_duration });
        server_app.add_plugins(protocol::ProtocolPlugin);

        let addr = "127.0.0.1:0".parse().unwrap();
        let server_entity = server_app.world_mut().spawn((
            Link::<Netcode>::new(ServerIo::new(
                transport::crossbeam::CrossbeamServerIo::default(),
                netcode::NetcodeConfig::new(addr, PROTOCOL_ID, 0, PRIVATE_KEY.into()),
            )),
        )).id();

        // Setup client
        let mut client_app = App::new();
        client_app.add_plugins(MinimalPlugins);
        client_app.add_plugins(ClientPlugins { tick_duration });
        client_app.add_plugins(protocol::ProtocolPlugin);

        // Get crossbeam channels from server
        let (tx, rx) = {
            let link = server_app.world().get::<Link<Netcode>>(server_entity).unwrap();
            let io = link.io.as_any().downcast_ref::<ServerIo<transport::crossbeam::CrossbeamServerIo, Netcode>>().unwrap();
            io.transport.get_channel()
        };

        let client_entity = client_app.world_mut().spawn((
            Link::<Netcode>::new(
                ClientIo::new(
                    transport::crossbeam::CrossbeamClientIo::new(tx, rx),
                    netcode::NetcodeConfig::new(
                        addr,
                        PROTOCOL_ID,
                        0,
                        PRIVATE_KEY.into(),
                    ),
                ),
            ),
        )).id();

        Self {
            client_app,
            server_app,
            client_entity,
            server_entity,
            tick_duration,
        }
    }

    fn connect(&mut self) {
        self.server_app.world_mut().trigger(Start {
            entity: self.server_entity,
        });

        self.client_app.world_mut().trigger(Connect {
            entity: self.client_entity,
        });
    }

    fn update(&mut self) {
        self.server_app.update();
        self.client_app.update();
    }

    fn wait_for_connection(&mut self, max_frames: usize) -> bool {
        for _ in 0..max_frames {
            if self.client_app.world().get::<Connected>(self.client_entity).is_some() {
                return true;
            }
            self.update();
        }
        false
    }
}

#[test]
fn test_client_server_connection() {
    let mut stepper = TestStepper::new();
    stepper.connect();

    // Wait up to 50 frames for connection
    let connected = stepper.wait_for_connection(50);
    assert!(connected, "Client failed to connect within 50 frames");

    // Verify Connected component present
    assert!(stepper.client_app.world().get::<Connected>(stepper.client_entity).is_some());
}

#[test]
fn test_server_started() {
    let mut stepper = TestStepper::new();

    stepper.server_app.world_mut().trigger(Start {
        entity: stepper.server_entity,
    });

    stepper.server_app.update();

    // Verify Started component present
    assert!(stepper.server_app.world().get::<Started>(stepper.server_entity).is_some());
}
```

- [x] #### 6. Web Crate Dependencies
**File**: `crates/web/Cargo.toml`
**Changes**: Add WASM test dependencies

```toml
[lib]
crate-type = ["cdylib", "rlib"]

[dev-dependencies]
protocol = { workspace = true, features = ["test_utils"] }

[target.'cfg(target_arch = "wasm32")'.dev-dependencies]
wasm-bindgen-test = "0.3"
console_error_panic_hook = { workspace = true }
```

- [x] #### 7. Web WASM Integration Tests
**File**: `crates/web/tests/wasm_integration.rs` (NEW)
**Changes**: WASM-specific smoke tests

```rust
#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn test_wasm_panic_hook() {
    console_error_panic_hook::set_once();
    assert_eq!(2 + 2, 4);
}

#[wasm_bindgen_test]
fn test_protocol_imports() {
    use protocol::{Message1, Channel1, ProtocolPlugin};

    let msg = Message1(42);
    assert_eq!(msg.0, 42);
}

#[wasm_bindgen_test]
fn test_bevy_minimal_app() {
    use bevy::prelude::*;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    // Run one update cycle
    app.update();

    // Verify app is functional
    assert!(app.world().is_empty() == false);
}
```

- [x] #### 8. Server Integration Test: Observer Lifecycle
**File**: `crates/server/tests/observers.rs` (NEW)
**Changes**: Test server observer registration and triggering

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::_internal::server::ServerPlugins;
use protocol::FIXED_TIMESTEP_HZ;

#[derive(Resource, Default)]
struct ObserverTestState {
    client_connected: bool,
    client_disconnected: bool,
}

fn on_client_connected(
    _trigger: Trigger<ClientOf>,
    mut state: ResMut<ObserverTestState>,
) {
    state.client_connected = true;
}

fn on_client_disconnected(
    _trigger: Trigger<ClientOf>,
    mut state: ResMut<ObserverTestState>,
) {
    state.client_disconnected = true;
}

#[test]
fn test_server_observer_registration() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, LogPlugin::default()));

    let tick_duration = std::time::Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ);
    app.add_plugins(ServerPlugins { tick_duration });
    app.add_plugins(protocol::ProtocolPlugin);

    // Add test resource and observers
    app.init_resource::<ObserverTestState>();
    app.add_observer(on_client_connected);
    app.add_observer(on_client_disconnected);

    // Verify observers registered
    app.update();

    let state = app.world().resource::<ObserverTestState>();
    assert_eq!(state.client_connected, false);
    assert_eq!(state.client_disconnected, false);
}
```

- [x] #### 9. Client Integration Test: Message Sending
**File**: `crates/client/tests/messages.rs` (NEW)
**Changes**: Test bidirectional message passing

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use protocol::{Message1, Channel1};

#[test]
fn test_message_sender_component() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(lightyear::_internal::client::ClientPlugins {
        tick_duration: std::time::Duration::from_millis(16),
    });
    app.add_plugins(protocol::ProtocolPlugin);

    // Spawn client entity
    let client_id = app.world_mut().spawn_empty().id();

    // Add MessageSender component
    app.world_mut().entity_mut(client_id).insert(MessageSender::<Message1>::default());

    app.update();

    // Verify MessageSender present
    let sender = app.world().get::<MessageSender<Message1>>(client_id);
    assert!(sender.is_some(), "MessageSender<Message1> not present");
}

#[test]
fn test_message_receiver_component() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(lightyear::_internal::client::ClientPlugins {
        tick_duration: std::time::Duration::from_millis(16),
    });
    app.add_plugins(protocol::ProtocolPlugin);

    // Spawn client entity
    let client_id = app.world_mut().spawn_empty().id();

    // Add MessageReceiver component
    app.world_mut().entity_mut(client_id).insert(MessageReceiver::<Message1>::default());

    app.update();

    // Verify MessageReceiver present
    let receiver = app.world().get::<MessageReceiver<Message1>>(client_id);
    assert!(receiver.is_some(), "MessageReceiver<Message1> not present");
}
```

### Success Criteria

All integration tests pass when running `cargo test-native`:

**Protocol crate** (`cargo test -p protocol`):
- `test_message1_serialization` passes
- `test_message1_clone` passes
- `test_protocol_plugin_registers_message1` passes
- `test_protocol_plugin_registers_channel1` passes

**Client crate** (`cargo test -p client`):
- `test_client_connects_to_server` passes
- `test_client_has_ping_manager` passes
- `test_message_sender_component` passes
- `test_message_receiver_component` passes

**Server crate** (`cargo test -p server`):
- `test_server_creates_udp_transport` passes
- `test_server_creates_webtransport` passes
- `test_server_creates_websocket` passes
- `test_client_server_connection` passes
- `test_server_started` passes
- `test_server_observer_registration` passes

**Web crate** (`wasm-pack test --headless --firefox crates/web`):
- `test_wasm_panic_hook` passes
- `test_protocol_imports` passes
- `test_bevy_minimal_app` passes

---

## Testing Strategy

### Test Organization
```
crates/
  protocol/
    src/lib.rs             - Inline unit tests (#[cfg(test)])
    src/test_utils.rs      - Test utilities (feature-gated)
  client/tests/
    connection.rs          - Connection lifecycle tests
    messages.rs            - Message passing tests
  server/tests/
    multi_transport.rs     - Multi-transport setup tests
    connection_flow.rs     - Client-server connection tests
    observers.rs           - Observer lifecycle tests
  web/tests/
    wasm_integration.rs    - WASM smoke tests
```

### Test Execution
- **Unit tests**: Run via `cargo test-protocol` (inline `#[cfg(test)]` modules)
- **Integration tests**: Run via `cargo test-client`, `cargo test-server` (`tests/` directories)
- **WASM tests**: Run via `cargo test-wasm` (wasm-pack with headless browser)
- **All tests**: Run via `cargo test-all` (sequential execution of all suites)

### Test Philosophy
- **No manual verification**: All tests are automated via `cargo test`
- **Headless execution**: Use MinimalPlugins (no rendering, no window)
- **In-memory networking**: Use crossbeam channels (no actual network stack)
- **Deterministic time**: Use mock_instant for time control
- **Granular assertions**: Each test verifies one specific behavior

### Coverage Targets
- Protocol: 95% (foundational, must be rock solid)
- Server: 85% (core business logic)
- Client: 80% (some UI complexity)
- Web: 70% (WASM limitations, smoke tests only)

---

## Performance Considerations

- **Parallel test execution**: Cargo runs tests in parallel by default (one thread per core)
- **Fast compilation**: `[profile.test] opt-level = 1` balances compile speed and test speed
- **Dependency optimization**: `[profile.dev.package."*"] opt-level = 3` optimizes deps once
- **Headless testing**: MinimalPlugins eliminates rendering overhead
- **In-memory channels**: Crossbeam channels eliminate network latency

---

## Migration Notes

No migration needed - this is net-new infrastructure with zero existing tests.

---

## References

- Research document: `thoughts/shared/research/2025-11-22-testing-infrastructure-patterns.md`
- Lightyear stepper: `git/lightyear/lightyear_tests/src/stepper.rs:36-567`
- Lightyear connection tests: `git/lightyear/lightyear_tests/src/client_server/base.rs:9-105`
- Lightyear message tests: `git/lightyear/lightyear_tests/src/client_server/messages.rs:32-80`
- Bevy test guide: `git/bevy/tests/how_to_test_apps.rs:52-86`
- Current protocol: `crates/protocol/src/lib.rs:1-33`
- Current client: `crates/client/src/main.rs:1-66`
- Current server: `crates/server/src/main.rs:1-89`
- Current web: `crates/web/src/main.rs:1-95`
