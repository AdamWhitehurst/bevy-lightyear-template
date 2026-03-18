use std::collections::HashMap;

use bevy::asset::{LoadContext, RenderAssetUsages};
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;

use super::meshing::{mesh_vox_model, mesh_vox_model_from_dense};
use super::types::VoxModelVoxel;

/// Minimum dimension size below which no further LODs are generated.
const MIN_LOD_DIMENSION: u32 = 4;

/// Maximum number of LOD levels (LOD 0 through LOD 2).
const MAX_LOD_LEVELS: usize = 3;

/// Generates LOD meshes from a `.vox` model and registers them as labeled sub-assets.
///
/// LOD 0 is the full-resolution mesh. Each subsequent LOD downsamples by 2x in all axes.
/// Generation stops when any axis would drop below [`MIN_LOD_DIMENSION`] or after LOD 2.
///
/// Returns handles to the registered meshes, ordered by LOD level.
pub fn generate_lod_meshes(
    model: &dot_vox::Model,
    palette: &[dot_vox::Color],
    load_context: &mut LoadContext<'_>,
) -> Vec<Handle<Mesh>> {
    let mut handles = Vec::new();

    let lod0_mesh = mesh_vox_model(model, palette);
    handles.push(register_lod_mesh(load_context, lod0_mesh, 0));

    let (mut dense, mut size) = rasterize_to_dense(model);

    for lod_level in 1..MAX_LOD_LEVELS {
        if !can_downsample(size) {
            trace!(
                lod_level,
                ?size,
                "Stopping LOD generation: dimension below minimum"
            );
            break;
        }

        (dense, size) = downsample_2x(&dense, size);
        let mesh = mesh_vox_model_from_dense(&dense, size, palette);
        handles.push(register_lod_mesh(load_context, mesh, lod_level));
    }

    handles
}

/// Registers a mesh as a labeled sub-asset with the label `"mesh_lod{level}"`.
///
/// If the mesh is `None` (no visible voxels), an empty triangle-list mesh is registered instead.
fn register_lod_mesh(
    load_context: &mut LoadContext<'_>,
    mesh: Option<Mesh>,
    level: usize,
) -> Handle<Mesh> {
    let label = format!("mesh_lod{level}");
    let mesh = mesh.unwrap_or_else(|| {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
    });
    load_context.add_labeled_asset(label, mesh)
}

/// Returns `true` if all dimensions are at least `2 * MIN_LOD_DIMENSION` (so the
/// downsampled result is at least `MIN_LOD_DIMENSION` in every axis).
fn can_downsample(size: UVec3) -> bool {
    size.x >= MIN_LOD_DIMENSION * 2
        && size.y >= MIN_LOD_DIMENSION * 2
        && size.z >= MIN_LOD_DIMENSION * 2
}

/// Rasterizes sparse `dot_vox` voxels into a flat dense array in Bevy Y-up coordinates.
///
/// MagicaVoxel `(x, y, z)` maps to Bevy `(x, z, y)`.
/// Index formula: `x + y * size.x + z * size.x * size.y`.
fn rasterize_to_dense(model: &dot_vox::Model) -> (Vec<VoxModelVoxel>, UVec3) {
    let size = UVec3::new(model.size.x, model.size.z, model.size.y);
    let len = (size.x * size.y * size.z) as usize;
    let mut voxels = vec![VoxModelVoxel::Empty; len];

    for v in &model.voxels {
        let (x, y, z) = (u32::from(v.x), u32::from(v.z), u32::from(v.y));
        let idx = (x + y * size.x + z * size.x * size.y) as usize;
        debug_assert!(idx < voxels.len(), "voxel position out of bounds");
        voxels[idx] = VoxModelVoxel::Filled(v.i);
    }

    (voxels, size)
}

/// Downsamples a dense voxel array by 2x in each axis using majority-vote.
///
/// Each 2x2x2 block in the input maps to one voxel in the output.
/// The palette index that appears most often in the block wins.
/// Ties are broken arbitrarily. Blocks with no filled voxels produce `Empty`.
fn downsample_2x(voxels: &[VoxModelVoxel], size: UVec3) -> (Vec<VoxModelVoxel>, UVec3) {
    let new_size = size / 2;
    let len = (new_size.x * new_size.y * new_size.z) as usize;
    let mut output = vec![VoxModelVoxel::Empty; len];

    for nz in 0..new_size.z {
        for ny in 0..new_size.y {
            for nx in 0..new_size.x {
                let voxel = majority_vote_block(voxels, size, nx * 2, ny * 2, nz * 2);
                let dst = (nx + ny * new_size.x + nz * new_size.x * new_size.y) as usize;
                output[dst] = voxel;
            }
        }
    }

    (output, new_size)
}

