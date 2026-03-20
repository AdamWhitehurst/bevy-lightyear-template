# Procedural Map Generation System — Implementation Plan

## Overview

Replace the hardcoded `flat_terrain_voxels` generator with a data-driven procedural generation system. Terrain configuration is authored in `.terrain.ron` files using the same reflect-based component-map format as `.object.ron`. Noise-based heightmaps, biome rules, and world object placement are all driven by which components are present on the map entity (archetype pattern). Per-chunk entity persistence enables procedural objects to survive unload/reload with mutation tracking.

## Current State Analysis

- **Single generator**: `flat_terrain_voxels` (`meshing.rs:66`) — fills `Solid(0)` below y=0, `Air` above. Used by all map types (overworld, homebase, arena) on both server and client.
- **`VoxelGenerator`**: `Arc<dyn Fn(IVec3) -> Vec<WorldVoxel> + Send + Sync>` (`config.rs:9`). Stored in `VoxelMapConfig.generator`.
- **Seed exists but unused**: `VoxelMapConfig.seed` is set (default 999 for overworld) but never passed to any generator.
- **`WorldObjectDef` pattern**: RON → `reflect_loader::deserialize_component_map` → `Vec<Box<dyn PartialReflect>>` → `apply_object_components`. Proven, extensible.
- **One world object**: `tree_circle.object.ron`, spawned by `spawn_test_tree` with hardcoded position.
- **Entity persistence**: Flat `entities.bin` per map for `RespawnPoint` entities only. No per-chunk entity storage.
- **No noise crate** in any `Cargo.toml`.

### Key Discoveries:
- `VoxelMapInstance` constructors (`overworld`, `homebase`, `arena` in `instance.rs:51-103`) all accept a `VoxelGenerator` parameter — the swap point is clean.
- `ChunkGenResult` (`generation.rs:13-19`) has no world object field yet.
- `remove_out_of_range_chunks` (`lifecycle.rs:139-163`) already saves dirty chunks on eviction — same pattern needed for chunk entities.
- Client `generator_for_map` (`client/src/map.rs:584-591`) matches `MapInstanceId` to generator. Client sets `generates_chunks = false`, so this is only used as a fallback — can remain `flat_terrain_voxels`.
- `reflect_loader::deserialize_component_map` (`reflect_loader.rs:56-64`) is the shared deserialization engine — reusable for `TerrainDef` without modification.

## Desired End State

- `.terrain.ron` files define terrain configuration per map type via reflected components.
- `TerrainDefRegistry` resource maps terrain IDs to loaded `TerrainDef` assets.
- `build_generator(entity, seed)` reads `HeightMap`, `MoistureMap`, `BiomeRules` from the map entity and produces a `VoxelGenerator` closure.
- `PlacementRules` drives Poisson-disk-sampled object spawning per chunk.
- Procedurally spawned objects load/unload with their chunks.
- Per-chunk entity files track mutations (destroyed, moved, component overrides) for procedural objects and store non-procedural (player-placed) objects.
- Existing flat terrain behavior is preserved for maps with empty `.terrain.ron` files.

### Verification:
- `cargo check-all` passes.
- `cargo server` generates noise-based overworld terrain visible at runtime.
- `cargo client -c 1` renders the streamed terrain.
- World objects spawn procedurally in correct biomes.
- Objects despawn when their chunk unloads, respawn when it reloads.
- Destroyed objects stay destroyed across chunk unload/reload cycles.

## What We're NOT Doing

- **3D cave/tunnel carving** — deferred to a future `CaveCarver` component.
- **Biome blending** — hard boundaries only; no noise-based edge smoothing.
- **Client-side terrain config** — chunks are streamed from server; client keeps `flat_terrain_voxels` fallback.
- **Material palette asset** — `u8` material indices exist but visual mapping (color/texture per index) is out of scope.
- **Structure generation** (villages, dungeons) — out of scope; `PlacementRules` handles individual objects only.
- **Mobile entity persistence** (NPCs, wildlife) — only static/scenery objects. Mobile entity chunk persistence is a separate future system.

## Implementation Approach

Reuse the `WorldObjectDef` pattern: RON component maps deserialized via `reflect_loader`, stored as `Vec<Box<dyn PartialReflect>>`, applied to entities via `apply_object_components`. Each terrain aspect is an independent ECS component. Generator behavior is determined by component presence (archetype pattern). World object placement returns spawn descriptors from the async generation task; the main thread spawns entities. Per-chunk entity files use the same versioned-envelope + atomic-write pattern as chunk voxel persistence.

---

## Phase 1: Terrain Types & Noise Generation

### Overview
Define the terrain component types and the core noise-driven chunk generation function. No asset loading yet — types are constructed in code for testing.

### Changes Required:

#### 1. Add `noise` dependency
**File**: `crates/voxel_map_engine/Cargo.toml`
**Changes**: Add `noise = "0.9"` to `[dependencies]`.

```toml
[dependencies]
# ... existing deps ...
noise = "0.9"
```

#### 2. Create terrain types module
**File**: `crates/voxel_map_engine/src/terrain.rs` (new)
**Changes**: Define all terrain component types, enums, and the noise construction function.

```rust
use bevy::prelude::*;
use ndshape::ConstShape;
use noise::{
    Fbm, HybridMulti, MultiFractal, NoiseFn, OpenSimplex, Perlin, RidgedMulti, ScalePoint,
    Seedable, SuperSimplex, Value, Worley,
};
use serde::{Deserialize, Serialize};

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
```

#### 3. Implement noise construction
**File**: `crates/voxel_map_engine/src/terrain.rs` (continued)
**Changes**: `build_noise_fn` returns a `Box<dyn NoiseFn<f64, 2>>` from a `NoiseDef` + world seed.

The `noise` crate's `NoiseFn` trait is generic over dimensionality. All terrain sampling uses 2D `[f64; 2]` inputs (x, z world coordinates). The function dispatches on `(NoiseType, FractalType)` to construct the appropriate noise generator. For fractal types (`Fbm`, `RidgedMulti`, `HybridMulti`), frequency is set via `MultiFractal::set_frequency()`. For `FractalType::Raw`, the base noise is wrapped in `ScalePoint` to scale input coordinates by `frequency`.

Seed combination: `(seed as u32).wrapping_add(def.seed_offset)` — each noise layer gets a distinct seed derived from the world seed plus the configured offset.

The 20 combinations (5 noise types × 4 fractal types) are factored into four generic helper functions (`build_raw`, `build_fbm`, `build_ridged`, `build_hybrid`) bounded by `Default + Seedable + NoiseFn<f64, 2> + 'static`, with the top-level match dispatching to the appropriate helper:

