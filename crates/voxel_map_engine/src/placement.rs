use bevy::prelude::*;

/// Deterministic jittered grid sampling with cross-chunk `min_spacing` enforcement.
///
/// Divides world space into cells of size `min_spacing`. Each cell gets at most
/// one candidate point at a deterministic jittered offset. `density` controls
/// the probability that a cell spawns a point (`spawn_prob = density * min_spacing²`).
///
/// Returns world-space XZ positions that fall within this chunk's footprint.
pub fn jittered_grid_sample(
    seed: u64,
    chunk_pos: IVec3,
    chunk_size: u32,
    min_spacing: f64,
    density: f64,
) -> Vec<Vec2> {
    if min_spacing <= 0.0 || density <= 0.0 {
        return Vec::new();
    }

    let chunk_size_f = chunk_size as f64;
    let chunk_world_x = chunk_pos.x as f64 * chunk_size_f;
    let chunk_world_z = chunk_pos.z as f64 * chunk_size_f;

    // Grid cells overlapping this chunk (no margin needed — we only emit points inside chunk)
    let cell_min_x = (chunk_world_x / min_spacing).floor() as i64;
    let cell_max_x = ((chunk_world_x + chunk_size_f) / min_spacing).ceil() as i64;
    let cell_min_z = (chunk_world_z / min_spacing).floor() as i64;
    let cell_max_z = ((chunk_world_z + chunk_size_f) / min_spacing).ceil() as i64;

    let spawn_prob = (density * min_spacing * min_spacing).min(1.0);
    let mut points = Vec::new();

    for cx in cell_min_x..cell_max_x {
        for cz in cell_min_z..cell_max_z {
            let cell_seed = cell_hash(seed, cx, cz);
            let mut rng = simple_rng(cell_seed);

            // Density gate: probability this cell spawns anything
            if rng_f64(&mut rng) >= spawn_prob {
                continue;
            }

            // Jittered position within the cell
            let world_x = (cx as f64 + rng_f64(&mut rng)) * min_spacing;
            let world_z = (cz as f64 + rng_f64(&mut rng)) * min_spacing;

            // Only keep if inside this chunk
            if world_x >= chunk_world_x
                && world_x < chunk_world_x + chunk_size_f
                && world_z >= chunk_world_z
                && world_z < chunk_world_z + chunk_size_f
            {
                points.push(Vec2::new(world_x as f32, world_z as f32));
            }
        }
    }

    points
}

/// Hash a grid cell coordinate into a deterministic seed.
fn cell_hash(seed: u64, cx: i64, cz: i64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut hasher);
    cx.hash(&mut hasher);
    cz.hash(&mut hasher);
    hasher.finish()
}

/// Simple xorshift64 RNG for deterministic sampling without external deps.
fn simple_rng(seed: u64) -> u64 {
    if seed == 0 { 1 } else { seed }
}

/// Next value from xorshift64, returns value in [0, 1).
fn rng_f64(state: &mut u64) -> f64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state as f64) / (u64::MAX as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jittered_grid_points_within_chunk() {
        let points = jittered_grid_sample(42, IVec3::ZERO, 16, 3.0, 1.0);
        let chunk_min = 0.0;
        let chunk_max = 16.0_f32;
        for p in &points {
            assert!(
                p.x >= chunk_min && p.x < chunk_max,
                "x out of bounds: {}",
                p.x
            );
            assert!(
                p.y >= chunk_min && p.y < chunk_max,
                "z out of bounds: {}",
                p.y
            );
        }
    }

    #[test]
    fn jittered_grid_deterministic() {
        let a = jittered_grid_sample(42, IVec3::new(1, 2, 3), 16, 4.0, 0.5);
        let b = jittered_grid_sample(42, IVec3::new(1, 2, 3), 16, 4.0, 0.5);
        assert_eq!(a, b);
    }

    #[test]
    fn jittered_grid_different_chunks_differ() {
        let a = jittered_grid_sample(42, IVec3::ZERO, 16, 3.0, 1.0);
        let b = jittered_grid_sample(42, IVec3::new(10, 0, 10), 16, 3.0, 1.0);
        assert_ne!(a, b);
    }

    #[test]
    fn jittered_grid_low_density_fewer_points() {
        let high = jittered_grid_sample(42, IVec3::ZERO, 16, 3.0, 1.0);
        let low = jittered_grid_sample(42, IVec3::ZERO, 16, 3.0, 0.1);
        assert!(
            low.len() <= high.len(),
            "low density ({}) should produce <= high density ({})",
            low.len(),
            high.len()
        );
    }

    #[test]
    fn jittered_grid_zero_density_returns_empty() {
        let points = jittered_grid_sample(42, IVec3::ZERO, 16, 3.0, 0.0);
        assert!(points.is_empty());
    }

    #[test]
    fn jittered_grid_zero_spacing_returns_empty() {
        let points = jittered_grid_sample(42, IVec3::ZERO, 16, 0.0, 1.0);
        assert!(points.is_empty());
    }

    #[test]
    fn jittered_grid_large_spacing_few_points() {
        // min_spacing larger than chunk → at most 1 point per chunk
        let points = jittered_grid_sample(42, IVec3::ZERO, 16, 100.0, 1.0);
        assert!(
            points.len() <= 1,
            "expected 0-1 points, got {}",
            points.len()
        );
    }
}
