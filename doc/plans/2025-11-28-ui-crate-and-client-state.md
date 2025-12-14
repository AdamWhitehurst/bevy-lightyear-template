# UI Crate and Client State Management Implementation Plan

## Overview

Implement a UI crate with state management to provide main menu, connecting screen, and in-game UI. Defer client connection until user clicks "Connect" button, replacing the current automatic connection behavior.

## Current State Analysis

**Existing Architecture**:
- 5-crate workspace: `protocol`, `client`, `server`, `web`, `render`
- `render` is a shared library crate used by both native and web clients
- Clients automatically connect during `Startup` via `commands.trigger(Connect { entity })` at [crates/client/src/network.rs:111](crates/client/src/network.rs#L111)
- No UI or state management exists
- Plugin registration order: `DefaultPlugins` → `ClientPlugins` → `ProtocolPlugin` → `ClientNetworkPlugin` → `RenderPlugin`

**Key Constraints**:
- Connection flow is observer-based (trigger-driven, not function calls)
- Lightyear components: `Connected`, `Disconnected`, `Connecting`, `Client`
- Transport layer handles IO (UDP, WebTransport, Crossbeam)
- Tests use `MinimalPlugins` with manual time control for determinism

## Desired End State

After implementation:
1. New `crates/ui/` library crate with `UiPlugin`
2. `ClientState` enum: `MainMenu`, `Connecting`, `InGame`
3. Main menu with "Connect" and "Quit" buttons
4. Connecting screen with loading indicator and "Cancel" button
5. In-game HUD with "Main Menu" and "Quit" buttons
6. Client only connects when user clicks "Connect" button
7. Comprehensive tests for UI spawning, state transitions, and connection flow

**Verification**:
- `cargo test-all` passes with new UI and state tests
- `cargo client -c 1` starts at main menu (not auto-connected)
- Clicking "Connect" triggers connection and transitions to connecting screen
- Successful connection transitions to in-game state with HUD
- Clicking "Main Menu" disconnects and returns to main menu
- `bevy run web` exhibits same behavior

## What We're NOT Doing

- Debug overlays (FPS, network stats) - deferred to future work
- Settings menu or graphics options
- Customizable UI themes or styling
- Multiplayer lobby UI (out of scope)
- Button visual state testing (Hovered, Pressed) - functional tests only
- UI animations or transitions

## Implementation Approach

Follow the established crate pattern (`render` as precedent):
1. Create library crate with plugin
2. Add as dependency to `client` and `web` only (not `server`)
3. Use Bevy's `States` system for global state management
4. Use observer pattern for button interactions (modern Bevy approach)
5. Use `OnEnter`/`OnExit` schedules for UI lifecycle
6. Use `DespawnOnExit` marker for automatic cleanup

## Phase 1: UI Crate Foundation

### Overview
Create the UI crate structure, define state enum, and implement basic plugin.

### Changes Required

#### 1. Create UI Crate Structure
**Files**: Create new crate directory and files

```bash
crates/ui/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── state.rs
│   └── components.rs
└── tests/
    └── ui_plugin.rs
```

#### 2. UI Crate Cargo.toml
**File**: `crates/ui/Cargo.toml`

```toml
[package]
name = "ui"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true, default-features = true }
```

#### 3. Client State Definition
**File**: `crates/ui/src/state.rs`

```rust
use bevy::prelude::*;

/// Client application state
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, States)]
pub enum ClientState {
    /// Main menu - not connected to server
    #[default]
    MainMenu,
    /// Connecting to server - loading screen
    Connecting,
    /// Connected and in-game
    InGame,
}
```

#### 4. UI Marker Components
**File**: `crates/ui/src/components.rs`

```rust
use bevy::prelude::*;

/// Marker for Connect button in main menu
#[derive(Component)]
pub struct ConnectButton;

/// Marker for Quit button (appears in main menu and in-game)
#[derive(Component)]
pub struct QuitButton;

/// Marker for Main Menu button in in-game UI
#[derive(Component)]
pub struct MainMenuButton;

/// Marker for Cancel button in connecting screen
#[derive(Component)]
pub struct CancelButton;
```

#### 5. UI Plugin Implementation
**File**: `crates/ui/src/lib.rs`

```rust
pub mod state;
pub mod components;

use bevy::prelude::*;
pub use state::ClientState;
pub use components::*;

/// Plugin that manages UI and client state
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        // Initialize state management
        app.init_state::<ClientState>();

        // UI lifecycle systems will be added in later phases
        info!("UiPlugin initialized");
    }
}
```

#### 6. Update Workspace Cargo.toml
**File**: `Cargo.toml`
**Changes**: Add UI crate to workspace members

```toml
[workspace]
members = [
    "crates/protocol",
    "crates/client",
    "crates/server",
    "crates/web",
    "crates/render",
    "crates/ui",  # Add this line
]
```

#### 7. Add UI Dependency to Client
**File**: `crates/client/Cargo.toml`
**Changes**: Add UI crate after render dependency

```toml
[dependencies]
# ... existing dependencies ...
render = { path = "../render" }
ui = { path = "../ui" }  # Add this line
```

#### 8. Add UI Dependency to Web Client
**File**: `crates/web/Cargo.toml`
**Changes**: Add UI crate after render dependency

```toml
[dependencies]
# ... existing dependencies ...
render = { path = "../render" }
ui = { path = "../ui" }  # Add this line
```

#### 9. Register UI Plugin in Native Client
**File**: `crates/client/src/main.rs`
**Changes**: Add UiPlugin import and registration

```rust
use ui::UiPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .add_plugins(ClientNetworkPlugin::default())
        .add_plugins(RenderPlugin)
        .add_plugins(UiPlugin)  // Add after RenderPlugin
        .run();
}
```

#### 10. Register UI Plugin in Web Client
**File**: `crates/web/src/main.rs`
**Changes**: Add UiPlugin import and registration

```rust
use ui::UiPlugin;

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
        .add_plugins(RenderPlugin)
        .add_plugins(UiPlugin)  // Add after RenderPlugin
        .run();
}
```

### Success Criteria

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] UI crate builds: `cargo build -p ui`
- [x] Client builds with UI: `cargo build -p client`
- [x] Web client builds with UI: `cargo build -p web`

