use std::sync::Arc;

use bevy::prelude::*;
use ndshape::ConstShape;
use noise::{
    Fbm, HybridMulti, MultiFractal, NoiseFn, OpenSimplex, Perlin, RidgedMulti, ScalePoint,
    Seedable, SuperSimplex, Value, Worley,
};
use serde::{Deserialize, Serialize};

use crate::config::VoxelGenerator;
use crate::meshing::flat_terrain_voxels;
use crate::types::{CHUNK_SIZE, PaddedChunkShape, WorldVoxel};

/// Base noise algorithm.
#[derive(Clone, Debug, Serialize, Deserialize, Reflect, Default)]
pub enum NoiseType {
    #[default]
    Perlin,
    OpenSimplex,
    Value,
    Worley,
    SuperSimplex,
}

/// Fractal layering applied on top of a base noise. `Raw` uses the base noise directly.
#[derive(Clone, Debug, Serialize, Deserialize, Reflect, Default)]
pub enum FractalType {
    #[default]
    Fbm,
    RidgedMulti,
    HybridMulti,
    /// No fractal layering; use raw base noise.
    Raw,
}

/// Noise sampling parameters that define how a noise function is constructed.
#[derive(Clone, Debug, Serialize, Deserialize, Reflect)]
pub struct NoiseDef {
    pub noise_type: NoiseType,
    pub fractal: FractalType,
    pub seed_offset: u32,
    pub octaves: u32,
    pub frequency: f64,
    pub lacunarity: f64,
    pub persistence: f64,
}

/// Terrain height noise configuration. Presence enables noise-based terrain;
/// absence produces a flat plane at y=0.
#[derive(Component, Clone, Debug, Serialize, Deserialize, Reflect)]
#[reflect(Component, Serialize, Deserialize)]
pub struct HeightMap {
    pub noise: NoiseDef,
    pub base_height: i32,
    pub amplitude: f64,
}

/// Moisture noise configuration for biome selection.
#[derive(Component, Clone, Debug, Serialize, Deserialize, Reflect)]
#[reflect(Component, Serialize, Deserialize)]
pub struct MoistureMap {
    pub noise: NoiseDef,
}

/// Ordered list of biome selection rules. First matching rule wins.
#[derive(Component, Clone, Debug, Serialize, Deserialize, Reflect)]
#[reflect(Component, Serialize, Deserialize)]
pub struct BiomeRules(pub Vec<BiomeRule>);

/// A single biome's selection criteria and material assignment.
#[derive(Clone, Debug, Serialize, Deserialize, Reflect)]
pub struct BiomeRule {
    pub biome_id: String,
    pub height_range: (f64, f64),
    pub moisture_range: (f64, f64),
    pub surface_material: u8,
    pub subsurface_material: u8,
    pub subsurface_depth: u32,
}

/// Constructs a 2D noise function from a [`NoiseDef`] and world seed.
///
/// Covers all 20 combinations of [`NoiseType`] x [`FractalType`].
/// The combined seed is `(seed as u32).wrapping_add(def.seed_offset)`.
pub fn build_noise_fn(def: &NoiseDef, seed: u64) -> Box<dyn NoiseFn<f64, 2>> {
    let combined_seed = (seed as u32).wrapping_add(def.seed_offset);

    match (&def.noise_type, &def.fractal) {
        (NoiseType::Perlin, FractalType::Raw) => build_raw(Perlin::new(combined_seed), def),
        (NoiseType::OpenSimplex, FractalType::Raw) => {
            build_raw(OpenSimplex::new(combined_seed), def)
        }
        (NoiseType::Value, FractalType::Raw) => build_raw(Value::new(combined_seed), def),
        (NoiseType::Worley, FractalType::Raw) => build_raw(Worley::new(combined_seed), def),
        (NoiseType::SuperSimplex, FractalType::Raw) => {
            build_raw(SuperSimplex::new(combined_seed), def)
        }

        (NoiseType::Perlin, FractalType::Fbm) => build_fbm::<Perlin>(combined_seed, def),
        (NoiseType::OpenSimplex, FractalType::Fbm) => build_fbm::<OpenSimplex>(combined_seed, def),
        (NoiseType::Value, FractalType::Fbm) => build_fbm::<Value>(combined_seed, def),
        (NoiseType::Worley, FractalType::Fbm) => build_fbm::<Worley>(combined_seed, def),
        (NoiseType::SuperSimplex, FractalType::Fbm) => {
            build_fbm::<SuperSimplex>(combined_seed, def)
        }

        (NoiseType::Perlin, FractalType::RidgedMulti) => build_ridged::<Perlin>(combined_seed, def),
        (NoiseType::OpenSimplex, FractalType::RidgedMulti) => {
            build_ridged::<OpenSimplex>(combined_seed, def)
        }
        (NoiseType::Value, FractalType::RidgedMulti) => build_ridged::<Value>(combined_seed, def),
        (NoiseType::Worley, FractalType::RidgedMulti) => build_ridged::<Worley>(combined_seed, def),
        (NoiseType::SuperSimplex, FractalType::RidgedMulti) => {
            build_ridged::<SuperSimplex>(combined_seed, def)
        }

        (NoiseType::Perlin, FractalType::HybridMulti) => build_hybrid::<Perlin>(combined_seed, def),
        (NoiseType::OpenSimplex, FractalType::HybridMulti) => {
            build_hybrid::<OpenSimplex>(combined_seed, def)
        }
        (NoiseType::Value, FractalType::HybridMulti) => build_hybrid::<Value>(combined_seed, def),
        (NoiseType::Worley, FractalType::HybridMulti) => build_hybrid::<Worley>(combined_seed, def),
        (NoiseType::SuperSimplex, FractalType::HybridMulti) => {
            build_hybrid::<SuperSimplex>(combined_seed, def)
        }
    }
}

