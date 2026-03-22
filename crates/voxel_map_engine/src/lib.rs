pub mod api;
pub mod chunk;
pub mod config;
pub mod generation;
pub mod instance;
pub mod lifecycle;
pub mod mesh_cache;
pub mod meshing;
pub mod palette;
pub mod persistence;
pub mod propagator;
pub mod raycast;
pub mod terrain;
pub mod ticket;
pub mod types;

use bevy::prelude::*;

/// Insert this resource to enable chunk generation systems (propagator,
/// task spawning/polling). Servers insert this; clients omit it since
/// they receive chunks via network push.
#[derive(Resource)]
pub struct ChunkGenerationEnabled;

pub struct VoxelPlugin;

impl Plugin for VoxelPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<terrain::HeightMap>();
        app.register_type::<terrain::MoistureMap>();
        app.register_type::<terrain::BiomeRules>();
        app.register_type::<terrain::BiomeRule>();
        app.register_type::<terrain::NoiseDef>();
        app.register_type::<terrain::NoiseType>();
        app.register_type::<terrain::FractalType>();

        let generation_enabled = resource_exists::<ChunkGenerationEnabled>;

        app.add_systems(Startup, lifecycle::init_default_material);
        app.add_systems(
            Update,
            (
                lifecycle::ensure_pending_chunks,
                (lifecycle::update_chunks, lifecycle::poll_chunk_tasks).run_if(generation_enabled),
                lifecycle::despawn_out_of_range_chunks,
                lifecycle::spawn_remesh_tasks,
                lifecycle::poll_remesh_tasks,
            )
                .chain(),
        );
    }
}

pub mod prelude {
    pub use crate::api::*;
    pub use crate::chunk::*;
    pub use crate::config::*;
    pub use crate::generation::*;
    pub use crate::instance::*;
    pub use crate::lifecycle::DefaultVoxelMaterial;
    pub use crate::mesh_cache::*;
    pub use crate::meshing::*;
    pub use crate::palette::*;
    pub use crate::propagator::*;
    pub use crate::raycast::*;
    pub use crate::terrain::*;
    pub use crate::ticket::*;
    pub use crate::types::*;
    pub use crate::{ChunkGenerationEnabled, VoxelPlugin};
}
