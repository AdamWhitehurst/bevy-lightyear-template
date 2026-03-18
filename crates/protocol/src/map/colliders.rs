use avian3d::prelude::*;
use bevy::prelude::*;
use voxel_map_engine::prelude::VoxelChunk;

use crate::hit_detection::terrain_collision_layers;

use super::types::MapInstanceId;

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
        bundle.insert((collider, RigidBody::Static, terrain_collision_layers()));
        let map_instance_id = map_ids
            .get(child_of.parent())
            .expect("Chunk parent map entity must have MapInstanceId");

        bundle.insert(map_instance_id.clone());
    }
}
