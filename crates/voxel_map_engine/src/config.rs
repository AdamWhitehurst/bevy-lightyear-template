use bevy::prelude::*;
use std::sync::Arc;

/// Generation function: given chunk position, returns SDF values for the padded 18^3 array.
pub type SdfGenerator = Arc<dyn Fn(IVec3) -> Vec<f32> + Send + Sync>;

/// Configuration for a map instance.
#[derive(Component)]
pub struct VoxelMapConfig {
    pub seed: u64,
    pub spawning_distance: u32,
    pub bounds: Option<IVec3>,
    pub tree_height: u32,
    pub generator: SdfGenerator,
}