```rust
/// Constructs a 2D noise function from a [`NoiseDef`] and world seed.
pub fn build_noise_fn(def: &NoiseDef, seed: u64) -> Box<dyn NoiseFn<f64, 2>> {
    let combined_seed = (seed as u32).wrapping_add(def.seed_offset);

    match (&def.noise_type, &def.fractal) {
        (NoiseType::Perlin, FractalType::Raw) => build_raw(Perlin::new(combined_seed), def),
        (NoiseType::OpenSimplex, FractalType::Raw) => build_raw(OpenSimplex::new(combined_seed), def),
        // ... all 20 arms enumerated, dispatching to build_raw/build_fbm/build_ridged/build_hybrid
    }
}

fn build_raw<T: NoiseFn<f64, 2> + 'static>(base: T, def: &NoiseDef) -> Box<dyn NoiseFn<f64, 2>> {
    Box::new(ScalePoint::new(base).set_x_scale(def.frequency).set_y_scale(def.frequency))
}

fn build_fbm<T: Default + Seedable + NoiseFn<f64, 2> + 'static>(
    seed: u32, def: &NoiseDef,
) -> Box<dyn NoiseFn<f64, 2>> {
    Box::new(
        Fbm::<T>::new(seed)
            .set_octaves(def.octaves as usize)
            .set_frequency(def.frequency)
            .set_lacunarity(def.lacunarity)
            .set_persistence(def.persistence),
    )
}

// build_ridged and build_hybrid follow the same pattern with RidgedMulti<T> and HybridMulti<T>.
```

The `Box<dyn NoiseFn<f64, 2>>` erasure happens at the outer level — inner noise types are concrete generics that cannot be type-erased.

Note: The return type omits `Send + Sync` bounds because `build_noise_fn` is called inside the generator closure (which itself is `Send + Sync`), and the boxed noise function is consumed within the same closure invocation, never sent across threads independently.

#### 4. Implement heightmap chunk generation
**File**: `crates/voxel_map_engine/src/terrain.rs` (continued)
**Changes**: `generate_heightmap_chunk` — the core generation function. Uses 2D cache arrays to avoid redundant noise evaluation per Y level.

```rust
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
                world_y, terrain_height, xz_index(px, pz),
                moisture_cache.as_ref(), biome_rules,
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
    chunk_pos: IVec3, noise: &dyn NoiseFn<f64, 2>, height_map: &HeightMap,
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
    debug_assert!(!rules.is_empty(), "BiomeRules must contain at least one rule");
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
```

**Note on height/moisture normalization**: The `height_range` and `moisture_range` in `BiomeRule` match against the **raw noise output** (roughly [-1, 1] for Fbm) scaled by amplitude and offset by base_height. For the overworld `.terrain.ron`, these ranges are authored in world-height units (e.g., -20.0 to 20.0 for amplitude 20). For moisture, the raw noise range [-1, 1] is used directly. The initial implementation uses raw values; normalization to [0, 1] can be added later if authoring becomes cumbersome.

#### 5. Register module
**File**: `crates/voxel_map_engine/src/lib.rs`
**Changes**: Add `pub mod terrain;` and register types in `VoxelPlugin::build`.

```rust
pub mod terrain;

// In VoxelPlugin::build():
app.register_type::<terrain::HeightMap>();
app.register_type::<terrain::MoistureMap>();
app.register_type::<terrain::BiomeRules>();
app.register_type::<terrain::NoiseDef>();
app.register_type::<terrain::NoiseType>();
app.register_type::<terrain::FractalType>();
app.register_type::<terrain::BiomeRule>();
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` passes
- [x] Unit tests for `build_noise_fn` (deterministic output for same seed, different output for different seeds)
- [x] Unit tests for `generate_heightmap_chunk` (non-flat terrain at origin, all-solid underground, biome material diversity)
- [x] Unit tests for `select_biome` (exact match, fallback to first rule, first-wins on overlap)

#### Manual Verification:
- [x] None required — no runtime integration yet.

---

## Phase 2: TerrainDef Asset Pipeline

### Overview
Create the `TerrainDef` asset type and loader, following the same pattern as `WorldObjectDef`. Load `.terrain.ron` files during startup, build `TerrainDefRegistry`.

### Changes Required:

#### 1. Create terrain module in protocol
**File**: `crates/protocol/src/terrain/mod.rs` (new)
**Changes**: Module root with re-exports.

```rust
mod loading;
mod plugin;
mod registry;

pub use plugin::TerrainPlugin;
pub use registry::TerrainDefRegistry;
```

#### 2. TerrainDef type
**File**: `crates/protocol/src/terrain/types.rs` (new)
**Changes**: Asset type, structurally identical to `WorldObjectDef`.

```rust
use bevy::prelude::*;

/// A loaded terrain definition. Component map loaded from `.terrain.ron`.
#[derive(Asset, TypePath)]
pub struct TerrainDef {
    pub components: Vec<Box<dyn PartialReflect>>,
}
```

Implement `Clone` and `Debug` the same way `WorldObjectDef` does — via `reflect_clone()` for Clone, type-path-only for Debug.

#### 3. TerrainDefLoader
**File**: `crates/protocol/src/terrain/loader.rs` (new)
**Changes**: `AssetLoader` implementation, mirrors `WorldObjectLoader`.

```rust
use bevy::asset::{AssetLoader, LoadContext, Reader};
use bevy::reflect::TypeRegistryArc;
use bevy::prelude::*;
use crate::reflect_loader;

pub(super) struct TerrainDefLoader {
    type_registry: TypeRegistryArc,
}

impl FromWorld for TerrainDefLoader {
    fn from_world(world: &mut World) -> Self {
        Self {
            type_registry: world.resource::<AppTypeRegistry>().0.clone(),
        }
    }
}

impl AssetLoader for TerrainDefLoader {
    type Asset = super::types::TerrainDef;
    type Settings = ();
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn extensions(&self) -> &[&str] {
        &["terrain.ron"]
    }

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let registry = self.type_registry.read();
        let components = reflect_loader::deserialize_component_map(&bytes, &registry)?;
        Ok(super::types::TerrainDef { components })
    }
}
```

**Empty RON files**: An empty `{ }` RON map deserializes to an empty `Vec<Box<dyn PartialReflect>>`. This is the "homebase" case — no terrain components means flat terrain.

#### 4. TerrainDefRegistry
**File**: `crates/protocol/src/terrain/registry.rs` (new)
**Changes**: Resource holding the loaded terrain definitions.

```rust
use bevy::prelude::*;
use std::collections::HashMap;
use super::types::TerrainDef;

/// Maps terrain definition IDs (e.g., "overworld") to loaded `TerrainDef` assets.
#[derive(Resource)]
pub struct TerrainDefRegistry {
    pub terrains: HashMap<String, TerrainDef>,
}

impl TerrainDefRegistry {
    pub fn get(&self, id: &str) -> Option<&TerrainDef> {
        self.terrains.get(id)
    }
}
```

#### 5. Loading pipeline
**File**: `crates/protocol/src/terrain/loading.rs` (new)
**Changes**: Same dual-path (native/WASM) pattern as `world_object/loading.rs`.

**Native**: `load_folder("terrain")` → `TrackedAssets` → `insert_terrain_defs` once all loaded.
**WASM**: Load `terrain.manifest.ron` → individual file loads → `insert_terrain_defs`.

ID derivation: strip `.terrain.ron` suffix from filename, same as `object_id_from_path`.

#### 6. TerrainPlugin
**File**: `crates/protocol/src/terrain/plugin.rs` (new)
**Changes**: Registers the asset type, loader, type reflections, and loading systems.

```rust
impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<TerrainDef>();
        app.init_asset_loader::<TerrainDefLoader>();
        // Type registrations for terrain components are already done in VoxelPlugin.
        // TerrainPlugin only handles the asset pipeline.
        app.add_systems(Startup, loading::load_terrain_defs);
        app.add_systems(Update,
            loading::insert_terrain_defs.run_if(not(resource_exists::<TerrainDefRegistry>)),
        );
    }
}
```

