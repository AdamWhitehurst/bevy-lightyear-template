use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use std::collections::HashSet;

use crate::chunk::{ChunkTarget, VoxelChunk};
use crate::config::VoxelMapConfig;
use crate::generation::{PendingChunks, spawn_chunk_gen_task};
use crate::instance::VoxelMapInstance;
use crate::types::CHUNK_SIZE;

const MAX_TASKS_PER_FRAME: usize = 32;

/// Default PBR material applied to voxel chunk meshes.
#[derive(Resource)]
pub struct DefaultVoxelMaterial(pub Handle<StandardMaterial>);

/// Startup system that creates the default voxel material.
pub fn init_default_material(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let handle = materials.add(StandardMaterial {
        base_color: Color::srgb(0.5, 0.7, 0.3),
        perceptual_roughness: 0.9,
        ..default()
    });
    commands.insert_resource(DefaultVoxelMaterial(handle));
}

/// Determine which chunk positions should be loaded based on all targets for a map.
/// Spawn async generation tasks for missing chunks and mark out-of-range chunks for removal.
pub fn update_chunks(
    mut map_query: Query<(
        Entity,
        &mut VoxelMapInstance,
        &VoxelMapConfig,
        &mut PendingChunks,
    )>,
    target_query: Query<(&ChunkTarget, &GlobalTransform)>,
) {
    for (map_entity, mut instance, config, mut pending) in &mut map_query {
        let desired = collect_desired_positions(map_entity, config, &target_query);

        remove_out_of_range_chunks(&mut instance, &desired);
        spawn_missing_chunks(&mut instance, &mut pending, config, &desired);
    }
}

fn collect_desired_positions(
    map_entity: Entity,
    config: &VoxelMapConfig,
    target_query: &Query<(&ChunkTarget, &GlobalTransform)>,
) -> HashSet<IVec3> {
    let mut desired = HashSet::new();

    for (target, transform) in target_query.iter() {
        if target.map_entity != map_entity {
            continue;
        }
        let center = world_to_chunk_pos(transform.translation());
        let dist = target.distance as i32;

        for x in -dist..=dist {
            for y in -dist..=dist {
                for z in -dist..=dist {
                    let pos = center + IVec3::new(x, y, z);
                    if is_within_bounds(pos, config.bounds) {
                        desired.insert(pos);
                    }
                }
            }
        }
    }

    desired
}

fn world_to_chunk_pos(translation: Vec3) -> IVec3 {
    (translation / CHUNK_SIZE as f32).floor().as_ivec3()
}

fn is_within_bounds(pos: IVec3, bounds: Option<IVec3>) -> bool {
    match bounds {
        Some(b) => pos.x.abs() < b.x && pos.y.abs() < b.y && pos.z.abs() < b.z,
        None => true,
    }
}

fn remove_out_of_range_chunks(instance: &mut VoxelMapInstance, desired: &HashSet<IVec3>) {
    instance.loaded_chunks.retain(|pos| desired.contains(pos));
}

fn spawn_missing_chunks(
    instance: &mut VoxelMapInstance,
    pending: &mut PendingChunks,
    config: &VoxelMapConfig,
    desired: &HashSet<IVec3>,
) {
    let mut spawned = 0;

    for &pos in desired {
        if spawned >= MAX_TASKS_PER_FRAME {
            break;
        }
        if instance.loaded_chunks.contains(&pos) {
            continue;
        }
        if is_already_pending(pending, pos) {
            continue;
        }

        spawn_chunk_gen_task(pending, pos, &config.generator);
        spawned += 1;
    }
}

fn is_already_pending(pending: &PendingChunks, pos: IVec3) -> bool {
    pending.pending_positions.contains(&pos)
}

/// Poll pending chunk generation tasks and spawn mesh entities for completed ones.
pub fn poll_chunk_tasks(
    mut commands: Commands,
    mut map_query: Query<(Entity, &mut VoxelMapInstance, &mut PendingChunks)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    default_material: Option<Res<DefaultVoxelMaterial>>,
) {
    let Some(default_material) = default_material else {
        warn!("DefaultVoxelMaterial resource not found; chunk meshes will not spawn");
        return;
    };

    for (map_entity, mut instance, mut pending) in &mut map_query {
        let mut i = 0;
        while i < pending.tasks.len() {
            if let Some(result) = check_ready(&mut pending.tasks[i]) {
                let _ = pending.tasks.swap_remove(i);
                pending.pending_positions.remove(&result.position);
                handle_completed_chunk(
                    &mut commands,
                    &mut instance,
                    &mut meshes,
                    &mut materials,
                    &default_material,
                    map_entity,
                    result,
                );
            } else {
                i += 1;
            }
        }
    }
}

fn color_from_chunk_pos(pos: IVec3) -> Color {
    let hash = (pos.x.wrapping_mul(73856093))
        ^ (pos.y.wrapping_mul(19349663))
        ^ (pos.z.wrapping_mul(83492791));
    let hue = ((hash as u32) % 360) as f32;
    Color::hsl(hue, 0.5, 0.5)
}

