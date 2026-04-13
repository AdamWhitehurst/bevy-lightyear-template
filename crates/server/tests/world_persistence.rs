use std::sync::Arc;

use bevy::prelude::*;
use persistence::Store;
use server::persistence::fs_map_entities::FsMapEntitiesStore;
use server::persistence::fs_map_meta::FsMapMetaStore;
use server::persistence::{map_save_dir, MapMeta};
use voxel_map_engine::persistence::fs_chunk::FsChunkStore;
use voxel_map_engine::persistence::{chunk_file_path, ChunkFileEnvelope, CHUNK_SAVE_VERSION};
use voxel_map_engine::prelude::*;

use protocol::map::{SavedEntity, SavedEntityKind};
use protocol::MapInstanceId;

/// Padded chunk volume for the default `chunk_size=16`, used by tests.
const PADDED_VOLUME_16: usize = 18 * 18 * 18;

fn test_chunk_store(dir: &std::path::Path) -> FsChunkStore {
    FsChunkStore {
        map_dir: Arc::new(dir.to_path_buf()),
    }
}

fn test_meta_store(dir: &std::path::Path) -> FsMapMetaStore {
    FsMapMetaStore {
        map_dir: Arc::new(dir.to_path_buf()),
    }
}

fn test_entity_store(dir: &std::path::Path) -> FsMapEntitiesStore {
    FsMapEntitiesStore {
        map_dir: Arc::new(dir.to_path_buf()),
    }
}

fn save_chunk(store: &FsChunkStore, pos: IVec3, chunk_size: u32, data: &ChunkData) {
    let envelope = ChunkFileEnvelope {
        version: CHUNK_SAVE_VERSION,
        chunk_size,
        data: data.clone(),
    };
    store.save(&pos, &envelope).unwrap();
}

/// Save all dirty chunks from an instance via the store.
fn save_dirty_chunks_sync(instance: &mut VoxelMapInstance, store: &FsChunkStore) {
    let chunk_size = instance.chunk_size;
    let dirty: Vec<IVec3> = instance.dirty_chunks.drain().collect();
    for chunk_pos in dirty {
        if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
            save_chunk(store, chunk_pos, chunk_size, chunk_data);
        }
    }
}

#[test]
fn terrain_persists_across_server_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let map_dir = tmp.path().join("overworld");
    let chunk_store = test_chunk_store(&map_dir);
    let meta_store = test_meta_store(&map_dir);

    // First run: save chunk data and metadata
    {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME_16];
        voxels[100] = WorldVoxel::Solid(42);
        let chunk_data = ChunkData::from_voxels(&voxels, ChunkStatus::Full);
        save_chunk(&chunk_store, IVec3::ZERO, 16, &chunk_data);

        let meta = MapMeta {
            version: 1,
            seed: 999,
            generation_version: 0,
            spawn_points: vec![Vec3::new(0.0, 5.0, 0.0)],
        };
        meta_store.save(&(), &meta).expect("save meta");
    }

    // Second run: verify data loads correctly
    {
        let loaded = chunk_store
            .load(&IVec3::ZERO)
            .expect("load chunk")
            .expect("chunk should exist");

        let loaded_voxels = loaded.data.voxels.to_voxels();
        assert_eq!(loaded_voxels[100], WorldVoxel::Solid(42));
        assert_eq!(loaded_voxels[0], WorldVoxel::Air);

        let meta = meta_store
            .load(&())
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
    let store = test_chunk_store(&map_dir);

    let positions = [
        IVec3::new(0, 0, 0),
        IVec3::new(1, 0, 0),
        IVec3::new(-1, 2, 3),
    ];

    // Save three chunks with distinct data
    for (i, &pos) in positions.iter().enumerate() {
        let mut voxels = vec![WorldVoxel::Air; PADDED_VOLUME_16];
        voxels[i + 10] = WorldVoxel::Solid(i as u8 + 1);
        let chunk_data = ChunkData::from_voxels(&voxels, ChunkStatus::Full);
        save_chunk(&store, pos, 16, &chunk_data);
    }

    // Verify each loads independently with correct data
    for (i, &pos) in positions.iter().enumerate() {
        let loaded = store.load(&pos).unwrap().expect("chunk should exist");
        let voxels = loaded.data.voxels.to_voxels();
        assert_eq!(voxels[i + 10], WorldVoxel::Solid(i as u8 + 1));
    }

    // Verify files exist on disk
    for &pos in &positions {
        assert!(chunk_file_path(&map_dir, pos).exists());
    }
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
    let store = test_chunk_store(&map_dir);

    // Create instance, make edits, save dirty chunks
    let mut instance = VoxelMapInstance::new(5, 16);
    let chunk_pos = IVec3::ZERO;
    let voxels = vec![WorldVoxel::Air; PADDED_VOLUME_16];
    instance.insert_chunk_data(
        chunk_pos,
        ChunkData::from_voxels(&voxels, ChunkStatus::Full),
    );
    instance.chunk_levels.insert(chunk_to_column(chunk_pos), 0);

    // Mutate a voxel (marks chunk dirty)
    instance.set_voxel(IVec3::new(5, 5, 5), WorldVoxel::Solid(99));
    assert!(instance.dirty_chunks.contains(&chunk_pos));

    // Save dirty chunks
    save_dirty_chunks_sync(&mut instance, &store);
    assert!(instance.dirty_chunks.is_empty());

    // Reload from disk and verify the edit persisted
    let loaded = store
        .load(&chunk_pos)
        .unwrap()
        .expect("chunk should exist on disk");
    let local = IVec3::new(5, 5, 5);
    let padded = [
        (local.x + 1) as u32,
        (local.y + 1) as u32,
        (local.z + 1) as u32,
    ];
    let index = RuntimeShape::<u32, 3>::new([18, 18, 18]).linearize(padded) as usize;
    assert_eq!(loaded.data.voxels.get(index), WorldVoxel::Solid(99));
}

