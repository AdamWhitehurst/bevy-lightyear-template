use avian3d::prelude::Collider;
use bevy::prelude::*;
use lightyear::prelude::*;
use protocol::map::*;
use protocol::transition::*;
use protocol::MapRegistry;
use ui::state::MapTransitionState;
use voxel_map_engine::lifecycle::ChunkWorkTracker;
use voxel_map_engine::prelude::*;

/// Receives MapTransitionEnd, sets end_received flag.
pub fn receive_transition_end(
    mut receivers: Query<&mut MessageReceiver<MapTransitionEnd>>,
    mut state: ResMut<ClientTransitionState>,
) {
    for mut receiver in &mut receivers {
        for _end in receiver.receive() {
            trace!("Received MapTransitionEnd");
            state.end_received = true;
        }
    }
}

/// Receives MapTransitionEntity, accumulates server entity IDs.
pub fn receive_transition_entities(
    mut receivers: Query<&mut MessageReceiver<MapTransitionEntity>>,
    mut state: ResMut<ClientTransitionState>,
) {
    for mut receiver in &mut receivers {
        for msg in receiver.receive() {
            trace!("Received MapTransitionEntity entity={:?}", msg.entity);
            state.pending_entities.push(msg.entity);
        }
    }
}

/// Main state machine driver. Evaluates phase gates, advances phase.
pub fn update_transition_state(
    mut state: ResMut<ClientTransitionState>,
    registry: Res<MapRegistry>,
    map_query: Query<(&VoxelMapInstance, &ChunkWorkTracker, &Children)>,
    chunk_query: Query<(&VoxelChunk, Has<Collider>), With<Mesh3d>>,
    mut ready_senders: Query<&mut MessageSender<MapTransitionReady>>,
    manager_query: Query<&MessageManager, With<Client>>,
    entity_exists: Query<Entity>,
    mut next_transition: ResMut<NextState<MapTransitionState>>,
) {
    match state.phase {
        TransitionPhase::Idle => return,

        TransitionPhase::Cleanup => {
            let Some(target) = &state.target_map else {
                debug_assert!(false, "Cleanup phase with no target_map");
                return;
            };
            let Some(&map_entity) = registry.0.get(target) else {
                return; // Map not yet spawned
            };
            if map_query.get(map_entity).is_err() {
                return; // VoxelMapInstance not yet on entity
            }
            trace!("Transition: Cleanup -> Loading");
            state.phase = TransitionPhase::Loading;
        }

        TransitionPhase::Loading => {
            if !check_spatial_readiness(&state, &registry, &map_query, &chunk_query) {
                return;
            }
            for mut sender in &mut ready_senders {
                sender.send::<MapChannel>(MapTransitionReady);
            }
            trace!("Transition: Loading -> Ready (sent MapTransitionReady)");
            state.phase = TransitionPhase::Ready;
        }

        TransitionPhase::Ready => {
            if !state.end_received {
                return;
            }
            if !state.pending_entities.is_empty()
                && !check_entities_resolved(&state, &manager_query, &entity_exists)
            {
                return;
            }
            trace!("Transition: Ready -> Complete");
            state.phase = TransitionPhase::Complete;
        }

        TransitionPhase::Complete => {
            next_transition.set(MapTransitionState::Playing);
            state.reset();
            trace!("Transition: Complete -> Idle");
        }
    }
}

/// Check that all columns within Chebyshev radius of spawn have loaded data,
/// no pending remesh work, and meshed chunks have colliders.
fn check_spatial_readiness(
    state: &ClientTransitionState,
    registry: &MapRegistry,
    map_query: &Query<(&VoxelMapInstance, &ChunkWorkTracker, &Children)>,
    chunk_query: &Query<(&VoxelChunk, Has<Collider>), With<Mesh3d>>,
) -> bool {
    let Some(target) = state.target_map.as_ref() else {
        return false;
    };
    let Some(&map_entity) = registry.0.get(target) else {
        return false;
    };
    let Ok((instance, tracker, children)) = map_query.get(map_entity) else {
        return false;
    };

    let radius = state.readiness_radius as i32;
    let (y_min, y_max) = state.column_y_range;

    for dx in -radius..=radius {
        for dz in -radius..=radius {
            let col = IVec2::new(state.spawn_column.x + dx, state.spawn_column.y + dz);

            if !instance.chunk_levels.contains_key(&col) {
                return false;
            }

            for y in y_min..=y_max {
                let pos = IVec3::new(col.x, y, col.y);
                if instance.chunks_needing_remesh.contains(&pos) || tracker.remeshing.contains(&pos)
                {
                    return false;
                }
            }
        }
    }

    // Verify VoxelChunk children within radius have Collider.
    // Uses VoxelChunk.position (chunk-space IVec3) directly.
    for child in children.iter() {
        if let Ok((chunk, has_collider)) = chunk_query.get(child) {
            let chunk_col = IVec2::new(chunk.position.x, chunk.position.z);
            let dx = (chunk_col.x - state.spawn_column.x).abs();
            let dz = (chunk_col.y - state.spawn_column.y).abs();
            if dx <= radius && dz <= radius && !has_collider {
                return false;
            }
        }
    }

    true
}

/// Check that all pending server entity IDs have been mapped to local entities.
fn check_entities_resolved(
    state: &ClientTransitionState,
    manager_query: &Query<&MessageManager, With<Client>>,
    entity_exists: &Query<Entity>,
) -> bool {
    let Ok(manager) = manager_query.single() else {
        return false;
    };
    state.pending_entities.iter().all(|&remote| {
        manager
            .entity_mapper
            .get_local(remote)
            .is_some_and(|local| entity_exists.get(local).is_ok())
    })
}

pub struct ClientTransitionPlugin;

impl Plugin for ClientTransitionPlugin {
    fn build(&self, app: &mut App) {
        use ui::state::ClientState;
        app.add_systems(
            Update,
            (
                receive_transition_end,
                receive_transition_entities,
                update_transition_state,
            )
                .chain()
                .run_if(in_state(ClientState::InGame)),
        );
    }
}
