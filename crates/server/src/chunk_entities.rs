use std::collections::HashMap;

use avian3d::prelude::Position;
use bevy::prelude::*;
use persistence::{PendingStoreOps, StoreBackend};
use protocol::map::{ChunkEntityRef, MapInstanceId};
use protocol::vox_model::{VoxModelAsset, VoxModelRegistry};
use protocol::world_object::{
    ActiveTransformation, PlacementOffset, WorldObjectDefRegistry, WorldObjectId,
};
use voxel_map_engine::config::{WorldObjectPositionKind, WorldObjectSpawn};
use voxel_map_engine::persistence::fs_chunk_entities::FsChunkEntitiesStore;
use voxel_map_engine::prelude::{
    chunk_to_column, PendingEntitySpawns, PersistedComponent, VoxelMapInstance,
};

use crate::world_object::spawn_world_object;

/// Computes the chunk position containing a world-space position.
pub(crate) fn chunk_pos_for_world_position(position: Vec3, chunk_size: u32) -> IVec3 {
    voxel_map_engine::lifecycle::world_to_chunk_pos(position, chunk_size)
}

/// Query shape used when saving loaded world objects from a chunk.
pub type ChunkEntitySaveQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static ChunkEntityRef,
        &'static WorldObjectId,
        &'static Position,
        Option<&'static ActiveTransformation>,
        Option<&'static protocol::Health>,
    ),