#[test]
fn meta_and_chunks_coexist_in_map_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let map_dir = tmp.path().join("overworld");
    let chunk_store = test_chunk_store(&map_dir);
    let meta_store = test_meta_store(&map_dir);

    // Save metadata
    let meta = MapMeta {
        version: 1,
        seed: 42,
        generation_version: 1,
        spawn_points: vec![Vec3::new(10.0, 20.0, 30.0)],
    };
    meta_store.save(&(), &meta).unwrap();

    // Save a chunk
    let voxels = vec![WorldVoxel::Solid(1); PADDED_VOLUME_16];
    save_chunk(
        &chunk_store,
        IVec3::ZERO,
        16,
        &ChunkData::from_voxels(&voxels, ChunkStatus::Full),
    );

    // Both should exist and load independently
    assert!(map_dir.join("map.meta.bin").exists());
    assert!(map_dir.join("terrain").exists());

    let loaded_meta = meta_store.load(&()).unwrap().expect("meta exists");
    assert_eq!(loaded_meta.seed, 42);

    let loaded_chunk = chunk_store
        .load(&IVec3::ZERO)
        .unwrap()
        .expect("chunk exists");
    assert_eq!(loaded_chunk.data.voxels.get(0), WorldVoxel::Solid(1));
}

#[test]
fn multiple_maps_save_independently() {
    let tmp = tempfile::tempdir().unwrap();
    let ow_dir = map_save_dir(tmp.path(), &MapInstanceId::Overworld);
    let hb_dir = map_save_dir(tmp.path(), &MapInstanceId::Homebase { owner: 42 });
    let ow_store = test_chunk_store(&ow_dir);
    let hb_store = test_chunk_store(&hb_dir);

    let mut ow_voxels = vec![WorldVoxel::Air; PADDED_VOLUME_16];
    ow_voxels[0] = WorldVoxel::Solid(1);
    save_chunk(
        &ow_store,
        IVec3::ZERO,
        16,
        &ChunkData::from_voxels(&ow_voxels, ChunkStatus::Full),
    );

    let mut hb_voxels = vec![WorldVoxel::Air; PADDED_VOLUME_16];
    hb_voxels[0] = WorldVoxel::Solid(99);
    save_chunk(
        &hb_store,
        IVec3::ZERO,
        16,
        &ChunkData::from_voxels(&hb_voxels, ChunkStatus::Full),
    );

    let ow_loaded = ow_store.load(&IVec3::ZERO).unwrap().unwrap();
    let hb_loaded = hb_store.load(&IVec3::ZERO).unwrap().unwrap();
    assert_eq!(ow_loaded.data.voxels.get(0), WorldVoxel::Solid(1));
    assert_eq!(hb_loaded.data.voxels.get(0), WorldVoxel::Solid(99));
}

