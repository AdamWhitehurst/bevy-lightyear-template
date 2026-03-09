use std::sync::Arc;

use avian3d::prelude::{ColliderDisabled, RigidBodyDisabled};
use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use lightyear::prelude::{Controlled, DisableRollback, Predicted};
use protocol::map::TransitionReadySent;
use protocol::PendingTransition;
use protocol::{CharacterMarker, MapInstanceId, MapRegistry};
use ui::{ClientState, MapTransitionState};
use voxel_map_engine::prelude::{
    flat_terrain_voxels, ChunkTarget, PendingChunks, VoxelMapConfig, VoxelMapInstance,
};

fn transition_test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(StatesPlugin);
    app.init_resource::<MapRegistry>();
    app.insert_state(ClientState::InGame);
    app.add_sub_state::<MapTransitionState>();
    app.add_systems(
        Update,
        client::map::check_transition_chunks_loaded
            .run_if(in_state(MapTransitionState::Transitioning)),
    );
    app
}

fn spawn_map(app: &mut App) -> Entity {
    let map = app
        .world_mut()
        .spawn((
            VoxelMapInstance::new(3),
            VoxelMapConfig::new(0, 1, None, 3, Arc::new(flat_terrain_voxels)),
            Transform::default(),
            MapInstanceId::Overworld,
            PendingChunks::default(),
        ))
        .id();
    app.world_mut()
        .resource_mut::<MapRegistry>()
        .insert(MapInstanceId::Overworld, map);
    map
}

fn spawn_frozen_player(app: &mut App, map: Entity) -> Entity {
    app.world_mut()
        .spawn((
            CharacterMarker,
            Predicted,
            Controlled,
            RigidBodyDisabled,
            ColliderDisabled,
            DisableRollback,
            ChunkTarget::new(map, 0),
            Transform::default(),
        ))
        .id()
}

fn set_transitioning(app: &mut App, player: Entity) {
    app.world_mut()
        .entity_mut(player)
        .insert(PendingTransition(MapInstanceId::Overworld));
    app.world_mut()
        .resource_mut::<NextState<MapTransitionState>>()
        .set(MapTransitionState::Transitioning);
}

#[test]
fn stays_transitioning_while_chunks_loading() {
    let mut app = transition_test_app();

    let map = spawn_map(&mut app);
    let player = spawn_frozen_player(&mut app, map);
    set_transitioning(&mut app, player);

    // loaded_chunks is empty — condition not met
    for _ in 0..5 {
        app.update();
    }

    assert_eq!(
        *app.world().resource::<State<MapTransitionState>>().get(),
        MapTransitionState::Transitioning,
        "Should remain Transitioning while loaded_chunks is empty"
    );
    assert!(
        app.world().get::<TransitionReadySent>(player).is_none(),
        "TransitionReadySent should not be inserted while chunks are still loading"
    );
}

#[test]
fn inserts_ready_sent_after_chunks_load() {
    let mut app = transition_test_app();

    let map = spawn_map(&mut app);
    let player = spawn_frozen_player(&mut app, map);

    // Manually simulate loaded: insert a chunk coord, leave PendingChunks empty
    app.world_mut()
        .entity_mut(map)
        .get_mut::<VoxelMapInstance>()
        .unwrap()
        .loaded_chunks
        .insert(IVec3::ZERO);

    set_transitioning(&mut app, player);

    for _ in 0..3 {
        app.update();
    }

    // With the confirmation protocol, check_transition_chunks_loaded sends
    // MapTransitionReady and inserts TransitionReadySent, but does NOT
    // transition to Playing (that requires MapTransitionEnd from server).
    assert_eq!(
        *app.world().resource::<State<MapTransitionState>>().get(),
        MapTransitionState::Transitioning,
        "Should stay Transitioning until MapTransitionEnd arrives from server"
    );
    assert!(
        app.world().get::<TransitionReadySent>(player).is_some(),
        "TransitionReadySent should be inserted after chunks load"
    );
    assert!(
        app.world().get::<PendingTransition>(player).is_some(),
        "PendingTransition should remain until MapTransitionEnd"
    );
}
