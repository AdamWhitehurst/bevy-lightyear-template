use std::collections::HashSet;
use std::sync::Arc;

use avian3d::prelude::{ColliderDisabled, LinearVelocity, Position, RigidBodyDisabled};
use bevy::{prelude::*, window::PrimaryWindow};
use leafwing_input_manager::prelude::*;
use lightyear::prelude::{Controlled, DisableRollback, MessageReceiver, MessageSender, Predicted};
use protocol::map::{MapChannel, MapTransitionEnd, MapTransitionReady, MapTransitionStart};
use protocol::{
    CharacterMarker, ChunkChannel, ChunkDataSync, ChunkRequest, ChunkUnload, MapInstanceId,
    MapRegistry, PendingTransition, PlayerActions, TransitionReadySent, VoxelChannel, VoxelEditAck,
    VoxelEditBroadcast, VoxelEditReject, VoxelEditRequest, VoxelType,
};
use ui::MapTransitionState;
use voxel_map_engine::prelude::{
    flat_terrain_voxels, mesh_chunk_greedy, ChunkData, ChunkTarget, DefaultVoxelMaterial,
    VoxelChunk, VoxelMapConfig, VoxelMapInstance, VoxelPlugin, VoxelWorld, WorldVoxel,
};

const RAYCAST_MAX_DISTANCE: f32 = 100.0;

/// How often (in seconds) to retry requesting chunks that haven't arrived.
const CHUNK_REQUEST_RETRY_INTERVAL: f32 = 0.1;

/// Tracks which chunks the client has received from the server.
#[derive(Component)]
pub struct ClientChunkState {
    pub received: HashSet<IVec3>,
    pending_requests: HashSet<IVec3>,
    retry_timer: f32,
}

impl Default for ClientChunkState {
    fn default() -> Self {
        Self {
            received: HashSet::new(),
            pending_requests: HashSet::new(),
            retry_timer: 0.0,
        }
    }
}

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
            .add_systems(Startup, spawn_overworld)
            .add_systems(
                Update,
                (
                    attach_chunk_target_to_player,
                    handle_voxel_broadcasts,
                    handle_voxel_edit_ack,
                    handle_voxel_edit_reject,
                    request_missing_chunks,
                    handle_chunk_data_sync,
                    handle_chunk_unload,
                    protocol::attach_chunk_colliders,
                )
                    .run_if(in_state(ui::ClientState::InGame)),
            )
            .add_systems(
                PostUpdate,
                handle_voxel_input
                    .run_if(in_state(ui::ClientState::InGame))
                    .after(TransformSystems::Propagate),
            )
            .add_systems(Update, handle_map_transition_start)
            .add_systems(
                Update,
                (check_transition_chunks_loaded, handle_map_transition_end)
                    .run_if(in_state(MapTransitionState::Transitioning)),
            );
    }
}

/// Resource tracking the primary overworld map entity.
#[derive(Resource)]
pub struct OverworldMap(pub Entity);

