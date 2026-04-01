use std::collections::{HashMap, HashSet};
use std::path::Path;

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
    CharacterMarker, ChunkChannel, ChunkDataSync, MapInstanceId, MapRegistry, PendingTransition,
    SectionBlocksUpdate, UnloadColumn, VoxelChannel, VoxelEditAck, VoxelEditBroadcast,
    VoxelEditReject, VoxelEditRequest, VoxelType,
};
#[allow(unused_imports)]
use tracy_client::plot;
use voxel_map_engine::lifecycle::{self, PendingSaves};
use voxel_map_engine::prelude::{
    build_generator, seed_from_id, ChunkTicket, VoxelGenerator, VoxelMapConfig, VoxelMapInstance,
    VoxelPlugin, VoxelWorld, WorldVoxel,
};

use crate::persistence::{
    load_entities, load_map_meta, map_save_dir, save_entities, save_map_meta, MapMeta,
    WorldSavePath,
};
use protocol::map::{MapSaveTarget, SavedEntity, SavedEntityKind};
use protocol::vox_model::VoxModelRegistry;
use protocol::world_object::{apply_object_components, WorldObjectDefRegistry};
use protocol::{RespawnPoint, TerrainDefRegistry};
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
            trace!("Created room for map {id:?}: {room:?}");
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

    let mut config = VoxelMapConfig::new(seed, generation_version, 2, None, 5);
    config.save_dir = Some(map_dir);

    let map = commands
        .spawn((
            VoxelMapInstance::new(5),
            config,
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();

    // Terrain components applied later by apply_terrain_defs (when TerrainDefRegistry is loaded).
    // VoxelGenerator is then built by build_terrain_generators.

    commands.insert_resource(OverworldMap(map));
    registry.insert(MapInstanceId::Overworld, map);
}

/// Marker indicating terrain definition components have been applied to this map entity.
#[derive(Component)]
struct TerrainDefApplied;

/// Applies terrain definition components from `TerrainDefRegistry` to map entities.
///
/// Waits for `TerrainDefRegistry` to be loaded (async asset pipeline), then applies
/// terrain components from the matching `.terrain.ron` file onto each map entity.
fn apply_terrain_defs(
    mut commands: Commands,
    query: Query<(Entity, &MapInstanceId), (With<VoxelMapInstance>, Without<TerrainDefApplied>)>,
    terrain_registry: Res<TerrainDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
) {
    for (entity, map_id) in &query {
        let def_name = terrain_def_name(map_id);
        if let Some(terrain_def) = terrain_registry.get(&def_name) {
            let components = clone_terrain_components(terrain_def);
            apply_object_components(&mut commands, entity, components, type_registry.0.clone());
        }
        commands.entity(entity).insert(TerrainDefApplied);
        trace!("Applied terrain def '{def_name}' to map entity {entity:?}");
    }
}

/// Maps a `MapInstanceId` to its terrain definition name.
fn terrain_def_name(map_id: &MapInstanceId) -> String {
    match map_id {
        MapInstanceId::Overworld => "overworld".to_string(),
        MapInstanceId::Homebase { .. } => "homebase".to_string(),
    }
}

/// Clone terrain definition components via `reflect_clone`.
fn clone_terrain_components(
    def: &protocol::terrain::TerrainDef,
) -> Vec<Box<dyn bevy::reflect::PartialReflect>> {
    def.components
        .iter()
        .map(|c| {
            c.reflect_clone()
                .expect("terrain component must be cloneable")
                .into_partial_reflect()
        })
        .collect()
}

/// Builds `VoxelGenerator` for map entities whose terrain components have been flushed.
///
/// Exclusive system: needs `&mut World` to pass `EntityRef` to `build_generator`,
/// keeping that function extensible to new terrain components without signature changes.
fn build_terrain_generators(world: &mut World) {
    let mut query = world.query_filtered::<(Entity, &VoxelMapConfig), (
        With<VoxelMapInstance>,
        With<TerrainDefApplied>,
        Without<VoxelGenerator>,
    )>();
    let entities: Vec<(Entity, u64)> = query
        .iter(world)
        .map(|(e, config)| (e, config.seed))
        .collect();

    for (entity, seed) in entities {
        let entity_ref = world.entity(entity);
        let generator = build_generator(entity_ref, seed);
        world.entity_mut(entity).insert(generator);
        trace!("Built terrain generator for map entity {entity:?}");
    }
}

fn save_dirty_chunks_debounced(
    time: Res<Time>,
    mut dirty_state: ResMut<WorldDirtyState>,
    save_path: Res<WorldSavePath>,
    mut map_query: Query<(
        &mut VoxelMapInstance,
        &VoxelMapConfig,
        &MapInstanceId,
        &mut PendingSaves,
    )>,
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

    for (mut instance, config, map_id, mut pending_saves) in &mut map_query {
        let Some(map_dir) = config.save_dir.as_deref() else {
            trace!("save_dirty_chunks_debounced: no save_dir for {map_id:?}, skipping");
            continue;
        };

        enqueue_dirty_chunks(&mut instance, &mut pending_saves, map_dir);

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

/// Drain dirty chunks from an instance into the async `PendingSaves` queue.
pub fn enqueue_dirty_chunks(
    instance: &mut VoxelMapInstance,
    pending_saves: &mut PendingSaves,
    map_dir: &Path,
) {
    let dirty: Vec<IVec3> = instance.dirty_chunks.drain().collect();
    for chunk_pos in dirty {
        if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
            pending_saves.enqueue(chunk_pos, chunk_data.clone(), map_dir.to_path_buf());
        }
    }
}

/// Synchronously flush all dirty chunks to disk. Used only during shutdown
/// where we must guarantee persistence before the process exits.
pub fn save_dirty_chunks_sync(instance: &mut VoxelMapInstance, map_dir: &Path) {
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
        save_dirty_chunks_sync(&mut instance, map_dir);

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
            .insert_resource(voxel_map_engine::ChunkGenerationEnabled)
            .init_resource::<MapRegistry>()
            .init_resource::<RoomRegistry>()
            .init_resource::<WorldDirtyState>()
            .init_resource::<PendingVoxelBroadcasts>()
            .init_resource::<WorldSavePath>()
            .add_systems(Startup, (spawn_overworld, load_startup_entities).chain())
            .add_systems(
                Update,
                (
                    apply_terrain_defs.run_if(resource_exists::<TerrainDefRegistry>),
                    ApplyDeferred,
                    build_terrain_generators,
                )
                    .chain()
                    .before(lifecycle::ensure_pending_chunks),
            )
            .add_systems(
                Update,
                (
                    (handle_voxel_edit_requests, flush_voxel_broadcasts).chain(),
                    push_chunks_to_clients,
                    save_dirty_chunks_debounced,
                    handle_map_switch_requests,
                    handle_map_transition_ready,
                    protocol::attach_chunk_colliders,
                    crate::chunk_entities::spawn_chunk_entities
                        .after(lifecycle::poll_chunk_tasks)
                        .run_if(
                            resource_exists::<WorldObjectDefRegistry>
                                .and(resource_exists::<VoxModelRegistry>),
                        ),
                    crate::chunk_entities::evict_chunk_entities
                        .after(lifecycle::despawn_out_of_range_chunks),
                ),
            )
            .add_systems(
                Last,
                (
                    save_world_on_shutdown,
                    crate::chunk_entities::save_all_chunk_entities_on_exit,
                ),
            )
            .add_observer(on_map_instance_id_added);
    }
}

/// Resolves which map entity a client's character is on.
fn resolve_player_map(
    client_entity: Entity,
    controlled_query: &Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: &MapRegistry,
) -> Option<(Entity, MapInstanceId)> {
    let (_, player_map_id) = controlled_query
        .iter()
        .find(|(ctrl, _)| ctrl.owner == client_entity)?;
    Some((map_registry.get(player_map_id), player_map_id.clone()))
}

/// Validates the edit and sends a reject if invalid. Returns `true` if edit is valid.
fn is_edit_valid(
    request: &VoxelEditRequest,
    map_entity: Entity,
    client_entity: Entity,
    voxel_world: &VoxelWorld,
    reject_senders: &mut Query<&mut MessageSender<VoxelEditReject>>,
) -> bool {
    if validate_voxel_edit(request, map_entity, voxel_world) {
        return true;
    }
    let current_voxel = voxel_world.get_voxel(map_entity, request.position);
    if let Ok(mut sender) = reject_senders.get_mut(client_entity) {
        sender.send::<VoxelChannel>(VoxelEditReject {
            sequence: request.sequence,
            position: request.position,
            correct_voxel: current_voxel.into(),
        });
    }
    false
}

/// Applies the voxel edit and marks the world dirty.
fn apply_voxel_edit(
    request: &VoxelEditRequest,
    map_entity: Entity,
    voxel_world: &mut VoxelWorld,
    dirty_state: &mut WorldDirtyState,
    time: &Time,
) {
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
}

/// Sends an edit acknowledgment to the originating client.
fn send_edit_ack(
    client_entity: Entity,
    sequence: u32,
    ack_senders: &mut Query<&mut MessageSender<VoxelEditAck>>,
) {
    if let Ok(mut sender) = ack_senders.get_mut(client_entity) {
        sender.send::<VoxelChannel>(VoxelEditAck { sequence });
    } else {
        warn!("send_edit_ack: no ack sender for {client_entity:?}");
    }
}

/// Queues a voxel edit for batched broadcast.
fn queue_edit_broadcast(edit: PendingVoxelEdit, pending: &mut PendingVoxelBroadcasts) {
    let chunk_pos = voxel_map_engine::prelude::voxel_to_chunk_pos(edit.position);
    pending.per_chunk.entry(chunk_pos).or_default().push(edit);
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
            let Some((map_entity, player_map_id)) =
                resolve_player_map(client_entity, &controlled_query, &*map_registry)
            else {
                trace!("handle_voxel_edit_requests: no character for client {client_entity:?}");
                continue;
            };

            if !is_edit_valid(
                &request,
                map_entity,
                client_entity,
                &voxel_world,
                &mut reject_senders,
            ) {
                continue;
            }

            apply_voxel_edit(
                &request,
                map_entity,
                &mut voxel_world,
                &mut *dirty_state,
                &*time,
            );
            send_edit_ack(client_entity, request.sequence, &mut ack_senders);
            queue_edit_broadcast(
                PendingVoxelEdit {
                    position: request.position,
                    voxel: request.voxel,
                    originator: client_entity,
                    map_id: player_map_id,
                },
                &mut *pending_broadcasts,
            );
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

/// Drains accumulated voxel edits and broadcasts them to clients in the same room.
/// Single edits send individual `VoxelEditBroadcast`; 2+ edits in the same chunk
/// send a batched `SectionBlocksUpdate`. The originating client is excluded.
pub fn flush_voxel_broadcasts(
    mut pending: ResMut<PendingVoxelBroadcasts>,
    mut sender: ServerMultiMessageSender,
    room_registry: Res<RoomRegistry>,
    rooms: Query<&Room>,
) {
    if pending.per_chunk.is_empty() {
        return;
    }

    for (chunk_pos, edits) in pending.per_chunk.drain() {
        let Some(first) = edits.first() else {
            continue;
        };
        let Some(&room_entity) = room_registry.0.get(&first.map_id) else {
            warn!("flush_voxel_broadcasts: no room for map {:?}", first.map_id);
            continue;
        };
        let Ok(room) = rooms.get(room_entity) else {
            warn!("flush_voxel_broadcasts: room entity {room_entity:?} has no Room component");
            continue;
        };

        let originators: bevy::ecs::entity::EntityHashSet =
            edits.iter().map(|e| e.originator).collect();
        let targets: bevy::ecs::entity::EntityHashSet = room
            .clients
            .iter()
            .filter(|e| !originators.contains(*e))
            .copied()
            .collect();

        if edits.len() == 1 {
            let edit = &edits[0];
            sender
                .send_to_entities::<_, VoxelChannel>(
                    &VoxelEditBroadcast {
                        position: edit.position,
                        voxel: edit.voxel,
                    },
                    &targets,
                )
                .ok();
        } else {
            let changes: Vec<(IVec3, VoxelType)> =
                edits.iter().map(|e| (e.position, e.voxel)).collect();
            sender
                .send_to_entities::<_, VoxelChannel>(
                    &SectionBlocksUpdate { chunk_pos, changes },
                    &targets,
                )
                .ok();
        }
    }
}

/// Per-player tracking of which chunks have been sent to the client.
#[derive(Component, Default)]
pub struct ClientChunkVisibility {
    /// Individual chunks (IVec3) whose data has been sent.
    sent_chunks: HashSet<IVec3>,
    /// Columns the client believes are loaded (for sending UnloadColumn).
    sent_columns: HashSet<IVec2>,
    /// The map entity these tracking sets are scoped to. Reset when the
    /// player's ticket switches maps (e.g. map transition).
    tracked_map: Option<Entity>,
}

/// Maximum chunk data messages sent to a single client per tick.
const MAX_CHUNK_SENDS_PER_TICK: usize = 16;

/// Server system: for each connected player, compare their ticket's loaded columns
/// against what we've already sent. Push new chunks (throttled, closest first),
/// send unload for removed.
pub fn push_chunks_to_clients(
    mut player_query: Query<(
        &ChunkTicket,
        &ControlledBy,
        &Position,
        &mut ClientChunkVisibility,
    )>,
    map_query: Query<(&VoxelMapInstance, &MapInstanceId)>,
    mut senders: Query<&mut MessageSender<ChunkDataSync>>,
    mut multi_sender: ServerMultiMessageSender,
) {
    for (ticket, controlled_by, pos, mut visibility) in &mut player_query {
        if visibility.tracked_map != Some(ticket.map_entity) {
            visibility.sent_chunks.clear();
            visibility.sent_columns.clear();
            visibility.tracked_map = Some(ticket.map_entity);
        }

        let Ok((instance, map_id)) = map_query.get(ticket.map_entity) else {
            trace!(
                "push_chunks_to_clients: map entity {:?} not found",
                ticket.map_entity
            );
            continue;
        };

        let player_col = voxel_map_engine::lifecycle::world_to_column_pos(pos.0);
        let current_columns = compute_loaded_columns(ticket, instance, player_col);
        let client_entity = controlled_by.owner;

        let sent = send_unsent_chunks(
            &current_columns,
            &mut visibility,
            instance,
            map_id,
            player_col,
            client_entity,
            &mut senders,
        );
        plot!("chunks_sent_this_tick", sent as f64);

        unload_stale_columns(
            &mut visibility,
            &current_columns,
            map_id,
            client_entity,
            &mut multi_sender,
        );

        visibility.sent_columns = current_columns;
    }
}

/// Computes which columns are currently in the player's loaded range.
fn compute_loaded_columns(
    ticket: &ChunkTicket,
    instance: &VoxelMapInstance,
    player_col: IVec2,
) -> HashSet<IVec2> {
    let radius = ticket.radius as i32;
    let mut columns = HashSet::new();
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            let col = player_col + IVec2::new(dx, dz);
            let distance = dx.abs().max(dz.abs()) as u32;
            let level = ticket.ticket_type.base_level() + distance;
            if level > voxel_map_engine::prelude::LOAD_LEVEL_THRESHOLD {
                continue;
            }
            if instance.chunk_levels.contains_key(&col) {
                columns.insert(col);
            }
        }
    }
    columns
}

