use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use block_mesh::{greedy_quads, GreedyQuadsBuffer, RIGHT_HANDED_Y_UP_CONFIG};
use ndshape::{RuntimeShape, Shape};

use super::types::VoxModelVoxel;

/// Converts a `dot_vox::Model` into a Bevy `Mesh` centered at the origin.
///
/// The mesh uses `ATTRIBUTE_COLOR` (linear RGBA from the palette) instead of UV coordinates.
/// Pair with a `StandardMaterial { ..default() }` (white base_color passes vertex colors through).
///
/// Returns `None` if the model contains no visible voxels.
pub fn mesh_vox_model(model: &dot_vox::Model, palette: &[dot_vox::Color]) -> Option<Mesh> {
    let padded_dims = padded_dimensions(model);
    let shape = RuntimeShape::<u32, 3>::new(padded_dims);
    let voxels = rasterize_model(model, &shape);

    let mut buffer = GreedyQuadsBuffer::new(shape.usize());
    let max = [padded_dims[0] - 1, padded_dims[1] - 1, padded_dims[2] - 1];
    let faces = RIGHT_HANDED_Y_UP_CONFIG.faces;
    greedy_quads(&voxels, &shape, [0; 3], max, &faces, &mut buffer);

    if buffer.quads.num_quads() == 0 {
        trace!("No quads found in model");
        return None;
    }

    let center = model_center(model);
    build_mesh(&buffer, &faces, &voxels, &shape, palette, center)
}

/// Converts a pre-rasterized dense voxel array into a Bevy `Mesh` centered at the origin.
///
/// The input must be in Bevy Y-up coordinates (no padding, no axis remap needed).
/// Used by LOD generation where the dense array is already downsampled.
///
/// Returns `None` if the array contains no visible voxels.
pub fn mesh_vox_model_from_dense(
    voxels: &[VoxModelVoxel],
    size: UVec3,
    palette: &[dot_vox::Color],
) -> Option<Mesh> {
    let padded = pad_dense_array(voxels, size);
    let padded_dims = [size.x + 2, size.y + 2, size.z + 2];
    let shape = RuntimeShape::<u32, 3>::new(padded_dims);

    let mut buffer = GreedyQuadsBuffer::new(shape.usize());
    let max = [padded_dims[0] - 1, padded_dims[1] - 1, padded_dims[2] - 1];
    let faces = RIGHT_HANDED_Y_UP_CONFIG.faces;
    greedy_quads(&padded, &shape, [0; 3], max, &faces, &mut buffer);

    if buffer.quads.num_quads() == 0 {
        trace!("No quads found in dense voxel array");
        return None;
    }

    let center = Vec3::new(
        size.x as f32 / 2.0,
        size.y as f32 / 2.0,
        size.z as f32 / 2.0,
    );
    build_mesh(&buffer, &faces, &padded, &shape, palette, center)
}

/// Wraps a dense unpadded voxel array in a 1-wide empty border.
fn pad_dense_array(voxels: &[VoxModelVoxel], size: UVec3) -> Vec<VoxModelVoxel> {
    let padded_dims = [size.x + 2, size.y + 2, size.z + 2];
    let shape = RuntimeShape::<u32, 3>::new(padded_dims);
    let mut padded = vec![VoxModelVoxel::Empty; shape.usize()];

    for z in 0..size.z {
        for y in 0..size.y {
            for x in 0..size.x {
                let src_idx = (x + y * size.x + z * size.x * size.y) as usize;
                let dst_idx = shape.linearize([x + 1, y + 1, z + 1]) as usize;
                padded[dst_idx] = voxels[src_idx];
            }
        }
    }

    padded
}

/// Padded dimensions: model size + 2 in each axis (1 border on each side), with Z-up to Y-up remap.
fn padded_dimensions(model: &dot_vox::Model) -> [u32; 3] {
    [model.size.x + 2, model.size.z + 2, model.size.y + 2]
}

/// Center offset for the model (half-extents in Bevy coordinates after Z-up to Y-up remap).
fn model_center(model: &dot_vox::Model) -> Vec3 {
    Vec3::new(
        model.size.x as f32 / 2.0,
        model.size.z as f32 / 2.0,
        model.size.y as f32 / 2.0,
    )
}

/// Rasterize sparse `dot_vox` voxels into a padded dense array.
///
/// Applies the MagicaVoxel Z-up to Bevy Y-up coordinate remap: `(x, y, z) -> (x, z, y)`.
/// Voxels are offset by +1 to sit inside the 1-wide border.
fn rasterize_model(model: &dot_vox::Model, shape: &RuntimeShape<u32, 3>) -> Vec<VoxModelVoxel> {
    let mut voxels = vec![VoxModelVoxel::Empty; shape.usize()];

    for v in &model.voxels {
        let pos = [u32::from(v.x) + 1, u32::from(v.z) + 1, u32::from(v.y) + 1];
        let idx = shape.linearize(pos) as usize;
        debug_assert!(idx < voxels.len(), "voxel position out of bounds");
        voxels[idx] = VoxModelVoxel::Filled(v.i);
    }

    voxels
}

