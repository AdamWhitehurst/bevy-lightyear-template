use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
pub use voxel_map_engine::prelude::{VoxelChunk, VoxelType};

/// Channel for voxel editing messages
pub struct VoxelChannel;

/// Shared voxel world configuration for server and client
#[derive(Resource, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct MapWorld {
    pub seed: u64,
    pub generation_version: u32,
}

impl Default for MapWorld {
    fn default() -> Self {
        Self {
            seed: 999,
            generation_version: 0,
        }
    }
}

/// Identifies which map instance an entity belongs to.
/// Semantic enum — safe to replicate, no Entity references.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, Reflect)]
#[require(ActiveCollisionHooks::FILTER_PAIRS)]
pub enum MapInstanceId {
    Overworld,
    Homebase { owner: u64 },
}

/// Maps semantic `MapInstanceId` to local `VoxelMapInstance` entities.
/// Each side (server/client) maintains independently.
#[derive(Resource, Default)]
pub struct MapRegistry(pub HashMap<MapInstanceId, Entity>);

impl MapRegistry {
    pub fn get(&self, id: &MapInstanceId) -> Entity {
        *self
            .0
            .get(id)
            .unwrap_or_else(|| panic!("MapRegistry lookup failed for {id:?} — map not registered"))
    }

    pub fn insert(&mut self, id: MapInstanceId, entity: Entity) {
        self.0.insert(id, entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_instance_id_equality() {
        assert_eq!(MapInstanceId::Overworld, MapInstanceId::Overworld);
        assert_ne!(
            MapInstanceId::Overworld,
            MapInstanceId::Homebase { owner: 0 }
        );
    }

    #[test]
    fn map_registry_get_panics_on_missing() {
        let registry = MapRegistry::default();
        let result = std::panic::catch_unwind(|| registry.get(&MapInstanceId::Overworld));
        assert!(result.is_err());
    }

    #[test]
    fn map_registry_insert_and_get() {
        let mut registry = MapRegistry::default();
        let entity = Entity::from_bits(42);
        registry.insert(MapInstanceId::Overworld, entity);
        assert_eq!(registry.get(&MapInstanceId::Overworld), entity);
    }
}

/// Client requests a voxel edit (admin only)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct VoxelEditRequest {
    pub position: IVec3,
    pub voxel: VoxelType,
}

/// Server broadcasts voxel edit to all clients
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct VoxelEditBroadcast {
    pub position: IVec3,
    pub voxel: VoxelType,
}

/// Server sends all modifications to connecting client
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
pub struct VoxelStateSync {
    pub modifications: Vec<(IVec3, VoxelType)>,
}

/// Attaches trimesh colliders to voxel chunks whenever their mesh changes.
/// Inherits `MapInstanceId` from the parent map entity.
pub fn attach_chunk_colliders(
    mut commands: Commands,
    chunks: Query<
        (Entity, &Mesh3d, &ChildOf, Option<&Collider>),
        (With<VoxelChunk>, Or<(Changed<Mesh3d>, Added<Mesh3d>)>),
    >,
    map_ids: Query<&MapInstanceId>,
    meshes: Res<Assets<Mesh>>,
) {
    for (entity, mesh_handle, child_of, existing_collider) in chunks.iter() {
        let Some(mesh) = meshes.get(&mesh_handle.0) else {
            warn!("Chunk entity {entity:?} has Mesh3d but mesh asset not found");
            continue;
        };
        let Some(collider) = Collider::trimesh_from_mesh(mesh) else {
            warn!("Failed to create trimesh collider for chunk entity {entity:?}");
            continue;
        };
        if existing_collider.is_some() {
            commands.entity(entity).remove::<Collider>();
        }
        let mut bundle = commands.entity(entity);
        bundle.insert((
            collider,
            RigidBody::Static,
            crate::hit_detection::terrain_collision_layers(),
        ));
        let map_instance_id = map_ids
            .get(child_of.parent())
            .expect("Chunk parent map entity must have MapInstanceId");

        bundle.insert(map_instance_id.clone());
    }
}
