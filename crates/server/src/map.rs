use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use avian3d::prelude::{ColliderDisabled, Position, RigidBodyDisabled};
use bevy::app::AppExit;
use bevy::prelude::*;
use lightyear::prelude::{
    Connected, ControlledBy, DisableRollback, MessageReceiver, MessageSender, NetworkTarget,
    RemoteId, Room, RoomEvent, RoomTarget, Server, ServerMultiMessageSender,
};
use protocol::map::{
    MapChannel, MapSwitchTarget, MapTransitionEnd, MapTransitionReady, MapTransitionStart,
    PlayerMapSwitchRequest,
};
use protocol::{
    CharacterMarker, MapInstanceId, MapRegistry, MapWorld, PendingTransition, VoxelChannel,
    VoxelEditBroadcast, VoxelEditRequest, VoxelStateSync, VoxelType,
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

/// Tracks all voxel modifications for state sync (kept until Phase 5).
#[derive(Resource, Default)]
pub struct VoxelModifications {
    pub modifications: Vec<(IVec3, VoxelType)>,
}

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
    commands.trigger(RoomEvent {
        room,
        target: RoomTarget::AddEntity(entity),
    });
}

impl Plugin for ServerMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(lightyear::prelude::RoomPlugin)
            .add_plugins(VoxelPlugin)
            .init_resource::<MapWorld>() // Keep until Phase 5
            .init_resource::<MapRegistry>()
            .init_resource::<RoomRegistry>()
            .init_resource::<VoxelModifications>() // Keep until Phase 5
            .init_resource::<WorldDirtyState>()
            .init_resource::<WorldSavePath>()
            .add_systems(Startup, (spawn_overworld, load_startup_entities).chain())
            .add_systems(
                Update,
                (
                    handle_voxel_edit_requests,
                    save_dirty_chunks_debounced,
                    handle_map_switch_requests,
                    handle_map_transition_ready,
                    protocol::attach_chunk_colliders,
                ),
            )
            .add_systems(Last, save_world_on_shutdown)
            .add_observer(send_initial_voxel_state)
            .add_observer(on_map_instance_id_added);
    }
}

fn handle_voxel_edit_requests(
    mut receiver: Query<&mut MessageReceiver<VoxelEditRequest>>,
    mut sender: ServerMultiMessageSender,
    server: Single<&Server>,
    mut modifications: ResMut<VoxelModifications>,
    mut dirty_state: ResMut<WorldDirtyState>,
    time: Res<Time>,
    overworld: Res<OverworldMap>,
    mut voxel_world: VoxelWorld,
) {
    let server_ref = server.into_inner();
    for mut message_receiver in receiver.iter_mut() {
        for request in message_receiver.receive() {
            voxel_world.set_voxel(
                overworld.0,
                request.position,
                WorldVoxel::from(request.voxel),
            );

            modifications
                .modifications
                .push((request.position, request.voxel));

            let now = time.elapsed_secs_f64();
            if !dirty_state.is_dirty {
                dirty_state.first_dirty_time = Some(now);
            }
            dirty_state.is_dirty = true;
            dirty_state.last_edit_time = now;

            sender
                .send::<_, VoxelChannel>(
                    &VoxelEditBroadcast {
                        position: request.position,
                        voxel: request.voxel,
                    },
                    server_ref,
                    &NetworkTarget::All,
                )
                .ok();
        }
    }
}

/// System to send initial state to newly connected clients.
fn send_initial_voxel_state(
    trigger: On<Add, Connected>,
    modifications: Res<VoxelModifications>,
    mut sender: Query<&mut MessageSender<VoxelStateSync>>,
) {
    let Ok(mut message_sender) = sender.get_mut(trigger.entity) else {
        return;
    };

    message_sender.send::<VoxelChannel>(VoxelStateSync {
        modifications: modifications.modifications.clone(),
    });
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
    map_world: Res<MapWorld>,
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
                &*map_world,
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

#[allow(clippy::too_many_arguments)]
fn execute_server_transition(
    commands: &mut Commands,
    player_entity: Entity,
    client_entity: Entity,
    current_map_id: &MapInstanceId,
    target_map_id: &MapInstanceId,
    registry: &mut MapRegistry,
    room_registry: &mut RoomRegistry,
    map_world: &MapWorld,
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

    let map_entity = ensure_map_exists(commands, target_map_id, registry, map_world);
    commands
        .entity(player_entity)
        .insert(ChunkTarget::new(map_entity, 4));

    let spawn_position = crate::gameplay::DEFAULT_SPAWN_POS;
    commands.entity(player_entity).insert((
        avian3d::prelude::Position(spawn_position),
        avian3d::prelude::LinearVelocity(Vec3::ZERO),
    ));

    let (seed, bounds) = match target_map_id {
        MapInstanceId::Overworld => (map_world.seed, None),
        MapInstanceId::Homebase { owner } => (*owner, Some(IVec3::new(4, 4, 4))),
    };

    let mut sender = senders
        .get_mut(client_entity)
        .expect("Client entity must have MessageSender<MapTransitionStart>");
    sender.send::<MapChannel>(MapTransitionStart {
        target: target_map_id.clone(),
        seed,
        generation_version: map_world.generation_version,
        bounds,
        spawn_position,
    });
}

fn ensure_map_exists(
    commands: &mut Commands,
    map_id: &MapInstanceId,
    registry: &mut MapRegistry,
    _map_world: &MapWorld,
) -> Entity {
    if let Some(&entity) = registry.0.get(map_id) {
        return entity;
    }

    match map_id {
        MapInstanceId::Overworld => {
            panic!("Overworld must already be registered in MapRegistry");
        }
        MapInstanceId::Homebase { owner } => {
            let bounds = IVec3::new(4, 4, 4);
            let (instance, config, marker) =
                VoxelMapInstance::homebase(*owner, bounds, Arc::new(flat_terrain_voxels));
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
            entity
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
