use bevy::prelude::*;
use ndshape::ConstShape;
use serde::{Deserialize, Serialize};

/// 16^3 voxel chunks with 1-voxel padding on each side -> 18^3 padded array
pub type PaddedChunkShape = ndshape::ConstShape3u32<18, 18, 18>;

pub const CHUNK_SIZE: u32 = 16;
pub const PADDED_CHUNK_SIZE: u32 = 18;

/// Voxel data stored per position
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub enum WorldVoxel {
    Air,
    Unset,
    Solid(u8),
}

impl Default for WorldVoxel {
    fn default() -> Self {
        Self::Unset
    }
}

/// How a chunk is filled (optimization for uniform chunks)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillType {
    Empty,
    Mixed,
    Uniform(WorldVoxel),
}

/// Voxel data for one chunk (16^3 with 1-voxel padding = 18^3)
#[derive(Clone)]
pub struct ChunkData {
    pub voxels: Vec<WorldVoxel>,
    pub fill_type: FillType,
    pub hash: u64,
}

impl ChunkData {
    pub fn new_empty() -> Self {
        Self {
            voxels: vec![WorldVoxel::Air; PaddedChunkShape::SIZE as usize],
            fill_type: FillType::Empty,
            hash: 0,
        }
    }
}

/// Network-serializable voxel type (mirrors WorldVoxel without Unset)
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub enum VoxelType {
    Air,
    Solid(u8),
}

impl From<VoxelType> for WorldVoxel {
    fn from(v: VoxelType) -> Self {
        match v {
            VoxelType::Air => WorldVoxel::Air,
            VoxelType::Solid(m) => WorldVoxel::Solid(m),
        }
    }
}

impl From<WorldVoxel> for VoxelType {
    fn from(v: WorldVoxel) -> Self {
        match v {
            WorldVoxel::Air | WorldVoxel::Unset => VoxelType::Air,
            WorldVoxel::Solid(m) => VoxelType::Solid(m),
        }
    }
}
