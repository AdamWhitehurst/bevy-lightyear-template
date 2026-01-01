use avian3d::prelude::Position;
use bevy::prelude::*;
use bevy_voxel_world::prelude::*;
use lightyear::frame_interpolation::{FrameInterpolate, FrameInterpolationPlugin};
use lightyear::prelude::*;
use protocol::*;

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        if !app.world().is_resource_added::<Assets<Mesh>>() {
            app.init_resource::<Assets<Mesh>>();
        }
        if !app.world().is_resource_added::<Assets<StandardMaterial>>() {
            app.init_resource::<Assets<StandardMaterial>>();
        }
        if !app.world().is_resource_added::<Time<Fixed>>() {
            app.init_resource::<Time<Fixed>>();
        }
        if !app.world().is_resource_added::<InterpolationRegistry>() {
            app.init_resource::<InterpolationRegistry>();
        }

        app.add_systems(Startup, (setup_camera, setup_lighting));
        app.add_systems(Update, (add_character_cosmetics, add_floor_cosmetics));

        // FrameInterpolationPlugin for visual smoothing between physics ticks
        app.add_plugins(FrameInterpolationPlugin::<Position>::default());
        app.add_plugins(FrameInterpolationPlugin::<avian3d::prelude::Rotation>::default());

        // Add visual interpolation components to predicted entities
        app.add_observer(add_visual_interpolation_components);
    }
}

fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 4.5, -9.0).looking_at(Vec3::ZERO, Dir3::Y),
        VoxelWorldCamera::<MapWorld>::default(),
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
        FrameInterpolate::<avian3d::prelude::Rotation> {
            trigger_change_detection: true,
            ..default()
        },
    ));
}

fn add_character_cosmetics(
    mut commands: Commands,
    character_query: Query<
        (Entity, Option<&ColorComponent>),
        (
            Or<(Added<Predicted>, Added<Replicated>, Added<Interpolated>)>,
            With<CharacterMarker>,
        ),
    >,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, color) in &character_query {
        let color = color.map(|c| c.0).unwrap_or(Color::srgb(0.5, 0.5, 0.5));
        info!(?entity, "Adding cosmetics to character");
        commands.entity(entity).insert((
            Mesh3d(meshes.add(Capsule3d::new(
                CHARACTER_CAPSULE_RADIUS,
                CHARACTER_CAPSULE_HEIGHT,
            ))),
            MeshMaterial3d(materials.add(color)),
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
            MeshMaterial3d(materials.add(Color::srgb(0.3, 0.5, 0.3))),
        ));
    }
}
