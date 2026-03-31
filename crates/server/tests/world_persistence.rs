use bevy::prelude::*;
use ndshape::ConstShape;
use server::map::save_dirty_chunks_sync;
use server::persistence::{
    load_entities, load_map_meta, map_save_dir, save_entities, save_map_meta, MapMeta,
};
use voxel_map_engine::persistence as chunk_persist;
use voxel_map_engine::prelude::*;

use protocol::map::{SavedEntity, SavedEntityKind};
use protocol::MapInstanceId;

#[test]
fn terrain_persists_across_server_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let map_dir = tmp.path().join("overworld");

    // First run: save chunk data and metadata
    {
        let mut voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        voxels[100] = WorldVoxel::Solid(42);
        let chunk_data = ChunkData::from_voxels(&voxels, ChunkStatus::Full);
        let chunk_pos = IVec3::new(0, 0, 0);

        chunk_persist::save_chunk(&map_dir, chunk_pos, &chunk_data).expect("save chunk");

        let meta = MapMeta {
            version: 1,
            seed: 999,
            generation_version: 0,
            spawn_points: vec![Vec3::new(0.0, 5.0, 0.0)],
        };
        save_map_meta(&map_dir, &meta).expect("save meta");
    }

    // Second run: verify data loads correctly
    {
        let chunk_pos = IVec3::new(0, 0, 0);
        let loaded = chunk_persist::load_chunk(&map_dir, chunk_pos)
            .expect("load chunk")
            .expect("chunk should exist");

        let loaded_voxels = loaded.voxels.to_voxels();
        assert_eq!(loaded_voxels[100], WorldVoxel::Solid(42));
        assert_eq!(loaded_voxels[0], WorldVoxel::Air);

        let meta = load_map_meta(&map_dir)
            .expect("load meta")
            .expect("meta should exist");
        assert_eq!(meta.seed, 999);
        assert_eq!(meta.spawn_points.len(), 1);
    }
}

#[test]
fn multiple_chunks_persist_independently() {
    let tmp = tempfile::tempdir().unwrap();
    let map_dir = tmp.path().join("overworld");

    let positions = [
        IVec3::new(0, 0, 0),
        IVec3::new(1, 0, 0),
        IVec3::new(-1, 2, 3),
    ];

    // Save three chunks with distinct data
    for (i, &pos) in positions.iter().enumerate() {
        let mut voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        voxels[i + 10] = WorldVoxel::Solid(i as u8 + 1);
        let chunk_data = ChunkData::from_voxels(&voxels, ChunkStatus::Full);
        chunk_persist::save_chunk(&map_dir, pos, &chunk_data).unwrap();
    }

    // Verify each loads independently with correct data
    for (i, &pos) in positions.iter().enumerate() {
        let loaded = chunk_persist::load_chunk(&map_dir, pos)
            .unwrap()
            .expect("chunk should exist");
        let voxels = loaded.voxels.to_voxels();
        assert_eq!(voxels[i + 10], WorldVoxel::Solid(i as u8 + 1));
    }

    // Verify listing
    let mut found = chunk_persist::list_saved_chunks(&map_dir).unwrap();
    found.sort_by_key(|p| (p.x, p.y, p.z));
    let mut expected = positions.to_vec();
    expected.sort_by_key(|p| (p.x, p.y, p.z));
    assert_eq!(found, expected);
}

#[test]
fn map_save_dir_routes_correctly() {
    let base = std::path::Path::new("/tmp/test_worlds");
    assert_eq!(
        map_save_dir(base, &MapInstanceId::Overworld),
        std::path::PathBuf::from("/tmp/test_worlds/overworld")
    );
    assert_eq!(
        map_save_dir(base, &MapInstanceId::Homebase { owner: 42 }),
        std::path::PathBuf::from("/tmp/test_worlds/homebase-42")
    );
}

