use std::sync::Arc;

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use voxel_map_engine::prelude::*;

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::transform::TransformPlugin);
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.add_plugins(VoxelPlugin);
    app
}

fn spawn_map(app: &mut App, spawning_distance: u32) -> Entity {
    let generator: SdfGenerator = Arc::new(flat_terrain_sdf);
    app.world_mut()
        .spawn((
            VoxelMapInstance::new(5),
            VoxelMapConfig::new(0, spawning_distance, None, 5, generator),
            Transform::default(),
        ))
        .id()
}

fn spawn_target(app: &mut App, map_entity: Entity, position: Vec3, distance: u32) -> Entity {
    app.world_mut()
        .spawn((
            ChunkTarget {
                map_entity,
                distance,
            },
            Transform::from_translation(position),
        ))
        .id()
}

fn tick(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

/// Test: set_voxel queues to write buffer, flush moves to modified_voxels,
/// get_voxel returns the written value.
#[test]
fn set_get_voxel_round_trip() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);
    spawn_target(&mut app, map, Vec3::ZERO, 0);

    // Tick once so PendingChunks is inserted
    tick(&mut app, 1);

    // Use VoxelWorld::set_voxel via a one-shot system
    let edit_pos = IVec3::new(3, 5, 7);
    app.world_mut()
        .run_system_once(move |mut vw: VoxelWorld| {
            vw.set_voxel(map, edit_pos, WorldVoxel::Solid(42));
        })
        .unwrap();

    // Verify it's in the write buffer but NOT yet in modified_voxels
    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    assert_eq!(instance.write_buffer.len(), 1);
    assert!(!instance.modified_voxels.contains_key(&edit_pos));

    // Tick to run flush_write_buffer
    tick(&mut app, 1);

    // Verify write buffer was drained and modified_voxels was populated
    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    assert!(instance.write_buffer.is_empty());
    assert_eq!(
        instance.modified_voxels.get(&edit_pos),
        Some(&WorldVoxel::Solid(42))
    );

    // Verify get_voxel returns the written value
    app.world_mut()
        .run_system_once(move |vw: VoxelWorld| {
            let voxel = vw.get_voxel(map, edit_pos);
            assert_eq!(voxel, WorldVoxel::Solid(42));
        })
        .unwrap();
}

/// Test: get_voxel falls back to SDF for unmodified positions.
#[test]
fn get_voxel_reads_sdf_for_unmodified() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);
    tick(&mut app, 1);

    app.world_mut()
        .run_system_once(move |vw: VoxelWorld| {
            // Flat terrain SDF: y < 0 is solid, y >= 0 is air
            let below = vw.get_voxel(map, IVec3::new(0, -1, 0));
            assert_eq!(below, WorldVoxel::Solid(0));

            let above = vw.get_voxel(map, IVec3::new(0, 1, 0));
            assert_eq!(above, WorldVoxel::Air);
        })
        .unwrap();
}

/// Test: flush_write_buffer invalidates the affected chunk so it gets remeshed.
#[test]
fn flush_write_buffer_triggers_remesh() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);
    spawn_target(&mut app, map, Vec3::ZERO, 0);

    // Let chunks generate
    tick(&mut app, 20);

    let loaded_before = app
        .world()
        .get::<VoxelMapInstance>(map)
        .unwrap()
        .loaded_chunks
        .len();
    assert!(loaded_before > 0, "should have loaded chunks");

    // The origin chunk (0,0,0) should be loaded
    assert!(
        app.world()
            .get::<VoxelMapInstance>(map)
            .unwrap()
            .loaded_chunks
            .contains(&IVec3::ZERO)
    );

    // Write a voxel inside the origin chunk
    let edit_pos = IVec3::new(8, 8, 8); // center of chunk (0,0,0)
    app.world_mut()
        .get_mut::<VoxelMapInstance>(map)
        .unwrap()
        .write_buffer
        .push((edit_pos, WorldVoxel::Solid(1)));

    // Tick to run flush_write_buffer - should invalidate chunk (0,0,0)
    tick(&mut app, 1);

    // After flush, the origin chunk should be removed from loaded_chunks
    // (invalidated for remesh)
    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    assert!(
        !instance.loaded_chunks.contains(&IVec3::ZERO),
        "flush should invalidate the chunk containing the edited voxel"
    );

    // After more ticks, the chunk should be regenerated (re-added to loaded_chunks)
    tick(&mut app, 20);

    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    assert!(
        instance.loaded_chunks.contains(&IVec3::ZERO),
        "invalidated chunk should be regenerated"
    );
}

