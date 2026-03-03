use bevy::prelude::*;
use grid_tree::OctreeI32;
use std::collections::{HashMap, HashSet};

use crate::config::{SdfGenerator, VoxelMapConfig};
use crate::types::{ChunkData, WorldVoxel};

/// Marker: this map is the shared overworld.
#[derive(Component)]
pub struct Overworld;

/// Marker: this map is a player's homebase.
#[derive(Component)]
pub struct Homebase {
    pub owner: Entity,
}

/// Marker: this map is a competition arena.
#[derive(Component)]
pub struct Arena {
    pub id: u64,
}

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

    /// Bundle for an unbounded overworld map.
    pub fn overworld(seed: u64, generator: SdfGenerator) -> (Self, VoxelMapConfig, Overworld) {
        let tree_height = 5;
        (
            Self::new(tree_height),
            VoxelMapConfig::new(seed, 10, None, tree_height, generator),
            Overworld,
        )
    }

    /// Bundle for a player's bounded homebase map.
    pub fn homebase(
        owner: Entity,
        bounds: IVec3,
        generator: SdfGenerator,
    ) -> (Self, VoxelMapConfig, Homebase) {
        let tree_height = 3;
        let spawning_distance = bounds_to_spawning_distance(bounds);
        (
            Self::new(tree_height),
            VoxelMapConfig::new(
                seed_from_entity(owner),
                spawning_distance,
                Some(bounds),
                tree_height,
                generator,
            ),
            Homebase { owner },
        )
    }

    /// Bundle for a bounded competition arena map.
    pub fn arena(
        id: u64,
        seed: u64,
        bounds: IVec3,
        generator: SdfGenerator,
    ) -> (Self, VoxelMapConfig, Arena) {
        let tree_height = 3;
        let spawning_distance = bounds_to_spawning_distance(bounds);
        (
            Self::new(tree_height),
            VoxelMapConfig::new(
                seed,
                spawning_distance,
                Some(bounds),
                tree_height,
                generator,
            ),
            Arena { id },
        )
    }
}

fn bounds_to_spawning_distance(bounds: IVec3) -> u32 {
    bounds.max_element().max(1) as u32
}

fn seed_from_entity(entity: Entity) -> u64 {
    entity.to_bits()
}

#[cfg(test)]
mod tests {
    use super::*;
    use grid_tree::{NodeKey, VisitCommand};
    use std::sync::Arc;

    fn dummy_generator() -> SdfGenerator {
        Arc::new(|_| vec![1.0])
    }

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

    #[test]
    fn overworld_bundle_has_correct_config() {
        let (instance, config, _marker) = VoxelMapInstance::overworld(42, dummy_generator());
        assert_eq!(config.seed, 42);
        assert_eq!(config.tree_height, 5);
        assert_eq!(config.spawning_distance, 10);
        assert!(config.bounds.is_none());
        assert!(instance.loaded_chunks.is_empty());
    }

    #[test]
    fn homebase_bundle_has_correct_config() {
        let owner = Entity::from_bits(7);
        let bounds = IVec3::new(4, 8, 6);
        let (instance, config, marker) =
            VoxelMapInstance::homebase(owner, bounds, dummy_generator());
        assert_eq!(config.seed, owner.to_bits());
        assert_eq!(config.tree_height, 3);
        assert_eq!(config.spawning_distance, 8);
        assert_eq!(config.bounds, Some(bounds));
        assert_eq!(marker.owner, owner);
        assert!(instance.loaded_chunks.is_empty());
    }

    #[test]
    fn arena_bundle_has_correct_config() {
        let bounds = IVec3::new(3, 5, 4);
        let (instance, config, marker) =
            VoxelMapInstance::arena(99, 123, bounds, dummy_generator());
        assert_eq!(config.seed, 123);
        assert_eq!(config.tree_height, 3);
        assert_eq!(config.spawning_distance, 5);
        assert_eq!(config.bounds, Some(bounds));
        assert_eq!(marker.id, 99);
        assert!(instance.loaded_chunks.is_empty());
    }

    #[test]
    fn bounds_to_spawning_distance_uses_max_axis() {
        assert_eq!(bounds_to_spawning_distance(IVec3::new(1, 2, 3)), 3);
        assert_eq!(bounds_to_spawning_distance(IVec3::new(10, 1, 1)), 10);
        assert_eq!(bounds_to_spawning_distance(IVec3::new(1, 1, 1)), 1);
    }
}
