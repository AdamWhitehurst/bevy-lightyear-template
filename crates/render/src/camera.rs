use std::f32::consts::{FRAC_PI_2, PI, TAU};

use avian3d::prelude::{Position, SpatialQuery, SpatialQueryFilter};
use bevy::image::{ImageAddressMode, ImageLoaderSettings};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use lightyear::prelude::*;
use protocol::{MapInstanceId, RespawnTimer, VoxelChunk, WorldObjectId};

const BASE_OFFSET: Vec3 = Vec3::new(0.0, 18.0, -36.0);
const BASE_LIGHT_OFFSET: Vec3 = Vec3::new(8.0, 16.0, 8.0);
const ORBIT_LERP_SPEED: f32 = 20.0;
const CAMERA_OCCLUSION_PADDING: f32 = 0.75;
const CAMERA_MIN_DISTANCE: f32 = 6.0;
const CAMERA_COLLISION_LERP_SPEED: f32 = 18.0;
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
    /// Target angle in radians (one of 0, π/2, π, 3π/2)
    pub target_angle: f32,
    /// Current angle in radians (lerps toward target)
    pub current_angle: f32,
    /// Current camera arm length after occlusion avoidance.
    pub current_distance: f32,
}

impl Default for CameraOrbitState {
    fn default() -> Self {
        Self {
            target_angle: 0.0,
            current_angle: 0.0,
            current_distance: BASE_OFFSET.length(),
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

/// Removes the lock and snaps the orbit target back onto the discrete 90° grid.
pub fn release_lock_on(
    commands: &mut Commands,
    camera_entity: Entity,
    orbit: &mut CameraOrbitState,
) {
    orbit.target_angle = (orbit.target_angle / FRAC_PI_2).round() * FRAC_PI_2;
    commands.entity(camera_entity).remove::<LockOnTarget>();
}

/// Marker for the main scene light that follows camera rotation.
#[derive(Component)]
pub struct MainLight;

/// Spherical background that follows camera translation without inheriting rotation.
#[derive(Component)]
pub struct BackgroundSphere;

pub(crate) fn setup_camera(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let camera_transform = Transform::from_xyz(0.0, 18.0, -36.0).looking_at(Vec3::ZERO, Dir3::Y);
    commands.spawn((
        Camera3d::default(),
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
    spatial_query: SpatialQuery,
    player_query: Query<(Entity, &Position, Option<&MapInstanceId>), With<Controlled>>,
    target_positions: Query<&Position, Without<Controlled>>,
    map_ids: Query<&MapInstanceId>,
    occluders: Query<(), Or<(With<WorldObjectId>, With<VoxelChunk>, With<Mesh3d>)>>,
    mut camera_query: Query<
        (
            &mut Transform,
            &mut CameraOrbitState,
            Option<&LockOnTarget>,
            &Projection,
        ),
        With<Camera3d>,
    >,
) {
    let Ok((player_entity, player_pos, player_map)) = player_query.single() else {
        trace!("follow_player: controlled player is not available yet");
        return;
    };
    let Ok((mut camera_transform, mut orbit, lock, projection)) = camera_query.single_mut() else {
        trace!("follow_player: camera is not available yet");
        return;
    };

    let frame = camera_frame(
        player_entity,
        player_pos.0,
        lock,
        &target_positions,
        projection,
    );
    let desired_offset = Quat::from_rotation_y(orbit.current_angle) * BASE_OFFSET;
    let desired_direction = Dir3::new(desired_offset.normalize())
        .expect("BASE_OFFSET must be non-zero so the camera can follow the player");
    let target_distance = occlusion_adjusted_camera_distance(
        frame.anchor,
        &frame.excluded,
        player_map,
        desired_direction,
        frame.desired_distance,
        &spatial_query,
        &map_ids,
        &occluders,
    );

    let lerp_factor = (CAMERA_COLLISION_LERP_SPEED * time.delta_secs()).min(1.0);
    orbit.current_distance += (target_distance - orbit.current_distance) * lerp_factor;
    camera_transform.translation =
        frame.anchor + desired_direction.as_vec3() * orbit.current_distance;
    camera_transform.look_at(frame.anchor, Dir3::Y);
}

/// Where the camera looks, how far back it sits, and which entities its occlusion ray ignores.
struct CameraFrame {
    anchor: Vec3,
    desired_distance: f32,
    excluded: Vec<Entity>,
}

/// Frames the player alone, or the midpoint of player and lock-on target with enough
/// arm length to keep both visible.
fn camera_frame(
    player_entity: Entity,
    player_pos: Vec3,
    lock: Option<&LockOnTarget>,
    target_positions: &Query<&Position, Without<Controlled>>,
    projection: &Projection,
) -> CameraFrame {
    let locked_target = lock.and_then(|lock| {
        target_positions
            .get(lock.target)
            .ok()
            .map(|pos| (lock.target, pos.0))
    });
    match locked_target {
        Some((target_entity, target_pos)) => CameraFrame {
            anchor: player_pos.midpoint(target_pos),
            desired_distance: lock_on_framing_distance(
                player_pos.distance(target_pos),
                tan_half_horizontal_fov(projection),
            ),
            excluded: vec![player_entity, target_entity],
        },
        // steer_lock_on_camera releases stale locks; a dangling lock this frame
        // simply frames the player until the release command applies.
        None => CameraFrame {
            anchor: player_pos,
            desired_distance: BASE_OFFSET.length(),
            excluded: vec![player_entity],
        },
    }
}

/// Arm length that keeps both lock-on participants inside the horizontal FOV,
/// never closer than the base orbit distance.
fn lock_on_framing_distance(separation: f32, tan_half_hfov: f32) -> f32 {
    let required = (separation * 0.5 + LOCK_ON_FRAME_MARGIN) / tan_half_hfov;
    required.max(BASE_OFFSET.length())
}

/// Half-FOV tangent along the screen's horizontal axis.
fn tan_half_horizontal_fov(projection: &Projection) -> f32 {
    let Projection::Perspective(perspective) = projection else {
        panic!("lock-on camera framing requires a perspective projection");
    };
    (perspective.fov * 0.5).tan() * perspective.aspect_ratio
}

/// Returns the camera arm length clamped in front of the nearest opaque-ish collider.
fn occlusion_adjusted_camera_distance(
    anchor: Vec3,
    excluded: &[Entity],
    player_map: Option<&MapInstanceId>,
    desired_direction: Dir3,
    desired_distance: f32,
    spatial_query: &SpatialQuery,
    map_ids: &Query<&MapInstanceId>,
    occluders: &Query<(), Or<(With<WorldObjectId>, With<VoxelChunk>, With<Mesh3d>)>>,
) -> f32 {
    let filter = SpatialQueryFilter::from_excluded_entities(excluded.iter().copied());
    spatial_query
        .cast_ray_predicate(
            anchor,
            desired_direction,
            desired_distance,
            true,
            &filter,
            &|entity| is_camera_occluder(entity, player_map, map_ids, occluders),
        )
        .map(|hit| (hit.distance - CAMERA_OCCLUSION_PADDING).max(CAMERA_MIN_DISTANCE))
        .unwrap_or(desired_distance)
}

/// Treats world objects, voxel chunks, and mesh collider entities as camera occluders.
fn is_camera_occluder(
    entity: Entity,
    player_map: Option<&MapInstanceId>,
    map_ids: &Query<&MapInstanceId>,
    occluders: &Query<(), Or<(With<WorldObjectId>, With<VoxelChunk>, With<Mesh3d>)>>,
) -> bool {
    if !occluders.contains(entity) {
        return false;
    }
    match (player_map, map_ids.get(entity).ok()) {
        (Some(player_map), Some(entity_map)) => player_map == entity_map,
        _ => true,
    }
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
            let offset = Quat::from_rotation_y(angle) * BASE_OFFSET;
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

    /// Framing never pulls closer than the base orbit arm and grows with separation.
    #[test]
    fn framing_distance_grows_with_separation() {
        let tan_half = 1.0;
        assert_eq!(
            lock_on_framing_distance(0.0, tan_half),
            BASE_OFFSET.length()
        );
        let far = lock_on_framing_distance(200.0, tan_half);
        assert!((far - (100.0 + LOCK_ON_FRAME_MARGIN)).abs() < 1e-4);
    }
}
