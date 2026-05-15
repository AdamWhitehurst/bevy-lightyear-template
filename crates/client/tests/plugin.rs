#[cfg(feature = "spawn-panel")]
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use client_lightyear::ClientNetworkPlugin;
use lightyear::prelude::client as lightyear_client;
use lightyear::prelude::*;
use lightyear_client::*;
use protocol::*;

#[cfg(feature = "spawn-panel")]
#[test]
fn world_object_placement_ui_sequences_and_pending_ack() {
    use bevy::prelude::Vec3;
    use dev::panels::spawn::{PendingWorldObjectPlacement, WorldObjectPlacementUi};
    use protocol::world_object::WorldObjectId;

    let mut ui = WorldObjectPlacementUi::default();
    assert_eq!(ui.next_sequence(), 0);
    assert_eq!(ui.next_sequence(), 1);

    ui.pending.push(PendingWorldObjectPlacement {
        sequence: 1,
        object_id: WorldObjectId("test:crate".to_string()),
        base_position: Vec3::new(1.0, 2.0, 3.0),
        accepted_final_position: None,
    });
    ui.pending[0].accepted_final_position = Some(Vec3::new(1.0, 3.5, 3.0));
    assert_eq!(
        ui.pending[0].accepted_final_position,
        Some(Vec3::new(1.0, 3.5, 3.0))
    );
}

#[cfg(feature = "spawn-panel")]
#[test]
fn world_object_selection_ui_sequences_and_pending_delete_ack() {
    use dev::panels::spawn::{NearbyWorldObject, PendingWorldObjectDelete, WorldObjectSelectionUi};
    use protocol::world_object::WorldObjectId;

    let mut ui = WorldObjectSelectionUi::default();
    let target = Entity::from_raw_u32(42).expect("test entity id should be valid");
    assert!(!ui.cursor_pick_armed);
    assert!(!ui.nearby_scan_requested);
    assert!(ui.nearby_objects.is_empty());
    ui.cursor_pick_armed = true;
    ui.nearby_scan_requested = true;
    ui.nearby_objects.push(NearbyWorldObject {
        entity: target,
        object_id: WorldObjectId("near".to_string()),
        distance: 2.0,
    });
    assert!(ui.cursor_pick_armed);
    assert!(ui.nearby_scan_requested);
    assert_eq!(ui.nearby_objects[0].entity, target);
    assert_eq!(ui.next_sequence(), 0);
    assert_eq!(ui.next_sequence(), 1);
    ui.pending_deletes.push(PendingWorldObjectDelete {
        sequence: 0,
        target,
        accepted: false,
    });
    ui.pending_deletes[0].accepted = true;
    assert!(ui.pending_deletes[0].accepted);
}

#[cfg(feature = "spawn-panel")]
#[test]
fn nearby_world_objects_in_radius_lists_replicated_objects_by_distance() {
    use ::client::map::nearby_world_objects_in_radius;
    use avian3d::prelude::Position;
    use protocol::world_object::WorldObjectId;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    let receiver = app.world_mut().spawn_empty().id();
    let _far = app
        .world_mut()
        .spawn((
            WorldObjectId("far".to_string()),
            Position(Vec3::new(5.0, 0.0, 0.0).into()),
            MapInstanceId::Overworld,
            Replicated { receiver },
        ))
        .id();
    let near = app
        .world_mut()
        .spawn((
            WorldObjectId("near".to_string()),
            Position(Vec3::new(2.0, 0.0, 0.0).into()),
            MapInstanceId::Overworld,
            Replicated { receiver },
        ))
        .id();

    let nearby = app
        .world_mut()
        .run_system_once(
            |query: Query<
                (Entity, &WorldObjectId, &Position, Option<&MapInstanceId>),
                (With<WorldObjectId>, With<Replicated>),
            >| {
                nearby_world_objects_in_radius(
                    Vec3::ZERO,
                    6.0,
                    &query,
                    Some(&MapInstanceId::Overworld),
                )
            },
        )
        .expect("selection system should run");
    assert_eq!(nearby.len(), 2);
    assert_eq!(nearby[0].entity, near);
    assert_eq!(nearby[0].object_id, WorldObjectId("near".to_string()));
    assert!(nearby[0].distance < nearby[1].distance);
}

