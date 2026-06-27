use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};

use avian3d::prelude::Position;
use bevy::camera::ScalingMode;
use bevy::image::{ImageAddressMode, ImageLoaderSettings};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use lightyear::prelude::*;
use protocol::RespawnTimer;

/// Camera down-tilt in degrees, measured from the horizontal. Common choices:
/// 26.57° (2:1 pixel iso), 30° (≈ reference), 35.26° (true iso), 45° (military),
/// 60° (near overhead). Yaw stays the 45° corner view regardless of this value.
const CAMERA_PITCH_DEGREES: f32 = 30.0;
const BASE_LIGHT_OFFSET: Vec3 = Vec3::new(8.0, 16.0, 8.0);
const ORBIT_LERP_SPEED: f32 = 20.0;
/// Fixed camera arm length. Under orthographic projection, distance along the view
/// axis does not change the image, so the camera sits far enough back that the whole
/// scene stays at positive depth (in front of the `near = 0` plane) — well clear of
/// any terrain rising toward it, yet inside the background sphere so it stays a backdrop.
const CAMERA_DISTANCE: f32 = 100.0;
/// World-space vertical extent the orthographic camera shows at base zoom (`scale = 1.0`).
const CAMERA_VIEW_HEIGHT: f32 = 33.0;
/// Frame-rate-independent approach speed for the orthographic zoom (lock-on framing).
const CAMERA_ZOOM_LERP_SPEED: f32 = 8.0;
/// Lock-on releases automatically beyond this player↔target distance.
const LOCK_ON_BREAK_DISTANCE: f32 = 60.0;
/// Extra world-space margin kept around both characters when framing a lock-on.
const LOCK_ON_FRAME_MARGIN: f32 = 4.0;
const BACKGROUND_IMAGE: &str = "sprites/background.png";
const BACKGROUND_RADIUS: f32 = 1000.0;
const BACKGROUND_LATITUDE_SEGMENTS: u32 = 32;
const BACKGROUND_LONGITUDE_SEGMENTS: u32 = 64;
const BACKGROUND_TILES_X: f32 = 6.0;
const BACKGROUND_TILES_Y: f32 = 4.0;

/// Orbital camera state for discrete 90° rotation around the player.
#[derive(Component)]
pub struct CameraOrbitState {
    /// Target yaw in radians (one of the corner rest angles 45°/135°/225°/315°).
    pub target_angle: f32,
    /// Current yaw in radians (lerps toward target).
    pub current_angle: f32,
}

impl Default for CameraOrbitState {
    fn default() -> Self {
        Self {
            target_angle: FRAC_PI_4,
            current_angle: FRAC_PI_4,
        }
    }
}

/// Active camera lock-on: frames the line of action between the player and a target.
#[derive(Component)]
pub struct LockOnTarget {
    /// The locked character entity (client-side predicted entity).
    pub target: Entity,
    /// Which side of the line of action the camera sits on: `+1.0` or `-1.0`.
    pub side: f32,
}

/// Removes the lock and snaps the orbit target back onto the discrete corner grid.
pub fn release_lock_on(
    commands: &mut Commands,
    camera_entity: Entity,
    orbit: &mut CameraOrbitState,
) {
    orbit.target_angle = nearest_orbit_rest_angle(orbit.target_angle);
    commands.entity(camera_entity).remove::<LockOnTarget>();
}

/// Snaps a yaw to the nearest 45°-offset corner rest angle (45°/135°/225°/315°),
/// the orientations that view axis-aligned world geometry corner-on.
fn nearest_orbit_rest_angle(angle: f32) -> f32 {
    ((angle - FRAC_PI_4) / FRAC_PI_2).round() * FRAC_PI_2 + FRAC_PI_4
}

/// Marker for the main scene light that follows camera rotation.
#[derive(Component)]
pub struct MainLight;

/// Spherical background that follows camera translation without inheriting rotation.
#[derive(Component)]
pub struct BackgroundSphere;