#[test]
fn dirty_instance_save_then_reload() {
    let tmp = tempfile::tempdir().unwrap();
    let map_dir = tmp.path().join("overworld");

    // Create instance, make edits, save dirty chunks
    let mut instance = VoxelMapInstance::new(5);
    let chunk_pos = IVec3::ZERO;
    let voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
    instance.insert_chunk_data(
        chunk_pos,
        ChunkData::from_voxels(&voxels, ChunkStatus::Full),
    );
    instance.chunk_levels.insert(chunk_to_column(chunk_pos), 0);

    // Mutate a voxel (marks chunk dirty)
    instance.set_voxel(IVec3::new(5, 5, 5), WorldVoxel::Solid(99));
    assert!(instance.dirty_chunks.contains(&chunk_pos));

    // Save dirty chunks
    save_dirty_chunks_sync(&mut instance, &map_dir);
    assert!(instance.dirty_chunks.is_empty());

    // Reload from disk and verify the edit persisted
    let loaded = chunk_persist::load_chunk(&map_dir, chunk_pos)
        .unwrap()
        .expect("chunk should exist on disk");
    let local = IVec3::new(5, 5, 5);
    let padded = [
        (local.x + 1) as u32,
        (local.y + 1) as u32,
        (local.z + 1) as u32,
    ];
    let index = PaddedChunkShape::linearize(padded) as usize;
    assert_eq!(loaded.voxels.get(index), WorldVoxel::Solid(99));
}

#[test]
fn meta_and_chunks_coexist_in_map_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let map_dir = tmp.path().join("overworld");

    // Save metadata
    let meta = MapMeta {
        version: 1,
        seed: 42,
        generation_version: 1,
        spawn_points: vec![Vec3::new(10.0, 20.0, 30.0)],
    };
    save_map_meta(&map_dir, &meta).unwrap();

    // Save a chunk
    let voxels = vec![WorldVoxel::Solid(1); PaddedChunkShape::USIZE];
    chunk_persist::save_chunk(
        &map_dir,
        IVec3::ZERO,
        &ChunkData::from_voxels(&voxels, ChunkStatus::Full),
    )
    .unwrap();

    // Both should exist and load independently
    assert!(map_dir.join("map.meta.bin").exists());
    assert!(map_dir.join("terrain").exists());

    let loaded_meta = load_map_meta(&map_dir).unwrap().expect("meta exists");
    assert_eq!(loaded_meta.seed, 42);

    let loaded_chunk = chunk_persist::load_chunk(&map_dir, IVec3::ZERO)
        .unwrap()
        .expect("chunk exists");
    assert_eq!(loaded_chunk.voxels.get(0), WorldVoxel::Solid(1));
}

#[test]
fn multiple_maps_save_independently() {
    let tmp = tempfile::tempdir().unwrap();
    let ow_dir = map_save_dir(tmp.path(), &MapInstanceId::Overworld);
    let hb_dir = map_save_dir(tmp.path(), &MapInstanceId::Homebase { owner: 42 });

    let mut ow_voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
    ow_voxels[0] = WorldVoxel::Solid(1);
    chunk_persist::save_chunk(
        &ow_dir,
        IVec3::ZERO,
        &ChunkData::from_voxels(&ow_voxels, ChunkStatus::Full),
    )
    .unwrap();

    let mut hb_voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
    hb_voxels[0] = WorldVoxel::Solid(99);
    chunk_persist::save_chunk(
        &hb_dir,
        IVec3::ZERO,
        &ChunkData::from_voxels(&hb_voxels, ChunkStatus::Full),
    )
    .unwrap();

    let ow_loaded = chunk_persist::load_chunk(&ow_dir, IVec3::ZERO)
        .unwrap()
        .unwrap();
    let hb_loaded = chunk_persist::load_chunk(&hb_dir, IVec3::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(ow_loaded.voxels.get(0), WorldVoxel::Solid(1));
    assert_eq!(hb_loaded.voxels.get(0), WorldVoxel::Solid(99));
}

#[test]
fn homebase_metadata_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let hb_dir = map_save_dir(tmp.path(), &MapInstanceId::Homebase { owner: 123 });

    let meta = MapMeta {
        version: 1,
        seed: 123,
        generation_version: 0,
        spawn_points: vec![Vec3::new(0.0, 5.0, 0.0)],
    };
    save_map_meta(&hb_dir, &meta).unwrap();

    let loaded = load_map_meta(&hb_dir).unwrap().expect("meta should exist");
    assert_eq!(loaded.seed, 123);
}

#[test]
fn homebase_entities_saved_separately() {
    let tmp = tempfile::tempdir().unwrap();
    let ow_dir = map_save_dir(tmp.path(), &MapInstanceId::Overworld);
    let hb_dir = map_save_dir(tmp.path(), &MapInstanceId::Homebase { owner: 1 });

    save_entities(
        &ow_dir,
        &[SavedEntity {
            kind: SavedEntityKind::RespawnPoint,
            position: Vec3::ZERO,
        }],
    )
    .unwrap();
    save_entities(
        &hb_dir,
        &[
            SavedEntity {
                kind: SavedEntityKind::RespawnPoint,
                position: Vec3::ONE,
            },
            SavedEntity {
                kind: SavedEntityKind::RespawnPoint,
                position: Vec3::NEG_ONE,
            },
        ],
    )
    .unwrap();

    assert_eq!(load_entities(&ow_dir).unwrap().len(), 1);
    assert_eq!(load_entities(&hb_dir).unwrap().len(), 2);
}

