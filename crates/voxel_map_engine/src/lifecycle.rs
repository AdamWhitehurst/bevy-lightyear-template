use bevy::log::info_span;
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use std::collections::{HashMap, HashSet};
#[allow(unused_imports)]
use tracy_client::plot;

use crate::chunk::VoxelChunk;
use crate::config::{VoxelGenerator, VoxelMapConfig};
use crate::generation::{PendingChunks, spawn_chunk_gen_task};
use crate::instance::VoxelMapInstance;
use crate::meshing::mesh_chunk_greedy;
use crate::propagator::TicketLevelPropagator;
use crate::ticket::{
    ChunkTicket, DEFAULT_COLUMN_Y_MAX, DEFAULT_COLUMN_Y_MIN, TicketType, chunk_to_column,
    column_to_chunks,
};
use crate::types::CHUNK_SIZE;

/// Per-frame time budget for chunk pipeline work on a single map.
/// Reset at the start of each frame by `update_chunks`.
/// All downstream systems check `has_time()` before doing work.
#[derive(Component)]
pub struct ChunkWorkBudget {
    start: std::time::Instant,
    budget: std::time::Duration,
}

/// Default budget: ~25% of a 16ms frame at 60fps.
const CHUNK_WORK_BUDGET_MS: u64 = 4;

/// Safety caps -- even within budget, don't exceed these per frame.
const MAX_GEN_SPAWNS_PER_FRAME: usize = 64;
const MAX_GEN_POLLS_PER_FRAME: usize = 32;
const MAX_REMESH_SPAWNS_PER_FRAME: usize = 32;
const MAX_REMESH_POLLS_PER_FRAME: usize = 32;

impl ChunkWorkBudget {
    fn reset(&mut self) {
        self.start = std::time::Instant::now();
    }

    /// Returns true if there is time remaining in the budget.
    pub fn has_time(&self) -> bool {
        self.start.elapsed() < self.budget
    }
}

impl Default for ChunkWorkBudget {
    fn default() -> Self {
        Self {
            start: std::time::Instant::now(),
            budget: std::time::Duration::from_millis(CHUNK_WORK_BUDGET_MS),
        }
    }
}

/// Why a throttled loop stopped processing. Emitted as a Tracy plot for tuning.
#[repr(u8)]
enum StopReason {
    /// All available work was processed.
    Completed = 0,
    /// Time budget exhausted.
    TimeBudget = 1,
    /// Per-frame hard cap reached.
    HardCap = 2,
    /// Total in-flight task cap reached.
    #[allow(dead_code)]
    InFlightCap = 3,
}

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

/// A pending async remesh task for a chunk mutated in-place.
struct RemeshTask {
    chunk_pos: IVec3,
    task: Task<Option<Mesh>>,
}

/// Component tracking pending remesh tasks for a map instance.
#[derive(Component, Default)]
pub struct PendingRemeshes {
    tasks: Vec<RemeshTask>,
}

/// Cached state for a single ticket, used to detect changes.
pub(crate) struct CachedTicket {
    column: IVec2,
    map_entity: Entity,
    ticket_type: TicketType,
    radius: u32,
}

/// Convert a world-space position to a 2D column position (drop Y).
pub fn world_to_column_pos(translation: Vec3) -> IVec2 {
    let chunk = world_to_chunk_pos(translation);
    IVec2::new(chunk.x, chunk.z)
}

/// Auto-insert `PendingChunks`, `PendingRemeshes`, and `TicketLevelPropagator`
/// on map entities that lack them.
///
/// Gated on `With<VoxelGenerator>` -- maps without a generator don't start loading chunks.
pub fn ensure_pending_chunks(
    mut commands: Commands,
    chunks_query: Query<
        Entity,
        (
            With<VoxelMapInstance>,
            With<VoxelGenerator>,
            Without<PendingChunks>,
        ),
    >,
    remesh_query: Query<
        Entity,
        (
            With<VoxelMapInstance>,
            With<VoxelGenerator>,
            Without<PendingRemeshes>,
        ),
    >,
    propagator_query: Query<
        Entity,
        (
            With<VoxelMapInstance>,
            With<VoxelGenerator>,
            Without<TicketLevelPropagator>,
        ),
    >,
    budget_query: Query<
        Entity,
        (
            With<VoxelMapInstance>,
            With<VoxelGenerator>,
            Without<ChunkWorkBudget>,
        ),
    >,
) {
    for entity in &chunks_query {
        info!("ensure_pending_chunks: adding PendingChunks to {entity:?}");
        commands.entity(entity).insert(PendingChunks::default());
    }
    for entity in &remesh_query {
        info!("ensure_pending_chunks: adding PendingRemeshes to {entity:?}");
        commands.entity(entity).insert(PendingRemeshes::default());
    }
    for entity in &propagator_query {
        info!("ensure_pending_chunks: adding TicketLevelPropagator to {entity:?}");
        commands
            .entity(entity)
            .insert(TicketLevelPropagator::default());
    }
    for entity in &budget_query {
        info!("ensure_pending_chunks: adding ChunkWorkBudget to {entity:?}");
        commands.entity(entity).insert(ChunkWorkBudget::default());
    }
}

