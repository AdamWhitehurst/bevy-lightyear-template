use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use voxel_map_engine::prelude::PalettedChunk;

/// Channel for chunk data streaming.
pub struct ChunkChannel;

/// Server sends a full chunk's palette-compressed data to a client.
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
pub struct ChunkDataSync {
    pub chunk_pos: IVec3,
    pub data: PalettedChunk,
}

/// Client requests a chunk from the server.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct ChunkRequest {
    pub chunk_pos: IVec3,
}
