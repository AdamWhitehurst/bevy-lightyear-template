mod camera;
mod health_bar;

pub use camera::CameraOrbitState;

use avian3d::prelude::Position;
use bevy::prelude::*;
use lightyear::frame_interpolation::{FrameInterpolate, FrameInterpolationPlugin};
use lightyear::prelude::*;
use protocol::billboard::billboard_material::BillboardMaterial;
use protocol::billboard::shadow_only_material::ShadowOnlyMaterial;
use protocol::billboard::sprite_rig_material::SpriteRigMaterial;
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

        app.add_plugins(bevy::pbr::MaterialPlugin::<BillboardMaterial>::default());
        app.add_plugins(bevy::pbr::MaterialPlugin::<SpriteRigMaterial>::default());
        app.add_plugins(bevy::pbr::MaterialPlugin::<ShadowOnlyMaterial>::default());

        app.add_systems(Startup, (camera::setup_camera, camera::setup_lighting));
        app.add_systems(
            Update,
            (
                camera::handle_camera_rotation_input,
                camera::update_camera_orbit,
                camera::follow_player,
                camera::update_light_position,
                health_bar::update_health_bars,
            )
                .chain(),
        );

        app.add_observer(add_health_bars);
        app.add_observer(health_bar::on_invulnerable_added);
        app.add_observer(health_bar::on_invulnerable_removed);

        app.add_plugins(sprite_rig::SpriteRigPlugin);

        // FrameInterpolationPlugin for visual smoothing between physics ticks
        app.add_plugins(FrameInterpolationPlugin::<Position>::default());
        app.add_plugins(FrameInterpolationPlugin::<avian3d::prelude::Rotation>::default());

        // Add visual interpolation components to predicted entities
        app.add_observer(add_visual_interpolation_components);
    }
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

/// Spawns a health bar for any entity that receives a `Health` component.
///
/// This observer fires synchronously when lightyear inserts `Health` in
/// PreUpdate, before `on_world_object_replicated` has a chance to add
/// `Transform`. We insert it here from `Position`/`Rotation` so the health
/// bar children have a parent with `GlobalTransform`.
fn add_health_bars(
    trigger: On<Add, Health>,
    physics: Query<(Option<&Position>, Option<&avian3d::prelude::Rotation>)>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BillboardMaterial>>,
) {
    let entity = trigger.entity;
    if let Ok((pos, rot)) = physics.get(entity) {
        commands
            .entity(entity)
            .insert(protocol::transform_from_physics(pos, rot));
    }
    health_bar::spawn_health_bar(&mut commands, entity, &mut *meshes, &mut *materials);
}
