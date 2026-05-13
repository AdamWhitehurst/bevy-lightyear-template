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
