mod chunk;
mod colliders;
mod persistence;
mod transition;
mod types;
mod voxel;

use bevy::prelude::*;

pub use voxel_map_engine::prelude::{VoxelChunk, VoxelType};

pub use chunk::{ChunkChannel, ChunkDataSync, UnloadColumn};
pub use colliders::attach_chunk_colliders;
pub use persistence::{MapSaveTarget, SavedEntity, SavedEntityKind};
pub use transition::{
    MapChannel, MapTransitionEnd, MapTransitionReady, MapTransitionStart, PendingTransition,
    PlayerMapSwitchRequest, TransitionReadySent,
};
pub use types::{MapInstanceId, MapRegistry, MapSwitchTarget};
pub use voxel::{
    SectionBlocksUpdate, VoxelChannel, VoxelEditAck, VoxelEditBroadcast, VoxelEditReject,
    VoxelEditRequest,
};

/// Tags an entity as belonging to a specific chunk on a specific map.
/// Used to save/despawn entities when their chunk is evicted.
#[derive(Component, Clone, Debug)]
pub struct ChunkEntityRef {
    pub chunk_pos: IVec3,
    pub map_entity: Entity,
}