fn build_raw<T>(base: T, def: &NoiseDef) -> Box<dyn NoiseFn<f64, 2>>
where
    T: NoiseFn<f64, 2> + 'static,
{
    Box::new(
        ScalePoint::new(base)
            .set_x_scale(def.frequency)
            .set_y_scale(def.frequency),
    )
}

fn build_fbm<T>(seed: u32, def: &NoiseDef) -> Box<dyn NoiseFn<f64, 2>>
where
    T: Default + Seedable + NoiseFn<f64, 2> + 'static,
{
    Box::new(
        Fbm::<T>::new(seed)
            .set_octaves(def.octaves as usize)
            .set_frequency(def.frequency)
            .set_lacunarity(def.lacunarity)
            .set_persistence(def.persistence),
    )
}

fn build_ridged<T>(seed: u32, def: &NoiseDef) -> Box<dyn NoiseFn<f64, 2>>
where
    T: Default + Seedable + NoiseFn<f64, 2> + 'static,
{
    Box::new(
        RidgedMulti::<T>::new(seed)
            .set_octaves(def.octaves as usize)
            .set_frequency(def.frequency)
            .set_lacunarity(def.lacunarity)
            .set_persistence(def.persistence),
    )
}

fn build_hybrid<T>(seed: u32, def: &NoiseDef) -> Box<dyn NoiseFn<f64, 2>>
where
    T: Default + Seedable + NoiseFn<f64, 2> + 'static,
{
    Box::new(
        HybridMulti::<T>::new(seed)
            .set_octaves(def.octaves as usize)
            .set_frequency(def.frequency)
            .set_lacunarity(def.lacunarity)
            .set_persistence(def.persistence),
    )
}

const PADDED_XZ: usize = 18;
const CACHE_LEN: usize = PADDED_XZ * PADDED_XZ;

/// Generates voxel data for a single padded chunk (18^3) using heightmap noise.
///
/// When `moisture_map` and `biome_rules` are both provided, biome-aware material
/// selection is used. Otherwise all solid voxels use material index 0.
pub fn generate_heightmap_chunk(
    chunk_pos: IVec3,
    seed: u64,
    height_map: &HeightMap,
    moisture_map: Option<&MoistureMap>,
    biome_rules: Option<&BiomeRules>,
) -> Vec<WorldVoxel> {
    let height_noise = build_noise_fn(&height_map.noise, seed);
    let moisture_noise = moisture_map.map(|m| build_noise_fn(&m.noise, seed));

    let height_cache = build_height_cache(chunk_pos, &*height_noise, height_map);
    let moisture_cache = moisture_noise
        .as_ref()
        .map(|noise| build_2d_cache(chunk_pos, &**noise));

    let total = PaddedChunkShape::SIZE as usize;
    let mut voxels = vec![WorldVoxel::Air; total];

    for i in 0..total {
        let [px, py, pz] = PaddedChunkShape::delinearize(i as u32);
        let world_y = chunk_pos.y * CHUNK_SIZE as i32 + py as i32 - 1;
        let terrain_height = height_cache[xz_index(px, pz)];

        if world_y as f64 <= terrain_height {
            let material = pick_material(
                world_y,
                terrain_height,
                xz_index(px, pz),
                moisture_cache.as_ref(),
                biome_rules,
            );
            voxels[i] = WorldVoxel::Solid(material);
        }
    }

    voxels
}

