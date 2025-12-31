use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_voxel_world::prelude::*;
use serde::{Deserialize, Serialize};

/// Channel for voxel editing messages
pub struct VoxelChannel;

/// Shared voxel world configuration for server and client
#[derive(Resource, Clone, Default)]
pub struct MapWorld;

impl VoxelWorldConfig for MapWorld {
    type MaterialIndex = u8;
    type ChunkUserBundle = ();

    fn spawning_distance(&self) -> u32 {
        10
    }

    fn voxel_lookup_delegate(&self) -> VoxelLookupDelegate<Self::MaterialIndex> {
        Box::new(|_chunk_pos| {
            Box::new(move |pos: IVec3| {
                // Flat terrain: solid below y=0
                if pos.y < 0 {
                    WorldVoxel::Solid(0)
                } else {
                    WorldVoxel::Air
                }
            })
        })
    }
}

/// Voxel type for network serialization
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
            WorldVoxel::Air => VoxelType::Air,
            WorldVoxel::Solid(m) => VoxelType::Solid(m),
            WorldVoxel::Unset => VoxelType::Air,
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

/// Shared system to attach/update trimesh colliders when chunk meshes are created/updated
pub fn attach_chunk_colliders(
    mut commands: Commands,
    chunks: Query<
        (Entity, &Mesh3d, Option<&Collider>),
        (With<Chunk<MapWorld>>, Or<(Changed<Mesh3d>, Added<Mesh3d>)>),
    >,
    meshes: Res<Assets<Mesh>>,
) {
    for (entity, mesh_handle, existing_collider) in chunks.iter() {
        let Some(mesh) = meshes.get(&mesh_handle.0) else {
            continue;
        };

        let Some(collider) = Collider::trimesh_from_mesh(mesh) else {
            continue;
        };

        if existing_collider.is_some() {
            commands.entity(entity).remove::<Collider>();
        }

        commands
            .entity(entity)
            .insert((collider, RigidBody::Static));
    }
}
