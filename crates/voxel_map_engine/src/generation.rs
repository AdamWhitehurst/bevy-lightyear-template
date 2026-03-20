use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use crate::config::VoxelGenerator;
use crate::meshing::mesh_chunk_greedy;
use crate::types::WorldVoxel;

/// Result of an async chunk generation task.
pub struct ChunkGenResult {
    pub position: IVec3,
    pub mesh: Option<Mesh>,
    pub voxels: Vec<WorldVoxel>,
    /// Whether this chunk was loaded from disk rather than generated.
    pub from_disk: bool,
}

/// Pending async chunk generation tasks for a map entity.
#[derive(Component, Default)]
pub struct PendingChunks {
    pub tasks: Vec<Task<ChunkGenResult>>,
    pub pending_positions: HashSet<IVec3>,
}

/// Spawn an async task that loads a chunk from disk (if available) or generates it.
pub fn spawn_chunk_gen_task(
    pending: &mut PendingChunks,
    position: IVec3,
    generator: &VoxelGenerator,
    save_dir: Option<PathBuf>,
) {
    let generator = Arc::clone(&generator.0);
    let pool = AsyncComputeTaskPool::get();

    let task = pool.spawn(async move {
        if let Some(ref dir) = save_dir {
            match crate::persistence::load_chunk(dir, position) {
                Ok(Some(chunk_data)) => {
                    let voxels = chunk_data.voxels.to_voxels();
                    let mesh = mesh_chunk_greedy(&voxels);
                    return ChunkGenResult {
                        position,
                        mesh,
                        voxels,
                        from_disk: true,
                    };
                }
                Ok(None) => {} // No saved file, generate fresh
                Err(e) => {
                    bevy::log::warn!("Failed to load chunk at {position}: {e}, regenerating");
                }
            }
        }

        generate_chunk(position, &*generator)
    });

    pending.tasks.push(task);
    pending.pending_positions.insert(position);
}

fn generate_chunk(position: IVec3, generator: &dyn Fn(IVec3) -> Vec<WorldVoxel>) -> ChunkGenResult {
    let voxels = generator(position);
    let mesh = mesh_chunk_greedy(&voxels);
    ChunkGenResult {
        position,
        mesh,
        voxels,
        from_disk: false,
    }
}
