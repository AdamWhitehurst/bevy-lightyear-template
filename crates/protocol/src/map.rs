use bevy::prelude::*;
use serde::{Deserialize, Serialize};
pub use voxel_map_engine::prelude::VoxelType;

/// Channel for voxel editing messages
pub struct VoxelChannel;

/// Shared voxel world configuration for server and client
#[derive(Resource, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct MapWorld {
    pub seed: u64,
    pub generation_version: u32,
}

impl Default for MapWorld {
    fn default() -> Self {
        Self {
            seed: 999,
            generation_version: 0,
        }
    }
}

/// Client requests a voxel edit (admin only)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct VoxelEditRequest {
    pub position: IVec3,
    pub voxel: VoxelType,
}

/// Server broadcasts voxel edit to all clients
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct VoxelEditBroadcast {
    pub position: IVec3,
    pub voxel: VoxelType,
}

/// Server sends all modifications to connecting client
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
pub struct VoxelStateSync {
    pub modifications: Vec<(IVec3, VoxelType)>,
}

/// Temporarily stubbed -- will be restored with VoxelChunk component
pub fn attach_chunk_colliders() {}
