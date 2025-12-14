---
date: 2025-12-07T10:56:35-08:00
researcher: claude
git_commit: 24a300667401cda4ce8ab2e3abdde7c022b7fa39
branch: master
repository: bevy-lightyear-template
topic: "Game World Loading Implementation"
tags: [research, networking, physics, avian3d, lightyear, bevy, character-controller, prediction, interpolation]
status: complete
last_updated: 2025-12-07
last_updated_by: claude
---

# Research: Game World Loading Implementation

**Date**: 2025-12-07T10:56:35-08:00
**Researcher**: claude
**Git Commit**: 24a300667401cda4ce8ab2e3abdde7c022b7fa39
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to load the game world like the lightyear `avian_3d_character` example with:
- 3D map from Mesh and 3D pillbox physics character using avian3d + leafwing inputs
- Shared components in protocol crate
- Asset loader plugin triggered on client connect (general use, not character-specific)
-GameplayPlugins for client/server applying appropriate components
- Local character prediction, remote character interpolation

## Summary

### Key API Corrections from Previous Research

| Previous (Wrong) | Correct |
|------------------|---------|
| `app.replicate_once::<T>()` | `app.register_component::<T>()` |
| `app.replicate::<T>().add_prediction(ComponentSyncMode::Full)` | `app.register_component::<T>().add_prediction()` |
| `.add_linear_correction_fn(custom_fn)` | `.add_linear_correction_fn()` (no args, uses default) |
| `.add_rollback(\|a, b\| ...)` | `.add_should_rollback(\|a: &T, b: &T\| bool)` |
| `Has<ControlledBy>` in client query | `Has<Controlled>` (simpler marker) |

## Dependency Versions

| Crate | Version | Status |
|-------|---------|--------|
| bevy | 0.17.2 | Workspace dependency |
| lightyear | 0.25.5 | Workspace dependency |
| avian3d | 0.4.1 | Workspace dependency |
| leafwing-input-manager | 0.17+ | Needs adding |

## Architecture Overview

```
Server (authoritative, headless)
├── SharedGameplayPlugin (protocol)
│   ├── ProtocolPlugin         - Component registration
│   └── LightyearAvianPlugin   - Physics integration
├── ServerSpawnPlugin
│   ├── setup()                - Spawn floor on startup
│   └── handle_connected()     - Spawn character on client connect
└── MovementSystem             - Apply ActionState to physics (FixedUpdate)

Client (predicted + interpolated)
├── SharedGameplayPlugin (protocol)
├── ClientSpawnPlugin
│   ├── handle_new_floor()     - Added<Replicated> -> add physics + mesh
│   └── handle_new_character() - Added<Predicted> -> add physics + mesh + input
├── MovementSystem             - Same as server (shared)
└── RendererPlugin
    ├── Camera3d
    ├── PointLight
    └── FrameInterpolation     - Smooth visuals between ticks

Protocol (shared crate)
├── ProtocolPlugin             - Register components, inputs
├── SharedGameplayPlugin             - Physics + protocol setup (used by both client/server)
├── Marker components          - CharacterMarker, FloorMarker
├── Physics bundles            - CharacterPhysicsBundle, FloorPhysicsBundle
├── CharacterAction            - Leafwing Actionlike enum
└── Movement function          - apply_character_action()
```

## Detailed Implementation

### Phase 1: Protocol - Component Registration

**File**: `crates/protocol/src/lib.rs`

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;
use lightyear::avian3d::plugin::{AvianReplicationMode, LightyearAvianPlugin};
use lightyear::input::prelude::InputConfig;
use lightyear::prelude::input::leafwing;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};



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
                .disable::<PhysicsTransformPlugin>()
                .disable::<PhysicsInterpolationPlugin>(),
        );
    }
}



use avian3d::prelude::forces::ForcesItem;