#[cfg(feature = "spawn-panel")]
#[test]
fn placement_preview_entities_are_visual_only() {
    use ::client::map::{spawn_world_object_placement_preview, WorldObjectPlacementPreview};
    use ::client::world_object::DefaultVoxModelMaterial;
    use avian3d::prelude::{Collider, Position};
    use protocol::vox_model::{VoxModelAsset, VoxModelRegistry};
    use protocol::world_object::{WorldObjectDef, WorldObjectId};
    use std::collections::HashMap;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<Assets<VoxModelAsset>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.insert_resource(VoxModelRegistry {
        models: HashMap::new(),
    });
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    app.insert_resource(DefaultVoxModelMaterial(material));

    let def = WorldObjectDef { components: vec![] };
    let object_id = WorldObjectId("test:preview".to_string());
    let entity = app
        .world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  vox_registry: Res<VoxModelRegistry>,
                  vox_assets: Res<Assets<VoxModelAsset>>,
                  default_material: Res<DefaultVoxModelMaterial>| {
                spawn_world_object_placement_preview(
                    &mut commands,
                    None,
                    object_id.clone(),
                    Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)),
                    &def,
                    &vox_registry,
                    &vox_assets,
                    &default_material,
                )
            },
        )
        .expect("preview spawn system should run");
    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(entity_ref.contains::<WorldObjectPlacementPreview>());
    assert!(entity_ref.contains::<Transform>());
    assert!(!entity_ref.contains::<Collider>());
    assert!(!entity_ref.contains::<Position>());
    assert!(!entity_ref.contains::<MapInstanceId>());
    assert!(!entity_ref.contains::<Replicated>());
    assert!(!entity_ref.contains::<protocol::world_object::WorldObjectId>());
}

#[cfg(feature = "spawn-panel")]
#[test]
fn replicated_object_reconciles_matching_preview_only() {
    use ::client::map::{reconcile_placement_preview_on_replication, WorldObjectPlacementPreview};
    use avian3d::prelude::Position;
    use dev::panels::spawn::{PendingWorldObjectPlacement, SpawnPanelUi};
    use protocol::world_object::WorldObjectId;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    let matching_id = WorldObjectId("test:matching".to_string());
    let other_id = WorldObjectId("test:other".to_string());
    app.init_resource::<SpawnPanelUi>();
    {
        let mut ui = app.world_mut().resource_mut::<SpawnPanelUi>();
        ui.selected_object = Some(matching_id.clone());
        ui.placement.pending.push(PendingWorldObjectPlacement {
            sequence: 7,
            object_id: matching_id.clone(),
            base_position: Vec3::ZERO,
            accepted_final_position: Some(Vec3::new(1.0, 2.0, 3.0)),
        });
    }

    let matched_preview = app
        .world_mut()
        .spawn((
            WorldObjectPlacementPreview {
                sequence: Some(7),
                object_id: matching_id.clone(),
            },
            Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        ))
        .id();
    let other_preview = app
        .world_mut()
        .spawn((
            WorldObjectPlacementPreview {
                sequence: Some(8),
                object_id: other_id,
            },
            Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        ))
        .id();
    let receiver = app.world_mut().spawn_empty().id();
    app.world_mut().spawn((
        matching_id,
        Position(Vec3::new(1.0, 2.0, 3.0).into()),
        Replicated { receiver },
    ));

    app.world_mut()
        .run_system_once(reconcile_placement_preview_on_replication)
        .expect("reconciliation system should run");
    app.update();

    assert!(app.world().get_entity(matched_preview).is_err());
    assert!(app.world().get_entity(other_preview).is_ok());
    assert!(app
        .world()
        .resource::<SpawnPanelUi>()
        .placement
        .pending
        .is_empty());
}

#[cfg(feature = "spawn-panel")]
#[test]
fn edit_preview_transform_applies_placement_offset() {
    use ::client::map::preview_transform;
    use protocol::world_object::{PlacementOffset, WorldObjectDef};

    let def = WorldObjectDef {
        components: vec![Box::new(PlacementOffset(Vec3::new(0.0, 1.5, 0.0)))],
    };

    assert_eq!(
        preview_transform(&def, Vec3::new(2.0, 3.0, 4.0)).translation,
        Vec3::new(2.0, 4.5, 4.0)
    );
}

