---
date: 2025-11-28T07:47:49-08:00
researcher: Claude
git_commit: 70387d0842932434d9db6bab03e87eea74b26551
branch: master
repository: bevy-lightyear-template
topic: "UI Crate Setup and Client State Management"
tags: [research, codebase, ui, client-state, bevy-ui, plugin-architecture, testing]
status: complete
last_updated: 2025-11-28
last_updated_by: Claude
---

# Research: UI Crate Setup and Client State Management

**Date**: 2025-11-28T07:47:49-08:00
**Researcher**: Claude
**Git Commit**: 70387d0842932434d9db6bab03e87eea74b26551
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to set up a UI crate and UiPlugin for main menu and in-game UI, with client state management to defer connection, including:
- UI crate used by native and web clients but not server
- Main menu UI with "connect" and "quit" buttons
- In-game UI with "main menu" and "quit" buttons
- Client state to defer connecting until button click
- Comprehensive testing for UI and state transitions

## Summary

The codebase currently has no UI implementation and no state management system. Research identified the existing plugin architecture patterns, crate organization, and testing infrastructure that would support adding a UI crate and client state system.

**Current Architecture:**
- 5-crate workspace: `protocol`, `client`, `server`, `web`, `render`
- Render crate is shared between native and web clients
- Plugin-based architecture with configuration structs
- Component-based networking with observer patterns
- Comprehensive testing infrastructure using headless apps

**Patterns Available:**
- Plugin patterns from existing code (simple, configurable, wrapper)
- State management patterns from lightyear lobby example
- Conditional compilation for client/server/web differentiation
- Integration testing with client-server interaction
- Observer-based event handling for state transitions

## Detailed Findings

### Existing Crate Structure

**Workspace Members** ([Cargo.toml](Cargo.toml)):
- `crates/protocol/` - Shared network protocol (all targets)
- `crates/client/` - Native client binary
- `crates/server/` - Server binary
- `crates/web/` - WASM web client binary
- `crates/render/` - Shared rendering library (clients only)

**Render Crate Pattern** ([crates/render/src/lib.rs](crates/render/src/lib.rs)):
- Library crate (no `main.rs`)
- Contains `RenderPlugin` shared by both clients
- Not used by server (dependency only in client/web Cargo.toml)
- Currently minimal: only spawns 3D camera

A UI crate would follow the same pattern as `render`:
- Library crate with `UiPlugin`
- Dependency in `client` and `web` Cargo.toml
- Not included in `server` dependencies
- Contains UI systems, components, and state definitionspub struct WebClientPlugin;
```rust
impl Default for WebClientPlugin {
    fn default() -> Self {
        Self
    }
}

impl Plugin for WebClientPlugin {
    fn build(&self, app: &mut App) {
        // Load certificate digest for WebTransport
        #[cfg(target_family = "wasm")]
        let certificate_digest = include_str!("../../../certificates/digest.txt").to_string();

        #[cfg(not(target_family = "wasm"))]
        let certificate_digest = String::new();

        // Configure for WebTransport on port 5001
        let config = ClientNetworkConfig {
            client_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            server_addr: SocketAddr::from(([127, 0, 0, 1], 5001)),
            client_id: 0,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            transport: ClientTransport::WebTransport { certificate_digest },
        };

        // Reuse ClientNetworkPlugin with WebTransport config
        app.add_plugins(ClientNetworkPlugin { config });
    }
}
```

### Plugin Architecture Patterns

**Pattern 1: Simple Plugin (No Configuration)**

