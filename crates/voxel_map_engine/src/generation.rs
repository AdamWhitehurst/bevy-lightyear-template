use std::path::PathBuf;
use std::sync::Arc;

use bevy::log::info_span;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use crate::config::{SurfaceHeightMap, VoxelGenerator, VoxelGeneratorImpl, WorldObjectSpawn};
use crate::meshing::mesh_chunk_greedy;
use crate::palette::PalettedChunk;
use crate::types::{ChunkData, ChunkStatus, FillType, WorldVoxel};

/// Number of chunks to generate per async task.
pub const GEN_BATCH_SIZE: usize = 8;

/// Result of an async chunk generation task.
pub struct ChunkGenResult {
    pub position: IVec3,
    pub mesh: Option<Mesh>,
    /// `None` for stages that update chunk status in-place (Features).
    pub chunk_data: Option<ChunkData>,
    pub entity_spawns: Vec<WorldObjectSpawn>,
    /// Whether this chunk was loaded from disk rather than generated.
    pub from_disk: bool,
}

/// Pending async chunk generation tasks for a map entity.
#[derive(Component, Default)]
pub struct PendingChunks {
    pub tasks: Vec<Task<Vec<ChunkGenResult>>>,
}

/// Queued entity spawns from completed Features stages, awaiting server-side processing.
#[derive(Component, Default)]
pub struct PendingEntitySpawns(pub Vec<(IVec3, Vec<WorldObjectSpawn>)>);

/// Spawn an async task that generates terrain for a batch of chunks.
///
/// Each position is first checked on disk; disk-loaded chunks return at their
/// saved status with mesh if non-empty (fast path). Newly generated chunks
/// produce `ChunkStatus::Terrain` with no mesh.
pub fn spawn_terrain_batch(
    pending: &mut PendingChunks,
    positions: Vec<IVec3>,
    generator: &VoxelGenerator,
    save_dir: Option<PathBuf>,
) {
    let generator = Arc::clone(&generator.0);
    let pool = AsyncComputeTaskPool::get();

    let task = pool.spawn(async move {
        let _span = info_span!("terrain_batch", count = positions.len()).entered();
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
                            let entity_spawns = if let Some(ref dir) = save_dir {
                                match crate::persistence::load_chunk_entities(dir, pos) {
                                    Ok(Some(spawns)) => spawns,
                                    Ok(None) => vec![],
                                    Err(e) => {
                                        bevy::log::warn!("Failed to load entities at {pos}: {e}");
                                        vec![]
                                    }
                                }
                            } else {
                                vec![]
                            };
                            return ChunkGenResult {
                                position: pos,
                                mesh,
                                chunk_data: Some(chunk_data),
                                entity_spawns,
                                from_disk: true,
                            };
                        }
                        Ok(None) => {}
                        Err(e) => {
                            bevy::log::warn!("Failed to load chunk at {pos}: {e}, regenerating");
                        }
                    }
                }
                generate_terrain(pos, &*generator)
            })
            .collect()
    });

    pending.tasks.push(task);
}

/// Spawn an async task that runs the Features stage for a single chunk.
///
/// Tries loading entity data from disk first (generate-once, save-forever).
/// If no saved entities exist, runs the generator's `place_features`.
/// Returns a result with `chunk_data: None` (status update is handled
/// in-place by the caller) and any entity spawns.
pub fn spawn_features_task(
    pending: &mut PendingChunks,
    position: IVec3,
    height_map: SurfaceHeightMap,
    generator: &VoxelGenerator,
    save_dir: Option<PathBuf>,
) {
    let generator = Arc::clone(&generator.0);
    let pool = AsyncComputeTaskPool::get();

    let task = pool.spawn(async move {
        let _span = info_span!("features_stage", ?position).entered();
        let entity_spawns = if let Some(ref dir) = save_dir {
            match crate::persistence::load_chunk_entities(dir, position) {
                Ok(Some(spawns)) => spawns,
                Ok(None) => generator.place_features(position, &height_map),
                Err(e) => {
                    bevy::log::warn!("Failed to load entities at {position}: {e}");
                    generator.place_features(position, &height_map)
                }
            }
        } else {
            generator.place_features(position, &height_map)
        };
        vec![ChunkGenResult {
            position,
            mesh: None,
            chunk_data: None,
            entity_spawns,
            from_disk: false,
        }]
    });

    pending.tasks.push(task);
}

