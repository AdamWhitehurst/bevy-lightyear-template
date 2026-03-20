use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;

use crate::types::WorldVoxel;

/// The chunk generation function for a map instance.
///
/// Separate component from `VoxelMapConfig` so maps can exist without a
/// generator while terrain components are being applied (deferred commands).
#[derive(Component, Clone)]
pub struct VoxelGenerator(pub Arc<dyn Fn(IVec3) -> Vec<WorldVoxel> + Send + Sync>);

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
