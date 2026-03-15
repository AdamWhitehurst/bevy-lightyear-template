# World Object RON Assets Implementation Plan

## Overview

Implement a data-driven world object definition system using RON assets with Reflect-based component insertion. World objects (trees, buildings, ores, NPCs, items) are defined in `.object.ron` files as **flat maps of type-path → component data**. A custom `AssetLoader` deserializes arbitrary ECS components through Bevy's `TypeRegistry`, and the resulting `WorldObjectDef` is a single `Vec<Box<dyn PartialReflect>>`. Objects are spawned exclusively by the server; clients react to replicated entities via Lightyear's `Added<Replicated>` observer pattern.

## Current State Analysis

- **Ability system** provides the complete loading pattern: per-file RON assets, manifest for WASM, `TrackedAssets` load-gating, `AssetEvent`-driven hot-reload, aggregation into a `HashMap` resource.
- **No world object infrastructure** exists. The only placed entities are `RespawnPoint` and a hardcoded `DummyTarget`.
- **No reflection infrastructure** exists — all `Reflect` derives are for Lightyear networking. No `register_type`, `ReflectComponent`, or `TypeRegistry` usage.
- **`bevy_common_assets` `RonAssetPlugin`** cannot be used because it uses plain `serde::Deserialize` with no `TypeRegistry` access. A custom `AssetLoader` is required.
- **Lightyear room visibility**: An observer `on_map_instance_id_added` ([server/src/map.rs:323-339](../crates/server/src/map.rs#L323-L339)) automatically adds any entity with `MapInstanceId` to the appropriate Lightyear room. Adding `MapInstanceId` to spawned world objects is sufficient for correct replication scoping.

### Key Discoveries

- `AssetLoader` requires `FromWorld` to access `AppTypeRegistry` at loader construction time.
- `TypedReflectDeserializer` + `TypeRegistrationDeserializer` (from `bevy::reflect::serde`) are the exact pair used by Bevy's `SceneMapDeserializer` — the correct reference implementation.
- `ReflectComponent::insert` accepts `&dyn PartialReflect` directly; no intermediate `ron::Value` or `String` representation is needed.
- Static world objects use `Replicate` but **no `PredictionTarget`** — Lightyear therefore creates **exactly one entity** on each client (the confirmed `Replicated` entity). There is no shadow predicted/interpolated copy. This means clients never see duplicate world object entities.
- Room integration is automatic: `MapInstanceId` on a server entity triggers `RoomEvent::AddEntity` via the existing observer without any additional code at the spawn site.
- Message API (confirmed from `PlayerMapSwitchRequest` pattern): `Query<&mut MessageSender<T>>` on client, `Query<(Entity, &mut MessageReceiver<T>)>` on server, with `.send::<Channel>(msg)` / `.receive()`.
- **avian3d `serialize` feature** enables `Serialize`/`Deserialize` on physics types (`ColliderConstructor`, `RigidBody`, `CollisionLayers`, etc.), allowing them to appear directly in the RON component map without wrapper types.
- **lightyear `avian3d` feature** enables Lightyear's built-in replication support for avian physics components.

## Desired End State

1. `.object.ron` files in `assets/objects/` define world objects as flat type-path → component maps
2. `WorldObjectDefRegistry` resource holds all loaded definitions, keyed by `WorldObjectId`
3. Hot-reload updates definitions at runtime
4. Server spawns world objects via `spawn_world_object` (server crate); Lightyear replicates them to clients
5. Client reacts to replicated world objects via `Added<Replicated>` + `With<WorldObjectId>`, attaches reflected components from `WorldObjectDefRegistry` and a placeholder mesh
6. Unit tests verify deserialization success and error paths
7. Integration tests verify the full pipeline: RON file → load → query components
8. Debug UI button sends a `SpawnWorldObjectRequest` to the server, which spawns a test tree at the requested position

### Example RON file (`assets/objects/tree_circle.object.ron`):
```ron
{
    "protocol::world_object::types::ObjectCategory": Scenery,
    "protocol::world_object::types::VisualKind": Vox("models/trees/tree_circle.vox"),
    "avian3d::collision::collider::constructor::ColliderConstructor": Cylinder(radius: 0.5, height: 3.0),
    "protocol::Health": (
        current: 50.0,
        max: 50.0,
    ),
    "avian3d::dynamics::rigid_body::RigidBody": Static,
    "avian3d::collision::collider::layers::CollisionLayers": (
        memberships: (16),
        filters: (2),
    ),
    "avian3d::physics_transform::transform::Position": ((5.0, 5.0, 5.0)),
}
```

## What We're NOT Doing

- Procedural generation placement — separate future system
- Extending `SavedEntity`/`SavedEntityKind` for persistence — separate plan
- Loading `.vox` models — covered by separate vox loading research/plan
- Visual asset resolution at spawn time — deferred until vox/sprite rendering is ready
- Interest management / spatial filtering for large worlds — future work
- Auto-updating `objects.manifest.ron` when files are added/removed — the manifest is manually maintained, same as `abilities.manifest.ron`

## Implementation Approach

Follow the ability system pattern exactly for loading infrastructure. Diverge only where the custom `AssetLoader` replaces `RonAssetPlugin` (required for `TypeRegistry` access). Shared utilities live in `protocol`. Server spawning lives in `server`. Client reaction lives in `client`. Component types used in RON files must be registered with `#[reflect(Component)]` and `app.register_type::<T>()`.

**Key design decision**: `WorldObjectDef` contains only `components: Vec<Box<dyn PartialReflect>>` — a flat list of type-erased components. There are no typed fields for `category`, `visual`, or `collider`. Everything is a component in the same flat map. This eliminates the need for a custom struct-level deserializer (`WorldObjectDefSeed`/`WorldObjectDefVisitor`) and makes the RON format maximally extensible — any registered `Component` type can be added to an object definition without code changes.

---

## Phase 1: Core Types, Custom AssetLoader, and Dependency Features

### Overview
Define `WorldObjectDef`, `WorldObjectId`, `ObjectCategory`, `VisualKind`. Implement `WorldObjectLoader` using `TypedReflectDeserializer`. Add `serialize` feature to avian3d and `avian3d` feature to lightyear. Register the loader with Bevy.

### Changes Required:

#### 1. Add dependency features
**File**: `Cargo.toml` (workspace root)

Add `serialize` feature to avian3d to enable `Serialize`/`Deserialize` on physics types:
```toml
avian3d = { version = "0.5.0", default-features = false, features = ["3d", "f32", "parry-f32", "default-collider", "collider-from-mesh", "serialize"] }
```

**File**: `crates/protocol/Cargo.toml`

Add `bevy_asset` to bevy features, `avian3d` feature to lightyear, and `serialize` feature to avian3d:
```toml
[dependencies]
avian3d = { workspace = true, features = ["serialize"] }
bevy = { workspace = true, features = ["bevy_color", "bevy_state", "bevy_asset"] }
lightyear = { workspace = true, features = ["avian3d", "leafwing"] }
serde = { version = "1.0", features = ["derive"] }
```

#### 2. Create world_object module

The module is split into focused files under `crates/protocol/src/world_object/`:

| File | Contents |
|---|---|
| `mod.rs` | Re-exports |
| `types.rs` | Core types: `WorldObjectId`, `ObjectCategory`, `VisualKind`, `WorldObjectDef`, `WorldObjectLoadError` |
| `loader.rs` | `WorldObjectLoader`, serde visitors, `deserialize_world_object` |
| `spawn.rs` | `apply_object_components`, `insert_reflected_component` |
| `registry.rs` | `WorldObjectDefRegistry`, `WorldObjectManifest` |
| `loading.rs` | Internal handle resources, load systems, hot-reload, `object_id_from_path` |
| `plugin.rs` | `WorldObjectPlugin` |

##### Types
**File**: `crates/protocol/src/world_object/types.rs`

`WorldObjectId` gets the full set of derives it will need across all phases — including Lightyear networking — from the start. `ObjectCategory` and `VisualKind` are `Component` types with `#[reflect(Component, Serialize, Deserialize)]` so they can appear in the RON component map:

```rust
use bevy::prelude::*;
use bevy::reflect::PartialReflect;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique identifier for a world object definition. Derived from the `.object.ron` filename.
///
/// Also used as a replicated ECS component — the single component Lightyear sends to clients
/// to identify which definition to look up in `WorldObjectDefRegistry`.
#[derive(Component, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct WorldObjectId(pub String);

/// Broad classification of world objects.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
#[reflect(Component, Serialize, Deserialize)]
pub enum ObjectCategory {
    Scenery,
    Interactive,
    ResourceNode,
    Item,
    Npc,
}

/// How the object is visually represented.
///
/// Visual assets are resolved lazily at spawn time via `asset_server.load`, following
/// the sprite rig cross-reference pattern. Deferred to the vox loading plan.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
#[reflect(Component, Serialize, Deserialize)]
pub enum VisualKind {
    /// Path to a .vox model relative to assets/.
    Vox(String),
    /// Path to a .rig.ron file.
    SpriteRig(String),
    /// Path to a sprite image.
    Sprite(String),
    /// No visual (server-only or invisible).
    None,
}

/// A loaded world object definition.
///
/// All fields are stored as type-erased reflect components. They are inserted via
/// `apply_object_components`, which uses `ReflectComponent::insert` on each.
#[derive(Asset, TypePath)]
pub struct WorldObjectDef {
    /// Reflect components deserialized from RON via `TypeRegistry`.
    /// Inserted on both server and client via `apply_object_components`.
    pub components: Vec<Box<dyn PartialReflect>>,
}

impl Clone for WorldObjectDef {
    fn clone(&self) -> Self {
        Self {
            components: self
                .components
                .iter()
                .map(|c| {
                    c.reflect_clone()
                        .expect("world object component must be cloneable")
                        .into_partial_reflect()
                })
                .collect(),
        }
    }
}

impl fmt::Debug for WorldObjectDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorldObjectDef")
            .field(
                "components",
                &self
                    .components
                    .iter()
                    .map(|c| c.reflect_type_path())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}
```

##### Custom AssetLoader
**File**: `crates/protocol/src/world_object/loader.rs`

```rust
/// Custom asset loader that uses `TypeRegistry` for reflect-based component deserialization.
#[derive(TypePath)]
pub(super) struct WorldObjectLoader {
    type_registry: TypeRegistryArc,
}

impl FromWorld for WorldObjectLoader {
    fn from_world(world: &mut World) -> Self {
        Self {
            type_registry: world.resource::<AppTypeRegistry>().0.clone(),
        }
    }
}

impl AssetLoader for WorldObjectLoader {
    type Asset = WorldObjectDef;
    type Settings = ();
    type Error = WorldObjectLoadError;

    fn extensions(&self) -> &[&str] {
        &["object.ron"]
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
        deserialize_world_object(&bytes, &registry)
    }
}
```

##### Deserialization
**File**: `crates/protocol/src/world_object/loader.rs` (continued)

The RON file is a flat map of type paths to component data. The entire file is deserialized by `ComponentMapDeserializer` — no struct-level visitor needed:

```rust
/// Deserializes a `WorldObjectDef` from RON bytes using the given `TypeRegistry`.
///
/// The RON format is a flat map of type paths to component data:
/// ```ron
/// {
///     "protocol::world_object::ObjectCategory": Scenery,
///     "protocol::world_object::VisualKind": Vox("models/trees/tree.vox"),
///     "protocol::Health": (current: 50.0, max: 50.0),
/// }
/// ```
pub fn deserialize_world_object(
    bytes: &[u8],
    registry: &TypeRegistry,
) -> Result<WorldObjectDef, WorldObjectLoadError> {
    let mut deserializer = ron::de::Deserializer::from_bytes(bytes)?;
    let components = ComponentMapDeserializer { registry }.deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(WorldObjectDef { components })
}

struct ComponentMapDeserializer<'a> {
    registry: &'a TypeRegistry,
}

impl<'a, 'de> DeserializeSeed<'de> for ComponentMapDeserializer<'a> {
    type Value = Vec<Box<dyn PartialReflect>>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(ComponentMapVisitor {
            registry: self.registry,
        })
    }
}