[crates/render/src/lib.rs:4-10](crates/render/src/lib.rs#L4-L10):
```rust
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_camera);
    }
}
```

**Pattern 2: Plugin with Configuration**

[crates/client/src/network.rs:52-73](crates/client/src/network.rs#L52-L73):
```rust
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
        app.add_systems(Startup, move |commands: Commands| {
            setup_client(commands, config.clone());
        });
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);
    }
}
```

**Pattern 3: Wrapper Plugin**

[crates/web/src/network.rs:9-39](crates/web/src/network.rs#L9-L39):
```rust
pub struct WebClientPlugin;

impl Plugin for WebClientPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(target_family = "wasm")]
        let certificate_digest = include_str!("../../../certificates/digest.txt").to_string();

        #[cfg(not(target_family = "wasm"))]
        let certificate_digest = String::new();

        let config = ClientNetworkConfig {
            // ... configuration
            transport: ClientTransport::WebTransport { certificate_digest },
        };

        app.add_plugins(ClientNetworkPlugin { config });
    }
}
```

### State Management (From Lightyear Examples)

The codebase does not currently use Bevy states. However, the lightyear lobby example demonstrates the pattern:

**State Definition** ([git/lightyear/examples/lobby/src/client.rs:16-27](git/lightyear/examples/lobby/src/client.rs#L16-L27)):
```rust
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppState {
    Lobby { joined_lobby: Option<usize> },
    Game,
}

impl Default for AppState {
    fn default() -> Self {
        AppState::Lobby { joined_lobby: None }
    }
}
```

**State Registration** ([git/lightyear/examples/lobby/src/client.rs:32](git/lightyear/examples/lobby/src/client.rs#L32)):
```rust
app.init_state::<AppState>();
```

**Conditional System Execution** ([git/lightyear/examples/lobby/src/client.rs:37-49](git/lightyear/examples/lobby/src/client.rs#L37-L49)):
```rust
app.add_systems(
    FixedPreUpdate,
    game::buffer_input
        .in_set(InputSystems::WriteClientInputs)
        .run_if(in_state(AppState::Game)),
);

app.add_systems(
    FixedUpdate,
    game::player_movement.run_if(in_state(AppState::Game)),
);
```

**State Transitions** (from lightyear examples):
State transitions use `NextState<T>` resource:
```rust
fn connect_button_clicked(mut next_state: ResMut<NextState<AppState>>) {
    next_state.set(AppState::Game);
}
```

### Client/Server Differentiation

**Binary Target Separation:**
- [crates/client/src/main.rs](crates/client/src/main.rs) - Native client entry
- [crates/web/src/main.rs](crates/web/src/main.rs) - WASM client entry
- [crates/server/src/main.rs](crates/server/src/main.rs) - Server entry

**Plugin Sets:**
- Native client: `DefaultPlugins` (full windowing/rendering)
- WASM client: `DefaultPlugins` with window customization
- Server: `MinimalPlugins` + `LogPlugin` (headless)

**Cargo Dependencies:**
Render crate dependencies:
- [crates/client/Cargo.toml:10](crates/client/Cargo.toml#L10): `render = { path = "../render" }`
- [crates/web/Cargo.toml:24](crates/web/Cargo.toml#L24): `render = { path = "../render" }`
- Server Cargo.toml: No render dependency

UI crate would follow the same dependency pattern.

### Bevy UI Components

No Bevy UI implementation exists in the current codebase. Bevy provides:
- `bevy::ui` module (part of `DefaultPlugins`)
- Components: `Node`, `Button`, `Text`
- Interaction system: `Interaction` component (Pressed, Hovered, None)
- Styling via `Style`, `BackgroundColor`, `BorderColor`

UI hierarchy pattern (from Bevy documentation):
```rust
commands.spawn(NodeBundle { /* root */ })
    .with_children(|parent| {
        parent.spawn(ButtonBundle { /* button */ })
            .with_children(|parent| {
                parent.spawn(TextBundle { /* button text */ });
            });
    });
```

### Observer-Based Event Handling

**Current Pattern** ([crates/client/src/network.rs:110-121](crates/client/src/network.rs#L110-L121)):
```rust
fn on_connected(trigger: Trigger<OnAdd, Connected>, mut commands: Commands) {
    let entity = trigger.entity();
    info!("Client {:?} connected", entity);
}

fn on_disconnected(trigger: Trigger<OnAdd, Disconnected>, mut commands: Commands) {
    let entity = trigger.entity();
    info!("Client {:?} disconnected", entity);
}
```

Observers are registered in plugin:
```rust
app.add_observer(on_connected);
app.add_observer(on_disconnected);
```

State transitions could use similar observer pattern to trigger UI updates or connection logic.

### Testing Infrastructure

**Test Organization:**
- Integration tests in `crates/*/tests/` directories
- Unit tests in `#[cfg(test)]` modules
- Test utilities in `crates/protocol/src/test_utils.rs`

**Test Pattern 1: Plugin Verification** ([crates/client/tests/plugin.rs:10-43](crates/client/tests/plugin.rs#L10-L43)):
```rust
#[test]
fn test_client_network_plugin_spawns_entity() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins { tick_duration });
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(ClientNetworkPlugin::default());

    app.update();

    // Verify entity spawned
    let mut query = app.world_mut().query_filtered::<Entity, With<Client>>();
    let count = query.iter(app.world()).count();
    assert_eq!(count, 1);
}
```

**Test Pattern 2: Observer Testing** ([crates/client/tests/plugin.rs:60-72](crates/client/tests/plugin.rs#L60-L72)):
```rust
#[test]
fn test_client_network_plugin_registers_observers() {
    // Setup app with plugin
    let mut app = App::new();
    app.add_plugins(ClientNetworkPlugin::default());

    // Get client entity
    let mut query = app.world_mut().query_filtered::<Entity, With<Client>>();
    let client_entity = query.single(app.world()).unwrap();

    // Manually trigger event
    app.world_mut().entity_mut(client_entity).insert(Connected);
    app.update();

    // Verify observer processed without panic
    let has_connected = app.world().entity(client_entity).contains::<Connected>();
    assert!(has_connected);
}
```

**Test Pattern 3: Integration Testing** ([crates/server/tests/integration.rs:82-109](crates/server/tests/integration.rs#L82-L109)):
```rust
// Manual time control for deterministic testing
let mut current_time = Instant::now();
server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));

// Step both apps in lockstep
for i in 0..300 {
    current_time += frame_duration;
    server_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));
    client_app.insert_resource(TimeUpdateStrategy::ManualInstant(current_time));

    server_app.update();
    client_app.update();

    std::thread::sleep(Duration::from_micros(100));

    // Check for connection
    let mut query = client_app.world_mut().query_filtered::<Entity, (With<Client>, With<Connected>)>();
    if query.iter(client_app.world()).count() > 0 {
        break;
    }
}
```

**Test Pattern 4: WASM Testing** ([crates/web/tests/wasm_integration.rs:1-36](crates/web/tests/wasm_integration.rs#L1-L36)):
```rust
#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn test_bevy_minimal_app() {
    use bevy::prelude::*;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.world_mut().spawn_empty();
    app.update();
    assert!(app.world().entities().len() > 0);
}
```

**Test Pattern 5: Component Query Testing**

Common pattern for verifying UI components:
```rust
// Query for specific component combinations
let mut query = app.world_mut().query_filtered::<Entity, (With<Button>, With<MainMenuMarker>)>();
let button_count = query.iter(app.world()).count();
assert_eq!(button_count, 2); // Connect and Quit buttons
```

**Test Pattern 6: State Transition Testing**

From lightyear lobby example pattern:
```rust
// Initialize app with state
app.init_state::<ClientState>();
app.update();

// Verify initial state
assert_eq!(*app.world().resource::<State<ClientState>>(), ClientState::MainMenu);

// Trigger state change
app.world_mut().resource_mut::<NextState<ClientState>>().set(ClientState::Playing);
app.update();

// Verify state changed
assert_eq!(*app.world().resource::<State<ClientState>>(), ClientState::Playing);
```

### Conditional Compilation for UI

**Pattern from web client** ([crates/web/src/network.rs:20-24](crates/web/src/network.rs#L20-L24)):
```rust
#[cfg(target_family = "wasm")]
let certificate_digest = include_str!("../../../certificates/digest.txt").to_string();

#[cfg(not(target_family = "wasm"))]
let certificate_digest = String::new();
```

UI crate can use similar patterns for platform-specific UI code if needed, though most Bevy UI works identically on native and WASM.

## Code References

### Crate Organization
- [Cargo.toml](Cargo.toml) - Workspace definition
- [crates/render/src/lib.rs](crates/render/src/lib.rs) - Example shared library crate
- [crates/render/Cargo.toml](crates/render/Cargo.toml) - Library crate configuration

### Plugin Patterns
- [crates/render/src/lib.rs:4-10](crates/render/src/lib.rs#L4-L10) - Simple plugin
- [crates/client/src/network.rs:52-73](crates/client/src/network.rs#L52-L73) - Configurable plugin
- [crates/web/src/network.rs:9-39](crates/web/src/network.rs#L9-L39) - Wrapper plugin
- [crates/protocol/src/lib.rs:13-28](crates/protocol/src/lib.rs#L13-L28) - Registration plugin

### Application Entry Points
- [crates/client/src/main.rs:10-20](crates/client/src/main.rs#L10-L20) - Native client app
- [crates/web/src/main.rs:10-29](crates/web/src/main.rs#L10-L29) - WASM client app
- [crates/server/src/main.rs:8-18](crates/server/src/main.rs#L8-L18) - Server app

### Observer Patterns
- [crates/client/src/network.rs:110-121](crates/client/src/network.rs#L110-L121) - Connection observers
- [crates/server/src/network.rs:175-195](crates/server/src/network.rs#L175-L195) - New client observer

### Testing Patterns
- [crates/client/tests/plugin.rs](crates/client/tests/plugin.rs) - Plugin tests
- [crates/client/tests/connection.rs](crates/client/tests/connection.rs) - Connection tests
- [crates/server/tests/integration.rs](crates/server/tests/integration.rs) - Client-server integration
- [crates/web/tests/wasm_integration.rs](crates/web/tests/wasm_integration.rs) - WASM tests

### Test Utilities
- [crates/protocol/src/test_utils.rs](crates/protocol/src/test_utils.rs) - Shared test helpers
- [crates/protocol/Cargo.toml:6-7](crates/protocol/Cargo.toml#L6-L7) - Test utils feature flag

## Architecture Documentation

### Current Plugin Registration Flow

**Native Client** ([crates/client/src/main.rs](crates/client/src/main.rs)):
1. `DefaultPlugins` - Full Bevy functionality
2. `ClientPlugins` - Lightyear client framework
3. `ProtocolPlugin` - Network protocol registration
4. `ClientNetworkPlugin` - Connection handling
5. `RenderPlugin` - Camera setup

**WASM Client** ([crates/web/src/main.rs](crates/web/src/main.rs)):
1. `DefaultPlugins` - Full Bevy functionality (with window config)
2. `ClientPlugins` - Lightyear client framework
3. `ProtocolPlugin` - Network protocol registration
4. `WebClientPlugin` - WebTransport connection (wraps `ClientNetworkPlugin`)
5. `RenderPlugin` - Camera setup

**Server** ([crates/server/src/main.rs](crates/server/src/main.rs)):
1. `MinimalPlugins` - Headless framework
2. `LogPlugin` - Console logging
3. `ServerPlugins` - Lightyear server framework
4. `ProtocolPlugin` - Network protocol registration
5. `ServerNetworkPlugin` - Multi-transport handling

### Component-Based Architecture

**Network State Components:**
- `Client` - Marks client entity
- `Connected` - Added when connection established
- `Disconnected` - Added when connection lost
- `LocalAddr` / `PeerAddr` - Network addresses
- `Link` - Connection state management
- `ReplicationReceiver` / `ReplicationSender` - State sync

**Observer Pattern:**
Components are added/removed, triggering observers:
```rust
app.add_observer(on_connected);  // Runs when Connected added
app.add_observer(on_disconnected);  // Runs when Disconnected added
```

### Testing Architecture

**Test Hierarchy:**
1. **Unit tests**: Inline `#[cfg(test)]` modules verify data structures
2. **Plugin tests**: Verify plugins add correct components/systems
3. **Observer tests**: Manually trigger component insertion, verify observers run
4. **Integration tests**: Full client-server interaction with manual time control
5. **WASM tests**: Browser-based execution via `wasm-bindgen-test`

**Test Utilities:**
- `MinimalPlugins` for headless testing
- `TimeUpdateStrategy::ManualInstant` for deterministic time
- `query_filtered` for component presence verification
- `Crossbeam` transport for fast in-memory testing

### State Management Patterns (From Lightyear)

Lightyear lobby example shows two approaches:

**1. Bevy States for App Flow:**
```rust
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AppState {
    Lobby { joined_lobby: Option<usize> },
    Game,
}
```
- Registered with `app.init_state::<AppState>()`
- Systems conditionally run with `.run_if(in_state(...))`
- Transitions via `NextState<T>` resource

**2. Component-Based Network State:**
```rust
#[derive(Component)]
struct Connected;
```
- Added/removed dynamically
- Observers react to changes
- Query-based logic in systems

UI could use Bevy states for app flow (MainMenu, Playing) while networking continues using component-based state.

## Related Research

No prior research documents exist for UI or state management in this project.

## External References

Bevy UI documentation and examples would provide implementation details for:
- UI component hierarchy
- Button interaction systems
- Text rendering
- Layout and styling
- WASM compatibility

Lightyear examples provide patterns for:
- State management with networking
- Lobby/game state transitions
- Integration with Bevy UI

## Open Questions

None - research comprehensively documented existing patterns and integration points for UI crate and state management implementation.
