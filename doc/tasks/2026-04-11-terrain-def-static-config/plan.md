# Implementation Plan

## Overview

Move static per-map-type fields (`chunk_size`, `padded_size`, `column_y_range`, `tree_height`, `bounds`) from `VoxelMapConfig` into a new `MapDimensions` reflection component stored in `.terrain.ron` files. Defer `spawn_overworld`/`spawn_homebase` from `Startup` to `OnEnter(AppState::Ready)` so terrain defs are guaranteed loaded before map construction. Remove the `TerrainDefApplied` gating dance (`apply_terrain_defs` + `build_terrain_generators`) — spawn now happens post-asset-load so everything can be constructed inline.

### Post-refactor field ownership

- **`MapDimensions`** (reflection component from def): `chunk_size`, `column_y_range`, `tree_height`, `bounds`
- **`VoxelMapConfig`** (runtime-resolved): `seed`, `generation_version`, `spawning_distance`, `save_dir`, `generates_chunks`
- **`VoxelMapInstance`** (unchanged — already caches): `chunk_size`, `padded_size`, `shape` (runtime)

### Design constraints

- `VoxelMapInstance` borrow contention: `MapDimensions` stays a separate read-only component so `Query<&MapDimensions>` readers keep running in parallel with `Query<&mut VoxelMapInstance>` mutators.
- Each arena has its own `.terrain.ron` template; bounds is always static from the def.
- `MapTransitionStart` message still carries `chunk_size`/`column_y_range`/`bounds` as-is (wire format unchanged, client ignores them and reads from its own `TerrainDefRegistry`). Trimming the wire format is out of scope.

---

## Phase 1: Add `MapDimensions` component + update `.terrain.ron` files

Additive only — new data path exists but nothing consumes it yet. Existing code continues to hardcode values.

### Changes

#### 1. `crates/voxel_map_engine/src/config.rs`

**Action**: modify

Add a new reflection component alongside `VoxelMapConfig`:

```rust
/// Static per-map-type dimensional config, loaded from `.terrain.ron`.
///
/// Inserted onto map entities via the terrain def pipeline. Separate from
/// `VoxelMapConfig` so systems that only need dimensional data can query
/// this component without contending with runtime state.
#[derive(Component, Reflect, Clone, Debug)]
#[reflect(Component)]
pub struct MapDimensions {
    /// Edge length of a chunk in voxels. Power of two, >= 8.
    pub chunk_size: u32,
    /// Inclusive-exclusive Y chunk range for column expansion: `(y_min, y_max)`.
    pub column_y_range: (i32, i32),
    /// Octree tree_height for this map.
    pub tree_height: u32,
    /// Fixed map dimensions. `None` = infinite generation.
    pub bounds: Option<IVec3>,
}

impl MapDimensions {
    /// `chunk_size + 2`.
    pub fn padded_size(&self) -> u32 {
        self.chunk_size + 2
    }
}
```

#### 2. `crates/voxel_map_engine/src/lib.rs`

**Action**: modify

Register the new component for reflection (add near existing `register_type` calls at line ~32):

```rust
app.register_type::<config::MapDimensions>();
```

#### 3. `assets/terrain/overworld.terrain.ron`

**Action**: modify

Add `MapDimensions` entry with current production values (chunk_size=64, column_y_range=(-2,2), tree_height=5, unbounded):

```ron
"voxel_map_engine::config::MapDimensions": (
    chunk_size: 64,
    column_y_range: (-2, 2),
    tree_height: 5,
    bounds: None,
),
```

#### 4. `assets/terrain/homebase.terrain.ron`

**Action**: modify

File is currently `{}`. Replace with:

```ron
{
    "voxel_map_engine::config::MapDimensions": (
        chunk_size: 32,
        column_y_range: (-4, 4),
        tree_height: 3,
        bounds: Some((4, 4, 4)),
    ),
}
```

#### 5. `assets/terrain/arena_hills.terrain.ron`

**Action**: modify

