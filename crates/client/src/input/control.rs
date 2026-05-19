use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::Controlled;
use protocol::NetworkedPlayerActions;
use render::CameraOrbitState;

const CAMERA_ROTATION_STEP: f32 = std::f32::consts::FRAC_PI_2;

use super::ownership::ClientInputOwnershipSnapshot;
use super::raw::RawClientActions;

/// Copies ownership-filtered raw movement controls into networked input transport.
pub fn write_filtered_control_actions(
    ownership: Res<ClientInputOwnershipSnapshot>,
    mut camera_query: Query<&mut CameraOrbitState>,
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
    camera_query: &mut Query<&mut CameraOrbitState>,
) {
    let Ok(orbit) = camera_query.single_mut() else {
        trace!("write_filtered_control_actions: no unique camera orbit state");
        return;
    };
    raw_actions.set_value(&RawClientActions::CameraYaw, orbit.target_angle);
}

fn write_camera_rotation(
    ownership: &ClientInputOwnershipSnapshot,
    raw_actions: &mut ActionState<RawClientActions>,
    camera_query: &mut Query<&mut CameraOrbitState>,
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
    let Ok(mut orbit) = camera_query.single_mut() else {
        trace!("write_filtered_control_actions: no unique camera orbit state for rotation");
        return;
    };

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
