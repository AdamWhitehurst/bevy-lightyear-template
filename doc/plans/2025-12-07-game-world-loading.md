# Game World Loading Implementation Plan

## Overview

Implement networked 3D game world with physics-based character controller using avian3d, lightyear prediction/interpolation, and leafwing inputs. Server spawns floor and characters; clients add non-replicated components (meshes, colliders). Local characters are predicted, remote characters are interpolated.

## Current State Analysis

- **Protocol**: Basic `ProtocolPlugin` with message/channel registration only
- **Server**: `ServerNetworkPlugin` handles connections, adds `ReplicationSender`
- **Client**: `ClientNetworkPlugin` connects to server
- **Render**: `RenderPlugin` spawns camera only
- **UI**: `ClientState` (MainMenu → Connecting → InGame) state machine

### Key Discoveries:
- Server uses `handle_new_client` observer on `Add<Connected>` ([network.rs:173](crates/server/src/network.rs#L173))
- Client state transitions on `Add<Connected>` → `InGame` ([lib.rs:62](crates/ui/src/lib.rs#L62))
- `avian3d = "0.4.1"` already in workspace dependencies
- `lightyear = "0.25.5"` already configured

## Desired End State

After implementation:
1. Server spawns static floor entity on startup (replicated to all clients)
2. Server spawns character entity when client connects (predicted by owner, interpolated by others)
3. Client adds physics colliders and meshes to replicated entities
4. Local character responds to WASD/Space input with physics-based movement
5. Remote characters smoothly interpolate between server updates
6. Visual interpolation smooths movement between physics ticks

### Verification:
- Run `cargo server` and `cargo client -c 2`
- Both clients see floor and two colored capsule characters
- Each client can move their own character with WASD/Space
- Remote character movements appear smooth

## What We're NOT Doing

- GLTF/scene loading (programmatic meshes only)
- Character animations
- Multiple maps/levels
- Weapons or combat
- Audio

## Implementation Approach

Follow the lightyear `avian_3d_character` example architecture:
1. Shared protocol defines markers, bundles, actions, component registration
2. Server spawns authoritative entities with `Replicate` + `PredictionTarget`
3. Client detects new entities via `Added<Predicted>` / `Added<Replicated>` and adds local components
4. Render adds cosmetics on entity detection
5. Movement system runs on both client (predicted) and server (authoritative)

---

## Phase 1: Dependencies & Protocol Components

### Overview
Add leafwing-input-manager dependency and implement shared protocol components.

### Changes Required:

#### 1. Workspace Cargo.toml
**File**: `Cargo.toml`
**Changes**: Add leafwing-input-manager to workspace dependencies

```toml
leafwing-input-manager = "0.17"
```

#### 2. Protocol Cargo.toml
**File**: `crates/protocol/Cargo.toml`
**Changes**: Add avian3d, leafwing-input-manager dependencies

```toml
[dependencies]
avian3d = { workspace = true }
leafwing-input-manager = { workspace = true }
```

#### 3. Protocol lib.rs
**File**: `crates/protocol/src/lib.rs`
**Changes**: Complete rewrite with markers, bundles, actions, component registration, SharedGameplayPlugin

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;
use lightyear::avian3d::plugin::{AvianReplicationMode, LightyearAvianPlugin};
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};



pub const PROTOCOL_ID: u64 = 0;
pub const PRIVATE_KEY: [u8; 32] = [0; 32];
pub const FIXED_TIMESTEP_HZ: f64 = 64.0;

pub const FLOOR_WIDTH: f32 = 100.0;
pub const FLOOR_HEIGHT: f32 = 1.0;
pub const CHARACTER_CAPSULE_RADIUS: f32 = 0.5;
pub const CHARACTER_CAPSULE_HEIGHT: f32 = 0.5;



#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CharacterMarker;

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FloorMarker;

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ColorComponent(pub Color);



#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Reflect, Serialize, Deserialize)]
pub enum CharacterAction {
    Move,
    Jump,
}

impl Actionlike for CharacterAction {
    fn input_control_kind(&self) -> InputControlKind {
        match self {
            Self::Move => InputControlKind::DualAxis,
            Self::Jump => InputControlKind::Button,
        }
    }
}



#[derive(Bundle)]
pub struct CharacterPhysicsBundle {
    pub collider: Collider,
    pub rigid_body: RigidBody,
    pub locked_axes: LockedAxes,
    pub friction: Friction,
}

impl Default for CharacterPhysicsBundle {
    fn default() -> Self {
        Self {
            collider: Collider::capsule(CHARACTER_CAPSULE_RADIUS, CHARACTER_CAPSULE_HEIGHT),
            rigid_body: RigidBody::Dynamic,
            locked_axes: LockedAxes::default()
                .lock_rotation_x()
                .lock_rotation_y()
                .lock_rotation_z(),
            friction: Friction::new(0.0).with_combine_rule(CoefficientCombine::Min),
        }
    }
}

#[derive(Bundle)]
pub struct FloorPhysicsBundle {
    pub collider: Collider,
    pub rigid_body: RigidBody,
}

impl Default for FloorPhysicsBundle {
    fn default() -> Self {
        Self {
            collider: Collider::cuboid(FLOOR_WIDTH, FLOOR_HEIGHT, FLOOR_WIDTH),
            rigid_body: RigidBody::Static,
        }
    }
}



pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        // Leafwing input plugin
        app.add_plugins(leafwing::InputPlugin::<CharacterAction> {
            config: InputConfig::<CharacterAction> {
                rebroadcast_inputs: true,
                ..default()
            },
        });

        // Marker components
        app.register_component::<ColorComponent>();
        app.register_component::<Name>();
        app.register_component::<CharacterMarker>();
        app.register_component::<FloorMarker>();

        // Velocity prediction without visual correction
        app.register_component::<LinearVelocity>()
            .add_prediction()
            .add_should_rollback(linear_velocity_should_rollback);

        app.register_component::<AngularVelocity>()
            .add_prediction()
            .add_should_rollback(angular_velocity_should_rollback);

        // Position/Rotation with prediction + visual correction + interpolation
        app.register_component::<Position>()
            .add_prediction()
            .add_should_rollback(position_should_rollback)
            .add_linear_correction_fn()
            .add_linear_interpolation();

        app.register_component::<Rotation>()
            .add_prediction()
            .add_should_rollback(rotation_should_rollback)
            .add_linear_correction_fn()
            .add_linear_interpolation();
    }
}

fn position_should_rollback(this: &Position, that: &Position) -> bool {
    (this.0 - that.0).length() >= 0.01
}

fn rotation_should_rollback(this: &Rotation, that: &Rotation) -> bool {
    this.angle_between(*that) >= 0.01
}

fn linear_velocity_should_rollback(this: &LinearVelocity, that: &LinearVelocity) -> bool {
    (this.0 - that.0).length() >= 0.01
}

fn angular_velocity_should_rollback(this: &AngularVelocity, that: &AngularVelocity) -> bool {
    (this.0 - that.0).length() >= 0.01
}



pub struct SharedGameplayPlugin;

impl Plugin for SharedGameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ProtocolPlugin);

        // Lightyear-Avian integration
        app.add_plugins(LightyearAvianPlugin {
            replication_mode: AvianReplicationMode::Position,
            ..default()
        });

        // Disable Avian's transform sync (lightyear handles it)
        app.add_plugins(
            PhysicsPlugins::default()
                .build()
                .disable::<SyncPlugin>(),
        );
    }
}