/// Build a Bevy `Mesh` from greedy quads output with vertex colors from the palette.
fn build_mesh(
    buffer: &GreedyQuadsBuffer,
    faces: &[block_mesh::OrientedBlockFace; 6],
    voxels: &[VoxModelVoxel],
    shape: &RuntimeShape<u32, 3>,
    palette: &[dot_vox::Color],
    center: Vec3,
) -> Option<Mesh> {
    let num_vertices = buffer.quads.num_quads() * 4;
    let num_indices = buffer.quads.num_quads() * 6;

    let mut positions = Vec::with_capacity(num_vertices);
    let mut normals = Vec::with_capacity(num_vertices);
    let mut colors = Vec::with_capacity(num_vertices);
    let mut indices = Vec::with_capacity(num_indices);

    for (group, face) in buffer.quads.groups.iter().zip(faces.iter()) {
        for quad in group.iter() {
            let color = palette_color_for_quad(quad, voxels, shape, palette);

            indices.extend_from_slice(&face.quad_mesh_indices(positions.len() as u32));
            for pos in face.quad_mesh_positions(quad, 1.0) {
                positions.push((Vec3::from_array(pos) - center).to_array());
            }
            normals.extend_from_slice(&face.quad_mesh_normals());
            colors.extend_from_slice(&[color; 4]);
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
    mesh.try_insert_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .expect("valid color attribute");
    mesh.try_insert_indices(Indices::U32(indices))
        .expect("valid indices");
    Some(mesh)
}

/// Look up the palette color for a quad by reading the voxel at `quad.minimum`.
fn palette_color_for_quad(
    quad: &block_mesh::UnorientedQuad,
    voxels: &[VoxModelVoxel],
    shape: &RuntimeShape<u32, 3>,
    palette: &[dot_vox::Color],
) -> [f32; 4] {
    let linear = shape.linearize(quad.minimum) as usize;
    debug_assert!(
        matches!(voxels[linear], VoxModelVoxel::Filled(_)),
        "quad minimum voxel must be filled"
    );
    let palette_idx = match voxels[linear] {
        VoxModelVoxel::Filled(idx) => idx as usize,
        VoxModelVoxel::Empty => 0,
    };

    palette
        .get(palette_idx)
        .map(|c| srgb_color_to_linear(c.r, c.g, c.b, c.a))
        .unwrap_or([1.0, 0.0, 1.0, 1.0])
}

/// Convert an sRGB u8 channel value to linear f32.
fn srgb_channel_to_linear(value: u8) -> f32 {
    let s = value as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// Convert sRGB u8 RGBA to linear f32 RGBA. Alpha is not gamma-corrected.
fn srgb_color_to_linear(r: u8, g: u8, b: u8, a: u8) -> [f32; 4] {
    [
        srgb_channel_to_linear(r),
        srgb_channel_to_linear(g),
        srgb_channel_to_linear(b),
        a as f32 / 255.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use dot_vox::{Color, Model, Size, Voxel};
    use ndshape::{RuntimeShape, Shape};

    fn make_palette() -> Vec<Color> {
        (0..=255)
            .map(|i| Color {
                r: i,
                g: 255 - i,
                b: i / 2,
                a: 255,
            })
            .collect()
    }

    fn make_2x2x2_model() -> Model {
        let mut voxels = Vec::new();
        for x in 0..2u8 {
            for y in 0..2u8 {
                for z in 0..2u8 {
                    voxels.push(Voxel {
                        x,
                        y,
                        z,
                        i: x * 4 + y * 2 + z,
                    });
                }
            }
        }
        Model {
            size: Size { x: 2, y: 2, z: 2 },
            voxels,
        }
    }

    #[test]
    fn mesh_2x2x2_has_expected_attributes() {
        let model = make_2x2x2_model();
        let palette = make_palette();
        let mesh = mesh_vox_model(&model, &palette).expect("2x2x2 model should produce a mesh");

        assert!(mesh.attribute(Mesh::ATTRIBUTE_POSITION).is_some());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_NORMAL).is_some());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_some());
        assert!(mesh.indices().is_some());

        let vertex_count = mesh.count_vertices();
        let index_count = mesh.indices().unwrap().len();

        assert_eq!(vertex_count % 4, 0);
        assert_eq!(index_count % 6, 0);
        assert_eq!(index_count / 6, vertex_count / 4);
        assert!(vertex_count > 0, "2x2x2 solid cube should produce vertices");
    }

    #[test]
    fn empty_model_returns_none() {
        let model = Model {
            size: Size { x: 4, y: 4, z: 4 },
            voxels: vec![],
        };
        let palette = make_palette();
        assert!(mesh_vox_model(&model, &palette).is_none());
    }

    #[test]
    fn rasterize_maps_magicavoxel_coords_to_bevy_coords() {
        let model = Model {
            size: Size { x: 4, y: 4, z: 4 },
            voxels: vec![Voxel {
                x: 1,
                y: 2,
                z: 3,
                i: 42,
            }],
        };
        let padded = padded_dimensions(&model);
        let shape = RuntimeShape::<u32, 3>::new(padded);
        let voxels = rasterize_model(&model, &shape);

        let bevy_pos = [1u32 + 1, 3u32 + 1, 2u32 + 1];
        let idx = shape.linearize(bevy_pos) as usize;
        assert_eq!(voxels[idx], VoxModelVoxel::Filled(42));
    }

    #[test]
    fn srgb_channel_to_linear_boundary_values() {
        assert!((srgb_channel_to_linear(0) - 0.0).abs() < 0.001);
        assert!((srgb_channel_to_linear(255) - 1.0).abs() < 0.001);
    }

    #[test]
    fn srgb_channel_to_linear_midpoint() {
        let result = srgb_channel_to_linear(128);
        assert!(
            (result - 0.216).abs() < 0.01,
            "srgb_channel_to_linear(128) = {result}, expected ~0.216"
        );
    }
}