struct ComponentMapVisitor<'a> {
    registry: &'a TypeRegistry,
}

impl<'a, 'de> Visitor<'de> for ComponentMapVisitor<'a> {
    type Value = Vec<Box<dyn PartialReflect>>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a map of component type paths to component data")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let mut components = Vec::new();
        while let Some(registration) =
            map.next_key_seed(TypeRegistrationDeserializer::new(self.registry))?
        {
            let value =
                map.next_value_seed(TypedReflectDeserializer::new(registration, self.registry))?;
            // Attempt to convert the dynamic representation to a concrete type.
            let value = self
                .registry
                .get(registration.type_id())
                .and_then(|tr| tr.data::<ReflectFromReflect>())
                .and_then(|fr| fr.from_reflect(value.as_partial_reflect()))
                .map(PartialReflect::into_partial_reflect)
                .unwrap_or(value);
            components.push(value);
        }
        Ok(components)
    }
}
```

##### Error type
**File**: `crates/protocol/src/world_object/types.rs` (continued)

No `thiserror` or `anyhow` in the workspace — implement standard error traits manually. Includes `From<ron::error::Error>` in addition to `From<ron::error::SpannedError>` because `deserializer.end()` returns the unspanned variant:

```rust
#[derive(Debug)]
pub enum WorldObjectLoadError {
    Io(std::io::Error),
    Ron(ron::error::SpannedError),
}