pub fn apply_character_action(
    entity: Entity,
    mass: &ComputedMass,
    time: &Res<Time>,
    spatial_query: &SpatialQuery,
    action_state: &ActionState<CharacterAction>,
    position: &Position,
    linear_velocity: &LinearVelocity,
    external_force: &mut ExternalForce,
    external_impulse: &mut ExternalImpulse,
) {
    const MAX_SPEED: f32 = 5.0;
    const MAX_ACCELERATION: f32 = 20.0;

    let max_velocity_delta_per_tick = MAX_ACCELERATION * time.delta_secs();

    // Jump with raycast ground detection
    if action_state.just_pressed(&CharacterAction::Jump) {
        let ray_cast_origin = position.0
            + Vec3::new(
                0.0,
                -CHARACTER_CAPSULE_HEIGHT / 2.0 - CHARACTER_CAPSULE_RADIUS,
                0.0,
            );

        if spatial_query
            .cast_ray(
                ray_cast_origin,
                Dir3::NEG_Y,
                0.01,
                true,
                &SpatialQueryFilter::from_excluded_entities([entity]),
            )
            .is_some()
        {
            external_impulse.apply_impulse(Vec3::new(0.0, 5.0, 0.0));
        }
    }

    // Horizontal movement
    let move_dir = action_state
        .axis_pair(&CharacterAction::Move)
        .clamp_length_max(1.0);
    let move_dir = Vec3::new(-move_dir.x, 0.0, move_dir.y);

    let ground_linear_velocity = Vec3::new(linear_velocity.x, 0.0, linear_velocity.z);

    let desired_ground_linear_velocity = move_dir * MAX_SPEED;
    let new_ground_linear_velocity = ground_linear_velocity
        .move_towards(desired_ground_linear_velocity, max_velocity_delta_per_tick);

    let required_acceleration =
        (new_ground_linear_velocity - ground_linear_velocity) / time.delta_secs();

    external_force.apply_force(required_acceleration * mass.value());
}

