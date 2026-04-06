use std::collections::HashMap;

use avian3d::prelude::Position;
use bevy::prelude::*;
use bevy::tasks::AsyncComputeTaskPool;
use protocol::map::{ChunkEntityRef, MapInstanceId};
use protocol::vox_model::{VoxModelAsset, VoxModelRegistry};
use protocol::world_object::{
    ActiveTransformation, PlacementOffset, WorldObjectDefRegistry, WorldObjectId,
};
use voxel_map_engine::prelude::{
    chunk_to_column, PendingEntitySpawns, PersistedComponent, VoxelMapConfig, VoxelMapInstance,
    WorldObjectSpawn,
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
                let is_reload = !spawn.persisted_components.is_empty();
                let offset = extract_placement_offset(def, is_reload);
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
                let position = Vec3::from(spawn.position) + offset;
                commands.entity(entity).insert((
                    Position(position.into()),
                    ChunkEntityRef {
                        chunk_pos,
                        map_entity,
                    },
                ));

                if is_reload {
                    restore_persisted(
                        &mut commands,
                        entity,
                        &spawn.persisted_components,
                        def,
                        &defs,
                        &type_registry,
                        &vox_registry,
                        &vox_assets,
                        &meshes,
                    );
                }
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
    entity_query: Query<(
        Entity,
        &ChunkEntityRef,
        &WorldObjectId,
        &Position,
        Option<&ActiveTransformation>,
        Option<&protocol::Health>,
    )>,
    map_query: Query<(&VoxelMapInstance, &VoxelMapConfig)>,
) {
    let mut by_chunk: HashMap<(Entity, IVec3), Vec<(Entity, WorldObjectSpawn)>> = HashMap::new();

    for (entity, chunk_ref, obj_id, pos, active_transform, health) in &entity_query {
        let Ok((instance, _)) = map_query.get(chunk_ref.map_entity) else {
            continue;
        };
        let col = chunk_to_column(chunk_ref.chunk_pos);
        if instance.chunk_levels.contains_key(&col) {
            continue;
        }

        let persisted = serialize_persisted(active_transform, health);

        by_chunk
            .entry((chunk_ref.map_entity, chunk_ref.chunk_pos))
            .or_default()
            .push((
                entity,
                WorldObjectSpawn {
                    object_id: obj_id.0.clone(),
                    position: Vec3::from(pos.0),
                    persisted_components: persisted,
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
    entity_query: Query<(
        &ChunkEntityRef,
        &WorldObjectId,
        &Position,
        Option<&ActiveTransformation>,
        Option<&protocol::Health>,
    )>,
    map_query: Query<&VoxelMapConfig>,
) {
    if exit_reader.is_empty() {
        return;
    }
    exit_reader.clear();
    let mut by_chunk: HashMap<(Entity, IVec3), Vec<WorldObjectSpawn>> = HashMap::new();
    for (chunk_ref, obj_id, pos, active_transform, health) in &entity_query {
        by_chunk
            .entry((chunk_ref.map_entity, chunk_ref.chunk_pos))
            .or_default()
            .push(WorldObjectSpawn {
                object_id: obj_id.0.clone(),
                position: Vec3::from(pos.0),
                persisted_components: serialize_persisted(active_transform, health),
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

/// Serializes persistable components into `PersistedComponent` entries.
fn serialize_persisted(
    active_transform: Option<&ActiveTransformation>,
    health: Option<&protocol::Health>,
) -> Vec<PersistedComponent> {
    let mut result = Vec::new();
    if let Some(at) = active_transform {
        if let Ok(ron_data) = ron::to_string(at) {
            result.push(PersistedComponent {
                type_path: std::any::type_name::<ActiveTransformation>().to_string(),
                ron_data,
            });
        }
    }
    if let Some(h) = health {
        if let Ok(ron_data) = ron::to_string(h) {
            result.push(PersistedComponent {
                type_path: std::any::type_name::<protocol::Health>().to_string(),
                ron_data,
            });
        }
    }
    result
}

/// Restores persisted components on a reloaded entity.
///
/// If `ActiveTransformation` is persisted, applies the source def's components
/// (transforming the entity back to its transformed state).
#[allow(clippy::too_many_arguments)]
fn restore_persisted(
    commands: &mut Commands,
    entity: Entity,
    persisted: &[PersistedComponent],
    base_def: &protocol::world_object::WorldObjectDef,
    defs: &WorldObjectDefRegistry,
    type_registry: &AppTypeRegistry,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    meshes: &Assets<Mesh>,
) {
    let at_type = std::any::type_name::<ActiveTransformation>();
    let health_type = std::any::type_name::<protocol::Health>();

    let mut active_transform: Option<ActiveTransformation> = None;
    let mut persisted_health: Option<protocol::Health> = None;

    for pc in persisted {
        if pc.type_path == at_type {
            match ron::from_str::<ActiveTransformation>(&pc.ron_data) {
                Ok(at) => active_transform = Some(at),
                Err(e) => warn!("Failed to deserialize ActiveTransformation: {e}"),
            }
        } else if pc.type_path == health_type {
            match ron::from_str::<protocol::Health>(&pc.ron_data) {
                Ok(h) => persisted_health = Some(h),
                Err(e) => warn!("Failed to deserialize Health: {e}"),
            }
        }
    }

    if let Some(at) = active_transform {
        let source_id = WorldObjectId(at.source.clone());
        if let Some(source_def) = defs.get(&source_id) {
            crate::world_object::apply_transformation(
                commands,
                entity,
                base_def,
                source_def,
                type_registry,
                vox_registry,
                vox_assets,
                meshes,
            );
        }
        commands.entity(entity).insert(at);
    }

    if let Some(health) = persisted_health {
        commands.entity(entity).insert(health);
    }
}

/// Extracts `PlacementOffset` from a world object definition's reflected components.
///
/// Returns `Vec3::ZERO` if no `PlacementOffset` is present or if `is_reload` is true
/// (reloaded entities already have their final position).
fn extract_placement_offset(def: &protocol::world_object::WorldObjectDef, is_reload: bool) -> Vec3 {
    if is_reload {
        return Vec3::ZERO;
    }
    def.components
        .iter()
        .find_map(|c| c.try_downcast_ref::<PlacementOffset>())
        .map(|o| o.0)
        .unwrap_or(Vec3::ZERO)
}