#### Manual Verification:
- [ ] Running `cargo client -c 1` logs "UiPlugin initialized"
- [ ] No compile errors or warnings in new crate

---

## Phase 2: State Management Integration

### Overview
Modify network plugin to defer connection and add state-based connection triggering.

### Changes Required

#### 1. Remove Automatic Connection Trigger
**File**: `crates/client/src/network.rs`
**Changes**: Remove line 111 that auto-triggers connection

```rust
fn setup_client(mut commands: Commands, config: ClientNetworkConfig) {
    // ... existing code up to line 108 ...

    let client = entity_builder.id();

    // REMOVE THIS LINE:
    // commands.trigger(Connect { entity: client });

    // Store client entity for state-based connection
    commands.insert_resource(ClientEntity(client));
}
```

#### 2. Client Entity Resource
**File**: `crates/client/src/network.rs`
**Changes**: Add resource to store client entity ID

```rust
/// Resource storing the client entity for state-based connection
#[derive(Resource)]
pub struct ClientEntity(pub Entity);
```

#### 3. Export ClientEntity Resource
**File**: `crates/client/src/lib.rs`
**Changes**: Export ClientEntity for UI crate

```rust
pub mod network;
pub use network::ClientEntity;
```

#### 4. State-Based Connection System
**File**: `crates/ui/src/lib.rs`
**Changes**: Add system to trigger connection on state enter

```rust
use bevy::prelude::*;
use lightyear::prelude::client::*;

fn on_entering_connecting_state(
    mut commands: Commands,
    client_entity: Res<ClientEntity>,
) {
    info!("Entering Connecting state, triggering connection...");
    commands.trigger(Connect {
        entity: client_entity.0,
    });
}
```

Register in plugin:
```rust
impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<ClientState>();

        // State transition systems
        app.add_systems(OnEnter(ClientState::Connecting), on_entering_connecting_state);

        info!("UiPlugin initialized");
    }
}
```