#[cfg(feature = "test_utils")]
pub mod test_utils;
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check -p protocol` compiles without errors
- [x] `cargo test -p protocol` passes

#### Manual Verification:
- [x] None for this phase

**Implementation Notes:**
- Skipped leafwing-input-manager due to bevy version conflict (lightyear's `leafwing` feature requires bevy 0.16, we use 0.17)
- Used lightyear's `avian3d` feature without leafwing
- Movement uses avian3d 0.4.1's `Forces` QueryData and `ForcesItem` instead of `ExternalForce`/`ExternalImpulse` components
- Input will be handled via simple keyboard input rather than leafwing action system

---

## Phase 2: ServerGameplayPlugin

### Overview
Implement server-side spawning of floor (on startup) and characters (on client connect).

### Changes Required:

#### 1. Server Cargo.toml
**File**: `crates/server/Cargo.toml`
**Changes**: Add avian3d dependency

```toml
[dependencies]
avian3d = { workspace = true }
```

#### 2. Server gameplay.rs (new file)
**File**: `crates/server/src/gameplay.rs`
**Changes**: Create ServerGameplayPlugin

```rust
use avian3d::prelude::*;
use bevy::color::palettes::css;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use lightyear::connection::client::Connected;
use lightyear::connection::client_of::ClientOf;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use protocol::*;

pub struct ServerGameplayPlugin;

impl Plugin for ServerGameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup);
        app.add_observer(handle_connected);
        app.add_systems(FixedUpdate, handle_character_actions);
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Name::new("Floor"),
        FloorPhysicsBundle::default(),
        FloorMarker,
        Position::new(Vec3::ZERO),
        Replicate::to_clients(NetworkTarget::All),
    ));
}

fn handle_connected(
    trigger: On<Add, Connected>,
    query: Query<&RemoteId, With<ClientOf>>,
    mut commands: Commands,
    character_query: Query<Entity, With<CharacterMarker>>,
) {
    let Ok(client_id) = query.get(trigger.entity) else {
        return;
    };
    let client_id = client_id.0;
    info!("Client connected with client-id {client_id:?}. Spawning character entity.");

    let num_characters = character_query.iter().count();

    let available_colors = [
        css::LIMEGREEN,
        css::PINK,
        css::YELLOW,
        css::AQUA,
        css::CRIMSON,
    ];
    let color = available_colors[num_characters % available_colors.len()];

    let angle: f32 = num_characters as f32 * 5.0;
    let x = 2.0 * angle.cos();
    let z = 2.0 * angle.sin();

    commands.spawn((
        Name::new("Character"),
        ActionState::<CharacterAction>::default(),
        Position(Vec3::new(x, 3.0, z)),
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
        ControlledBy {
            owner: trigger.entity,
            lifetime: Default::default(),
        },
        CharacterPhysicsBundle::default(),
        ColorComponent(color.into()),
        CharacterMarker,
    ));
}

