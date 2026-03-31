use bevy::prelude::*;
use grid_tree::{NodeKey, OctreeI32, VisitCommand};
use ndshape::ConstShape;
use std::collections::{HashMap, HashSet};

use crate::api::voxel_to_chunk_pos;
use crate::config::VoxelMapConfig;
use crate::types::{CHUNK_SIZE, ChunkData, PaddedChunkShape, WorldVoxel};

/// Marker: this map is the shared overworld.
#[derive(Component)]
pub struct Overworld;

/// Marker: this map is a player's homebase.
#[derive(Component)]
pub struct Homebase {
    /// PeerId bits -- using u64 because voxel_map_engine doesn't depend on lightyear.
    pub owner: u64,
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
    /// Effective level per loaded column (level ≤ LOAD_LEVEL_THRESHOLD only).
    pub chunk_levels: HashMap<IVec2, u32>,
    /// Chunks with unsaved voxel modifications.
    pub dirty_chunks: HashSet<IVec3>,
    /// Chunks that need async remeshing after in-place mutation.
    pub chunks_needing_remesh: HashSet<IVec3>,
    pub debug_colors: bool,
}

impl VoxelMapInstance {
    pub fn new(tree_height: u32) -> Self {
        Self {
            tree: OctreeI32::new(tree_height as u8),
            chunk_levels: HashMap::new(),
            dirty_chunks: HashSet::new(),
            chunks_needing_remesh: HashSet::new(),
            debug_colors: false,
        }
    }

    /// Bundle for an unbounded overworld map.
    pub fn overworld(seed: u64) -> (Self, VoxelMapConfig, Overworld) {
        let tree_height = 5;
        (
            Self::new(tree_height),
            VoxelMapConfig::new(seed, 0, 10, None, tree_height),
            Overworld,
        )
    }

    /// Bundle for a player's bounded homebase map.
    pub fn homebase(owner_id: u64, bounds: IVec3) -> (Self, VoxelMapConfig, Homebase) {
        let tree_height = 3;
        let spawning_distance = bounds_to_spawning_distance(bounds);
        (
            Self::new(tree_height),
            VoxelMapConfig::new(
                seed_from_id(owner_id),
                0,
                spawning_distance,
                Some(bounds),
                tree_height,
            ),
            Homebase { owner: owner_id },
        )
    }

    /// Bundle for a bounded competition arena map.
    pub fn arena(id: u64, seed: u64, bounds: IVec3) -> (Self, VoxelMapConfig, Arena) {
        let tree_height = 3;
        let spawning_distance = bounds_to_spawning_distance(bounds);
        (
            Self::new(tree_height),
            VoxelMapConfig::new(seed, 0, spawning_distance, Some(bounds), tree_height),
            Arena { id },
        )
    }
}

/// Octree chunk data operations.
impl VoxelMapInstance {
    /// Insert chunk data into the octree at the given chunk position.
    pub fn insert_chunk_data(&mut self, chunk_pos: IVec3, data: ChunkData) {
        let key = NodeKey::new(0, chunk_pos);
        self.tree.fill_path_to_node_from_root(key, |_key, entry| {
            entry.or_insert_with(|| None);
            VisitCommand::Continue
        });
        let relation = self.tree.find_node(key).expect("just created path");
        *self
            .tree
            .get_value_mut(relation.child)
            .expect("just created node") = Some(data);
    }

    /// Remove chunk data from the octree. Returns the data if it existed.
    pub fn remove_chunk_data(&mut self, chunk_pos: IVec3) -> Option<ChunkData> {
        let key = NodeKey::new(0, chunk_pos);
        let relation = self.tree.find_node(key)?;
        let value = self.tree.get_value_mut(relation.child)?;
        value.take()
    }

    /// Get a reference to chunk data in the octree.
    pub fn get_chunk_data(&self, chunk_pos: IVec3) -> Option<&ChunkData> {
        let key = NodeKey::new(0, chunk_pos);
        let relation = self.tree.find_node(key)?;
        self.tree.get_value(relation.child)?.as_ref()
    }

    /// Get a mutable reference to chunk data in the octree.
    pub fn get_chunk_data_mut(&mut self, chunk_pos: IVec3) -> Option<&mut ChunkData> {
        let key = NodeKey::new(0, chunk_pos);
        let relation = self.tree.find_node(key)?;
        self.tree.get_value_mut(relation.child)?.as_mut()
    }