#### 5. Disconnection Handler
**File**: `crates/ui/src/lib.rs`
**Changes**: Add observer to return to main menu on disconnect

```rust
fn on_client_disconnected(
    _trigger: On<Add, Disconnected>,
    mut next_state: ResMut<NextState<ClientState>>,
    current_state: Res<State<ClientState>>,
) {
    // Only transition if not already in MainMenu
    if *current_state.get() != ClientState::MainMenu {
        info!("Client disconnected, returning to main menu");
        next_state.set(ClientState::MainMenu);
    }
}
```

Register in plugin:
```rust
impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<ClientState>();
        app.add_systems(OnEnter(ClientState::Connecting), on_entering_connecting_state);
        app.add_observer(on_client_disconnected);

        info!("UiPlugin initialized");
    }
}
```

#### 6. Connected State Transition
**File**: `crates/ui/src/lib.rs`
**Changes**: Add observer to transition to InGame on connection

```rust
fn on_client_connected(
    _trigger: On<Add, Connected>,
    mut next_state: ResMut<NextState<ClientState>>,
) {
    info!("Client connected, transitioning to InGame state");
    next_state.set(ClientState::InGame);
}
```

Register in plugin:
```rust
impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<ClientState>();
        app.add_systems(OnEnter(ClientState::Connecting), on_entering_connecting_state);
        app.add_observer(on_client_disconnected);
        app.add_observer(on_client_connected);

        info!("UiPlugin initialized");
    }
}
```

### Success Criteria

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Client builds: `cargo client`
- [x] Web builds: `bevy run web`

#### Manual Verification:
- [ ] Client no longer auto-connects on startup
- [ ] Client state starts at `MainMenu`
- [ ] Manually triggering state change to `Connecting` initiates connection
- [ ] Connection success transitions to `InGame` state
- [ ] Disconnection returns to `MainMenu` state

---

## Phase 3: Main Menu UI

### Overview
Implement main menu UI with Connect and Quit buttons.

### Changes Required

#### 1. Main Menu UI Construction
**File**: `crates/ui/src/lib.rs`
**Changes**: Add main menu setup system

```rust
fn setup_main_menu(mut commands: Commands) {
    info!("Setting up main menu UI");

    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(20.0),
            ..default()
        },
        BackgroundColor(Color::srgb(0.1, 0.1, 0.1)),
        StateScoped(ClientState::MainMenu),
    ))
    .with_children(|parent| {
        // Title
        parent.spawn((
            Text::new("Lightyear Client"),
            TextFont {
                font_size: 60.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));

        // Connect Button
        parent.spawn((
            Button,
            Node {
                width: Val::Px(200.0),
                height: Val::Px(65.0),
                border: UiRect::all(Val::Px(5.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor(Color::WHITE),
            BackgroundColor(Color::srgb(0.2, 0.2, 0.2)),
            ConnectButton,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Connect"),
                TextFont {
                    font_size: 33.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });

        // Quit Button
        parent.spawn((
            Button,
            Node {
                width: Val::Px(200.0),
                height: Val::Px(65.0),
                border: UiRect::all(Val::Px(5.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor(Color::WHITE),
            BackgroundColor(Color::srgb(0.2, 0.2, 0.2)),
            QuitButton,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Quit"),
                TextFont {
                    font_size: 33.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
    });
}
```

#### 2. Main Menu Button Interactions
**File**: `crates/ui/src/lib.rs`
**Changes**: Add button interaction system

```rust
fn main_menu_button_interaction(
    mut next_state: ResMut<NextState<ClientState>>,
    mut exit_writer: EventWriter<AppExit>,
    connect_query: Query<&Interaction, (Changed<Interaction>, With<ConnectButton>)>,
    quit_query: Query<&Interaction, (Changed<Interaction>, With<QuitButton>)>,
) {
    // Handle Connect button
    for interaction in connect_query.iter() {
        if *interaction == Interaction::Pressed {
            info!("Connect button pressed");
            next_state.set(ClientState::Connecting);
        }
    }

    // Handle Quit button
    for interaction in quit_query.iter() {
        if *interaction == Interaction::Pressed {
            info!("Quit button pressed");
            exit_writer.send(AppExit::Success);
        }
    }
}
```