impl fmt::Display for WorldObjectLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Ron(e) => write!(f, "RON error: {e}"),
        }
    }
}

impl std::error::Error for WorldObjectLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Ron(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for WorldObjectLoadError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}

impl From<ron::error::SpannedError> for WorldObjectLoadError {
    fn from(e: ron::error::SpannedError) -> Self { Self::Ron(e) }
}

impl From<ron::error::Error> for WorldObjectLoadError {
    fn from(e: ron::error::Error) -> Self {
        Self::Ron(ron::error::SpannedError {
            code: e,
            span: ron::error::Span {
                start: ron::error::Position { line: 0, col: 0 },
                end: ron::error::Position { line: 0, col: 0 },
            },
        })
    }
}
```

##### Shared component insertion utility
**File**: `crates/protocol/src/world_object/spawn.rs`

Used by both server spawn and client observer to insert reflected components onto an entity:

```rust
/// Queues a command to insert all reflected components from a `WorldObjectDef` onto `entity`.
///
/// Must be called via `commands.queue` because `ReflectComponent::insert` requires
/// `EntityWorldMut`, which is only available in command execution.
pub fn apply_object_components(
    commands: &mut Commands,
    entity: Entity,
    components: Vec<Box<dyn PartialReflect>>,
    registry: TypeRegistryArc,
) {
    commands.queue(move |world: &mut World| {
        let registry = registry.read();
        let mut entity_mut = world.entity_mut(entity);
        for component in &components {
            insert_reflected_component(&mut entity_mut, component.as_ref(), &registry);
        }
    });
}

fn insert_reflected_component(
    entity_mut: &mut EntityWorldMut,
    component: &dyn PartialReflect,
    registry: &TypeRegistry,
) {
    let type_path = component.reflect_type_path();
    let Some(registration) = registry.get_with_type_path(type_path) else {
        warn!("World object component type not registered: {type_path}");
        return;
    };
    let Some(reflect_component) = registration.data::<ReflectComponent>() else {
        warn!("Type missing #[reflect(Component)]: {type_path}");
        return;
    };
    reflect_component.insert(entity_mut, component, registry);
}
```

#### 3. Unit tests for deserialization
**File**: `crates/protocol/src/world_object/loader.rs` (in `#[cfg(test)] mod tests`)

