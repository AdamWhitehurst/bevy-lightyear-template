use std::sync::Arc;

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
            VoxelMapConfig {
                seed: 0,
                spawning_distance,
                bounds: None,
                tree_height: 5,
                generator,
            },
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

fn loaded_chunk_count(app: &App, map_entity: Entity) -> usize {
    app.world()
        .get::<VoxelMapInstance>(map_entity)
        .unwrap()
        .loaded_chunks
        .len()
}

#[test]
fn pending_chunks_auto_inserted() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);
    assert!(app.world().get::<PendingChunks>(map).is_none());
    app.update();
    assert!(app.world().get::<PendingChunks>(map).is_some());
}

#[test]
fn chunks_spawn_within_range() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);

    // Target at origin with distance=1 -> 3x3x3 = 27 chunk positions
    spawn_target(&mut app, map, Vec3::ZERO, 1);

    // Several ticks to let ensure_pending_chunks run, then async tasks complete
    tick(&mut app, 20);

    let loaded = loaded_chunk_count(&app, map);
    assert_eq!(
        loaded, 27,
        "distance=1 around origin should load 3^3=27 chunks"
    );
}

#[test]
fn chunks_despawn_outside_range() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);
    let target = spawn_target(&mut app, map, Vec3::ZERO, 1);

    // Let chunks generate
    tick(&mut app, 20);
    let initial_loaded = loaded_chunk_count(&app, map);
    assert!(initial_loaded > 0, "should have loaded some chunks");

    // Move target far away - all original chunks should unload
    app.world_mut()
        .entity_mut(target)
        .insert(Transform::from_translation(Vec3::new(10000.0, 0.0, 0.0)));

    tick(&mut app, 5);

    // Original chunks at origin should be unloaded (no longer in loaded set)
    let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
    let has_origin = instance.loaded_chunks.contains(&IVec3::ZERO);
    assert!(
        !has_origin,
        "origin chunk should be unloaded after target moved away"
    );
}

#[test]
fn bounded_map_respects_bounds() {
    let mut app = test_app();
    let generator: SdfGenerator = Arc::new(flat_terrain_sdf);
    let map = app
        .world_mut()
        .spawn((
            VoxelMapInstance::new(5),
            VoxelMapConfig {
                seed: 0,
                spawning_distance: 5,
                bounds: Some(IVec3::new(2, 2, 2)),
                tree_height: 5,
                generator,
            },
            Transform::default(),
        ))
        .id();

    // Target at origin with distance=5 but bounds=2 -> only -1..1 per axis = 3^3 = 27
    spawn_target(&mut app, map, Vec3::ZERO, 5);

    tick(&mut app, 30);

    let loaded = loaded_chunk_count(&app, map);
    assert_eq!(
        loaded, 27,
        "bounded map with bounds=2 should limit to 3^3=27 chunks (range -1..1)"
    );
}

#[test]
fn chunk_entities_are_children_of_map() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);
    spawn_target(&mut app, map, Vec3::ZERO, 0);

    tick(&mut app, 20);

    // distance=0 -> 1 chunk position at origin
    let loaded = loaded_chunk_count(&app, map);
    assert_eq!(loaded, 1, "distance=0 should load exactly 1 chunk");

    // Any mesh entities that exist should be children of the map
    let orphan_count: usize = app
        .world_mut()
        .query::<(&VoxelChunk, &ChildOf)>()
        .iter(app.world())
        .filter(|(_, child_of)| child_of.0 != map)
        .count();
    assert_eq!(
        orphan_count, 0,
        "all chunk entities should be children of map entity"
    );
}