pub fn apply_character_action(
    entity: Entity,
    mass: &ComputedMass,
    time: &Res<Time>,
    spatial_query: &SpatialQuery,
    action_state: &ActionState<CharacterAction>,
    mut forces: ForcesItem,
) {
    const MAX_SPEED: f32 = 5.0;
    const MAX_ACCELERATION: f32 = 20.0;

    let max_velocity_delta_per_tick = MAX_ACCELERATION * time.delta_secs();

    // Jump with raycast ground detection
    if action_state.just_pressed(&CharacterAction::Jump) {
        let ray_cast_origin = forces.position().0
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
            forces.apply_linear_impulse(Vec3::new(0.0, 5.0, 0.0));
        }
    }

    // Horizontal movement
    let move_dir = action_state
        .axis_pair(&CharacterAction::Move)
        .clamp_length_max(1.0);
    let move_dir = Vec3::new(-move_dir.x, 0.0, move_dir.y);

    let linear_velocity = forces.linear_velocity();
    let ground_linear_velocity = Vec3::new(linear_velocity.x, 0.0, linear_velocity.z);

    let desired_ground_linear_velocity = move_dir * MAX_SPEED;
    let new_ground_linear_velocity = ground_linear_velocity
        .move_towards(desired_ground_linear_velocity, max_velocity_delta_per_tick);

    let required_acceleration =
        (new_ground_linear_velocity - ground_linear_velocity) / time.delta_secs();

    forces.apply_force(required_acceleration * mass.value());
}
```

### Phase 2: Server - Spawn Plugin

**File**: `crates/server/src/spawn.rs`

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

pub struct ServerSpawnPlugin;

impl Plugin for ServerSpawnPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup);
        app.add_observer(handle_new_client);
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

pub fn handle_new_client(trigger: On<Add, LinkOf>, mut commands: Commands) {
    commands
        .entity(trigger.entity)
        .insert(ReplicationSender::new(
            core::time::Duration::from_millis(40),
            SendUpdatesMode::SinceLastAck,
            false,
        ));
}

pub fn handle_connected(
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
    mut query: Query<(Entity, &ComputedMass, &ActionState<CharacterAction>, Forces)>,
) {
    for (entity, mass, action_state, forces) in &mut query {
        apply_character_action(entity, mass, &time, &spatial_query, action_state, forces);
    }
}
```

### Phase 3: Client - Spawn Plugin

**File**: `crates/client/src/spawn.rs`

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use protocol::*;

pub struct ClientSpawnPlugin;

impl Plugin for ClientSpawnPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, handle_character_actions);
        app.add_systems(
            Update,
            (handle_new_floor, handle_new_character),
        );
    }
}

fn handle_character_actions(
    time: Res<Time>,
    spatial_query: SpatialQuery,
    mut query: Query<
        (Entity, &ComputedMass, &ActionState<CharacterAction>, Forces),
        With<Predicted>,
    >,
    timeline: Single<&LocalTimeline>,
) {
    for (entity, computed_mass, action_state, forces) in &mut query {
        apply_character_action(
            entity,
            computed_mass,
            &time,
            &spatial_query,
            action_state,
            forces,
        );
    }
}

fn handle_new_character(
    mut commands: Commands,
    mut character_query: Query<
        (Entity, &ColorComponent, Has<Controlled>),
        (Added<Predicted>, With<CharacterMarker>),
    >,
) {
    for (entity, _color, is_controlled) in &mut character_query {
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

### Phase 4: Renderer Plugin

**File**: `crates/render/src/lib.rs` (extend existing)

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::frame_interpolation::{FrameInterpolate, FrameInterpolationPlugin};
use protocol::*;

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_camera, setup_lighting));
        app.add_systems(Update, (add_character_cosmetics, add_floor_cosmetics));

        // FrameInterpolationPlugin for visual smoothing
        app.add_plugins(FrameInterpolationPlugin::<Position>::default());
        app.add_plugins(FrameInterpolationPlugin::<Rotation>::default());

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
        FrameInterpolate::<Rotation> {
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
            MeshMaterial3d(materials.add(Color::srgb(1.0, 1.0, 1.0))),
        ));
    }
}
```

### Phase 5: Asset Loader Plugin (General Purpose)

**File**: `crates/protocol/src/assets.rs`

```rust
use bevy::prelude::*;

/// Marker component for entities needing asset loading
#[derive(Component)]
pub struct NeedsAssets;

/// General-purpose asset handles resource
#[derive(Resource, Default)]
pub struct GameAssets {
    pub default_material: Option<Handle<StandardMaterial>>,
    pub ground_material: Option<Handle<StandardMaterial>>,
}

/// Asset loading states
#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum AssetLoadState {
    #[default]
    NotLoaded,
    Loading,
    Ready,
}

/// General-purpose asset loader plugin
/// Triggered on client connect, loads materials and other assets
pub struct AssetLoaderPlugin;

impl Plugin for AssetLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AssetLoadState>();
        app.init_resource::<GameAssets>();
        app.add_systems(OnEnter(AssetLoadState::Loading), load_assets);
        app.add_systems(
            Update,
            check_assets_ready.run_if(in_state(AssetLoadState::Loading)),
        );
    }
}

