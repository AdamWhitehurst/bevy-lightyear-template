use bevy::{prelude::*, window::PrimaryWindow};
use leafwing_input_manager::prelude::*;
use lightyear::prelude::{Controlled, MessageReceiver, MessageSender, Predicted};
use protocol::{
    CharacterMarker, ChunkDataSync, MapInstanceId, MapRegistry, PlayerActions, SectionBlocksUpdate,
    UnloadColumn, VoxelChannel, VoxelEditAck, VoxelEditBroadcast, VoxelEditReject,
    VoxelEditRequest, VoxelType,
};
use voxel_map_engine::prelude::{
    chunk_to_column, column_to_chunks, ChunkData, ChunkStatus, ChunkTicket, MapDimensions,
    VoxelMapInstance, VoxelPlugin, VoxelWorld, WorldVoxel,
};

const RAYCAST_MAX_DISTANCE: f32 = 100.0;

/// Buffers ChunkDataSync messages that arrive before the client player is ready.
/// Lightyear clears MessageReceiver each frame in Last, so we must drain and
/// Tracks pending predictions for block edits awaiting server acknowledgment.
#[derive(Resource, Default)]
pub struct VoxelPredictionState {
    pub next_sequence: u32,
    pub pending: Vec<VoxelPrediction>,
}

/// A single pending block edit prediction awaiting server acknowledgment.
pub struct VoxelPrediction {
    pub sequence: u32,
    pub position: IVec3,
    pub old_voxel: VoxelType,
    pub new_voxel: VoxelType,
}

impl VoxelPredictionState {
    /// Returns the next sequence number, incrementing the counter.
    pub fn next(&mut self) -> u32 {
        let seq = self.next_sequence;
        self.next_sequence += 1;
        seq
    }
}

/// Plugin managing client-side voxel map functionality.
pub struct ClientMapPlugin;

impl Plugin for ClientMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelPlugin)
            .init_resource::<MapRegistry>()
            .init_resource::<VoxelPredictionState>()
            .add_systems(
                Update,
                (
                    attach_chunk_ticket_to_player,
                    handle_voxel_broadcasts,
                    handle_section_blocks_update,
                    handle_voxel_edit_ack,
                    handle_voxel_edit_reject,
                    handle_chunk_data_sync,
                    handle_unload_column,
                    protocol::attach_chunk_colliders,
                )
                    .run_if(in_state(ui::ClientState::InGame)),
            )
            .add_systems(
                PostUpdate,
                handle_voxel_input
                    .run_if(in_state(ui::ClientState::InGame))
                    .after(TransformSystems::Propagate),
            );
    }
}

fn attach_chunk_ticket_to_player(
    mut commands: Commands,
    registry: Res<MapRegistry>,
    players: Query<
        (Entity, &MapInstanceId),
        (With<Predicted>, With<CharacterMarker>, Without<ChunkTicket>),
    >,
) {
    for (entity, map_id) in &players {
        let Some(&map_entity) = registry.0.get(map_id) else {
            trace!("attach_chunk_ticket_to_player: map {map_id:?} not yet registered, expected during transition");
            continue;
        };
        trace!("Attaching ChunkTicket to player {entity:?} on map {map_id:?}");
        commands
            .entity(entity)
            .insert(ChunkTicket::player(map_entity));
    }
}

/// Receives chunk data from server and queues async meshing via the remesh pipeline.
fn handle_chunk_data_sync(
    mut receivers: Query<&mut MessageReceiver<ChunkDataSync>>,
    mut map_query: Query<&mut VoxelMapInstance>,
    registry: Res<MapRegistry>,
) {
    let mut incoming: Vec<ChunkDataSync> = Vec::new();
    for mut receiver in &mut receivers {
        incoming.extend(receiver.receive());
    }

    if incoming.is_empty() {
        return;
    }

    for sync in incoming {
        let Some(&map_entity) = registry.0.get(&sync.map_id) else {
            continue;
        };
        let Ok(mut instance) = map_query.get_mut(map_entity) else {
            continue;
        };

        if sync.chunk_size != instance.chunk_size {
            error!(
                "ChunkDataSync chunk_size mismatch for {:?}: server={}, client={}",
                sync.map_id, sync.chunk_size, instance.chunk_size
            );
            continue;
        }

        let chunk_data = ChunkData::from_voxels(&sync.data.to_voxels(), ChunkStatus::Full);

        instance.insert_chunk_data(sync.chunk_pos, chunk_data);
        instance
            .chunk_levels
            .entry(chunk_to_column(sync.chunk_pos))
            .or_insert(0);
        instance.chunks_needing_remesh.insert(sync.chunk_pos);
    }
}