`deserialize_world_object` is a pure function — testable without a Bevy `App`. Build a `TypeRegistry` manually, register needed types, test directly. Tests use the flat map format:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_object::types::{ObjectCategory, VisualKind};
    use crate::Health;

    fn test_registry() -> TypeRegistry {
        let mut registry = TypeRegistry::default();
        registry.register::<Health>();
        registry.register::<ObjectCategory>();
        registry.register::<VisualKind>();
        registry
    }

    #[test]
    fn deserialize_valid_world_object() {
        let registry = test_registry();
        let ron = br#"{
            "protocol::world_object::types::ObjectCategory": Scenery,
            "protocol::world_object::types::VisualKind": Vox("models/trees/tree_circle.vox"),
            "protocol::Health": (current: 50.0, max: 50.0),
        }"#;
        let def = deserialize_world_object(ron, &registry).unwrap();
        assert_eq!(def.components.len(), 3);
    }

    #[test]
    fn deserialize_empty_components() {
        let registry = test_registry();
        let ron = br#"{}"#;
        let def = deserialize_world_object(ron, &registry).unwrap();
        assert!(def.components.is_empty());
    }

    #[test]
    fn deserialize_unregistered_type_errors() {
        let registry = TypeRegistry::default();
        let ron = br#"{
            "protocol::Health": (current: 1.0, max: 1.0),
        }"#;
        assert!(deserialize_world_object(ron, &registry).is_err());
    }

    #[test]
    fn deserialize_malformed_ron_errors() {
        let registry = test_registry();
        assert!(deserialize_world_object(b"not valid ron {{{", &registry).is_err());
    }
}
```

#### 4. Register the module
**File**: `crates/protocol/src/lib.rs`

Add `pub mod world_object;` and re-export key types from `world_object/mod.rs`.

### Success Criteria:

#### Automated Verification:
- [x] Workspace compiles: `cargo check-all`
- [x] Unit tests pass: `cargo test -p protocol -- world_object`

#### Manual Verification:
- [x] None yet — no runtime behavior in this phase

---

## Phase 2: Loading Infrastructure

### Overview
Implement the full loading lifecycle: startup load, WASM manifest support, aggregation into `WorldObjectDefRegistry` resource, hot-reload. Follows the ability system pattern exactly.

### Changes Required:

#### 1. Aggregation resource and manifest type
**File**: `crates/protocol/src/world_object/registry.rs`

```rust
/// All loaded world object definitions, keyed by ID.
///
/// Populated during `AppState::Loading` via `WorldObjectPlugin` systems.
/// Available to both server and client after `AppState::Ready`.
#[derive(Resource, Clone, Debug)]
pub struct WorldObjectDefRegistry {
    pub objects: HashMap<WorldObjectId, WorldObjectDef>,
}

impl WorldObjectDefRegistry {
    pub fn get(&self, id: &WorldObjectId) -> Option<&WorldObjectDef> {
        self.objects.get(id)
    }
}

/// Lists object IDs for WASM builds (where `load_folder` is unavailable).
///
/// Must be updated manually when `.object.ron` files are added or removed —
/// the same convention as `abilities.manifest.ron`.
#[derive(Deserialize, Asset, TypePath)]
pub struct WorldObjectManifest(pub Vec<String>);
```

#### 2. Internal handle resources
**File**: `crates/protocol/src/world_object/loading.rs`

These are internal to the loading systems; they track asset handles so `TrackedAssets` and the insert system can reference them:

```rust
/// Holds the folder handle returned by `load_folder("objects")` (native only).
/// Kept alive to prevent asset unloading; also used to enumerate loaded objects.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource)]
struct ObjectFolderHandle(Handle<LoadedFolder>);

/// Holds the manifest handle (WASM only).
/// Once loaded, the insert system reads the list of IDs and starts individual loads.
#[cfg(target_arch = "wasm32")]
#[derive(Resource)]
struct ObjectManifestHandle(Handle<WorldObjectManifest>);

/// Accumulates individual object handles as they are loaded from the manifest (WASM only).
/// Each handle is also added to `TrackedAssets` for load-gating.
#[cfg(target_arch = "wasm32")]
#[derive(Resource, Default)]
struct PendingObjectHandles(Vec<Handle<WorldObjectDef>>);
```

#### 3. WorldObjectPlugin
**File**: `crates/protocol/src/world_object/plugin.rs`

```rust
pub struct WorldObjectPlugin;

