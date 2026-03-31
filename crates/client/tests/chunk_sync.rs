use std::sync::Arc;

use bevy::prelude::*;
use lightyear::prelude::{Controlled, Predicted};
use protocol::{CharacterMarker, MapInstanceId, MapRegistry};
use voxel_map_engine::prelude::{
    chunk_to_column, column_to_chunks, flat_terrain_voxels, ChunkData, ChunkStatus, ChunkTicket,
    FillType, PalettedChunk, TicketType, VoxelChunk, VoxelGenerator, VoxelMapConfig,
    VoxelMapInstance, VoxelPlugin, WorldVoxel, DEFAULT_COLUMN_Y_MAX, DEFAULT_COLUMN_Y_MIN,
};

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::transform::TransformPlugin);
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.add_plugins(VoxelPlugin);
    app.init_resource::<MapRegistry>();
    app
}

fn spawn_client_map(app: &mut App) -> Entity {
    let mut config = VoxelMapConfig::new(0, 0, 1, None, 3);
    config.generates_chunks = false;
    let map = app
        .world_mut()
        .spawn((
            VoxelMapInstance::new(3),
            config,
            VoxelGenerator(Arc::new(flat_terrain_voxels)),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();
    app.world_mut()
        .resource_mut::<MapRegistry>()
        .insert(MapInstanceId::Overworld, map);
    map
}

/// Simulate receiving ChunkDataSync by inserting chunk data directly into the
/// VoxelMapInstance — the same operations handle_chunk_data_sync performs.
fn simulate_chunk_sync(app: &mut App, map: Entity, chunk_pos: IVec3, voxels: PalettedChunk) {
    let mut instance = app
        .world_mut()
        .get_mut::<VoxelMapInstance>(map)
        .expect("map must have VoxelMapInstance");
    let chunk_data = ChunkData {
        voxels,
        fill_type: FillType::Uniform(WorldVoxel::Solid(1)),
        hash: 1,
        status: ChunkStatus::Full,
    };
    instance.insert_chunk_data(chunk_pos, chunk_data);
    instance
        .chunk_levels
        .entry(chunk_to_column(chunk_pos))
        .or_insert(0);
}

/// Simulate receiving UnloadColumn by removing chunk data and chunk_levels
/// entry — the same operations handle_unload_column performs.
fn simulate_unload_column(app: &mut App, map: Entity, col: IVec2) {
    let mut instance = app
        .world_mut()
        .get_mut::<VoxelMapInstance>(map)
        .expect("map must have VoxelMapInstance");
    for chunk_pos in column_to_chunks(col, DEFAULT_COLUMN_Y_MIN, DEFAULT_COLUMN_Y_MAX) {
        instance.remove_chunk_data(chunk_pos);
    }
    instance.chunk_levels.remove(&col);
}

#[test]
fn chunk_data_stored_after_sync() {
    let mut app = test_app();
    let map = spawn_client_map(&mut app);
    app.update();

    let chunk_pos = IVec3::ZERO;
    let voxels = PalettedChunk::SingleValue(WorldVoxel::Solid(42));
    simulate_chunk_sync(&mut app, map, chunk_pos, voxels.clone());

    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    assert!(
        instance.get_chunk_data(chunk_pos).is_some(),
        "Chunk data must be stored in VoxelMapInstance after sync"
    );
    assert_eq!(
        instance.get_chunk_data(chunk_pos).unwrap().voxels,
        voxels,
        "Stored voxel data must match what was synced"
    );
    assert!(
        instance
            .chunk_levels
            .contains_key(&chunk_to_column(chunk_pos)),
        "chunk_levels must have an entry for the synced column"
    );
}

#[test]
fn chunk_levels_not_overwritten_by_second_sync_in_same_column() {
    let mut app = test_app();
    let map = spawn_client_map(&mut app);
    app.update();

    let col = IVec2::ZERO;
    let pos_a = IVec3::new(0, 0, 0);
    let pos_b = IVec3::new(0, 1, 0);

    simulate_chunk_sync(
        &mut app,
        map,
        pos_a,
        PalettedChunk::SingleValue(WorldVoxel::Solid(1)),
    );
    simulate_chunk_sync(
        &mut app,
        map,
        pos_b,
        PalettedChunk::SingleValue(WorldVoxel::Solid(2)),
    );

    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    assert!(
        instance.get_chunk_data(pos_a).is_some(),
        "First chunk in column must still exist"
    );
    assert!(
        instance.get_chunk_data(pos_b).is_some(),
        "Second chunk in column must exist"
    );
    assert_eq!(
        instance.chunk_levels.get(&col),
        Some(&0),
        "chunk_levels entry for column must exist"
    );
}

#[test]
fn unload_column_removes_data_and_levels() {
    let mut app = test_app();
    let map = spawn_client_map(&mut app);
    app.update();

    let col = IVec2::ZERO;
    let pos_a = IVec3::new(0, 0, 0);
    let pos_b = IVec3::new(0, -1, 0);

    simulate_chunk_sync(
        &mut app,
        map,
        pos_a,
        PalettedChunk::SingleValue(WorldVoxel::Solid(1)),
    );
    simulate_chunk_sync(
        &mut app,
        map,
        pos_b,
        PalettedChunk::SingleValue(WorldVoxel::Solid(2)),
    );

    simulate_unload_column(&mut app, map, col);

    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    assert!(
        instance.get_chunk_data(pos_a).is_none(),
        "Chunk data must be removed after unload"
    );
    assert!(
        instance.get_chunk_data(pos_b).is_none(),
        "All chunks in column must be removed after unload"
    );
    assert!(
        !instance.chunk_levels.contains_key(&col),
        "chunk_levels entry must be removed after unload"
    );
}

#[test]
fn despawn_out_of_range_removes_mesh_entities_after_unload() {
    let mut app = test_app();
    let map = spawn_client_map(&mut app);
    app.update(); // Let ensure_pending_chunks run

    let chunk_pos = IVec3::ZERO;
    simulate_chunk_sync(
        &mut app,
        map,
        chunk_pos,
        PalettedChunk::SingleValue(WorldVoxel::Solid(1)),
    );

    // Spawn a mesh entity as child of the map, simulating what handle_chunk_data_sync does
    let chunk_entity = app
        .world_mut()
        .spawn((
            VoxelChunk {
                position: chunk_pos,
                lod_level: 0,
            },
            Transform::default(),
        ))
        .id();
    app.world_mut().entity_mut(map).add_child(chunk_entity);
    app.update(); // Propagate hierarchy

    // Verify the chunk entity exists
    assert!(
        app.world().get_entity(chunk_entity).is_ok(),
        "Chunk mesh entity must exist before unload"
    );

    // Simulate unload
    simulate_unload_column(&mut app, map, IVec2::ZERO);
    app.update(); // despawn_out_of_range_chunks runs

    // Mesh entity should be despawned since chunk_levels no longer contains the column
    assert!(
        app.world().get_entity(chunk_entity).is_err(),
        "Chunk mesh entity must be despawned after unload removes chunk_levels entry"
    );
}

#[test]
fn client_propagator_does_not_remove_server_pushed_data() {
    let mut app = test_app();
    // Note: ChunkGenerationEnabled is NOT inserted — client mode
    let map = spawn_client_map(&mut app);

    // Spawn a ticket entity (simulating predicted player) with GlobalTransform
    let _player = app
        .world_mut()
        .spawn((
            CharacterMarker,
            Predicted,
            Controlled,
            ChunkTicket::new(map, TicketType::Player, 2),
            Transform::from_translation(Vec3::new(0.0, 5.0, 0.0)),
        ))
        .id();

    app.update(); // ensure_pending_chunks + propagator (should be skipped)

    // Simulate server pushing a chunk far from the player (distance > LOAD_LEVEL_THRESHOLD)
    // The propagator would unload this if it were running.
    let far_pos = IVec3::new(100, 0, 100);
    simulate_chunk_sync(
        &mut app,
        map,
        far_pos,
        PalettedChunk::SingleValue(WorldVoxel::Solid(1)),
    );

    // Run several frames
    for _ in 0..10 {
        app.update();
    }

    // Data should still be present — propagator must not have removed it
    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    assert!(
        instance.get_chunk_data(far_pos).is_some(),
        "Server-pushed chunk data must survive when ChunkGenerationEnabled is absent"
    );
    assert!(
        instance
            .chunk_levels
            .contains_key(&chunk_to_column(far_pos)),
        "chunk_levels entry must survive when propagator is disabled"
    );
}
