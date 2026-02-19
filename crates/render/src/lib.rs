use avian3d::prelude::Position;
use bevy::prelude::*;
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
        app.add_systems(
            Update,
            (
                add_character_cosmetics,
                follow_player,
                billboard_face_camera,
                update_health_bars,
            ),
        );

        app.add_observer(on_invulnerable_added);
        app.add_observer(on_invulnerable_removed);

        // FrameInterpolationPlugin for visual smoothing between physics ticks
        app.add_plugins(FrameInterpolationPlugin::<Position>::default());
        app.add_plugins(FrameInterpolationPlugin::<avian3d::prelude::Rotation>::default());

        // Add visual interpolation components to predicted entities
        app.add_observer(add_visual_interpolation_components);
    }
}

#[derive(Component)]
struct HealthBarRoot;

#[derive(Component)]
struct HealthBarForeground;

#[derive(Component)]
struct Billboard;

const HEALTH_BAR_WIDTH: f32 = 1.5;
const HEALTH_BAR_HEIGHT: f32 = 0.15;
const HEALTH_BAR_Y_OFFSET: f32 = 2.5;
const HEALTH_BAR_FG_NORMAL: Color = Color::srgb(0.1, 0.9, 0.1);
const HEALTH_BAR_FG_INVULN: Color = Color::srgb(0.2, 0.5, 1.0);

fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 9.0, -18.0).looking_at(Vec3::ZERO, Dir3::Y),
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
    query: Query<Entity, With<Predicted>>,
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

fn spawn_health_bar(
    commands: &mut Commands,
    entity: Entity,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let bg_mesh = meshes.add(Plane3d::new(
        Vec3::Z,
        Vec2::new(HEALTH_BAR_WIDTH / 2.0, HEALTH_BAR_HEIGHT / 2.0),
    ));
    let fg_mesh = meshes.add(Plane3d::new(
        Vec3::Z,
        Vec2::new(HEALTH_BAR_WIDTH / 2.0, HEALTH_BAR_HEIGHT / 2.0),
    ));
    let bg_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.8, 0.1, 0.1),
        unlit: true,
        ..default()
    });
    let fg_material = materials.add(StandardMaterial {
        base_color: HEALTH_BAR_FG_NORMAL,
        unlit: true,
        ..default()
    });

    commands.entity(entity).with_children(|parent| {
        parent
            .spawn((
                HealthBarRoot,
                Billboard,
                Transform::from_translation(Vec3::Y * HEALTH_BAR_Y_OFFSET),
            ))
            .with_children(|bar| {
                bar.spawn((
                    Mesh3d(bg_mesh),
                    MeshMaterial3d(bg_material),
                    Transform::from_translation(Vec3::Z * -0.01),
                ));
                bar.spawn((
                    HealthBarForeground,
                    Mesh3d(fg_mesh),
                    MeshMaterial3d(fg_material),
                    Transform::default(),
                ));
            });
    });
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
        spawn_health_bar(&mut commands, entity, &mut meshes, &mut materials);
    }
}

fn follow_player(
    player_query: Query<&Position, With<Controlled>>,
    mut camera_query: Query<&mut Transform, With<Camera3d>>,
) {
    let Ok(player_pos) = player_query.single() else {
        return;
    };
    let Ok(mut camera_transform) = camera_query.single_mut() else {
        return;
    };

    let offset = Vec3::new(0.0, 9.0, -18.0);
    camera_transform.translation = **player_pos + offset;
    camera_transform.look_at(**player_pos, Dir3::Y);
}

fn billboard_face_camera(
    camera_query: Query<&GlobalTransform, With<Camera3d>>,
    mut billboard_query: Query<(&GlobalTransform, &mut Transform, &ChildOf), With<Billboard>>,
    parent_query: Query<&GlobalTransform, Without<Billboard>>,
) {
    let Ok(camera_gt) = camera_query.single() else {
        return;
    };
    let camera_pos = camera_gt.translation();
    for (global_transform, mut transform, child_of) in &mut billboard_query {
        let billboard_pos = global_transform.translation();
        let direction = (camera_pos - billboard_pos).with_y(0.0);
        if direction.length_squared() < 0.001 {
            continue;
        }
        let world_rotation = Quat::from_rotation_arc(Vec3::Z, direction.normalize());
        let parent_rotation = parent_query
            .get(child_of.parent())
            .map(|gt| gt.to_scale_rotation_translation().1)
            .unwrap_or(Quat::IDENTITY);
        transform.rotation = parent_rotation.inverse() * world_rotation;
    }
}

fn on_invulnerable_added(
    trigger: On<Add, Invulnerable>,
    children_query: Query<&Children>,
    fg_query: Query<&MeshMaterial3d<StandardMaterial>, With<HealthBarForeground>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    set_fg_color(trigger.entity, HEALTH_BAR_FG_INVULN, &children_query, &fg_query, &mut materials);
}

fn on_invulnerable_removed(
    trigger: On<Remove, Invulnerable>,
    children_query: Query<&Children>,
    fg_query: Query<&MeshMaterial3d<StandardMaterial>, With<HealthBarForeground>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    set_fg_color(trigger.entity, HEALTH_BAR_FG_NORMAL, &children_query, &fg_query, &mut materials);
}

/// Walk character → HealthBarRoot children → HealthBarForeground grandchildren and set material color.
fn set_fg_color(
    character: Entity,
    color: Color,
    children_query: &Query<&Children>,
    fg_query: &Query<&MeshMaterial3d<StandardMaterial>, With<HealthBarForeground>>,
    materials: &mut Assets<StandardMaterial>,
) {
    let Ok(children) = children_query.get(character) else { return };
    for &bar_root in children {
        let Ok(grandchildren) = children_query.get(bar_root) else { continue };
        for &grandchild in grandchildren {
            if let Ok(handle) = fg_query.get(grandchild) {
                if let Some(mat) = materials.get_mut(&handle.0) {
                    mat.base_color = color;
                }
            }
        }
    }
}

fn update_health_bars(
    health_query: Query<&Health, With<CharacterMarker>>,
    bar_root_query: Query<(&ChildOf, &Children), With<HealthBarRoot>>,
    mut fg_query: Query<&mut Transform, With<HealthBarForeground>>,
) {
    for (child_of, children) in &bar_root_query {
        let Ok(health) = health_query.get(child_of.parent()) else {
            continue;
        };
        let ratio = (health.current / health.max).clamp(0.0, 1.0);
        for child in children {
            if let Ok(mut transform) = fg_query.get_mut(*child) {
                transform.scale.x = ratio;
                let offset = (1.0 - ratio) * HEALTH_BAR_WIDTH * -0.5;
                transform.translation.x = offset;
            }
        }
    }
}
