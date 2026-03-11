use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use block_mesh::{GreedyQuadsBuffer, RIGHT_HANDED_Y_UP_CONFIG, greedy_quads};
use ndshape::ConstShape;

use crate::types::{CHUNK_SIZE, PaddedChunkShape, WorldVoxel};

/// Mesh a padded 18^3 voxel array into a Bevy Mesh using greedy quads.
pub fn mesh_chunk_greedy(voxels: &[WorldVoxel]) -> Option<Mesh> {
    debug_assert_eq!(voxels.len(), PaddedChunkShape::USIZE);

    let mut buffer = GreedyQuadsBuffer::new(voxels.len());
    let faces = RIGHT_HANDED_Y_UP_CONFIG.faces;
    greedy_quads(
        voxels,
        &PaddedChunkShape {},
        [0; 3],
        [17; 3],
        &faces,
        &mut buffer,
    );

    if buffer.quads.num_quads() == 0 {
        return None;
    }

    let num_vertices = buffer.quads.num_quads() * 4;
    let num_indices = buffer.quads.num_quads() * 6;

    let mut positions = Vec::with_capacity(num_vertices);
    let mut normals = Vec::with_capacity(num_vertices);
    let mut indices = Vec::with_capacity(num_indices);
    let mut tex_coords = Vec::with_capacity(num_vertices);

    for (group, face) in buffer.quads.groups.iter().zip(faces.iter()) {
        for quad in group.iter() {
            indices.extend_from_slice(&face.quad_mesh_indices(positions.len() as u32));
            positions.extend_from_slice(&face.quad_mesh_positions(quad, 1.0));
            normals.extend_from_slice(&face.quad_mesh_normals());
            tex_coords.extend_from_slice(&face.tex_coords(
                RIGHT_HANDED_Y_UP_CONFIG.u_flip_face,
                true,
                quad,
            ));
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.try_insert_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .expect("valid position attribute");
    mesh.try_insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .expect("valid normal attribute");
    mesh.try_insert_attribute(Mesh::ATTRIBUTE_UV_0, tex_coords)
        .expect("valid uv attribute");
    mesh.try_insert_indices(Indices::U32(indices))
        .expect("valid indices");
    Some(mesh)
}

/// Generate voxels for flat terrain at y=0.
/// world_y <= 0 → Solid(0), world_y > 0 → Air.
pub fn flat_terrain_voxels(chunk_pos: IVec3) -> Vec<WorldVoxel> {
    let mut voxels = vec![WorldVoxel::Air; PaddedChunkShape::USIZE];
    for i in 0..PaddedChunkShape::SIZE {
        let [_x, y, _z] = PaddedChunkShape::delinearize(i);
        let world_y = chunk_pos.y * CHUNK_SIZE as i32 + y as i32 - 1;
        if world_y <= 0 {
            voxels[i as usize] = WorldVoxel::Solid(0);
        }
    }
    voxels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_terrain_voxels_produces_mesh_at_surface() {
        let voxels = flat_terrain_voxels(IVec3::new(0, 0, 0));
        let mesh = mesh_chunk_greedy(&voxels);
        assert!(
            mesh.is_some(),
            "y=0 chunk should contain a surface crossing"
        );
    }

    #[test]
    fn flat_terrain_voxels_no_mesh_for_underground() {
        let voxels = flat_terrain_voxels(IVec3::new(0, -2, 0));
        let mesh = mesh_chunk_greedy(&voxels);
        assert!(
            mesh.is_none(),
            "fully underground chunk should produce no mesh"
        );
    }

    #[test]
    fn flat_terrain_voxels_no_mesh_for_sky() {
        let voxels = flat_terrain_voxels(IVec3::new(0, 2, 0));
        let mesh = mesh_chunk_greedy(&voxels);
        assert!(
            mesh.is_none(),
            "fully above-surface chunk should produce no mesh"
        );
    }

    #[test]
    fn mesh_has_valid_attributes() {
        let voxels = flat_terrain_voxels(IVec3::new(0, 0, 0));
        let mesh = mesh_chunk_greedy(&voxels).expect("should produce mesh");
        assert!(mesh.attribute(Mesh::ATTRIBUTE_POSITION).is_some());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_NORMAL).is_some());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_UV_0).is_some());
        assert!(mesh.indices().is_some());
    }
}
