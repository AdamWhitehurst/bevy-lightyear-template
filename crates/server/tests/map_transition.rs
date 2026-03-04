use avian3d::prelude::*;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use lightyear::prelude::DisableRollback;
use protocol::map::MapInstanceId;
use server::map_transition::{
    find_or_spawn_homebase, initiate_map_transition, tick_map_transition_timers, MapTransitionTimer,
};
use voxel_map_engine::prelude::{ChunkTarget, Homebase};

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app
}

#[test]
fn initiate_transition_inserts_components() {
    let mut app = test_app();
    let target_map = app.world_mut().spawn_empty().id();
    let player = app
        .world_mut()
        .spawn((
            Position(Vec3::new(100.0, 100.0, 100.0)),
            LinearVelocity(Vec3::new(5.0, 0.0, 0.0)),
        ))
        .id();

    let mut commands = app.world_mut().commands();
    initiate_map_transition(&mut commands, player, target_map);
    app.update();

    let world = app.world();
    assert!(
        world.get::<RigidBodyDisabled>(player).is_some(),
        "Should have RigidBodyDisabled"
    );
    assert!(
        world.get::<DisableRollback>(player).is_some(),
        "Should have DisableRollback"
    );
    assert_eq!(world.get::<MapInstanceId>(player).unwrap().0, target_map);
    assert_eq!(
        world.get::<ChunkTarget>(player).unwrap().map_entity,
        target_map
    );
    assert_eq!(
        world.get::<Position>(player).unwrap().0,
        Vec3::new(0.0, 30.0, 0.0)
    );
    assert_eq!(world.get::<LinearVelocity>(player).unwrap().0, Vec3::ZERO);
    assert!(
        world.get::<MapTransitionTimer>(player).is_some(),
        "Should have MapTransitionTimer"
    );
}

#[test]
fn find_existing_homebase() {
    let mut app = test_app();
    let player = app.world_mut().spawn_empty().id();
    let existing_map = app.world_mut().spawn(Homebase { owner: player }).id();

    app.world_mut()
        .run_system_once(
            move |mut commands: Commands, homebases: Query<(Entity, &Homebase)>| {
                let result = find_or_spawn_homebase(&mut commands, player, &homebases);
                assert_eq!(result, existing_map);
            },
        )
        .unwrap();
}

#[test]
fn spawn_new_homebase() {
    let mut app = test_app();
    let player = app.world_mut().spawn_empty().id();

    app.world_mut()
        .run_system_once(
            move |mut commands: Commands, homebases: Query<(Entity, &Homebase)>| {
                let result = find_or_spawn_homebase(&mut commands, player, &homebases);
                assert_ne!(result, Entity::PLACEHOLDER);
            },
        )
        .unwrap();
    app.update();

    let mut query = app.world_mut().query::<&Homebase>();
    let hb = query
        .iter(app.world())
        .next()
        .expect("Homebase should be spawned");
    assert_eq!(hb.owner, player);
}

#[test]
fn transition_timer_removes_disabled() {
    let mut app = test_app();
    app.add_systems(Update, tick_map_transition_timers);

    let entity = app
        .world_mut()
        .spawn((
            RigidBodyDisabled,
            DisableRollback,
            MapTransitionTimer(Timer::from_seconds(0.1, TimerMode::Once)),
        ))
        .id();

    // Use ManualInstant to control time deterministically
    let mut current_time = bevy::platform::time::Instant::now();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualInstant(current_time));
    app.update(); // first update to initialize time

    current_time += std::time::Duration::from_millis(200);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualInstant(current_time));
    app.update();

    assert!(
        app.world().get::<RigidBodyDisabled>(entity).is_none(),
        "RigidBodyDisabled should be removed after timer expires"
    );
    assert!(
        app.world().get::<DisableRollback>(entity).is_none(),
        "DisableRollback should be removed after timer expires"
    );
    assert!(
        app.world().get::<MapTransitionTimer>(entity).is_none(),
        "MapTransitionTimer should be removed after timer expires"
    );
}
