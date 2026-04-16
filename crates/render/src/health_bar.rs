use bevy::mesh::Indices;
use bevy::prelude::*;
use bevy::render::render_resource::PrimitiveTopology;
use protocol::billboard::billboard_material::{BillboardExt, BillboardMaterial};
use protocol::*;

#[derive(Component)]
pub(crate) struct HealthBarRoot;

#[derive(Component)]
pub(crate) struct HealthBarForeground;

const HEALTH_BAR_WIDTH: f32 = 3.0;
const HEALTH_BAR_HEIGHT: f32 = 0.3;
const HEALTH_BAR_Y_OFFSET: f32 = 5.0;
const HEALTH_BAR_FG_NORMAL: Color = Color::srgb(0.1, 0.9, 0.1);
const HEALTH_BAR_FG_INVULN: Color = Color::srgb(0.2, 0.5, 1.0);

/// Creates a Z-facing quad centered at origin.
fn health_bar_quad() -> Mesh {
    let hw = HEALTH_BAR_WIDTH / 2.0;
    let hh = HEALTH_BAR_HEIGHT / 2.0;
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, default());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-hw, -hh, 0.0],
            [hw, -hh, 0.0],
            [hw, hh, 0.0],
            [-hw, hh, 0.0],
        ],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4]);
    mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    mesh
}

pub(crate) fn spawn_health_bar(
    commands: &mut Commands,
    entity: Entity,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<BillboardMaterial>,
) {
    let bg_mesh = meshes.add(health_bar_quad());
    let fg_mesh = meshes.add(health_bar_quad());
    // bg uses depth_bias to render behind fg instead of a world-space Z offset,
    // because the character entity has a Y-rotation from movement that would
    // transform a Z offset into a camera-dependent direction.
    let bg_material = materials.add(BillboardMaterial {
        base: StandardMaterial {
            base_color: Color::srgb(0.8, 0.1, 0.1),
            unlit: true,
            double_sided: true,
            cull_mode: None,
            depth_bias: -1.0,
            ..default()
        },
        extension: BillboardExt {},
    });
    let fg_material = materials.add(BillboardMaterial {
        base: StandardMaterial {
            base_color: HEALTH_BAR_FG_NORMAL,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        },
        extension: BillboardExt {},
    });

    commands
        .entity(entity)
        .insert(Visibility::default())
        .with_children(|parent| {
            parent
                .spawn((
                    HealthBarRoot,
                    Visibility::default(),
                    Transform::from_translation(Vec3::Y * HEALTH_BAR_Y_OFFSET),
                ))
                .with_children(|bar| {
                    bar.spawn((
                        Mesh3d(bg_mesh),
                        MeshMaterial3d(bg_material),
                        Transform::default(),
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

pub(crate) fn on_invulnerable_added(
    trigger: On<Add, Invulnerable>,
    children_query: Query<&Children>,
    fg_query: Query<&MeshMaterial3d<BillboardMaterial>, With<HealthBarForeground>>,
    mut materials: ResMut<Assets<BillboardMaterial>>,
) {
    set_fg_color(
        trigger.entity,
        HEALTH_BAR_FG_INVULN,
        &children_query,
        &fg_query,
        &mut materials,
    );
}

pub(crate) fn on_invulnerable_removed(
    trigger: On<Remove, Invulnerable>,
    children_query: Query<&Children>,
    fg_query: Query<&MeshMaterial3d<BillboardMaterial>, With<HealthBarForeground>>,
    mut materials: ResMut<Assets<BillboardMaterial>>,
) {
    set_fg_color(
        trigger.entity,
        HEALTH_BAR_FG_NORMAL,
        &children_query,
        &fg_query,
        &mut materials,
    );
}

/// Walk character → HealthBarRoot children → HealthBarForeground grandchildren and set material color.
fn set_fg_color(
    character: Entity,
    color: Color,
    children_query: &Query<&Children>,
    fg_query: &Query<&MeshMaterial3d<BillboardMaterial>, With<HealthBarForeground>>,
    materials: &mut Assets<BillboardMaterial>,
) {
    let Ok(children) = children_query.get(character) else {
        return;
    };
    for &bar_root in children {
        let Ok(grandchildren) = children_query.get(bar_root) else {
            continue;
        };
        for &grandchild in grandchildren {
            if let Ok(handle) = fg_query.get(grandchild) {
                if let Some(mat) = materials.get_mut(&handle.0) {
                    mat.base.base_color = color;
                }
            }
        }
    }
}

/// Updates fg mesh vertex positions to reflect current health.
///
/// Shrinks the fg quad from the left edge while keeping the right edge fixed,
/// so the green bar recedes leftward as health decreases. Vertex positions are
/// modified directly rather than using Transform.translation because the billboard
/// shader operates in view space — a local-space translation offset would get
/// rotated by the character's Y-rotation before the shader sees it.
pub(crate) fn update_health_bars(
    health_query: Query<&Health>,
    bar_root_query: Query<(&ChildOf, &Children), With<HealthBarRoot>>,
    fg_query: Query<&Mesh3d, With<HealthBarForeground>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let hw = HEALTH_BAR_WIDTH / 2.0;
    let hh = HEALTH_BAR_HEIGHT / 2.0;

    for (child_of, children) in &bar_root_query {
        let Ok(health) = health_query.get(child_of.parent()) else {
            continue;
        };
        let ratio = (health.current / health.max).clamp(0.0, 1.0);
        let left_x = hw - HEALTH_BAR_WIDTH * ratio;

        for child in children {
            let Ok(mesh_handle) = fg_query.get(*child) else {
                continue;
            };
            let Some(mesh) = meshes.get_mut(&mesh_handle.0) else {
                continue;
            };
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_POSITION,
                vec![
                    [left_x, -hh, 0.0],
                    [hw, -hh, 0.0],
                    [hw, hh, 0.0],
                    [left_x, hh, 0.0],
                ],
            );
        }
    }
}