    /// Mutate a voxel directly in the octree. Marks the chunk dirty and queues
    /// it for async remesh. Also updates neighbor chunk padding for boundary voxels.
    /// If the chunk is not loaded, the edit is silently dropped.
    pub fn set_voxel(&mut self, world_pos: IVec3, voxel: WorldVoxel) {
        let chunk_pos = voxel_to_chunk_pos(world_pos);
        let local = world_pos - chunk_pos * CHUNK_SIZE as i32;

        {
            let Some(chunk_data) = self.get_chunk_data_mut(chunk_pos) else {
                trace!("set_voxel: chunk {chunk_pos} not loaded, edit at {world_pos} dropped");
                return;
            };
            let padded = [
                (local.x + 1) as u32,
                (local.y + 1) as u32,
                (local.z + 1) as u32,
            ];
            let index = PaddedChunkShape::linearize(padded) as usize;
            chunk_data.voxels.set(index, voxel);
        }

        self.dirty_chunks.insert(chunk_pos);
        self.chunks_needing_remesh.insert(chunk_pos);

        self.update_neighbor_padding(chunk_pos, local, voxel);
    }

    /// Update padding voxels in neighboring chunks when a boundary voxel is modified.
    fn update_neighbor_padding(&mut self, chunk_pos: IVec3, local: IVec3, voxel: WorldVoxel) {
        for axis in 0..3 {
            let l = local[axis];
            if l == 0 {
                let mut neighbor = chunk_pos;
                neighbor[axis] -= 1;
                if let Some(nd) = self.get_chunk_data_mut(neighbor) {
                    let mut pl = local;
                    pl[axis] = CHUNK_SIZE as i32;
                    let padded = [(pl.x + 1) as u32, (pl.y + 1) as u32, (pl.z + 1) as u32];
                    let idx = PaddedChunkShape::linearize(padded) as usize;
                    nd.voxels.set(idx, voxel);
                }
                self.chunks_needing_remesh.insert(neighbor);
            }
            if l == CHUNK_SIZE as i32 - 1 {
                let mut neighbor = chunk_pos;
                neighbor[axis] += 1;
                if let Some(nd) = self.get_chunk_data_mut(neighbor) {
                    let mut pl = local;
                    pl[axis] = -1;
                    let padded = [(pl.x + 1) as u32, (pl.y + 1) as u32, (pl.z + 1) as u32];
                    let idx = PaddedChunkShape::linearize(padded) as usize;
                    nd.voxels.set(idx, voxel);
                }
                self.chunks_needing_remesh.insert(neighbor);
            }
        }
    }
}

fn bounds_to_spawning_distance(bounds: IVec3) -> u32 {
    bounds.max_element().max(1) as u32
}