>;

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
                let offset = extract_placement_offset(def, spawn.position_kind);
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
                let position = spawn.position + offset;
                commands.entity(entity).insert((
                    Position(position),
                    ChunkEntityRef {
                        chunk_pos,
                        map_entity,
                    },
                ));

                if !spawn.persisted_components.is_empty() {
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
                    position: pos.0,
                    position_kind: WorldObjectPositionKind::Final,
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

/// Collects saved spawn data for one loaded chunk.
pub fn collect_chunk_entity_spawns(
    map_entity: Entity,
    chunk_pos: IVec3,
    exclude_entity: Option<Entity>,
    entity_query: &Query<(
        Entity,
        &ChunkEntityRef,
        &WorldObjectId,
        &Position,
        Option<&ActiveTransformation>,
        Option<&protocol::Health>,
    )>,
) -> Vec<WorldObjectSpawn> {
    entity_query
        .iter()
        .filter(|(entity, chunk_ref, _, _, _, _)| {
            Some(*entity) != exclude_entity
                && chunk_ref.map_entity == map_entity
                && chunk_ref.chunk_pos == chunk_pos
        })
        .map(
            |(_, _, obj_id, pos, active_transform, health)| WorldObjectSpawn {
                object_id: obj_id.0.clone(),
                position: pos.0,
                position_kind: WorldObjectPositionKind::Final,
                persisted_components: serialize_persisted(active_transform, health),
            },
        )
        .collect()
}

/// Queues an immediate save for one loaded chunk, writing an empty entity file when no objects remain.
pub fn save_chunk_entities_now_or_queue(
    map_entity: Entity,
    chunk_pos: IVec3,
    exclude_entity: Option<Entity>,
    entity_query: &Query<(
        Entity,
        &ChunkEntityRef,
        &WorldObjectId,
        &Position,
        Option<&ActiveTransformation>,
        Option<&protocol::Health>,
    )>,
    store_query: &mut Query<(
        &StoreBackend<IVec3, Vec<WorldObjectSpawn>, FsChunkEntitiesStore>,
        &mut PendingStoreOps<IVec3, Vec<WorldObjectSpawn>>,
    )>,
) {
    let spawns = collect_chunk_entity_spawns(map_entity, chunk_pos, exclude_entity, entity_query);
    let Ok((store, mut ops)) = store_query.get_mut(map_entity) else {
        trace!(
            "save_chunk_entities_now_or_queue: map entity {map_entity:?} has no chunk entity store"
        );
        return;
    };
    ops.spawn_save(&store.0, chunk_pos, spawns);
}

/// Collect all living chunk entities grouped by `(map_entity, chunk_pos)`.
fn collect_chunk_entities(
    entity_query: &Query<(
        Entity,
        &ChunkEntityRef,
        &WorldObjectId,
        &Position,
        Option<&ActiveTransformation>,
        Option<&protocol::Health>,
    )>,
) -> HashMap<(Entity, IVec3), Vec<WorldObjectSpawn>> {
    let mut by_chunk: HashMap<(Entity, IVec3), Vec<WorldObjectSpawn>> = HashMap::new();
    for (_, chunk_ref, _, _, _, _) in entity_query {
        by_chunk
            .entry((chunk_ref.map_entity, chunk_ref.chunk_pos))
            .or_insert_with(|| {
                collect_chunk_entity_spawns(
                    chunk_ref.map_entity,
                    chunk_ref.chunk_pos,
                    None,
                    entity_query,
                )
            });
    }
    by_chunk
}

/// Save collected chunk entities to their stores (async, no flush).
fn save_chunk_entities_to_stores(
    by_chunk: HashMap<(Entity, IVec3), Vec<WorldObjectSpawn>>,
    store_query: &mut Query<(
        &StoreBackend<IVec3, Vec<WorldObjectSpawn>, FsChunkEntitiesStore>,
        &mut PendingStoreOps<IVec3, Vec<WorldObjectSpawn>>,
    )>,
) {
    for ((map_entity, chunk_pos), spawns) in by_chunk {
        let Ok((store, mut ops)) = store_query.get_mut(map_entity) else {
            continue;
        };
        ops.spawn_save(&store.0, chunk_pos, spawns);
    }
}

/// Periodically saves all loaded chunk entities to disk.
///
/// Uses its own debounce timer independent of the voxel dirty state,
/// ensuring entity-only changes (e.g. placed objects without voxel edits)
/// are persisted even if the chunk is never evicted.
pub fn save_chunk_entities_periodic(
    time: Res<Time>,
    mut last_save: Local<f64>,
    entity_query: Query<(
        Entity,
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
    const CHUNK_ENTITY_SAVE_INTERVAL: f64 = 5.0;

    let now = time.elapsed_secs_f64();
    if now - *last_save < CHUNK_ENTITY_SAVE_INTERVAL {
        return;
    }
    *last_save = now;

    let by_chunk = collect_chunk_entities(&entity_query);
    if by_chunk.is_empty() {
        return;
    }
    save_chunk_entities_to_stores(by_chunk, &mut store_query);
}

/// On server shutdown, saves entity files for all loaded chunks.
///
/// Ensures destroyed entities (no longer in the query) are excluded from
/// the saved file, maintaining the "generate once, save forever" invariant.
pub fn save_all_chunk_entities_on_exit(
    mut exit_reader: MessageReader<AppExit>,
    entity_query: Query<(
        Entity,
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
    let by_chunk = collect_chunk_entities(&entity_query);
    save_chunk_entities_to_stores(by_chunk, &mut store_query);
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

/// Extracts `PlacementOffset` when a spawn stores a placement-base position.
fn extract_placement_offset(
    def: &protocol::world_object::WorldObjectDef,
    position_kind: WorldObjectPositionKind,
) -> Vec3 {
    match position_kind {
        WorldObjectPositionKind::Final => Vec3::ZERO,
        WorldObjectPositionKind::PlacementBase => def
            .components
            .iter()
            .find_map(|c| c.try_downcast_ref::<PlacementOffset>())
            .map(|offset| offset.0)
            .unwrap_or(Vec3::ZERO),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::reflect::PartialReflect;
    use protocol::world_object::WorldObjectDef;

    #[test]
    fn extract_placement_offset_uses_position_kind() {
        let offset = Vec3::new(0.0, 1.5, 0.0);
        let def = WorldObjectDef {
            components: vec![Box::new(PlacementOffset(offset)) as Box<dyn PartialReflect>],
        };

        assert_eq!(
            extract_placement_offset(&def, WorldObjectPositionKind::PlacementBase),
            offset
        );
        assert_eq!(
            extract_placement_offset(&def, WorldObjectPositionKind::Final),
            Vec3::ZERO
        );
    }
}