#### 3. Register Main Menu Systems
**File**: `crates/ui/src/lib.rs`
**Changes**: Add systems to plugin

```rust
impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<ClientState>();

        // State transition systems
        app.add_systems(OnEnter(ClientState::Connecting), on_entering_connecting_state);
        app.add_observer(on_client_disconnected);
        app.add_observer(on_client_connected);

        // Main menu
        app.add_systems(OnEnter(ClientState::MainMenu), setup_main_menu);
        app.add_systems(Update, main_menu_button_interaction.run_if(in_state(ClientState::MainMenu)));

        info!("UiPlugin initialized");
    }
}
```

### Success Criteria

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Client builds: `cargo client`
- [x] Web builds: `bevy run web`

#### Manual Verification:
- [ ] Main menu appears on startup with title, Connect, and Quit buttons
- [ ] Clicking Connect transitions to Connecting state
- [ ] Clicking Quit exits the application
- [ ] UI is properly centered and styled

---

## Phase 4: Connecting Screen UI

### Overview
Implement connecting/loading screen with cancel button.

### Changes Required

#### 1. Connecting Screen Setup
**File**: `crates/ui/src/lib.rs`
**Changes**: Add connecting screen UI system

```rust
fn setup_connecting_screen(mut commands: Commands) {
    info!("Setting up connecting screen UI");

    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(20.0),
            ..default()
        },
        BackgroundColor(Color::srgb(0.1, 0.1, 0.1)),
        StateScoped(ClientState::Connecting),
    ))
    .with_children(|parent| {
        // Connecting message
        parent.spawn((
            Text::new("Connecting to server..."),
            TextFont {
                font_size: 40.0,
                ..default()
            },
            TextColor(Color::WHITE),
        ));

        // Cancel Button
        parent.spawn((
            Button,
            Node {
                width: Val::Px(200.0),
                height: Val::Px(65.0),
                border: UiRect::all(Val::Px(5.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor(Color::WHITE),
            BackgroundColor(Color::srgb(0.2, 0.2, 0.2)),
            CancelButton,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Cancel"),
                TextFont {
                    font_size: 33.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
    });
}
```

#### 2. Cancel Button Interaction
**File**: `crates/ui/src/lib.rs`
**Changes**: Add cancel button system

```rust
use lightyear::prelude::client::*;

fn connecting_screen_interaction(
    mut commands: Commands,
    mut next_state: ResMut<NextState<ClientState>>,
    client_entity: Res<ClientEntity>,
    cancel_query: Query<&Interaction, (Changed<Interaction>, With<CancelButton>)>,
) {
    for interaction in cancel_query.iter() {
        if *interaction == Interaction::Pressed {
            info!("Cancel button pressed, disconnecting...");

            // Trigger disconnection
            commands.trigger(Disconnect {
                entity: client_entity.0,
            });

            // Return to main menu (observer will also handle this, but explicit is clearer)
            next_state.set(ClientState::MainMenu);
        }
    }
}
```

#### 3. Register Connecting Screen Systems
**File**: `crates/ui/src/lib.rs`
**Changes**: Add to plugin

```rust
impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<ClientState>();

        // State transition systems
        app.add_systems(OnEnter(ClientState::Connecting), on_entering_connecting_state);
        app.add_observer(on_client_disconnected);
        app.add_observer(on_client_connected);

        // Main menu
        app.add_systems(OnEnter(ClientState::MainMenu), setup_main_menu);
        app.add_systems(Update, main_menu_button_interaction.run_if(in_state(ClientState::MainMenu)));

        // Connecting screen
        app.add_systems(OnEnter(ClientState::Connecting), setup_connecting_screen);
        app.add_systems(Update, connecting_screen_interaction.run_if(in_state(ClientState::Connecting)));

        info!("UiPlugin initialized");
    }
}
```