Add `MapDimensions` entry. Use existing arena defaults (chunk_size=16, tree_height=3, bounds=(10,4,10) from `VoxelMapInstance::arena` example uses):

```ron
"voxel_map_engine::config::MapDimensions": (
    chunk_size: 16,
    column_y_range: (-8, 8),
    tree_height: 3,
    bounds: Some((10, 4, 10)),
),
```

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test -p voxel_map_engine` passes (pre-existing unrelated `ticket::tests` failures ignored)
- [x] `cargo test -p server` passes
- [x] `cargo test -p client` passes

#### Manual
- [ ] `cargo server` launches without errors — `MapDimensions` deserializes from all three defs (check logs for any terrain def parse failures)

---

## Phase 2: Defer `spawn_overworld` (server) to `OnEnter(AppState::Ready)`

Server only. Move spawn scheduling, read dimensions from def, inline the terrain-def application and generator construction.

### Changes

#### 1. `crates/server/src/map.rs`

**Action**: modify

Change `spawn_overworld` scheduling (line ~460):

```rust
// Before
.add_systems(Startup, (spawn_overworld, load_startup_entities).chain())

// After
.add_systems(
    OnEnter(AppState::Ready),
    (spawn_overworld, load_startup_entities).chain(),
)
```

Rewrite `spawn_overworld` to read `MapDimensions` from the registry and inline everything:

```rust
pub fn spawn_overworld(
    mut commands: Commands,
    mut registry: ResMut<MapRegistry>,
    save_path: Res<WorldSavePath>,
    terrain_registry: Res<TerrainDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
) {
    let map_dir = map_save_dir(&save_path.0, &MapInstanceId::Overworld);

    let (seed, generation_version) = match load_map_meta(&map_dir) {
        Ok(Some(meta)) => (meta.seed, meta.generation_version),
        _ => (DEFAULT_OVERWORLD_SEED, GENERATION_VERSION),
    };

    let terrain_def = terrain_registry
        .get("overworld")
        .expect("overworld.terrain.ron must be loaded by AppState::Ready");
    let dimensions = extract_map_dimensions(terrain_def)
        .expect("overworld.terrain.ron must contain MapDimensions");

    let mut config = VoxelMapConfig::new(seed, generation_version, 2, false);
    config.save_dir = Some(map_dir);

    let instance = VoxelMapInstance::new(&dimensions);
    let shape = instance.shape.clone();

    let map = commands
        .spawn((
            instance,
            config,
            dimensions.clone(),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();

    // Apply remaining terrain components (HeightMap, BiomeRules, etc.) inline.
    let components = clone_terrain_components_excluding_dimensions(terrain_def);
    apply_object_components(&mut commands, map, components, type_registry.0.clone());

    // Build generator inline using the def components we just applied.
    let generator = build_generator_from_def(
        terrain_def,
        seed,
        dimensions.chunk_size,
        dimensions.padded_size(),
        shape,
    );
    commands.entity(map).insert(generator);

    commands.insert_resource(OverworldMap(map));
    registry.insert(MapInstanceId::Overworld, map);
}
```

Add helper to extract `MapDimensions` from a `TerrainDef`:

```rust
fn extract_map_dimensions(def: &TerrainDef) -> Option<MapDimensions> {
    def.components
        .iter()
        .find_map(|c| c.as_reflect().downcast_ref::<MapDimensions>().cloned())
}
```

Add helper to skip `MapDimensions` when applying the remaining components (since we already consumed it):

```rust
fn clone_terrain_components_excluding_dimensions(
    def: &TerrainDef,
) -> Vec<Box<dyn bevy::reflect::PartialReflect>> {
    def.components
        .iter()
        .filter(|c| c.as_reflect().downcast_ref::<MapDimensions>().is_none())
        .map(|c| {
            c.reflect_clone()
                .expect("terrain component must be cloneable")
                .into_partial_reflect()
        })
        .collect()
}
```

Add generator construction helper (replaces `build_terrain_generators` for the inline case):

```rust
fn build_generator_from_def(
    def: &TerrainDef,
    seed: u64,
    chunk_size: u32,
    padded_size: u32,
    shape: RuntimeShape<u32, 3>,
) -> VoxelGenerator {
    // Scan def for HeightMap/MoistureMap/BiomeRules/PlacementRules directly.
    let height = find_component::<HeightMap>(def);
    let moisture = find_component::<MoistureMap>(def);
    let biomes = find_component::<BiomeRules>(def);
    let placement = find_component::<PlacementRules>(def);
    build_generator_from_components(
        seed, chunk_size, padded_size, shape,
        height, moisture, biomes, placement,
    )
}
```

Note: `build_generator` in `terrain.rs` currently takes an `EntityRef`. Either refactor it to take the components directly (simpler with the inline flow), or call it after the `apply_object_components` commands flush — which is deferred. Prefer refactoring; see Phase 4.

#### 2. `crates/voxel_map_engine/src/instance.rs`

**Action**: modify

Update `VoxelMapInstance::new` to take a `MapDimensions` reference:

```rust
impl VoxelMapInstance {
    pub fn new(dimensions: &MapDimensions) -> Self {
        let chunk_size = dimensions.chunk_size;
        debug_assert!(chunk_size.is_power_of_two() && chunk_size >= 8);
        let padded_size = dimensions.padded_size();
        Self {
            tree: OctreeI32::new(dimensions.tree_height as u8),
            // ... existing fields unchanged ...
            chunk_size,
            padded_size,
            shape: RuntimeShape::<u32, 3>::new([padded_size, padded_size, padded_size]),
        }
    }
}
```

Keep `VoxelMapInstance::overworld`/`homebase`/`arena` convenience constructors for now (used by tests/examples); update them to build a `MapDimensions` internally with the same hardcoded values. They will be deleted in Phase 5.

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test -p server` passes (integration tests may need adjustment — see Phase 6)

#### Manual
- [ ] `cargo server` launches, overworld spawns after asset load (check trace logs for `"Applied terrain def 'overworld'"` or equivalent)
- [ ] Server continues to register entities, accept client connections

---

## Phase 3: Defer `spawn_overworld` (client) to `OnEnter(AppState::Ready)`

Same changes as Phase 2 but in client code.

### Changes

#### 1. `crates/client/src/map.rs`

**Action**: modify

Change scheduling (line ~57):

```rust
// Before
.add_systems(Startup, spawn_overworld)

// After
.add_systems(OnEnter(AppState::Ready), spawn_overworld)
```

Rewrite `spawn_overworld` to read `MapDimensions` from `TerrainDefRegistry`:

```rust
fn spawn_overworld(
    mut commands: Commands,
    mut registry: ResMut<MapRegistry>,
    terrain_registry: Res<TerrainDefRegistry>,
) {
    let terrain_def = terrain_registry
        .get("overworld")
        .expect("overworld.terrain.ron must be loaded by AppState::Ready");
    let dimensions = extract_map_dimensions(terrain_def)
        .expect("overworld.terrain.ron must contain MapDimensions");

    let mut config = VoxelMapConfig::new(0, 0, 2, false);
    config.generates_chunks = false;

    let instance = VoxelMapInstance::new(&dimensions);
    let padded = dimensions.padded_size();

    let map = commands
        .spawn((
            instance,
            config,
            dimensions.clone(),
            VoxelGenerator(Arc::new(FlatGenerator {
                chunk_size: dimensions.chunk_size,
                shape: RuntimeShape::<u32, 3>::new([padded, padded, padded]),
            })),
            Transform::default(),
            MapInstanceId::Overworld,
        ))
        .id();
    commands.insert_resource(OverworldMap(map));
    registry.insert(MapInstanceId::Overworld, map);
}
```

Copy `extract_map_dimensions` helper to client side (or pull into a shared location — probably `voxel_map_engine::terrain` since it's a one-liner).

**Recommended**: add to `voxel_map_engine/src/terrain.rs` as a public helper so both server + client can share:

```rust
pub fn extract_map_dimensions(def: &TerrainDef) -> Option<MapDimensions> {
    def.components
        .iter()
        .find_map(|c| c.as_reflect().downcast_ref::<MapDimensions>().cloned())
}
```

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test -p client` passes

#### Manual
- [ ] `cargo client` launches and connects to server
- [ ] Client overworld map is spawned with correct `chunk_size=64` (check via any existing debug overlay or by successful chunk sync)

---

## Phase 4: Update `spawn_homebase` (server) + `handle_map_transition_start` (client) to read from def

Homebase spawning runs in `Update`, not `Startup`, so no scheduling change. Just read from def instead of hardcoding.

### Changes

#### 1. `crates/server/src/map.rs`

**Action**: modify

Update `spawn_homebase` to read `MapDimensions` from def:

```rust
fn spawn_homebase(
    commands: &mut Commands,
    owner: u64,
    save_path: &WorldSavePath,
    registry: &mut MapRegistry,
    map_id: &MapInstanceId,
    terrain_registry: &TerrainDefRegistry,
    type_registry: &AppTypeRegistry,
) -> (Entity, MapTransitionParams) {
    let map_dir = map_save_dir(&save_path.0, map_id);
    let seed = load_homebase_seed(&map_dir, owner);

    let terrain_def = terrain_registry
        .get("homebase")
        .expect("homebase.terrain.ron must be loaded");
    let dimensions = extract_map_dimensions(terrain_def)
        .expect("homebase.terrain.ron must contain MapDimensions");

    let bounds = dimensions.bounds;
    let spawning_distance = bounds_to_spawning_distance(bounds.unwrap_or(IVec3::ONE));

    let mut config = VoxelMapConfig::new(seed, 0, spawning_distance, true);
    config.save_dir = Some(map_dir);

    let instance = VoxelMapInstance::new(&dimensions);
    let shape = instance.shape.clone();

    let params = MapTransitionParams {
        seed,
        generation_version: 0,
        bounds,
        chunk_size: dimensions.chunk_size,
        column_y_range: dimensions.column_y_range,
    };

    let entity = commands
        .spawn((
            instance,
            config,
            dimensions.clone(),
            Homebase { owner },
            Transform::default(),
            map_id.clone(),
        ))
        .id();

    // Apply non-dimensions components + build generator inline (same pattern as spawn_overworld).
    let components = clone_terrain_components_excluding_dimensions(terrain_def);
    apply_object_components(commands, entity, components, type_registry.0.clone());

    let generator = build_generator_from_def(
        terrain_def, seed,
        dimensions.chunk_size, dimensions.padded_size(), shape,
    );
    commands.entity(entity).insert(generator);

    registry.insert(map_id.clone(), entity);

    let entity_count = load_map_entities(commands, save_path, map_id);
    if entity_count > 0 {
        trace!("Loaded {entity_count} entities for homebase-{owner}");
    }

    (entity, params)
}
```

Propagate the new `terrain_registry`/`type_registry` params up through `ensure_map_exists` and its callers. Caller chain: `handle_map_switch_requests` → `execute_server_transition` → `ensure_map_exists` → `spawn_homebase`. Add `terrain_registry: Res<TerrainDefRegistry>`, `type_registry: Res<AppTypeRegistry>` to the top-level system's signature.

#### 2. `crates/client/src/map.rs`

**Action**: modify

In `handle_map_transition_start`, when spawning a new map instance (line ~440), read `MapDimensions` from `TerrainDefRegistry` instead of `transition.chunk_size`/`transition.column_y_range`:

```rust
pub fn handle_map_transition_start(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<MapTransitionStart>>,
    mut registry: ResMut<MapRegistry>,
    terrain_registry: Res<TerrainDefRegistry>,
    player_query: Query<Entity, (With<Predicted>, With<CharacterMarker>, With<Controlled>)>,
    world_objects: Query<(Entity, &MapInstanceId), With<WorldObjectId>>,
) {
    // ... existing loop ...

    if !registry.0.contains_key(&transition.target) {
        let def_name = terrain_def_name(&transition.target);
        let terrain_def = terrain_registry
            .get(&def_name)
            .expect("terrain def must be loaded");
        let dimensions = extract_map_dimensions(terrain_def)
            .expect("terrain def must contain MapDimensions");

        let generator = generator_for_map(&transition.target, &dimensions);
        let map_entity = spawn_map_instance(
            &mut commands,
            &transition.target,
            transition.seed,
            &dimensions,
            generator,
        );
        registry.insert(transition.target.clone(), map_entity);
    }
    // ... rest unchanged ...
}
```

Update `generator_for_map` to take `&MapDimensions`:

```rust
fn generator_for_map(map_id: &MapInstanceId, dimensions: &MapDimensions) -> VoxelGenerator {
    let padded = dimensions.padded_size();
    let flat = || FlatGenerator {
        chunk_size: dimensions.chunk_size,
        shape: RuntimeShape::<u32, 3>::new([padded, padded, padded]),
    };
    match map_id {
        MapInstanceId::Overworld => VoxelGenerator(Arc::new(flat())),
        MapInstanceId::Homebase { .. } => VoxelGenerator(Arc::new(flat())),
    }
}
```

Update `spawn_map_instance` to take `&MapDimensions`:

```rust
fn spawn_map_instance(
    commands: &mut Commands,
    map_id: &MapInstanceId,
    seed: u64,
    dimensions: &MapDimensions,
    generator: VoxelGenerator,
) -> Entity {
    let spawning_distance = dimensions
        .bounds
        .map(|b| b.max_element().max(1) as u32)
        .unwrap_or(10);

    let mut config = VoxelMapConfig::new(seed, 0, spawning_distance, true);
    config.generates_chunks = false;

    let instance = VoxelMapInstance::new(dimensions);

    commands
        .spawn((
            instance,
            config,
            dimensions.clone(),
            generator,
            Transform::default(),
            map_id.clone(),
        ))
        .id()
}
```

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test -p server` passes
- [x] `cargo test -p client` passes

#### Manual
- [ ] Player map-switches to homebase, homebase loads with `chunk_size=32` from def
- [ ] Player map-switches back to overworld
- [ ] Homebase save/reload still works

---

## Phase 5: Remove static fields from `VoxelMapConfig`

Now that all consumers have a `MapDimensions` component available, remove the duplicated fields from `VoxelMapConfig`.

### Changes

#### 1. `crates/voxel_map_engine/src/config.rs`

**Action**: modify

Strip `VoxelMapConfig`:

```rust
#[derive(Component)]
pub struct VoxelMapConfig {
    pub seed: u64,
    pub generation_version: u32,
    pub spawning_distance: u32,
    pub save_dir: Option<PathBuf>,
    pub generates_chunks: bool,
}

impl VoxelMapConfig {
    pub fn new(
        seed: u64,
        generation_version: u32,
        spawning_distance: u32,
        generates_chunks: bool,
    ) -> Self {
        debug_assert!(spawning_distance > 0, "VoxelMapConfig: spawning_distance must be > 0");
        Self {
            seed,
            generation_version,
            spawning_distance,
            save_dir: None,
            generates_chunks,
        }
    }
}
```

Note: `generates_chunks` moves into the constructor signature (was set post-construction via `config.generates_chunks = false` on the client). Update client spawn to pass it directly.

#### 2. `crates/voxel_map_engine/src/lifecycle.rs`

**Action**: modify

Systems reading `config.column_y_range`, `config.bounds` switch to `Query<&MapDimensions>`:

- Line ~336: Query tuple that currently has `&VoxelMapConfig` → add `&MapDimensions` (or replace if config fields are no longer needed).
- Line ~368: `config.column_y_range` → `dimensions.column_y_range`.
- Lines ~389, ~394, ~405: `config.bounds` → `dimensions.bounds`.
- Line ~435: Query tuple — same treatment.
- Line ~653 (`fn` signature): change `config: &VoxelMapConfig` to `dimensions: &MapDimensions` where the function only reads dimensional data. If it reads both, split params.
- Line ~692: `config.bounds` → `dimensions.bounds`.

#### 3. `crates/voxel_map_engine/src/api.rs`

**Action**: modify

`VoxelWorld` SystemParam (line ~14-24): `&'static VoxelMapConfig` → drop it or replace with `&'static MapDimensions`. Audit whether it's read at all — grep shows `_config` bindings unused in `get_voxel`/`set_voxel`; likely just drop.

#### 4. `crates/voxel_map_engine/src/propagator.rs`

**Action**: modify

Any `config.column_y_range` / `config.bounds` reads → `dimensions.*`. Confirm via grep after Phase 5 changes.

#### 5. `crates/server/src/map.rs`

**Action**: modify

- Line ~194-195 (`build_terrain_generators`): delete this function entirely — Phase 2/4 moved generator construction inline.
- Lines ~756, ~767 (`config.column_y_range`): query `&MapDimensions` from these systems instead.
- Lines ~1045-1047, ~1083-1085 (`MapTransitionParams` construction): read from `MapDimensions` component instead of config.

#### 6. `crates/server/src/chunk_entities.rs`

**Action**: modify

- Line ~25 and ~99: Query tuples — replace `&VoxelMapConfig` with appropriate component (audit what's actually read).
- Line ~76: function signature — change param type if reading removed fields.
- Line ~157: `Query<&VoxelMapConfig>` → `Query<&MapDimensions>` if reading chunk_size/bounds, else adjust.

#### 7. `crates/client/src/map.rs`

**Action**: modify

- Line ~183: Query tuple update.
- Line ~194: `config.column_y_range` → read from `MapDimensions`.

#### 8. `crates/voxel_map_engine/src/instance.rs`

**Action**: modify

Delete convenience constructors `VoxelMapInstance::overworld`, `homebase`, `arena` — tests should construct `MapDimensions` + `VoxelMapInstance::new` directly. Update in-file tests accordingly.

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test -p voxel_map_engine` passes
- [x] `cargo test -p server` passes
- [x] `cargo test -p client` passes
- [x] grep: `config.chunk_size`, `config.padded_size`, `config.bounds`, `config.column_y_range`, `config.tree_height` → zero matches in `crates/`

#### Manual
- [ ] `cargo server` + `cargo client` still run full happy path: connect, load overworld, transition to homebase, edit blocks, save, reload

---

## Phase 6: Remove `apply_terrain_defs` / `build_terrain_generators` / `TerrainDefApplied`

Final cleanup — these systems exist only because spawn used to happen before the def was loaded.

### Changes

#### 1. `crates/server/src/map.rs`

**Action**: modify

- Delete `struct TerrainDefApplied` (line ~130).
- Delete `fn apply_terrain_defs` (line ~136).
- Delete `fn clone_terrain_components` (line ~162) — replaced by `clone_terrain_components_excluding_dimensions` helper introduced in Phase 2 (or consolidate).
- Delete `fn build_terrain_generators` (line ~179).
- Delete `fn terrain_def_name` (line ~154) unless still used elsewhere — move to a shared location if client uses it.
- Remove `apply_terrain_defs` and `build_terrain_generators` from system registration (line ~464).

#### 2. `crates/voxel_map_engine/src/terrain.rs`

**Action**: modify

- `build_generator` (line ~515): current signature takes `EntityRef`. Refactor to take the four optional components directly (`Option<HeightMap>`, `Option<MoistureMap>`, etc.). Rename to `build_generator_from_components` if that clarifies. This is needed because Phase 2's inline construction doesn't have access to an `EntityRef` with the components inserted yet — it has the raw `TerrainDef`.

```rust
pub fn build_generator_from_components(
    seed: u64,
    chunk_size: u32,
    padded_size: u32,
    shape: RuntimeShape<u32, 3>,
    height: Option<HeightMap>,
    moisture: Option<MoistureMap>,
    biomes: Option<BiomeRules>,
    placement: Option<PlacementRules>,
) -> VoxelGenerator {
    // ... existing body, minus the `entity.get::<>()` calls at the top ...
}
```

Delete the old `build_generator(entity: EntityRef, ...)` signature.

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] All tests pass
- [x] grep: `TerrainDefApplied`, `apply_terrain_defs`, `build_terrain_generators` → zero matches in `crates/`

#### Manual
- [ ] Full end-to-end flow still works: server + client launch, map loads, transitions work, edits work, save/reload works

---

## Phase 7: Tests and examples cleanup

Sweep all test call sites and examples for the removed `VoxelMapConfig::new` signature and deleted convenience constructors.

### Changes

#### 1. `crates/voxel_map_engine/tests/api.rs`

**Action**: modify

- Line 33: `VoxelMapConfig::new(0, 0, spawning_distance, None, 5, 16, (-8, 8))` → `VoxelMapConfig::new(0, 0, spawning_distance, true)`.
- Spawn tuples need `MapDimensions { chunk_size: 16, column_y_range: (-8, 8), tree_height: 5, bounds: None }` added.
- `VoxelMapInstance::new(5, 16)` → `VoxelMapInstance::new(&dimensions)`.

#### 2. `crates/voxel_map_engine/tests/lifecycle.rs`

**Action**: modify

- Line 19: same signature update.
- Line 124: same (bounded case).

#### 3. `crates/voxel_map_engine/examples/terrain.rs`, `editing.rs`, `multi_instance.rs`

**Action**: modify

- Update `VoxelMapConfig::new` calls to new signature.
- Add `MapDimensions` components to spawn tuples.
- `multi_instance.rs`: replace `VoxelMapInstance::overworld`/`homebase`/`arena` calls with direct `MapDimensions` + `VoxelMapInstance::new` construction.

#### 4. `crates/server/tests/integration.rs`

**Action**: modify

- Lines 851, 1201, 1357, 1489, 1609: `VoxelMapConfig::new` signature updates.
- Lines 1290-1303: tests that read `config.bounds`, `config.tree_height` → read from `MapDimensions` component.
- Spawn tuples: add `MapDimensions` component.

#### 5. `crates/server/tests/map_transition.rs`

**Action**: modify

- Lines 106-107: `VoxelMapInstance::homebase(...)` calls — replace with explicit construction.

#### 6. `crates/protocol/tests/physics_isolation.rs`

**Action**: modify

- Lines 147, 156, 250: `VoxelMapConfig::new` signature updates.
- Spawn tuples: add `MapDimensions`.

#### 7. `crates/client/tests/chunk_sync.rs`

**Action**: modify

- Line 27: signature update.
- Spawn tuple: add `MapDimensions`.

#### 8. `crates/client/tests/map_transition.rs`

**Action**: modify

- Signature + component updates as above.

### Verification

#### Automated
- [ ] `cargo check-all` passes
- [ ] `cargo test --workspace` passes (all crates, all tests)

#### Manual
- [ ] `cargo run --example multi_instance` runs (from `voxel_map_engine`)
- [ ] `cargo run --example terrain` runs
- [ ] `cargo run --example editing` runs

---

## Notes / Open Items

- **`MapDimensions` location**: placed in `voxel_map_engine::config` since it's dimensional config. Could arguably live in `terrain.rs` alongside other reflection components. Keeping it in `config.rs` keeps the module purpose clear: config-shaped data, whether from file or runtime.
- **`generates_chunks` constructor param**: Phase 5 moves this from post-construction mutation to constructor parameter. Server sets `true`, client sets `false`. This is a minor API change but cleaner than the current `config.generates_chunks = false` pattern.
- **`MapTransitionStart` wire format**: still carries `chunk_size`/`column_y_range`/`bounds`. Client ignores them and reads from def. Trimming the wire format is deferred; it requires versioning considerations.
- **Arena template**: only `arena_hills.terrain.ron` exists. If more arena types are added later, each needs its own def with its own `MapDimensions`.
