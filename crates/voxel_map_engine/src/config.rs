use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::types::WorldVoxel;

/// Trait for multi-stage chunk generation.
///
/// Implementors produce terrain voxels and optionally place entity-based features.
/// Each method corresponds to a pipeline stage.
pub trait VoxelGeneratorImpl: Send + Sync {
    /// Stage 1: Base terrain shape. Returns 18³ padded voxel array.
    fn generate_terrain(&self, chunk_pos: IVec3) -> Vec<WorldVoxel>;

    /// Stage 2: Entity placement on terrain surface.
    /// Receives a 16×16 surface height map (not raw voxels). Default: no features.
    fn place_features(
        &self,
        _chunk_pos: IVec3,
        _heights: &SurfaceHeightMap,
    ) -> Vec<WorldObjectSpawn> {
        Vec::new()
    }
}

/// Spawn data for a world object placed during the Features stage.
///
/// Uses bare `String` for `object_id` (not `WorldObjectId`) because `WorldObjectId`
/// lives in the `protocol` crate, and `voxel_map_engine` must not depend on it.
/// The server spawn system converts to `WorldObjectId` at the boundary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldObjectSpawn {
    pub object_id: String,
    pub position: Vec3,
    /// RON-serialized persisted components. Empty for fresh spawns.
    #[serde(default)]
    pub persisted_components: Vec<PersistedComponent>,
}

/// A single persisted component: type path + RON data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedComponent {
    pub type_path: String,
    pub ron_data: String,
}

/// `CHUNK_SIZE` × `CHUNK_SIZE` surface height map built from PalettedChunk on the main thread.
/// `heights[x * CHUNK_SIZE + z]` = world Y of highest solid voxel, or `None` if all air.
pub struct SurfaceHeightMap {
    pub chunk_pos: IVec3,
    pub heights: [Option<f64>; 256],
}

/// The chunk generation implementation for a map instance.
///
/// Separate component from `VoxelMapConfig` so maps can exist without a
/// generator while terrain components are being applied (deferred commands).
#[derive(Component, Clone)]
pub struct VoxelGenerator(pub Arc<dyn VoxelGeneratorImpl>);

/// Configuration for a map instance.
#[derive(Component)]
pub struct VoxelMapConfig {
    pub seed: u64,
    /// Tracks the version of the generation algorithm for save compatibility.
    pub generation_version: u32,
    pub spawning_distance: u32,
    pub bounds: Option<IVec3>,
    pub tree_height: u32,
    /// Directory for persisting chunk data. `None` means no persistence.
    pub save_dir: Option<PathBuf>,
    /// Whether this map generates chunks locally. Server sets `true`, client sets `false`
    /// when chunks are streamed from the server.
    pub generates_chunks: bool,
}

impl VoxelMapConfig {
    pub fn new(
        seed: u64,
        generation_version: u32,
        spawning_distance: u32,
        bounds: Option<IVec3>,
        tree_height: u32,
    ) -> Self {
        debug_assert!(tree_height > 0, "VoxelMapConfig: tree_height must be > 0");
        debug_assert!(
            spawning_distance > 0,
            "VoxelMapConfig: spawning_distance must be > 0"
        );
        if let Some(b) = bounds {
            debug_assert!(
                b.x > 0 && b.y > 0 && b.z > 0,
                "VoxelMapConfig: bounded maps must have all-positive bounds, got {b}"
            );
        }
        Self {
            seed,
            generation_version,
            spawning_distance,
            bounds,
            tree_height,
            save_dir: None,
            generates_chunks: true,
        }
    }
}
