use bevy::prelude::*;

/// Marker on chunk mesh entities (children of map entity).
#[derive(Component)]
pub struct VoxelChunk {
    pub position: IVec3,
    pub lod_level: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

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
