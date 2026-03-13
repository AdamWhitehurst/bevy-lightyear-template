use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use avian3d::prelude::{ColliderDisabled, Position, RigidBodyDisabled};
use bevy::app::AppExit;
use bevy::prelude::*;
use lightyear::prelude::{
    ControlledBy, DisableRollback, MessageReceiver, MessageSender, NetworkVisibility, RemoteId,
    Room, RoomEvent, RoomTarget, ServerMultiMessageSender,
};
use protocol::map::{
    MapChannel, MapSwitchTarget, MapTransitionEnd, MapTransitionReady, MapTransitionStart,
    PlayerMapSwitchRequest,
};
use protocol::{
    CharacterMarker, ChunkChannel, ChunkDataSync, ChunkRequest, MapInstanceId, MapRegistry,
    PendingTransition, VoxelChannel, VoxelEditAck, VoxelEditBroadcast, VoxelEditReject,
    VoxelEditRequest, VoxelType,
};
use voxel_map_engine::prelude::{
    flat_terrain_voxels, ChunkTarget, VoxelMapConfig, VoxelMapInstance, VoxelPlugin, VoxelWorld,
    WorldVoxel,
};

use crate::persistence::{
    load_entities, load_map_meta, map_save_dir, save_entities, save_map_meta, MapMeta,
    WorldSavePath,
};
use protocol::map::{MapSaveTarget, SavedEntity, SavedEntityKind};
use protocol::RespawnPoint;
use voxel_map_engine::persistence as chunk_persist;

/// Plugin managing server-side voxel map functionality.
pub struct ServerMapPlugin;

/// Maps `MapInstanceId` to lightyear room entities. Server-only.
#[derive(Resource, Default)]
pub struct RoomRegistry(pub HashMap<MapInstanceId, Entity>);

impl RoomRegistry {
    pub fn get_or_create(&mut self, id: &MapInstanceId, commands: &mut Commands) -> Entity {
        *self.0.entry(id.clone()).or_insert_with(|| {
            let room = commands.spawn(Room::default()).id();
            info!("Created room for map {id:?}: {room:?}");
            room
        })
    }
}

/// Resource tracking the primary overworld map entity.
#[derive(Resource)]
pub struct OverworldMap(pub Entity);

const DEFAULT_OVERWORLD_SEED: u64 = 999;
const GENERATION_VERSION: u32 = 0;
const SAVE_DEBOUNCE_SECONDS: f64 = 1.0;
const MAX_DIRTY_SECONDS: f64 = 5.0;

/// Tracks whether any map has unsaved dirty chunks.
#[derive(Resource)]
pub struct WorldDirtyState {
    pub is_dirty: bool,
    pub last_edit_time: f64,
    pub first_dirty_time: Option<f64>,
}

impl Default for WorldDirtyState {
    fn default() -> Self {
        Self {
            is_dirty: false,
            last_edit_time: 0.0,
            first_dirty_time: None,
        }
    }
}

/// A voxel edit pending broadcast, with context for room-scoped sending.
pub struct PendingVoxelEdit {
    pub position: IVec3,
    pub voxel: VoxelType,
    /// Client entity that made the edit (excluded from broadcast).
    pub originator: Entity,
    pub map_id: MapInstanceId,
}

/// Accumulates voxel edits per chunk during a tick for batching.
#[derive(Resource, Default)]
pub struct PendingVoxelBroadcasts {
    pub per_chunk: HashMap<IVec3, Vec<PendingVoxelEdit>>,
}