#[test]
fn homebase_metadata_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let hb_dir = map_save_dir(tmp.path(), &MapInstanceId::Homebase { owner: 123 });
    let store = test_meta_store(&hb_dir);

    let meta = MapMeta {
        version: 1,
        seed: 123,
        generation_version: 0,
        spawn_points: vec![Vec3::new(0.0, 5.0, 0.0)],
    };
    store.save(&(), &meta).unwrap();

    let loaded = store.load(&()).unwrap().expect("meta should exist");
    assert_eq!(loaded.seed, 123);
}

#[test]
fn homebase_entities_saved_separately() {
    let tmp = tempfile::tempdir().unwrap();
    let ow_dir = map_save_dir(tmp.path(), &MapInstanceId::Overworld);
    let hb_dir = map_save_dir(tmp.path(), &MapInstanceId::Homebase { owner: 1 });
    let ow_store = test_entity_store(&ow_dir);
    let hb_store = test_entity_store(&hb_dir);

    ow_store
        .save(
            &(),
            &vec![SavedEntity {
                kind: SavedEntityKind::RespawnPoint,
                position: Vec3::ZERO,
            }],
        )
        .unwrap();
    hb_store
        .save(
            &(),
            &vec![
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

    assert_eq!(ow_store.load(&()).unwrap().unwrap().len(), 1);
    assert_eq!(hb_store.load(&()).unwrap().unwrap().len(), 2);
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
    let entity_store = test_entity_store(&map_dir);
    let meta_store = test_meta_store(&map_dir);

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
        entity_store.save(&(), &entities).expect("save entities");

        let meta = MapMeta {
            version: 1,
            seed: 999,
            generation_version: 0,
            spawn_points: vec![Vec3::new(0.0, 5.0, 0.0), Vec3::new(10.0, 20.0, 30.0)],
        };
        meta_store.save(&(), &meta).expect("save meta");
    }

    // Verify entities.bin exists on disk
    assert!(map_dir.join("entities.bin").exists());

    // Second run: verify entities load correctly from disk
    {
        let loaded = entity_store
            .load(&())
            .expect("load entities")
            .expect("entities should exist");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].kind, SavedEntityKind::RespawnPoint);
        assert_eq!(loaded[0].position, Vec3::new(0.0, 5.0, 0.0));
        assert_eq!(loaded[1].position, Vec3::new(10.0, 20.0, 30.0));

        let meta = meta_store
            .load(&())
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
    let entity_store = test_entity_store(&map_dir);
    let chunk_store = test_chunk_store(&map_dir);
    let meta_store = test_meta_store(&map_dir);

    // Save entities
    let entities = vec![SavedEntity {
        kind: SavedEntityKind::RespawnPoint,
        position: Vec3::new(5.0, 10.0, 15.0),
    }];
    entity_store.save(&(), &entities).unwrap();

    // Save a chunk
    let voxels = vec![WorldVoxel::Solid(1); PADDED_VOLUME_16];
    save_chunk(
        &chunk_store,
        IVec3::ZERO,
        16,
        &ChunkData::from_voxels(&voxels, ChunkStatus::Full),
    );

    // Save metadata
    let meta = MapMeta {
        version: 1,
        seed: 42,
        generation_version: 0,
        spawn_points: vec![Vec3::new(5.0, 10.0, 15.0)],
    };
    meta_store.save(&(), &meta).unwrap();

    // All three coexist and load independently
    assert!(map_dir.join("entities.bin").exists());
    assert!(map_dir.join("map.meta.bin").exists());
    assert!(map_dir.join("terrain").exists());

    let loaded_entities = entity_store.load(&()).unwrap().unwrap();
    assert_eq!(loaded_entities.len(), 1);

    let loaded_meta = meta_store.load(&()).unwrap().expect("meta exists");
    assert_eq!(loaded_meta.seed, 42);

    let loaded_chunk = chunk_store
        .load(&IVec3::ZERO)
        .unwrap()
        .expect("chunk exists");
    assert_eq!(loaded_chunk.data.voxels.get(0), WorldVoxel::Solid(1));
}