#[cfg(feature = "spawn-panel")]
#[test]
fn edit_preview_entities_are_visual_only() {
    use ::client::map::{spawn_world_object_edit_preview, WorldObjectEditPreview};
    use ::client::world_object::DefaultVoxModelMaterial;
    use avian3d::prelude::{Collider, Position};
    use protocol::vox_model::{VoxModelAsset, VoxModelRegistry};
    use protocol::world_object::{WorldObjectDef, WorldObjectId};
    use std::collections::HashMap;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<Assets<VoxModelAsset>>();
    app.init_resource::<Assets<StandardMaterial>>();
    app.insert_resource(VoxModelRegistry {
        models: HashMap::new(),
    });
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    app.insert_resource(DefaultVoxModelMaterial(material));

    let def = WorldObjectDef { components: vec![] };
    let object_id = WorldObjectId("test:edit-preview".to_string());
    let target = app.world_mut().spawn_empty().id();
    let entity = app
        .world_mut()
        .run_system_once(
            move |mut commands: Commands,
                  vox_registry: Res<VoxModelRegistry>,
                  vox_assets: Res<Assets<VoxModelAsset>>,
                  default_material: Res<DefaultVoxModelMaterial>| {
                spawn_world_object_edit_preview(
                    &mut commands,
                    Some(2),
                    target,
                    object_id.clone(),
                    Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)),
                    &def,
                    &vox_registry,
                    &vox_assets,
                    &default_material,
                )
            },
        )
        .expect("edit preview spawn system should run");
    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(entity_ref.contains::<WorldObjectEditPreview>());
    assert!(entity_ref.contains::<Transform>());
    assert!(!entity_ref.contains::<Collider>());
    assert!(!entity_ref.contains::<Position>());
    assert!(!entity_ref.contains::<MapInstanceId>());
    assert!(!entity_ref.contains::<Replicated>());
    assert!(!entity_ref.contains::<protocol::world_object::WorldObjectId>());
}

#[cfg(feature = "spawn-panel")]
#[test]
fn edit_preview_reconciles_when_replicated_rotation_matches_accepted_rotate() {
    use ::client::map::{reconcile_edit_preview_on_transform_replication, WorldObjectEditPreview};
    use avian3d::prelude::Rotation;
    use dev::panels::spawn::{PendingWorldObjectRotation, SpawnPanelUi};
    use protocol::world_object::WorldObjectId;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<SpawnPanelUi>();
    let object_id = WorldObjectId("test:rotated".to_string());
    let rotation = Quat::from_rotation_y(1.0);
    let receiver = app.world_mut().spawn_empty().id();
    let target = app
        .world_mut()
        .spawn((
            object_id.clone(),
            Rotation(rotation),
            Replicated { receiver },
        ))
        .id();
    app.world_mut()
        .resource_mut::<SpawnPanelUi>()
        .selection
        .pending_rotations
        .push(PendingWorldObjectRotation {
            sequence: 10,
            target,
            rotation,
            accepted: true,
        });
    let preview = app
        .world_mut()
        .spawn((
            WorldObjectEditPreview {
                sequence: Some(10),
                target,
                object_id,
            },
            Transform {
                rotation,
                ..default()
            },
        ))
        .id();

    app.world_mut()
        .run_system_once(reconcile_edit_preview_on_transform_replication)
        .expect("edit reconciliation system should run");
    app.update();

    assert!(app.world().get_entity(preview).is_err());
    assert!(app
        .world()
        .resource::<SpawnPanelUi>()
        .selection
        .pending_rotations
        .is_empty());
}

#[cfg(feature = "spawn-panel")]
#[test]
fn cursor_pick_prefers_nearest_object_along_ray() {
    use ::client::map::pick_world_object_collider_from_ray;
    use avian3d::prelude::{
        Collider, CollisionLayers, PhysicsPlugins, Position, RigidBody, Rotation, SpatialQuery,
    };
    use protocol::physics::MapCollisionHooks;
    use protocol::world_object::WorldObjectId;

    fn pick_system(
        query: Query<(), (With<WorldObjectId>, With<Replicated>)>,
        mut spatial_query: SpatialQuery,
    ) -> Option<Entity> {
        spatial_query.update_pipeline();
        pick_world_object_collider_from_ray(
            Ray3d::new(Vec3::new(0.0, 4.0, 0.0), Dir3::X),
            &query,
            &spatial_query,
        )
    }

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::asset::AssetPlugin::default());
    app.add_plugins(bevy::transform::TransformPlugin);
    app.add_plugins(bevy::mesh::MeshPlugin);
    app.add_plugins(PhysicsPlugins::default().with_collision_hooks::<MapCollisionHooks>());
    app.finish();
    let receiver = app.world_mut().spawn_empty().id();
    let near = app
        .world_mut()
        .spawn((
            WorldObjectId("test:near".to_string()),
            Replicated { receiver },
            RigidBody::Static,
            Collider::cuboid(1.0, 5.0, 1.0),
            Position(Vec3::new(5.0, 5.0, 0.0).into()),
            Rotation::default(),
            CollisionLayers::default(),
        ))
        .id();
    app.world_mut().spawn((
        WorldObjectId("test:far".to_string()),
        Replicated { receiver },
        RigidBody::Static,
        Collider::cuboid(1.0, 5.0, 1.0),
        Position(Vec3::new(9.0, 5.0, 0.0).into()),
        Rotation::default(),
        CollisionLayers::default(),
    ));
    app.update();

    let picked = app
        .world_mut()
        .run_system_once(pick_system)
        .expect("pick system should run");

    assert_eq!(picked, Some(near));
}