impl Plugin for WorldObjectPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<WorldObjectDef>();
        app.init_asset_loader::<WorldObjectLoader>();

        #[cfg(target_arch = "wasm32")]
        app.add_plugins(RonAssetPlugin::<WorldObjectManifest>::new(&[
            "objects.manifest.ron",
        ]));

        app.add_systems(Startup, load_world_object_defs);

        #[cfg(target_arch = "wasm32")]
        app.add_systems(
            PreUpdate,
            trigger_individual_object_loads
                .run_if(in_state(crate::app_state::AppState::Loading)),
        );

        app.add_systems(
            Update,
            insert_world_object_defs.run_if(not(resource_exists::<WorldObjectDefRegistry>)),
        );
        app.add_systems(
            Update,
            reload_world_object_defs.run_if(in_state(AppState::Ready)),
        );

        // Register types for RON reflect-based component deserialization.
        app.register_type::<crate::Health>();
        app.register_type::<ObjectCategory>();
        app.register_type::<VisualKind>();
        app.register_type::<ColliderConstructor>();
    }
}
```

#### 4. Load systems

Follow the ability pattern exactly — a platform-split pair for each concern.

**Native** (`cfg(not(target_arch = "wasm32"))`):
**File**: `crates/protocol/src/world_object/loading.rs` (continued)

```rust
fn load_world_object_defs(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<TrackedAssets>,
) {
    let handle = asset_server.load_folder("objects");
    tracked.add(handle.clone_untyped());
    commands.insert_resource(ObjectFolderHandle(handle));
}

fn insert_world_object_defs(
    mut commands: Commands,
    folder_handle: Option<Res<ObjectFolderHandle>>,
    loaded_folders: Res<Assets<LoadedFolder>>,
    object_assets: Res<Assets<WorldObjectDef>>,
    asset_server: Res<AssetServer>,
) {
    let Some(folder_handle) = folder_handle else { return; };
    let Some(folder) = loaded_folders.get(&folder_handle.0) else { return; };

    let mut objects = HashMap::new();
    for handle in folder.handles.iter().filter_map(|h| h.clone().typed::<WorldObjectDef>().ok()) {
        let Some(def) = object_assets.get(&handle) else { continue; }
        let Some(id) = object_id_from_path(&asset_server.get_path(&handle).unwrap()) else { continue; };
        objects.insert(id, def.clone());
    }
    info!("Loaded {} world object definitions", objects.len());
    commands.insert_resource(WorldObjectDefRegistry { objects });
}
```

Note: `insert_world_object_defs` uses `run_if(not(resource_exists::<WorldObjectDefRegistry>))` in the plugin instead of an internal `existing: Option<Res<...>>` guard.

**WASM** (`cfg(target_arch = "wasm32")`): Analogous to the ability WASM pattern — load manifest, on manifest ready load individual files.

**Hot-reload** (both platforms):
**File**: `crates/protocol/src/world_object/loading.rs` (continued)

```rust
fn reload_world_object_defs(
    mut events: EventReader<AssetEvent<WorldObjectDef>>,
    object_assets: Res<Assets<WorldObjectDef>>,
    asset_server: Res<AssetServer>,
    mut registry: ResMut<WorldObjectDefRegistry>,
) {
    let modified = events.read().any(|e| matches!(e, AssetEvent::Modified { .. }));
    if !modified { return; }

    let mut objects = HashMap::new();
    for (handle_id, def) in object_assets.iter() {
        let handle = Handle::Weak(handle_id);
        let Some(path) = asset_server.get_path(&handle) else { continue; };
        let Some(id) = object_id_from_path(&path) else { continue; };
        objects.insert(id, def.clone());
    }
    info!("Hot-reloaded {} world object definitions", objects.len());
    registry.objects = objects;
}
```

**ID extraction** (`str::strip_suffix` is a stable Rust method):
**File**: `crates/protocol/src/world_object/loading.rs` (continued)

```rust
fn object_id_from_path(path: &AssetPath) -> Option<WorldObjectId> {
    let name = path.path().file_name()?.to_str()?;
    Some(WorldObjectId(name.strip_suffix(".object.ron")?.to_string()))
}
```

#### 5. Modify `Health` for Reflect support
**File**: `crates/protocol/src/lib.rs`

`Health` must be `Reflect + Default` to be usable in RON component maps:

```rust
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Default)]
#[reflect(Component, Default)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}
```

#### 6. Add WorldObjectPlugin to SharedGameplayPlugin
**File**: `crates/protocol/src/lib.rs`

```rust
app.add_plugins(WorldObjectPlugin);
```

#### 7. Create initial test assets
**File**: `assets/objects/tree_circle.object.ron`

```ron
{
    "protocol::world_object::types::ObjectCategory": Scenery,
    "protocol::world_object::types::VisualKind": Vox("models/trees/tree_circle.vox"),
    "avian3d::collision::collider::constructor::ColliderConstructor": Cylinder(radius: 0.5, height: 3.0),
    "protocol::Health": (
        current: 50.0,
        max: 50.0,
    ),
    "avian3d::dynamics::rigid_body::RigidBody": Static,
    "avian3d::collision::collider::layers::CollisionLayers": (
        memberships: (16),
        filters: (2),
    ),
    "avian3d::physics_transform::transform::Position": ((5.0, 5.0, 5.0)),
}
```

**File**: `assets/objects.manifest.ron` (manually maintained)

```ron
(["tree_circle"])
```

### Success Criteria:

#### Automated Verification:
- [x] Workspace compiles: `cargo check-all`

#### Manual Verification:
- [ ] `cargo server` — logs "Loaded 1 world object definitions" at startup
- [ ] Editing `tree_circle.object.ron` while server runs — logs "Hot-reloaded 1 world object definitions"

---

## Phase 3: Replication and Spawn

### Overview

Register `WorldObjectId` with Lightyear. Server spawns world objects with `Replicate` (no `PredictionTarget` — static objects). Client detects the single `Replicated` confirmed entity and attaches reflected components plus a placeholder mesh. No duplication.

### Replication Flow

```
Server: spawn(WorldObjectId, Rotation, MapInstanceId, Replicate)
         + apply_object_components(Position, RigidBody, CollisionLayers, ColliderConstructor, etc.)
                │
                │  MapInstanceId observer fires → RoomEvent::AddEntity
                │  (client on same map now sees this entity)
                │
                ▼