/// Collect tickets, propagate levels, and spawn/remove chunks based on the diff.
pub(crate) fn update_chunks(
    mut map_query: Query<(
        Entity,
        &mut VoxelMapInstance,
        &VoxelMapConfig,
        &VoxelGenerator,
        &mut PendingChunks,
        &mut TicketLevelPropagator,
        &GlobalTransform,
        &mut ChunkWorkBudget,
    )>,
    ticket_query: Query<(Entity, &ChunkTicket, &GlobalTransform)>,
    mut tick: Local<u32>,
    mut ticket_cache: Local<HashMap<Entity, CachedTicket>>,
) {
    *tick += 1;

    collect_tickets(&mut map_query, &ticket_query, &mut ticket_cache);

    let y_min = DEFAULT_COLUMN_Y_MIN;
    let y_max = DEFAULT_COLUMN_Y_MAX;

    for (
        _map_entity,
        mut instance,
        config,
        generator,
        mut pending,
        mut propagator,
        _,
        mut budget,
    ) in &mut map_query
    {
        let diff = {
            let _span = info_span!("propagate_ticket_levels").entered();
            propagator.propagate()
        };

        // Reset budget AFTER propagation so BFS doesn't eat the spawn/poll budget.
        // TODO(Phase 4): Once propagation is amortized (max_steps per frame), move
        // this reset back before propagation so BFS counts against the budget too.
        budget.reset();

        for &col in &diff.unloaded {
            remove_column_chunks(&mut instance, col, config.save_dir.as_deref(), y_min, y_max);
        }
        for &(col, level) in &diff.loaded {
            if is_column_within_bounds(col, config.bounds) {
                instance.chunk_levels.insert(col, level);
            }
        }
        for &(col, level) in &diff.changed {
            if is_column_within_bounds(col, config.bounds) {
                instance.chunk_levels.insert(col, level);
            }
        }

        if config.generates_chunks {
            spawn_missing_chunks(
                &instance,
                &mut pending,
                config,
                generator,
                y_min,
                y_max,
                &budget,
            );
        }

        plot!(
            "chunk_work_budget_remaining_us",
            budget
                .budget
                .saturating_sub(budget.start.elapsed())
                .as_micros() as f64
        );
    }
}

/// Detect stale and changed tickets, updating propagator sources accordingly.
fn collect_tickets(
    map_query: &mut Query<(
        Entity,
        &mut VoxelMapInstance,
        &VoxelMapConfig,
        &VoxelGenerator,
        &mut PendingChunks,
        &mut TicketLevelPropagator,
        &GlobalTransform,
        &mut ChunkWorkBudget,
    )>,
    ticket_query: &Query<(Entity, &ChunkTicket, &GlobalTransform)>,
    ticket_cache: &mut HashMap<Entity, CachedTicket>,
) {
    let _span = info_span!("collect_tickets").entered();

    let active: HashSet<Entity> = ticket_query.iter().map(|(e, _, _)| e).collect();
    let stale: Vec<Entity> = ticket_cache
        .keys()
        .filter(|e| !active.contains(e))
        .copied()
        .collect();
    for entity in stale {
        if let Some(cached) = ticket_cache.remove(&entity) {
            if let Ok((_, _, _, _, _, mut prop, _, _)) = map_query.get_mut(cached.map_entity) {
                prop.remove_source(entity);
            }
        }
    }

    for (ticket_entity, ticket, transform) in ticket_query.iter() {
        // Compute column from immutable access; borrow drops at end of block.
        let column = {
            let Ok((_, _, _, _, _, _, map_transform, _)) = map_query.get(ticket.map_entity) else {
                trace!(
                    "collect_tickets: ticket {ticket_entity:?} references non-existent map {:?}, expected during deferred command application",
                    ticket.map_entity
                );
                continue;
            };
            let map_inv = map_transform.affine().inverse();
            let local_pos = map_inv.transform_point3(transform.translation());
            world_to_column_pos(local_pos)
        };

        let needs_update = match ticket_cache.get(&ticket_entity) {
            Some(cached) => {
                cached.column != column
                    || cached.map_entity != ticket.map_entity
                    || cached.ticket_type != ticket.ticket_type
                    || cached.radius != ticket.radius
            }
            None => true,
        };

        if needs_update {
            // If map changed, remove source from old map's propagator first
            if let Some(cached) = ticket_cache.get(&ticket_entity) {
                if cached.map_entity != ticket.map_entity {
                    if let Ok((_, _, _, _, _, mut old_prop, _, _)) =
                        map_query.get_mut(cached.map_entity)
                    {
                        old_prop.remove_source(ticket_entity);
                    }
                }
            }
            if let Ok((_, _, _, _, _, mut prop, _, _)) = map_query.get_mut(ticket.map_entity) {
                prop.set_source(
                    ticket_entity,
                    column,
                    ticket.ticket_type.base_level(),
                    ticket.radius,
                );
            }
            ticket_cache.insert(
                ticket_entity,
                CachedTicket {
                    column,
                    map_entity: ticket.map_entity,
                    ticket_type: ticket.ticket_type,
                    radius: ticket.radius,
                },
            );
        }
    }
}