fn handle_completed_chunk(
    commands: &mut Commands,
    instance: &mut VoxelMapInstance,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    default_material: &DefaultVoxelMaterial,
    map_entity: Entity,
    result: crate::generation::ChunkGenResult,
) {
    instance.loaded_chunks.insert(result.position);

    let Some(mesh) = result.mesh else {
        return;
    };

    let mesh_handle = meshes.add(mesh);
    let offset = chunk_world_offset(result.position);

    let material = if instance.debug_colors {
        materials.add(StandardMaterial {
            base_color: color_from_chunk_pos(result.position),
            perceptual_roughness: 0.9,
            ..default()
        })
    } else {
        default_material.0.clone()
    };

    let chunk_entity = commands
        .spawn((
            VoxelChunk {
                position: result.position,
                lod_level: 0,
            },
            Mesh3d(mesh_handle),
            MeshMaterial3d(material),
            Transform::from_translation(offset),
        ))
        .id();

    commands.entity(map_entity).add_child(chunk_entity);
}

fn chunk_world_offset(chunk_pos: IVec3) -> Vec3 {
    chunk_pos.as_vec3() * CHUNK_SIZE as f32
}

/// Despawn chunk entities whose position is no longer in the parent map's loaded_chunks.
pub fn despawn_out_of_range_chunks(
    mut commands: Commands,
    chunk_query: Query<(Entity, &VoxelChunk, &ChildOf)>,
    map_query: Query<&VoxelMapInstance>,
) {
    for (entity, chunk, child_of) in &chunk_query {
        let Ok(instance) = map_query.get(child_of.0) else {
            warn!(
                "VoxelChunk entity {:?} has ChildOf pointing to non-map entity {:?}",
                entity, child_of.0
            );
            continue;
        };

        if !instance.loaded_chunks.contains(&chunk.position) {
            commands.entity(entity).despawn();
        }
    }
}

/// Drain the write buffer, apply modifications, and invalidate affected chunks.
pub fn flush_write_buffer(mut map_query: Query<&mut VoxelMapInstance>) {
    for mut instance in &mut map_query {
        if instance.write_buffer.is_empty() {
            continue;
        }

        let buffer: Vec<_> = instance.write_buffer.drain(..).collect();
        let mut invalidated = HashSet::new();

        for (world_pos, voxel) in buffer {
            instance.modified_voxels.insert(world_pos, voxel);
            let chunk_pos = voxel_to_chunk_pos(world_pos);
            invalidated.insert(chunk_pos);
            invalidate_neighbors(world_pos, chunk_pos, &mut invalidated);
        }

        for pos in invalidated {
            instance.loaded_chunks.remove(&pos);
        }
    }
}

fn voxel_to_chunk_pos(voxel_pos: IVec3) -> IVec3 {
    IVec3::new(
        voxel_pos.x.div_euclid(CHUNK_SIZE as i32),
        voxel_pos.y.div_euclid(CHUNK_SIZE as i32),
        voxel_pos.z.div_euclid(CHUNK_SIZE as i32),
    )
}

/// If a voxel is on a chunk boundary, the adjacent chunk also needs regeneration
/// because of the 1-voxel padding overlap.
fn invalidate_neighbors(voxel_pos: IVec3, chunk_pos: IVec3, invalidated: &mut HashSet<IVec3>) {
    let local = voxel_pos - chunk_pos * CHUNK_SIZE as i32;

    if local.x == 0 {
        invalidated.insert(chunk_pos - IVec3::X);
    }
    if local.x == CHUNK_SIZE as i32 - 1 {
        invalidated.insert(chunk_pos + IVec3::X);
    }
    if local.y == 0 {
        invalidated.insert(chunk_pos - IVec3::Y);
    }
    if local.y == CHUNK_SIZE as i32 - 1 {
        invalidated.insert(chunk_pos + IVec3::Y);
    }
    if local.z == 0 {
        invalidated.insert(chunk_pos - IVec3::Z);
    }
    if local.z == CHUNK_SIZE as i32 - 1 {
        invalidated.insert(chunk_pos + IVec3::Z);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_to_chunk_pos_positive() {
        let pos = world_to_chunk_pos(Vec3::new(20.0, 0.0, 5.0));
        assert_eq!(pos, IVec3::new(1, 0, 0));
    }

    #[test]
    fn world_to_chunk_pos_negative() {
        let pos = world_to_chunk_pos(Vec3::new(-1.0, -17.0, 0.0));
        assert_eq!(pos, IVec3::new(-1, -2, 0));
    }

    #[test]
    fn voxel_to_chunk_pos_basic() {
        assert_eq!(voxel_to_chunk_pos(IVec3::new(0, 0, 0)), IVec3::ZERO);
        assert_eq!(voxel_to_chunk_pos(IVec3::new(16, 0, 0)), IVec3::X);
        assert_eq!(voxel_to_chunk_pos(IVec3::new(-1, 0, 0)), -IVec3::X);
    }

    #[test]
    fn bounds_check() {
        assert!(is_within_bounds(IVec3::ZERO, Some(IVec3::new(5, 5, 5))));
        assert!(!is_within_bounds(
            IVec3::new(5, 0, 0),
            Some(IVec3::new(5, 5, 5))
        ));
        assert!(is_within_bounds(IVec3::new(100, 100, 100), None));
    }

    #[test]
    fn chunk_world_offset_calculation() {
        let offset = chunk_world_offset(IVec3::new(1, 2, 3));
        assert_eq!(offset, Vec3::new(16.0, 32.0, 48.0));
    }
}