/// Unit camera-arm direction (camera position relative to its anchor) at the rest yaw,
/// before the orbit yaw is applied. Tilt comes from [`CAMERA_PITCH_DEGREES`].
fn base_camera_direction() -> Vec3 {
    let pitch = CAMERA_PITCH_DEGREES.to_radians();
    Vec3::new(0.0, pitch.sin(), -pitch.cos())
}

pub(crate) fn setup_camera(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let rest_direction = Quat::from_rotation_y(FRAC_PI_4) * base_camera_direction();
    let camera_transform = Transform::from_translation(rest_direction * CAMERA_DISTANCE)
        .looking_at(Vec3::ZERO, Dir3::Y);
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: CAMERA_VIEW_HEIGHT,
            },
            ..OrthographicProjection::default_3d()
        }),
        camera_transform,
        CameraOrbitState::default(),
    ));

    commands.spawn((
        BackgroundSphere,
        NotShadowCaster,
        NotShadowReceiver,
        Mesh3d(meshes.add(background_sphere_mesh())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(
                asset_server.load_with_settings::<Image, ImageLoaderSettings>(
                    BACKGROUND_IMAGE,
                    |settings| {
                        settings
                            .sampler
                            .get_or_init_descriptor()
                            .set_address_mode(ImageAddressMode::Repeat);
                    },
                ),
            ),
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
        Transform::from_translation(camera_transform.translation),
    ));
}