Both functions are defined in `crates/protocol/src/terrain/loading.rs` (section 5 above). They follow the same pattern as `world_object/loading.rs`:

- `load_terrain_defs`: Startup system. Native: `asset_server.load_folder("terrain")` + add to `TrackedAssets`. WASM: load `terrain.manifest.ron`, then individual files.
- `insert_terrain_defs`: Update system gated by `not(resource_exists::<TerrainDefRegistry>)`. Waits for all terrain assets to load, then collects them into `TerrainDefRegistry` using `terrain_id_from_path` (strips `.terrain.ron` suffix from filename).
```

#### 7. Register module in protocol
**File**: `crates/protocol/src/lib.rs`
**Changes**: Add `pub mod terrain;` and add `TerrainPlugin` to `ProtocolPlugin` (or whichever plugin aggregates protocol plugins).

#### 8. Create `.terrain.ron` asset files

**File**: `assets/terrain/overworld.terrain.ron` (new)
```ron
{
    "voxel_map_engine::terrain::HeightMap": (
        noise: (noise_type: Perlin, fractal: Fbm, seed_offset: 0,
                octaves: 6, frequency: 0.005, lacunarity: 2.0, persistence: 0.5),
        base_height: 0,
        amplitude: 20.0,
    ),
    "voxel_map_engine::terrain::MoistureMap": (
        noise: (noise_type: Perlin, fractal: Fbm, seed_offset: 100,
                octaves: 4, frequency: 0.008, lacunarity: 2.0, persistence: 0.6),
    ),
    "voxel_map_engine::terrain::BiomeRules": ([
        (biome_id: "grassland", height_range: (-5.0, 5.0), moisture_range: (-0.3, 0.3),
         surface_material: 1, subsurface_material: 2, subsurface_depth: 3),
        (biome_id: "desert", height_range: (-5.0, 5.0), moisture_range: (-1.0, -0.3),
         surface_material: 3, subsurface_material: 3, subsurface_depth: 5),
        (biome_id: "forest", height_range: (-5.0, 10.0), moisture_range: (0.3, 1.0),
         surface_material: 1, subsurface_material: 2, subsurface_depth: 4),
    ]),
}
```

**File**: `assets/terrain/homebase.terrain.ron` (new)
```ron
{
}
```

**File**: `assets/terrain/arena_hills.terrain.ron` (new)
```ron
{
    "voxel_map_engine::terrain::HeightMap": (
        noise: (noise_type: Perlin, fractal: Fbm, seed_offset: 0,
                octaves: 3, frequency: 0.02, lacunarity: 2.0, persistence: 0.4),
        base_height: 0,
        amplitude: 5.0,
    ),
    "voxel_map_engine::terrain::BiomeRules": ([
        (biome_id: "sand", height_range: (-100.0, 100.0), moisture_range: (-1.0, 1.0),
         surface_material: 2, subsurface_material: 2, subsurface_depth: 99),
    ]),
}
```

**File**: `assets/terrain.manifest.ron` (new, for WASM)
```ron
["overworld", "homebase", "arena_hills"]
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` passes
- [x] `.terrain.ron` files parse without errors at startup (verified via `cargo server` log output)

#### Manual Verification:
- [x] `cargo server` starts and logs "Loaded N terrain definitions"

---

## Phase 3: Wire Terrain Into Map Spawning

### Overview
Connect the `TerrainDefRegistry` to map spawning. Apply terrain components onto map entities, then `build_generator` reads them to produce a `VoxelGenerator`.

### Changes Required:

#### 1. Implement `build_generator`
**File**: `crates/voxel_map_engine/src/terrain.rs`
**Changes**: Add `build_generator` function.

```rust
use crate::config::VoxelGenerator;
use crate::meshing::flat_terrain_voxels;
use std::sync::Arc;

/// Build a `VoxelGenerator` closure from terrain components on a map entity.
///
/// Reads `HeightMap`, `MoistureMap`, and `BiomeRules` from the entity.
/// If no `HeightMap` is present, falls back to `flat_terrain_voxels`.
pub fn build_generator(entity: EntityRef, seed: u64) -> VoxelGenerator {
    let height = entity.get::<HeightMap>().cloned();
    let moisture = entity.get::<MoistureMap>().cloned();
    let biomes = entity.get::<BiomeRules>().cloned();

    // Validate component combinations.
    debug_assert!(
        moisture.is_none() || height.is_some(),
        "MoistureMap without HeightMap is meaningless — moisture is only sampled during heightmap generation"
    );
    debug_assert!(
        biomes.is_none() || height.is_some(),
        "BiomeRules without HeightMap is meaningless — biome selection requires terrain height"
    );
    debug_assert!(
        moisture.is_some() || biomes.is_none(),
        "BiomeRules without MoistureMap: biome selection needs moisture values. \
         Use a MoistureMap or remove BiomeRules (defaults to Solid(0))"
    );

    match height {
        Some(h) => VoxelGenerator(Arc::new(move |chunk_pos| {
            generate_heightmap_chunk(
                chunk_pos,
                seed,
                &h,
                moisture.as_ref(),
                biomes.as_ref(),
            )
        })),
        None => VoxelGenerator(Arc::new(flat_terrain_voxels)),
    }
}
```

#### 2. Apply terrain def to map entity
**File**: `crates/server/src/map.rs`
**Changes**: Modify `spawn_overworld` and `spawn_homebase` to:
1. Look up `TerrainDef` from `TerrainDefRegistry`
2. Apply terrain components onto the map entity via `apply_object_components`
3. Build generator from the entity's terrain components

**Architectural change**: Move `generator` out of `VoxelMapConfig` and make `VoxelGenerator` itself a component. This eliminates the need for placeholder generators and `NeedsGeneratorBuild` entirely — systems simply check `Without<VoxelGenerator>` to skip maps that don't have a generator yet.

**File**: `crates/voxel_map_engine/src/config.rs`
**Changes**: Remove `generator` field from `VoxelMapConfig`. Change `VoxelGenerator` from a type alias to a component:

```rust
/// The chunk generation function for a map instance.
///
/// Separate component from `VoxelMapConfig` so maps can exist without a
/// generator while terrain components are being applied (deferred commands).
#[derive(Component, Clone)]
pub struct VoxelGenerator(pub Arc<dyn Fn(IVec3) -> Vec<WorldVoxel> + Send + Sync>);
```

Remove the `generator` parameter from `VoxelMapConfig::new`. Update all code that reads `config.generator` to query `VoxelGenerator` instead. Call sites change from `(generator)(pos)` to `(generator.0)(pos)`.

**Why**: `apply_object_components` defers insertion via `commands.queue()`, so terrain components aren't available until the next command flush. By making the generator a separate component, we split map spawning into two natural steps:
1. `spawn_overworld`: Spawns entity with `VoxelMapConfig` + terrain components (deferred). No generator yet.
2. `build_terrain_generators` system: Queries entities with terrain components but `Without<VoxelGenerator>`. Calls `build_generator`, inserts `VoxelGenerator`.

Lifecycle systems (`ensure_pending_chunks`, `update_chunks`) already need to query the generator — they simply add `With<VoxelGenerator>` to their query filter. No placeholder, no marker component, no wasted frame.

**Command flush timing**: Insert `apply_deferred` between `build_terrain_generators` and the lifecycle chain to ensure terrain components are flushed before the generator build runs in the same frame:

```rust
// In ServerMapPlugin system registration:
app.add_systems(Update, (
    build_terrain_generators,
    apply_deferred,
    // ... existing VoxelPlugin lifecycle chain ...
).chain());
```

Or, since `VoxelPlugin` registers its own lifecycle chain, the server plugin adds `build_terrain_generators` to run before `VoxelPlugin`'s systems with an `apply_deferred` barrier between them. The exact ordering depends on how system sets are structured — the key invariant is: `apply_object_components` flush → `build_terrain_generators` → chunk lifecycle.

**Flow**:

```rust
// spawn_overworld (Startup):
let config = VoxelMapConfig::new(seed, generation_version, 2, None, 5);
// No generator field — VoxelMapConfig is just config data now.

