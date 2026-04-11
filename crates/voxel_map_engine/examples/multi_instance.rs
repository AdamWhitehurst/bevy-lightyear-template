use std::sync::Arc;

use bevy::prelude::*;
use ndshape::{RuntimeShape, Shape};
use voxel_map_engine::prelude::*;

#[derive(Resource)]
struct MapInstances {
    overworld: Entity,
    homebase: Entity,
    arena: Entity,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(VoxelPlugin)
        .insert_resource(ChunkGenerationEnabled)
        .add_systems(Startup, setup)
        .add_systems(Update, (move_camera, teleport_camera))
        .run();
}

fn setup(mut commands: Commands) {
    let overworld = spawn_overworld(&mut commands);
    let homebase = spawn_homebase(&mut commands);
    let arena = spawn_arena(&mut commands);

    commands.insert_resource(MapInstances {
        overworld,
        homebase,
        arena,
    });

    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(Vec3::new(30.0, 20.0, 30.0)).looking_at(Vec3::ZERO, Vec3::Y),
        ChunkTicket::new(overworld, TicketType::Player, 5),
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

fn spawn_overworld(commands: &mut Commands) -> Entity {
    let (mut instance, config, marker) = VoxelMapInstance::overworld(42);
    instance.debug_colors = true;
    commands
        .spawn((
            instance,
            config,
            VoxelGenerator(Arc::new(FlatGenerator {
                chunk_size: 16,
                shape: RuntimeShape::<u32, 3>::new([18, 18, 18]),
            })),
            marker,
            PendingChunks::default(),
            Transform::default(),
        ))
        .id()
}

fn spawn_homebase(commands: &mut Commands) -> Entity {
    let (mut instance, config, marker) = VoxelMapInstance::homebase(0, IVec3::new(8, 4, 8));
    instance.debug_colors = true;
    commands
        .spawn((
            instance,
            config,
            VoxelGenerator(Arc::new(RaisedGenerator)),
            marker,
            PendingChunks::default(),
            Transform::from_translation(Vec3::new(200.0, 0.0, 0.0)),
        ))
        .id()
}

fn spawn_arena(commands: &mut Commands) -> Entity {
    let (mut instance, config, marker) = VoxelMapInstance::arena(1, 99, IVec3::new(10, 4, 10));
    instance.debug_colors = true;
    commands
        .spawn((
            instance,
            config,
            VoxelGenerator(Arc::new(BowlGenerator)),
            marker,
            PendingChunks::default(),
            Transform::from_translation(Vec3::new(-200.0, 0.0, 0.0)),
        ))
        .id()
}

struct RaisedGenerator;

impl VoxelGeneratorImpl for RaisedGenerator {
    fn generate_terrain(&self, chunk_pos: IVec3) -> Vec<WorldVoxel> {
        let shape = RuntimeShape::<u32, 3>::new([18, 18, 18]);
        let mut voxels = vec![WorldVoxel::Air; shape.usize()];
        for i in 0..shape.size() {
            let [_x, y, _z] = shape.delinearize(i);
            let world_y = chunk_pos.y * CHUNK_SIZE as i32 + y as i32 - 1;
            if world_y < 4 {
                voxels[i as usize] = WorldVoxel::Solid(0);
            }
        }
        voxels
    }
}

struct BowlGenerator;

impl VoxelGeneratorImpl for BowlGenerator {
    fn generate_terrain(&self, chunk_pos: IVec3) -> Vec<WorldVoxel> {
        let shape = RuntimeShape::<u32, 3>::new([18, 18, 18]);
        let mut voxels = vec![WorldVoxel::Air; shape.usize()];
        for i in 0..shape.size() {
            let [x, y, z] = shape.delinearize(i);
            let world_x = (chunk_pos.x * CHUNK_SIZE as i32 + x as i32 - 1) as f32;
            let world_y = (chunk_pos.y * CHUNK_SIZE as i32 + y as i32 - 1) as f32;
            let world_z = (chunk_pos.z * CHUNK_SIZE as i32 + z as i32 - 1) as f32;
            let dist = (world_x * world_x + world_z * world_z).sqrt();
            let surface_y = -2.0 + dist * 0.15;
            if world_y < surface_y {
                voxels[i as usize] = WorldVoxel::Solid(0);
            }
        }
        voxels
    }
}

fn teleport_camera(
    keys: Res<ButtonInput<KeyCode>>,
    instances: Res<MapInstances>,
    mut query: Query<(&mut Transform, &mut ChunkTicket), With<Camera3d>>,
) {
    let Ok((mut transform, mut ticket)) = query.single_mut() else {
        return;
    };

    if keys.just_pressed(KeyCode::Digit1) {
        teleport_to(
            &mut transform,
            &mut ticket,
            instances.overworld,
            Vec3::new(30.0, 20.0, 30.0),
        );
    } else if keys.just_pressed(KeyCode::Digit2) {
        teleport_to(
            &mut transform,
            &mut ticket,
            instances.homebase,
            Vec3::new(230.0, 20.0, 30.0),
        );
    } else if keys.just_pressed(KeyCode::Digit3) {
        teleport_to(
            &mut transform,
            &mut ticket,
            instances.arena,
            Vec3::new(-170.0, 20.0, 30.0),
        );
    }
}

fn teleport_to(
    transform: &mut Transform,
    ticket: &mut ChunkTicket,
    map_entity: Entity,
    position: Vec3,
) {
    let look_at = Vec3::new(position.x - 30.0, 0.0, position.z - 30.0);
    *transform = Transform::from_translation(position).looking_at(look_at, Vec3::Y);
    ticket.map_entity = map_entity;
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
