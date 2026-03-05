use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::*;
use protocol::map::MapTransitionStart;
use protocol::CharacterMarker;
use ui::state::MapTransitionState;
use voxel_map_engine::prelude::{ChunkTarget, PendingChunks, VoxelMapInstance};

pub struct ClientMapTransitionPlugin;

impl Plugin for ClientMapTransitionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                receive_map_transition_start,
                check_map_transition_complete.run_if(in_state(MapTransitionState::Transitioning)),
            ),
        );
        app.add_systems(
            OnEnter(MapTransitionState::Transitioning),
            on_enter_transitioning,
        );
        app.add_systems(
            OnExit(MapTransitionState::Transitioning),
            on_exit_transitioning,
        );
    }
}

fn receive_map_transition_start(
    mut receiver: Query<&mut MessageReceiver<MapTransitionStart>>,
    mut next_state: ResMut<NextState<MapTransitionState>>,
) {
    for mut msg_receiver in receiver.iter_mut() {
        for _msg in msg_receiver.receive() {
            next_state.set(MapTransitionState::Transitioning);
        }
    }
}

fn on_enter_transitioning(
    mut commands: Commands,
    player: Query<Entity, (With<Predicted>, With<CharacterMarker>)>,
) {
    if let Ok(entity) = player.single() {
        commands
            .entity(entity)
            .insert((RigidBodyDisabled, DisableRollback));
    }
}

fn on_exit_transitioning(
    mut commands: Commands,
    player: Query<Entity, (With<Predicted>, With<CharacterMarker>)>,
) {
    if let Ok(entity) = player.single() {
        commands
            .entity(entity)
            .remove::<(RigidBodyDisabled, DisableRollback)>();
    }
}

fn check_map_transition_complete(
    maps: Query<(&VoxelMapInstance, &PendingChunks)>,
    player: Query<&ChunkTarget, (With<Predicted>, With<CharacterMarker>)>,
    mut next_state: ResMut<NextState<MapTransitionState>>,
) {
    let Ok(target) = player.single() else { return };
    let Ok((instance, pending)) = maps.get(target.map_entity) else {
        return;
    };
    if pending.tasks.is_empty() && instance.desired_chunks.is_subset(&instance.loaded_chunks) {
        next_state.set(MapTransitionState::Playing);
    }
}
