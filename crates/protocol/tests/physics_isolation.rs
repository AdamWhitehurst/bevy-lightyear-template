use std::time::Duration;

use avian3d::prelude::*;
use bevy::{ecs::system::RunSystemOnce, prelude::*, time::TimeUpdateStrategy};
use protocol::hit_detection::terrain_collision_layers;
use protocol::map::MapInstanceId;
use protocol::physics::MapCollisionHooks;

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app
}

/// filter_pairs returns true when both entities share the same MapInstanceId.
#[test]
fn filter_pairs_same_map_allows_collision() {
    let mut app = test_app();
    let map = app.world_mut().spawn_empty().id();
    let a = app.world_mut().spawn(MapInstanceId(map)).id();
    let b = app.world_mut().spawn(MapInstanceId(map)).id();

    app.world_mut()
        .run_system_once(move |hooks: MapCollisionHooks, mut commands: Commands| {
            assert!(hooks.filter_pairs(a, b, &mut commands));
        })
        .unwrap();
}

/// filter_pairs returns false when entities have different MapInstanceIds.
#[test]
fn filter_pairs_different_map_blocks_collision() {
    let mut app = test_app();
    let map_a = app.world_mut().spawn_empty().id();
    let map_b = app.world_mut().spawn_empty().id();
    let a = app.world_mut().spawn(MapInstanceId(map_a)).id();
    let b = app.world_mut().spawn(MapInstanceId(map_b)).id();

    app.world_mut()
        .run_system_once(move |hooks: MapCollisionHooks, mut commands: Commands| {
            assert!(!hooks.filter_pairs(a, b, &mut commands));
        })
        .unwrap();
}

/// filter_pairs returns true when one or both entities lack MapInstanceId.
#[test]
fn filter_pairs_missing_map_allows_collision() {
    let mut app = test_app();
    let map = app.world_mut().spawn_empty().id();
    let a = app.world_mut().spawn(MapInstanceId(map)).id();
    let b = app.world_mut().spawn_empty().id();

    app.world_mut()
        .run_system_once(move |hooks: MapCollisionHooks, mut commands: Commands| {
            assert!(hooks.filter_pairs(a, b, &mut commands));
        })
        .unwrap();
}

/// MapInstanceId's #[require] automatically inserts ActiveCollisionHooks::FILTER_PAIRS.
#[test]
fn map_instance_id_requires_active_collision_hooks() {
    let mut app = test_app();
    let map = app.world_mut().spawn_empty().id();
    let entity = app.world_mut().spawn(MapInstanceId(map)).id();
    app.update();

    let hooks = app.world().get::<ActiveCollisionHooks>(entity);
    assert!(hooks.is_some(), "ActiveCollisionHooks should be present");
    assert!(
        hooks.unwrap().contains(ActiveCollisionHooks::FILTER_PAIRS),
        "FILTER_PAIRS flag should be set"
    );
}

/// SpatialQuery raycast only detects terrain with matching MapInstanceId.
#[test]
fn raycast_ignores_different_map_terrain() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy::asset::AssetPlugin::default(),
        bevy::mesh::MeshPlugin,
        PhysicsPlugins::default()
            .with_collision_hooks::<MapCollisionHooks>()
            .build(),
    ));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
        1.0 / 60.0,
    )));
    app.finish();

    let map_a = app.world_mut().spawn_empty().id();
    let map_b = app.world_mut().spawn_empty().id();

    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(10.0, 1.0, 10.0),
        Position(Vec3::ZERO),
        Rotation::default(),
        MapInstanceId(map_a),
        terrain_collision_layers(),
    ));

    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(10.0, 1.0, 10.0),
        Position(Vec3::ZERO),
        Rotation::default(),
        MapInstanceId(map_b),
        terrain_collision_layers(),
    ));

    // Step physics to build spatial index
    for _ in 0..3 {
        app.update();
    }

    // Run assertions within the schedule so SpatialQuery is properly populated
    app.add_systems(
        Update,
        move |spatial_query: SpatialQuery, map_ids: Query<&MapInstanceId>| {
            let origin = Vec3::new(0.0, 5.0, 0.0);
            let filter = SpatialQueryFilter::default();

            let hit_a = spatial_query.cast_ray_predicate(
                origin,
                Dir3::NEG_Y,
                10.0,
                false,
                &filter,
                &|hit_entity| match map_ids.get(hit_entity).ok() {
                    Some(id) => id.0 == map_a,
                    None => true,
                },
            );
            assert!(hit_a.is_some(), "Should hit map_a terrain");

            let hit_b = spatial_query.cast_ray_predicate(
                origin,
                Dir3::NEG_Y,
                10.0,
                false,
                &filter,
                &|hit_entity| match map_ids.get(hit_entity).ok() {
                    Some(id) => id.0 == map_b,
                    None => true,
                },
            );
            assert!(hit_b.is_some(), "Should hit map_b terrain");

            let map_c = Entity::from_bits(9999);
            let hit_c = spatial_query.cast_ray_predicate(
                origin,
                Dir3::NEG_Y,
                10.0,
                false,
                &filter,
                &|hit_entity| match map_ids.get(hit_entity).ok() {
                    Some(id) => id.0 == map_c,
                    None => true,
                },
            );
            assert!(
                hit_c.is_none(),
                "Should not hit terrain from nonexistent map"
            );
        },
    );

    app.update();
}
