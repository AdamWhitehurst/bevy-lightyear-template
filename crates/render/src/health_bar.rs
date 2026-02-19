use bevy::prelude::*;
use protocol::*;

#[derive(Component)]
pub(crate) struct HealthBarRoot;

#[derive(Component)]
pub(crate) struct HealthBarForeground;

#[derive(Component)]
pub(crate) struct Billboard;

const HEALTH_BAR_WIDTH: f32 = 1.5;
const HEALTH_BAR_HEIGHT: f32 = 0.15;
const HEALTH_BAR_Y_OFFSET: f32 = 2.5;
const HEALTH_BAR_FG_NORMAL: Color = Color::srgb(0.1, 0.9, 0.1);
const HEALTH_BAR_FG_INVULN: Color = Color::srgb(0.2, 0.5, 1.0);

pub(crate) fn spawn_health_bar(
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

pub(crate) fn billboard_face_camera(
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

pub(crate) fn on_invulnerable_added(
    trigger: On<Add, Invulnerable>,
    children_query: Query<&Children>,
    fg_query: Query<&MeshMaterial3d<StandardMaterial>, With<HealthBarForeground>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    set_fg_color(trigger.entity, HEALTH_BAR_FG_INVULN, &children_query, &fg_query, &mut materials);
}

pub(crate) fn on_invulnerable_removed(
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

pub(crate) fn update_health_bars(
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
