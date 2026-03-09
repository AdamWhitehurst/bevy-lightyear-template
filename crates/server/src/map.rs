use std::collections::HashMap;
use std::sync::Arc;

use avian3d::prelude::{ColliderDisabled, RigidBodyDisabled};
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
use serde::{Deserialize, Serialize};
use voxel_map_engine::prelude::{
    flat_terrain_voxels, ChunkTarget, VoxelMapConfig, VoxelMapInstance, VoxelPlugin, VoxelWorld,
    WorldVoxel,
};

/// Plugin managing server-side voxel map functionality
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

pub fn spawn_overworld(
    mut commands: Commands,
    map_world: Res<MapWorld>,
    mut registry: ResMut<MapRegistry>,
) {
    let map = commands
        .spawn((
            VoxelMapInstance::new(5),
            VoxelMapConfig::new(map_world.seed, 2, None, 5, Arc::new(flat_terrain_voxels)),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();
    commands.insert_resource(OverworldMap(map));
    registry.insert(MapInstanceId::Overworld, map);
}

fn load_voxel_world(
    mut modifications: ResMut<VoxelModifications>,
    map_world: Res<MapWorld>,
    save_path: Res<VoxelSavePath>,
    overworld: Res<OverworldMap>,
    mut voxel_world: VoxelWorld,
) {
    let loaded_mods = load_voxel_world_from_disk_at(&*map_world, &save_path.0);

    if loaded_mods.is_empty() {
        return;
    }

    modifications.modifications = loaded_mods.clone();

    for &(pos, voxel_type) in &loaded_mods {
        voxel_world.set_voxel(overworld.0, pos, WorldVoxel::from(voxel_type));
    }

    info!("Loaded {} voxel modifications", loaded_mods.len());
}

fn save_voxel_world_debounced(
    modifications: Res<VoxelModifications>,
    map_world: Res<MapWorld>,
    mut dirty_state: ResMut<VoxelDirtyState>,
    save_path: Res<VoxelSavePath>,
    time: Res<Time>,
) {
    if !dirty_state.is_dirty {
        return;
    }

    let now = time.elapsed_secs_f64();
    let time_since_edit = now - dirty_state.last_edit_time;
    let time_since_first_dirty = dirty_state.first_dirty_time.map(|t| now - t).unwrap_or(0.0);

    let should_save =
        time_since_edit >= SAVE_DEBOUNCE_SECONDS || time_since_first_dirty >= MAX_DIRTY_SECONDS;

    if should_save {
        if let Err(e) =
            save_voxel_world_to_disk_at(&modifications.modifications, &*map_world, &save_path.0)
        {
            error!("Failed to save voxel world: {}", e);
        }

        dirty_state.is_dirty = false;
        dirty_state.first_dirty_time = None;
    }
}

pub fn save_voxel_world_on_shutdown(
    mut exit_reader: MessageReader<AppExit>,
    modifications: Res<VoxelModifications>,
    map_world: Res<MapWorld>,
    save_path: Res<VoxelSavePath>,
    dirty_state: Res<VoxelDirtyState>,
) {
    if exit_reader.is_empty() {
        return;
    }
    exit_reader.clear();

    if dirty_state.is_dirty {
        info!("Saving voxel world on shutdown...");
        if let Err(e) =
            save_voxel_world_to_disk_at(&modifications.modifications, &*map_world, &save_path.0)
        {
            error!("Failed to save voxel world on shutdown: {}", e);
        }
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
            .init_resource::<MapWorld>()
            .init_resource::<MapRegistry>()
            .init_resource::<RoomRegistry>()
            .init_resource::<VoxelModifications>()
            .init_resource::<VoxelDirtyState>()
            .init_resource::<VoxelSavePath>()
            .add_systems(Startup, (spawn_overworld, load_voxel_world).chain())
            .add_systems(
                Update,
                (
                    handle_voxel_edit_requests,
                    handle_map_switch_requests,
                    handle_map_transition_ready,
                    protocol::attach_chunk_colliders,
                ),
            )
            .add_systems(Update, save_voxel_world_debounced)
            .add_systems(Last, save_voxel_world_on_shutdown)
            .add_observer(send_initial_voxel_state)
            .add_observer(on_map_instance_id_added);
    }
}

/// Tracks all voxel modifications for state sync
#[derive(Resource, Default)]
pub struct VoxelModifications {
    pub modifications: Vec<(IVec3, VoxelType)>,
}

#[derive(Resource)]
pub struct VoxelDirtyState {
    pub is_dirty: bool,
    pub last_edit_time: f64,
    pub first_dirty_time: Option<f64>,
}

impl Default for VoxelDirtyState {
    fn default() -> Self {
        Self {
            is_dirty: false,
            last_edit_time: 0.0,
            first_dirty_time: None,
        }
    }
}

const SAVE_DEBOUNCE_SECONDS: f64 = 1.0;
const MAX_DIRTY_SECONDS: f64 = 5.0;

#[derive(Serialize, Deserialize)]
struct VoxelWorldSave {
    version: u32,
    generation_seed: u64,
    generation_version: u32,
    modifications: Vec<(IVec3, VoxelType)>,
}

const SAVE_VERSION: u32 = 1;
const DEFAULT_SAVE_PATH: &str = "world_save/voxel_world.bin";

#[derive(Resource)]
pub struct VoxelSavePath(pub String);

impl Default for VoxelSavePath {
    fn default() -> Self {
        Self(DEFAULT_SAVE_PATH.to_string())
    }
}

pub fn save_voxel_world_to_disk_at(
    modifications: &[(IVec3, VoxelType)],
    map_world: &MapWorld,
    path: &str,
) -> std::io::Result<()> {
    use std::fs;
    use std::path::Path;

    let save_data = VoxelWorldSave {
        version: SAVE_VERSION,
        generation_seed: map_world.seed,
        generation_version: map_world.generation_version,
        modifications: modifications.to_vec(),
    };

    // Create directory if it doesn't exist
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent)?;
    }

    // Serialize to bytes
    let bytes = bincode::serialize(&save_data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    // Atomic write: temp file + rename
    let temp_path = format!("{}.tmp", path);
    fs::write(&temp_path, bytes)?;
    fs::rename(temp_path, path)?;

    info!(
        "Saved {} voxel modifications to {}",
        modifications.len(),
        path
    );
    Ok(())
}

pub fn load_voxel_world_from_disk_at(
    map_world: &MapWorld,
    save_path: &str,
) -> Vec<(IVec3, VoxelType)> {
    use std::fs;
    use std::path::Path;

    let path = Path::new(save_path);

    // File doesn't exist - normal for first run
    if !path.exists() {
        info!(
            "No save file found at {}, starting with empty world",
            save_path
        );
        return Vec::new();
    }

    // Read file
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            error!("Error reading save file: {}, starting with empty world", e);
            return Vec::new();
        }
    };

    // Deserialize
    let save_data: VoxelWorldSave = match bincode::deserialize(&bytes) {
        Ok(data) => data,
        Err(e) => {
            error!("Error deserializing save file: {}", e);
            // Backup corrupt file
            let backup_path = format!("{}.corrupt", save_path);
            if let Err(e) = fs::rename(path, &backup_path) {
                error!("Failed to backup corrupt file: {}", e);
            } else {
                info!("Backed up corrupt file to {}", backup_path);
            }
            info!("Starting with empty world");
            return Vec::new();
        }
    };

    // Check save file version
    if save_data.version != SAVE_VERSION {
        warn!(
            "Save file version mismatch (expected {}, got {}), starting with empty world",
            SAVE_VERSION, save_data.version
        );
        return Vec::new();
    }

    // Check generation compatibility
    if save_data.generation_seed != map_world.seed {
        warn!(
            "Save file generation seed mismatch (saved: {}, current: {})",
            save_data.generation_seed, map_world.seed
        );
        warn!("Modifications may not align with current procedural terrain!");
        warn!("Starting with empty world to avoid inconsistencies");
        return Vec::new();
    }

    if save_data.generation_version != map_world.generation_version {
        warn!(
            "Generation algorithm version mismatch (saved: {}, current: {})",
            save_data.generation_version, map_world.generation_version
        );
        warn!("Modifications may not align with current procedural terrain!");
        warn!("Starting with empty world to avoid inconsistencies");
        return Vec::new();
    }

    info!(
        "Loaded {} voxel modifications from {}",
        save_data.modifications.len(),
        save_path
    );
    save_data.modifications
}

fn handle_voxel_edit_requests(
    mut receiver: Query<&mut MessageReceiver<VoxelEditRequest>>,
    mut sender: ServerMultiMessageSender,
    server: Single<&Server>,
    mut modifications: ResMut<VoxelModifications>,
    mut dirty_state: ResMut<VoxelDirtyState>,
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

/// System to send initial state to newly connected clients
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

    let spawn_position = Vec3::new(0.0, 5.0, 0.0);
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