/// Test: modified voxels persist through a chunk despawn/respawn cycle.
#[test]
fn modified_voxels_survive_chunk_cycle() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);
    let target = spawn_target(&mut app, map, Vec3::ZERO, 0);

    // Let chunks generate around origin
    tick(&mut app, 20);
    assert!(
        app.world()
            .get::<VoxelMapInstance>(map)
            .unwrap()
            .loaded_chunks
            .contains(&IVec3::ZERO)
    );

    // Write a voxel edit
    let edit_pos = IVec3::new(8, 8, 8);
    app.world_mut()
        .get_mut::<VoxelMapInstance>(map)
        .unwrap()
        .write_buffer
        .push((edit_pos, WorldVoxel::Solid(99)));

    // Flush
    tick(&mut app, 1);

    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    assert_eq!(
        instance.modified_voxels.get(&edit_pos),
        Some(&WorldVoxel::Solid(99))
    );

    // Move target far away so origin chunk despawns
    app.world_mut()
        .entity_mut(target)
        .insert(Transform::from_translation(Vec3::new(10000.0, 0.0, 0.0)));
    tick(&mut app, 5);

    // Origin chunk should be unloaded
    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    assert!(
        !instance.loaded_chunks.contains(&IVec3::ZERO),
        "chunk should be unloaded after target moved away"
    );

    // But modified_voxels should still have the edit
    assert_eq!(
        instance.modified_voxels.get(&edit_pos),
        Some(&WorldVoxel::Solid(99)),
        "modified_voxels should survive chunk despawn"
    );

    // Move target back to origin so chunk respawns
    app.world_mut()
        .entity_mut(target)
        .insert(Transform::from_translation(Vec3::ZERO));
    tick(&mut app, 20);

    // Chunk should be reloaded
    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    assert!(
        instance.loaded_chunks.contains(&IVec3::ZERO),
        "chunk should be reloaded after target returns"
    );

    // modified_voxels should still have the edit
    assert_eq!(
        instance.modified_voxels.get(&edit_pos),
        Some(&WorldVoxel::Solid(99)),
        "modified_voxels should survive full chunk cycle"
    );
}

/// Test: raycast hits flat terrain from above.
#[test]
fn raycast_hits_flat_terrain() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);
    tick(&mut app, 1);

    app.world_mut()
        .run_system_once(move |vw: VoxelWorld| {
            // Cast ray straight down from y=10 toward y=-10
            let ray = Ray3d::new(Vec3::new(0.5, 10.0, 0.5), Dir3::NEG_Y);
            let result = vw.raycast(map, ray, 50.0, |v| matches!(v, WorldVoxel::Solid(_)));
            let hit = result.expect("should hit flat terrain");

            // Flat terrain is solid at y < 0, so first hit should be at y = -1
            assert_eq!(hit.position.y, -1, "should hit at y=-1 (first solid)");
            assert_eq!(hit.voxel, WorldVoxel::Solid(0));
            assert_eq!(hit.normal, Some(Vec3::Y), "should enter from top face");
        })
        .unwrap();
}

/// Test: raycast misses when pointed at empty space.
#[test]
fn raycast_misses_empty_space() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);
    tick(&mut app, 1);

    app.world_mut()
        .run_system_once(move |vw: VoxelWorld| {
            // Cast ray horizontally through air (y=10, well above terrain)
            let ray = Ray3d::new(Vec3::new(0.5, 10.0, 0.5), Dir3::X);
            let result = vw.raycast(map, ray, 20.0, |v| matches!(v, WorldVoxel::Solid(_)));
            assert!(result.is_none(), "should not hit anything above terrain");
        })
        .unwrap();
}