pub fn spawn_overworld(
    mut commands: Commands,
    mut registry: ResMut<MapRegistry>,
    save_path: Res<WorldSavePath>,
) {
    let map_dir = map_save_dir(&save_path.0, &MapInstanceId::Overworld);

    let (seed, generation_version) = match load_map_meta(&map_dir) {
        Ok(Some(meta)) => (meta.seed, meta.generation_version),
        _ => (DEFAULT_OVERWORLD_SEED, GENERATION_VERSION),
    };

    let mut config = VoxelMapConfig::new(
        seed,
        generation_version,
        2,
        None,
        5,
        Arc::new(flat_terrain_voxels),
    );
    config.save_dir = Some(map_dir);

    let map = commands
        .spawn((
            VoxelMapInstance::new(5),
            config,
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();
    commands.insert_resource(OverworldMap(map));
    registry.insert(MapInstanceId::Overworld, map);
}

fn save_dirty_chunks_debounced(
    time: Res<Time>,
    mut dirty_state: ResMut<WorldDirtyState>,
    save_path: Res<WorldSavePath>,
    mut map_query: Query<(&mut VoxelMapInstance, &VoxelMapConfig, &MapInstanceId)>,
    entity_query: Query<(
        &MapSaveTarget,
        &MapInstanceId,
        &Position,
        Option<&RespawnPoint>,
    )>,
    respawn_query: Query<(&Position, &MapInstanceId), With<RespawnPoint>>,
) {
    if !dirty_state.is_dirty {
        return;
    }

    let now = time.elapsed_secs_f64();
    let time_since_edit = now - dirty_state.last_edit_time;
    let time_since_first_dirty = dirty_state.first_dirty_time.map(|t| now - t).unwrap_or(0.0);

    let should_save =
        time_since_edit >= SAVE_DEBOUNCE_SECONDS || time_since_first_dirty >= MAX_DIRTY_SECONDS;

    if !should_save {
        return;
    }

    for (mut instance, config, map_id) in &mut map_query {
        let Some(map_dir) = config.save_dir.as_deref() else {
            trace!("save_dirty_chunks_debounced: no save_dir for {map_id:?}, skipping");
            continue;
        };

        save_dirty_chunks_for_instance(&mut instance, map_dir);

        let spawn_points: Vec<Vec3> = respawn_query
            .iter()
            .filter(|(_, mid)| *mid == map_id)
            .map(|(pos, _)| pos.0)
            .collect();
        let meta = MapMeta {
            version: 1,
            seed: config.seed,
            generation_version: config.generation_version,
            spawn_points,
        };
        if let Err(e) = save_map_meta(map_dir, &meta) {
            error!("Failed to save map meta for {map_id:?}: {e}");
        }
    }

    collect_and_save_entities(&save_path, &entity_query);

    dirty_state.is_dirty = false;
    dirty_state.first_dirty_time = None;
}

/// Drain dirty chunks from an instance and persist them to disk.
pub fn save_dirty_chunks_for_instance(instance: &mut VoxelMapInstance, map_dir: &Path) {
    let dirty: Vec<IVec3> = instance.dirty_chunks.drain().collect();
    for chunk_pos in dirty {
        if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
            if let Err(e) = chunk_persist::save_chunk(map_dir, chunk_pos, chunk_data) {
                error!("Failed to save chunk at {chunk_pos}: {e}");
                instance.dirty_chunks.insert(chunk_pos);
            }
        }
    }
}

pub fn save_world_on_shutdown(
    mut exit_reader: MessageReader<AppExit>,
    mut map_query: Query<(&mut VoxelMapInstance, &VoxelMapConfig, &MapInstanceId)>,
    dirty_state: Res<WorldDirtyState>,
    save_path: Res<WorldSavePath>,
    entity_query: Query<(
        &MapSaveTarget,
        &MapInstanceId,
        &Position,
        Option<&RespawnPoint>,
    )>,
    respawn_query: Query<(&Position, &MapInstanceId), With<RespawnPoint>>,
) {
    if exit_reader.is_empty() {
        return;
    }
    exit_reader.clear();

    if !dirty_state.is_dirty {
        return;
    }

    for (mut instance, config, map_id) in &mut map_query {
        let Some(map_dir) = config.save_dir.as_deref() else {
            continue;
        };
        save_dirty_chunks_for_instance(&mut instance, map_dir);

        let spawn_points: Vec<Vec3> = respawn_query
            .iter()
            .filter(|(_, mid)| *mid == map_id)
            .map(|(pos, _)| pos.0)
            .collect();
        let meta = MapMeta {
            version: 1,
            seed: config.seed,
            generation_version: config.generation_version,
            spawn_points,
        };
        if let Err(e) = save_map_meta(map_dir, &meta) {
            error!("Failed to save meta on shutdown for {map_id:?}: {e}");
        }
    }

    collect_and_save_entities(&save_path, &entity_query);
    info!("World saved on shutdown");
}

/// Collect all persistable entities grouped by map and save to disk.
fn collect_and_save_entities(
    save_path: &WorldSavePath,
    entity_query: &Query<(
        &MapSaveTarget,
        &MapInstanceId,
        &Position,
        Option<&RespawnPoint>,
    )>,
) {
    let mut by_map: HashMap<MapInstanceId, Vec<SavedEntity>> = HashMap::new();

    for item in entity_query.iter() {
        let (_marker, map_id, position, respawn): (
            &MapSaveTarget,
            &MapInstanceId,
            &Position,
            Option<&RespawnPoint>,
        ) = item;
        let kind = if respawn.is_some() {
            SavedEntityKind::RespawnPoint
        } else {
            debug_assert!(
                false,
                "Entity with MapSaveTarget has no recognized SavedEntityKind"
            );
            continue;
        };

        by_map.entry(map_id.clone()).or_default().push(SavedEntity {
            kind,
            position: position.0,
        });
    }

    for (map_id, entities) in &by_map {
        let map_dir = map_save_dir(&save_path.0, map_id);
        if let Err(e) = save_entities(&map_dir, entities) {
            error!("Failed to save entities for {map_id:?}: {e}");
        }
    }
}

/// Load entities from disk for a map and spawn them in the ECS.
fn load_map_entities(
    commands: &mut Commands,
    save_path: &WorldSavePath,
    map_id: &MapInstanceId,
) -> usize {
    let map_dir = map_save_dir(&save_path.0, map_id);
    let entities = match load_entities(&map_dir) {
        Ok(entities) => entities,
        Err(e) => {
            warn!("Failed to load entities for {map_id:?}: {e}");
            return 0;
        }
    };

    let count = entities.len();
    for saved in entities {
        match saved.kind {
            SavedEntityKind::RespawnPoint => {
                commands.spawn((RespawnPoint, Position(saved.position), map_id.clone()));
            }
        }
    }
    count
}

/// Load persisted entities for the overworld on startup.
pub fn load_startup_entities(mut commands: Commands, save_path: Res<WorldSavePath>) {
    let count = load_map_entities(&mut commands, &save_path, &MapInstanceId::Overworld);
    if count > 0 {
        info!("Loaded {count} entities for overworld");
    }
}

fn on_map_instance_id_added(
    trigger: On<Add, MapInstanceId>,
    mut commands: Commands,
    map_ids: Query<&MapInstanceId>,
    mut room_registry: ResMut<RoomRegistry>,
) {
    let entity = trigger.entity;
    let map_id = map_ids
        .get(entity)
        .expect("Entity with MapInstanceId trigger must have MapInstanceId");
    let room = room_registry.get_or_create(map_id, &mut commands);
    commands.entity(entity).try_insert(NetworkVisibility);
    commands.trigger(RoomEvent {
        room,
        target: RoomTarget::AddEntity(entity),
    });
}

impl Plugin for ServerMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(lightyear::prelude::RoomPlugin)
            .add_plugins(VoxelPlugin)
            .init_resource::<MapRegistry>()
            .init_resource::<RoomRegistry>()
            .init_resource::<WorldDirtyState>()
            .init_resource::<PendingVoxelBroadcasts>()
            .init_resource::<WorldSavePath>()
            .add_systems(Startup, (spawn_overworld, load_startup_entities).chain())
            .add_systems(
                Update,
                (
                    handle_voxel_edit_requests,
                    flush_voxel_broadcasts,
                    handle_chunk_requests,
                    save_dirty_chunks_debounced,
                    handle_map_switch_requests,
                    handle_map_transition_ready,
                    protocol::attach_chunk_colliders,
                ),
            )
            .add_systems(Last, save_world_on_shutdown)
            .add_observer(on_map_instance_id_added);
    }
}