fn handle_character_actions(
    time: Res<Time>,
    spatial_query: SpatialQuery,
    mut query: Query<(
        Entity,
        &ComputedMass,
        &ActionState<CharacterAction>,
        &Position,
        &LinearVelocity,
        &mut ExternalForce,
        &mut ExternalImpulse,
    )>,
) {
    for (entity, mass, action_state, position, linear_velocity, mut external_force, mut external_impulse) in &mut query {
        apply_character_action(
            entity,
            mass,
            &time,
            &spatial_query,
            action_state,
            position,
            linear_velocity,
            &mut external_force,
            &mut external_impulse,
        );
    }
}
```

#### 3. Server lib.rs
**File**: `crates/server/src/lib.rs`
**Changes**: Add gameplay module

```rust
pub mod network;
pub mod gameplay;
```

#### 4. Server main.rs
**File**: `crates/server/src/main.rs`
**Changes**: Add SharedGameplayPlugin and ServerGameplayPlugin

```rust
pub mod network;
pub mod gameplay;

use bevy::prelude::*;
use network::ServerNetworkPlugin;
use protocol::*;
use gameplay::ServerGameplayPlugin;
use std::time::Duration;

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(bevy::log::LogPlugin::default())
        .add_plugins(lightyear::prelude::server::ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(SharedGameplayPlugin)
        .add_plugins(ServerNetworkPlugin::default())
        .add_plugins(ServerGameplayPlugin)
        .run();
}
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check -p server` compiles without errors
- [x] `cargo server` starts without panic

#### Manual Verification:
- [ ] Server logs "Starting multi-transport server..." and floor spawn

**Implementation Notes:**
- Disabled `collider-from-mesh` feature in avian3d to avoid mesh asset dependencies
- Added AssetPlugin/ScenePlugin required by avian3d
- Observer uses `On<Add, Connected>` syntax
- PredictionTarget set to all clients (not per-client)

---

## Phase 3: ClientGameplayPlugin

### Overview
Implement client-side handling of replicated/predicted entities - add physics and input.

### Changes Required:

#### 1. Client Cargo.toml
**File**: `crates/client/Cargo.toml`
**Changes**: Add avian3d, leafwing-input-manager dependencies

```toml
[dependencies]
avian3d = { workspace = true }
leafwing-input-manager = { workspace = true }
```

#### 2. Client gameplay.rs (new file)
**File**: `crates/client/src/gameplay.rs`
**Changes**: Create ClientGameplayPlugin

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use protocol::*;

pub struct ClientGameplayPlugin;

impl Plugin for ClientGameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, handle_character_actions);
        app.add_systems(Update, (handle_new_floor, handle_new_character));
    }
}

fn handle_character_actions(
    time: Res<Time>,
    spatial_query: SpatialQuery,
    mut query: Query<
        (
            Entity,
            &ComputedMass,
            &ActionState<CharacterAction>,
            &Position,
            &LinearVelocity,
            &mut ExternalForce,
            &mut ExternalImpulse,
        ),
        With<Predicted>,
    >,
) {
    for (entity, computed_mass, action_state, position, linear_velocity, mut external_force, mut external_impulse) in &mut query {
        apply_character_action(
            entity,
            computed_mass,
            &time,
            &spatial_query,
            action_state,
            position,
            linear_velocity,
            &mut external_force,
            &mut external_impulse,
        );
    }
}

fn handle_new_character(
    mut commands: Commands,
    character_query: Query<
        (Entity, Has<Controlled>),
        (Added<Predicted>, With<CharacterMarker>),
    >,
) {
    for (entity, is_controlled) in &character_query {
        if is_controlled {
            info!("Adding InputMap to controlled and predicted entity {entity:?}");
            commands.entity(entity).insert(
                InputMap::new([(CharacterAction::Jump, KeyCode::Space)])
                    .with(CharacterAction::Jump, GamepadButton::South)
                    .with_dual_axis(CharacterAction::Move, GamepadStick::LEFT)
                    .with_dual_axis(CharacterAction::Move, VirtualDPad::wasd()),
            );
        } else {
            info!("Remote character predicted for us: {entity:?}");
        }
        info!(?entity, "Adding physics to character");
        commands
            .entity(entity)
            .insert(CharacterPhysicsBundle::default());
    }
}

fn handle_new_floor(
    mut commands: Commands,
    floor_query: Query<Entity, (Added<Replicated>, With<FloorMarker>)>,
) {
    for entity in &floor_query {
        info!(?entity, "Adding physics to floor");
        commands
            .entity(entity)
            .insert(FloorPhysicsBundle::default());
    }
}
```

