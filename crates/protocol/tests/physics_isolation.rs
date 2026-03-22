use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use protocol::map::{attach_chunk_colliders, MapInstanceId, MapRegistry, VoxelChunk};
use protocol::physics::MapCollisionHooks;
use voxel_map_engine::prelude::{
    flat_terrain_voxels, ChunkTicket, TicketType, VoxelGenerator, VoxelMapConfig, VoxelMapInstance,
    VoxelPlugin,
};

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::asset::AssetPlugin::default());
    app.add_plugins(bevy::diagnostic::DiagnosticsPlugin);
    app.add_plugins(bevy::transform::TransformPlugin);
    app.add_plugins(bevy::mesh::MeshPlugin);
    app.add_plugins(PhysicsPlugins::default().with_collision_hooks::<MapCollisionHooks>());
    app.init_resource::<MapRegistry>();
    app.finish();
    app
}

fn spawn_body_bundle(pos: Vec3) -> impl Bundle {
    (
        RigidBody::Dynamic,
        Collider::sphere(1.0),
        Position(pos),
        GravityScale(0.0),
        CollidingEntities::default(),
    )
}

fn run_physics(app: &mut App) {
    for _ in 0..200 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        app.update();
    }
}

fn are_colliding(app: &App, a: Entity, b: Entity) -> bool {
    let world = app.world();
    let a_colliding = world.get::<CollidingEntities>(a).unwrap();
    let b_colliding = world.get::<CollidingEntities>(b).unwrap();
    a_colliding.contains(&b) || b_colliding.contains(&a)
}

/// Sanity check: two overlapping bodies with no MapInstanceId should collide.
#[test]
fn baseline_collision_works() {
    let mut app = test_app();
    let a = app.world_mut().spawn(spawn_body_bundle(Vec3::ZERO)).id();
    let b = app
        .world_mut()
        .spawn(spawn_body_bundle(Vec3::new(0.5, 0.0, 0.0)))
        .id();

    run_physics(&mut app);

    assert!(
        are_colliding(&app, a, b),
        "Baseline: two overlapping bodies should collide. a={:?}, b={:?}",
        app.world().get::<CollidingEntities>(a).unwrap(),
        app.world().get::<CollidingEntities>(b).unwrap(),
    );
}

#[test]
fn same_map_entities_collide() {
    let mut app = test_app();
    let a = app
        .world_mut()
        .spawn((spawn_body_bundle(Vec3::ZERO), MapInstanceId::Overworld))
        .id();
    let b = app
        .world_mut()
        .spawn((
            spawn_body_bundle(Vec3::new(0.5, 0.0, 0.0)),
            MapInstanceId::Overworld,
        ))
        .id();

    run_physics(&mut app);

    assert!(
        are_colliding(&app, a, b),
        "Entities on the same map instance should collide"
    );
}

#[test]
fn different_map_entities_do_not_collide() {
    let mut app = test_app();
    let a = app
        .world_mut()
        .spawn((spawn_body_bundle(Vec3::ZERO), MapInstanceId::Overworld))
        .id();
    let b = app
        .world_mut()
        .spawn((
            spawn_body_bundle(Vec3::new(0.5, 0.0, 0.0)),
            MapInstanceId::Homebase { owner: 0 },
        ))
        .id();

    run_physics(&mut app);

    assert!(
        !are_colliding(&app, a, b),
        "Entities on different map instances should not collide"
    );
}

#[test]
#[should_panic(expected = "Entity missing MapInstanceId")]
fn entity_without_map_id_panics_in_filter_pairs() {
    let mut app = test_app();
    app.world_mut()
        .spawn((spawn_body_bundle(Vec3::ZERO), MapInstanceId::Overworld));
    // b has no MapInstanceId — filter_pairs should panic to catch the bug
    app.world_mut()
        .spawn(spawn_body_bundle(Vec3::new(0.5, 0.0, 0.0)));

    run_physics(&mut app);
}