pub fn handle_voxel_edit_requests(
    mut receivers: Query<(Entity, &mut MessageReceiver<VoxelEditRequest>)>,
    mut ack_senders: Query<&mut MessageSender<VoxelEditAck>>,
    mut reject_senders: Query<&mut MessageSender<VoxelEditReject>>,
    mut pending_broadcasts: ResMut<PendingVoxelBroadcasts>,
    mut dirty_state: ResMut<WorldDirtyState>,
    time: Res<Time>,
    mut voxel_world: VoxelWorld,
    controlled_query: Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: Res<MapRegistry>,
) {
    for (client_entity, mut receiver) in &mut receivers {
        for request in receiver.receive() {
            debug!("handle_voxel_edit_requests: received edit from {client_entity:?} at {:?} voxel={:?} seq={}", request.position, request.voxel, request.sequence);
            let Some((_, player_map_id)) = controlled_query
                .iter()
                .find(|(ctrl, _)| ctrl.owner == client_entity)
            else {
                trace!("handle_voxel_edit_requests: no character for client {client_entity:?}");
                continue;
            };
            let map_entity = map_registry.get(player_map_id);

            if !validate_voxel_edit(&request, map_entity, &voxel_world) {
                let current_voxel = voxel_world.get_voxel(map_entity, request.position);
                if let Ok(mut sender) = reject_senders.get_mut(client_entity) {
                    sender.send::<VoxelChannel>(VoxelEditReject {
                        sequence: request.sequence,
                        position: request.position,
                        correct_voxel: current_voxel.into(),
                    });
                }
                continue;
            }

            voxel_world.set_voxel(
                map_entity,
                request.position,
                WorldVoxel::from(request.voxel),
            );

            let now = time.elapsed_secs_f64();
            if !dirty_state.is_dirty {
                dirty_state.first_dirty_time = Some(now);
            }
            dirty_state.is_dirty = true;
            dirty_state.last_edit_time = now;

            if let Ok(mut sender) = ack_senders.get_mut(client_entity) {
                debug!(
                    "handle_voxel_edit_requests: ack seq={} to {client_entity:?}",
                    request.sequence
                );
                sender.send::<VoxelChannel>(VoxelEditAck {
                    sequence: request.sequence,
                });
            } else {
                warn!("handle_voxel_edit_requests: no ack sender for {client_entity:?}");
            }

            let chunk_pos = voxel_map_engine::prelude::voxel_to_chunk_pos(request.position);
            pending_broadcasts
                .per_chunk
                .entry(chunk_pos)
                .or_default()
                .push(PendingVoxelEdit {
                    position: request.position,
                    voxel: request.voxel,
                    originator: client_entity,
                    map_id: player_map_id.clone(),
                });
        }
    }
}