Client: receives confirmed entity with Replicated marker
                │
                │  on_world_object_replicated fires (Added<Replicated> + With<WorldObjectId>)
                │
                ▼
Client entity: WorldObjectId, Replicated
             + all reflected components from def (Position, ObjectCategory, VisualKind, etc.)
             + placeholder Mesh3d derived from ColliderConstructor
```

Because there is **no `PredictionTarget`**, Lightyear does not create a predicted or interpolated shadow. Each client has exactly one entity per world object.

### Changes Required:

#### 1. Register WorldObjectId with Lightyear
**File**: `crates/protocol/src/lib.rs` (in `ProtocolPlugin::build`)

World objects are static — no prediction:
```rust
app.register_component::<world_object::WorldObjectId>();
```

#### 2. Server spawn function
**File**: `crates/server/src/world_object.rs` (new file)

The server is the sole spawner of world objects. This function belongs in the server crate because it inserts `Replicate`, which is a Lightyear server-only concept. Note: **no `position` parameter** — `Position` and all other gameplay components come from the definition's reflected components via `apply_object_components`:

```rust
use avian3d::prelude::Rotation;
use bevy::prelude::*;
use lightyear::prelude::*;
use protocol::map::MapInstanceId;
use protocol::world_object::{apply_object_components, WorldObjectDef, WorldObjectId};

/// Spawns a world object entity on the server.
///
/// Lightyear replicates it to all clients on the same map via the room system.
/// `MapInstanceId` triggers `on_map_instance_id_added`, which automatically adds
/// the entity to the correct Lightyear room.
///
/// All gameplay components (Position, RigidBody, CollisionLayers, ColliderConstructor,
/// ObjectCategory, VisualKind, etc.) come from the definition's reflected components.
pub fn spawn_world_object(
    commands: &mut Commands,
    id: WorldObjectId,
    def: &WorldObjectDef,
    map_id: MapInstanceId,
    registry: &AppTypeRegistry,
) -> Entity {
    let entity = commands
        .spawn((
            id,
            Rotation::default(),
            map_id,
            Replicate::to_clients(NetworkTarget::All),
        ))
        .id();

    let components = def
        .components
        .iter()
        .map(|c| {
            c.reflect_clone()
                .expect("world object component must be cloneable")
                .into_partial_reflect()
        })
        .collect();
    apply_object_components(commands, entity, components, registry.0.clone());
    entity
}
```

#### 3. Client-side observer for replicated world objects
**File**: `crates/client/src/world_object.rs` (new file)

The client hydrates replicated world objects with all definition components and a placeholder mesh derived from the collider shape (temporary until vox loading is implemented):

```rust
use avian3d::prelude::ColliderConstructor;
use bevy::prelude::*;
use lightyear::prelude::Replicated;
use protocol::world_object::{apply_object_components, WorldObjectDefRegistry, WorldObjectId};

/// Reacts when Lightyear replicates a world object entity to this client.
///
/// Attaches all reflected gameplay components (including `RigidBody`, `CollisionLayers`,
/// `ColliderConstructor`, `ObjectCategory`, `VisualKind`, etc.) from the definition,
/// then inserts a placeholder mesh derived from the collider shape.
pub fn on_world_object_replicated(
    query: Query<(Entity, &WorldObjectId), Added<Replicated>>,
    registry: Res<WorldObjectDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, id) in &query {
        let Some(def) = registry.get(id) else {
            warn!("Replicated world object has unknown id: {:?}", id.0);
            continue;
        };

        // Extract collider from the components vec for the placeholder mesh.
        let collider = def
            .components
            .iter()
            .find_map(|c| c.try_downcast_ref::<ColliderConstructor>().cloned());

        insert_placeholder_mesh(
            &mut commands.entity(entity),
            collider.as_ref(),
            &mut meshes,
            &mut materials,
        );

        let components = def
            .components
            .iter()
            .map(|c| {
                c.reflect_clone()
                    .expect("world object component must be cloneable")
                    .into_partial_reflect()
            })
            .collect();
        apply_object_components(&mut commands, entity, components, type_registry.0.clone());
    }
}

/// Inserts a `Mesh3d` placeholder derived from the collider shape.
///
/// Once the vox loading pipeline is implemented, this will be replaced by the
/// actual visual from `VisualKind`.
fn insert_placeholder_mesh(
    ecmds: &mut EntityCommands,
    collider: Option<&ColliderConstructor>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let Some(mesh) = collider_to_mesh(collider) else {
        return;
    };
    let mesh_handle = meshes.add(mesh);
    let material_handle = materials.add(StandardMaterial {
        base_color: Color::srgb(0.3, 0.6, 0.2),
        ..default()
    });
    ecmds.insert((Mesh3d(mesh_handle), MeshMaterial3d(material_handle)));
}

