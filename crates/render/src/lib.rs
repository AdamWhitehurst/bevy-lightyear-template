mod camera;
mod health_bar;

pub use camera::CameraOrbitState;

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

        app.add_systems(Startup, (camera::setup_camera, camera::setup_lighting));
        app.add_systems(
            Update,
            (
                camera::handle_camera_rotation_input,
                camera::update_camera_orbit,
                camera::follow_player,
                camera::update_light_position,
                health_bar::billboard_face_camera,
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
fn add_health_bars(
    trigger: On<Add, Health>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    health_bar::spawn_health_bar(&mut commands, trigger.entity, &mut *meshes, &mut *materials);
}
