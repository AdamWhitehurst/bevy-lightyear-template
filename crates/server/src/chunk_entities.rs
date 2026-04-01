use std::collections::HashMap;

use avian3d::prelude::Position;
use bevy::prelude::*;
use bevy::tasks::AsyncComputeTaskPool;
use protocol::map::{ChunkEntityRef, MapInstanceId};
use protocol::vox_model::{VoxModelAsset, VoxModelRegistry};
use protocol::world_object::{WorldObjectDefRegistry, WorldObjectId};
use voxel_map_engine::prelude::{
    chunk_to_column, PendingEntitySpawns, VoxelMapConfig, VoxelMapInstance, WorldObjectSpawn,
};

use crate::world_object::spawn_world_object;

/// Spawns world objects from completed Features stages.
///
/// Drains `PendingEntitySpawns` and calls `spawn_world_object` for each entry,
/// tagging entities with `ChunkEntityRef` for lifecycle management. Also saves
/// newly generated entity data to disk (generate-once, save-forever).
pub fn spawn_chunk_entities(
    mut commands: Commands,
    mut map_query: Query<(
        Entity,
        &MapInstanceId,
        &VoxelMapConfig,
        &mut PendingEntitySpawns,
    )>,
    defs: Res<WorldObjectDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
) {
    for (map_entity, map_id, config, mut pending) in &mut map_query {
        for (chunk_pos, spawns) in pending.0.drain(..) {
            if spawns.is_empty() {
                continue;
            }

            save_new_chunk_entities(config, chunk_pos, &spawns);

            for spawn in &spawns {
                let id = WorldObjectId(spawn.object_id.clone());
                let Some(def) = defs.get(&id) else {
                    warn!(
                        "Unknown world object '{}' in placement rules",
                        spawn.object_id
                    );
                    continue;
                };
                let entity = spawn_world_object(
                    &mut commands,
                    id,
                    def,
                    map_id.clone(),
                    &type_registry,
                    &vox_registry,
                    &vox_assets,
                    &meshes,
                );
                commands.entity(entity).insert((
                    Position(spawn.position.into()),
                    ChunkEntityRef {
                        chunk_pos,
                        map_entity,
                    },
                ));
            }
        }
    }
}

/// Saves entity spawn data to disk asynchronously (fire-and-forget).
fn save_new_chunk_entities(config: &VoxelMapConfig, chunk_pos: IVec3, spawns: &[WorldObjectSpawn]) {
    let Some(ref dir) = config.save_dir else {
        return;
    };
    let dir = dir.clone();
    let spawns = spawns.to_vec();
    let pool = AsyncComputeTaskPool::get();
    pool.spawn(async move {
        if let Err(e) = voxel_map_engine::persistence::save_chunk_entities(&dir, chunk_pos, &spawns)
        {
            error!("Failed to save new chunk entities at {chunk_pos}: {e}");
        }
    })
    .detach();
}

/// Saves and despawns chunk entities when their chunk is evicted (column unloaded).
///
/// Checks each `ChunkEntityRef` entity — if its chunk's column is no longer in
/// `chunk_levels`, the entity is saved to disk and despawned.
pub fn evict_chunk_entities(
    mut commands: Commands,
    entity_query: Query<(Entity, &ChunkEntityRef, &WorldObjectId, &Position)>,
    map_query: Query<(&VoxelMapInstance, &VoxelMapConfig)>,
) {
    let mut by_chunk: HashMap<(Entity, IVec3), Vec<(Entity, WorldObjectSpawn)>> = HashMap::new();

    for (entity, chunk_ref, obj_id, pos) in &entity_query {
        let Ok((instance, _)) = map_query.get(chunk_ref.map_entity) else {
            continue;
        };
        let col = chunk_to_column(chunk_ref.chunk_pos);
        if instance.chunk_levels.contains_key(&col) {
            continue;
        }

        by_chunk
            .entry((chunk_ref.map_entity, chunk_ref.chunk_pos))
            .or_default()
            .push((
                entity,
                WorldObjectSpawn {
                    object_id: obj_id.0.clone(),
                    position: Vec3::from(pos.0),
                },
            ));
    }

    for ((map_entity, chunk_pos), entities) in by_chunk {
        let Ok((_, config)) = map_query.get(map_entity) else {
            continue;
        };

        let spawns: Vec<WorldObjectSpawn> = entities.iter().map(|(_, s)| s.clone()).collect();

        if let Some(ref dir) = config.save_dir {
            let dir = dir.clone();
            let pool = AsyncComputeTaskPool::get();
            pool.spawn(async move {
                if let Err(e) =
                    voxel_map_engine::persistence::save_chunk_entities(&dir, chunk_pos, &spawns)
                {
                    error!("Failed to save evicted chunk entities at {chunk_pos}: {e}");
                }
            })
            .detach();
        }

        for (entity, _) in entities {
            commands.entity(entity).despawn();
        }
    }
}

/// On server shutdown, saves entity files for all loaded chunks.
///
/// Ensures destroyed entities (no longer in the query) are excluded from
/// the saved file, maintaining the "generate once, save forever" invariant.
pub fn save_all_chunk_entities_on_exit(
    mut exit_reader: MessageReader<AppExit>,
    entity_query: Query<(&ChunkEntityRef, &WorldObjectId, &Position)>,
    map_query: Query<&VoxelMapConfig>,
) {
    if exit_reader.is_empty() {
        return;
    }
    exit_reader.clear();
    let mut by_chunk: HashMap<(Entity, IVec3), Vec<WorldObjectSpawn>> = HashMap::new();
    for (chunk_ref, obj_id, pos) in &entity_query {
        by_chunk
            .entry((chunk_ref.map_entity, chunk_ref.chunk_pos))
            .or_default()
            .push(WorldObjectSpawn {
                object_id: obj_id.0.clone(),
                position: Vec3::from(pos.0),
            });
    }
    for ((map_entity, chunk_pos), spawns) in by_chunk {
        let Ok(config) = map_query.get(map_entity) else {
            continue;
        };
        if let Some(ref dir) = config.save_dir {
            if let Err(e) =
                voxel_map_engine::persistence::save_chunk_entities(dir, chunk_pos, &spawns)
            {
                error!("Shutdown save failed for chunk {chunk_pos}: {e}");
            }
        }
    }
}