/// Remove all chunk data for a column being unloaded.
fn remove_column_chunks(
    instance: &mut VoxelMapInstance,
    col: IVec2,
    save_dir: Option<&std::path::Path>,
    y_min: i32,
    y_max: i32,
) {
    let _span = info_span!("remove_column_chunks").entered();
    for chunk_pos in column_to_chunks(col, y_min, y_max) {
        if instance.dirty_chunks.remove(&chunk_pos) {
            if let Some(dir) = save_dir {
                if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
                    let data = chunk_data.clone();
                    let dir = dir.to_path_buf();
                    AsyncComputeTaskPool::get()
                        .spawn(async move {
                            if let Err(e) = crate::persistence::save_chunk(&dir, chunk_pos, &data) {
                                error!("Failed to save evicted dirty chunk at {chunk_pos}: {e}");
                            }
                        })
                        .detach();
                }
            }
        }
        instance.remove_chunk_data(chunk_pos);
    }
    instance.chunk_levels.remove(&col);
}

/// Spawn generation tasks for chunks in loaded columns that lack data.
fn spawn_missing_chunks(
    instance: &VoxelMapInstance,
    pending: &mut PendingChunks,
    config: &VoxelMapConfig,
    generator: &VoxelGenerator,
    y_min: i32,
    y_max: i32,
    budget: &ChunkWorkBudget,
) {
    let _span = info_span!("spawn_missing_chunks").entered();
    let mut spawned = 0;

    let mut cols: Vec<_> = instance.chunk_levels.iter().collect();
    cols.sort_by_key(|(_, lvl)| **lvl);

    'outer: for (&col, &_level) in cols {
        for chunk_pos in column_to_chunks(col, y_min, y_max) {
            if !budget.has_time() || spawned >= MAX_GEN_SPAWNS_PER_FRAME {
                break 'outer;
            }
            if !is_within_bounds(chunk_pos, config.bounds) {
                continue;
            }
            if instance.get_chunk_data(chunk_pos).is_some() {
                continue;
            }
            if is_already_pending(pending, chunk_pos) {
                continue;
            }
            spawn_chunk_gen_task(pending, chunk_pos, generator, config.save_dir.clone());
            spawned += 1;
        }
    }

    let stop_reason = if spawned >= MAX_GEN_SPAWNS_PER_FRAME {
        StopReason::HardCap
    } else if !budget.has_time() {
        StopReason::TimeBudget
    } else {
        StopReason::Completed
    };
    plot!("gen_spawn_stop_reason", stop_reason as u8 as f64);
    plot!("gen_spawned_this_frame", spawned as f64);
    plot!("gen_queue_depth", instance.chunk_levels.len() as f64);
}

fn is_already_pending(pending: &PendingChunks, pos: IVec3) -> bool {
    pending.pending_positions.contains(&pos)
}