/// Handle server's UnloadColumn message — remove chunk data for all chunks in the column.
/// Mesh entity cleanup is handled by the existing `despawn_out_of_range_chunks` system
/// which checks `chunk_levels.contains_key()`.
fn handle_unload_column(
    mut receivers: Query<&mut MessageReceiver<UnloadColumn>>,
    registry: Res<MapRegistry>,
    mut map_query: Query<(&mut VoxelMapInstance, &MapDimensions)>,
) {
    for mut receiver in &mut receivers {
        for unload in receiver.receive() {
            let Some(&map_entity) = registry.0.get(&unload.map_id) else {
                continue;
            };
            let Ok((mut instance, dimensions)) = map_query.get_mut(map_entity) else {
                continue;
            };
            let col = unload.column;
            for chunk_pos in column_to_chunks(col, dimensions.column_y_range) {
                instance.remove_chunk_data(chunk_pos);
            }
            instance.chunk_levels.remove(&col);
        }
    }
}

/// Applies voxel edit broadcasts from the server, skipping positions with pending predictions.
fn handle_voxel_broadcasts(
    mut receiver: Query<&mut MessageReceiver<VoxelEditBroadcast>>,
    player_query: Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    mut voxel_world: VoxelWorld,
    prediction_state: Res<VoxelPredictionState>,
) {
    let Ok(chunk_ticket) = player_query.single() else {
        trace!("handle_voxel_broadcasts: no predicted player with ChunkTicket");
        return;
    };
    for mut message_receiver in receiver.iter_mut() {
        for broadcast in message_receiver.receive() {
            let has_pending_prediction = prediction_state
                .pending
                .iter()
                .any(|p| p.position == broadcast.position);
            if has_pending_prediction {
                trace!(
                    "handle_voxel_broadcasts: skipping broadcast at {:?} (pending prediction)",
                    broadcast.position
                );
                continue;
            }

            trace!(
                "handle_voxel_broadcasts: applying broadcast at {:?} voxel={:?}",
                broadcast.position,
                broadcast.voxel
            );
            voxel_world.set_voxel(
                chunk_ticket.map_entity,
                broadcast.position,
                WorldVoxel::from(broadcast.voxel),
            );
        }
    }
}

/// Handles batched block updates from server.
fn handle_section_blocks_update(
    mut receivers: Query<&mut MessageReceiver<SectionBlocksUpdate>>,
    player_query: Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    mut voxel_world: VoxelWorld,
    prediction_state: Res<VoxelPredictionState>,
) {
    let Ok(chunk_ticket) = player_query.single() else {
        trace!("handle_section_blocks_update: no predicted player with ChunkTicket");
        return;
    };
    for mut receiver in receivers.iter_mut() {
        for update in receiver.receive() {
            for (pos, voxel) in &update.changes {
                let has_pending_prediction =
                    prediction_state.pending.iter().any(|p| p.position == *pos);
                if has_pending_prediction {
                    trace!(
                        "handle_section_blocks_update: skipping change at {:?} (pending prediction)",
                        pos
                    );
                    continue;
                }
                voxel_world.set_voxel(chunk_ticket.map_entity, *pos, WorldVoxel::from(*voxel));
            }
        }
    }
}

fn handle_voxel_input(
    player_query: Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    mut voxel_world: VoxelWorld,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    action_query: Query<&ActionState<PlayerActions>, With<Controlled>>,
    mut message_sender: Query<&mut MessageSender<VoxelEditRequest>>,
    mut prediction_state: ResMut<VoxelPredictionState>,
) {
    let Ok(chunk_ticket) = player_query.single() else {
        trace!("handle_voxel_input: no predicted player with ChunkTicket");
        return;
    };
    let Ok(action_state) = action_query.single() else {
        trace!("handle_voxel_input: no entity with ActionState + Controlled");
        return;
    };

    let removing = action_state.just_pressed(&PlayerActions::RemoveVoxel);
    let placing = action_state.just_pressed(&PlayerActions::PlaceVoxel);
    if !removing && !placing {
        return;
    }

    let Some(ray) = camera_ray(&camera_query, &window_query) else {
        warn!("handle_voxel_input: no camera ray (no cursor position?)");
        return;
    };

    let Some(hit) = voxel_world.raycast(chunk_ticket.map_entity, ray, RAYCAST_MAX_DISTANCE, |v| {
        matches!(v, WorldVoxel::Solid(_))
    }) else {
        trace!("handle_voxel_input: raycast hit nothing");
        return;
    };

    let (position, voxel) = if removing {
        (hit.position, VoxelType::Air)
    } else if let Some(normal) = hit.normal {
        (hit.position + normal.as_ivec3(), VoxelType::Solid(0))
    } else {
        trace!("handle_voxel_input: place hit has no normal");
        return;
    };

    let sequence = prediction_state.next();
    let old_voxel = voxel_world
        .get_voxel(chunk_ticket.map_entity, position)
        .into();

    voxel_world.set_voxel(chunk_ticket.map_entity, position, WorldVoxel::from(voxel));

    prediction_state.pending.push(VoxelPrediction {
        sequence,
        position,
        old_voxel,
        new_voxel: voxel,
    });

    for mut sender in message_sender.iter_mut() {
        trace!("Sending voxel edit request to server: {:?}", position);
        sender.send::<VoxelChannel>(VoxelEditRequest {
            position,
            voxel,
            sequence,
        });
    }
}

