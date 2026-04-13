use std::collections::HashMap;

use avian3d::prelude::Position;
use bevy::prelude::*;
use persistence::{PendingStoreOps, StoreBackend};
use protocol::map::{ChunkEntityRef, MapInstanceId};
use protocol::vox_model::{VoxModelAsset, VoxModelRegistry};
use protocol::world_object::{
    ActiveTransformation, PlacementOffset, WorldObjectDefRegistry, WorldObjectId,
};
use voxel_map_engine::config::WorldObjectSpawn;
use voxel_map_engine::persistence::fs_chunk_entities::FsChunkEntitiesStore;
use voxel_map_engine::prelude::{
    chunk_to_column, PendingEntitySpawns, PersistedComponent, VoxelMapInstance,
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
        &mut PendingEntitySpawns,
        Option<&StoreBackend<IVec3, Vec<WorldObjectSpawn>, FsChunkEntitiesStore>>,
        Option<&mut PendingStoreOps<IVec3, Vec<WorldObjectSpawn>>>,
    )>,
    defs: Res<WorldObjectDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
) {
    for (map_entity, map_id, mut pending, store, mut ops) in &mut map_query {
        for (chunk_pos, spawns) in pending.0.drain(..) {
            if spawns.is_empty() {
                continue;
            }

            if let (Some(store), Some(ref mut ops)) = (&store, &mut ops) {
                ops.spawn_save(&store.0, chunk_pos, spawns.clone());
            }

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
    map_query: Query<&VoxelMapInstance>,
    mut store_query: Query<(
        &StoreBackend<IVec3, Vec<WorldObjectSpawn>, FsChunkEntitiesStore>,
        &mut PendingStoreOps<IVec3, Vec<WorldObjectSpawn>>,
    )>,
) {
    let mut by_chunk: HashMap<(Entity, IVec3), Vec<(Entity, WorldObjectSpawn)>> = HashMap::new();

    for (entity, chunk_ref, obj_id, pos, active_transform, health) in &entity_query {
        let Ok(instance) = map_query.get(chunk_ref.map_entity) else {
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
        let spawns: Vec<WorldObjectSpawn> = entities.iter().map(|(_, s)| s.clone()).collect();

        if let Ok((store, mut ops)) = store_query.get_mut(map_entity) {
            ops.spawn_save(&store.0, chunk_pos, spawns);
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
    mut store_query: Query<(
        &StoreBackend<IVec3, Vec<WorldObjectSpawn>, FsChunkEntitiesStore>,
        &mut PendingStoreOps<IVec3, Vec<WorldObjectSpawn>>,
    )>,
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
        let Ok((store, mut ops)) = store_query.get_mut(map_entity) else {
            continue;
        };
        ops.spawn_save(&store.0, chunk_pos, spawns);
    }
    for (_, mut ops) in &mut store_query {
        ops.flush();
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