let map = commands.spawn((
    VoxelMapInstance::new(5), config, Transform::default(), MapInstanceId::Overworld,
)).id();

if let Some(terrain_def) = terrain_registry.get("overworld") {
    apply_object_components(&mut commands, map, terrain_def.components.clone(), type_registry.0.clone());
}
// No VoxelGenerator yet. Lifecycle systems skip this entity.

// build_terrain_generators (Update, runs after apply_deferred):
fn build_terrain_generators(
    mut commands: Commands,
    query: Query<(Entity, &VoxelMapConfig), (
        With<VoxelMapInstance>,
        Without<VoxelGenerator>,
    )>,
    world: &World,
) {
    for (entity, config) in &query {
        let entity_ref = world.entity(entity);
        let generator = build_generator(entity_ref, config.seed);
        commands.entity(entity).insert(VoxelGenerator(generator));
        info!("Built terrain generator for map entity {entity:?}");
    }
}
```

**Lifecycle gating**: In `ensure_pending_chunks`, add `With<VoxelGenerator>` filter:

```rust
fn ensure_pending_chunks(
    mut commands: Commands,
    query: Query<Entity, (With<VoxelMapInstance>, With<VoxelGenerator>, Without<PendingChunks>)>,
) { ... }
```

Maps without a generator simply don't start loading chunks — clean and explicit.

**Files that need updating for `generator` field removal**:
- `config.rs`: Remove field, remove from `new()` params
- `generation.rs`: `spawn_chunk_gen_task` takes `&VoxelGenerator` — call sites pass `&generator.0` or destructure
- `lifecycle.rs`: `spawn_missing_chunks` queries `&VoxelGenerator` as a component instead of reading from config
- `instance.rs`: `overworld()`, `homebase()`, `arena()` constructors no longer take `generator` param
- `server/src/map.rs`: All map spawn sites drop the generator arg from `VoxelMapConfig::new`
- `client/src/map.rs`: `generator_for_map` returns `VoxelGenerator(Arc::new(flat_terrain_voxels))` to insert as a component
- Existing tests that construct `VoxelMapConfig` with a generator param

#### 3. Modify `spawn_overworld`
**File**: `crates/server/src/map.rs`
**Changes**: Add `TerrainDefRegistry` and `AppTypeRegistry` parameters. Look up "overworld" terrain def. Apply components. Add `NeedsGeneratorBuild`.

```rust
pub fn spawn_overworld(
    mut commands: Commands,
    mut registry: ResMut<MapRegistry>,
    save_path: Res<WorldSavePath>,
    terrain_registry: Res<TerrainDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
) {
    // ... existing seed/version loading ...

    let mut config = VoxelMapConfig::new(
        seed, generation_version, 2, None, 5,
        Arc::new(flat_terrain_voxels), // placeholder
    );
    config.save_dir = Some(map_dir);

    let map = commands
        .spawn((
            VoxelMapInstance::new(5),
            config,
            Transform::default(),
            MapInstanceId::Overworld,
            NeedsGeneratorBuild,
        ))
        .id();

    // Apply terrain components from def
    if let Some(terrain_def) = terrain_registry.get("overworld") {
        apply_object_components(
            &mut commands, map,
            terrain_def.components.clone(),
            type_registry.0.clone(),
        );
    }

    commands.insert_resource(OverworldMap(map));
    registry.insert(MapInstanceId::Overworld, map);
}
```

#### 4. Modify `spawn_homebase`
**File**: `crates/server/src/map.rs`
**Changes**: Same pattern — look up "homebase" terrain def, apply, add `NeedsGeneratorBuild`.

#### 5. Add `build_terrain_generators` system
**File**: `crates/server/src/map.rs` (or a new `crates/server/src/terrain.rs`)
**Changes**: System that builds generators from terrain components, as described above. Register in `ServerMapPlugin`.

#### 6. Gate `ensure_pending_chunks`
**File**: `crates/voxel_map_engine/src/lifecycle.rs`
**Changes**: Add `Without<NeedsGeneratorBuild>` filter to the query in `ensure_pending_chunks`, so maps don't start generating chunks until their generator is built.

The `NeedsGeneratorBuild` component is defined in `voxel_map_engine` (not server) so the lifecycle system can reference it without a server dependency.

#### 7. Delete existing save data
Existing saved chunks were generated with `flat_terrain_voxels`. After changing the generator, old chunks would mismatch new procedural terrain.

**Approach**: Bump `GENERATION_VERSION` from 0 to 1. In `spawn_chunk_gen_task`, when loading from disk, compare the saved chunk's generation version against the config's. If mismatched, regenerate instead of using the saved data.

This requires adding `generation_version` to `ChunkData` (it's already in `VoxelMapConfig` but not persisted per-chunk). Alternatively, simpler: bump `CHUNK_SAVE_VERSION` in `persistence.rs`, which causes all old chunks to fail version validation and be regenerated. This is the simplest approach and appropriate since the generation algorithm fundamentally changed.

**File**: `crates/voxel_map_engine/src/persistence.rs`
**Changes**: Bump `CHUNK_SAVE_VERSION` from 1 to 2.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` passes
- [ ] Existing tests pass (may need updates for new system parameters)

#### Manual Verification:
- [ ] `cargo server` generates non-flat overworld terrain
- [ ] `cargo client -c 1` renders the noise-based terrain
- [ ] Homebase remains flat (empty terrain def)
- [ ] Server log shows "Built terrain generator for map entity"
- [ ] Multiple materials visible on terrain surface (different biomes)

---

## Phase 4: World Object Placement

### Overview
Add `PlacementRules` component, implement Poisson disk sampling for object positions, extend `ChunkGenResult` to carry spawn descriptors, and spawn/despawn objects with their chunks.

### Changes Required:

#### 1. Add `fast_poisson` dependency
**File**: `crates/voxel_map_engine/Cargo.toml`
**Changes**: Add `fast_poisson = "0.5"`.

Note: `fast_poisson` does not compile to WASM, but object placement only runs server-side. The voxel_map_engine crate is used by the client too, so the placement code must be behind a `#[cfg(not(target_arch = "wasm32"))]` gate or in a separate module that the client doesn't compile. Since `generates_chunks = false` on the client, the placement code is never called there — but the crate still must compile for WASM.

**Alternative**: Use `poisson_diskus` which is pure Rust and WASM-compatible. Or implement Bridson's algorithm directly (it's ~50 lines).