/// Validates a voxel edit request. Returns false if the edit should be rejected.
fn validate_voxel_edit(
    _request: &VoxelEditRequest,
    _map_entity: Entity,
    _voxel_world: &VoxelWorld,
) -> bool {
    // TODO: Add validation rules as needed (bounds, range, anti-cheat)
    true
}

/// Drains accumulated voxel edits and broadcasts them to clients in the same room,
/// excluding the originating client.
pub fn flush_voxel_broadcasts(
    mut pending: ResMut<PendingVoxelBroadcasts>,
    mut sender: ServerMultiMessageSender,
    room_registry: Res<RoomRegistry>,
    rooms: Query<&Room>,
) {
    if pending.per_chunk.is_empty() {
        return;
    }

    for (_chunk_pos, edits) in pending.per_chunk.drain() {
        for edit in edits {
            let Some(&room_entity) = room_registry.0.get(&edit.map_id) else {
                warn!("flush_voxel_broadcasts: no room for map {:?}", edit.map_id);
                continue;
            };
            let Ok(room) = rooms.get(room_entity) else {
                warn!("flush_voxel_broadcasts: room entity {room_entity:?} has no Room component");
                continue;
            };

            let broadcast = VoxelEditBroadcast {
                position: edit.position,
                voxel: edit.voxel,
            };

            let targets: bevy::ecs::entity::EntityHashSet = room
                .clients
                .iter()
                .filter(|e| **e != edit.originator)
                .copied()
                .collect();
            sender
                .send_to_entities::<_, VoxelChannel>(&broadcast, &targets)
                .ok();
        }
    }
}

