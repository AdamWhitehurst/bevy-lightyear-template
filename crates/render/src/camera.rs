use avian3d::prelude::Position;
use bevy::image::{ImageAddressMode, ImageLoaderSettings};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use lightyear::prelude::*;

const BASE_OFFSET: Vec3 = Vec3::new(0.0, 18.0, -36.0);
const BASE_LIGHT_OFFSET: Vec3 = Vec3::new(8.0, 16.0, 8.0);
const ORBIT_LERP_SPEED: f32 = 20.0;
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
