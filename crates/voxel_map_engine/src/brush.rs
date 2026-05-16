use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::raycast::VoxelRaycastResult;

/// Shape used to expand a terrain brush anchor into voxel positions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Default)]
pub enum TerrainBrushShape {
    #[default]
    Rect,
    Sphere,
}

/// Editing behavior applied to a terrain brush footprint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Default)]
pub enum TerrainBrushMode {
    #[default]
    FillAir,
    ReplaceAll,
    PaintExisting,
    Remove,
}

/// Returns the world-space brush anchor implied by a raycast hit and mode.
pub fn brush_anchor(hit: &VoxelRaycastResult, mode: TerrainBrushMode) -> Option<IVec3> {
    match mode {
        TerrainBrushMode::FillAir => hit.normal.map(|normal| hit.position + normal.as_ivec3()),
        TerrainBrushMode::ReplaceAll
        | TerrainBrushMode::PaintExisting
        | TerrainBrushMode::Remove => Some(hit.position),
    }
}

/// Returns deterministic world-space positions covered by a brush.
pub fn brush_footprint(
    anchor: IVec3,
    shape: TerrainBrushShape,
    width: u32,
    height: u32,
) -> Vec<IVec3> {
    match shape {
        TerrainBrushShape::Rect => rect_footprint(anchor, width, height),
        TerrainBrushShape::Sphere => sphere_footprint(anchor, width),
    }
}

fn rect_footprint(anchor: IVec3, width: u32, height: u32) -> Vec<IVec3> {
    let (horizontal_start, horizontal_end) = dimension_offsets(width);
    let (vertical_start, vertical_end) = dimension_offsets(height);
    let mut positions = Vec::new();
    for z in horizontal_start..=horizontal_end {
        for y in vertical_start..=vertical_end {
            for x in horizontal_start..=horizontal_end {
                positions.push(anchor + IVec3::new(x, y, z));
            }
        }
    }
    positions
}

fn sphere_footprint(anchor: IVec3, width: u32) -> Vec<IVec3> {
    let (start, end) = dimension_offsets(width);
    let mut positions = Vec::new();
    for z in start..=end {
        for y in start..=end {
            for x in start..=end {
                let offset = IVec3::new(x, y, z);
                if includes_sphere_offset(offset, start, end) {
                    positions.push(anchor + offset);
                }
            }
        }
    }
    positions
}

fn dimension_offsets(size: u32) -> (i32, i32) {
    let size = size.max(1) as i32;
    let start = -((size - 1) / 2);
    let end = start + size - 1;
    (start, end)
}

fn includes_sphere_offset(offset: IVec3, start: i32, end: i32) -> bool {
    let center = (start + end) as f32 / 2.0;
    let radius = (end - start + 1) as f32 / 2.0;
    let offset = offset.as_vec3() - Vec3::splat(center);
    offset.length_squared() <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WorldVoxel;

    fn hit(position: IVec3, normal: Option<Vec3>) -> VoxelRaycastResult {
        VoxelRaycastResult {
            position,
            normal,
            voxel: WorldVoxel::Solid(0),
            t: 0.0,
        }
    }

    #[test]
    fn rect_width_one_height_one_returns_anchor_only() {
        assert_eq!(
            brush_footprint(IVec3::new(1, 2, 3), TerrainBrushShape::Rect, 1, 1),
            vec![IVec3::new(1, 2, 3)]
        );
    }

    #[test]
    fn rect_width_two_height_one_returns_two_by_two_floor() {
        let anchor = IVec3::new(10, 20, 30);
        let footprint = brush_footprint(anchor, TerrainBrushShape::Rect, 2, 1);
        assert_eq!(footprint.len(), 4);
        assert_eq!(footprint[0], anchor);
        assert_eq!(footprint[3], anchor + IVec3::new(1, 0, 1));
    }

    #[test]
    fn rect_width_two_height_three_returns_twelve_voxels() {
        let anchor = IVec3::new(10, 20, 30);
        let footprint = brush_footprint(anchor, TerrainBrushShape::Rect, 2, 3);
        assert_eq!(footprint.len(), 12);
        assert_eq!(footprint[0], anchor + IVec3::new(0, -1, 0));
        assert_eq!(footprint[11], anchor + IVec3::new(1, 1, 1));
    }

    #[test]
    fn sphere_excludes_cube_corners() {
        let anchor = IVec3::ZERO;
        let footprint = brush_footprint(anchor, TerrainBrushShape::Sphere, 3, 1);
        assert!(!footprint.contains(&IVec3::new(1, 1, 1)));
        assert!(footprint.contains(&IVec3::new(1, 0, 0)));
        assert!(footprint.contains(&anchor));
    }

    #[test]
    fn fill_air_anchor_uses_hit_normal() {
        let hit = hit(IVec3::new(4, 5, 6), Some(Vec3::Y));
        assert_eq!(
            brush_anchor(&hit, TerrainBrushMode::FillAir),
            Some(IVec3::new(4, 6, 6))
        );
    }

    #[test]
    fn remove_paint_replace_anchor_use_hit_position() {
        let hit = hit(IVec3::new(4, 5, 6), Some(Vec3::Y));
        assert_eq!(
            brush_anchor(&hit, TerrainBrushMode::Remove),
            Some(hit.position)
        );
        assert_eq!(
            brush_anchor(&hit, TerrainBrushMode::PaintExisting),
            Some(hit.position)
        );
        assert_eq!(
            brush_anchor(&hit, TerrainBrushMode::ReplaceAll),
            Some(hit.position)
        );
    }
}
