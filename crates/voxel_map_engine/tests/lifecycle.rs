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
    app.world_mut()
        .spawn((
            VoxelMapInstance::new(5),
            VoxelMapConfig::new(0, 0, spawning_distance, None, 5),
            VoxelGenerator(Arc::new(flat_terrain_voxels)),
            Transform::default(),
        ))
        .id()
}

fn spawn_ticket(app: &mut App, map_entity: Entity, position: Vec3, distance: u32) -> Entity {
    app.world_mut()
        .spawn((
            ChunkTicket::new(map_entity, TicketType::Player, distance),
            Transform::from_translation(position),
            GlobalTransform::default(),
        ))
        .id()
}

const MAX_TICKS: usize = 200;

fn tick_until(app: &mut App, condition: impl Fn(&App) -> bool) {
    for _ in 0..MAX_TICKS {
        app.update();
        if condition(app) {
            return;
        }
    }
    panic!("condition not met after {MAX_TICKS} ticks");
}

fn loaded_chunk_count(app: &App, map_entity: Entity) -> usize {
    app.world()
        .get::<VoxelMapInstance>(map_entity)
        .unwrap()
        .chunk_levels
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

    // Target at origin with distance=1 -> 3x3 = 9 columns
    spawn_ticket(&mut app, map, Vec3::ZERO, 1);

    // Poll until async chunk generation tasks complete
    tick_until(&mut app, |app| loaded_chunk_count(app, map) == 9);

    assert_eq!(
        loaded_chunk_count(&app, map),
        9,
        "distance=1 around origin should load 3x3=9 columns"
    );
}

#[test]
fn chunks_despawn_outside_range() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);
    let target = spawn_ticket(&mut app, map, Vec3::ZERO, 1);

    tick_until(&mut app, |app| loaded_chunk_count(app, map) > 0);

    // Move target far away - all original chunks should unload
    app.world_mut()
        .entity_mut(target)
        .insert(Transform::from_translation(Vec3::new(10000.0, 0.0, 0.0)));

    tick_until(&mut app, |app| {
        !app.world()
            .get::<VoxelMapInstance>(map)
            .unwrap()
            .chunk_levels
            .contains_key(&chunk_to_column(IVec3::ZERO))
    });

    assert!(
        !app.world()
            .get::<VoxelMapInstance>(map)
            .unwrap()
            .chunk_levels
            .contains_key(&chunk_to_column(IVec3::ZERO)),
        "origin chunk should be unloaded after target moved away"
    );
}

#[test]
fn bounded_map_respects_bounds() {
    let mut app = test_app();
    let map = app
        .world_mut()
        .spawn((
            VoxelMapInstance::new(5),
            VoxelMapConfig::new(0, 0, 5, Some(IVec3::new(2, 2, 2)), 5),
            VoxelGenerator(Arc::new(flat_terrain_voxels)),
            Transform::default(),
        ))
        .id();

    // Target at origin with distance=5 but bounds=2 -> only -1..1 per axis = 3x3 = 9 columns
    spawn_ticket(&mut app, map, Vec3::ZERO, 5);

    tick_until(&mut app, |app| loaded_chunk_count(app, map) == 9);

    assert_eq!(
        loaded_chunk_count(&app, map),
        9,
        "bounded map with bounds=2 should limit to 3x3=9 columns (range -1..1)"
    );
}

#[test]
fn chunk_ticket_routes_to_correct_map() {
    let mut app = test_app();
    let map_a = spawn_map(&mut app, 1);
    let map_b = spawn_map(&mut app, 1);

    // Target only points at map_a
    spawn_ticket(&mut app, map_a, Vec3::ZERO, 0);

    tick_until(&mut app, |app| loaded_chunk_count(app, map_a) == 1);

    assert_eq!(
        loaded_chunk_count(&app, map_a),
        1,
        "map_a should have 1 loaded chunk (distance=0)"
    );
    assert_eq!(
        loaded_chunk_count(&app, map_b),
        0,
        "map_b should have 0 loaded chunks (no target pointing to it)"
    );
}

#[test]
fn switching_chunk_ticket_between_maps() {
    let mut app = test_app();
    let map_a = spawn_map(&mut app, 1);
    let map_b = spawn_map(&mut app, 1);

    let target = spawn_ticket(&mut app, map_a, Vec3::ZERO, 0);

    tick_until(&mut app, |app| loaded_chunk_count(app, map_a) == 1);
    assert_eq!(loaded_chunk_count(&app, map_b), 0);

    // Switch target to map_b
    app.world_mut()
        .entity_mut(target)
        .insert(ChunkTicket::new(map_b, TicketType::Player, 0));

    tick_until(&mut app, |app| {
        loaded_chunk_count(app, map_a) == 0 && loaded_chunk_count(app, map_b) == 1
    });

    assert_eq!(
        loaded_chunk_count(&app, map_a),
        0,
        "map_a should unload after target switched away"
    );
    assert_eq!(
        loaded_chunk_count(&app, map_b),
        1,
        "map_b should load after target switched to it"
    );
}

#[test]
fn multiple_targets_on_different_maps() {
    let mut app = test_app();
    let map_a = spawn_map(&mut app, 1);
    let map_b = spawn_map(&mut app, 1);

    // Target A at origin → map_a, Target B at origin → map_b
    spawn_ticket(&mut app, map_a, Vec3::ZERO, 1);
    spawn_ticket(&mut app, map_b, Vec3::ZERO, 0);

    tick_until(&mut app, |app| {
        loaded_chunk_count(app, map_a) == 9 && loaded_chunk_count(app, map_b) == 1
    });

    assert_eq!(
        loaded_chunk_count(&app, map_a),
        9,
        "map_a should have 3x3=9 columns (distance=1)"
    );
    assert_eq!(
        loaded_chunk_count(&app, map_b),
        1,
        "map_b should have 1 chunk (distance=0)"
    );
}

#[test]
fn player_entity_drives_chunk_loading() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);

    // Simulate player entity with ChunkTicket (instead of camera)
    let player = app
        .world_mut()
        .spawn((
            ChunkTicket::new(map, TicketType::Player, 1),
            Transform::from_translation(Vec3::ZERO),
            GlobalTransform::default(),
        ))
        .id();

    tick_until(&mut app, |app| loaded_chunk_count(app, map) == 9);

    assert_eq!(
        loaded_chunk_count(&app, map),
        9,
        "player-driven ChunkTicket should load 3x3 columns"
    );

    // Remove ChunkTicket — chunks should unload
    app.world_mut().entity_mut(player).remove::<ChunkTicket>();

    tick_until(&mut app, |app| loaded_chunk_count(app, map) == 0);

    assert_eq!(
        loaded_chunk_count(&app, map),
        0,
        "chunks should unload after ChunkTicket removed from player"
    );
}

#[test]
fn chunk_entities_are_children_of_map() {
    let mut app = test_app();
    let map = spawn_map(&mut app, 1);
    spawn_ticket(&mut app, map, Vec3::ZERO, 0);

    tick_until(&mut app, |app| loaded_chunk_count(app, map) == 1);

    assert_eq!(
        loaded_chunk_count(&app, map),
        1,
        "distance=0 should load exactly 1 chunk"
    );

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
