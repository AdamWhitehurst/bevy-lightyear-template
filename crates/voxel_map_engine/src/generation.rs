use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use ndshape::ConstShape;

use crate::config::SdfGenerator;
use crate::meshing::{SurfaceNetsMesher, VoxelMesher};
use crate::types::{CHUNK_SIZE, PaddedChunkShape, WorldVoxel};

/// Result of an async chunk generation task.
pub struct ChunkGenResult {
    pub position: IVec3,
    pub mesh: Option<Mesh>,
}

/// Pending async chunk generation tasks for a map entity.
#[derive(Component, Default)]
pub struct PendingChunks {
    pub tasks: Vec<Task<ChunkGenResult>>,
    pub pending_positions: HashSet<IVec3>,
}

/// Spawn an async task that generates SDF data, applies voxel modifications, and meshes a chunk.
pub fn spawn_chunk_gen_task(
    pending: &mut PendingChunks,
    position: IVec3,
    generator: &SdfGenerator,
    modified_voxels: &HashMap<IVec3, WorldVoxel>,
) {
    let generator = Arc::clone(generator);
    let overrides = collect_chunk_overrides(position, modified_voxels);
    let pool = AsyncComputeTaskPool::get();

    let task = pool.spawn(async move { generate_chunk(position, &generator, &overrides) });

    pending.tasks.push(task);
    pending.pending_positions.insert(position);
}

/// Collect modified voxels that fall within this chunk's padded region.
fn collect_chunk_overrides(
    chunk_pos: IVec3,
    modified_voxels: &HashMap<IVec3, WorldVoxel>,
) -> Vec<(IVec3, WorldVoxel)> {
    let min = chunk_pos * CHUNK_SIZE as i32 - IVec3::ONE;
    let max = min + IVec3::splat(PADDED_CHUNK_SIZE as i32);

    modified_voxels
        .iter()
        .filter(|(pos, _)| {
            pos.x >= min.x
                && pos.x < max.x
                && pos.y >= min.y
                && pos.y < max.y
                && pos.z >= min.z
                && pos.z < max.z
        })
        .map(|(&pos, &voxel)| (pos, voxel))
        .collect()
}

const PADDED_CHUNK_SIZE: i32 = 18;

fn generate_chunk(
    position: IVec3,
    generator: &SdfGenerator,
    overrides: &[(IVec3, WorldVoxel)],
) -> ChunkGenResult {
    let mut sdf = generator(position);
    apply_overrides(&mut sdf, position, overrides);
    let mesh = SurfaceNetsMesher.mesh_chunk(&sdf);
    ChunkGenResult { position, mesh }
}

/// Override SDF values for modified voxels.
/// Solid → large negative (well inside surface), Air → large positive (well outside surface).
fn apply_overrides(sdf: &mut [f32], chunk_pos: IVec3, overrides: &[(IVec3, WorldVoxel)]) {
    let chunk_origin = chunk_pos * CHUNK_SIZE as i32;

    for &(world_pos, voxel) in overrides {
        let local = world_pos - chunk_origin;
        let padded = [
            (local.x + 1) as u32,
            (local.y + 1) as u32,
            (local.z + 1) as u32,
        ];
        let index = PaddedChunkShape::linearize(padded) as usize;

        if index < sdf.len() {
            sdf[index] = match voxel {
                WorldVoxel::Solid(_) => -1.0,
                WorldVoxel::Air | WorldVoxel::Unset => 1.0,
            };
        }
    }
}