/// Sends up to `MAX_CHUNK_SENDS_PER_TICK` unsent chunks, closest to player first.
/// Returns the number of chunks sent.
fn send_unsent_chunks(
    current_columns: &HashSet<IVec2>,
    visibility: &mut ClientChunkVisibility,
    instance: &VoxelMapInstance,
    map_id: &MapInstanceId,
    player_col: IVec2,
    client_entity: Entity,
    senders: &mut Query<&mut MessageSender<ChunkDataSync>>,
) -> usize {
    let mut candidates: Vec<(IVec3, u32)> = Vec::new();
    for &col in current_columns {
        let dist = (col.x - player_col.x)
            .abs()
            .max((col.y - player_col.y).abs()) as u32;
        for chunk_pos in voxel_map_engine::prelude::column_to_chunks(
            col,
            voxel_map_engine::prelude::DEFAULT_COLUMN_Y_MIN,
            voxel_map_engine::prelude::DEFAULT_COLUMN_Y_MAX,
        ) {
            if visibility.sent_chunks.contains(&chunk_pos) {
                continue;
            }
            if instance.get_chunk_data(chunk_pos).is_none() {
                continue;
            }
            candidates.push((chunk_pos, dist));
        }
    }
    candidates.sort_unstable_by_key(|&(_, dist)| dist);

    let mut sent = 0;
    for (chunk_pos, _) in candidates {
        if sent >= MAX_CHUNK_SENDS_PER_TICK {
            break;
        }
        let Some(chunk_data) = instance.get_chunk_data(chunk_pos) else {
            continue;
        };
        if let Ok(mut sender) = senders.get_mut(client_entity) {
            sender.send::<ChunkChannel>(ChunkDataSync {
                map_id: map_id.clone(),
                chunk_pos,
                data: chunk_data.voxels.clone(),
            });
        }
        visibility.sent_chunks.insert(chunk_pos);
        sent += 1;
    }
    sent
}

