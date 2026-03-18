use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use voxel_map_engine::prelude::VoxelType;

/// Channel for voxel editing messages
pub struct VoxelChannel;

/// Client requests a voxel edit (admin only).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct VoxelEditRequest {
    pub position: IVec3,
    pub voxel: VoxelType,
    pub sequence: u32,
}

/// Server broadcasts voxel edit to all clients.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct VoxelEditBroadcast {
    pub position: IVec3,
    pub voxel: VoxelType,
}

/// Server acknowledges a block edit up to this sequence number.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct VoxelEditAck {
    pub sequence: u32,
}

/// Server rejects a block edit — client must roll back.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct VoxelEditReject {
    pub sequence: u32,
    pub position: IVec3,
    pub correct_voxel: VoxelType,
}

/// Batched block changes for a single chunk, sent when 2+ changes happen in one tick.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct SectionBlocksUpdate {
    pub chunk_pos: IVec3,
    pub changes: Vec<(IVec3, VoxelType)>,
}
