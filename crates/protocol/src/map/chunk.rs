use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use voxel_map_engine::prelude::PalettedChunk;

use super::types::MapInstanceId;

/// Channel for chunk data streaming.
pub struct ChunkChannel;

/// Server sends a full chunk's palette-compressed data to a client.
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
pub struct ChunkDataSync {
    pub map_id: MapInstanceId,
    pub chunk_pos: IVec3,
    pub data: PalettedChunk,
}

/// Server tells client to drop all chunks in a column.
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
pub struct UnloadColumn {
    pub map_id: MapInstanceId,
    pub column: IVec2,
}