/// Converts a `ColliderConstructor` into an approximate `Mesh` for visualization.
fn collider_to_mesh(collider: Option<&ColliderConstructor>) -> Option<Mesh> {
    match collider? {
        ColliderConstructor::Sphere { radius } => Some(Sphere::new(*radius).into()),
        ColliderConstructor::Cuboid {
            x_length,
            y_length,
            z_length,
        } => Some(Cuboid::new(*x_length, *y_length, *z_length).into()),
        ColliderConstructor::Cylinder { radius, height } => {
            Some(Cylinder::new(*radius, *height).into())
        }
        ColliderConstructor::Capsule { radius, height } => {
            Some(Capsule3d::new(*radius, *height).into())
        }
        _ => {
            trace!("No placeholder mesh for collider shape");
            None
        }
    }
}
```

Register in the client plugin after `AppState::Ready`:

```rust
app.add_systems(
    Update,
    on_world_object_replicated.run_if(in_state(AppState::Ready)),
);
```

### Success Criteria:

#### Automated Verification:
- [x] Workspace compiles: `cargo check-all`
- [ ] Existing tests pass: `cargo test --workspace`

#### Manual Verification:
- [ ] Start server, connect client — world object entities replicate; client logs confirm definition lookup

---

## Phase 4: Integration Tests

### Overview
Test the full pipeline: load `.object.ron` → verify `WorldObjectDefRegistry` → query components.

### Changes Required:

#### 1. Create test asset directory and files
**Directory**: `crates/protocol/tests/assets/objects/`

**File**: `crates/protocol/tests/assets/objects/test_tree.object.ron`
```ron
{
    "protocol::world_object::types::ObjectCategory": Scenery,
    "protocol::world_object::types::VisualKind": Vox("models/trees/tree_circle.vox"),
    "protocol::Health": (current: 50.0, max: 50.0),
}
```

**File**: `crates/protocol/tests/assets/objects/bare_rock.object.ron`
```ron
{
    "protocol::world_object::types::ObjectCategory": Scenery,
    "protocol::world_object::types::VisualKind": None,
}
```

**File**: `crates/protocol/tests/assets/objects.manifest.ron`
```ron
(["test_tree", "bare_rock"])
```

#### 2. Dev-dependencies
**File**: `crates/protocol/Cargo.toml`

```toml
[dev-dependencies]
bevy = { workspace = true, features = ["bevy_color", "bevy_state", "bevy_mesh", "bevy_asset"] }
```

#### 3. Integration test file
**File**: `crates/protocol/tests/world_object.rs`

```rust
use bevy::prelude::*;
use protocol::world_object::*;
use protocol::Health;

const MAX_TICKS: usize = 200;

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::asset::AssetPlugin {
        file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/assets").to_string(),
        ..default()
    });
    app.add_plugins(WorldObjectPlugin);
    app.finish();
    app
}

fn tick_until(app: &mut App, condition: impl Fn(&App) -> bool) {
    for _ in 0..MAX_TICKS {
        app.update();
        if condition(app) { return; }
    }
    panic!("condition not met after {MAX_TICKS} ticks");
}

#[test]
fn world_object_defs_loaded() {
    let mut app = test_app();
    tick_until(&mut app, |app| app.world().get_resource::<WorldObjectDefRegistry>().is_some());

    let defs = app.world().resource::<WorldObjectDefRegistry>();
    let id = WorldObjectId("test_tree".to_string());
    let def = defs.get(&id).expect("test_tree should be loaded");
    assert_eq!(def.components.len(), 3); // ObjectCategory, VisualKind, Health
}

#[test]
fn world_object_reflected_components_deserialize() {
    let mut app = test_app();
    tick_until(&mut app, |app| app.world().get_resource::<WorldObjectDefRegistry>().is_some());

    let defs = app.world().resource::<WorldObjectDefRegistry>();
    let id = WorldObjectId("test_tree".to_string());
    let def = defs.get(&id).unwrap();

    // Spawn with apply_object_components, then verify Health landed on the entity.
    let entity = app.world_mut().spawn_empty().id();
    let components: Vec<_> = def.components.iter().map(|c| {
        c.reflect_clone()
            .expect("component must be cloneable")
            .into_partial_reflect()
    }).collect();
    let registry = app.world().resource::<AppTypeRegistry>().0.clone();

    app.world_mut().run_system_once(move |mut commands: Commands| {
        apply_object_components(&mut commands, entity, components.clone(), registry.clone());
    }).unwrap();
    app.update(); // flush command queue

    let health = app.world().entity(entity).get::<Health>().expect("Health component inserted");
    assert_eq!(health.current, 50.0);
    assert_eq!(health.max, 50.0);
}

