use bevy::app::AppExit;
use bevy::prelude::*;
use protocol::{MapWorld, VoxelType};
use server::map::{save_voxel_world_to_disk_at, load_voxel_world_from_disk_at,
                   VoxelModifications, VoxelDirtyState};
use std::fs;
use std::path::Path;

// Helper: Get unique test directory for each test
fn get_test_dir(test_name: &str) -> String {
    format!("world_save_test/{}", test_name)
}

// Helper: Get save path for a specific test
fn get_save_path(test_name: &str) -> String {
    format!("{}/voxel_world.bin", get_test_dir(test_name))
}

// Helper: Get corrupt backup path for a specific test
fn get_corrupt_backup_path(test_name: &str) -> String {
    format!("{}/voxel_world.bin.corrupt", get_test_dir(test_name))
}

// Helper: Clean up test files for a specific test
fn cleanup_test_files(test_name: &str) {
    let _ = fs::remove_dir_all(get_test_dir(test_name));
}

// Helper: Create test app with ServerMapPlugin for shutdown test
fn create_test_app_for_shutdown() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<VoxelModifications>();
    app.init_resource::<VoxelDirtyState>();
    app.insert_resource(MapWorld::default());
    // Add just the shutdown save system
    app.add_systems(Last, server::map::save_voxel_world_on_shutdown);
    app
}

// Helper system to add voxels directly to VoxelModifications (for shutdown test)
fn add_test_modifications(
    mut modifications: ResMut<VoxelModifications>,
    voxels: Res<TestVoxels>,
) {
    modifications.modifications = voxels.0.clone();
}

// Helper resource to pass test voxels
#[derive(Resource)]
struct TestVoxels(Vec<(IVec3, VoxelType)>);

#[test]
fn test_save_load_cycle() {
    let test_name = "save_load_cycle";
    cleanup_test_files(test_name);

    let save_path = get_save_path(test_name);

    // Test data
    let test_voxels = vec![
        (IVec3::new(0, 0, 0), VoxelType::Solid(1)),
        (IVec3::new(5, 10, 15), VoxelType::Solid(2)),
        (IVec3::new(-3, 7, -2), VoxelType::Solid(3)),
    ];

    let map_world = MapWorld::default();

    // Save directly using the save function (creates directory if needed)
    save_voxel_world_to_disk_at(&test_voxels, &map_world, &save_path).unwrap();

    // Verify save file exists
    assert!(Path::new(&save_path).exists(), "Save file should exist");

    // Load using the load function
    let loaded_mods = load_voxel_world_from_disk_at(&map_world, &save_path);

    // Verify all 3 voxels loaded
    assert_eq!(loaded_mods.len(), 3, "Should load 3 voxels");

    // Verify specific positions and materials match
    for (pos, voxel_type) in &test_voxels {
        let found = loaded_mods.iter()
            .any(|(p, v)| p == pos && v == voxel_type);
        assert!(found, "Should find voxel at {:?} with type {:?}", pos, voxel_type);
    }

    cleanup_test_files(test_name);
}

#[test]
fn test_corrupt_file_recovery() {
    let test_name = "corrupt_file_recovery";
    cleanup_test_files(test_name);

    let save_path = get_save_path(test_name);
    let corrupt_backup_path = get_corrupt_backup_path(test_name);

    // Write corrupt data to save file
    fs::create_dir_all(get_test_dir(test_name)).unwrap();
    fs::write(&save_path, b"corrupt data").unwrap();

    // Try to load (should detect corruption and create backup)
    let map_world = MapWorld::default();
    let loaded_mods = load_voxel_world_from_disk_at(&map_world, &save_path);

    // Verify backup file created
    assert!(Path::new(&corrupt_backup_path).exists(), "Corrupt backup file should exist");

    // Verify loaded data is empty (clean start)
    assert_eq!(loaded_mods.len(), 0, "Should start with empty world after corrupt file");

    cleanup_test_files(test_name);
}

#[test]
fn test_generation_metadata_mismatch() {
    let test_name = "generation_metadata_mismatch";
    cleanup_test_files(test_name);

    let save_path = get_save_path(test_name);

    // Phase 1: Save with default MapWorld (seed=0, version=1)
    let test_voxels = vec![
        (IVec3::new(1, 2, 3), VoxelType::Solid(1)),
        (IVec3::new(4, 5, 6), VoxelType::Solid(2)),
    ];

    let map_world = MapWorld { seed: 0, generation_version: 1 };
    save_voxel_world_to_disk_at(&test_voxels, &map_world, &save_path).unwrap();

    // Phase 2: Try to load with mismatched seed (999)
    let mismatched_seed = MapWorld { seed: 999, generation_version: 1 };
    let loaded_mods = load_voxel_world_from_disk_at(&mismatched_seed, &save_path);

    // Verify rejected due to seed mismatch
    assert_eq!(loaded_mods.len(), 0, "Should reject save due to seed mismatch");

    // Phase 3: Try to load with mismatched generation_version
    let mismatched_version = MapWorld { seed: 0, generation_version: 999 };
    let loaded_mods = load_voxel_world_from_disk_at(&mismatched_version, &save_path);

    // Verify rejected due to version mismatch
    assert_eq!(loaded_mods.len(), 0, "Should reject save due to generation_version mismatch");

    cleanup_test_files(test_name);
}

#[test]
fn test_shutdown_save() {
    // Use default path for this test since the shutdown system uses hardcoded path
    // Clean up the default production path
    let _ = fs::remove_file("world_save/voxel_world.bin");
    let _ = fs::remove_file("world_save/voxel_world.bin.tmp");
    let _ = fs::remove_dir("world_save");

    // Create app with minimal setup for shutdown save system
    let mut app = create_test_app_for_shutdown();

    // Add voxels to VoxelModifications
    let test_voxels = vec![
        (IVec3::new(10, 20, 30), VoxelType::Solid(1)),
        (IVec3::new(40, 50, 60), VoxelType::Solid(2)),
    ];

    app.insert_resource(TestVoxels(test_voxels.clone()));
    app.add_systems(Update, add_test_modifications);
    app.update(); // Run Update to add modifications

    // Set VoxelDirtyState.is_dirty = true manually
    {
        let mut dirty_state = app.world_mut().resource_mut::<VoxelDirtyState>();
        dirty_state.is_dirty = true;
        dirty_state.last_edit_time = 0.0;
        dirty_state.first_dirty_time = Some(0.0);
    }

    // Send AppExit::Success event
    app.world_mut().write_message(AppExit::Success);

    // Run Last schedule (triggers save system)
    app.update(); // This runs all schedules including Last

    // Verify save file created (uses default production path)
    assert!(Path::new("world_save/voxel_world.bin").exists(), "Save file should be created on shutdown");

    // Verify file contains voxels using the _at function
    let map_world = MapWorld::default();
    let loaded_mods = load_voxel_world_from_disk_at(&map_world, "world_save/voxel_world.bin");
    assert_eq!(loaded_mods.len(), 2, "Should save 2 voxels on shutdown");

    // Cleanup the default production path
    let _ = fs::remove_file("world_save/voxel_world.bin");
    let _ = fs::remove_file("world_save/voxel_world.bin.tmp");
    let _ = fs::remove_dir("world_save");
}