fn xz_index(px: u32, pz: u32) -> usize {
    px as usize * PADDED_XZ + pz as usize
}

fn build_height_cache(
    chunk_pos: IVec3,
    noise: &dyn NoiseFn<f64, 2>,
    height_map: &HeightMap,
) -> [f64; CACHE_LEN] {
    let mut cache = [0.0; CACHE_LEN];
    for px in 0..PADDED_XZ as u32 {
        for pz in 0..PADDED_XZ as u32 {
            let world_x = chunk_pos.x * CHUNK_SIZE as i32 + px as i32 - 1;
            let world_z = chunk_pos.z * CHUNK_SIZE as i32 + pz as i32 - 1;
            let sample = noise.get([world_x as f64, world_z as f64]);
            cache[xz_index(px, pz)] = height_map.base_height as f64 + sample * height_map.amplitude;
        }
    }
    cache
}

fn build_2d_cache(chunk_pos: IVec3, noise: &dyn NoiseFn<f64, 2>) -> [f64; CACHE_LEN] {
    let mut cache = [0.0; CACHE_LEN];
    for px in 0..PADDED_XZ as u32 {
        for pz in 0..PADDED_XZ as u32 {
            let world_x = chunk_pos.x * CHUNK_SIZE as i32 + px as i32 - 1;
            let world_z = chunk_pos.z * CHUNK_SIZE as i32 + pz as i32 - 1;
            cache[xz_index(px, pz)] = noise.get([world_x as f64, world_z as f64]);
        }
    }
    cache
}

fn pick_material(
    world_y: i32,
    terrain_height: f64,
    cache_idx: usize,
    moisture_cache: Option<&[f64; CACHE_LEN]>,
    biome_rules: Option<&BiomeRules>,
) -> u8 {
    let (Some(moisture), Some(rules)) = (moisture_cache, biome_rules) else {
        return 0;
    };

    let biome = select_biome(&rules.0, terrain_height, moisture[cache_idx]);
    let depth_below_surface = (terrain_height - world_y as f64) as u32;

    if depth_below_surface < biome.subsurface_depth {
        biome.surface_material
    } else {
        biome.subsurface_material
    }
}

/// Returns the first biome whose height and moisture ranges both contain the
/// given values. Falls back to the first rule if none match.
pub fn select_biome<'a>(rules: &'a [BiomeRule], height: f64, moisture: f64) -> &'a BiomeRule {
    debug_assert!(
        !rules.is_empty(),
        "BiomeRules must contain at least one rule"
    );
    rules
        .iter()
        .find(|r| {
            height >= r.height_range.0
                && height <= r.height_range.1
                && moisture >= r.moisture_range.0
                && moisture <= r.moisture_range.1
        })
        .unwrap_or(&rules[0])
}