/// Handles client chunk requests — sends chunk data if available.
pub fn handle_chunk_requests(
    mut receivers: Query<(Entity, &mut MessageReceiver<ChunkRequest>)>,
    mut senders: Query<&mut MessageSender<ChunkDataSync>>,
    controlled_query: Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: Res<MapRegistry>,
    map_query: Query<&VoxelMapInstance>,
) {
    for (client_entity, mut receiver) in &mut receivers {
        for request in receiver.receive() {
            let Some((_, player_map_id)) = controlled_query
                .iter()
                .find(|(ctrl, _)| ctrl.owner == client_entity)
            else {
                trace!("handle_chunk_requests: no character for client {client_entity:?}");
                continue;
            };

            let map_entity = map_registry.get(player_map_id);
            let Ok(instance) = map_query.get(map_entity) else {
                trace!("handle_chunk_requests: map entity not ready for {player_map_id:?}");
                continue;
            };

            let data = if let Some(chunk_data) = instance.get_chunk_data(request.chunk_pos) {
                Some(chunk_data.voxels.clone())
            } else if instance.loaded_chunks.contains(&request.chunk_pos) {
                // Chunk was generated but is empty (all air) — tell client so it stops requesting.
                Some(voxel_map_engine::prelude::PalettedChunk::SingleValue(
                    WorldVoxel::Air,
                ))
            } else {
                trace!("Chunk {} not yet generated", request.chunk_pos);
                None
            };

            if let Some(data) = data {
                if let Ok(mut sender) = senders.get_mut(client_entity) {
                    sender.send::<ChunkChannel>(ChunkDataSync {
                        chunk_pos: request.chunk_pos,
                        data,
                    });
                }
            }
        }
    }
}