/// Sends `UnloadColumn` messages for columns that left the player's loaded range.
fn unload_stale_columns(
    visibility: &mut ClientChunkVisibility,
    current_columns: &HashSet<IVec2>,
    map_id: &MapInstanceId,
    client_entity: Entity,
    multi_sender: &mut ServerMultiMessageSender,
) {
    let unloaded_cols: Vec<IVec2> = visibility
        .sent_columns
        .difference(current_columns)
        .copied()
        .collect();
    if !unloaded_cols.is_empty() {
        let targets: bevy::ecs::entity::EntityHashSet = [client_entity].into_iter().collect();
        for &col in &unloaded_cols {
            multi_sender
                .send_to_entities::<_, ChunkChannel>(
                    &UnloadColumn {
                        map_id: map_id.clone(),
                        column: col,
                    },
                    &targets,
                )
                .ok();
            for chunk_pos in voxel_map_engine::prelude::column_to_chunks(
                col,
                voxel_map_engine::prelude::DEFAULT_COLUMN_Y_MIN,
                voxel_map_engine::prelude::DEFAULT_COLUMN_Y_MAX,
            ) {
                visibility.sent_chunks.remove(&chunk_pos);
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
    save_path: Res<WorldSavePath>,
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
                &*save_path,
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
    save_path: &WorldSavePath,
) {
    trace!("Transitioning player {player_entity:?} from {current_map_id:?} to {target_map_id:?}");

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

    let (map_entity, params) =
        ensure_map_exists(commands, target_map_id, registry, config_query, save_path);
    commands
        .entity(player_entity)
        .insert(ChunkTicket::player(map_entity));

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
    save_path: &WorldSavePath,
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
            let (entity, params) = spawn_homebase(commands, *owner, save_path, registry, map_id);
            (entity, params)
        }
    }
}

/// Spawns a new homebase map, loading seed and entities from disk if saved.
fn spawn_homebase(
    commands: &mut Commands,
    owner: u64,
    save_path: &WorldSavePath,
    registry: &mut MapRegistry,
    map_id: &MapInstanceId,
) -> (Entity, MapTransitionParams) {
    let map_dir = map_save_dir(&save_path.0, map_id);

    let seed = load_homebase_seed(&map_dir, owner);

    let bounds = IVec3::new(4, 4, 4);
    let (instance, mut config, marker) = VoxelMapInstance::homebase(owner, bounds);
    config.seed = seed;
    config.save_dir = Some(map_dir);

    let params = MapTransitionParams {
        seed: config.seed,
        generation_version: config.generation_version,
        bounds: config.bounds,
    };
    // No VoxelGenerator here — build_terrain_generators will add it next frame.
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

    let entity_count = load_map_entities(commands, save_path, map_id);
    if entity_count > 0 {
        trace!("Loaded {entity_count} entities for homebase-{owner}");
    }

    trace!("Spawned server homebase for owner {owner}: {entity:?}");
    (entity, params)
}

/// Loads the seed for a homebase from saved metadata, falling back to `seed_from_id`.
fn load_homebase_seed(map_dir: &Path, owner: u64) -> u64 {
    match load_map_meta(map_dir) {
        Ok(Some(meta)) => {
            trace!(
                "Loading homebase-{owner} from saved metadata (seed={})",
                meta.seed
            );
            meta.seed
        }
        _ => {
            let seed = seed_from_id(owner);
            trace!("Creating new homebase-{owner} (seed={seed})");
            seed
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

            trace!("Client confirmed transition ready for player {player_entity:?}, unfreezing");

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

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::map::VoxelType;

    fn make_edit(position: IVec3, voxel: VoxelType) -> PendingVoxelEdit {
        PendingVoxelEdit {
            position,
            voxel,
            originator: Entity::PLACEHOLDER,
            map_id: MapInstanceId::Overworld,
        }
    }

    #[test]
    fn single_change_takes_individual_broadcast_path() {
        let mut pending = PendingVoxelBroadcasts::default();
        pending
            .per_chunk
            .entry(IVec3::ZERO)
            .or_default()
            .push(make_edit(IVec3::new(1, 2, 3), VoxelType::Solid(1)));

        for (_, edits) in pending.per_chunk.drain() {
            assert_eq!(
                edits.len(),
                1,
                "single edit should take individual broadcast path"
            );
        }
    }

    #[test]
    fn multiple_changes_in_same_chunk_takes_batched_path() {
        let mut pending = PendingVoxelBroadcasts::default();
        let entry = pending.per_chunk.entry(IVec3::ZERO).or_default();
        entry.push(make_edit(IVec3::new(1, 2, 3), VoxelType::Solid(1)));
        entry.push(make_edit(IVec3::new(4, 5, 6), VoxelType::Air));

        for (_, edits) in pending.per_chunk.drain() {
            assert_eq!(edits.len(), 2, "multi-edit should take batched update path");
        }
    }

    #[test]
    fn different_chunks_produce_separate_entries() {
        let mut pending = PendingVoxelBroadcasts::default();
        pending
            .per_chunk
            .entry(IVec3::ZERO)
            .or_default()
            .push(make_edit(IVec3::new(1, 2, 3), VoxelType::Solid(1)));
        pending
            .per_chunk
            .entry(IVec3::ONE)
            .or_default()
            .push(make_edit(IVec3::new(17, 18, 19), VoxelType::Solid(2)));

        let chunks: Vec<_> = pending.per_chunk.drain().collect();
        assert_eq!(
            chunks.len(),
            2,
            "different chunks should produce separate entries"
        );
        for (_, edits) in &chunks {
            assert_eq!(edits.len(), 1);
        }
    }

    #[test]
    fn pending_cleared_after_drain() {
        let mut pending = PendingVoxelBroadcasts::default();
        pending
            .per_chunk
            .entry(IVec3::ZERO)
            .or_default()
            .push(make_edit(IVec3::new(1, 2, 3), VoxelType::Solid(1)));

        for _ in pending.per_chunk.drain() {}
        assert!(
            pending.per_chunk.is_empty(),
            "pending should be empty after drain"
        );
    }
}