#### 3. Client lib.rs
**File**: `crates/client/src/lib.rs`
**Changes**: Add gameplay module

```rust
pub mod network;
pub mod gameplay;
```

#### 4. Client main.rs
**File**: `crates/client/src/main.rs`
**Changes**: Add SharedGameplayPlugin and ClientGameplayPlugin

```rust
pub mod network;
pub mod gameplay;

use bevy::prelude::*;
use lightyear::prelude::client::*;
use network::ClientNetworkPlugin;
use protocol::*;
use render::RenderPlugin;
use gameplay::ClientGameplayPlugin;
use ui::UiPlugin;
use std::time::Duration;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(SharedGameplayPlugin)
        .add_plugins(ClientNetworkPlugin::default())
        .add_plugins(ClientGameplayPlugin)
        .add_plugins(RenderPlugin)
        .add_plugins(UiPlugin)
        .run();
}
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check -p client` compiles without errors
- [x] `cargo client` starts without panic

#### Manual Verification:
- [ ] Client logs floor/character physics additions on connect

**Implementation Notes:**
- Using simple keyboard input (WASD+Space) instead of leafwing ActionState
- Movement system runs in FixedUpdate with Predicted+Controlled filter
- Forces QueryData used correctly for physics

---

## Phase 4: Render Cosmetics & Interpolation

### Overview
Add meshes/materials to entities and visual interpolation for smooth movement.

### Changes Required:

#### 1. Render Cargo.toml
**File**: `crates/render/Cargo.toml`
**Changes**: Add protocol, lightyear dependencies

```toml
[dependencies]
bevy = { workspace = true, default-features = true }
lightyear = { workspace = true, features = ["frame_interpolation"] }
protocol = { workspace = true }
```

#### 2. Render lib.rs
**File**: `crates/render/src/lib.rs`
**Changes**: Complete rewrite with cosmetics and interpolation

```rust
use avian3d::prelude::Position;
use bevy::prelude::*;
use lightyear::frame_interpolation::{FrameInterpolate, FrameInterpolationPlugin};
use lightyear::prelude::*;
use protocol::*;

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_camera, setup_lighting));
        app.add_systems(Update, (add_character_cosmetics, add_floor_cosmetics));

        // FrameInterpolationPlugin for visual smoothing
        app.add_plugins(FrameInterpolationPlugin::<Position>::default());
        app.add_plugins(FrameInterpolationPlugin::<avian3d::prelude::Rotation>::default());

        app.add_observer(add_visual_interpolation_components);
    }
}

fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 4.5, -9.0).looking_at(Vec3::ZERO, Dir3::Y),
    ));
}

fn setup_lighting(mut commands: Commands) {
    commands.spawn((
        PointLight {
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0),
    ));
}

fn add_visual_interpolation_components(
    trigger: On<Add, Position>,
    query: Query<Entity, (With<Predicted>, Without<FloorMarker>)>,
    mut commands: Commands,
) {
    if !query.contains(trigger.entity) {
        return;
    }
    commands.entity(trigger.entity).insert((
        FrameInterpolate::<Position> {
            trigger_change_detection: true,
            ..default()
        },
        FrameInterpolate::<avian3d::prelude::Rotation> {
            trigger_change_detection: true,
            ..default()
        },
    ));
}

fn add_character_cosmetics(
    mut commands: Commands,
    character_query: Query<
        (Entity, &ColorComponent),
        (
            Or<(Added<Predicted>, Added<Replicate>, Added<Interpolated>)>,
            With<CharacterMarker>,
        ),
    >,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, color) in &character_query {
        info!(?entity, "Adding cosmetics to character");
        commands.entity(entity).insert((
            Mesh3d(meshes.add(Capsule3d::new(
                CHARACTER_CAPSULE_RADIUS,
                CHARACTER_CAPSULE_HEIGHT,
            ))),
            MeshMaterial3d(materials.add(color.0)),
        ));
    }
}

fn add_floor_cosmetics(
    mut commands: Commands,
    floor_query: Query<Entity, Added<FloorMarker>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for entity in &floor_query {
        info!(?entity, "Adding cosmetics to floor");
        commands.entity(entity).insert((
            Mesh3d(meshes.add(Cuboid::new(FLOOR_WIDTH, FLOOR_HEIGHT, FLOOR_WIDTH))),
            MeshMaterial3d(materials.add(Color::srgb(0.3, 0.5, 0.3))),
        ));
    }
}
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check -p render` compiles without errors