fn spawn_overworld(mut commands: Commands, mut registry: ResMut<MapRegistry>) {
    let mut config = VoxelMapConfig::new(0, 0, 2, None, 5, Arc::new(flat_terrain_voxels));
    config.generates_chunks = false;

    let map = commands
        .spawn((
            VoxelMapInstance::new(5),
            config,
            ClientChunkState::default(),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();
    commands.insert_resource(OverworldMap(map));
    registry.insert(MapInstanceId::Overworld, map);
}

fn attach_chunk_target_to_player(
    mut commands: Commands,
    registry: Res<MapRegistry>,
    players: Query<
        (Entity, &MapInstanceId),
        (With<Predicted>, With<CharacterMarker>, Without<ChunkTarget>),
    >,
) {
    for (entity, map_id) in &players {
        info!("Attaching ChunkTarget to player {entity:?} on map {map_id:?}");
        let map_entity = registry.get(map_id);
        commands
            .entity(entity)
            .insert(ChunkTarget::new(map_entity, 4));
    }
}

/// Requests chunks from server that the client needs but doesn't have.
fn request_missing_chunks(
    mut chunk_state_query: Query<&mut ClientChunkState>,
    chunk_targets: Query<
        (&ChunkTarget, &Position),
        (With<Predicted>, With<CharacterMarker>, With<Controlled>),
    >,
    map_query: Query<(&VoxelMapInstance, &VoxelMapConfig)>,
    mut senders: Query<&mut MessageSender<ChunkRequest>>,
    time: Res<Time>,
) {
    let Ok((target, pos)) = chunk_targets.single() else {
        trace!("request_missing_chunks: no predicted player with ChunkTarget + Position");
        return;
    };
    let Ok((instance, config)) = map_query.get(target.map_entity) else {
        trace!(
            "request_missing_chunks: map entity {:?} missing VoxelMapInstance or VoxelMapConfig",
            target.map_entity
        );
        return;
    };
    let Ok(mut state) = chunk_state_query.get_mut(target.map_entity) else {
        trace!(
            "request_missing_chunks: map entity {:?} missing ClientChunkState",
            target.map_entity
        );
        return;
    };

    // Periodically clear pending requests so unfulfilled ones get retried.
    state.retry_timer += time.delta_secs();
    if state.retry_timer >= CHUNK_REQUEST_RETRY_INTERVAL {
        state.retry_timer = 0.0;
        state.pending_requests.clear();
    }

    let center = (pos.0 / 16.0).floor().as_ivec3();
    let dist = target.distance as i32;
    let mut desired = HashSet::new();
    for x in -dist..=dist {
        for y in -dist..=dist {
            for z in -dist..=dist {
                let p = center + IVec3::new(x, y, z);
                if config.bounds.map_or(true, |b| {
                    p.x.abs() < b.x && p.y.abs() < b.y && p.z.abs() < b.z
                }) {
                    desired.insert(p);
                }
            }
        }
    }

    for &chunk_pos in &desired {
        if instance.loaded_chunks.contains(&chunk_pos) {
            continue;
        }
        if state.received.contains(&chunk_pos) {
            continue;
        }
        if state.pending_requests.contains(&chunk_pos) {
            continue;
        }

        for mut sender in senders.iter_mut() {
            sender.send::<ChunkChannel>(ChunkRequest { chunk_pos });
        }
        state.pending_requests.insert(chunk_pos);
    }

    state.received.retain(|pos| desired.contains(pos));
    state.pending_requests.retain(|pos| desired.contains(pos));
}

/// Receives chunk data from server and inserts into the local VoxelMapInstance.
fn handle_chunk_data_sync(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<ChunkDataSync>>,
    mut map_query: Query<(Entity, &mut VoxelMapInstance)>,
    mut chunk_state_query: Query<&mut ClientChunkState>,
    player_query: Query<&ChunkTarget, (With<Predicted>, With<CharacterMarker>, With<Controlled>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    default_material: Res<DefaultVoxelMaterial>,
) {
    let Ok(chunk_target) = player_query.single() else {
        trace!("handle_chunk_data_sync: no predicted player with ChunkTarget");
        return;
    };
    let Ok((map_entity, mut instance)) = map_query.get_mut(chunk_target.map_entity) else {
        trace!(
            "handle_chunk_data_sync: map entity {:?} missing VoxelMapInstance",
            chunk_target.map_entity
        );
        return;
    };
    let Ok(mut chunk_state) = chunk_state_query.get_mut(chunk_target.map_entity) else {
        trace!(
            "handle_chunk_data_sync: map entity {:?} missing ClientChunkState",
            chunk_target.map_entity
        );
        return;
    };

    for mut receiver in &mut receivers {
        for sync in receiver.receive() {
            let voxels = sync.data.to_voxels();
            let chunk_data = ChunkData::from_voxels(&voxels);

            instance.insert_chunk_data(sync.chunk_pos, chunk_data);
            instance.loaded_chunks.insert(sync.chunk_pos);

            chunk_state.pending_requests.remove(&sync.chunk_pos);
            chunk_state.received.insert(sync.chunk_pos);

            let Some(mesh) = mesh_chunk_greedy(&voxels) else {
                continue;
            };
            let mesh_handle = meshes.add(mesh);
            let offset = sync.chunk_pos.as_vec3() * 16.0 - Vec3::ONE;
            let material = if instance.debug_colors {
                let hash = (sync.chunk_pos.x.wrapping_mul(73856093))
                    ^ (sync.chunk_pos.y.wrapping_mul(19349663))
                    ^ (sync.chunk_pos.z.wrapping_mul(83492791));
                let hue = ((hash as u32) % 360) as f32;
                materials.add(StandardMaterial {
                    base_color: Color::hsl(hue, 0.5, 0.5),
                    perceptual_roughness: 0.9,
                    ..default()
                })
            } else {
                default_material.0.clone()
            };

            let chunk_entity = commands
                .spawn((
                    VoxelChunk {
                        position: sync.chunk_pos,
                        lod_level: 0,
                    },
                    Mesh3d(mesh_handle),
                    MeshMaterial3d(material),
                    Transform::from_translation(offset),
                ))
                .id();
            commands.entity(map_entity).add_child(chunk_entity);
        }
    }
}

/// Handles server chunk unload messages.
fn handle_chunk_unload(
    mut receivers: Query<&mut MessageReceiver<ChunkUnload>>,
    mut map_query: Query<&mut VoxelMapInstance>,
    mut chunk_state_query: Query<&mut ClientChunkState>,
    player_query: Query<&ChunkTarget, (With<Predicted>, With<CharacterMarker>, With<Controlled>)>,
) {
    let Ok(chunk_target) = player_query.single() else {
        trace!("handle_chunk_unload: no controlled predicted player with ChunkTarget");
        return;
    };
    let Ok(mut instance) = map_query.get_mut(chunk_target.map_entity) else {
        return;
    };

    for mut receiver in &mut receivers {
        for unload in receiver.receive() {
            instance.loaded_chunks.remove(&unload.chunk_pos);
            instance.remove_chunk_data(unload.chunk_pos);
            if let Ok(mut state) = chunk_state_query.get_mut(chunk_target.map_entity) {
                state.received.remove(&unload.chunk_pos);
            }
        }
    }
}

/// Applies voxel edit broadcasts from the server, skipping positions with pending predictions.
fn handle_voxel_broadcasts(
    mut receiver: Query<&mut MessageReceiver<VoxelEditBroadcast>>,
    player_query: Query<&ChunkTarget, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    mut voxel_world: VoxelWorld,
    prediction_state: Res<VoxelPredictionState>,
) {
    let Ok(chunk_target) = player_query.single() else {
        trace!("handle_voxel_broadcasts: no predicted player with ChunkTarget");
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

            debug!(
                "handle_voxel_broadcasts: applying broadcast at {:?} voxel={:?}",
                broadcast.position, broadcast.voxel
            );
            voxel_world.set_voxel(
                chunk_target.map_entity,
                broadcast.position,
                WorldVoxel::from(broadcast.voxel),
            );
        }
    }
}

fn handle_voxel_input(
    player_query: Query<&ChunkTarget, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    mut voxel_world: VoxelWorld,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    action_query: Query<&ActionState<PlayerActions>, With<Controlled>>,
    mut message_sender: Query<&mut MessageSender<VoxelEditRequest>>,
    mut prediction_state: ResMut<VoxelPredictionState>,
) {
    let Ok(chunk_target) = player_query.single() else {
        trace!("handle_voxel_input: no predicted player with ChunkTarget");
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

    let Some(hit) = voxel_world.raycast(chunk_target.map_entity, ray, RAYCAST_MAX_DISTANCE, |v| {
        matches!(v, WorldVoxel::Solid(_))
    }) else {
        debug!("handle_voxel_input: raycast hit nothing");
        return;
    };

    let (position, voxel) = if removing {
        (hit.position, VoxelType::Air)
    } else if let Some(normal) = hit.normal {
        (hit.position + normal.as_ivec3(), VoxelType::Solid(0))
    } else {
        debug!("handle_voxel_input: place hit has no normal");
        return;
    };

    let sequence = prediction_state.next();
    let old_voxel = voxel_world
        .get_voxel(chunk_target.map_entity, position)
        .into();

    voxel_world.set_voxel(chunk_target.map_entity, position, WorldVoxel::from(voxel));

    prediction_state.pending.push(VoxelPrediction {
        sequence,
        position,
        old_voxel,
        new_voxel: voxel,
    });

    for mut sender in message_sender.iter_mut() {
        debug!("Sending voxel edit request to server: {:?}", position);
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
            debug!(
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
    player_query: Query<&ChunkTarget, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
) {
    let Ok(chunk_target) = player_query.single() else {
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
                chunk_target.map_entity,
                reject.position,
                WorldVoxel::from(reject.correct_voxel),
            );
            prediction_state
                .pending
                .retain(|p| p.sequence != reject.sequence);
        }
    }
}

pub fn handle_map_transition_start(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<MapTransitionStart>>,
    mut registry: ResMut<MapRegistry>,
    player_query: Query<
        (Entity, &MapInstanceId),
        (With<Predicted>, With<CharacterMarker>, With<Controlled>),
    >,
) {
    for mut receiver in &mut receivers {
        for transition in receiver.receive() {
            info!("Received MapTransitionStart for {:?}", transition.target);

            let (player, current_map_id) = player_query
                .single()
                .expect("Predicted player must exist when receiving MapTransitionStart");

            despawn_old_map(&mut commands, &mut *registry, current_map_id);

            commands.entity(player).insert((
                RigidBodyDisabled,
                ColliderDisabled,
                DisableRollback,
                PendingTransition(transition.target.clone()),
                Position(transition.spawn_position),
                LinearVelocity(Vec3::ZERO),
            ));

            if !registry.0.contains_key(&transition.target) {
                let generator = generator_for_map(&transition.target);
                let map_entity = spawn_map_instance(
                    &mut commands,
                    &transition.target,
                    transition.seed,
                    transition.bounds,
                    generator,
                );
                registry.insert(transition.target.clone(), map_entity);
            }

            let map_entity = registry.get(&transition.target);
            commands
                .entity(player)
                .insert(ChunkTarget::new(map_entity, 4));
        }
    }
}

/// Despawn the old map entity and all its children (chunk meshes), removing it from the registry.
/// When the player transitions back, the map will be re-created and chunks re-requested.
fn despawn_old_map(
    commands: &mut Commands,
    registry: &mut MapRegistry,
    old_map_id: &MapInstanceId,
) {
    if let Some(old_map_entity) = registry.0.remove(old_map_id) {
        info!("Despawning old map {old_map_id:?} entity {old_map_entity:?}");
        commands.entity(old_map_entity).despawn();
    }
}

fn generator_for_map(
    map_id: &MapInstanceId,
) -> Arc<dyn Fn(IVec3) -> Vec<WorldVoxel> + Send + Sync> {
    match map_id {
        MapInstanceId::Overworld => Arc::new(flat_terrain_voxels),
        MapInstanceId::Homebase { .. } => Arc::new(flat_terrain_voxels),
    }
}

fn spawn_map_instance(
    commands: &mut Commands,
    map_id: &MapInstanceId,
    seed: u64,
    bounds: Option<IVec3>,
    generator: Arc<dyn Fn(IVec3) -> Vec<WorldVoxel> + Send + Sync>,
) -> Entity {
    let tree_height = match map_id {
        MapInstanceId::Overworld => 5,
        MapInstanceId::Homebase { .. } => 3,
    };
    let spawning_distance = bounds.map(|b| b.max_element().max(1) as u32).unwrap_or(10);

    let mut config =
        VoxelMapConfig::new(seed, 0, spawning_distance, bounds, tree_height, generator);
    config.generates_chunks = false;

    let entity = commands
        .spawn((
            VoxelMapInstance::new(tree_height),
            config,
            ClientChunkState::default(),
            Transform::default(),
            map_id.clone(),
        ))
        .id();

    info!("Spawned client map instance for {map_id:?}: {entity:?}");
    entity
}

/// Checks if the client has received chunks for the transition target map.
/// The transition is "ready" when at least one chunk has been received from the server.
pub fn check_transition_chunks_loaded(
    mut commands: Commands,
    player_query: Query<
        (Entity, &PendingTransition),
        (
            With<Predicted>,
            With<CharacterMarker>,
            Without<TransitionReadySent>,
        ),
    >,
    registry: Res<MapRegistry>,
    chunk_state_query: Query<&ClientChunkState>,
    mut senders: Query<&mut MessageSender<MapTransitionReady>>,
) {
    let Ok((player, pending)) = player_query.single() else {
        return;
    };
    let map_entity = registry.get(&pending.0);
    let Ok(chunk_state) = chunk_state_query.get(map_entity) else {
        trace!("check_transition_chunks_loaded: no ClientChunkState on map entity yet");
        return;
    };

    if chunk_state.received.is_empty() {
        return;
    }

    info!(
        "Transition chunks loaded for {:?} ({} received), sending ready to server",
        pending.0,
        chunk_state.received.len()
    );

    commands.entity(player).insert(TransitionReadySent);

    for mut sender in &mut senders {
        sender.send::<MapChannel>(MapTransitionReady);
    }
}

pub fn handle_map_transition_end(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<MapTransitionEnd>>,
    player_query: Query<
        Entity,
        (
            With<Predicted>,
            With<CharacterMarker>,
            With<PendingTransition>,
        ),
    >,
    mut next_transition: ResMut<NextState<MapTransitionState>>,
) {
    for mut receiver in &mut receivers {
        for _end in receiver.receive() {
            info!("Received MapTransitionEnd, resuming play");

            let Ok(player) = player_query.single() else {
                warn!("Received MapTransitionEnd but no transitioning player");
                continue;
            };

            commands.entity(player).remove::<(
                RigidBodyDisabled,
                ColliderDisabled,
                DisableRollback,
                PendingTransition,
                TransitionReadySent,
            )>();

            next_transition.set(MapTransitionState::Playing);
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
}
