use std::collections::HashSet;
use std::sync::Arc;

use avian3d::prelude::{ColliderDisabled, LinearVelocity, Position, RigidBodyDisabled};
use bevy::{prelude::*, window::PrimaryWindow};
use leafwing_input_manager::prelude::*;
use lightyear::prelude::{Controlled, DisableRollback, MessageReceiver, MessageSender, Predicted};
use protocol::map::{MapChannel, MapTransitionEnd, MapTransitionReady, MapTransitionStart};
use protocol::{
    CharacterMarker, ChunkChannel, ChunkDataSync, ChunkRequest, ChunkUnload, MapInstanceId,
    MapRegistry, PendingTransition, PlayerActions, TransitionReadySent, VoxelChannel,
    VoxelEditBroadcast, VoxelEditRequest, VoxelType,
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

/// Plugin managing client-side voxel map functionality.
pub struct ClientMapPlugin;

impl Plugin for ClientMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelPlugin)
            .init_resource::<MapRegistry>()
            .add_systems(Startup, spawn_overworld)
            .add_systems(
                Update,
                (
                    attach_chunk_target_to_player,
                    handle_voxel_broadcasts,
                    request_missing_chunks,
                    handle_chunk_data_sync,
                    handle_chunk_unload,
                    protocol::attach_chunk_colliders,
                )
                    .run_if(in_state(ui::ClientState::InGame)),
            )
            .add_systems(
                PostUpdate,
                handle_voxel_input.after(TransformSystems::Propagate),
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

fn handle_voxel_broadcasts(
    mut receiver: Query<&mut MessageReceiver<VoxelEditBroadcast>>,
    player_query: Query<&ChunkTarget, (With<Predicted>, With<CharacterMarker>)>,
    mut voxel_world: VoxelWorld,
) {
    let Ok(chunk_target) = player_query.single() else {
        return;
    };
    for mut message_receiver in receiver.iter_mut() {
        for broadcast in message_receiver.receive() {
            voxel_world.set_voxel(
                chunk_target.map_entity,
                broadcast.position,
                WorldVoxel::from(broadcast.voxel),
            );
        }
    }
}

fn handle_voxel_input(
    player_query: Query<&ChunkTarget, (With<Predicted>, With<CharacterMarker>)>,
    voxel_world: VoxelWorld,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    action_query: Query<&ActionState<PlayerActions>, With<Controlled>>,
    message_sender: Query<&mut MessageSender<VoxelEditRequest>>,
) {
    let Ok(chunk_target) = player_query.single() else {
        return;
    };
    let Ok(action_state) = action_query.single() else {
        return;
    };

    let removing = action_state.just_pressed(&PlayerActions::RemoveVoxel);
    let placing = action_state.just_pressed(&PlayerActions::PlaceVoxel);
    if !removing && !placing {
        return;
    }

    let Some(ray) = camera_ray(&camera_query, &window_query) else {
        return;
    };

    let Some(hit) = voxel_world.raycast(chunk_target.map_entity, ray, RAYCAST_MAX_DISTANCE, |v| {
        matches!(v, WorldVoxel::Solid(_))
    }) else {
        return;
    };

    if removing {
        send_voxel_edit(hit.position, VoxelType::Air, message_sender);
    } else if let Some(normal) = hit.normal {
        let place_pos = hit.position + normal.as_ivec3();
        send_voxel_edit(place_pos, VoxelType::Solid(0), message_sender);
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

/// Send a voxel edit request to the server.
pub fn send_voxel_edit(
    position: IVec3,
    voxel: VoxelType,
    mut message_sender: Query<&mut MessageSender<VoxelEditRequest>>,
) {
    for mut sender in message_sender.iter_mut() {
        debug!("Sending voxel edit request to server: {:?}", position);
        sender.send::<VoxelChannel>(VoxelEditRequest { position, voxel });
    }
}

pub fn handle_map_transition_start(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<MapTransitionStart>>,
    mut registry: ResMut<MapRegistry>,
    player_query: Query<Entity, (With<Predicted>, With<CharacterMarker>, With<Controlled>)>,
) {
    for mut receiver in &mut receivers {
        for transition in receiver.receive() {
            info!("Received MapTransitionStart for {:?}", transition.target);

            let player = player_query
                .single()
                .expect("Predicted player must exist when receiving MapTransitionStart");

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
