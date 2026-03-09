use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use lightyear::prelude::{Room, RoomEvent, RoomPlugin, RoomTarget};
use protocol::map::{MapInstanceId, PendingTransition};
use server::map::RoomRegistry;
use voxel_map_engine::prelude::{VoxelMapInstance, WorldVoxel};

use std::sync::Arc;

fn dummy_generator() -> Arc<dyn Fn(IVec3) -> Vec<WorldVoxel> + Send + Sync> {
    Arc::new(|_| vec![WorldVoxel::Air; 1])
}

#[test]
fn room_registry_creates_separate_rooms_for_different_maps() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(RoomPlugin);
    app.init_resource::<RoomRegistry>();

    app.world_mut()
        .run_system_once(
            |mut registry: ResMut<RoomRegistry>, mut commands: Commands| {
                let ow = registry.get_or_create(&MapInstanceId::Overworld, &mut commands);
                let hb =
                    registry.get_or_create(&MapInstanceId::Homebase { owner: 42 }, &mut commands);
                assert_ne!(ow, hb, "Different maps should have different rooms");

                let ow2 = registry.get_or_create(&MapInstanceId::Overworld, &mut commands);
                assert_eq!(ow, ow2, "Same map should return same room");
            },
        )
        .unwrap();
}

#[test]
fn room_transfer_moves_entity_between_rooms() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(RoomPlugin);

    let room_a = app.world_mut().spawn(Room::default()).id();
    let room_b = app.world_mut().spawn(Room::default()).id();
    let entity = app.world_mut().spawn_empty().id();

    app.world_mut().trigger(RoomEvent {
        room: room_a,
        target: RoomTarget::AddEntity(entity),
    });
    app.update();

    assert!(
        app.world()
            .get::<Room>(room_a)
            .unwrap()
            .entities
            .contains(&entity),
        "Entity should be in room A initially"
    );

    // Same-frame transfer
    app.world_mut().trigger(RoomEvent {
        room: room_a,
        target: RoomTarget::RemoveEntity(entity),
    });
    app.world_mut().trigger(RoomEvent {
        room: room_b,
        target: RoomTarget::AddEntity(entity),
    });
    app.update();

    assert!(
        !app.world()
            .get::<Room>(room_a)
            .unwrap()
            .entities
            .contains(&entity),
        "Entity should leave old room"
    );
    assert!(
        app.world()
            .get::<Room>(room_b)
            .unwrap()
            .entities
            .contains(&entity),
        "Entity should be in new room"
    );
}

#[test]
fn pending_transition_marker_can_be_added_and_removed() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let entity = app
        .world_mut()
        .spawn(PendingTransition(MapInstanceId::Overworld))
        .id();
    app.update();
    assert!(app.world().get::<PendingTransition>(entity).is_some());

    app.world_mut()
        .entity_mut(entity)
        .remove::<PendingTransition>();
    app.update();
    assert!(app.world().get::<PendingTransition>(entity).is_none());
}

#[test]
fn different_homebase_owners_produce_different_seeds() {
    let bounds = IVec3::new(4, 4, 4);
    let (_, config_a, _) = VoxelMapInstance::homebase(111, bounds, dummy_generator());
    let (_, config_b, _) = VoxelMapInstance::homebase(222, bounds, dummy_generator());
    assert_ne!(
        config_a.seed, config_b.seed,
        "Different owners must produce different seeds"
    );
}