/// Spawn an async task that meshes a chunk from its voxel data.
///
/// Returns a result with `ChunkData` at `ChunkStatus::Mesh`.
pub fn spawn_mesh_task(pending: &mut PendingChunks, position: IVec3, voxels: Vec<WorldVoxel>) {
    let pool = AsyncComputeTaskPool::get();

    let task = pool.spawn(async move {
        let _span = info_span!("mesh_stage", ?position).entered();
        let mesh = {
            let _span = info_span!("mesh_chunk").entered();
            mesh_chunk_greedy(&voxels)
        };
        let chunk_data = {
            let _span = info_span!("palettize_chunk").entered();
            ChunkData::from_voxels(&voxels, ChunkStatus::Mesh)
        };
        vec![ChunkGenResult {
            position,
            mesh,
            chunk_data: Some(chunk_data),
            entity_spawns: vec![],
            from_disk: false,
        }]
    });

    pending.tasks.push(task);
}

/// Build a 16x16 surface height map from palettized chunk data.
///
/// Expands the palette to a full voxel array, then scans each XZ column
/// top-down for the highest solid voxel. Called on the main thread before
/// dispatching the Features async task.
pub fn build_surface_height_map(chunk_pos: IVec3, palette: &PalettedChunk) -> SurfaceHeightMap {
    use crate::types::{CHUNK_SIZE, PADDED_CHUNK_SIZE, PaddedChunkShape};
    use ndshape::ConstShape;

    let voxels = palette.to_voxels();
    let mut heights = [None; 256];

    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let px = x + 1;
            let pz = z + 1;
            for py in (0..PADDED_CHUNK_SIZE).rev() {
                let idx = PaddedChunkShape::linearize([px, py, pz]) as usize;
                if matches!(voxels[idx], WorldVoxel::Solid(_)) {
                    let world_y = chunk_pos.y as f64 * CHUNK_SIZE as f64 + py as f64 - 1.0 + 1.0;
                    heights[(x * CHUNK_SIZE + z) as usize] = Some(world_y);
                    break;
                }
            }
        }
    }

    SurfaceHeightMap { chunk_pos, heights }
}

/// Generate terrain-only for a single chunk position.
fn generate_terrain(position: IVec3, generator: &dyn VoxelGeneratorImpl) -> ChunkGenResult {
    let voxels = {
        let _span = info_span!("terrain_gen").entered();
        generator.generate_terrain(position)
    };
    let chunk_data = {
        let _span = info_span!("palettize_chunk").entered();
        ChunkData::from_voxels(&voxels, ChunkStatus::Terrain)
    };
    ChunkGenResult {
        position,
        mesh: None,
        chunk_data: Some(chunk_data),
        entity_spawns: vec![],
        from_disk: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meshing::flat_terrain_voxels;
    use crate::types::CHUNK_SIZE;

    #[test]
    fn surface_height_map_flat_terrain_at_origin() {
        let chunk_pos = IVec3::ZERO;
        let voxels = flat_terrain_voxels(chunk_pos);
        let palette = PalettedChunk::from_voxels(&voxels);
        let map = build_surface_height_map(chunk_pos, &palette);

        assert_eq!(map.chunk_pos, chunk_pos);
        // flat_terrain_voxels places surface at y=0, so all columns should have height
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let h = map.heights[(x * CHUNK_SIZE + z) as usize];
                assert!(h.is_some(), "expected surface at ({x}, {z})");
            }
        }
    }

    #[test]
    fn surface_height_map_all_air_chunk() {
        let chunk_pos = IVec3::new(0, 100, 0);
        let voxels = flat_terrain_voxels(chunk_pos);
        let palette = PalettedChunk::from_voxels(&voxels);
        let map = build_surface_height_map(chunk_pos, &palette);

        // chunk_pos.y=100 → world_y ~1600..1616, well above flat terrain surface
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let h = map.heights[(x * CHUNK_SIZE + z) as usize];
                assert!(
                    h.is_none(),
                    "expected no surface at ({x}, {z}) for sky chunk"
                );
            }
        }
    }

    #[test]
    fn surface_height_map_consistent_height_across_columns() {
        let chunk_pos = IVec3::ZERO;
        let voxels = flat_terrain_voxels(chunk_pos);
        let palette = PalettedChunk::from_voxels(&voxels);
        let map = build_surface_height_map(chunk_pos, &palette);

        // All columns on flat terrain should have the same height
        let first = map.heights[0].unwrap();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let h = map.heights[(x * CHUNK_SIZE + z) as usize].unwrap();
                assert_eq!(h, first, "height mismatch at ({x}, {z})");
            }
        }
    }
}
