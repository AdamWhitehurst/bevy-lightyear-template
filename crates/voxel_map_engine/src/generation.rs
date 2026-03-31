use std::path::PathBuf;
use std::sync::Arc;

use bevy::log::info_span;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use crate::config::VoxelGenerator;
use crate::meshing::mesh_chunk_greedy;
use crate::types::{ChunkData, ChunkStatus, FillType, WorldVoxel};

/// Number of chunks to generate per async task.
pub const GEN_BATCH_SIZE: usize = 8;

/// Result of an async chunk generation task.
pub struct ChunkGenResult {
    pub position: IVec3,
    pub mesh: Option<Mesh>,
    pub chunk_data: ChunkData,
    /// Whether this chunk was loaded from disk rather than generated.
    pub from_disk: bool,
}

/// Pending async chunk generation tasks for a map entity.
#[derive(Component, Default)]
pub struct PendingChunks {
    pub tasks: Vec<Task<Vec<ChunkGenResult>>>,
}

/// Spawn an async task that generates a batch of chunks.
///
/// Each position is first checked on disk; if not found, the generator is used.
pub fn spawn_chunk_gen_batch(
    pending: &mut PendingChunks,
    positions: Vec<IVec3>,
    generator: &VoxelGenerator,
    save_dir: Option<PathBuf>,
) {
    let generator = Arc::clone(&generator.0);
    let pool = AsyncComputeTaskPool::get();

    let task = pool.spawn(async move {
        let _span = info_span!("chunk_gen_batch", count = positions.len()).entered();
        positions
            .into_iter()
            .map(|pos| {
                if let Some(ref dir) = save_dir {
                    match crate::persistence::load_chunk(dir, pos) {
                        Ok(Some(chunk_data)) => {
                            let mesh = if chunk_data.fill_type == FillType::Empty {
                                None
                            } else {
                                let voxels = {
                                    let _span = info_span!("disk_load_expand").entered();
                                    chunk_data.voxels.to_voxels()
                                };
                                let _span = info_span!("mesh_chunk").entered();
                                mesh_chunk_greedy(&voxels)
                            };
                            return ChunkGenResult {
                                position: pos,
                                mesh,
                                chunk_data,
                                from_disk: true,
                            };
                        }
                        Ok(None) => {}
                        Err(e) => {
                            bevy::log::warn!("Failed to load chunk at {pos}: {e}, regenerating");
                        }
                    }
                }
                generate_chunk(pos, &*generator)
            })
            .collect()
    });

    pending.tasks.push(task);
}

fn generate_chunk(position: IVec3, generator: &dyn Fn(IVec3) -> Vec<WorldVoxel>) -> ChunkGenResult {
    let voxels = {
        let _span = info_span!("terrain_gen").entered();
        generator(position)
    };
    let chunk_data = {
        let _span = info_span!("palettize_chunk").entered();
        ChunkData::from_voxels(&voxels, ChunkStatus::Full)
    };
    let mesh = if chunk_data.fill_type == FillType::Empty {
        None
    } else {
        let _span = info_span!("mesh_chunk").entered();
        mesh_chunk_greedy(&voxels)
    };
    ChunkGenResult {
        position,
        mesh,
        chunk_data,
        from_disk: false,
    }
}
