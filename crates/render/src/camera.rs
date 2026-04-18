use avian3d::prelude::Position;
use bevy::prelude::*;
use lightyear::prelude::*;

const BASE_OFFSET: Vec3 = Vec3::new(0.0, 18.0, -36.0);
const BASE_LIGHT_OFFSET: Vec3 = Vec3::new(8.0, 16.0, 8.0);
const ORBIT_LERP_SPEED: f32 = 20.0;

/// Orbital camera state for discrete 90° rotation around the player.
#[derive(Component)]
pub struct CameraOrbitState {
    /// Target angle in radians (one of 0, π/2, π, 3π/2)
    pub target_angle: f32,
    /// Current angle in radians (lerps toward target)
    pub current_angle: f32,
}

impl Default for CameraOrbitState {
    fn default() -> Self {
        Self {
            target_angle: 0.0,
            current_angle: 0.0,
        }
    }
}

/// Marker for the main scene light that follows camera rotation.
#[derive(Component)]
pub struct MainLight;

pub(crate) fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 18.0, -36.0).looking_at(Vec3::ZERO, Dir3::Y),
        CameraOrbitState::default(),
    ));
}

pub(crate) fn setup_lighting(mut commands: Commands) {
    commands.spawn((
        PointLight {
            shadows_enabled: true,
            ..default()
        },
        Transform::from_translation(BASE_LIGHT_OFFSET),
        MainLight,
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: light_consts::lux::AMBIENT_DAYLIGHT,
            shadows_enabled: true,
            ..default()
        },
        Transform::default().looking_to(Vec3::new(-0.5, -1.0, -0.5), Vec3::Y),
    ));
}

/// Handles Q/E input to rotate camera orbit by 90° increments.
pub(crate) fn handle_camera_rotation_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut query: Query<&mut CameraOrbitState>,
) {
    let Ok(mut orbit) = query.single_mut() else {
        return;
    };

    if keys.just_pressed(KeyCode::KeyQ) {
        orbit.target_angle += std::f32::consts::FRAC_PI_2;
    }
    if keys.just_pressed(KeyCode::KeyE) {
        orbit.target_angle -= std::f32::consts::FRAC_PI_2;
    }
}

/// Lerps camera orbit angle toward the target using frame-rate-independent exponential approach.
pub(crate) fn update_camera_orbit(time: Res<Time>, mut query: Query<&mut CameraOrbitState>) {
    let dt = time.delta_secs();
    let lerp_factor = (ORBIT_LERP_SPEED * dt).min(1.0);

    for mut orbit in &mut query {
        let diff = orbit.target_angle - orbit.current_angle;
        if diff.abs() > 0.001 {
            orbit.current_angle += diff * lerp_factor;
        } else {
            orbit.current_angle = orbit.target_angle;
        }
    }
}

pub(crate) fn follow_player(
    player_query: Query<&Position, With<Controlled>>,
    mut camera_query: Query<(&mut Transform, &CameraOrbitState), With<Camera3d>>,
) {
    let Ok(player_pos) = player_query.single() else {
        return;
    };
    let Ok((mut camera_transform, orbit)) = camera_query.single_mut() else {
        return;
    };

    let rotated_offset = Quat::from_rotation_y(orbit.current_angle) * BASE_OFFSET;
    camera_transform.translation = **player_pos + rotated_offset;
    camera_transform.look_at(**player_pos, Dir3::Y);
}

/// Updates light position to follow camera rotation around the player.
pub(crate) fn update_light_position(
    player_query: Query<&Position, With<Controlled>>,
    camera_query: Query<&CameraOrbitState>,
    mut light_query: Query<&mut Transform, With<MainLight>>,
) {
    let Ok(player_pos) = player_query.single() else {
        return;
    };
    let Ok(orbit) = camera_query.single() else {
        return;
    };
    let Ok(mut light_transform) = light_query.single_mut() else {
        return;
    };

    let rotated_offset = Quat::from_rotation_y(orbit.current_angle) * BASE_LIGHT_OFFSET;
    light_transform.translation = **player_pos + rotated_offset;
}
