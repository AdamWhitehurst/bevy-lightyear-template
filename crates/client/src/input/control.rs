use avian3d::prelude::Position;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::{Controlled, Predicted};
use protocol::{CharacterMarker, NetworkedPlayerActions};
use render::{release_lock_on, CameraOrbitState, LockOnTarget};

const CAMERA_ROTATION_STEP: f32 = std::f32::consts::FRAC_PI_2;
/// Maximum player↔character distance at which Tab acquires a lock-on.
const LOCK_ON_ACQUIRE_DISTANCE: f32 = 40.0;

use super::ownership::ClientInputOwnershipSnapshot;
use super::raw::RawClientActions;

/// Copies ownership-filtered raw movement controls into networked input transport.
pub fn write_filtered_control_actions(
    ownership: Res<ClientInputOwnershipSnapshot>,
    mut camera_query: Query<(&mut CameraOrbitState, Option<&mut LockOnTarget>)>,
    mut query: Query<
        (
            &mut ActionState<RawClientActions>,
            &mut ActionState<NetworkedPlayerActions>,
        ),
        With<Controlled>,
    >,
) {
    for (mut raw_actions, mut networked_actions) in &mut query {
        sync_raw_camera_yaw(&mut raw_actions, &mut camera_query);
        write_camera_rotation(&ownership, &mut raw_actions, &mut camera_query);
        write_movement(&ownership, &raw_actions, &mut networked_actions);
        write_jump(&ownership, &raw_actions, &mut networked_actions);
        write_camera_yaw(&ownership, &raw_actions, &mut networked_actions);
    }
}

fn write_movement(
    ownership: &ClientInputOwnershipSnapshot,
    raw_actions: &ActionState<RawClientActions>,
    networked_actions: &mut ActionState<NetworkedPlayerActions>,
) {
    let movement = if ownership.keyboard.allows_locomotion() {
        raw_actions
            .axis_pair(&RawClientActions::Move)
            .clamp_length_max(1.0)
    } else {
        trace!(
            owner = ?ownership.keyboard,
            "write_filtered_control_actions: locomotion input suppressed"
        );
        Vec2::ZERO
    };

    networked_actions.set_axis_pair(&NetworkedPlayerActions::Move, movement);
}

fn write_jump(
    ownership: &ClientInputOwnershipSnapshot,
    raw_actions: &ActionState<RawClientActions>,
    networked_actions: &mut ActionState<NetworkedPlayerActions>,
) {
    if ownership.keyboard.allows_jump() && raw_actions.pressed(&RawClientActions::Jump) {
        networked_actions.press(&NetworkedPlayerActions::Jump);
    } else {
        if !ownership.keyboard.allows_jump() && raw_actions.pressed(&RawClientActions::Jump) {
            trace!(
                owner = ?ownership.keyboard,
                "write_filtered_control_actions: jump input suppressed"
            );
        }
        networked_actions.release(&NetworkedPlayerActions::Jump);
    }
}

fn sync_raw_camera_yaw(
    raw_actions: &mut ActionState<RawClientActions>,
    camera_query: &mut Query<(&mut CameraOrbitState, Option<&mut LockOnTarget>)>,
) {
    let Ok((orbit, _)) = camera_query.single_mut() else {
        trace!("write_filtered_control_actions: no unique camera orbit state");
        return;
    };
    raw_actions.set_value(&RawClientActions::CameraYaw, orbit.target_angle);
}

