use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
pub use voxel_map_engine::prelude::{PalettedChunk, VoxelChunk, VoxelType};

/// Channel for voxel editing messages
pub struct VoxelChannel;

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

/// Channel for map transition messages
pub struct MapChannel;

/// Client requests to switch maps
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct PlayerMapSwitchRequest {
    pub target: MapSwitchTarget,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
pub enum MapSwitchTarget {
    Overworld,
    Homebase,
}

/// Marks a player entity as undergoing a map transition.
/// Carried on the player entity on both client and server.
#[derive(Component, Clone, Debug)]
pub struct PendingTransition(pub MapInstanceId);

/// Server tells client to begin transition
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
pub struct MapTransitionStart {
    pub target: MapInstanceId,
    pub seed: u64,
    pub generation_version: u32,
    pub bounds: Option<IVec3>,
    pub spawn_position: Vec3,
}

/// Client tells server that chunks are loaded and it's ready
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct MapTransitionReady;

/// Server tells client the transition is complete, player is unfrozen
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct MapTransitionEnd;

/// Marker: client has sent MapTransitionReady, awaiting MapTransitionEnd
#[derive(Component)]
pub struct TransitionReadySent;

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

/// Server tells client to discard a chunk (left view range).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct ChunkUnload {
    pub chunk_pos: IVec3,
}

/// Marker: this entity should be saved with its map.
#[derive(Component, Clone, Debug, Default)]
pub struct MapSaveTarget;

/// Identifies the type of a saved entity for reconstruction.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum SavedEntityKind {
    RespawnPoint,
}

/// A single entity serialized for persistence.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SavedEntity {
    pub kind: SavedEntityKind,
    pub position: Vec3,
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
