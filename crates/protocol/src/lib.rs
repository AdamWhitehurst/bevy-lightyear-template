use avian3d::prelude::{forces::ForcesItem, *};
use bevy::prelude::*;
use bevy_voxel_world::prelude::ChunkRenderTarget;
use leafwing_input_manager::prelude::*;
use lightyear::input::config::InputConfig;
use lightyear::prelude::input::leafwing::InputPlugin;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

pub mod ability;
pub mod app_state;
pub mod hit_detection;
pub mod map;

pub use ability::{
    ability_action_to_slot, AbilityBulletOf, AbilityBullets, AbilityCooldowns, AbilityDef,
    AbilityDefs, AbilityEffect, AbilityId, AbilityManifest, AbilityPhase, AbilityPlugin,
    AbilityProjectileSpawn, AbilitySlots, ActiveAbility, ActiveBuff, ActiveBuffs, ActiveShield,
    DefaultAbilitySlots, EffectTarget, EffectTrigger, ForceFrame, OnHitEffects,
    ProjectileSpawnEffect,
};
pub use app_state::{AppState, AppStatePlugin, TrackedAssets};
pub use hit_detection::{
    character_collision_layers, hitbox_collision_layers, projectile_collision_layers,
    terrain_collision_layers, GameLayer,
};
pub use map::{
    attach_chunk_colliders, MapWorld, VoxelChannel, VoxelEditBroadcast, VoxelEditRequest,
    VoxelStateSync, VoxelType,
};

pub const PROTOCOL_ID: u64 = 0;
pub const PRIVATE_KEY: [u8; 32] = [0; 32];
pub const FIXED_TIMESTEP_HZ: f64 = 64.0;

pub const CHARACTER_CAPSULE_RADIUS: f32 = 1.0;
pub const CHARACTER_CAPSULE_HEIGHT: f32 = 1.0;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy, Hash, Reflect)]
pub enum PlayerActions {
    Move,
    Jump,
    PlaceVoxel,
    RemoveVoxel,
    Ability1,
    Ability2,
    Ability3,
    Ability4,
}

impl Actionlike for PlayerActions {
    fn input_control_kind(&self) -> InputControlKind {
        match self {
            Self::Move => InputControlKind::DualAxis,
            _ => InputControlKind::Button,
        }
    }
}

/// Identifies which client owns this character. Replicated to all clients so
/// shared systems (e.g. prespawn salt computation) can access the owner's
/// `PeerId` without server-only queries.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub struct PlayerId(pub PeerId);

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CharacterMarker;

/// Marker to distinguish dummy targets from player characters.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DummyTarget;

/// Marks a respawn location. Server-only, not replicated.
#[derive(Component, Clone, Debug)]
pub struct RespawnPoint;

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    pub fn new(max: f32) -> Self {
        Self { current: max, max }
    }

    pub fn apply_damage(&mut self, damage: f32) {
        self.current = (self.current - damage).max(0.0);
    }

    pub fn is_dead(&self) -> bool {
        self.current <= 0.0
    }

    pub fn restore_full(&mut self) {
        self.current = self.max;
    }
}

/// Post-respawn invulnerability. Prevents damage while present.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Invulnerable {
    pub expires_at: Tick,
}

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ColorComponent(pub Color);

#[derive(Bundle)]
pub struct CharacterPhysicsBundle {
    pub collider: Collider,
    pub rigid_body: RigidBody,
    pub locked_axes: LockedAxes,
    pub friction: Friction,
    pub collision_layers: CollisionLayers,
}

impl Default for CharacterPhysicsBundle {
    fn default() -> Self {
        Self {
            collider: Collider::capsule(CHARACTER_CAPSULE_RADIUS, CHARACTER_CAPSULE_HEIGHT),
            rigid_body: RigidBody::Dynamic,
            locked_axes: LockedAxes::ROTATION_LOCKED,
            friction: Friction::default(),
            collision_layers: hit_detection::character_collision_layers(),
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
        app.register_component::<PlayerId>();
        app.register_component::<ColorComponent>().add_prediction();
        app.register_component::<Name>();
        app.register_component::<CharacterMarker>().add_prediction();
        app.register_component::<DummyTarget>().add_prediction();
        app.register_component::<Health>().add_prediction();
        app.register_component::<Invulnerable>().add_prediction();

        // Velocity prediction without visual correction
        app.register_component::<LinearVelocity>()
            .add_prediction()
            .add_should_rollback(linear_velocity_should_rollback);

        app.register_component::<AngularVelocity>()
            .add_prediction()
            .add_should_rollback(angular_velocity_should_rollback);

        // Ability components
        app.register_component::<AbilitySlots>();
        app.register_component::<ActiveAbility>()
            .add_prediction()
            .add_map_entities();
        app.register_component::<AbilityCooldowns>()
            .add_prediction();
        app.register_component::<ActiveShield>().add_prediction();
        app.register_component::<ActiveBuffs>().add_prediction();
        app.register_component::<AbilityProjectileSpawn>();

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
        app.add_plugins(AppStatePlugin);
        app.add_plugins(ProtocolPlugin);
        app.add_plugins(AbilityPlugin);

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

        let ready = in_state(AppState::Ready);

        app.add_systems(
            FixedUpdate,
            (
                ability::ability_activation,
                ability::update_active_abilities,
                ability::dispatch_effect_markers,
                ability::apply_on_tick_effects,
                ability::apply_while_active_effects,
                ability::apply_on_end_effects,
                ability::apply_on_input_effects,
                ability::ability_projectile_spawn,
            )
                .chain()
                .run_if(ready.clone()),
        );

        app.add_systems(
            FixedUpdate,
            (
                hit_detection::update_hitbox_positions,
                hit_detection::process_hitbox_hits,
                hit_detection::process_projectile_hits,
                hit_detection::cleanup_hitbox_entities,
            )
                .chain()
                .after(ability::apply_on_tick_effects)
                .run_if(ready.clone()),
        );

        app.add_systems(FixedUpdate, ability::expire_buffs.run_if(ready.clone()));
        app.add_systems(
            FixedUpdate,
            ability::aoe_hitbox_lifetime.run_if(ready.clone()),
        );
        app.add_systems(FixedUpdate, update_facing.run_if(ready.clone()));
        app.add_systems(PreUpdate, ability::handle_ability_projectile_spawn);
        app.add_systems(FixedUpdate, ability::ability_bullet_lifetime.run_if(ready));
        app.add_observer(ability::despawn_ability_projectile_spawn);
        app.add_observer(ability::cleanup_effect_markers_on_removal);
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
            .cast_ray(ray_cast_origin, Dir3::NEG_Y, 2.0, false, filter)
            .is_some()
        {
            forces.apply_linear_impulse(Vec3::new(0.0, 50.0, 0.0));
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

/// Update character facing direction based on movement input.
/// Separate from `apply_movement` because `Forces` already accesses `Rotation`.
pub fn update_facing(
    mut query: Query<(&ActionState<PlayerActions>, &mut Rotation), With<CharacterMarker>>,
) {
    for (action_state, mut rotation) in &mut query {
        let move_dir = action_state
            .axis_pair(&PlayerActions::Move)
            .clamp_length_max(1.0);
        if move_dir != Vec2::ZERO {
            *rotation = Rotation(Quat::from_rotation_y(f32::atan2(move_dir.x, -move_dir.y)));
        }
    }
}

#[cfg(feature = "test_utils")]
pub mod test_utils;
