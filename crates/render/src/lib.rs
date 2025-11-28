use bevy::prelude::*;

/// Plugin that sets up rendering components (camera, etc.)
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_camera);
    }
}

fn setup_camera(mut commands: Commands) {
    // Spawn 3D camera offset from origin, looking at origin
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(-5.0, 3.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
