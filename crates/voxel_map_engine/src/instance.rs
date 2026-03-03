use bevy::prelude::*;
use grid_tree::OctreeI32;
use std::collections::{HashMap, HashSet};

use crate::types::{ChunkData, WorldVoxel};

/// Core component on every map entity. Owns the spatial index and per-instance state.
#[derive(Component)]
pub struct VoxelMapInstance {
    pub tree: OctreeI32<Option<ChunkData>>,
    pub modified_voxels: HashMap<IVec3, WorldVoxel>,
    pub write_buffer: Vec<(IVec3, WorldVoxel)>,
    pub loaded_chunks: HashSet<IVec3>,
    pub debug_colors: bool,
}

impl VoxelMapInstance {
    pub fn new(tree_height: u32) -> Self {
        Self {
            tree: OctreeI32::new(tree_height as u8),
            modified_voxels: HashMap::new(),
            write_buffer: Vec::new(),
            loaded_chunks: HashSet::new(),
            debug_colors: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grid_tree::{NodeKey, VisitCommand};

    #[test]
    fn new_creates_empty_instance() {
        let instance = VoxelMapInstance::new(3);
        assert!(instance.write_buffer.is_empty());
        assert!(instance.modified_voxels.is_empty());
    }

    #[test]
    fn insert_and_find_chunk() {
        let mut instance = VoxelMapInstance::new(3);
        let key = NodeKey::new(0, IVec3::new(1, 0, 2));

        instance
            .tree
            .fill_path_to_node_from_root(key, |_node_key, entry| {
                entry.or_insert_with(|| None);
                VisitCommand::Continue
            });

        let relation = instance.tree.find_node(key).unwrap();
        *instance.tree.get_value_mut(relation.child).unwrap() = Some(ChunkData::new_empty());

        let found = instance.tree.find_node(key);
        assert!(found.is_some());
        let data = instance.tree.get_value(found.unwrap().child).unwrap();
        assert!(data.is_some());
    }
}
