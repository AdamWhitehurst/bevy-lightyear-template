use super::types::{CharacterMarker, IsGrounded};
use crate::map::MapInstanceId;
use crate::PlayerActions;
use avian3d::prelude::{forces::ForcesItem, *};
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
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
    player_map_id: Option<&MapInstanceId>,
    map_ids: &Query<&MapInstanceId>,
) {
    const MAX_SPEED: f32 = 15.0;
    const MAX_ACCELERATION: f32 = 500.0;

    let max_velocity_delta_per_tick = MAX_ACCELERATION * delta_secs;

    if action_state.just_pressed(&PlayerActions::Jump) {
        let ray_cast_origin = position.0;

        let filter = SpatialQueryFilter::from_excluded_entities([entity]);

        if spatial_query
            .cast_ray_predicate(
                ray_cast_origin,
                Dir3::NEG_Y,
                4.0,
                false,
                &filter,
                &|hit_entity| match (player_map_id, map_ids.get(hit_entity).ok()) {
                    (Some(a), Some(b)) => a == b,
                    _ => true,
                },
            )
            .is_some()
        {
            forces.apply_linear_impulse(Vec3::new(0.0, 2000.0, 0.0));
        }
    }

    // Horizontal movement (camera-relative)
    let move_dir = action_state
        .axis_pair(&PlayerActions::Move)
        .clamp_length_max(1.0);
    let yaw = action_state.value(&PlayerActions::CameraYaw);
    let move_dir = Quat::from_rotation_y(yaw) * Vec3::new(-move_dir.x, 0.0, move_dir.y);

    let linear_velocity = forces.linear_velocity();
    let ground_linear_velocity = Vec3::new(linear_velocity.x, 0.0, linear_velocity.z);

    let desired_ground_linear_velocity = move_dir * MAX_SPEED;
    let new_ground_linear_velocity = ground_linear_velocity
        .move_towards(desired_ground_linear_velocity, max_velocity_delta_per_tick);

    let required_acceleration = (new_ground_linear_velocity - ground_linear_velocity) / delta_secs;

    forces.apply_force(required_acceleration * mass.value());
}

/// Maintains the `IsGrounded` marker on character entities by ray casting
/// downward from the capsule center each tick. Must run before
/// `handle_character_movement` and `ability_activation` so consumers see a
/// fresh marker.
pub fn detect_grounded(
    mut commands: Commands,
    spatial_query: SpatialQuery,
    map_ids: Query<&MapInstanceId>,
    characters: Query<
        (Entity, &Position, Option<&MapInstanceId>, Has<IsGrounded>),
        With<CharacterMarker>,
    >,
) {
    for (entity, position, player_map_id, has_grounded) in &characters {
        let filter = SpatialQueryFilter::from_excluded_entities([entity]);
        let hit = spatial_query
            .cast_ray_predicate(
                position.0,
                Dir3::NEG_Y,
                4.0,
                false,
                &filter,
                &|hit_entity| match (player_map_id, map_ids.get(hit_entity).ok()) {
                    (Some(a), Some(b)) => a == b,
                    _ => true,
                },
            )
            .is_some();
        match (hit, has_grounded) {
            (true, false) => {
                commands.entity(entity).insert(IsGrounded);
            }
            (false, true) => {
                commands.entity(entity).remove::<IsGrounded>();
            }
            _ => {}
        }
    }
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
            let yaw = action_state.value(&PlayerActions::CameraYaw);
            *rotation = Rotation(Quat::from_rotation_y(
                f32::atan2(move_dir.x, -move_dir.y) + yaw,
            ));
        }
    }
}
