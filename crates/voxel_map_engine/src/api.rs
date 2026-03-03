use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::config::VoxelMapConfig;
use crate::instance::VoxelMapInstance;
use crate::raycast::{VoxelRaycastResult, voxel_line_traversal};
use crate::types::{CHUNK_SIZE, PaddedChunkShape, WorldVoxel};
use ndshape::ConstShape;

/// SystemParam for reading/writing voxels on any map instance.
///
/// Every operation takes a `map: Entity` parameter to select which map instance to operate on.
#[derive(SystemParam)]
pub struct VoxelWorld<'w, 's> {
    maps: Query<'w, 's, (&'static mut VoxelMapInstance, &'static VoxelMapConfig)>,
}

impl VoxelWorld<'_, '_> {
    /// Get the voxel at a world-space integer position on a specific map instance.
    ///
    /// Checks `modified_voxels` first, then evaluates the SDF generator.
    pub fn get_voxel(&self, map: Entity, pos: IVec3) -> WorldVoxel {
        let Ok((instance, config)) = self.maps.get(map) else {
            warn!("get_voxel: entity {map:?} has no VoxelMapInstance");
            return WorldVoxel::Unset;
        };

        if let Some(&voxel) = instance.modified_voxels.get(&pos) {
            return voxel;
        }

        evaluate_sdf_at(pos, &config.generator)
    }

    /// Queue a voxel write. Applied during `flush_write_buffer` system.
    pub fn set_voxel(&mut self, map: Entity, pos: IVec3, voxel: WorldVoxel) {
        debug_assert!(
            voxel != WorldVoxel::Unset,
            "set_voxel: cannot write Unset (internal sentinel)"
        );

        let Ok((mut instance, _)) = self.maps.get_mut(map) else {
            warn!("set_voxel: entity {map:?} has no VoxelMapInstance");
            return;
        };

        instance.write_buffer.push((pos, voxel));
    }

    /// Raycast against a specific map instance.
    ///
    /// Casts a ray from `ray.origin` in `ray.direction` up to `max_distance`.
    /// Returns the first voxel matching `filter`.
    pub fn raycast(
        &self,
        map: Entity,
        ray: Ray3d,
        max_distance: f32,
        filter: impl Fn(WorldVoxel) -> bool,
    ) -> Option<VoxelRaycastResult> {
        let Ok((instance, config)) = self.maps.get(map) else {
            warn!("raycast: entity {map:?} has no VoxelMapInstance");
            return None;
        };

        let start = ray.origin;
        let end = ray.origin + *ray.direction * max_distance;

        // Cache the last chunk's SDF to avoid re-generating for adjacent voxels
        let mut cached_chunk: Option<(IVec3, Vec<f32>)> = None;

        let mut result = None;

        voxel_line_traversal(start, end, |voxel_pos, t, face| {
            let voxel = lookup_voxel(voxel_pos, &instance, &config.generator, &mut cached_chunk);

            if filter(voxel) {
                result = Some(VoxelRaycastResult {
                    position: voxel_pos,
                    normal: face.normal(),
                    voxel,
                    t,
                });
                return false; // stop traversal
            }
            true
        });

        result
    }
}

/// Look up a voxel at a world position, using the SDF cache for efficiency.
fn lookup_voxel(
    voxel_pos: IVec3,
    instance: &VoxelMapInstance,
    generator: &crate::config::SdfGenerator,
    cached_chunk: &mut Option<(IVec3, Vec<f32>)>,
) -> WorldVoxel {
    if let Some(&voxel) = instance.modified_voxels.get(&voxel_pos) {
        return voxel;
    }

    let chunk_pos = voxel_to_chunk_pos(voxel_pos);

    let needs_generate = match cached_chunk.as_ref() {
        Some((cached_pos, _)) if *cached_pos == chunk_pos => false,
        _ => true,
    };
    if needs_generate {
        *cached_chunk = Some((chunk_pos, generator(chunk_pos)));
    }

    let (_, sdf) = cached_chunk.as_ref().unwrap();
    sdf_to_voxel(sdf, voxel_pos, chunk_pos)
}

/// Evaluate the SDF at a single world-space position and return a WorldVoxel.
fn evaluate_sdf_at(pos: IVec3, generator: &crate::config::SdfGenerator) -> WorldVoxel {
    let chunk_pos = voxel_to_chunk_pos(pos);
    let sdf = generator(chunk_pos);
    sdf_to_voxel(&sdf, pos, chunk_pos)
}

fn sdf_to_voxel(sdf: &[f32], voxel_pos: IVec3, chunk_pos: IVec3) -> WorldVoxel {
    let local = voxel_pos - chunk_pos * CHUNK_SIZE as i32;
    // +1 for padding offset
    let padded = [
        (local.x + 1) as u32,
        (local.y + 1) as u32,
        (local.z + 1) as u32,
    ];
    let index = PaddedChunkShape::linearize(padded) as usize;

    if index < sdf.len() && sdf[index] < 0.0 {
        WorldVoxel::Solid(0)
    } else {
        WorldVoxel::Air
    }
}

pub(crate) fn voxel_to_chunk_pos(voxel_pos: IVec3) -> IVec3 {
    IVec3::new(
        voxel_pos.x.div_euclid(CHUNK_SIZE as i32),
        voxel_pos.y.div_euclid(CHUNK_SIZE as i32),
        voxel_pos.z.div_euclid(CHUNK_SIZE as i32),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voxel_to_chunk_pos_basic() {
        assert_eq!(voxel_to_chunk_pos(IVec3::new(0, 0, 0)), IVec3::ZERO);
        assert_eq!(voxel_to_chunk_pos(IVec3::new(16, 0, 0)), IVec3::X);
        assert_eq!(voxel_to_chunk_pos(IVec3::new(-1, 0, 0)), -IVec3::X);
        assert_eq!(voxel_to_chunk_pos(IVec3::new(15, 0, 0)), IVec3::ZERO);
    }

    #[test]
    fn evaluate_sdf_flat_terrain() {
        use crate::meshing::flat_terrain_sdf;
        use std::sync::Arc;
        let generator: crate::config::SdfGenerator = Arc::new(flat_terrain_sdf);

        // Below surface → solid
        let voxel = evaluate_sdf_at(IVec3::new(0, -1, 0), &generator);
        assert_eq!(voxel, WorldVoxel::Solid(0));

        // Above surface → air
        let voxel = evaluate_sdf_at(IVec3::new(0, 1, 0), &generator);
        assert_eq!(voxel, WorldVoxel::Air);
    }

    #[test]
    fn sdf_to_voxel_roundtrip() {
        use crate::meshing::flat_terrain_sdf;
        let chunk_pos = IVec3::ZERO;
        let sdf = flat_terrain_sdf(chunk_pos);

        // Voxel at y=-1 in chunk 0 should be solid (sdf = -1)
        let voxel = sdf_to_voxel(&sdf, IVec3::new(0, -1, 0), chunk_pos);
        assert_eq!(voxel, WorldVoxel::Solid(0));

        // Voxel at y=5 in chunk 0 should be air (sdf = 5)
        let voxel = sdf_to_voxel(&sdf, IVec3::new(0, 5, 0), chunk_pos);
        assert_eq!(voxel, WorldVoxel::Air);
    }
}
