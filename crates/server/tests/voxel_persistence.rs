use bevy::prelude::*;
use ndshape::ConstShape;
use server::map::save_dirty_chunks_sync;
use server::persistence::{load_map_meta, save_map_meta, MapMeta};
use voxel_map_engine::persistence as chunk_persist;
use voxel_map_engine::prelude::*;

#[test]
fn dirty_chunks_saved_on_debounce() {
    let dir = tempfile::tempdir().unwrap();
    let map_dir = dir.path().join("overworld");

    let mut instance = VoxelMapInstance::new(5);
    let chunk_pos = IVec3::new(1, 0, 0);
    let voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
    instance.insert_chunk_data(
        chunk_pos,
        ChunkData::from_voxels(&voxels, ChunkStatus::Full),
    );
    instance.chunk_levels.insert(chunk_to_column(chunk_pos), 0);
    instance.dirty_chunks.insert(chunk_pos);

    save_dirty_chunks_sync(&mut instance, &map_dir);

    assert!(chunk_persist::chunk_file_path(&map_dir, chunk_pos).exists());
    assert!(instance.dirty_chunks.is_empty());
}

#[test]
fn clean_chunks_not_saved() {
    let dir = tempfile::tempdir().unwrap();
    let map_dir = dir.path().join("overworld");

    let mut instance = VoxelMapInstance::new(5);
    let chunk_pos = IVec3::ZERO;
    let voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
    instance.insert_chunk_data(
        chunk_pos,
        ChunkData::from_voxels(&voxels, ChunkStatus::Full),
    );
    instance.chunk_levels.insert(chunk_to_column(chunk_pos), 0);
    // NOT marking dirty

    save_dirty_chunks_sync(&mut instance, &map_dir);

    assert!(!chunk_persist::chunk_file_path(&map_dir, chunk_pos).exists());
}

#[test]
fn terrain_persists_across_save_load() {
    let dir = tempfile::tempdir().unwrap();
    let map_dir = dir.path().join("overworld");

    // Save a chunk with a specific voxel edit
    {
        let mut voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        voxels[100] = WorldVoxel::Solid(42);
        let chunk_data = ChunkData::from_voxels(&voxels, ChunkStatus::Full);
        chunk_persist::save_chunk(&map_dir, IVec3::ZERO, &chunk_data).unwrap();

        let meta = MapMeta {
            version: 1,
            seed: 999,
            generation_version: 0,
            spawn_points: vec![Vec3::new(0.0, 5.0, 0.0)],
        };
        save_map_meta(&map_dir, &meta).unwrap();
    }

    // Load and verify
    {
        let loaded = chunk_persist::load_chunk(&map_dir, IVec3::ZERO)
            .unwrap()
            .expect("chunk should exist");
        let loaded_voxels = loaded.voxels.to_voxels();
        assert_eq!(loaded_voxels[100], WorldVoxel::Solid(42));
        assert_eq!(loaded_voxels[0], WorldVoxel::Air);

        let meta = load_map_meta(&map_dir).unwrap().expect("meta should exist");
        assert_eq!(meta.seed, 999);
        assert_eq!(meta.spawn_points.len(), 1);
    }
}

#[test]
fn evicted_dirty_chunk_saved_before_removal() {
    let dir = tempfile::tempdir().unwrap();
    let map_dir = dir.path().join("overworld");

    // Set up an instance with a dirty chunk
    let mut instance = VoxelMapInstance::new(5);
    let chunk_pos = IVec3::new(3, 0, 0);
    let mut voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
    voxels[50] = WorldVoxel::Solid(7);
    instance.insert_chunk_data(
        chunk_pos,
        ChunkData::from_voxels(&voxels, ChunkStatus::Full),
    );
    instance.chunk_levels.insert(chunk_to_column(chunk_pos), 0);
    instance.dirty_chunks.insert(chunk_pos);

    // Save all dirty chunks (simulates what eviction does before removing)
    save_dirty_chunks_sync(&mut instance, &map_dir);

    // Then remove from octree (simulates eviction completing)
    instance.chunk_levels.remove(&chunk_to_column(chunk_pos));
    instance.remove_chunk_data(chunk_pos);

    // Verify chunk was persisted before removal
    let loaded = chunk_persist::load_chunk(&map_dir, chunk_pos)
        .unwrap()
        .expect("evicted dirty chunk should have been saved");
    let loaded_voxels = loaded.voxels.to_voxels();
    assert_eq!(loaded_voxels[50], WorldVoxel::Solid(7));

    // Verify chunk is no longer in memory
    assert!(!instance
        .chunk_levels
        .contains_key(&chunk_to_column(chunk_pos)));
    assert!(instance.get_chunk_data(chunk_pos).is_none());
    assert!(instance.dirty_chunks.is_empty());
}
