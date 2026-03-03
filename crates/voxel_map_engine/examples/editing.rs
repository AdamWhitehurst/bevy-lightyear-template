use std::sync::Arc;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use voxel_map_engine::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(VoxelPlugin)
        .add_systems(Startup, setup)
        .add_systems(Update, (move_camera, handle_voxel_input))
        .run();
}

/// Resource tracking the overworld map entity for the example.
#[derive(Resource)]
struct MapEntity(Entity);

fn setup(mut commands: Commands) {
    let generator: SdfGenerator = Arc::new(flat_terrain_sdf);

    let mut instance = VoxelMapInstance::new(5);
    instance.debug_colors = true;

    let map_entity = commands
        .spawn((
            instance,
            VoxelMapConfig::new(0, 5, None, 5, generator),
            PendingChunks::default(),
            Transform::default(),
        ))
        .id();

    commands.insert_resource(MapEntity(map_entity));

    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(Vec3::new(8.0, 15.0, 30.0)).looking_at(Vec3::ZERO, Vec3::Y),
        ChunkTarget {
            map_entity,
            distance: 5,
        },
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.5, 0.5, 0.0)),
    ));
}

fn handle_voxel_input(
    mouse: Res<ButtonInput<MouseButton>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    map_entity: Res<MapEntity>,
    mut voxel_world: VoxelWorld,
) {
    let left = mouse.just_pressed(MouseButton::Left);
    let right = mouse.just_pressed(MouseButton::Right);
    if !left && !right {
        return;
    }

    let Ok((camera, cam_transform)) = camera_query.single() else {
        return;
    };
    let Ok(window) = window_query.single() else {
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(cam_transform, cursor_pos) else {
        return;
    };

    let max_distance = 100.0;
    let Some(hit) = voxel_world.raycast(map_entity.0, ray, max_distance, |v| {
        matches!(v, WorldVoxel::Solid(_))
    }) else {
        return;
    };

    if left {
        // Remove the hit voxel
        info!("Removing voxel at {:?}", hit.position);
        voxel_world.set_voxel(map_entity.0, hit.position, WorldVoxel::Air);
    } else if right {
        // Place a voxel on the adjacent face
        if let Some(normal) = hit.normal {
            let place_pos = hit.position + normal.as_ivec3();
            info!("Placing voxel at {:?}", place_pos);
            voxel_world.set_voxel(map_entity.0, place_pos, WorldVoxel::Solid(0));
        }
    }
}

fn move_camera(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut query: Query<&mut Transform, With<Camera3d>>,
) {
    let Ok(mut transform) = query.single_mut() else {
        return;
    };

    let speed = 40.0 * time.delta_secs();
    let forward = Vec3::new(transform.forward().x, 0.0, transform.forward().z).normalize_or_zero();
    let right = Vec3::new(transform.right().x, 0.0, transform.right().z).normalize_or_zero();

    if keys.pressed(KeyCode::KeyW) {
        transform.translation += forward * speed;
    }
    if keys.pressed(KeyCode::KeyS) {
        transform.translation -= forward * speed;
    }
    if keys.pressed(KeyCode::KeyA) {
        transform.translation -= right * speed;
    }
    if keys.pressed(KeyCode::KeyD) {
        transform.translation += right * speed;
    }
    if keys.pressed(KeyCode::Space) {
        transform.translation.y += speed;
    }
    if keys.pressed(KeyCode::ShiftLeft) {
        transform.translation.y -= speed;
    }
}
