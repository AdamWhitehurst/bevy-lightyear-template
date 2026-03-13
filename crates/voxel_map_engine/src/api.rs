use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use ndshape::ConstShape;

use crate::config::VoxelMapConfig;
use crate::instance::VoxelMapInstance;
use crate::raycast::{VoxelRaycastResult, voxel_line_traversal};
use crate::types::{CHUNK_SIZE, PaddedChunkShape, WorldVoxel};

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
    /// Checks the octree first, then evaluates the voxel generator as fallback.
    pub fn get_voxel(&self, map: Entity, pos: IVec3) -> WorldVoxel {
        let Ok((instance, config)) = self.maps.get(map) else {
            warn!("get_voxel: entity {map:?} has no VoxelMapInstance");
            return WorldVoxel::Unset;
        };

        let chunk_pos = voxel_to_chunk_pos(pos);
        if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
            let local = pos - chunk_pos * CHUNK_SIZE as i32;
            let padded = [
                (local.x + 1) as u32,
                (local.y + 1) as u32,
                (local.z + 1) as u32,
            ];
            let index = PaddedChunkShape::linearize(padded) as usize;
            return chunk_data.voxels.get(index);
        }

        evaluate_voxel_at(pos, &config.generator)
    }

    /// Mutate a voxel directly in the octree. Marks the chunk dirty and queues remesh.
    pub fn set_voxel(&mut self, map: Entity, pos: IVec3, voxel: WorldVoxel) {
        debug_assert!(
            voxel != WorldVoxel::Unset,
            "set_voxel: cannot write Unset (internal sentinel)"
        );

        let Ok((mut instance, _)) = self.maps.get_mut(map) else {
            warn!("set_voxel: entity {map:?} has no VoxelMapInstance");
            return;
        };

        instance.set_voxel(pos, voxel);
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

        let mut cached_chunk: Option<(IVec3, Vec<WorldVoxel>)> = None;

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
                return false;
            }
            true
        });

        result
    }
}

/// Look up a voxel at a world position, checking octree first then generator cache.
fn lookup_voxel(
    voxel_pos: IVec3,
    instance: &VoxelMapInstance,
    generator: &crate::config::VoxelGenerator,
    cached_chunk: &mut Option<(IVec3, Vec<WorldVoxel>)>,
) -> WorldVoxel {
    let chunk_pos = voxel_to_chunk_pos(voxel_pos);

    if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
        let local = voxel_pos - chunk_pos * CHUNK_SIZE as i32;
        let padded = [
            (local.x + 1) as u32,
            (local.y + 1) as u32,
            (local.z + 1) as u32,
        ];
        let index = PaddedChunkShape::linearize(padded) as usize;
        return chunk_data.voxels.get(index);
    }

    let needs_generate = match cached_chunk.as_ref() {
        Some((cached_pos, _)) if *cached_pos == chunk_pos => false,
        _ => true,
    };
    if needs_generate {
        *cached_chunk = Some((chunk_pos, generator(chunk_pos)));
    }

    let (_, voxels) = cached_chunk.as_ref().unwrap();
    lookup_voxel_in_chunk(voxels, voxel_pos, chunk_pos)
}

/// Evaluate the voxel generator at a single world-space position.
fn evaluate_voxel_at(pos: IVec3, generator: &crate::config::VoxelGenerator) -> WorldVoxel {
    let chunk_pos = voxel_to_chunk_pos(pos);
    let voxels = generator(chunk_pos);
    lookup_voxel_in_chunk(&voxels, pos, chunk_pos)
}

/// Index into a flat voxel array to get the voxel at a world position within the given chunk.
fn lookup_voxel_in_chunk(voxels: &[WorldVoxel], voxel_pos: IVec3, chunk_pos: IVec3) -> WorldVoxel {
    let local = voxel_pos - chunk_pos * CHUNK_SIZE as i32;
    let padded = [
        (local.x + 1) as u32,
        (local.y + 1) as u32,
        (local.z + 1) as u32,
    ];
    let index = PaddedChunkShape::linearize(padded) as usize;

    if index < voxels.len() {
        voxels[index]
    } else {
        WorldVoxel::Unset
    }
}

/// Converts a world-space voxel position to the chunk coordinate containing it.
pub fn voxel_to_chunk_pos(voxel_pos: IVec3) -> IVec3 {
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
    fn evaluate_voxel_flat_terrain() {
        use crate::meshing::flat_terrain_voxels;
        use std::sync::Arc;
        let generator: crate::config::VoxelGenerator = Arc::new(flat_terrain_voxels);

        let voxel = evaluate_voxel_at(IVec3::new(0, -1, 0), &generator);
        assert_eq!(voxel, WorldVoxel::Solid(0));

        let voxel = evaluate_voxel_at(IVec3::new(0, 1, 0), &generator);
        assert_eq!(voxel, WorldVoxel::Air);
    }

    #[test]
    fn lookup_voxel_in_chunk_roundtrip() {
        use crate::meshing::flat_terrain_voxels;
        let chunk_pos = IVec3::ZERO;
        let voxels = flat_terrain_voxels(chunk_pos);

        let voxel = lookup_voxel_in_chunk(&voxels, IVec3::new(0, -1, 0), chunk_pos);
        assert_eq!(voxel, WorldVoxel::Solid(0));

        let voxel = lookup_voxel_in_chunk(&voxels, IVec3::new(0, 5, 0), chunk_pos);
        assert_eq!(voxel, WorldVoxel::Air);
    }
}
