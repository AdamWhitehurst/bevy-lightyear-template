use bevy::prelude::*;

use crate::types::WorldVoxel;

/// The face through which a voxel was entered by a ray.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VoxelFace {
    None,
    Bottom,
    Top,
    Left,
    Right,
    Back,
    Forward,
}

impl VoxelFace {
    /// Convert to a normal vector pointing outward from the entered face.
    pub fn normal(self) -> Option<Vec3> {
        match self {
            VoxelFace::None => None,
            VoxelFace::Bottom => Some(-Vec3::Y),
            VoxelFace::Top => Some(Vec3::Y),
            VoxelFace::Left => Some(-Vec3::X),
            VoxelFace::Right => Some(Vec3::X),
            VoxelFace::Back => Some(-Vec3::Z),
            VoxelFace::Forward => Some(Vec3::Z),
        }
    }
}

/// Result of a voxel raycast.
#[derive(Clone, Debug)]
pub struct VoxelRaycastResult {
    pub position: IVec3,
    pub normal: Option<Vec3>,
    pub voxel: WorldVoxel,
    /// Normalized time along the ray [0, 1].
    pub t: f32,
}

const VOXEL_SIZE: f32 = 1.0;

/// Amanatides & Woo "Fast Voxel Traversal Algorithm".
///
/// Traverses all voxels along a ray from `start` to `end` (inclusive).
/// Calls `visit_voxel` with (voxel_coord, normalized_t, entered_face).
/// Returns early if `visit_voxel` returns `false`.
///
/// Adapted from bevy_voxel_world's voxel_traversal.rs.
pub fn voxel_line_traversal(
    start: Vec3,
    end: Vec3,
    mut visit_voxel: impl FnMut(IVec3, f32, VoxelFace) -> bool,
) {
    let ray = end - start;
    let end_t = ray.length();
    if end_t < f32::EPSILON {
        return;
    }
    let ray_dir = ray / end_t;
    let r_ray_dir = ray_dir.recip();
    let delta_t = (VOXEL_SIZE * r_ray_dir).abs();

    let step = ray_dir.signum().as_ivec3();

    let start_voxel = start.floor().as_ivec3();
    let end_voxel = end.floor().as_ivec3();

    let mut voxel = start_voxel;
    let mut max_t = compute_initial_max_t(start, start_voxel, step, r_ray_dir, end_t);

    let r_end_t = 1.0 / end_t;
    let mut time = max_t.min_element() * r_end_t;
    let mut face = VoxelFace::None;

    let out_of_bounds = end_voxel + step;
    let mut reached_end = voxel == end_voxel;
    let mut keep_going = visit_voxel(voxel, time, face);

    let faces = step_faces(step);

    while keep_going && !reached_end {
        if max_t.x < max_t.y && max_t.x < max_t.z {
            time = max_t.x * r_end_t;
            face = faces.0;
            voxel.x += step.x;
            max_t.x += delta_t.x;
            reached_end = voxel.x == out_of_bounds.x;
        } else if max_t.y < max_t.z {
            time = max_t.y * r_end_t;
            face = faces.1;
            voxel.y += step.y;
            max_t.y += delta_t.y;
            reached_end = voxel.y == out_of_bounds.y;
        } else {
            time = max_t.z * r_end_t;
            face = faces.2;
            voxel.z += step.z;
            max_t.z += delta_t.z;
            reached_end = voxel.z == out_of_bounds.z;
        }

        if !reached_end {
            keep_going = visit_voxel(voxel, time, face);
        }
    }
}

fn compute_initial_max_t(
    start: Vec3,
    start_voxel: IVec3,
    step: IVec3,
    r_ray_dir: Vec3,
    end_t: f32,
) -> Vec3 {
    let mut max_t = Vec3::ZERO;

    for axis in 0..3 {
        if step[axis] == 0 {
            max_t[axis] = end_t;
        } else {
            let o = if step[axis] > 0 { 1 } else { 0 };
            let plane = (start_voxel[axis] + o) as f32 * VOXEL_SIZE;
            max_t[axis] = (plane - start[axis]) * r_ray_dir[axis];
        }
    }

    max_t
}

fn step_faces(step: IVec3) -> (VoxelFace, VoxelFace, VoxelFace) {
    let x_face = if step.x > 0 {
        VoxelFace::Left
    } else {
        VoxelFace::Right
    };
    let y_face = if step.y > 0 {
        VoxelFace::Bottom
    } else {
        VoxelFace::Top
    };
    let z_face = if step.z > 0 {
        VoxelFace::Back
    } else {
        VoxelFace::Forward
    };
    (x_face, y_face, z_face)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_straight_line_x() {
        let mut visited = Vec::new();
        voxel_line_traversal(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(3.5, 0.5, 0.5),
            |pos, _t, _face| {
                visited.push(pos);
                true
            },
        );
        assert_eq!(
            visited,
            vec![
                IVec3::new(0, 0, 0),
                IVec3::new(1, 0, 0),
                IVec3::new(2, 0, 0),
                IVec3::new(3, 0, 0),
            ]
        );
    }

    #[test]
    fn traversal_reports_faces() {
        let mut faces = Vec::new();
        voxel_line_traversal(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(2.5, 0.5, 0.5),
            |_pos, _t, face| {
                faces.push(face);
                true
            },
        );
        assert_eq!(faces[0], VoxelFace::None);
        assert_eq!(faces[1], VoxelFace::Left);
    }

    #[test]
    fn traversal_early_exit() {
        let mut count = 0;
        voxel_line_traversal(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(10.5, 0.5, 0.5),
            |_pos, _t, _face| {
                count += 1;
                count < 3
            },
        );
        assert_eq!(count, 3);
    }

    #[test]
    fn traversal_diagonal() {
        let mut visited = Vec::new();
        voxel_line_traversal(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(2.5, 2.5, 0.5),
            |pos, _t, _face| {
                visited.push(pos);
                true
            },
        );
        // Should visit at least the start and end voxels
        assert!(visited.contains(&IVec3::new(0, 0, 0)));
        assert!(visited.contains(&IVec3::new(2, 2, 0)));
    }

    #[test]
    fn face_normals() {
        assert_eq!(VoxelFace::Top.normal(), Some(Vec3::Y));
        assert_eq!(VoxelFace::Bottom.normal(), Some(-Vec3::Y));
        assert_eq!(VoxelFace::None.normal(), None);
    }

    #[test]
    fn zero_length_ray_no_panic() {
        let mut visited = Vec::new();
        voxel_line_traversal(Vec3::ZERO, Vec3::ZERO, |pos, _t, _face| {
            visited.push(pos);
            true
        });
        assert!(visited.is_empty());
    }
}