fn write_camera_rotation(
    ownership: &ClientInputOwnershipSnapshot,
    raw_actions: &mut ActionState<RawClientActions>,
    camera_query: &mut Query<(&mut CameraOrbitState, Option<&mut LockOnTarget>)>,
) {
    if !raw_actions.just_pressed(&RawClientActions::CameraRotateLeft)
        && !raw_actions.just_pressed(&RawClientActions::CameraRotateRight)
    {
        trace!("write_filtered_control_actions: no camera rotation input pressed");
        return;
    }
    if !ownership.keyboard.allows_camera_control() {
        trace!(
            owner = ?ownership.keyboard,
            "write_filtered_control_actions: camera rotation input suppressed by keyboard ownership"
        );
        return;
    }
    let Ok((mut orbit, lock)) = camera_query.single_mut() else {
        trace!("write_filtered_control_actions: no unique camera orbit state for rotation");
        return;
    };

    if let Some(mut lock) = lock {
        // While locked on, Q/E swing the camera 180° to the other side of the
        // line of action instead of stepping the orbit; steer_lock_on_camera
        // derives the new target angle from the flipped side.
        lock.side = -lock.side;
        return;
    }

    let mut target_angle = orbit.target_angle;
    if raw_actions.just_pressed(&RawClientActions::CameraRotateLeft) {
        target_angle += CAMERA_ROTATION_STEP;
    }
    if raw_actions.just_pressed(&RawClientActions::CameraRotateRight) {
        target_angle -= CAMERA_ROTATION_STEP;
    }
    orbit.target_angle = target_angle;
    raw_actions.set_value(&RawClientActions::CameraYaw, target_angle);
}

/// Toggles camera lock-on: Tab acquires the nearest character or releases the current lock.
pub fn write_lock_on_toggle(
    mut commands: Commands,
    ownership: Res<ClientInputOwnershipSnapshot>,
    player_query: Query<(&Position, &ActionState<RawClientActions>), With<Controlled>>,
    candidates: Query<
        (Entity, &Position),
        (With<CharacterMarker>, With<Predicted>, Without<Controlled>),
    >,
    mut camera_query: Query<(Entity, &mut CameraOrbitState, Has<LockOnTarget>), With<Camera3d>>,
) {
    let Ok((player_pos, raw_actions)) = player_query.single() else {
        trace!("write_lock_on_toggle: controlled player is not available yet");
        return;
    };
    if !raw_actions.just_pressed(&RawClientActions::ToggleLockOn) {
        trace!("write_lock_on_toggle: no lock-on input pressed");
        return;
    }
    if !ownership.keyboard.allows_camera_control() {
        trace!(
            owner = ?ownership.keyboard,
            "write_lock_on_toggle: lock-on input suppressed by keyboard ownership"
        );
        return;
    }
    let Ok((camera_entity, mut orbit, locked)) = camera_query.single_mut() else {
        trace!("write_lock_on_toggle: camera is not available yet");
        return;
    };

    if locked {
        release_lock_on(&mut commands, camera_entity, &mut orbit);
        return;
    }
    let Some(target) = nearest_lock_on_candidate(player_pos.0, &candidates) else {
        trace!("write_lock_on_toggle: no character within lock-on range");
        return;
    };
    commands
        .entity(camera_entity)
        .insert(LockOnTarget { target, side: 1.0 });
}

/// Returns the closest predicted character within acquire range. Room-scoped
/// replication guarantees every candidate is already on the player's map.
fn nearest_lock_on_candidate(
    player_pos: Vec3,
    candidates: &Query<
        (Entity, &Position),
        (With<CharacterMarker>, With<Predicted>, Without<Controlled>),
    >,
) -> Option<Entity> {
    candidates
        .iter()
        .map(|(entity, pos)| (entity, pos.0.distance(player_pos)))
        .filter(|(_, distance)| *distance <= LOCK_ON_ACQUIRE_DISTANCE)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(entity, _)| entity)
}

fn write_camera_yaw(
    ownership: &ClientInputOwnershipSnapshot,
    raw_actions: &ActionState<RawClientActions>,
    networked_actions: &mut ActionState<NetworkedPlayerActions>,
) {
    if ownership.keyboard.allows_camera_control() {
        networked_actions.set_value(
            &NetworkedPlayerActions::CameraYaw,
            raw_actions.value(&RawClientActions::CameraYaw),
        );
    } else {
        trace!(
            owner = ?ownership.keyboard,
            "write_filtered_control_actions: camera yaw input suppressed by keyboard ownership"
        );
    }
}
