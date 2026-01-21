use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use bevy_voxel_world::prelude::*;
use lightyear::prelude::{
    Connected, MessageReceiver, MessageSender, NetworkTarget, Server, ServerMultiMessageSender,
};
use protocol::{
    MapWorld, VoxelChannel, VoxelEditBroadcast, VoxelEditRequest, VoxelStateSync, VoxelType,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Plugin managing server-side voxel map functionality
pub struct ServerMapPlugin;

fn load_voxel_world(
    mut voxel_world: VoxelWorld<MapWorld>,
    mut modifications: ResMut<VoxelModifications>,
    map_world: Res<MapWorld>,
) {
    let loaded_mods = load_voxel_world_from_disk(&map_world);

    if loaded_mods.is_empty() {
        return;
    }

    // Apply to VoxelModifications resource (for network sync)
    modifications.modifications = loaded_mods.clone();

    // Apply to VoxelWorld (populates bevy_voxel_world's internal ModifiedVoxels)
    for (pos, voxel_type) in &loaded_mods {
        voxel_world.set_voxel(*pos, (*voxel_type).into());
    }

    info!("Applied {} loaded modifications to voxel world", loaded_mods.len());
}

fn save_voxel_world_debounced(
    modifications: Res<VoxelModifications>,
    map_world: Res<MapWorld>,
    mut dirty_state: ResMut<VoxelDirtyState>,
    time: Res<Time>,
) {
    if !dirty_state.is_dirty {
        return;
    }

    let now = time.elapsed_secs_f64();
    let time_since_edit = now - dirty_state.last_edit_time;
    let time_since_first_dirty = dirty_state.first_dirty_time
        .map(|t| now - t)
        .unwrap_or(0.0);

    let should_save = time_since_edit >= SAVE_DEBOUNCE_SECONDS
        || time_since_first_dirty >= MAX_DIRTY_SECONDS;

    if should_save {
        if let Err(e) = save_voxel_world_to_disk(&modifications.modifications, &map_world) {
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
    dirty_state: Res<VoxelDirtyState>,
) {
    if exit_reader.is_empty() {
        return;
    }
    exit_reader.clear();

    if dirty_state.is_dirty {
        info!("Saving voxel world on shutdown...");
        if let Err(e) = save_voxel_world_to_disk(&modifications.modifications, &map_world) {
            error!("Failed to save voxel world on shutdown: {}", e);
        }
    }
}

impl Plugin for ServerMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelWorldPlugin::<MapWorld>::with_config(MapWorld::default()))
            .init_resource::<VoxelModifications>()
            .init_resource::<VoxelDirtyState>()
            .add_systems(Startup, load_voxel_world)
            .add_systems(
                Update,
                (
                    handle_voxel_edit_requests,
                    protocol::attach_chunk_colliders,
                    debug_server_chunks.run_if(on_timer(Duration::from_secs(5))),
                ),
            )
            .add_systems(Update, save_voxel_world_debounced)
            .add_systems(Last, save_voxel_world_on_shutdown)
            .add_observer(send_initial_voxel_state);
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
const SAVE_PATH: &str = "world_save/voxel_world.bin";

pub fn save_voxel_world_to_disk(
    modifications: &[(IVec3, VoxelType)],
    map_world: &MapWorld,
) -> std::io::Result<()> {
    save_voxel_world_to_disk_at(modifications, map_world, SAVE_PATH)
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

    info!("Saved {} voxel modifications to {}", modifications.len(), path);
    Ok(())
}

pub fn load_voxel_world_from_disk(
    map_world: &MapWorld,
) -> Vec<(IVec3, VoxelType)> {
    load_voxel_world_from_disk_at(map_world, SAVE_PATH)
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
        info!("No save file found at {}, starting with empty world", save_path);
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

    info!("Loaded {} voxel modifications from {}", save_data.modifications.len(), save_path);
    save_data.modifications
}

fn handle_voxel_edit_requests(
    mut receiver: Query<&mut MessageReceiver<VoxelEditRequest>>,
    mut sender: ServerMultiMessageSender,
    server: Single<&Server>,
    mut modifications: ResMut<VoxelModifications>,
    mut dirty_state: ResMut<VoxelDirtyState>,
    time: Res<Time>,
    mut voxel_world: VoxelWorld<MapWorld>,
    chunks: Query<(&Chunk<MapWorld>, &Transform, Option<&Mesh3d>)>,
    chunk_targets: Query<&Transform, With<ChunkRenderTarget<MapWorld>>>,
) {
    let server_ref = server.into_inner();
    for mut message_receiver in receiver.iter_mut() {
        for request in message_receiver.receive() {
            eprintln!("✓ Server received and processing voxel edit: {:?}", request);

            // DEBUG: Check chunk spawning
            let chunk_pos = (request.position.as_vec3() / 32.0).floor().as_ivec3();

            if let Some((_, transform, mesh)) =
                chunks.iter().find(|(c, _, _)| c.position == chunk_pos)
            {
                eprintln!(
                    "  → Chunk {:?} EXISTS at y={}, has_mesh={}",
                    chunk_pos,
                    transform.translation.y,
                    mesh.is_some()
                );
            } else {
                eprintln!("  → Chunk {:?} DOES NOT EXIST on server!", chunk_pos);

                // Check if ANY ChunkRenderTarget exists
                let target_count = chunk_targets.iter().count();
                eprintln!("  → ChunkRenderTarget count: {}", target_count);

                for (i, target_transform) in chunk_targets.iter().enumerate() {
                    let target_chunk = (target_transform.translation / 32.0).floor().as_ivec3();
                    let dist = chunk_pos.distance_squared(target_chunk);
                    eprintln!(
                        "  → Target {}: at chunk {:?}, distance_sq={}",
                        i, target_chunk, dist
                    );
                }
            }

            // TODO: Add admin permission check here

            // Apply voxel change
            voxel_world.set_voxel(request.position, request.voxel.into());

            // Track modification
            modifications
                .modifications
                .push((request.position, request.voxel));

            // Mark dirty
            let now = time.elapsed_secs_f64();
            if !dirty_state.is_dirty {
                dirty_state.first_dirty_time = Some(now);
            }
            dirty_state.is_dirty = true;
            dirty_state.last_edit_time = now;

            // Broadcast to all clients
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

/// Debug system to monitor server chunk spawning
fn debug_server_chunks(chunks: Query<(&Chunk<MapWorld>, &Transform, Option<&Mesh3d>)>) {
    let total = chunks.iter().count();
    let with_mesh = chunks.iter().filter(|(_, _, m)| m.is_some()).count();
    let above_ground = chunks
        .iter()
        .filter(|(_, t, _)| t.translation.y > 0.0)
        .count();

    eprintln!(
        "Server chunks: total={}, with_mesh={}, above_ground={}",
        total, with_mesh, above_ground
    );
}