/// Determines the majority-vote winner for a 2x2x2 block starting at `(bx, by, bz)`.
///
/// Counts filled voxels by palette index and returns the index with the highest count.
/// Returns `Empty` if no voxels in the block are filled.
fn majority_vote_block(
    voxels: &[VoxModelVoxel],
    size: UVec3,
    bx: u32,
    by: u32,
    bz: u32,
) -> VoxModelVoxel {
    let mut counts: HashMap<u8, u8> = HashMap::new();

    for dz in 0..2 {
        for dy in 0..2 {
            for dx in 0..2 {
                let (x, y, z) = (bx + dx, by + dy, bz + dz);
                let idx = (x + y * size.x + z * size.x * size.y) as usize;
                if let VoxModelVoxel::Filled(palette_idx) = voxels[idx] {
                    *counts.entry(palette_idx).or_insert(0) += 1;
                }
            }
        }
    }

    counts
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map_or(VoxModelVoxel::Empty, |(idx, _)| VoxModelVoxel::Filled(idx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dot_vox::{Model, Size, Voxel};

    fn filled_cube_model(side: u32, palette_idx: u8) -> Model {
        let mut voxels = Vec::new();
        for x in 0..side as u8 {
            for y in 0..side as u8 {
                for z in 0..side as u8 {
                    voxels.push(Voxel {
                        x,
                        y,
                        z,
                        i: palette_idx,
                    });
                }
            }
        }
        Model {
            size: Size {
                x: side,
                y: side,
                z: side,
            },
            voxels,
        }
    }

    #[test]
    fn rasterize_to_dense_applies_coord_remap() {
        let model = Model {
            size: Size { x: 4, y: 4, z: 4 },
            voxels: vec![Voxel {
                x: 1,
                y: 2,
                z: 3,
                i: 42,
            }],
        };
        let (dense, size) = rasterize_to_dense(&model);

        assert_eq!(size, UVec3::new(4, 4, 4));

        let (bx, by, bz) = (1, 3, 2);
        let idx = (bx + by * size.x + bz * size.x * size.y) as usize;
        assert_eq!(dense[idx], VoxModelVoxel::Filled(42));
    }

    #[test]
    fn rasterize_to_dense_non_cubic() {
        let model = Model {
            size: Size { x: 8, y: 4, z: 6 },
            voxels: vec![Voxel {
                x: 0,
                y: 0,
                z: 0,
                i: 1,
            }],
        };
        let (dense, size) = rasterize_to_dense(&model);

        assert_eq!(size, UVec3::new(8, 6, 4));
        assert_eq!(dense.len(), (8 * 6 * 4) as usize);
        assert_eq!(dense[0], VoxModelVoxel::Filled(1));
    }

    #[test]
    fn downsample_solid_cube() {
        let model = filled_cube_model(8, 5);
        let (dense, size) = rasterize_to_dense(&model);

        let (down, new_size) = downsample_2x(&dense, size);

        assert_eq!(new_size, UVec3::new(4, 4, 4));
        assert!(
            down.iter().all(|v| *v == VoxModelVoxel::Filled(5)),
            "all voxels in a solid downsampled cube should be filled with the same index"
        );
    }

    #[test]
    fn downsample_empty_stays_empty() {
        let size = UVec3::new(8, 8, 8);
        let voxels = vec![VoxModelVoxel::Empty; (size.x * size.y * size.z) as usize];

        let (down, new_size) = downsample_2x(&voxels, size);

        assert_eq!(new_size, UVec3::new(4, 4, 4));
        assert!(down.iter().all(|v| *v == VoxModelVoxel::Empty));
    }

    #[test]
    fn majority_vote_picks_most_common() {
        let size = UVec3::new(2, 2, 2);
        let voxels = vec![
            VoxModelVoxel::Filled(10), // (0,0,0)
            VoxModelVoxel::Filled(10), // (1,0,0)
            VoxModelVoxel::Filled(10), // (0,1,0)
            VoxModelVoxel::Filled(20), // (1,1,0)
            VoxModelVoxel::Filled(10), // (0,0,1)
            VoxModelVoxel::Filled(20), // (1,0,1)
            VoxModelVoxel::Filled(20), // (0,1,1)
            VoxModelVoxel::Empty,      // (1,1,1)
        ];

        let result = majority_vote_block(&voxels, size, 0, 0, 0);
        assert_eq!(result, VoxModelVoxel::Filled(10));
    }

    #[test]
    fn majority_vote_all_empty_returns_empty() {
        let size = UVec3::new(2, 2, 2);
        let voxels = vec![VoxModelVoxel::Empty; 8];

        let result = majority_vote_block(&voxels, size, 0, 0, 0);
        assert_eq!(result, VoxModelVoxel::Empty);
    }

    #[test]
    fn can_downsample_respects_minimum() {
        assert!(can_downsample(UVec3::new(8, 8, 8)));
        assert!(can_downsample(UVec3::new(8, 10, 8)));
        assert!(!can_downsample(UVec3::new(6, 8, 8)));
        assert!(!can_downsample(UVec3::new(8, 6, 8)));
        assert!(!can_downsample(UVec3::new(8, 8, 6)));
        assert!(!can_downsample(UVec3::new(4, 4, 4)));
    }

    #[test]
    fn downsample_halves_dimensions() {
        let size = UVec3::new(16, 8, 12);
        let voxels = vec![VoxModelVoxel::Empty; (size.x * size.y * size.z) as usize];

        let (_, new_size) = downsample_2x(&voxels, size);
        assert_eq!(new_size, UVec3::new(8, 4, 6));
    }
}