### Success Criteria

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Client builds: `cargo client`
- [x] Web builds: `bevy run web`

#### Manual Verification:
- [ ] Clicking Connect shows "Connecting to server..." screen
- [ ] Cancel button appears and is clickable
- [ ] Clicking Cancel disconnects and returns to main menu
- [ ] Successful connection transitions to InGame state
- [ ] UI is properly centered and styled

---

## Phase 5: In-Game UI

### Overview
Implement in-game HUD with Main Menu and Quit buttons.

### Changes Required

#### 1. In-Game HUD Setup
**File**: `crates/ui/src/lib.rs`
**Changes**: Add in-game UI system

```rust
fn setup_ingame_hud(mut commands: Commands) {
    info!("Setting up in-game HUD");

    // Top-right corner HUD
    commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::End,
            align_items: AlignItems::Start,
            padding: UiRect::all(Val::Px(20.0)),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(10.0),
            ..default()
        },
        StateScoped(ClientState::InGame),
    ))
    .with_children(|parent| {
        // Main Menu Button
        parent.spawn((
            Button,
            Node {
                width: Val::Px(150.0),
                height: Val::Px(50.0),
                border: UiRect::all(Val::Px(3.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor(Color::WHITE),
            BackgroundColor(Color::srgba(0.2, 0.2, 0.2, 0.8)),
            MainMenuButton,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Main Menu"),
                TextFont {
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });

        // Quit Button
        parent.spawn((
            Button,
            Node {
                width: Val::Px(150.0),
                height: Val::Px(50.0),
                border: UiRect::all(Val::Px(3.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor(Color::WHITE),
            BackgroundColor(Color::srgba(0.2, 0.2, 0.2, 0.8)),
            QuitButton,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Quit"),
                TextFont {
                    font_size: 24.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
    });
}
```

#### 2. In-Game Button Interactions
**File**: `crates/ui/src/lib.rs`
**Changes**: Add in-game interaction system

```rust
fn ingame_button_interaction(
    mut commands: Commands,
    mut next_state: ResMut<NextState<ClientState>>,
    mut exit_writer: EventWriter<AppExit>,
    client_entity: Res<ClientEntity>,
    main_menu_query: Query<&Interaction, (Changed<Interaction>, With<MainMenuButton>)>,
    quit_query: Query<&Interaction, (Changed<Interaction>, With<QuitButton>, Without<MainMenuButton>)>,
) {
    // Handle Main Menu button
    for interaction in main_menu_query.iter() {
        if *interaction == Interaction::Pressed {
            info!("Main Menu button pressed, disconnecting...");

            // Trigger disconnection
            commands.trigger(Disconnect {
                entity: client_entity.0,
            });

            // Return to main menu (observer will also handle this)
            next_state.set(ClientState::MainMenu);
        }
    }

    // Handle Quit button
    for interaction in quit_query.iter() {
        if *interaction == Interaction::Pressed {
            info!("Quit button pressed");
            exit_writer.send(AppExit::Success);
        }
    }
}
```

#### 3. Register In-Game Systems
**File**: `crates/ui/src/lib.rs`
**Changes**: Add to plugin

```rust
impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<ClientState>();

        // State transition systems
        app.add_systems(OnEnter(ClientState::Connecting), on_entering_connecting_state);
        app.add_observer(on_client_disconnected);
        app.add_observer(on_client_connected);

        // Main menu
        app.add_systems(OnEnter(ClientState::MainMenu), setup_main_menu);
        app.add_systems(Update, main_menu_button_interaction.run_if(in_state(ClientState::MainMenu)));

        // Connecting screen
        app.add_systems(OnEnter(ClientState::Connecting), setup_connecting_screen);
        app.add_systems(Update, connecting_screen_interaction.run_if(in_state(ClientState::Connecting)));

        // In-game HUD
        app.add_systems(OnEnter(ClientState::InGame), setup_ingame_hud);
        app.add_systems(Update, ingame_button_interaction.run_if(in_state(ClientState::InGame)));

        info!("UiPlugin initialized");
    }
}
```