#### Manual Verification:
- [ ] Floor renders as green cuboid
- [ ] Characters render as colored capsules

---

## Phase 5: Web Client Updates

### Overview
Update web client to use SharedGameplayPlugin and ClientGameplayPlugin.

### Changes Required:

#### 1. Web Cargo.toml
**File**: `crates/web/Cargo.toml`
**Changes**: Add avian3d, leafwing-input-manager dependencies

```toml
[dependencies]
avian3d = { workspace = true }
leafwing-input-manager = { workspace = true }
```

#### 2. Web main.rs
**File**: `crates/web/src/main.rs`
**Changes**: Add SharedGameplayPlugin, import ClientGameplayPlugin from client crate

Since web crate can't easily import from client crate, duplicate the gameplay plugin or extract to protocol. For simplicity, we'll add a minimal spawn module to web.

Create `crates/web/src/gameplay.rs` with same content as client gameplay.rs.

Update `crates/web/src/main.rs`:
```rust
pub mod network;
pub mod gameplay;

use bevy::prelude::*;
use lightyear::prelude::client::*;
use network::WebClientPlugin;
use protocol::*;
use render::RenderPlugin;
use gameplay::ClientGameplayPlugin;
use ui::UiPlugin;
use std::time::Duration;

fn main() {
    console_error_panic_hook::set_once();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Bevy Lightyear Template".to_string(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(SharedGameplayPlugin)
        .add_plugins(WebClientPlugin::default())
        .add_plugins(ClientGameplayPlugin)
        .add_plugins(RenderPlugin)
        .add_plugins(UiPlugin)
        .run();
}
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check -p web` compiles without errors
- [ ] `bevy run web` builds WASM without errors

#### Manual Verification:
- [ ] Web client connects and shows floor + character

---

## Phase 6: Integration Testing

### Overview
Verify full client-server flow works correctly.

### Changes Required:

None - this phase is verification only.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo test-all` passes

#### Manual Verification:
- [ ] Start server: `cargo server`
- [ ] Start client 1: `cargo client -c 1`
- [ ] Start client 2: `cargo client -c 2`
- [ ] Both clients see floor and two characters
- [ ] Each client can move their character with WASD
- [ ] Each client can jump with Space when grounded
- [ ] Remote character movements appear smooth (interpolated)
- [ ] Local character movements are responsive (predicted)

---

## Testing Strategy

### Unit Tests:
- Protocol component serialization
- Physics bundle defaults

### Integration Tests:
- Server spawns floor on startup
- Server spawns character on client connect
- Client receives floor replication
- Client receives character prediction

### Manual Testing Steps:
1. Start server, verify floor spawn log
2. Connect client, verify character spawn log
3. Move with WASD, verify physics response
4. Jump with Space, verify ground detection
5. Connect second client, verify both see each other
6. Move one client, verify other sees smooth movement

## Performance Considerations

- `FrameInterpolate` only added to predicted entities (not floor)
- Physics runs at fixed 64Hz timestep
- Replication interval is 100ms (configurable)

## References

- Research: [2025-12-07-game-world-loading-validated.md](../research/2025-12-07-game-world-loading-validated.md)
- Lightyear example: `git/lightyear/examples/avian_3d_character/`
- Avian example: `git/avian/crates/avian3d/examples/dynamic_character_3d/`
