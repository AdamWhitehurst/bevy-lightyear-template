use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use voxel_map_engine::prelude::{TerrainBrushMode, TerrainBrushShape, VoxelType};

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

/// One concrete voxel change accepted by the server.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
#[type_path = "protocol::map"]
pub struct VoxelChange {
    pub position: IVec3,
    pub voxel: VoxelType,
}

/// Client requests one logical terrain brush edit.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct VoxelBrushEditRequest {
    pub sequence: u32,
    pub anchor: IVec3,
    pub shape: TerrainBrushShape,
    pub width: u32,
    pub height: u32,
    pub mode: TerrainBrushMode,
    pub material: u8,
}

/// Server broadcasts voxel edit to all clients.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct VoxelEditBroadcast {
    pub position: IVec3,
    pub voxel: VoxelType,
}

/// Server acknowledges a terrain edit and returns concrete accepted changes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct VoxelEditAck {
    pub sequence: u32,
    pub changes: Vec<VoxelChange>,
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