#[test]
fn map_save_dir_different_homebases_are_isolated() {
    let base = std::path::Path::new("worlds");
    let dir1 = map_save_dir(base, &MapInstanceId::Homebase { owner: 1 });
    let dir2 = map_save_dir(base, &MapInstanceId::Homebase { owner: 2 });
    assert_ne!(dir1, dir2);
    assert_eq!(dir1, std::path::PathBuf::from("worlds/homebase-1"));
    assert_eq!(dir2, std::path::PathBuf::from("worlds/homebase-2"));
}

#[test]
fn overworld_and_homebase_dirs_are_isolated() {
    let base = std::path::Path::new("worlds");
    let ow = map_save_dir(base, &MapInstanceId::Overworld);
    let hb = map_save_dir(base, &MapInstanceId::Homebase { owner: 1 });
    assert_ne!(ow, hb);
}

#[test]
fn entities_persist_across_server_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let map_dir = tmp.path().join("overworld");

    // First run: save respawn points and metadata
    {
        let entities = vec![
            SavedEntity {
                kind: SavedEntityKind::RespawnPoint,
                position: Vec3::new(0.0, 5.0, 0.0),
            },
            SavedEntity {
                kind: SavedEntityKind::RespawnPoint,
                position: Vec3::new(10.0, 20.0, 30.0),
            },
        ];
        save_entities(&map_dir, &entities).expect("save entities");

        let meta = MapMeta {
            version: 1,
            seed: 999,
            generation_version: 0,
            spawn_points: vec![Vec3::new(0.0, 5.0, 0.0), Vec3::new(10.0, 20.0, 30.0)],
        };
        save_map_meta(&map_dir, &meta).expect("save meta");
    }

    // Verify entities.bin exists on disk
    assert!(map_dir.join("entities.bin").exists());

    // Second run: verify entities load correctly from disk
    {
        let loaded = load_entities(&map_dir).expect("load entities");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].kind, SavedEntityKind::RespawnPoint);
        assert_eq!(loaded[0].position, Vec3::new(0.0, 5.0, 0.0));
        assert_eq!(loaded[1].position, Vec3::new(10.0, 20.0, 30.0));

        let meta = load_map_meta(&map_dir)
            .expect("load meta")
            .expect("meta should exist");
        assert_eq!(meta.spawn_points.len(), 2);
        assert!(meta.spawn_points.contains(&Vec3::new(0.0, 5.0, 0.0)));
        assert!(meta.spawn_points.contains(&Vec3::new(10.0, 20.0, 30.0)));
    }
}

#[test]
fn entities_and_chunks_coexist_in_map_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let map_dir = tmp.path().join("overworld");

    // Save entities
    let entities = vec![SavedEntity {
        kind: SavedEntityKind::RespawnPoint,
        position: Vec3::new(5.0, 10.0, 15.0),
    }];
    save_entities(&map_dir, &entities).unwrap();

    // Save a chunk
    let voxels = vec![WorldVoxel::Solid(1); PaddedChunkShape::USIZE];
    chunk_persist::save_chunk(
        &map_dir,
        IVec3::ZERO,
        &ChunkData::from_voxels(&voxels, ChunkStatus::Full),
    )
    .unwrap();

    // Save metadata
    let meta = MapMeta {
        version: 1,
        seed: 42,
        generation_version: 0,
        spawn_points: vec![Vec3::new(5.0, 10.0, 15.0)],
    };
    save_map_meta(&map_dir, &meta).unwrap();

    // All three coexist and load independently
    assert!(map_dir.join("entities.bin").exists());
    assert!(map_dir.join("map.meta.bin").exists());
    assert!(map_dir.join("terrain").exists());

    let loaded_entities = load_entities(&map_dir).unwrap();
    assert_eq!(loaded_entities.len(), 1);

    let loaded_meta = load_map_meta(&map_dir).unwrap().expect("meta exists");
    assert_eq!(loaded_meta.seed, 42);

    let loaded_chunk = chunk_persist::load_chunk(&map_dir, IVec3::ZERO)
        .unwrap()
        .expect("chunk exists");
    assert_eq!(loaded_chunk.voxels.get(0), WorldVoxel::Solid(1));
}