#[test]
fn world_object_without_extra_components_loads_clean() {
    let mut app = test_app();
    tick_until(&mut app, |app| app.world().get_resource::<WorldObjectDefRegistry>().is_some());

    let defs = app.world().resource::<WorldObjectDefRegistry>();
    let id = WorldObjectId("bare_rock".to_string());
    let def = defs.get(&id).expect("bare_rock should be loaded");
    assert_eq!(def.components.len(), 2); // ObjectCategory, VisualKind only
}
```

### Success Criteria:

#### Automated Verification:
- [ ] All protocol tests pass: `cargo test -p protocol -- world_object`
- [ ] Full workspace: `cargo test --workspace`

#### Manual Verification:
- [ ] `cargo server` logs "Loaded N world object definitions"

---

## Phase 5: Debug UI Button

### Overview
A debug UI button sends `SpawnWorldObjectRequest` to the server, which calls `spawn_world_object` and logs the result. Enables end-to-end manual testing without editing server startup code.

### Changes Required:

#### 1. Define message in protocol
**File**: `crates/protocol/src/world_object/types.rs` (continued)

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
pub struct SpawnWorldObjectRequest {
    pub object_id: String,
    pub position: Vec3,
}
```

**File**: `crates/protocol/src/lib.rs` (in `ProtocolPlugin::build`)

```rust
app.register_message::<world_object::SpawnWorldObjectRequest>()
    .add_direction(NetworkDirection::ClientToServer);
```

Use `MapChannel` (existing ordered reliable channel).

#### 2. Server handler
**File**: `crates/server/src/world_object.rs`

```rust
pub fn handle_spawn_world_object_request(
    mut receivers: Query<(Entity, &mut MessageReceiver<SpawnWorldObjectRequest>)>,
    defs: Res<WorldObjectDefRegistry>,
    registry: Res<AppTypeRegistry>,
    mut commands: Commands,
) {
    for (_client_entity, mut receiver) in &mut receivers {
        for request in receiver.receive() {
            let id = WorldObjectId(request.object_id.clone());
            let Some(def) = defs.get(&id) else {
                warn!("Unknown world object id in request: {}", request.object_id);
                continue;
            };
            spawn_world_object(
                &mut commands,
                id,
                def,
                MapInstanceId::Overworld,
                &registry,
            );
        }
    }
}
```

Register in `ServerGameplayPlugin` under `AppState::Ready`.

#### 3. UI button
**File**: `crates/ui/src/components.rs`

Follow the existing `MapSwitchButton` pattern:
- Spawn a `SpawnTreeButton` marker entity with button UI nodes
- On press, read local player `Position`, send `SpawnWorldObjectRequest { object_id: "tree_circle".into(), position }` via `Query<&mut MessageSender<SpawnWorldObjectRequest>>` + `.send::<MapChannel>(request)`

### Success Criteria:

#### Automated Verification:
- [ ] Workspace compiles: `cargo check-all`

#### Manual Verification:
- [ ] Start server + client, click "Spawn Tree" button
- [ ] Server logs spawn of `tree_circle`
- [ ] World object entity appears on client with placeholder mesh (confirmed via entity inspector or log)
- [ ] Entity has `Health` component with `current: 50, max: 50`

---

## Testing Strategy

### Unit Tests (Phase 1):
- `deserialize_valid_world_object` — valid RON flat map → correct component count
- `deserialize_empty_components` — empty map `{}` works
- `deserialize_unregistered_type_errors` — unregistered type path produces error
- `deserialize_malformed_ron_errors` — invalid RON produces error

### Integration Tests (Phase 4):
- `world_object_defs_loaded` — verifies RON → asset → registry pipeline
- `world_object_reflected_components_deserialize` — verifies reflected components land on entity
- `world_object_without_extra_components_loads_clean` — verifies minimal component set works

### Manual Testing:
1. `cargo server` — verify log shows loaded world object count
2. Edit `.object.ron` while server runs — verify hot-reload log message
3. `cargo server` + `cargo client` — click "Spawn Tree", verify replication, placeholder mesh, and component presence

## Performance Considerations

- `TypeRegistry::read()` takes a read lock per load — fine for asset loading (infrequent)
- `apply_object_components` queues a world-access command — one-frame delay, acceptable
- `WorldObjectDefRegistry` rebuilds entirely on hot-reload (same as `AbilityDefs`) — fine for small object counts

## References

- Research: [doc/research/2026-03-13-world-object-ron-assets.md](../doc/research/2026-03-13-world-object-ron-assets.md)
- Ability system pattern: [crates/protocol/src/ability.rs:404-667](../crates/protocol/src/ability.rs#L404-L667)
- SceneMapDeserializer reference: `bevy_scene::serde::SceneMapDeserializer`
- ReflectComponent::insert: `bevy_ecs::reflect::component`
- TrackedAssets: [crates/protocol/src/app_state.rs](../crates/protocol/src/app_state.rs)
- Room integration observer: [crates/server/src/map.rs:323-339](../crates/server/src/map.rs#L323-L339)
- Message API pattern: [crates/server/src/map.rs:609-661](../crates/server/src/map.rs#L609-L661) (`PlayerMapSwitchRequest` handler)
- Client replication handler pattern: [crates/client/src/gameplay.rs:16-53](../crates/client/src/gameplay.rs#L16-L53)
- Sprite rig spawn observer: [crates/sprite_rig/src/spawn.rs:40-64](../crates/sprite_rig/src/spawn.rs#L40-L64)