**Decision**: Implement Bridson's algorithm directly in `terrain.rs`. It's simple, has no external dependency, and avoids WASM compilation issues entirely.

#### 2. PlacementRules component
**File**: `crates/voxel_map_engine/src/terrain.rs`
**Changes**: Add types.

```rust
/// Rules for procedural object placement. Multiple object types per map.
#[derive(Component, Clone, Debug, Serialize, Deserialize, Reflect)]
#[reflect(Component, Serialize, Deserialize)]
pub struct PlacementRules(pub Vec<PlacementRule>);

/// A single object placement rule.
#[derive(Clone, Debug, Serialize, Deserialize, Reflect)]
pub struct PlacementRule {
    /// World object definition ID (matches WorldObjectId).
    pub object_id: String,
    /// Biomes where this object may spawn.
    pub allowed_biomes: Vec<String>,
    /// Probability of accepting a candidate position (0.0–1.0).
    pub density: f64,
    /// Minimum distance between instances of this object type.
    pub min_spacing: f64,
}
```

Register `PlacementRules` and `PlacementRule` in `VoxelPlugin::build`.

#### 3. WorldObjectSpawn descriptor
**File**: `crates/voxel_map_engine/src/terrain.rs`
**Changes**: Add spawn descriptor returned from generation.

```rust
/// Descriptor for a world object to spawn at a specific position.
/// Produced by the generator, consumed by the main thread to spawn entities.
#[derive(Clone, Debug)]
pub struct WorldObjectSpawn {
    /// World object definition ID.
    pub object_id: String,
    /// World-space position.
    pub position: Vec3,
    /// Deterministic ID for this procedural spawn. Derived from
    /// `hash(seed, chunk_pos, spawn_index)`.
    pub procedural_id: u64,
}
```

#### 4. Extend ChunkGenResult
**File**: `crates/voxel_map_engine/src/generation.rs`
**Changes**: Add `world_objects` field.

```rust
pub struct ChunkGenResult {
    pub position: IVec3,
    pub mesh: Option<Mesh>,
    pub voxels: Vec<WorldVoxel>,
    pub from_disk: bool,
    pub world_objects: Vec<WorldObjectSpawn>,
}
```

Update all construction sites to include `world_objects: Vec::new()` (disk-loaded chunks and fresh generation without placement rules both produce empty vectors).

#### 5. Implement Poisson disk sampling
**File**: `crates/voxel_map_engine/src/terrain.rs`
**Changes**: Bridson's algorithm for 2D point generation within a chunk footprint.

```rust
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Deterministic per-chunk RNG seed.
fn chunk_rng_seed(world_seed: u64, chunk_pos: IVec3) -> u64 {
    let mut hasher = DefaultHasher::new();
    world_seed.hash(&mut hasher);
    chunk_pos.hash(&mut hasher);
    hasher.finish()
}

/// Generate world object spawns for a chunk using Poisson disk sampling.
pub fn generate_chunk_objects(
    chunk_pos: IVec3,
    seed: u64,
    placement_rules: &PlacementRules,
    height_cache: &[f64; 18 * 18],
    biome_ids: &[&str; 18 * 18],  // pre-computed biome ID per (x,z)
) -> Vec<WorldObjectSpawn> {
    let mut spawns = Vec::new();
    let chunk_seed = chunk_rng_seed(seed, chunk_pos);

    for (rule_idx, rule) in placement_rules.0.iter().enumerate() {
        let rule_seed = chunk_seed.wrapping_add(rule_idx as u64);
        let candidates = poisson_disk_2d(
            rule.min_spacing,
            CHUNK_SIZE as f64,
            CHUNK_SIZE as f64,
            rule_seed,
        );

        for (i, (lx, lz)) in candidates.into_iter().enumerate() {
            // Compute world position
            let world_x = chunk_pos.x as f64 * CHUNK_SIZE as f64 + lx;
            let world_z = chunk_pos.z as f64 * CHUNK_SIZE as f64 + lz;

            // Check biome at this position
            let px = (lx + 1.0) as usize; // offset for padding
            let pz = (lz + 1.0) as usize;
            let idx_2d = pz * 18 + px;
            if idx_2d >= 18 * 18 { continue; }
            let biome = biome_ids[idx_2d];
            if !rule.allowed_biomes.iter().any(|b| b == biome) { continue; }

            // Density check
            let accept_hash = chunk_rng_seed(rule_seed, IVec3::new(i as i32, 0, 0));
            let accept_prob = (accept_hash % 10000) as f64 / 10000.0;
            if accept_prob > rule.density { continue; }

            // Height at this position
            let height = height_cache[idx_2d];

            let proc_id = chunk_rng_seed(
                seed,
                IVec3::new(chunk_pos.x, rule_idx as i32, i as i32),
            );

            spawns.push(WorldObjectSpawn {
                object_id: rule.object_id.clone(),
                position: Vec3::new(world_x as f32, height as f32, world_z as f32),
                procedural_id: proc_id,
            });
        }
    }
    spawns
}
```

The `poisson_disk_2d` function implements Bridson's algorithm:

```rust
/// Bridson's Poisson disk sampling in 2D.
/// Returns a list of (x, z) points within [0, width) × [0, height) with
/// minimum spacing `r` between any two points.
fn poisson_disk_2d(r: f64, width: f64, height: f64, seed: u64) -> Vec<(f64, f64)> {
    const K: usize = 30; // candidates per active point

    let cell_size = r / std::f64::consts::SQRT_2;
    let grid_w = (width / cell_size).ceil() as usize;
    let grid_h = (height / cell_size).ceil() as usize;
    let mut grid: Vec<Option<usize>> = vec![None; grid_w * grid_h];
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    let mut rng = LcgRng::new(seed);

    // Seed point
    let first = (rng.next_f64() * width, rng.next_f64() * height);
    let gi = (first.0 / cell_size) as usize;
    let gj = (first.1 / cell_size) as usize;
    grid[gj * grid_w + gi] = Some(0);
    points.push(first);
    active.push(0);

    while let Some(&active_idx) = active.last() {
        let (px, py) = points[active_idx];
        let mut found = false;

        for _ in 0..K {
            let angle = rng.next_f64() * std::f64::consts::TAU;
            let dist = r + rng.next_f64() * r; // uniform in [r, 2r]
            let nx = px + angle.cos() * dist;
            let ny = py + angle.sin() * dist;

            if nx < 0.0 || nx >= width || ny < 0.0 || ny >= height {
                continue;
            }

            let gi = (nx / cell_size) as usize;
            let gj = (ny / cell_size) as usize;

            // Check 5×5 neighborhood in grid
            let mut too_close = false;
            for di in 0..5usize {
                for dj in 0..5usize {
                    let ni = gi.wrapping_add(di).wrapping_sub(2);
                    let nj = gj.wrapping_add(dj).wrapping_sub(2);
                    if ni >= grid_w || nj >= grid_h { continue; }
                    if let Some(idx) = grid[nj * grid_w + ni] {
                        let (qx, qy) = points[idx];
                        let dx = nx - qx;
                        let dy = ny - qy;
                        if dx * dx + dy * dy < r * r {
                            too_close = true;
                            break;
                        }
                    }
                }
                if too_close { break; }
            }

            if !too_close {
                let new_idx = points.len();
                grid[gj * grid_w + gi] = Some(new_idx);
                points.push((nx, ny));
                active.push(new_idx);
                found = true;
                break;
            }
        }

        if !found {
            active.pop();
        }
    }
    points
}

/// Minimal deterministic LCG (linear congruential generator).
struct LcgRng(u64);

impl LcgRng {
    fn new(seed: u64) -> Self { Self(seed) }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}
```