/// Derives a deterministic seed from a player/entity ID.
pub fn seed_from_id(id: u64) -> u64 {
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChunkStatus, FillType};

    #[test]
    fn new_creates_empty_instance() {
        let instance = VoxelMapInstance::new(3);
        assert!(instance.chunk_levels.is_empty());
        assert!(instance.dirty_chunks.is_empty());
        assert!(instance.chunks_needing_remesh.is_empty());
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
        let (instance, config, _marker) = VoxelMapInstance::overworld(42);
        assert_eq!(config.seed, 42);
        assert_eq!(config.tree_height, 5);
        assert_eq!(config.spawning_distance, 10);
        assert!(config.bounds.is_none());
        assert!(instance.chunk_levels.is_empty());
    }

    #[test]
    fn homebase_bundle_has_correct_config() {
        let owner_id: u64 = 7;
        let bounds = IVec3::new(4, 8, 6);
        let (instance, config, marker) = VoxelMapInstance::homebase(owner_id, bounds);
        assert_eq!(config.seed, owner_id);
        assert_eq!(config.tree_height, 3);
        assert_eq!(config.spawning_distance, 8);
        assert_eq!(config.bounds, Some(bounds));
        assert_eq!(marker.owner, owner_id);
        assert!(instance.chunk_levels.is_empty());
    }

    #[test]
    fn arena_bundle_has_correct_config() {
        let bounds = IVec3::new(3, 5, 4);
        let (instance, config, marker) = VoxelMapInstance::arena(99, 123, bounds);
        assert_eq!(config.seed, 123);
        assert_eq!(config.tree_height, 3);
        assert_eq!(config.spawning_distance, 5);
        assert_eq!(config.bounds, Some(bounds));
        assert_eq!(marker.id, 99);
        assert!(instance.chunk_levels.is_empty());
    }

    #[test]
    fn bounds_to_spawning_distance_uses_max_axis() {
        assert_eq!(bounds_to_spawning_distance(IVec3::new(1, 2, 3)), 3);
        assert_eq!(bounds_to_spawning_distance(IVec3::new(10, 1, 1)), 10);
        assert_eq!(bounds_to_spawning_distance(IVec3::new(1, 1, 1)), 1);
    }

    #[test]
    fn homebase_seed_deterministic() {
        let id: u64 = 12345;
        let bounds = IVec3::new(4, 4, 4);
        let (_, config1, _) = VoxelMapInstance::homebase(id, bounds);
        let (_, config2, _) = VoxelMapInstance::homebase(id, bounds);
        assert_eq!(config1.seed, config2.seed);
    }

    #[test]
    fn different_owners_different_seeds() {
        let bounds = IVec3::new(4, 4, 4);
        let (_, config1, _) = VoxelMapInstance::homebase(1, bounds);
        let (_, config2, _) = VoxelMapInstance::homebase(2, bounds);
        assert_ne!(config1.seed, config2.seed);
    }

    #[test]
    fn insert_and_retrieve_chunk_data() {
        let mut instance = VoxelMapInstance::new(5);
        let pos = IVec3::new(1, 0, 2);
        let chunk = ChunkData::new_empty();
        instance.insert_chunk_data(pos, chunk);
        assert!(instance.get_chunk_data(pos).is_some());
        assert_eq!(
            instance.get_chunk_data(pos).unwrap().fill_type,
            FillType::Empty
        );
    }

    #[test]
    fn remove_chunk_data_returns_data() {
        let mut instance = VoxelMapInstance::new(5);
        let pos = IVec3::ZERO;
        instance.insert_chunk_data(pos, ChunkData::new_empty());
        let removed = instance.remove_chunk_data(pos);
        assert!(removed.is_some());
        assert!(instance.get_chunk_data(pos).is_none());
    }

    #[test]
    fn remove_nonexistent_chunk_returns_none() {
        let mut instance = VoxelMapInstance::new(5);
        assert!(instance.remove_chunk_data(IVec3::ZERO).is_none());
    }

    #[test]
    fn get_nonexistent_chunk_returns_none() {
        let instance = VoxelMapInstance::new(5);
        assert!(instance.get_chunk_data(IVec3::new(99, 99, 99)).is_none());
    }

    #[test]
    fn overwrite_chunk_data() {
        let mut instance = VoxelMapInstance::new(5);
        let pos = IVec3::ZERO;
        instance.insert_chunk_data(pos, ChunkData::new_empty());

        let mut voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        voxels[0] = WorldVoxel::Solid(1);
        let solid_chunk = ChunkData::from_voxels(&voxels, ChunkStatus::Full);
        instance.insert_chunk_data(pos, solid_chunk);

        let data = instance.get_chunk_data(pos).unwrap();
        assert_eq!(data.fill_type, FillType::Mixed);
        assert_eq!(data.voxels.get(0), WorldVoxel::Solid(1));
    }

    #[test]
    fn set_voxel_mutates_octree_in_place() {
        let mut instance = VoxelMapInstance::new(5);
        let chunk_pos = IVec3::ZERO;
        let voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        instance.insert_chunk_data(
            chunk_pos,
            ChunkData::from_voxels(&voxels, ChunkStatus::Full),
        );

        let world_pos = IVec3::new(5, 5, 5);
        instance.set_voxel(world_pos, WorldVoxel::Solid(42));

        let data = instance.get_chunk_data(chunk_pos).unwrap();
        let local = world_pos - chunk_pos * CHUNK_SIZE as i32;
        let padded = [
            (local.x + 1) as u32,
            (local.y + 1) as u32,
            (local.z + 1) as u32,
        ];
        let index = PaddedChunkShape::linearize(padded) as usize;
        assert_eq!(data.voxels.get(index), WorldVoxel::Solid(42));
        assert!(instance.dirty_chunks.contains(&chunk_pos));
        assert!(instance.chunks_needing_remesh.contains(&chunk_pos));
    }

    #[test]
    fn set_voxel_on_unloaded_chunk_is_dropped() {
        let mut instance = VoxelMapInstance::new(5);
        instance.set_voxel(IVec3::new(5, 5, 5), WorldVoxel::Solid(1));
        assert!(instance.dirty_chunks.is_empty());
        assert!(instance.chunks_needing_remesh.is_empty());
    }

    #[test]
    fn multiple_edits_same_chunk_single_remesh() {
        let mut instance = VoxelMapInstance::new(5);
        let chunk_pos = IVec3::ZERO;
        let voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        instance.insert_chunk_data(
            chunk_pos,
            ChunkData::from_voxels(&voxels, ChunkStatus::Full),
        );

        instance.set_voxel(IVec3::new(1, 1, 1), WorldVoxel::Solid(1));
        instance.set_voxel(IVec3::new(2, 2, 2), WorldVoxel::Solid(2));
        instance.set_voxel(IVec3::new(3, 3, 3), WorldVoxel::Solid(3));

        assert_eq!(instance.chunks_needing_remesh.len(), 1);
        assert!(instance.chunks_needing_remesh.contains(&chunk_pos));
    }

    #[test]
    fn boundary_edit_updates_neighbor_padding() {
        let mut instance = VoxelMapInstance::new(5);
        let chunk_a = IVec3::ZERO;
        let chunk_b = IVec3::X; // neighbor in +x direction

        let voxels_a = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        let voxels_b = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        instance.insert_chunk_data(
            chunk_a,
            ChunkData::from_voxels(&voxels_a, ChunkStatus::Full),
        );
        instance.insert_chunk_data(
            chunk_b,
            ChunkData::from_voxels(&voxels_b, ChunkStatus::Full),
        );

        // Edit at x=15 (last voxel in chunk_a along x) — boundary with chunk_b
        let world_pos = IVec3::new(15, 5, 5);
        instance.set_voxel(world_pos, WorldVoxel::Solid(7));

        // Owning chunk should have the voxel at padded [16, 6, 6]
        let data_a = instance.get_chunk_data(chunk_a).unwrap();
        let idx_a = PaddedChunkShape::linearize([16, 6, 6]) as usize;
        assert_eq!(data_a.voxels.get(idx_a), WorldVoxel::Solid(7));

        // Neighbor chunk_b should have padding updated at padded [0, 6, 6]
        let data_b = instance.get_chunk_data(chunk_b).unwrap();
        let idx_b = PaddedChunkShape::linearize([0, 6, 6]) as usize;
        assert_eq!(
            data_b.voxels.get(idx_b),
            WorldVoxel::Solid(7),
            "neighbor chunk padding should mirror boundary edit"
        );

        // Both chunks should need remesh
        assert!(instance.chunks_needing_remesh.contains(&chunk_a));
        assert!(instance.chunks_needing_remesh.contains(&chunk_b));
    }

    #[test]
    fn boundary_edit_low_edge_updates_neighbor_padding() {
        let mut instance = VoxelMapInstance::new(5);
        let chunk_a = IVec3::ZERO;
        let chunk_neg = -IVec3::X; // neighbor in -x direction

        let voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
        instance.insert_chunk_data(chunk_a, ChunkData::from_voxels(&voxels, ChunkStatus::Full));
        instance.insert_chunk_data(
            chunk_neg,
            ChunkData::from_voxels(&voxels, ChunkStatus::Full),
        );

        // Edit at x=0 (first voxel in chunk_a along x) — boundary with chunk_neg
        let world_pos = IVec3::new(0, 3, 3);
        instance.set_voxel(world_pos, WorldVoxel::Solid(11));

        // Owning chunk should have the voxel at padded [1, 4, 4]
        let data_a = instance.get_chunk_data(chunk_a).unwrap();
        let idx_a = PaddedChunkShape::linearize([1, 4, 4]) as usize;
        assert_eq!(data_a.voxels.get(idx_a), WorldVoxel::Solid(11));

        // Neighbor chunk_neg should have padding updated at padded [17, 4, 4]
        let data_neg = instance.get_chunk_data(chunk_neg).unwrap();
        let idx_neg = PaddedChunkShape::linearize([17, 4, 4]) as usize;
        assert_eq!(
            data_neg.voxels.get(idx_neg),
            WorldVoxel::Solid(11),
            "negative neighbor padding should mirror boundary edit"
        );

        assert!(instance.chunks_needing_remesh.contains(&chunk_a));
        assert!(instance.chunks_needing_remesh.contains(&chunk_neg));
    }
}
