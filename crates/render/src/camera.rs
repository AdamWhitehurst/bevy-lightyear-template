use avian3d::prelude::Position;
use bevy::prelude::*;
use lightyear::prelude::*;

pub(crate) fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 9.0, -18.0).looking_at(Vec3::ZERO, Dir3::Y),
    ));
}

pub(crate) fn setup_lighting(mut commands: Commands) {
    commands.spawn((
        PointLight {
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0),
    ));
}

pub(crate) fn follow_player(
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