#### 6. Integrate placement into generator
**File**: `crates/voxel_map_engine/src/terrain.rs`
**Changes**: Modify `build_generator` to capture `PlacementRules` and return both voxels and object spawns. This requires changing the `VoxelGenerator` type signature.

**Problem**: `VoxelGenerator` currently returns `Vec<WorldVoxel>`. To include world object spawns, it needs to return `(Vec<WorldVoxel>, Vec<WorldObjectSpawn>)`.

**File**: `crates/voxel_map_engine/src/config.rs`
**Changes**: Update type alias.

```rust
pub type VoxelGenerator = Arc<
    dyn Fn(IVec3) -> (Vec<WorldVoxel>, Vec<WorldObjectSpawn>) + Send + Sync
>;
```

Update `flat_terrain_voxels` to return `(voxels, Vec::new())`.

Update all call sites: `generate_chunk` in `generation.rs`, any tests.

#### 7. Spawn objects when chunk loads
**File**: `crates/voxel_map_engine/src/lifecycle.rs`
**Changes**: In `handle_completed_chunk` (called from `poll_chunk_tasks`), after inserting chunk data and spawning the mesh entity, emit the `WorldObjectSpawn` list so the server can spawn the entities.

**Approach**: Store `world_objects` on the `VoxelChunk` component or emit them as events. Events are cleaner:

```rust
/// Emitted when a chunk completes generation with world object spawn requests.
#[derive(Event)]
pub struct ChunkObjectsReady {
    pub map_entity: Entity,
    pub chunk_pos: IVec3,
    pub objects: Vec<WorldObjectSpawn>,
}
```

The server listens for `ChunkObjectsReady` events and calls `spawn_world_object` for each.

#### 8. Despawn objects when chunk unloads
**File**: `crates/voxel_map_engine/src/lifecycle.rs`
**Changes**: In `despawn_out_of_range_chunks`, before despawning the `VoxelChunk` entity, also despawn all entities tagged with `ChunkEntityRef { chunk_pos, map_entity }`.

```rust
/// Tags an entity as belonging to a specific chunk. Despawned when the chunk unloads.
#[derive(Component)]
pub struct ChunkEntityRef {
    pub chunk_pos: IVec3,
    pub map_entity: Entity,
}
```

#### 9. Server-side spawn handler
**File**: `crates/server/src/world_object.rs` (or new file)
**Changes**: System that reads `ChunkObjectsReady` events and spawns world objects.

```rust
fn spawn_chunk_world_objects(
    mut commands: Commands,
    mut events: MessageReader<ChunkObjectsReady>,
    defs: Res<WorldObjectDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
    // ... other params for spawn_world_object ...
) {
    for event in events.read() {
        for obj in &event.objects {
            let id = WorldObjectId(obj.object_id.clone());
            let Some(def) = defs.get(&id) else {
                warn!("Unknown world object for placement: {}", obj.object_id);
                continue;
            };
            let entity = spawn_world_object(&mut commands, id, def, ...);
            // Override position from placement
            commands.entity(entity).insert(Position(obj.position));
            // Tag with chunk ref for lifecycle management
            commands.entity(entity).insert(ChunkEntityRef {
                chunk_pos: event.chunk_pos,
                map_entity: event.map_entity,
            });
        }
    }
}
```

#### 10. Update overworld `.terrain.ron` with placement rules
**File**: `assets/terrain/overworld.terrain.ron`
**Changes**: Add `PlacementRules`.

```ron
    "voxel_map_engine::terrain::PlacementRules": ([
        (object_id: "tree_circle", allowed_biomes: ["grassland", "forest"],
         density: 0.3, min_spacing: 4.0),
    ]),
```

#### 11. Remove `spawn_test_tree`
**File**: `crates/server/src/gameplay.rs`
**Changes**: Delete `spawn_test_tree` and its registration. Trees are now spawned procedurally.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` passes
- [ ] Unit tests for Poisson disk sampling (minimum spacing guarantee, determinism)
- [ ] Unit tests for `generate_chunk_objects` (correct biome filtering, density modulation)

#### Manual Verification:
- [ ] `cargo server` + `cargo client -c 1`: trees spawn across the overworld in grassland/forest biomes
- [ ] Trees do not spawn in desert biome areas
- [ ] Trees maintain minimum spacing
- [ ] Trees despawn when walking away (chunk unloads)
- [ ] Trees reappear when returning (chunk reloads, same positions)

---

## Phase 5: Per-Chunk Entity Persistence

### Overview
Persist chunk-associated entities (procedural objects with mutations, player-placed objects) to per-chunk entity files. On chunk load, merge procedural regeneration with persisted state.

### Changes Required:

#### 1. Per-chunk entity file format
**File**: `crates/voxel_map_engine/src/entity_persistence.rs` (new)
**Changes**: Types and save/load functions.

```rust
use bevy::math::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CHUNK_ENTITY_VERSION: u32 = 1;

/// A single persisted entity within a chunk.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChunkEntity {
    /// Deterministic ID for procedurally spawned objects. `None` for player-placed.
    pub procedural_id: Option<u64>,
    /// The world object definition ID.
    pub object_id: String,
    /// Current world-space position.
    pub position: Vec3,
    /// Component overrides from baseline `WorldObjectDef`.
    /// Key: fully-qualified type path, Value: RON-serialized component data.
    /// On load: spawn from def baseline, then deserialize+apply each override.
    /// Dead objects have `Health { current: 0.0, ... }` in overrides — no separate
    /// `destroyed` flag needed.
    pub component_overrides: HashMap<String, String>,
}

#[derive(Serialize, Deserialize)]
struct ChunkEntityEnvelope {
    version: u32,
    entities: Vec<ChunkEntity>,
}

pub fn chunk_entity_file_path(map_dir: &Path, chunk_pos: IVec3) -> PathBuf {
    map_dir
        .join("entities")
        .join(format!("chunk_{}_{}_{}.entities.bin", chunk_pos.x, chunk_pos.y, chunk_pos.z))
}

pub fn save_chunk_entities(
    map_dir: &Path,
    chunk_pos: IVec3,
    entities: &[ChunkEntity],
) -> Result<(), String> {
    // Same atomic write pattern as chunk voxel persistence.
    // Only write if entities is non-empty; delete file if empty.
}