#[cfg(feature = "spawn-panel")]
#[test]
fn cleanup_stale_edit_preview_is_removed_when_target_despawns() {
    use ::client::map::{cleanup_stale_world_object_edit_previews, WorldObjectEditPreview};
    use dev::panels::spawn::SpawnPanelUi;
    use protocol::world_object::WorldObjectId;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<SpawnPanelUi>();
    let missing_target = Entity::from_raw_u32(99).expect("test entity id should be valid");
    let preview = app
        .world_mut()
        .spawn((
            WorldObjectEditPreview {
                sequence: Some(1),
                target: missing_target,
                object_id: WorldObjectId("test:stale".to_string()),
            },
            Transform::default(),
        ))
        .id();

    app.world_mut()
        .run_system_once(cleanup_stale_world_object_edit_previews)
        .expect("cleanup system should run");
    app.update();

    assert!(app.world().get_entity(preview).is_err());
}

#[cfg(feature = "spawn-panel")]
#[test]
fn edit_preview_reconciles_when_replicated_transform_matches_accepted_move() {
    use ::client::map::{reconcile_edit_preview_on_transform_replication, WorldObjectEditPreview};
    use avian3d::prelude::Position;
    use dev::panels::spawn::{PendingWorldObjectMove, SpawnPanelUi};
    use protocol::world_object::WorldObjectId;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<SpawnPanelUi>();
    let object_id = WorldObjectId("test:moved".to_string());
    let receiver = app.world_mut().spawn_empty().id();
    let target = app
        .world_mut()
        .spawn((
            object_id.clone(),
            Position(Vec3::new(1.0, 2.0, 3.0).into()),
            Replicated { receiver },
        ))
        .id();
    app.world_mut()
        .resource_mut::<SpawnPanelUi>()
        .selection
        .pending_moves
        .push(PendingWorldObjectMove {
            sequence: 9,
            target,
            final_position: Vec3::new(1.0, 2.0, 3.0),
            old_chunk_pos: None,
            new_chunk_pos: None,
            accepted: true,
        });
    let preview = app
        .world_mut()
        .spawn((
            WorldObjectEditPreview {
                sequence: Some(9),
                target,
                object_id,
            },
            Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        ))
        .id();

    app.world_mut()
        .run_system_once(reconcile_edit_preview_on_transform_replication)
        .expect("edit reconciliation system should run");
    app.update();

    assert!(app.world().get_entity(preview).is_err());
    assert!(app
        .world()
        .resource::<SpawnPanelUi>()
        .selection
        .pending_moves
        .is_empty());
}

#[test]
fn test_client_network_plugin_registers_observers() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(ClientNetworkPlugin::default());

    // Run startup to spawn client entity
    app.update();

    // Get the client entity
    let mut query = app.world_mut().query_filtered::<Entity, With<Client>>();
    let client_entity = query.single(app.world()).unwrap();

    // Manually trigger Connected event by inserting component (with required RemoteId)
    app.world_mut()
        .entity_mut(client_entity)
        .insert((Connected, RemoteId(PeerId::Netcode(0))));

    // Run update to trigger observers
    app.update();

    // Verify observer ran without panicking and Connected component persists
    let has_connected = app.world().entity(client_entity).contains::<Connected>();
    assert!(
        has_connected,
        "Observer should process Connected component without removing it"
    );
}

#[test]
fn test_client_network_plugin_disconnected_observer() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ClientPlugins::default());
    app.add_plugins(ProtocolPlugin);
    app.add_plugins(ClientNetworkPlugin::default());

    // Run startup to spawn client entity
    app.update();

    // Get the client entity
    let mut query = app.world_mut().query_filtered::<Entity, With<Client>>();
    let client_entity = query.single(app.world()).unwrap();

    // Manually trigger Disconnected event by inserting component
    app.world_mut()
        .entity_mut(client_entity)
        .insert(Disconnected::default());

    // Run update to trigger observers
    app.update();

    // Verify observer ran without panicking
    let has_disconnected = app.world().entity(client_entity).contains::<Disconnected>();
    assert!(
        has_disconnected,
        "Observer should process Disconnected component"
    );
}