/// Creates a UV sphere mesh centered at origin for the camera to sit inside.
fn background_sphere_mesh() -> Mesh {
    let row_width = BACKGROUND_LONGITUDE_SEGMENTS + 1;
    let vertex_count = ((BACKGROUND_LATITUDE_SEGMENTS + 1) * row_width) as usize;
    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    let mut uvs = Vec::with_capacity(vertex_count);
    let mut indices = Vec::with_capacity(
        (BACKGROUND_LATITUDE_SEGMENTS * BACKGROUND_LONGITUDE_SEGMENTS * 6) as usize,
    );

    for latitude in 0..=BACKGROUND_LATITUDE_SEGMENTS {
        let v_fraction = latitude as f32 / BACKGROUND_LATITUDE_SEGMENTS as f32;
        let polar_angle = v_fraction * std::f32::consts::PI;
        let y = BACKGROUND_RADIUS * polar_angle.cos();
        let ring_radius = BACKGROUND_RADIUS * polar_angle.sin();

        for longitude in 0..=BACKGROUND_LONGITUDE_SEGMENTS {
            let u_fraction = longitude as f32 / BACKGROUND_LONGITUDE_SEGMENTS as f32;
            let azimuth = u_fraction * std::f32::consts::TAU;
            let (sin, cos) = azimuth.sin_cos();
            let normal = [
                ring_radius.signum() * cos,
                polar_angle.cos(),
                ring_radius.signum() * sin,
            ];

            positions.push([ring_radius * cos, y, ring_radius * sin]);
            normals.push(normal);
            uvs.push([
                u_fraction * BACKGROUND_TILES_X,
                v_fraction * BACKGROUND_TILES_Y,
            ]);
        }
    }

    for latitude in 0..BACKGROUND_LATITUDE_SEGMENTS {
        for longitude in 0..BACKGROUND_LONGITUDE_SEGMENTS {
            let lower_left = latitude * row_width + longitude;
            let lower_right = lower_left + 1;
            let upper_left = lower_left + row_width;
            let upper_right = upper_left + 1;
            indices.extend_from_slice(&[
                lower_left,
                upper_left,
                upper_right,
                lower_left,
                upper_right,
                lower_right,
            ]);
        }
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
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

/// Lerps camera orbit angle toward the target using frame-rate-independent exponential approach.
pub(crate) fn update_camera_orbit(time: Res<Time>, mut query: Query<&mut CameraOrbitState>) {
    let dt = time.delta_secs();
    let lerp_factor = (ORBIT_LERP_SPEED * dt).min(1.0);

    for mut orbit in &mut query {
        let diff = shortest_angle_diff(orbit.target_angle - orbit.current_angle);
        if diff.abs() > 0.001 {
            orbit.current_angle += diff * lerp_factor;
        } else {
            orbit.current_angle = orbit.target_angle;
        }
    }
}

/// Wraps an angle difference onto the shortest path in `[-π, π]`.
fn shortest_angle_diff(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

/// Steers the orbit target angle perpendicular to the player→target line of action,
/// releasing the lock when the target despawns, dies, or moves out of range.
pub(crate) fn steer_lock_on_camera(
    mut commands: Commands,
    player_query: Query<&Position, With<Controlled>>,
    targets: Query<(&Position, Has<RespawnTimer>), Without<Controlled>>,
    mut camera_query: Query<(Entity, &mut CameraOrbitState, &LockOnTarget), With<Camera3d>>,
) {
    let Ok((camera_entity, mut orbit, lock)) = camera_query.single_mut() else {
        trace!("steer_lock_on_camera: no locked camera this frame");
        return;
    };
    let Ok(player_pos) = player_query.single() else {
        trace!("steer_lock_on_camera: controlled player is not available yet");
        return;
    };
    let Ok((target_pos, target_dead)) = targets.get(lock.target) else {
        info!("steer_lock_on_camera: lock-on target despawned, releasing");
        release_lock_on(&mut commands, camera_entity, &mut orbit);
        return;
    };
    let to_target = target_pos.0 - player_pos.0;
    if target_dead || to_target.length() > LOCK_ON_BREAK_DISTANCE {
        info!(
            dead = target_dead,
            "steer_lock_on_camera: lock-on broken, releasing"
        );
        release_lock_on(&mut commands, camera_entity, &mut orbit);
        return;
    }
    orbit.target_angle = lock_on_target_angle(to_target, lock.side);
}

/// Returns the camera yaw perpendicular to the line of action so both characters
/// stay framed side-on (180° rule). `side` selects which side of the line.
fn lock_on_target_angle(to_target: Vec3, side: f32) -> f32 {
    to_target.x.atan2(to_target.z) + side * FRAC_PI_2
}

pub(crate) fn follow_player(
    time: Res<Time>,
    player_query: Query<&Position, With<Controlled>>,
    target_positions: Query<&Position, Without<Controlled>>,
    mut camera_query: Query<
        (
            &mut Transform,
            &CameraOrbitState,
            Option<&LockOnTarget>,
            &mut Projection,
        ),
        With<Camera3d>,
    >,
) {
    let Ok(player_pos) = player_query.single() else {
        trace!("follow_player: controlled player is not available yet");
        return;
    };
    let Ok((mut camera_transform, orbit, lock, mut projection)) = camera_query.single_mut() else {
        trace!("follow_player: camera is not available yet");
        return;
    };

    let frame = camera_frame(player_pos.0, lock, &target_positions);
    let desired_offset = Quat::from_rotation_y(orbit.current_angle) * base_camera_direction();
    let desired_direction = Dir3::new(desired_offset)
        .expect("base_camera_direction must be non-zero so the camera can follow the player");
    // Orthographic distance does not affect the image, so the camera sits at a fixed
    // far arm length that keeps the whole scene at positive depth (`near = 0`).
    camera_transform.translation = frame.anchor + desired_direction.as_vec3() * CAMERA_DISTANCE;
    camera_transform.look_at(frame.anchor, Dir3::Y);

    let zoom_lerp = (CAMERA_ZOOM_LERP_SPEED * time.delta_secs()).min(1.0);
    let Projection::Orthographic(ortho) = &mut *projection else {
        panic!("isometric camera requires an orthographic projection");
    };
    ortho.scale += (frame.desired_scale - ortho.scale) * zoom_lerp;
}

/// Where the camera looks and how far the orthographic projection is zoomed out.
struct CameraFrame {
    anchor: Vec3,
    desired_scale: f32,
}

/// Frames the player alone, or the midpoint of player and lock-on target zoomed out
/// enough to keep both visible.
fn camera_frame(
    player_pos: Vec3,
    lock: Option<&LockOnTarget>,
    target_positions: &Query<&Position, Without<Controlled>>,
) -> CameraFrame {
    let locked_target =
        lock.and_then(|lock| target_positions.get(lock.target).ok().map(|pos| pos.0));
    match locked_target {
        Some(target_pos) => CameraFrame {
            anchor: player_pos.midpoint(target_pos),
            desired_scale: lock_on_framing_scale(player_pos.distance(target_pos)),
        },
        // steer_lock_on_camera releases stale locks; a dangling lock this frame
        // simply frames the player until the release command applies.
        None => CameraFrame {
            anchor: player_pos,
            desired_scale: 1.0,
        },
    }
}

/// Orthographic zoom multiplier that keeps both lock-on participants in frame,
/// never tighter than the base zoom. Fits the separation to the vertical extent,
/// which guarantees both stay visible on landscape windows (slightly conservative).
fn lock_on_framing_scale(separation: f32) -> f32 {
    ((separation + 2.0 * LOCK_ON_FRAME_MARGIN) / CAMERA_VIEW_HEIGHT).max(1.0)
}

/// Centers the spherical background on the camera without copying camera rotation.
pub(crate) fn center_background_sphere(
    camera_query: Query<&Transform, (With<Camera3d>, Without<BackgroundSphere>)>,
    mut background_query: Query<&mut Transform, With<BackgroundSphere>>,
) {
    let Ok(camera_transform) = camera_query.single() else {
        trace!("center_background_sphere: camera is not available yet");
        return;
    };
    let Ok(mut background_transform) = background_query.single_mut() else {
        trace!("center_background_sphere: background sphere is not available yet");
        return;
    };

    background_transform.translation = camera_transform.translation;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The camera offset direction at the lock-on yaw must be perpendicular
    /// to the line of action (180° rule), for either side.
    #[test]
    fn lock_on_yaw_is_perpendicular_to_line_of_action() {
        let to_target = Vec3::new(3.0, 0.5, -7.0);
        for side in [1.0, -1.0] {
            let angle = lock_on_target_angle(to_target, side);
            let offset = Quat::from_rotation_y(angle) * base_camera_direction();
            let dot = offset.xz().normalize().dot(to_target.xz().normalize());
            assert!(dot.abs() < 1e-5, "side {side}: dot {dot}");
        }
    }

    /// Flipping the side rotates the camera yaw by exactly 180°.
    #[test]
    fn lock_on_side_flip_is_half_turn() {
        let to_target = Vec3::new(-2.0, 0.0, 5.0);
        let diff = lock_on_target_angle(to_target, 1.0) - lock_on_target_angle(to_target, -1.0);
        assert!((diff.abs() - PI).abs() < 1e-6);
    }

    #[test]
    fn shortest_angle_diff_wraps() {
        assert!((shortest_angle_diff(TAU + 0.1) - 0.1).abs() < 1e-6);
        assert!((shortest_angle_diff(-TAU - 0.1) + 0.1).abs() < 1e-6);
        assert!((shortest_angle_diff(PI + 0.1) + PI - 0.1).abs() < 1e-6);
    }

    /// Framing never zooms tighter than the base zoom and grows with separation.
    #[test]
    fn framing_scale_grows_with_separation() {
        assert_eq!(lock_on_framing_scale(0.0), 1.0);
        let far = lock_on_framing_scale(200.0);
        assert!((far - (200.0 + 2.0 * LOCK_ON_FRAME_MARGIN) / CAMERA_VIEW_HEIGHT).abs() < 1e-4);
        assert!(far > 1.0);
    }

    /// Release snaps yaw onto the 45°-offset corner grid, rounding halves up.
    #[test]
    fn orbit_rest_angle_snaps_to_corner_grid() {
        assert!((nearest_orbit_rest_angle(0.4) - FRAC_PI_4).abs() < 1e-6);
        assert!((nearest_orbit_rest_angle(FRAC_PI_4 + 0.3) - FRAC_PI_4).abs() < 1e-6);
        assert!((nearest_orbit_rest_angle(FRAC_PI_2) - (FRAC_PI_2 + FRAC_PI_4)).abs() < 1e-6);
    }
}