pub fn load_chunk_entities(
    map_dir: &Path,
    chunk_pos: IVec3,
) -> Result<Vec<ChunkEntity>, String> {
    // Returns empty Vec if file doesn't exist.
}
```

#### 2. Extend chunk load sequence
**File**: `crates/voxel_map_engine/src/lifecycle.rs` (or `generation.rs`)
**Changes**: After chunk generation completes and `ChunkObjectsReady` is emitted, the server-side handler merges procedural spawns with persisted entity data.

The merge logic runs in the server's `spawn_chunk_world_objects` system:

```rust
fn spawn_chunk_world_objects(
    mut commands: Commands,
    mut events: MessageReader<ChunkObjectsReady>,
    defs: Res<WorldObjectDefRegistry>,
    map_query: Query<&VoxelMapConfig>,
    // ...
) {
    for event in events.read() {
        let save_dir = map_query
            .get(event.map_entity)
            .ok()
            .and_then(|c| c.save_dir.as_ref());

        // Load persisted entities for this chunk
        let persisted = save_dir
            .map(|dir| load_chunk_entities(dir, event.chunk_pos).unwrap_or_default())
            .unwrap_or_default();

        for obj in &event.objects {
            // Check if persisted data overrides this procedural spawn
            let persisted_entry = persisted.iter().find(|e| {
                e.procedural_id == Some(obj.procedural_id)
            });

            match persisted_entry {
                Some(entry) => {
                    // Spawn from def baseline, then apply persisted overrides.
                    // If Health.current == 0, the entity spawns "dead" and the
                    // respawn timer system handles it normally.
                    spawn_with_overrides(&mut commands, entry, &defs, ...);
                }
                None => {
                    // No persisted state — spawn fresh from definition
                    spawn_from_def(&mut commands, obj, &defs, ...);
                }
            }
        }

        // Spawn non-procedural entities (player-placed)
        for entry in &persisted {
            if entry.procedural_id.is_none() {
                spawn_with_overrides(&mut commands, entry, &defs, ...);
            }
        }
    }
}
```

#### 3. Save entities on chunk unload
**File**: `crates/server/src/map.rs` (or dedicated system)
**Changes**: Before a chunk is evicted, collect all entities with matching `ChunkEntityRef`, serialize them, and write to the chunk entity file.

```rust
fn save_chunk_entities_on_unload(
    mut commands: Commands,
    removed_chunks: RemovedComponents<VoxelChunk>,
    chunk_entity_query: Query<(Entity, &ChunkEntityRef, &WorldObjectId, &Position, ...)>,
    map_query: Query<&VoxelMapConfig>,
) {
    // Listen for VoxelChunk entity despawns (via RemovedComponents)
    // OR hook into the existing remove_out_of_range_chunks flow.
}
```

**Better approach**: Emit a `ChunkUnloading` event from `despawn_out_of_range_chunks` before despawning. The server listens and saves entities.

```rust
/// Emitted before a chunk entity is despawned due to being out of range.
#[derive(Event)]
pub struct ChunkUnloading {
    pub map_entity: Entity,
    pub chunk_pos: IVec3,
}
```

Server handler:
```rust
fn persist_chunk_entities_on_unload(
    mut events: MessageReader<ChunkUnloading>,
    chunk_entity_query: Query<(&ChunkEntityRef, &WorldObjectId, &Position, Option<&Health>, ...)>,
    map_query: Query<&VoxelMapConfig>,
    type_registry: Res<AppTypeRegistry>,
) {
    for event in events.read() {
        let config = map_query.get(event.map_entity).ok();
        let Some(save_dir) = config.and_then(|c| c.save_dir.as_ref()) else { continue; };

        let entities: Vec<ChunkEntity> = chunk_entity_query
            .iter()
            .filter(|(cr, ..)| cr.chunk_pos == event.chunk_pos && cr.map_entity == event.map_entity)
            .map(|(_, obj_id, pos, ..)| {
                // Serialize component state that differs from the WorldObjectDef baseline.
                // Compare current entity components against the def's default components.
                // Only serialize differences as component_overrides.
                ChunkEntity {
                    procedural_id: /* from ProceduralSpawnId component */,
                    object_id: obj_id.0.clone(),
                    position: pos.0,
                    destroyed: false,
                    component_overrides: /* diff against baseline */,
                }
            })
            .collect();

        if let Err(e) = save_chunk_entities(save_dir, event.chunk_pos, &entities) {
            error!("Failed to save chunk entities at {}: {e}", event.chunk_pos);
        }
    }
}
```

#### 4. ProceduralSpawnId component
**File**: `crates/voxel_map_engine/src/terrain.rs`
**Changes**: Component to track the deterministic ID of procedurally spawned objects.

```rust
/// Marks an entity as procedurally spawned with a deterministic ID.
/// Used to match persisted mutation data against regenerated spawns.
#[derive(Component, Clone, Debug)]
pub struct ProceduralSpawnId(pub u64);
```

Inserted by `spawn_chunk_world_objects` when spawning procedural objects.

#### 5. Component persistence (no separate "destroyed" tracking)

No `DestroyedObject` marker or `destroyed: bool` field needed. Instead, persist all serializable components on chunk entities. A dead object is simply one whose `Health.current == 0.0` — the health component is saved and restored like any other.

**On chunk unload** (serialization): For each entity with `ChunkEntityRef` matching the unloading chunk, iterate the entity's components using Bevy's reflection. For each component that has `ReflectComponent` and `ReflectSerialize` registered, serialize it as RON via `TypedReflectSerializer`. Store in `component_overrides: HashMap<String, String>` keyed by type path.

**On chunk load** (deserialization): Spawn the entity from its `WorldObjectDef` baseline (gives default components), then for each entry in `component_overrides`, deserialize the RON string via `TypedReflectDeserializer` and insert via `ReflectComponent::apply` (overwrites the baseline value).

This is straightforward because the infrastructure already exists:
- `ReflectComponent::insert` is used by `apply_object_components` (proven pattern)
- `TypedReflectSerializer` / `TypedReflectDeserializer` are used by `reflect_loader` for RON
- The type registry resolves type paths to concrete types

```rust
/// Serialize all reflected components on an entity into the override map.
fn serialize_entity_components(
    entity_ref: EntityRef,
    registry: &TypeRegistry,
    baseline_type_paths: &HashSet<String>, // type paths from WorldObjectDef
) -> HashMap<String, String> {
    let mut overrides = HashMap::new();
    for component_id in entity_ref.archetype().components() {
        let Some(info) = entity_ref.world().components().get_info(component_id) else { continue };
        let type_id = info.type_id().unwrap();
        let Some(registration) = registry.get(type_id) else { continue };

        // Only serialize components that are part of the world object definition
        // (skip engine components like Transform, ChunkEntityRef, etc.)
        if !baseline_type_paths.contains(registration.type_info().type_path()) { continue }

        let Some(reflect_comp) = registration.data::<ReflectComponent>() else { continue };
        let Some(component) = reflect_comp.reflect(entity_ref) else { continue };

        // Serialize to RON
        let serializer = TypedReflectSerializer::new(component, registry);
        match ron::ser::to_string_pretty(&serializer, ron::ser::PrettyConfig::default()) {
            Ok(ron_str) => {
                overrides.insert(
                    registration.type_info().type_path().to_string(),
                    ron_str,
                );
            }
            Err(e) => warn!("Failed to serialize component {}: {e}", registration.type_info().type_path()),
        }
    }
    overrides
}
```

**Dead object handling**: When `Health.current` reaches 0 and the entity is despawned from the world, the entity no longer exists in the ECS. To persist this, the death handler must:
1. Before despawning, serialize the entity's components into `component_overrides`
2. Store a `ChunkEntity` with the serialized state (health=0 will be in the overrides)
3. Write to a "pending save" buffer on the map instance

On chunk reload, the merge step spawns the entity from baseline + applies overrides. The entity spawns with `Health { current: 0.0, max: 50.0 }`. The existing respawn timer system detects `current == 0` and handles it normally (either respawn after timer or stay dead).

Alternatively, simpler: don't despawn the ECS entity on death at all. Just set health to 0, despawn visuals, disable colliders. The entity persists in the ECS and gets serialized normally on chunk unload. This avoids the "serialize before despawn" dance. The respawn timer system can restore the entity when the timer expires.

**Decision**: Keep dead entities alive in the ECS with health=0. This is simpler and consistent with the existing `RespawnTimerConfig` pattern.

#### 6. Save triggers (no dirty tracking)

No per-entity dirty tracking needed. Following Minecraft's model, chunk entities are saved at two points:

1. **Chunk unload**: When a chunk leaves the loaded set (`ChunkUnloading` event), all entities in that chunk are serialized and written to disk. This is the primary save path.
2. **Server shutdown**: `save_world_on_shutdown` iterates all currently loaded chunks and saves their entities.

Entity state changes between chunk load and unload are held in memory (the live ECS components). This is safe because:
- Entity state changes are infrequent (tree gets damaged, resource gets harvested)
- The data is small (a few components per entity)
- A crash between edits and chunk unload loses at most the in-flight changes — same as Minecraft's behavior
- The existing voxel dirty/debounce system handles frequent block edits; entity state doesn't need the same treatment

#### 7. Save all chunk entities on shutdown
**File**: `crates/server/src/map.rs`
**Changes**: In `save_world_on_shutdown`, iterate all loaded chunks (not just dirty ones) and save their entity state. This ensures no data loss on shutdown.

```rust
fn save_all_chunk_entities_on_shutdown(
    map_query: Query<(Entity, &VoxelMapInstance, &VoxelMapConfig)>,
    chunk_entity_query: Query<(Entity, &ChunkEntityRef, &WorldObjectId)>,
    world: &World,
    type_registry: Res<AppTypeRegistry>,
) {
    let registry = type_registry.0.read();
    for (map_entity, instance, config) in &map_query {
        let Some(save_dir) = &config.save_dir else { continue };

        // Group chunk entities by chunk position
        let mut by_chunk: HashMap<IVec3, Vec<ChunkEntity>> = HashMap::new();
        for (entity, chunk_ref, obj_id) in &chunk_entity_query {
            if chunk_ref.map_entity != map_entity { continue }
            let entity_ref = world.entity(entity);
            let overrides = serialize_entity_components(entity_ref, &registry, ...);
            by_chunk.entry(chunk_ref.chunk_pos).or_default().push(ChunkEntity {
                procedural_id: entity_ref.get::<ProceduralSpawnId>().map(|id| id.0),
                object_id: obj_id.0.clone(),
                position: entity_ref.get::<Position>().map(|p| p.0).unwrap_or_default(),
                component_overrides: overrides,
            });
        }

        for (chunk_pos, entities) in &by_chunk {
            if let Err(e) = save_chunk_entities(save_dir, *chunk_pos, entities) {
                error!("Failed to save chunk entities at {chunk_pos}: {e}");
            }
        }
    }
}
```

This runs as part of the existing `save_world_on_shutdown` flow, after voxel saves.

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check-all` passes
- [ ] Unit tests for `save_chunk_entities` / `load_chunk_entities` (round-trip)
- [ ] Unit tests for merge logic (procedural + persisted, component override application)
- [ ] Unit tests for `serialize_entity_components` (round-trip through RON)