pub fn handle_map_switch_requests(
    mut commands: Commands,
    mut receivers: Query<(Entity, &mut MessageReceiver<PlayerMapSwitchRequest>)>,
    mut senders: Query<&mut MessageSender<MapTransitionStart>>,
    controlled_query: Query<(Entity, &ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    pending: Query<(), With<PendingTransition>>,
    remote_ids: Query<&RemoteId>,
    mut registry: ResMut<MapRegistry>,
    mut room_registry: ResMut<RoomRegistry>,
    config_query: Query<&VoxelMapConfig>,
) {
    for (client_entity, mut receiver) in &mut receivers {
        for request in receiver.receive() {
            let (player_entity, _controlled_by, current_map_id) = controlled_query
                .iter()
                .find(|(_, ctrl, _)| ctrl.owner == client_entity)
                .unwrap_or_else(|| {
                    panic!(
                        "No character entity found for client {client_entity:?} during map switch"
                    )
                });

            if pending.get(player_entity).is_ok() {
                warn!("Player {player_entity:?} already transitioning, ignoring request");
                continue;
            }

            let remote_id = remote_ids
                .get(client_entity)
                .expect("Client entity must have RemoteId during map switch");
            let target_map_id = resolve_switch_target(&request.target, remote_id.0.to_bits());

            if *current_map_id == target_map_id {
                warn!("Player {player_entity:?} already on target map {target_map_id:?}");
                continue;
            }

            execute_server_transition(
                &mut commands,
                player_entity,
                client_entity,
                current_map_id,
                &target_map_id,
                &mut *registry,
                &mut *room_registry,
                &config_query,
                &mut senders,
            );
        }
    }
}

/// Resolves a `MapSwitchTarget` to a `MapInstanceId` using the client's stable PeerId bits.
fn resolve_switch_target(target: &MapSwitchTarget, client_id_bits: u64) -> MapInstanceId {
    match target {
        MapSwitchTarget::Overworld => MapInstanceId::Overworld,
        MapSwitchTarget::Homebase => MapInstanceId::Homebase {
            owner: client_id_bits,
        },
    }
}

/// Seed, generation_version, and bounds for a map transition message.
struct MapTransitionParams {
    seed: u64,
    generation_version: u32,
    bounds: Option<IVec3>,
}

#[allow(clippy::too_many_arguments)]
fn execute_server_transition(
    commands: &mut Commands,
    player_entity: Entity,
    client_entity: Entity,
    current_map_id: &MapInstanceId,
    target_map_id: &MapInstanceId,
    registry: &mut MapRegistry,
    room_registry: &mut RoomRegistry,
    config_query: &Query<&VoxelMapConfig>,
    senders: &mut Query<&mut MessageSender<MapTransitionStart>>,
) {
    info!("Transitioning player {player_entity:?} from {current_map_id:?} to {target_map_id:?}");

    commands.entity(player_entity).insert((
        DisableRollback,
        ColliderDisabled,
        RigidBodyDisabled,
        PendingTransition(target_map_id.clone()),
    ));

    let old_room = room_registry.get_or_create(current_map_id, commands);
    let new_room = room_registry.get_or_create(target_map_id, commands);

    commands.trigger(RoomEvent {
        room: old_room,
        target: RoomTarget::RemoveEntity(player_entity),
    });
    commands.trigger(RoomEvent {
        room: old_room,
        target: RoomTarget::RemoveSender(client_entity),
    });
    commands.trigger(RoomEvent {
        room: new_room,
        target: RoomTarget::AddEntity(player_entity),
    });
    commands.trigger(RoomEvent {
        room: new_room,
        target: RoomTarget::AddSender(client_entity),
    });

    commands.entity(player_entity).insert(target_map_id.clone());

    let (map_entity, params) = ensure_map_exists(commands, target_map_id, registry, config_query);
    commands
        .entity(player_entity)
        .insert(ChunkTarget::new(map_entity, 4));

    let spawn_position = crate::gameplay::DEFAULT_SPAWN_POS;
    commands.entity(player_entity).insert((
        avian3d::prelude::Position(spawn_position),
        avian3d::prelude::LinearVelocity(Vec3::ZERO),
    ));

    let mut sender = senders
        .get_mut(client_entity)
        .expect("Client entity must have MessageSender<MapTransitionStart>");
    sender.send::<MapChannel>(MapTransitionStart {
        target: target_map_id.clone(),
        seed: params.seed,
        generation_version: params.generation_version,
        bounds: params.bounds,
        spawn_position,
    });
}

/// Returns the map entity and transition params. If the map already exists,
/// reads params from its `VoxelMapConfig`. If newly spawned, derives them
/// from the `MapInstanceId` (the entity isn't queryable yet via commands).
fn ensure_map_exists(
    commands: &mut Commands,
    map_id: &MapInstanceId,
    registry: &mut MapRegistry,
    config_query: &Query<&VoxelMapConfig>,
) -> (Entity, MapTransitionParams) {
    if let Some(&entity) = registry.0.get(map_id) {
        let config = config_query
            .get(entity)
            .expect("Existing map entity must have VoxelMapConfig");
        let params = MapTransitionParams {
            seed: config.seed,
            generation_version: config.generation_version,
            bounds: config.bounds,
        };
        return (entity, params);
    }

    match map_id {
        MapInstanceId::Overworld => {
            panic!("Overworld must already be registered in MapRegistry");
        }
        MapInstanceId::Homebase { owner } => {
            let bounds = IVec3::new(4, 4, 4);
            let (instance, config, marker) =
                VoxelMapInstance::homebase(*owner, bounds, Arc::new(flat_terrain_voxels));
            let params = MapTransitionParams {
                seed: config.seed,
                generation_version: config.generation_version,
                bounds: config.bounds,
            };
            let entity = commands
                .spawn((
                    instance,
                    config,
                    marker,
                    Transform::default(),
                    map_id.clone(),
                ))
                .id();
            registry.insert(map_id.clone(), entity);
            info!("Spawned server homebase for owner {owner}: {entity:?}");
            (entity, params)
        }
    }
}

pub fn handle_map_transition_ready(
    mut commands: Commands,
    mut receivers: Query<(Entity, &mut MessageReceiver<MapTransitionReady>)>,
    mut end_senders: Query<&mut MessageSender<MapTransitionEnd>>,
    controlled_query: Query<
        (Entity, &ControlledBy),
        (With<CharacterMarker>, With<PendingTransition>),
    >,
) {
    for (client_entity, mut receiver) in &mut receivers {
        for _ready in receiver.receive() {
            let Some((player_entity, _)) = controlled_query
                .iter()
                .find(|(_, ctrl)| ctrl.owner == client_entity)
            else {
                warn!("Received MapTransitionReady but no transitioning player for client {client_entity:?}");
                continue;
            };

            info!("Client confirmed transition ready for player {player_entity:?}, unfreezing");

            commands.entity(player_entity).remove::<(
                RigidBodyDisabled,
                ColliderDisabled,
                DisableRollback,
                PendingTransition,
            )>();

            let mut sender = end_senders
                .get_mut(client_entity)
                .expect("Client entity must have MessageSender<MapTransitionEnd>");
            sender.send::<MapChannel>(MapTransitionEnd);
        }
    }
}