fn load_assets(
    mut assets: ResMut<GameAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Create programmatic materials (no gltf)
    assets.default_material = Some(materials.add(StandardMaterial {
        base_color: Color::srgb(0.8, 0.8, 0.8),
        ..default()
    }));

    assets.ground_material = Some(materials.add(StandardMaterial {
        base_color: Color::srgb(0.3, 0.5, 0.3),
        perceptual_roughness: 0.9,
        ..default()
    }));
}

fn check_assets_ready(
    assets: Res<GameAssets>,
    mut next_state: ResMut<NextState<AssetLoadState>>,
) {
    if assets.default_material.is_some() && assets.ground_material.is_some() {
        next_state.set(AssetLoadState::Ready);
    }
}
```

## File Structure

```
Cargo.toml                           # Add leafwing-input-manager to workspace
crates/
├── protocol/
│   ├── Cargo.toml                   # Add: avian3d, leafwing-input-manager
│   └── src/
│       ├── lib.rs                   # Markers, bundles, ProtocolPlugin, SharedGameplayPlugin
│       └── assets.rs                # AssetLoaderPlugin (optional)
├── server/
│   ├── Cargo.toml                   # Add: avian3d
│   └── src/
│       ├── main.rs                  # Add SharedGameplayPlugin, ServerSpawnPlugin
│       ├── lib.rs                   # pub mod spawn;
│       └── spawn.rs                 # ServerSpawnPlugin
├── client/
│   ├── Cargo.toml                   # Add: avian3d
│   └── src/
│       ├── main.rs                  # Add SharedGameplayPlugin, ClientSpawnPlugin
│       ├── lib.rs                   # pub mod spawn;
│       └── spawn.rs                 # ClientSpawnPlugin
└── render/
    └── src/
        └── lib.rs                   # Extended with cosmetics + interpolation
```

## Dependencies to Add

**Workspace `Cargo.toml`**:
```toml
[workspace.dependencies]
leafwing-input-manager = "0.17"
lightyear = { version = "0.25.5", features = ["frame_interpolation"] }
```

## Key Patterns

### 1. Component Registration Chain

```rust
app.register_component::<Position>()
    .add_prediction()                    // Enable prediction
    .add_should_rollback(fn_ptr)         // Rollback threshold
    .add_linear_correction_fn()          // Visual correction (no args)
    .add_linear_interpolation();         // Visual interpolation (no args)
```

### 2. Server Entity Spawning

```rust
commands.spawn((
    Replicate::to_clients(NetworkTarget::All),      // Replication
    PredictionTarget::to_clients(NetworkTarget::All), // Prediction for all
    ControlledBy { owner: trigger.entity, .. },     // Owner tracking
    ActionState::<CharacterAction>::default(),       // Input state
    CharacterPhysicsBundle::default(),               // Physics
    CharacterMarker,                                 // Marker
));
```

### 3. Client Entity Detection

```rust
// For predicted entities (characters):
Query<(Entity, Has<Controlled>), (Added<Predicted>, With<CharacterMarker>)>

// For replicated-only entities (floor):
Query<Entity, (Added<Replicated>, With<FloorMarker>)>
```

### 4. Leafwing Input Setup

```rust
// Actionlike impl (not derive):
impl Actionlike for CharacterAction {
    fn input_control_kind(&self) -> InputControlKind {
        match self {
            Self::Move => InputControlKind::DualAxis,
            Self::Jump => InputControlKind::Button,
        }
    }
}

// InputMap construction:
InputMap::new([(CharacterAction::Jump, KeyCode::Space)])
    .with_dual_axis(CharacterAction::Move, VirtualDPad::wasd())
```

## Code References

- `git/lightyear/examples/avian_3d_character/src/protocol.rs` - Component registration
- `git/lightyear/examples/avian_3d_character/src/server.rs` - Server spawning
- `git/lightyear/examples/avian_3d_character/src/client.rs` - Client spawn handling
- `git/lightyear/examples/avian_3d_character/src/shared.rs` - Physics setup, movement
- `git/lightyear/examples/avian_3d_character/src/renderer.rs` - Visual interpolation
- `git/avian/crates/avian3d/examples/dynamic_character_3d/plugin.rs` - CharacterPhysicsBundle
- `git/bevy/examples/3d/3d_scene.rs` - Mesh3d, MeshMaterial3d, Camera3d, PointLight

## Related Research

- [2025-12-06-map-loading-implementation.md](2025-12-06-map-loading-implementation.md) - Voxel-based approach (bevy_voxel_world)