/// Build a [`VoxelGenerator`] closure from terrain components on a map entity.
///
/// Reads [`HeightMap`], [`MoistureMap`], and [`BiomeRules`] from the entity.
/// If no `HeightMap` is present, falls back to [`flat_terrain_voxels`].
pub fn build_generator(entity: EntityRef, seed: u64) -> VoxelGenerator {
    let height = entity.get::<HeightMap>().cloned();
    let moisture = entity.get::<MoistureMap>().cloned();
    let biomes = entity.get::<BiomeRules>().cloned();

    debug_assert!(
        moisture.is_none() || height.is_some(),
        "MoistureMap without HeightMap is meaningless"
    );
    debug_assert!(
        biomes.is_none() || height.is_some(),
        "BiomeRules without HeightMap is meaningless"
    );
    debug_assert!(
        moisture.is_some() || biomes.is_none(),
        "BiomeRules without MoistureMap: biome selection needs moisture values"
    );

    match height {
        Some(h) => VoxelGenerator(Arc::new(move |chunk_pos| {
            generate_heightmap_chunk(chunk_pos, seed, &h, moisture.as_ref(), biomes.as_ref())
        })),
        None => VoxelGenerator(Arc::new(flat_terrain_voxels)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PaddedChunkShape;
    use ndshape::ConstShape;
    use noise::NoiseFn;

    fn default_noise_def() -> NoiseDef {
        NoiseDef {
            noise_type: NoiseType::Perlin,
            fractal: FractalType::Fbm,
            seed_offset: 0,
            octaves: 4,
            frequency: 0.01,
            lacunarity: 2.0,
            persistence: 0.5,
        }
    }

    fn make_biome_rule(
        id: &str,
        height: (f64, f64),
        moisture: (f64, f64),
        surface: u8,
    ) -> BiomeRule {
        BiomeRule {
            biome_id: id.to_string(),
            height_range: height,
            moisture_range: moisture,
            surface_material: surface,
            subsurface_material: 10,
            subsurface_depth: 3,
        }
    }

    #[test]
    fn build_noise_fn_deterministic() {
        let def = default_noise_def();
        let a = build_noise_fn(&def, 42);
        let b = build_noise_fn(&def, 42);
        assert_eq!(a.get([10.0, 20.0]), b.get([10.0, 20.0]));
    }

    #[test]
    fn build_noise_fn_different_seeds() {
        let def = default_noise_def();
        let a = build_noise_fn(&def, 42);
        let b = build_noise_fn(&def, 9999);
        assert_ne!(a.get([10.0, 20.0]), b.get([10.0, 20.0]));
    }

    #[test]
    fn generate_heightmap_chunk_non_flat() {
        let height_map = HeightMap {
            noise: NoiseDef {
                frequency: 0.005,
                ..default_noise_def()
            },
            base_height: 0,
            amplitude: 20.0,
        };

        let voxels = generate_heightmap_chunk(IVec3::ZERO, 42, &height_map, None, None);
        let air_count = voxels.iter().filter(|v| **v == WorldVoxel::Air).count();
        let solid_count = voxels
            .iter()
            .filter(|v| matches!(v, WorldVoxel::Solid(_)))
            .count();

        assert!(air_count > 0, "expected some air voxels");
        assert!(solid_count > 0, "expected some solid voxels");
    }

    #[test]
    fn generate_heightmap_chunk_all_underground() {
        let height_map = HeightMap {
            noise: NoiseDef {
                frequency: 0.01,
                ..default_noise_def()
            },
            base_height: 0,
            amplitude: 2.0,
        };

        // Chunk at y=-5 means world_y ranges roughly from -80..-64, well below surface.
        let voxels = generate_heightmap_chunk(IVec3::new(0, -5, 0), 42, &height_map, None, None);
        let solid_count = voxels
            .iter()
            .filter(|v| matches!(v, WorldVoxel::Solid(_)))
            .count();
        let total = PaddedChunkShape::SIZE as usize;

        assert_eq!(
            solid_count, total,
            "expected all voxels to be solid underground"
        );
    }

    #[test]
    fn select_biome_exact_match() {
        let rules = vec![
            make_biome_rule("plains", (-100.0, 0.0), (-1.0, 0.0), 1),
            make_biome_rule("desert", (0.0, 100.0), (0.0, 1.0), 2),
        ];

        let result = select_biome(&rules, 50.0, 0.5);
        assert_eq!(result.biome_id, "desert");
    }

    #[test]
    fn select_biome_fallback() {
        let rules = vec![
            make_biome_rule("plains", (0.0, 10.0), (0.0, 0.5), 1),
            make_biome_rule("desert", (20.0, 30.0), (0.5, 1.0), 2),
        ];

        let result = select_biome(&rules, 999.0, 999.0);
        assert_eq!(result.biome_id, "plains");
    }

    #[test]
    fn select_biome_first_wins() {
        let rules = vec![
            make_biome_rule("first", (0.0, 100.0), (0.0, 1.0), 1),
            make_biome_rule("second", (0.0, 100.0), (0.0, 1.0), 2),
        ];

        let result = select_biome(&rules, 50.0, 0.5);
        assert_eq!(result.biome_id, "first");
    }

    #[test]
    fn generate_heightmap_chunk_with_biomes() {
        let height_map = HeightMap {
            noise: NoiseDef {
                frequency: 0.005,
                ..default_noise_def()
            },
            base_height: 0,
            amplitude: 20.0,
        };
        let moisture_map = MoistureMap {
            noise: NoiseDef {
                seed_offset: 1000,
                frequency: 0.01,
                ..default_noise_def()
            },
        };
        let biome_rules = BiomeRules(vec![
            make_biome_rule("grass", (-100.0, 100.0), (-1.0, 0.0), 1),
            make_biome_rule("sand", (-100.0, 100.0), (0.0, 1.0), 2),
        ]);

        let voxels = generate_heightmap_chunk(
            IVec3::ZERO,
            42,
            &height_map,
            Some(&moisture_map),
            Some(&biome_rules),
        );

        let mut materials: std::collections::HashSet<u8> = std::collections::HashSet::new();
        for v in &voxels {
            if let WorldVoxel::Solid(m) = v {
                materials.insert(*m);
            }
        }

        assert!(
            materials.len() >= 2,
            "expected at least 2 different materials, got {materials:?}"
        );
    }
}
