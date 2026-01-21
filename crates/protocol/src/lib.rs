use avian3d::prelude::{forces::ForcesItem, *};
use bevy::prelude::*;
use bevy_voxel_world::prelude::ChunkRenderTarget;
use leafwing_input_manager::prelude::*;
use lightyear::input::config::InputConfig;
use lightyear::prelude::input::leafwing::InputPlugin;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

pub mod map;
pub use map::{
    attach_chunk_colliders, MapWorld, VoxelChannel, VoxelEditBroadcast, VoxelEditRequest,
    VoxelStateSync, VoxelType,
};

pub const PROTOCOL_ID: u64 = 0;
pub const PRIVATE_KEY: [u8; 32] = [0; 32];
pub const FIXED_TIMESTEP_HZ: f64 = 64.0;

pub const CHARACTER_CAPSULE_RADIUS: f32 = 0.5;
pub const CHARACTER_CAPSULE_HEIGHT: f32 = 0.5;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy, Hash, Reflect)]
pub enum PlayerActions {
    Move,
    Jump,
    PlaceVoxel,
    RemoveVoxel,
}

impl Actionlike for PlayerActions {
    fn input_control_kind(&self) -> InputControlKind {
        match self {
            Self::Move => InputControlKind::DualAxis,
            Self::Jump | Self::PlaceVoxel | Self::RemoveVoxel => InputControlKind::Button,
        }
    }
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CharacterMarker;

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ColorComponent(pub Color);

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
            locked_axes: LockedAxes::ROTATION_LOCKED,
            friction: Friction::new(0.0).with_combine_rule(CoefficientCombine::Min),
        }
    }
}

#[cfg(feature = "test_utils")]
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Reflect, Event)]
pub struct TestTrigger {
    pub data: String,
}

pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InputPlugin::<PlayerActions> {
            config: InputConfig::<PlayerActions> {
                rebroadcast_inputs: true,
                ..default()
            },
        });

        // Voxel channel
        app.add_channel::<VoxelChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);

        // Voxel messages
        app.register_message::<VoxelEditRequest>()
            .add_direction(NetworkDirection::ClientToServer);
        app.register_message::<VoxelEditBroadcast>()
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message::<VoxelStateSync>()
            .add_direction(NetworkDirection::ServerToClient);

        #[cfg(feature = "test_utils")]
        app.register_event::<TestTrigger>()
            .add_direction(NetworkDirection::Bidirectional);

        app.register_component::<ChunkRenderTarget<MapWorld>>();
        // Marker components
        app.register_component::<ColorComponent>();
        app.register_component::<Name>();
        app.register_component::<CharacterMarker>();

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

        app.add_plugins(lightyear::avian3d::plugin::LightyearAvianPlugin {
            replication_mode: lightyear::avian3d::plugin::AvianReplicationMode::Position,
            ..default()
        });

        app.add_plugins(
            PhysicsPlugins::default()
                .build()
                .disable::<PhysicsTransformPlugin>()
                .disable::<PhysicsInterpolationPlugin>()
                .disable::<IslandSleepingPlugin>(),
        );
    }
}

/// Apply movement based on input direction and jump flag.
/// Movement uses acceleration-based physics with ground detection for jumping.
pub fn apply_movement(
    entity: Entity,
    mass: &ComputedMass,
    delta_secs: f32,
    spatial_query: &SpatialQuery,
    action_state: &ActionState<PlayerActions>,
    position: &Position,
    forces: &mut ForcesItem,
) {
    const MAX_SPEED: f32 = 5.0;
    const MAX_ACCELERATION: f32 = 20.0;

    let max_velocity_delta_per_tick = MAX_ACCELERATION * delta_secs;

    // Jump with raycast ground detection
    if action_state.just_pressed(&PlayerActions::Jump) {
        let ray_cast_origin = position.0;

        let filter = &SpatialQueryFilter::from_excluded_entities([entity]);

        if spatial_query
            .cast_ray(ray_cast_origin, Dir3::NEG_Y, 1.0, false, filter)
            .is_some()
        {
            println!("Jump!");
            forces.apply_linear_impulse(Vec3::new(0.0, 5.0, 0.0));
        } else {
            println!("No jumper");
        }
    }

    // Horizontal movement
    let move_dir = action_state
        .axis_pair(&PlayerActions::Move)
        .clamp_length_max(1.0);
    let move_dir = Vec3::new(-move_dir.x, 0.0, move_dir.y);

    let linear_velocity = forces.linear_velocity();
    let ground_linear_velocity = Vec3::new(linear_velocity.x, 0.0, linear_velocity.z);

    let desired_ground_linear_velocity = move_dir * MAX_SPEED;
    let new_ground_linear_velocity = ground_linear_velocity
        .move_towards(desired_ground_linear_velocity, max_velocity_delta_per_tick);

    let required_acceleration = (new_ground_linear_velocity - ground_linear_velocity) / delta_secs;

    forces.apply_force(required_acceleration * mass.value());
}

#[cfg(feature = "test_utils")]
pub mod test_utils;