#### Manual Verification:
- [ ] Damage a tree, walk away (chunk unloads), walk back — tree retains reduced health
- [ ] Kill a tree (health=0), walk away, return — tree spawns dead, respawn timer runs
- [ ] Trees spawn at same positions on chunk reload (deterministic)
- [ ] Server restart: overworld terrain regenerates, persisted entity state preserved
- [ ] New trees appear in newly explored chunks
- [ ] Entity files appear in `worlds/overworld/entities/` directory

---

## Testing Strategy

### Unit Tests:
- `build_noise_fn`: deterministic output for same seed, different output for different seeds
- `generate_heightmap_chunk`: non-flat terrain with HeightMap, flat terrain without
- `select_biome`: correct biome selection across height/moisture ranges, fallback to first
- `poisson_disk_2d`: minimum spacing guarantee, determinism, coverage
- `generate_chunk_objects`: biome filtering, density acceptance, deterministic IDs
- `save/load_chunk_entities`: round-trip serialization, empty file handling, version validation
- Merge logic: procedural + persisted, destroyed skipping, override application

### Integration Tests:
- Full pipeline: terrain def loaded → generator built → chunk generated → objects spawned
- Chunk load/unload cycle preserves entity state
- Multiple map types with different terrain defs

### Manual Testing Steps:
1. Start server, connect client — verify non-flat terrain renders
2. Walk around — terrain generates continuously, different biomes visible
3. Trees/objects appear in appropriate biomes with natural spacing
4. Walk far away from trees, return — same trees at same positions
5. Destroy a tree, walk away, return — tree remains destroyed
6. Restart server — terrain regenerates identically, destroyed trees stay destroyed
7. Check homebase — still flat terrain
8. Check that server logs show terrain def loading, generator building, object spawning

## Performance Considerations

- **Per-chunk 2D noise cache**: Pre-compute 18×18 heightmap and moisture map before iterating the 3D voxel array. Avoids redundant noise evaluation per Y level. Stack-allocated `[f64; 324]`.
- **Rate-limited task spawning**: Existing `MAX_TASKS_PER_FRAME = 32` cap applies to all generation tasks.
- **Noise evaluation cost**: `Fbm<Perlin>` with 6 octaves is the most expensive component. At 324 samples per chunk (18×18 footprint), this is ~2000 noise evaluations per chunk (324 × 6 octaves). Benchmarking required to verify this stays under the async task budget.
- **Poisson disk sampling**: O(N) per chunk where N is the number of placed objects. With `min_spacing = 4.0` on a 16×16 chunk, maximum ~16 objects per rule per chunk. Negligible cost.
- **Entity file I/O**: Per-chunk entity files are small (typically < 1KB). Atomic writes are fast. No compression needed.
- **Memory**: `ChunkEntityRef` is 20 bytes per entity. For 1000 loaded world objects across all chunks, this is 20KB.

## Migration Notes

- **Existing saved worlds**: Bumping `CHUNK_SAVE_VERSION` causes all old chunks to be regenerated with the new noise-based generator. Entity persistence files don't exist yet, so no migration needed there.
- **`spawn_test_tree` removal**: The single hardcoded tree at (5, 5, 5) is replaced by procedural placement. The `tree_circle.object.ron` definition is unchanged.
- **VoxelGenerator signature change** (Phase 4): Changes from `Fn(IVec3) -> Vec<WorldVoxel>` to `Fn(IVec3) -> (Vec<WorldVoxel>, Vec<WorldObjectSpawn>)`. All existing generator closures must be updated. `flat_terrain_voxels` returns `(voxels, Vec::new())`.

## References

- Research: `doc/research/2026-03-18-procedural-map-generation.md`
- Voxel map engine plan: `doc/plans/2026-02-28-voxel-map-engine.md`
- World object RON assets: `doc/plans/2026-03-14-world-object-ron-assets.md`
- Map persistence: `doc/plans/2026-03-09-map-as-directory-saving.md`
