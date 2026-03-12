use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use lightyear::prelude::{ReplicationSender, Room, RoomEvent, RoomPlugin, RoomTarget};
use protocol::MapInstanceId;
use server::map::RoomRegistry;

fn room_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(RoomPlugin);
    app.init_resource::<RoomRegistry>();
    app
}

/// Replicates the server's `on_map_instance_id_added` observer (module-private).
fn add_map_observer(app: &mut App) {
    app.add_observer(
        |trigger: On<Add, MapInstanceId>,
         mut commands: Commands,
         map_ids: Query<&MapInstanceId>,
         mut room_registry: ResMut<RoomRegistry>| {
            let entity = trigger.entity;
            let map_id = map_ids.get(entity).unwrap();
            let room = room_registry.get_or_create(map_id, &mut commands);
            commands.trigger(RoomEvent {
                room,
                target: RoomTarget::AddEntity(entity),
            });
        },
    );
}

#[test]
fn registry_creates_room_on_first_access() {
    let mut app = room_test_app();

    app.world_mut()
        .run_system_once(
            |mut registry: ResMut<RoomRegistry>, mut commands: Commands| {
                let room = registry.get_or_create(&MapInstanceId::Overworld, &mut commands);
                assert_ne!(room, Entity::PLACEHOLDER);
            },
        )
        .unwrap();
    app.update();

    let registry = app.world().resource::<RoomRegistry>();
    let room_entity = registry.0[&MapInstanceId::Overworld];
    assert!(
        app.world().get::<Room>(room_entity).is_some(),
        "Room entity must have Room component"
    );
}

#[test]
fn registry_reuses_existing_room() {
    let mut app = room_test_app();

    app.world_mut()
        .run_system_once(
            |mut registry: ResMut<RoomRegistry>, mut commands: Commands| {
                let first = registry.get_or_create(&MapInstanceId::Overworld, &mut commands);
                let second = registry.get_or_create(&MapInstanceId::Overworld, &mut commands);
                assert_eq!(first, second, "Same MapInstanceId must return same entity");
            },
        )
        .unwrap();
}

#[test]
fn registry_creates_separate_rooms_per_map() {
    let mut app = room_test_app();

    app.world_mut()
        .run_system_once(
            |mut registry: ResMut<RoomRegistry>, mut commands: Commands| {
                let overworld = registry.get_or_create(&MapInstanceId::Overworld, &mut commands);
                let homebase =
                    registry.get_or_create(&MapInstanceId::Homebase { owner: 42 }, &mut commands);
                assert_ne!(
                    overworld, homebase,
                    "Different maps must produce different rooms"
                );
            },
        )
        .unwrap();
}

#[test]
fn registry_distinguishes_homebase_owners() {
    let mut app = room_test_app();

    app.world_mut()
        .run_system_once(
            |mut registry: ResMut<RoomRegistry>, mut commands: Commands| {
                let a =
                    registry.get_or_create(&MapInstanceId::Homebase { owner: 1 }, &mut commands);
                let b =
                    registry.get_or_create(&MapInstanceId::Homebase { owner: 2 }, &mut commands);
                assert_ne!(
                    a, b,
                    "Homebases with different owners must be separate rooms"
                );
            },
        )
        .unwrap();
}

#[test]
fn observer_adds_entity_to_room() {
    let mut app = room_test_app();
    add_map_observer(&mut app);

    let entity = app.world_mut().spawn(MapInstanceId::Overworld).id();
    app.update();

    let registry = app.world().resource::<RoomRegistry>();
    let room_entity = registry.0[&MapInstanceId::Overworld];
    let room = app.world().get::<Room>(room_entity).unwrap();
    assert!(
        room.entities.contains(&entity),
        "Room must contain entity after observer fires"
    );
}