/// Poll pending chunk generation tasks and spawn mesh entities for completed ones.
pub fn poll_chunk_tasks(
    mut commands: Commands,
    mut map_query: Query<(
        Entity,
        &mut VoxelMapInstance,
        &mut PendingChunks,
        &ChunkWorkBudget,
    )>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    default_material: Option<Res<DefaultVoxelMaterial>>,
) {
    let Some(default_material) = default_material else {
        warn!("DefaultVoxelMaterial resource not found; chunk meshes will not spawn");
        return;
    };

    for (map_entity, mut instance, mut pending, budget) in &mut map_query {
        let mut i = 0;
        let mut polled = 0;
        while i < pending.tasks.len() && budget.has_time() && polled < MAX_GEN_POLLS_PER_FRAME {
            if let Some(result) = check_ready(&mut pending.tasks[i]) {
                let _ = pending.tasks.swap_remove(i);
                debug_assert!(
                    pending.pending_positions.contains(&result.position),
                    "poll_chunk_tasks: completed chunk at {:?} was not in pending_positions",
                    result.position
                );
                pending.pending_positions.remove(&result.position);
                handle_completed_chunk(
                    &mut commands,
                    &mut instance,
                    &mut *meshes,
                    &mut *materials,
                    &*default_material,
                    map_entity,
                    result,
                );
                polled += 1;
            } else {
                i += 1;
            }
        }

        let stop_reason = if polled >= MAX_GEN_POLLS_PER_FRAME {
            StopReason::HardCap
        } else if !budget.has_time() {
            StopReason::TimeBudget
        } else {
            StopReason::Completed
        };
        plot!("gen_poll_stop_reason", stop_reason as u8 as f64);
        plot!("gen_tasks_in_flight", pending.tasks.len() as f64);
        plot!("gen_tasks_polled_this_frame", polled as f64);
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
    instance.insert_chunk_data(result.position, result.chunk_data);

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

/// Whether a 2D column is within the map's optional bounds.
fn is_column_within_bounds(col: IVec2, bounds: Option<IVec3>) -> bool {
    match bounds {
        Some(b) => col.x.abs() < b.x && col.y.abs() < b.z,
        None => true,
    }
}

/// Whether a 3D chunk position is within the map's optional bounds.
fn is_within_bounds(pos: IVec3, bounds: Option<IVec3>) -> bool {
    match bounds {
        Some(b) => pos.x.abs() < b.x && pos.y.abs() < b.y && pos.z.abs() < b.z,
        None => true,
    }
}

fn world_to_chunk_pos(translation: Vec3) -> IVec3 {
    (translation / CHUNK_SIZE as f32).floor().as_ivec3()
}

fn chunk_world_offset(chunk_pos: IVec3) -> Vec3 {
    chunk_pos.as_vec3() * CHUNK_SIZE as f32 - Vec3::ONE
}

/// Despawn chunk entities whose column is no longer in the parent map's `chunk_levels`.
pub fn despawn_out_of_range_chunks(
    mut commands: Commands,
    chunk_query: Query<(Entity, &VoxelChunk, &ChildOf)>,
    map_query: Query<&VoxelMapInstance>,
) {
    for (entity, chunk, child_of) in &chunk_query {
        debug_assert!(
            map_query.get(child_of.0).is_ok(),
            "VoxelChunk {:?} at {:?} is child of {:?} which has no VoxelMapInstance",
            entity,
            chunk.position,
            child_of.0
        );
        let Ok(instance) = map_query.get(child_of.0) else {
            warn!(
                "VoxelChunk entity {:?} has ChildOf pointing to non-map entity {:?}",
                entity, child_of.0
            );
            continue;
        };

        if !instance
            .chunk_levels
            .contains_key(&chunk_to_column(chunk.position))
        {
            info!(
                "despawn_out_of_range_chunks: despawning chunk {:?} at {:?} (parent map {:?})",
                entity, chunk.position, child_of.0
            );
            commands.entity(entity).despawn();
        }
    }
}

/// Drains `chunks_needing_remesh` and spawns async mesh tasks from existing octree data.
pub fn spawn_remesh_tasks(
    mut map_query: Query<(
        &mut VoxelMapInstance,
        &mut PendingRemeshes,
        &ChunkWorkBudget,
    )>,
) {
    let pool = AsyncComputeTaskPool::get();
    for (mut instance, mut pending, budget) in &mut map_query {
        let positions: Vec<IVec3> = instance.chunks_needing_remesh.iter().copied().collect();

        let mut spawned = 0;
        for chunk_pos in positions {
            if !budget.has_time() || spawned >= MAX_REMESH_SPAWNS_PER_FRAME {
                break;
            }
            let Some(chunk_data) = instance.get_chunk_data(chunk_pos) else {
                trace!("spawn_remesh_tasks: chunk {chunk_pos} no longer in octree, skipping");
                instance.chunks_needing_remesh.remove(&chunk_pos);
                continue;
            };
            if chunk_data.fill_type == crate::types::FillType::Empty {
                trace!("spawn_remesh_tasks: chunk {chunk_pos} is empty, skipping remesh");
                instance.chunks_needing_remesh.remove(&chunk_pos);
                continue;
            }
            let voxels = {
                let _span = info_span!("expand_palette").entered();
                chunk_data.voxels.to_voxels()
            };
            let task = pool.spawn(async move { mesh_chunk_greedy(&voxels) });
            pending.tasks.push(RemeshTask { chunk_pos, task });
            instance.chunks_needing_remesh.remove(&chunk_pos);
            spawned += 1;
        }

        let stop_reason = if spawned >= MAX_REMESH_SPAWNS_PER_FRAME {
            StopReason::HardCap
        } else if !budget.has_time() {
            StopReason::TimeBudget
        } else {
            StopReason::Completed
        };
        plot!("remesh_spawn_stop_reason", stop_reason as u8 as f64);
        plot!("remesh_spawned_this_frame", spawned as f64);
        plot!("remesh_tasks_in_flight", pending.tasks.len() as f64);
    }
}

/// Polls completed remesh tasks and swaps meshes on existing chunk entities.
pub fn poll_remesh_tasks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    default_material: Res<DefaultVoxelMaterial>,
    mut map_query: Query<(
        Entity,
        &VoxelMapInstance,
        &mut PendingRemeshes,
        &ChunkWorkBudget,
    )>,
    chunk_query: Query<(Entity, &VoxelChunk, &ChildOf)>,
) {
    for (map_entity, instance, mut pending, budget) in &mut map_query {
        let mut i = 0;
        let mut polled = 0;
        while i < pending.tasks.len() && budget.has_time() && polled < MAX_REMESH_POLLS_PER_FRAME {
            let Some(mesh_opt) = check_ready(&mut pending.tasks[i].task) else {
                i += 1;
                continue;
            };
            let remesh = pending.tasks.swap_remove(i);
            polled += 1;

            if !instance
                .chunk_levels
                .contains_key(&chunk_to_column(remesh.chunk_pos))
            {
                continue;
            }

            let existing = chunk_query
                .iter()
                .find(|(_, vc, parent)| vc.position == remesh.chunk_pos && parent.0 == map_entity);

            match (mesh_opt, existing) {
                (Some(mesh), Some((entity, _, _))) => {
                    let handle = meshes.add(mesh);
                    commands.entity(entity).insert(Mesh3d(handle));
                }
                (Some(mesh), None) => {
                    let handle = meshes.add(mesh);
                    let offset = chunk_world_offset(remesh.chunk_pos);
                    let material = if instance.debug_colors {
                        materials.add(StandardMaterial {
                            base_color: color_from_chunk_pos(remesh.chunk_pos),
                            perceptual_roughness: 0.9,
                            ..default()
                        })
                    } else {
                        default_material.0.clone()
                    };
                    let chunk_entity = commands
                        .spawn((
                            VoxelChunk {
                                position: remesh.chunk_pos,
                                lod_level: 0,
                            },
                            Mesh3d(handle),
                            MeshMaterial3d(material),
                            Transform::from_translation(offset),
                        ))
                        .id();
                    commands.entity(map_entity).add_child(chunk_entity);
                }
                (None, Some((entity, _, _))) => {
                    commands.entity(entity).despawn();
                }
                (None, None) => {}
            }
        }

        let stop_reason = if polled >= MAX_REMESH_POLLS_PER_FRAME {
            StopReason::HardCap
        } else if !budget.has_time() {
            StopReason::TimeBudget
        } else {
            StopReason::Completed
        };
        plot!("remesh_poll_stop_reason", stop_reason as u8 as f64);
        plot!("remesh_polled_this_frame", polled as f64);
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
    fn chunk_world_offset_calculation() {
        let offset = chunk_world_offset(IVec3::new(1, 2, 3));
        assert_eq!(offset, Vec3::new(15.0, 31.0, 47.0));
    }

    #[test]
    fn world_to_column_pos_drops_y() {
        let col = world_to_column_pos(Vec3::new(20.0, 99.0, 5.0));
        assert_eq!(col, IVec2::new(1, 0));
    }
}
