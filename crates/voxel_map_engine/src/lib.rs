pub mod chunk;
pub mod config;
pub mod generation;
pub mod instance;
pub mod lifecycle;
pub mod mesh_cache;
pub mod meshing;
pub mod types;

use bevy::prelude::*;

pub struct VoxelPlugin;

impl Plugin for VoxelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, lifecycle::init_default_material);
        app.add_systems(
            Update,
            (
                lifecycle::update_chunks,
                lifecycle::poll_chunk_tasks,
                lifecycle::despawn_out_of_range_chunks,
                lifecycle::flush_write_buffer,
            )
                .chain(),
        );
    }
}

pub mod prelude {
    pub use crate::VoxelPlugin;
    pub use crate::chunk::*;
    pub use crate::config::*;
    pub use crate::generation::*;
    pub use crate::instance::*;
    pub use crate::lifecycle::DefaultVoxelMaterial;
    pub use crate::mesh_cache::*;
    pub use crate::meshing::*;
    pub use crate::types::*;
}