#[test]
fn observer_routes_entities_to_correct_rooms() {
    let mut app = room_test_app();
    add_map_observer(&mut app);

    let overworld_ent = app.world_mut().spawn(MapInstanceId::Overworld).id();
    let homebase_ent = app
        .world_mut()
        .spawn(MapInstanceId::Homebase { owner: 99 })
        .id();
    app.update();

    let registry = app.world().resource::<RoomRegistry>();

    let ow_room = app
        .world()
        .get::<Room>(registry.0[&MapInstanceId::Overworld])
        .unwrap();
    let hb_room = app
        .world()
        .get::<Room>(registry.0[&MapInstanceId::Homebase { owner: 99 }])
        .unwrap();

    assert!(ow_room.entities.contains(&overworld_ent));
    assert!(!ow_room.entities.contains(&homebase_ent));
    assert!(hb_room.entities.contains(&homebase_ent));
    assert!(!hb_room.entities.contains(&overworld_ent));
}

#[test]
fn sender_added_to_room_via_event() {
    let mut app = room_test_app();

    let room_entity = app.world_mut().spawn(Room::default()).id();
    let sender = app.world_mut().spawn(ReplicationSender::default()).id();

    app.world_mut().trigger(RoomEvent {
        room: room_entity,
        target: RoomTarget::AddSender(sender),
    });
    app.update();

    let room = app.world().get::<Room>(room_entity).unwrap();
    assert!(room.clients.contains(&sender));
}

#[test]
fn entity_added_to_room_via_event() {
    let mut app = room_test_app();

    let room_entity = app.world_mut().spawn(Room::default()).id();
    let entity = app.world_mut().spawn_empty().id();

    app.world_mut().trigger(RoomEvent {
        room: room_entity,
        target: RoomTarget::AddEntity(entity),
    });
    app.update();

    let room = app.world().get::<Room>(room_entity).unwrap();
    assert!(room.entities.contains(&entity));
}

#[test]
fn same_frame_room_transfer_moves_entity() {
    let mut app = room_test_app();

    let room_a = app.world_mut().spawn(Room::default()).id();
    let room_b = app.world_mut().spawn(Room::default()).id();
    let entity = app.world_mut().spawn_empty().id();

    // Entity starts in room A.
    app.world_mut().trigger(RoomEvent {
        room: room_a,
        target: RoomTarget::AddEntity(entity),
    });
    app.update();

    // Same-frame transfer: remove from A, add to B.
    app.world_mut().trigger(RoomEvent {
        room: room_a,
        target: RoomTarget::RemoveEntity(entity),
    });
    app.world_mut().trigger(RoomEvent {
        room: room_b,
        target: RoomTarget::AddEntity(entity),
    });
    app.update();

    let room_a_data = app.world().get::<Room>(room_a).unwrap();
    let room_b_data = app.world().get::<Room>(room_b).unwrap();
    assert!(
        !room_a_data.entities.contains(&entity),
        "Entity must not remain in room A"
    );
    assert!(
        room_b_data.entities.contains(&entity),
        "Entity must be in room B after transfer"
    );
}

#[test]
fn entity_not_in_unrelated_room() {
    let mut app = room_test_app();
    add_map_observer(&mut app);

    let sender = app.world_mut().spawn(ReplicationSender::default()).id();

    // Pre-create homebase room and add sender there.
    app.world_mut()
        .run_system_once(
            move |mut registry: ResMut<RoomRegistry>, mut commands: Commands| {
                let hb_room =
                    registry.get_or_create(&MapInstanceId::Homebase { owner: 1 }, &mut commands);
                commands.trigger(RoomEvent {
                    room: hb_room,
                    target: RoomTarget::AddSender(sender),
                });
            },
        )
        .unwrap();
    app.update();

    // Spawn entity in overworld (different room from sender).
    let entity = app.world_mut().spawn(MapInstanceId::Overworld).id();
    app.update();

    let registry = app.world().resource::<RoomRegistry>();
    let hb_room = app
        .world()
        .get::<Room>(registry.0[&MapInstanceId::Homebase { owner: 1 }])
        .unwrap();

    assert!(
        !hb_room.entities.contains(&entity),
        "Entity in overworld must not appear in homebase room"
    );
}
