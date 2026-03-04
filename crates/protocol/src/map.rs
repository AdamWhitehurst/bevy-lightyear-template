use avian3d::prelude::*;
use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
pub use voxel_map_engine::prelude::{ChunkTarget, VoxelChunk, VoxelType};

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

/// Channel for map transition messages
pub struct MapChannel;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
pub enum MapSwitchTarget {
    Overworld,
    Homebase,
}

/// Client requests to switch maps
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct PlayerMapSwitchRequest {
    pub target: MapSwitchTarget,
}

/// Server notifies client that a map transition is starting
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct MapTransitionStart {
    pub target: MapSwitchTarget,
}

/// Identifies which map instance a physics entity belongs to.
/// The inner Entity points to the VoxelMapInstance entity.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
#[require(ActiveCollisionHooks::FILTER_PAIRS)]
pub struct MapInstanceId(pub Entity);

impl MapEntities for MapInstanceId {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.0 = entity_mapper.get_mapped(self.0);
    }
}

/// Resource tracking the primary overworld map entity.
#[derive(Resource)]
pub struct OverworldMap(pub Entity);

/// Attaches trimesh colliders to voxel chunks whenever their mesh changes.
pub fn attach_chunk_colliders(
    mut commands: Commands,
    chunks: Query<
        (Entity, &Mesh3d, &ChildOf, Option<&Collider>),
        (With<VoxelChunk>, Or<(Changed<Mesh3d>, Added<Mesh3d>)>),
    >,
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
        commands.entity(entity).insert((
            collider,
            RigidBody::Static,
            crate::hit_detection::terrain_collision_layers(),
            MapInstanceId(child_of.parent()),
        ));
    }
}