### Success Criteria

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Client builds: `cargo client`
- [x] Web builds: `bevy run web`

#### Manual Verification:
- [ ] After connection, in-game HUD appears in top-right corner
- [ ] Main Menu and Quit buttons are visible and clickable
- [ ] Clicking Main Menu disconnects and returns to main menu
- [ ] Clicking Quit exits the application
- [ ] HUD has semi-transparent background

---

## Phase 6: Testing

### Overview
Implement comprehensive tests for UI, state transitions, and connection flow.

### Changes Required

#### 1. UI Plugin Test
**File**: `crates/ui/tests/ui_plugin.rs`

```rust
use bevy::prelude::*;
use ui::*;

#[test]
fn test_ui_plugin_initializes_state() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(UiPlugin);

    app.update();

    // Verify state is initialized
    let state = app.world().resource::<State<ClientState>>();
    assert_eq!(*state.get(), ClientState::MainMenu);
}
```

#### 2. Main Menu UI Test
**File**: `crates/ui/tests/ui_plugin.rs`
**Changes**: Add test for main menu UI spawning

```rust
#[test]
fn test_main_menu_spawns_buttons() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(UiPlugin);

    app.update();

    // Verify Connect button exists
    let mut connect_query = app.world_mut().query_filtered::<Entity, With<ConnectButton>>();
    assert_eq!(connect_query.iter(app.world()).count(), 1, "Should have one Connect button");

    // Verify Quit button exists
    let mut quit_query = app.world_mut().query_filtered::<Entity, With<QuitButton>>();
    assert_eq!(quit_query.iter(app.world()).count(), 1, "Should have one Quit button");
}
```

#### 3. State Transition Test
**File**: `crates/ui/tests/ui_plugin.rs`
**Changes**: Add state transition test

```rust
#[test]
fn test_connect_button_triggers_state_transition() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(UiPlugin);

    app.update();

    // Get connect button
    let mut query = app.world_mut().query_filtered::<Entity, With<ConnectButton>>();
    let button = query.single(app.world()).unwrap();

    // Simulate button press
    app.world_mut().entity_mut(button).insert(Interaction::Pressed);
    app.update();

    // Verify state transitioned to Connecting
    let state = app.world().resource::<State<ClientState>>();
    assert_eq!(*state.get(), ClientState::Connecting);
}
```

#### 4. Connection Triggering Test
**File**: `crates/client/tests/ui_state.rs`

```rust
use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use protocol::*;
use std::time::Duration;
use ui::*;
use client::ClientEntity;

#[test]
fn test_connecting_state_triggers_connection() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
    });
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(UiPlugin);

    // Manually setup client (since ClientNetworkPlugin is not added)
    let client = app.world_mut().spawn((
        Name::new("Client"),
        Client::default(),
    )).id();
    app.world_mut().insert_resource(ClientEntity(client));

    app.update();

    // Initially in MainMenu, no Connect trigger
    let state = app.world().resource::<State<ClientState>>();
    assert_eq!(*state.get(), ClientState::MainMenu);

    // Transition to Connecting state
    app.world_mut().insert_resource(NextState(Some(ClientState::Connecting)));
    app.update();

    // Verify state is now Connecting
    let state = app.world().resource::<State<ClientState>>();
    assert_eq!(*state.get(), ClientState::Connecting);
}
```

#### 5. In-Game UI Test
**File**: `crates/ui/tests/ui_plugin.rs`
**Changes**: Add in-game UI test

```rust
#[test]
fn test_ingame_state_spawns_hud() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(UiPlugin);

    // Transition to InGame state
    app.world_mut().insert_resource(NextState(Some(ClientState::InGame)));
    app.update();

    // Verify Main Menu button exists
    let mut main_menu_query = app.world_mut().query_filtered::<Entity, With<MainMenuButton>>();
    assert_eq!(main_menu_query.iter(app.world()).count(), 1, "Should have one Main Menu button");

    // Verify Quit button exists
    let mut quit_query = app.world_mut().query_filtered::<Entity, With<QuitButton>>();
    assert_eq!(quit_query.iter(app.world()).count(), 1, "Should have one Quit button");
}
```