#[test]
fn chunk_target_derived_from_map_registry() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::transform::TransformPlugin);
    app.init_resource::<Assets<Mesh>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.add_plugins(VoxelPlugin);
    app.insert_resource(voxel_map_engine::ChunkGenerationEnabled);
    app.init_resource::<MapRegistry>();

    let generator = VoxelGenerator(Arc::new(flat_terrain_voxels));
    let map_a = app
        .world_mut()
        .spawn((
            VoxelMapInstance::new(5),
            VoxelMapConfig::new(0, 0, 1, None, 5),
            generator.clone(),
            Transform::default(),
        ))
        .id();
    let map_b = app
        .world_mut()
        .spawn((
            VoxelMapInstance::new(5),
            VoxelMapConfig::new(1, 0, 1, None, 5),
            generator,
            Transform::default(),
        ))
        .id();

    {
        let mut registry = app.world_mut().resource_mut::<MapRegistry>();
        registry.insert(MapInstanceId::Overworld, map_a);
        registry.insert(MapInstanceId::Homebase { owner: 12345 }, map_b);
    }

    let target_map = app
        .world()
        .resource::<MapRegistry>()
        .get(&MapInstanceId::Overworld);
    let target = app
        .world_mut()
        .spawn((
            ChunkTicket::new(target_map, TicketType::Player, 0),
            Transform::from_translation(Vec3::ZERO),
        ))
        .id();

    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        app.update();
    }

    let count_a = app
        .world()
        .get::<VoxelMapInstance>(map_a)
        .unwrap()
        .chunk_levels
        .len();
    let count_b = app
        .world()
        .get::<VoxelMapInstance>(map_b)
        .unwrap()
        .chunk_levels
        .len();
    assert_eq!(count_a, 1, "map_a (Overworld) should have 1 loaded chunk");
    assert_eq!(count_b, 0, "map_b (Homebase) should have 0 loaded chunks");

    // Switch to Homebase via registry lookup
    let new_map = app
        .world()
        .resource::<MapRegistry>()
        .get(&MapInstanceId::Homebase { owner: 12345 });
    app.world_mut()
        .entity_mut(target)
        .insert(ChunkTicket::new(new_map, TicketType::Player, 0));

    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        app.update();
    }

    let count_a = app
        .world()
        .get::<VoxelMapInstance>(map_a)
        .unwrap()
        .chunk_levels
        .len();
    let count_b = app
        .world()
        .get::<VoxelMapInstance>(map_b)
        .unwrap()
        .chunk_levels
        .len();
    assert_eq!(count_a, 0, "old map should unload after switching");
    assert_eq!(count_b, 1, "new map should load after switching");
}

#[test]
fn chunk_colliders_inherit_map_instance_id() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::asset::AssetPlugin::default());
    app.add_plugins(bevy::diagnostic::DiagnosticsPlugin);
    app.add_plugins(bevy::transform::TransformPlugin);
    app.add_plugins(bevy::mesh::MeshPlugin);
    app.init_resource::<Assets<StandardMaterial>>();
    app.add_plugins(VoxelPlugin);
    app.insert_resource(voxel_map_engine::ChunkGenerationEnabled);
    app.add_plugins(PhysicsPlugins::default().with_collision_hooks::<MapCollisionHooks>());
    app.add_systems(Update, attach_chunk_colliders);
    app.init_resource::<MapRegistry>();
    app.finish();

    let map = app
        .world_mut()
        .spawn((
            VoxelMapInstance::new(5),
            VoxelMapConfig::new(0, 0, 1, None, 5),
            VoxelGenerator(Arc::new(flat_terrain_voxels)),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();

    app.world_mut().spawn((
        ChunkTicket::new(map, TicketType::Player, 0),
        Transform::default(),
    ));

    // Tick until chunks load and colliders attach
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(1));
        app.update();
    }

    // All VoxelChunk children of the map with a Collider should have MapInstanceId::Overworld
    let mut chunks_with_map_id = 0;
    let mut query = app
        .world_mut()
        .query_filtered::<(Entity, &ChildOf), (With<VoxelChunk>, With<Collider>)>();
    for (entity, child_of) in query.iter(app.world()) {
        if child_of.parent() == map {
            let map_id = app
                .world()
                .get::<MapInstanceId>(entity)
                .expect("Chunk collider entity should inherit MapInstanceId from parent map");
            assert_eq!(*map_id, MapInstanceId::Overworld);
            chunks_with_map_id += 1;
        }
    }
    assert!(
        chunks_with_map_id > 0,
        "Should have at least one chunk with inherited MapInstanceId"
    );
}
