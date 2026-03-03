use bevy::prelude::*;

/// Marker on chunk mesh entities (children of map entity).
#[derive(Component)]
pub struct VoxelChunk {
    pub position: IVec3,
    pub lod_level: u8,
}

/// Attach to entities whose Transform drives chunk loading for a specific map.
#[derive(Component)]
pub struct ChunkTarget {
    pub map_entity: Entity,
    pub distance: u32,
}

impl ChunkTarget {
    pub fn new(map_entity: Entity, distance: u32) -> Self {
        debug_assert!(
            map_entity != Entity::PLACEHOLDER,
            "ChunkTarget::new called with Entity::PLACEHOLDER — must point to a real map entity"
        );
        Self {
            map_entity,
            distance,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_target_construction() {
        let target = ChunkTarget {
            map_entity: Entity::PLACEHOLDER,
            distance: 4,
        };
        assert_eq!(target.distance, 4);
    }

    #[test]
    fn voxel_chunk_construction() {
        let chunk = VoxelChunk {
            position: IVec3::new(1, 2, 3),
            lod_level: 0,
        };
        assert_eq!(chunk.position, IVec3::new(1, 2, 3));
        assert_eq!(chunk.lod_level, 0);
    }
}