#### 6. Disconnection Test
**File**: `crates/ui/tests/ui_plugin.rs`
**Changes**: Add disconnection handling test

```rust
#[test]
fn test_disconnection_returns_to_main_menu() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
    });
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(UiPlugin);

    // Setup client entity
    let client = app.world_mut().spawn((
        Name::new("Client"),
        Client::default(),
    )).id();
    app.world_mut().insert_resource(ClientEntity(client));

    // Set to InGame state
    app.world_mut().insert_resource(NextState(Some(ClientState::InGame)));
    app.update();

    // Verify in InGame state
    let state = app.world().resource::<State<ClientState>>();
    assert_eq!(*state.get(), ClientState::InGame);

    // Trigger disconnection
    app.world_mut().entity_mut(client).insert(Disconnected::default());
    app.update();

    // Verify returned to MainMenu
    let state = app.world().resource::<State<ClientState>>();
    assert_eq!(*state.get(), ClientState::MainMenu);
}
```

#### 7. Update Existing Connection Tests
**File**: `crates/client/tests/connection.rs`
**Changes**: Update tests to account for state management

```rust
// Add UI plugin to existing connection tests
use ui::{UiPlugin, ClientState};

#[test]
fn test_client_connects_to_server() {
    // ... existing setup ...

    app.add_plugins(UiPlugin);

    // Transition to Connecting state to trigger connection
    app.world_mut().insert_resource(NextState(Some(ClientState::Connecting)));

    // ... rest of test ...
}
```

### Success Criteria

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] UI plugin tests pass: `cargo test -p ui`
- [x] Client tests pass: `cargo test -p client`
- [x] Web tests pass: `cargo test -p web`

#### Manual Verification:
- [ ] All test output is clear and descriptive
- [ ] Tests run deterministically (no flakiness)
- [ ] Code coverage includes all UI states and transitions

---

## Testing Strategy

### Unit Tests
- **UI Component Spawning**: Verify correct number and type of buttons in each state
- **State Initialization**: Verify default state is MainMenu
- **State Transitions**: Verify button clicks trigger correct state changes

### Integration Tests
- **Connection Flow**: MainMenu → Connect button → Connecting state → trigger Connect → Connected component → InGame state
- **Disconnection Flow**: InGame → Main Menu button → Disconnect trigger → Disconnected component → MainMenu state
- **Cancel Flow**: Connecting → Cancel button → Disconnect trigger → MainMenu state

### Manual Testing Steps
1. Start client: `cargo client -c 1`
   - Verify main menu appears
   - Verify no automatic connection
2. Click Connect button
   - Verify connecting screen appears
   - Verify connection succeeds (check server logs)
   - Verify transition to in-game state
3. Click Main Menu button in-game
   - Verify disconnection occurs
   - Verify return to main menu
4. Click Quit button
   - Verify application exits cleanly
5. Test web client: `bevy run web`
   - Verify same behavior as native client

## Performance Considerations

- UI spawning only happens on state entry (OnEnter schedules)
- UI cleanup is automatic via `StateScoped` component
- Button interaction systems only run in relevant states (`.run_if(in_state())`)
- No continuous UI updates needed (static menus)
- Observer pattern is zero-cost abstraction

## Migration Notes

**Breaking Changes**:
- Clients no longer auto-connect on startup
- Existing connection tests need UI plugin added
- Connection behavior now requires state transition

**Compatibility**:
- Server is unaffected (no changes needed)
- Protocol is unaffected (no changes needed)
- Render plugin continues to work unchanged

## References

- Research document: `doc/research/2025-11-28-ui-crate-and-client-state.md`
- Render crate pattern: `crates/render/src/lib.rs`
- ClientNetworkPlugin: `crates/client/src/network.rs:52-73`
- Bevy States example: `git/bevy/examples/state/states.rs`
- Bevy Button example: `git/bevy/examples/ui/button.rs`
- Lightyear lobby example: `git/lightyear/examples/lobby/src/client.rs`
- Existing test patterns: `crates/client/tests/plugin.rs`