fn camera_ray(
    camera_query: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: &Query<&Window, With<PrimaryWindow>>,
) -> Option<Ray3d> {
    let (camera, camera_transform) = camera_query.single().ok()?;
    let window = window_query.single().ok()?;
    let cursor_pos = window.cursor_position()?;
    let viewport_pos = if let Some(rect) = camera.logical_viewport_rect() {
        cursor_pos - rect.min
    } else {
        cursor_pos
    };

    camera
        .viewport_to_world(camera_transform, viewport_pos)
        .ok()
}

/// Processes server acknowledgments, clearing confirmed predictions.
fn handle_voxel_edit_ack(
    mut receivers: Query<&mut MessageReceiver<VoxelEditAck>>,
    mut prediction_state: ResMut<VoxelPredictionState>,
) {
    for mut receiver in &mut receivers {
        for ack in receiver.receive() {
            trace!(
                "handle_voxel_edit_ack: ack seq={}, clearing {} pending",
                ack.sequence,
                prediction_state.pending.len()
            );
            prediction_state
                .pending
                .retain(|p| p.sequence > ack.sequence);
        }
    }
}

/// Processes server rejections, rolling back the predicted voxel to the correct value.
fn handle_voxel_edit_reject(
    mut receivers: Query<&mut MessageReceiver<VoxelEditReject>>,
    mut prediction_state: ResMut<VoxelPredictionState>,
    mut voxel_world: VoxelWorld,
    player_query: Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
) {
    let Ok(chunk_ticket) = player_query.single() else {
        trace!("handle_voxel_edit_reject: no predicted player");
        return;
    };

    for mut receiver in &mut receivers {
        for reject in receiver.receive() {
            warn!(
                "handle_voxel_edit_reject: rejected seq={} at {:?}, correct={:?}",
                reject.sequence, reject.position, reject.correct_voxel
            );
            voxel_world.set_voxel(
                chunk_ticket.map_entity,
                reject.position,
                WorldVoxel::from(reject.correct_voxel),
            );
            prediction_state
                .pending
                .retain(|p| p.sequence != reject.sequence);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prediction_state_sequence_increments() {
        let mut state = VoxelPredictionState::default();
        assert_eq!(state.next(), 0);
        assert_eq!(state.next(), 1);
        assert_eq!(state.next(), 2);
    }

    #[test]
    fn ack_clears_predictions_up_to_sequence() {
        let mut state = VoxelPredictionState::default();
        for i in 0..5 {
            state.pending.push(VoxelPrediction {
                sequence: i,
                position: IVec3::ZERO,
                old_voxel: VoxelType::Air,
                new_voxel: VoxelType::Solid(1),
            });
        }
        // Ack sequence 2 — clears 0, 1, 2
        state.pending.retain(|p| p.sequence > 2);
        assert_eq!(state.pending.len(), 2);
        assert_eq!(state.pending[0].sequence, 3);
    }

    #[test]
    fn broadcast_skipped_for_pending_prediction_position() {
        let mut state = VoxelPredictionState::default();
        state.pending.push(VoxelPrediction {
            sequence: 0,
            position: IVec3::new(5, 10, 15),
            old_voxel: VoxelType::Solid(1),
            new_voxel: VoxelType::Air,
        });

        let broadcast_pos = IVec3::new(5, 10, 15);
        let has_pending = state.pending.iter().any(|p| p.position == broadcast_pos);
        assert!(
            has_pending,
            "broadcast at pending prediction position should be filtered"
        );

        let other_pos = IVec3::new(1, 2, 3);
        let has_pending_other = state.pending.iter().any(|p| p.position == other_pos);
        assert!(
            !has_pending_other,
            "broadcast at non-pending position should not be filtered"
        );
    }

    #[test]
    fn reject_removes_specific_prediction() {
        let mut state = VoxelPredictionState::default();
        for i in 0..5 {
            state.pending.push(VoxelPrediction {
                sequence: i,
                position: IVec3::new(i as i32, 0, 0),
                old_voxel: VoxelType::Air,
                new_voxel: VoxelType::Solid(1),
            });
        }

        let rejected_seq = 2u32;
        state.pending.retain(|p| p.sequence != rejected_seq);

        assert_eq!(state.pending.len(), 4);
        assert!(
            state.pending.iter().all(|p| p.sequence != 2),
            "rejected prediction should be removed"
        );
        assert!(
            state.pending.iter().any(|p| p.sequence == 0),
            "other predictions should remain"
        );
        assert!(
            state.pending.iter().any(|p| p.sequence == 1),
            "other predictions should remain"
        );
        assert!(
            state.pending.iter().any(|p| p.sequence == 3),
            "other predictions should remain"
        );
        assert!(
            state.pending.iter().any(|p| p.sequence == 4),
            "other predictions should remain"
        );
    }
}
