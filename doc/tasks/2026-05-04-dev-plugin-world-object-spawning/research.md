# Research Findings

## Q1: Trace the current dev spawn panel flow from UI interaction to entity creation: which plugins, resources, feature gates, marker components, object registries, and helper functions are involved, and how does the resulting entity differ from ordinary runtime world objects?

**Direct answer:** The dev spawn panel is a feature-gated `DevPlugin` panel that spawns client-local entities at `Transform::default()` with `DevSpawned`; definition-driven spawns reuse `WorldObjectDefRegistry` and `apply_object_components`, but they do not use the server `spawn_world_object` path, `Replicate`, `NetworkVisibility`, `Position`, or `ChunkEntityRef`.

### Evidence

- Spawn panel feature/plugin state is in `crates/dev/src/panels/spawn.rs`; its module docs state the local-only contract.

```rust
// crates/dev/src/panels/spawn.rs:1-7
//! Spawn panel. Two tabs:
//!   * **Def-driven**: pick a registered `WorldObjectId` and spawn via the
//!     existing `apply_object_components` pipeline.
//!   * **Free-form**: pick any reflected `Component` from the `AppTypeRegistry` and
//!     instantiate via `ReflectDefault`.
//! All spawns are client-local (no `Replicate`) at the world origin and carry a
//! `DevSpawned` marker.
```

- The local marker and panel resource are declared in the panel file.

```rust
// crates/dev/src/panels/spawn.rs:17-33
/// Marker for any entity spawned via the dev spawn panel. Client-local; not replicated.
#[derive(Component)]
pub struct DevSpawned;

#[derive(Default, PartialEq, Eq)]
enum SpawnTab {
    #[default]
    DefDriven,
    FreeForm,
}

#[derive(Resource, Default)]
struct SpawnPanelUi {
    tab: SpawnTab,
    selected_object: Option<WorldObjectId>,
    selected_freeform: Vec<String>,
}
```

- `SpawnPanelPlugin` installs the toggle and egui draw system; the run condition requires the dev UI and spawn panel to be enabled.

```rust
// crates/dev/src/panels/spawn.rs:37-50
impl Plugin for SpawnPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnPanelUi>()
            .add_systems(Update, toggle_spawn_panel)
            .add_systems(
                EguiPrimaryContextPass,
                draw_spawn_panel.run_if(spawn_panel_enabled),
            );
    }
}

fn spawn_panel_enabled(state: Res<DevInspectorState>) -> bool {
    state.enabled && state.panels.spawn_panel
}
```

- The panel input resources are `WorldObjectDefRegistry`, `AppTypeRegistry`, and `Commands`; the registry is optional because assets load during startup.

```rust
// crates/dev/src/panels/spawn.rs:58-65
fn draw_spawn_panel(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<SpawnPanelUi>,
    // Optional because definitions load during startup; the panel renders a loading label until ready.
    world_objects: Option<Res<WorldObjectDefRegistry>>,
    type_registry: Res<AppTypeRegistry>,
    mut commands: Commands,
) {
```

- The UI text repeats the spawn behavior.

```rust
// crates/dev/src/panels/spawn.rs:70-77
egui::Window::new("Spawn (client-local)").show(ctx, |ui| {
    ui.horizontal(|ui| {
        ui.selectable_value(&mut ui_state.tab, SpawnTab::DefDriven, "Def-driven");
        ui.selectable_value(&mut ui_state.tab, SpawnTab::FreeForm, "Free-form");
    });
    ui.separator();
    ui.label("Spawned at world origin; client-local (no Replicate).");
```

- Definition-driven spawn enumerates `WorldObjectDefRegistry.objects`, spawns a local ECS entity, clones reflected definition components, and calls the shared helper.

```rust
// crates/dev/src/panels/spawn.rs:111-138
let mut ids: Vec<&WorldObjectId> = reg.objects.keys().collect();
ids.sort_by(|a, b| a.0.cmp(&b.0));
for id in ids {
    ui.selectable_value(&mut ui_state.selected_object, Some(id.clone()), &id.0);
}
...
let entity = commands
    .spawn((
        id.clone(),
        Transform::default(),
        DevSpawned,
        MapInstanceId::Overworld,
        Name::new(format!("dev:{}", id.0)),
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
apply_object_components(commands, entity, components, type_registry.0.clone());
```

- Free-form spawn enumerates every reflected component in `AppTypeRegistry`, requires `ReflectDefault`, and spawns without `WorldObjectId`.

```rust
// crates/dev/src/panels/spawn.rs:153-198
let mut component_paths: Vec<String> = registry
    .iter()
    .filter(|reg| reg.data::<ReflectComponent>().is_some())
    .map(|reg| reg.type_info().type_path().to_string())
    .collect();
...
let Some(default) = reg.data::<ReflectDefault>() else {
    warn!("freeform spawn: type {path} has no ReflectDefault, skipping");
    continue;
};
components.push(default.default().into_partial_reflect());
...
let entity = commands
    .spawn((
        Transform::default(),
        DevSpawned,
        MapInstanceId::Overworld,
        Name::new("dev:freeform"),
    ))
    .id();
apply_object_components(commands, entity, components, type_registry.0.clone());
```

- Ordinary server runtime objects are spawned through `spawn_world_object`, which inserts replication and map visibility state that the dev panel does not insert.

```rust
// crates/server/src/world_object.rs:24-41
pub fn spawn_world_object(
    commands: &mut Commands,
    id: WorldObjectId,
    def: &WorldObjectDef,
    map_id: MapInstanceId,
    type_registry: &AppTypeRegistry,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    meshes: &Assets<Mesh>,
) -> Entity {
    let entity = commands
        .spawn((
            id,
            Rotation::default(),
            map_id,
            Replicate::to_clients(NetworkTarget::All),
            NetworkVisibility,
        ))
        .id();
```

- Generated runtime objects also receive `Position` and `ChunkEntityRef` after the server spawn.

```rust
// crates/server/src/chunk_entities.rs:58-77
let offset = extract_placement_offset(def, is_reload);
let entity = spawn_world_object(
    &mut commands,
    id,
    def,
    map_id.clone(),
    &type_registry,
    &vox_registry,
    &vox_assets,
    &meshes,
);
let position = Vec3::from(spawn.position) + offset;
commands.entity(entity).insert((
    Position(position.into()),
    ChunkEntityRef {
        chunk_pos,
        map_entity,
    },
));
```

## Q2: How are world-object definitions loaded, registered, represented, and applied at runtime — including `WorldObjectId`, reflected components, `PlacementOffset`, spawn-only versus persistent component markers, and the contract of `apply_object_components`?

**Direct answer:** World-object definitions are `.object.ron` assets deserialized into `WorldObjectDef { components: Vec<Box<dyn PartialReflect>> }`, indexed by filename-derived `WorldObjectId`, and applied by `apply_object_components`, which inserts each reflected component through `ReflectComponent`; `PlacementOffset` is marked `SpawnOnly`, `ActiveTransformation` is marked `Persist`, and current persistence code explicitly serializes `ActiveTransformation` and `Health`.

### Evidence

- `WorldObjectId` is both an asset key and replicated component.

```rust
// crates/protocol/src/world_object/types.rs:6-11
/// Unique identifier for a world object definition. Derived from the `.object.ron` filename.
///
/// Also used as a replicated ECS component — the single component Lightyear sends to clients
/// to identify which definition to look up in `WorldObjectDefRegistry`.
#[derive(Component, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct WorldObjectId(pub String);
```

- `PlacementOffset` is a component with custom reflect type data `SpawnOnly`.

```rust
// crates/protocol/src/world_object/types.rs:13-19
/// Offset applied to the placement position when spawning a world object.
///
/// Vox models are often centered at their geometric midpoint, so this shifts the
/// spawn position (e.g. `(0, 1.5, 0)` raises the object so its base sits on the surface).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
#[reflect(Component, Serialize, Deserialize, SpawnOnly)]
pub struct PlacementOffset(pub Vec3);
```

- `ActiveTransformation` is a reflected component marked `Persist`.

```rust
// crates/protocol/src/world_object/types.rs:64-70
/// Tracks an active transformation on a world object. Persisted across chunk eviction.
#[derive(Component, Reflect, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[reflect(Component, Persist)]
pub struct ActiveTransformation {
    pub source: String,
    pub ticks_remaining: Option<u16>,
}
```

- The marker type data definitions are present but are not themselves serialization logic.

```rust
// crates/protocol/src/world_object/types.rs:72-90
/// Reflect type data: marks a component for serialization during chunk eviction.
#[derive(Clone)]
pub struct ReflectPersist;

impl<T: Reflect> bevy::reflect::FromType<T> for ReflectPersist {
    fn from_type() -> Self {
        ReflectPersist
    }
}

/// Reflect type data: marks a component as spawn-only (skipped on reload).
#[derive(Clone)]
pub struct ReflectSpawnOnly;

impl<T: Reflect> bevy::reflect::FromType<T> for ReflectSpawnOnly {
    fn from_type() -> Self {
        ReflectSpawnOnly
    }
}
```

- `WorldObjectDef` stores only reflected components.

```rust
// crates/protocol/src/world_object/types.rs:92-100
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
```

- `apply_object_components` queues reflected insertion and delegates each value to `insert_reflected_component`.

```rust
// crates/protocol/src/world_object/spawn.rs:4-20
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
```

- Insertion requires both a type-path registration and `ReflectComponent` type data; missing registrations warn and skip the component.

```rust
// crates/protocol/src/world_object/spawn.rs:23-38
fn insert_reflected_component(
    entity_mut: &mut EntityWorldMut,
    component: &dyn PartialReflect,
    registry: &bevy::reflect::TypeRegistry,
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

- Server reloads skip `PlacementOffset`; fresh spawns apply it.

```rust
// crates/server/src/chunk_entities.rs:58-70
let is_reload = !spawn.persisted_components.is_empty();
let offset = extract_placement_offset(def, is_reload);
let entity = spawn_world_object(
    &mut commands,
    id,
    def,
    map_id.clone(),
    &type_registry,
    &vox_registry,
    &vox_assets,
    &meshes,
);
let position = Vec3::from(spawn.position) + offset;
```

### Runtime application sites observed

| Site | Evidence |
|---|---|
| Dev def-driven spawn | `crates/dev/src/panels/spawn.rs:117-138` clones def components and calls `apply_object_components`. |
| Dev free-form spawn | `crates/dev/src/panels/spawn.rs:176-198` builds default reflected components and calls `apply_object_components`. |
| Server spawn | `crates/server/src/world_object.rs:44-52` clones def components, applies them, then optionally inserts a vox collider. |
| Client hydration | `crates/client/src/world_object.rs:53-76` looks up the def, applies components, inserts collider/transform, and attaches visuals. |
| Transformation | `crates/server/src/world_object.rs:62-99` removes absent components, applies source-def components, and swaps collider. |

## Q3: Trace the generated world-object lifecycle from terrain placement rules through `WorldObjectSpawn`, pending chunk entity queues, `spawn_world_object`, replication setup, client hydration, and eventual despawn or transformation.

**Direct answer:** Generated world objects originate from voxel generator feature placement as `WorldObjectSpawn`, are queued in `PendingEntitySpawns`, materialized by the server with `spawn_world_object`, receive `Position`/`ChunkEntityRef`, are replicated via Lightyear room visibility, hydrated on clients from `WorldObjectId`, and are saved/despawned on chunk eviction or modified by transformation/death-effect systems.

### Evidence

- The generator trait has a feature-placement stage returning `Vec<WorldObjectSpawn>`.

```rust
// crates/voxel_map_engine/src/config.rs:13-25
pub trait VoxelGeneratorImpl: Send + Sync {
    /// Stage 1: Base terrain shape. Returns a padded voxel array sized `padded_size³`.
    fn generate_terrain(&self, chunk_pos: IVec3) -> Vec<WorldVoxel>;

    /// Stage 2: Entity placement on terrain surface.
    /// Receives a padded surface height map (not raw voxels). Default: no features.
    fn place_features(
        &self,
        _chunk_pos: IVec3,
        _heights: &SurfaceHeightMap,
    ) -> Vec<WorldObjectSpawn> {
        Vec::new()
    }
}
```

- `WorldObjectSpawn` is the cross-crate boundary type: string id, position, and serialized component snapshots.

```rust
// crates/voxel_map_engine/src/config.rs:28-47
/// Spawn data for a world object placed during the Features stage.
///
/// Uses bare `String` for `object_id` (not `WorldObjectId`) because `WorldObjectId`
/// lives in the `protocol` crate, and `voxel_map_engine` must not depend on it.
/// The server spawn system converts to `WorldObjectId` at the boundary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldObjectSpawn {
    pub object_id: String,
    pub position: Vec3,
    /// RON-serialized persisted components. Empty for fresh spawns.
    #[serde(default)]
    pub persisted_components: Vec<PersistedComponent>,
}

/// A single persisted component: type path + RON data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedComponent {
    pub type_path: String,
    pub ron_data: String,
}
```

- `spawn_chunk_entities` drains queued spawns, optionally saves them, resolves definitions, spawns the server object, inserts position and chunk membership, and restores persisted components on reload.

```rust
// crates/server/src/chunk_entities.rs:39-91
for (map_entity, map_id, mut pending, store, mut ops) in &mut map_query {
    for (chunk_pos, spawns) in pending.0.drain(..) {
        if spawns.is_empty() {
            continue;
        }

        if let (Some(store), Some(ref mut ops)) = (&store, &mut ops) {
            ops.spawn_save(&store.0, chunk_pos, spawns.clone());
        }

        for spawn in &spawns {
            let id = WorldObjectId(spawn.object_id.clone());
            let Some(def) = defs.get(&id) else {
                warn!(
                    "Unknown world object '{}' in placement rules",
                    spawn.object_id
                );
                continue;
            };
            let is_reload = !spawn.persisted_components.is_empty();
            let offset = extract_placement_offset(def, is_reload);
            let entity = spawn_world_object(...);
            let position = Vec3::from(spawn.position) + offset;
            commands.entity(entity).insert((
                Position(position.into()),
                ChunkEntityRef {
                    chunk_pos,
                    map_entity,
                },
            ));

            if is_reload {
                restore_persisted(...);
            }
        }
    }
}
```

- Server object spawn includes replication and network visibility.

```rust
// crates/server/src/world_object.rs:34-52
let entity = commands
    .spawn((
        id,
        Rotation::default(),
        map_id,
        Replicate::to_clients(NetworkTarget::All),
        NetworkVisibility,
    ))
    .id();

let vox_collider = vox_trimesh_collider(def, vox_registry, vox_assets, meshes);
let use_vox_collider = vox_collider.is_some();

let components = clone_def_components(def, use_vox_collider);
apply_object_components(commands, entity, components, type_registry.0.clone());

if let Some(collider) = vox_collider {
    commands.entity(entity).insert(collider);
}
```

- `MapInstanceId` addition moves replicated entities into Lightyear rooms.

```rust
// crates/server/src/map.rs:599-615
fn on_map_instance_id_added(
    trigger: On<Add, MapInstanceId>,
    mut commands: Commands,
    map_ids: Query<&MapInstanceId>,
    mut room_registry: ResMut<RoomRegistry>,
) {
    let entity = trigger.entity;
    let map_id = map_ids
        .get(entity)
        .expect("Entity with MapInstanceId trigger must have MapInstanceId");
    let room = room_registry.get_or_create(map_id, &mut commands);
    commands.entity(entity).try_insert(NetworkVisibility);
    commands.trigger(RoomEvent {
        room,
        target: RoomTarget::AddEntity(entity),
    });
}
```

- Client hydration is keyed by replicated `WorldObjectId` on `Added<Replicated>`.

```rust
// crates/client/src/world_object.rs:32-43
pub fn on_world_object_replicated(
    query: Query<(Entity, &WorldObjectId, Option<&Position>, Option<&Rotation>), Added<Replicated>>,
    registry: Res<WorldObjectDefRegistry>,
    map_registry: Res<MapRegistry>,
    map_id_query: Query<&MapInstanceId>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
    type_registry: Res<AppTypeRegistry>,
    default_material: Res<DefaultVoxModelMaterial>,
    mut commands: Commands,
) {
```

- Hydration applies definition components, derives local collider/transform, and attaches visuals.

```rust
// crates/client/src/world_object.rs:53-76
let Some(def) = registry.get(id) else {
    warn!("Replicated world object has unknown id: {:?}", id.0);
    continue;
};

let vox_collider = vox_trimesh_collider(def, &vox_registry, &vox_assets, &meshes);
let has_vox_collider = vox_collider.is_some();

let components = clone_def_components(def, has_vox_collider);
apply_object_components(&mut commands, entity, components, type_registry.0.clone());

if let Some(collider) = vox_collider {
    commands.entity(entity).insert(collider);
}

let transform = transform_from_physics(pos, rot);
commands.entity(entity).insert(transform);

attach_visual(...);
```

- Eviction saves world objects whose chunk column is unloaded, then despawns them.

```rust
// crates/server/src/chunk_entities.rs:119-151
for (entity, chunk_ref, obj_id, pos, active_transform, health) in &entity_query {
    let Ok(instance) = map_query.get(chunk_ref.map_entity) else {
        continue;
    };
    let col = chunk_to_column(chunk_ref.chunk_pos);
    if instance.chunk_levels.contains_key(&col) {
        continue;
    }

    let persisted = serialize_persisted(active_transform, health);

    by_chunk
        .entry((chunk_ref.map_entity, chunk_ref.chunk_pos))
        .or_default()
        .push((
            entity,
            WorldObjectSpawn {
                object_id: obj_id.0.clone(),
                position: Vec3::from(pos.0),
                persisted_components: persisted,
            },
        ));
}
...
for (entity, _) in entities {
    commands.entity(entity).despawn();
}
```

## Q4: How does the protocol layer register world-object and map-related components/messages for networking, and what existing client-to-server request/ack/reject patterns are used for authoritative world mutations such as voxel edits?

**Direct answer:** The protocol layer registers world objects as replicated components (`WorldObjectId`, `VisualKind`, `ActiveTransformation`), not as request messages; existing authoritative client-to-server world mutation uses voxel edit messages: request from client, ack/reject from server, plus server-to-client broadcasts/section updates.

### Evidence

- Voxel channels/messages are bidirectional for requests and server responses.

```rust
// crates/protocol/src/lib.rs:107-124
// Voxel channel
app.add_channel::<VoxelChannel>(ChannelSettings {
    mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
    ..default()
})
.add_direction(NetworkDirection::Bidirectional);

// Voxel messages
app.register_message::<VoxelEditRequest>()
    .add_direction(NetworkDirection::ClientToServer);
app.register_message::<VoxelEditBroadcast>()
    .add_direction(NetworkDirection::ServerToClient);
app.register_message::<VoxelEditAck>()
    .add_direction(NetworkDirection::ServerToClient);
app.register_message::<VoxelEditReject>()
    .add_direction(NetworkDirection::ServerToClient);
app.register_message::<SectionBlocksUpdate>()
    .add_direction(NetworkDirection::ServerToClient);
```

- Chunk streaming is server-to-client; map transition includes client-to-server request/readiness and server-to-client transition messages.

```rust
// crates/protocol/src/lib.rs:126-154
// Chunk streaming channel
app.add_channel::<ChunkChannel>(ChannelSettings {
    mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
    ..default()
})
.add_direction(NetworkDirection::ServerToClient);

// Chunk streaming messages
app.register_message::<ChunkDataSync>()
    .add_direction(NetworkDirection::ServerToClient);
app.register_message::<UnloadColumn>()
    .add_direction(NetworkDirection::ServerToClient);
...
app.register_message::<PlayerMapSwitchRequest>()
    .add_direction(NetworkDirection::ClientToServer);
app.register_message::<MapTransitionStart>()
    .add_direction(NetworkDirection::ServerToClient);
app.register_message::<MapTransitionReady>()
    .add_direction(NetworkDirection::ClientToServer);
app.register_message::<MapTransitionEnd>()
    .add_direction(NetworkDirection::ServerToClient);
```

- World-object networking registration is component-only in the observed protocol registration.

```rust
// crates/protocol/src/lib.rs:175-182
// Map instance identity
app.register_component::<MapInstanceId>().add_prediction();
app.register_component::<Owner>();

// World objects — static, no prediction needed
app.register_component::<world_object::WorldObjectId>();
app.register_component::<world_object::VisualKind>();
app.register_component::<world_object::ActiveTransformation>();
```

- Client voxel input sends `VoxelEditRequest { position, voxel, sequence }` after local prediction.

```rust
// crates/client/src/map.rs:274-294
let sequence = prediction_state.next();
let old_voxel = voxel_world
    .get_voxel(chunk_ticket.map_entity, position)
    .into();

voxel_world.set_voxel(chunk_ticket.map_entity, position, WorldVoxel::from(voxel));

prediction_state.pending.push(VoxelPrediction {
    sequence,
    position,
    old_voxel,
    new_voxel: voxel,
});

for mut sender in message_sender.iter_mut() {
    trace!("Sending voxel edit request to server: {:?}", position);
    sender.send::<VoxelChannel>(VoxelEditRequest {
        position,
        voxel,
        sequence,
    });
}
```

- Server validation sends reject with authoritative voxel or applies/acks/broadcasts.

```rust
// crates/server/src/map.rs:681-700
fn is_edit_valid(
    request: &VoxelEditRequest,
    map_entity: Entity,
    client_entity: Entity,
    voxel_world: &VoxelWorld,
    reject_senders: &mut Query<&mut MessageSender<VoxelEditReject>>,
) -> bool {
    if validate_voxel_edit(request, map_entity, voxel_world) {
        return true;
    }
    let current_voxel = voxel_world.get_voxel(map_entity, request.position);
    if let Ok(mut sender) = reject_senders.get_mut(client_entity) {
        sender.send::<VoxelChannel>(VoxelEditReject {
            sequence: request.sequence,
            position: request.position,
            correct_voxel: current_voxel.into(),
        });
    }
    false
}
```

```rust
// crates/server/src/map.rs:777-784
apply_voxel_edit(
    &request,
    map_entity,
    &mut voxel_world,
    &mut *dirty_state,
    &*time,
);
send_edit_ack(client_entity, request.sequence, &mut ack_senders);
```

- No world-object request/ack/reject registration was observed in `crates/protocol/src/lib.rs:107-182`; the adjacent authoritative mutation pattern is voxel edits.

## Q5: How does the server determine map scope, room visibility, and authority for map mutations and replicated entities — including `MapInstanceId`, `RoomRegistry`, chunk visibility, transitions, and validation of client-originated requests?

**Direct answer:** The server uses replicated semantic `MapInstanceId` to identify map scope, `MapRegistry` to resolve local map entities, and `RoomRegistry` to map each `MapInstanceId` to a Lightyear room; client voxel requests carry no map id, so the server derives authority from the client-owned `CharacterMarker` with `ControlledBy` and that character's `MapInstanceId`.

### Evidence

- `RoomRegistry` maps semantic map ids to Lightyear room entities.

```rust
// crates/server/src/map.rs:37-53
/// Maps `MapInstanceId` to lightyear room entities. Server-only.
#[derive(Resource, Default)]
pub struct RoomRegistry(pub HashMap<MapInstanceId, Entity>);

impl RoomRegistry {
    pub fn get_or_create(&mut self, id: &MapInstanceId, commands: &mut Commands) -> Entity {
        *self.0.entry(id.clone()).or_insert_with(|| {
            let room = commands.spawn(Room::default()).id();
            trace!("Created room for map {id:?}: {room:?}");
            room
        })
    }
}
```

- The server plugin initializes map/room state, runs voxel request handling, chunk streaming, map transitions, chunk entity spawning/eviction, and the `MapInstanceId` observer.

```rust
// crates/server/src/map.rs:624-665
impl Plugin for ServerMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(lightyear::prelude::RoomPlugin)
            .add_plugins(VoxelPlugin)
            .insert_resource(voxel_map_engine::ChunkGenerationEnabled)
            .init_resource::<MapRegistry>()
            .init_resource::<RoomRegistry>()
            .init_resource::<WorldDirtyState>()
            .init_resource::<PendingVoxelBroadcasts>()
            .init_resource::<WorldSavePath>()
            .add_systems(OnEnter(AppState::Ready), init_overworld_entity)
            .add_systems(
                Update,
                (
                    poll_map_meta.run_if(in_state(AppState::Ready)),
                    poll_map_entities.run_if(in_state(AppState::Ready)),
                    (handle_voxel_edit_requests, flush_voxel_broadcasts).chain(),
                    push_chunks_to_clients,
                    save_dirty_chunks_debounced,
                    handle_map_switch_requests.run_if(resource_exists::<TerrainDefRegistry>),
                    crate::transition::complete_map_transition,
                    protocol::attach_chunk_colliders,
                    crate::chunk_entities::spawn_chunk_entities
                        .after(lifecycle::poll_chunk_tasks)
                        .run_if(
                            resource_exists::<WorldObjectDefRegistry>
                                .and(resource_exists::<VoxModelRegistry>),
                        ),
                    crate::chunk_entities::evict_chunk_entities
                        .after(lifecycle::despawn_out_of_range_chunks),
                    poll_chunk_entity_ops,
                    crate::chunk_entities::save_chunk_entities_periodic,
                ),
            )
            .add_systems(
                Last,
                (
                    save_world_on_shutdown,
                    crate::chunk_entities::save_all_chunk_entities_on_exit,
                ),
            )
            .add_observer(on_map_instance_id_added);
    }
}
```

- The server resolves a client's mutable map from its controlled character, not from request-provided map data.

```rust
// crates/server/src/map.rs:669-679
/// Resolves which map entity a client's character is on.
fn resolve_player_map(
    client_entity: Entity,
    controlled_query: &Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: &MapRegistry,
) -> Option<(Entity, MapInstanceId)> {
    let (_, player_map_id) = controlled_query
        .iter()
        .find(|(ctrl, _)| ctrl.owner == client_entity)?;
    Some((map_registry.get(player_map_id), player_map_id.clone()))
}
```

- `handle_voxel_edit_requests` performs the full authority flow: receive per client, resolve map, validate, apply, ack, and queue a map-scoped broadcast.

```rust
// crates/server/src/map.rs:747-785
pub fn handle_voxel_edit_requests(
    mut receivers: Query<(Entity, &mut MessageReceiver<VoxelEditRequest>)>,
    mut ack_senders: Query<&mut MessageSender<VoxelEditAck>>,
    mut reject_senders: Query<&mut MessageSender<VoxelEditReject>>,
    mut pending_broadcasts: ResMut<PendingVoxelBroadcasts>,
    mut dirty_state: ResMut<WorldDirtyState>,
    time: Res<Time>,
    mut voxel_world: VoxelWorld,
    controlled_query: Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: Res<MapRegistry>,
) {
    for (client_entity, mut receiver) in &mut receivers {
        for request in receiver.receive() {
            let Some((map_entity, player_map_id)) =
                resolve_player_map(client_entity, &controlled_query, &*map_registry)
            else {
                trace!("handle_voxel_edit_requests: no character for client {client_entity:?}");
                continue;
            };

            if !is_edit_valid(... ) {
                continue;
            }

            apply_voxel_edit(...);
            send_edit_ack(client_entity, request.sequence, &mut ack_senders);
```

- Chunk visibility is map-entity scoped through `ChunkTicket`; the client sync state is reset when the ticket switches maps.

```rust
// crates/server/src/map.rs:850-860
#[derive(Component, Default)]
pub struct ClientChunkVisibility {
    /// Individual chunks (IVec3) whose data has been sent.
    sent_chunks: HashSet<IVec3>,
    /// Columns the client believes are loaded (for sending UnloadColumn).
    sent_columns: HashSet<IVec2>,
    /// The map entity these tracking sets are scoped to. Reset when the
    /// player's ticket switches maps (e.g. map transition).
    tracked_map: Option<Entity>,
}
```

- World-object scope comes from the map query's `MapInstanceId` and `ChunkEntityRef`.

```rust
// crates/server/src/chunk_entities.rs:39-77
for (map_entity, map_id, mut pending, store, mut ops) in &mut map_query {
    for (chunk_pos, spawns) in pending.0.drain(..) {
        ...
        let entity = spawn_world_object(
            &mut commands,
            id,
            def,
            map_id.clone(),
            &type_registry,
            &vox_registry,
            &vox_assets,
            &meshes,
        );
        let position = Vec3::from(spawn.position) + offset;
        commands.entity(entity).insert((
            Position(position.into()),
            ChunkEntityRef {
                chunk_pos,
                map_entity,
            },
        ));
```

## Q6: How does the client convert mouse or cursor state into world-space targets today, including camera ray construction, voxel raycasts, input actions, prediction state, rollback/rejection handling, and any existing pointer/gizmo rendering conventions?

**Direct answer:** Current cursor-to-world targeting exists for voxel edits: mouse button actions trigger `handle_voxel_input`, which creates a `Ray3d` from primary-window cursor and `Camera3d`, raycasts the current `ChunkTicket.map_entity`, locally predicts the voxel edit, and sends `VoxelEditRequest`; no world-object placement preview entity was observed in the inspected client/dev files.

### Evidence

- Voxel input requires controlled predicted player, action state, camera/window, message sender, and prediction state.

```rust
// crates/client/src/map.rs:230-237
player_query: Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
mut voxel_world: VoxelWorld,
camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
window_query: Query<&Window, With<PrimaryWindow>>,
action_query: Query<&ActionState<PlayerActions>, With<Controlled>>,
mut message_sender: Query<&mut MessageSender<VoxelEditRequest>>,
mut prediction_state: ResMut<VoxelPredictionState>,
```

- The function explicitly traces expected early-outs for missing player/action state; left/right actions trigger place/remove.

```rust
// crates/client/src/map.rs:238-255
let Ok(chunk_ticket) = player_query.single() else {
    trace!("handle_voxel_input: no predicted player with ChunkTicket");
    return;
};
let Ok(action_state) = action_query.single() else {
    trace!("handle_voxel_input: no entity with ActionState + Controlled");
    return;
};

let removing = action_state.just_pressed(&PlayerActions::RemoveVoxel);
let placing = action_state.just_pressed(&PlayerActions::PlaceVoxel);
if !removing && !placing {
    return;
}

let Some(ray) = camera_ray(&camera_query, &window_query) else {
    warn!("handle_voxel_input: no camera ray (no cursor position?)");
    return;
};
```

- Raycast uses the current map entity and solid-voxel filter; place targets adjacent voxel along the hit normal.

```rust
// crates/client/src/map.rs:258-272
let Some(hit) = voxel_world.raycast(chunk_ticket.map_entity, ray, RAYCAST_MAX_DISTANCE, |v| {
    matches!(v, WorldVoxel::Solid(_))
}) else {
    trace!("handle_voxel_input: raycast hit nothing");
    return;
};

let (position, voxel) = if removing {
    (hit.position, VoxelType::Air)
} else if let Some(normal) = hit.normal {
    (hit.position + normal.as_ivec3(), VoxelType::Solid(0))
} else {
    trace!("handle_voxel_input: place hit has no normal");
    return;
};
```

- Camera ray construction uses primary-window cursor, optional logical viewport origin adjustment, and Bevy `viewport_to_world`.

```rust
// crates/client/src/map.rs:298-314
fn camera_ray(
    camera_query: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: &Query<&Window, With<PrimaryWindow>>,
) -> Option<Ray3d> {
    let (camera, camera_transform) = camera_query.single().ok()?;
    let window = window_query.single().ok()?;
    let cursor_pos = window.cursor_position()?;
    let viewport_pos = if let Some(rect) = camera.logical_viewport_rect() {
        cursor_pos - rect.min
    } else {
        cursor_pos
    };

    camera
        .viewport_to_world(camera_transform, viewport_pos)
        .ok()
}
```

- Prediction and request send happen immediately after target selection.

```rust
// crates/client/src/map.rs:274-294
let sequence = prediction_state.next();
let old_voxel = voxel_world
    .get_voxel(chunk_ticket.map_entity, position)
    .into();

voxel_world.set_voxel(chunk_ticket.map_entity, position, WorldVoxel::from(voxel));

prediction_state.pending.push(VoxelPrediction {
    sequence,
    position,
    old_voxel,
    new_voxel: voxel,
});

for mut sender in message_sender.iter_mut() {
    trace!("Sending voxel edit request to server: {:?}", position);
    sender.send::<VoxelChannel>(VoxelEditRequest {
        position,
        voxel,
        sequence,
    });
}
```

- Ack removes confirmed predictions; reject writes authoritative voxel and removes the rejected pending edit.

```rust
// crates/client/src/map.rs:316-330
/// Processes server acknowledgments, clearing confirmed predictions.
fn handle_voxel_edit_ack(
    mut receivers: Query<&mut MessageReceiver<VoxelEditAck>>,
    mut prediction_state: ResMut<VoxelPredictionState>,
) {
    for mut receiver in &mut receivers {
        for ack in receiver.receive() {
            trace!(
                "handle_voxel_edit_ack: ack seq={}, clearing {} pending",
                ack.sequence,
                prediction_state.pending.len()
            );
            prediction_state
                .pending
                .retain(|p| p.sequence > ack.sequence);
```

```rust
// crates/client/src/map.rs:335-360
/// Processes server rejections, rolling back the predicted voxel to the correct value.
fn handle_voxel_edit_reject(
    mut receivers: Query<&mut MessageReceiver<VoxelEditReject>>,
    mut prediction_state: ResMut<VoxelPredictionState>,
    mut voxel_world: VoxelWorld,
    player_query: Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
) {
    let Ok(chunk_ticket) = player_query.single() else {
        trace!("handle_voxel_edit_reject: no predicted player");
        return;
    };

    for mut receiver in &mut receivers {
        for reject in receiver.receive() {
            warn!(
                "handle_voxel_edit_reject: rejected seq={} at {:?}, correct={:?}",
                reject.sequence, reject.position, reject.correct_voxel
            );
            voxel_world.set_voxel(
                chunk_ticket.map_entity,
                reject.position,
                WorldVoxel::from(reject.correct_voxel),
            );
            prediction_state
                .pending
                .retain(|p| p.sequence != reject.sequence);
```

- Adjacent gizmo convention: dev physics gizmos exist and are toggled elsewhere, but no placement-preview entity was observed in the inspected spawn panel/client map/world-object paths.

## Q7: How are replicated world objects hydrated on the client, and which components or child entities are local-only visuals/colliders versus authoritative replicated state?

**Direct answer:** Replicated world objects arrive with protocol-registered authoritative state such as `WorldObjectId`, `MapInstanceId`, `Position`/`Rotation`, `VisualKind`, `ActiveTransformation`, and other registered gameplay components; the client locally applies reflected def components, builds vox trimesh colliders, inserts `Transform`, and attaches mesh/material child visuals and health-bar children.

### Evidence

- Protocol-registered world-object and map/physics state includes `MapInstanceId`, `WorldObjectId`, `VisualKind`, and `ActiveTransformation`.

```rust
// crates/protocol/src/lib.rs:175-182
// Map instance identity
app.register_component::<MapInstanceId>().add_prediction();
app.register_component::<Owner>();

// World objects — static, no prediction needed
app.register_component::<world_object::WorldObjectId>();
app.register_component::<world_object::VisualKind>();
app.register_component::<world_object::ActiveTransformation>();
```

- Server spawn supplies the authoritative replicated entity shell.

```rust
// crates/server/src/world_object.rs:34-41
let entity = commands
    .spawn((
        id,
        Rotation::default(),
        map_id,
        Replicate::to_clients(NetworkTarget::All),
        NetworkVisibility,
    ))
    .id();
```

- Client hydration skips objects whose replicated `MapInstanceId` is not registered locally.

```rust
// crates/client/src/world_object.rs:44-51
for (entity, id, pos, rot) in &query {
    if let Ok(entity_mid) = map_id_query.get(entity) {
        if !map_registry.0.contains_key(entity_mid) {
            trace!("Despawning stale world object {entity:?} from map {entity_mid:?}");
            commands.entity(entity).despawn();
            continue;
        }
    }
```

- Client collider and definition components are local hydration work.

```rust
// crates/client/src/world_object.rs:58-66
let vox_collider = vox_trimesh_collider(def, &vox_registry, &vox_assets, &meshes);
let has_vox_collider = vox_collider.is_some();

let components = clone_def_components(def, has_vox_collider);
apply_object_components(&mut commands, entity, components, type_registry.0.clone());

if let Some(collider) = vox_collider {
    commands.entity(entity).insert(collider);
}
```

- The client inserts `Transform` from replicated physics state because child visuals need transform hierarchy support.

```rust
// crates/client/src/world_object.rs:68-73
// Insert Transform matching Position so children (Mesh3d) have a parent
// with GlobalTransform. PhysicsTransformPlugin is disabled, so Position
// does not auto-require Transform; lightyear's add_transform only runs
// in PostUpdate, after children are already attached.
let transform = transform_from_physics(pos, rot);
commands.entity(entity).insert(transform);
```

- Vox visual children are local `Mesh3d`/`MeshMaterial3d` children.

```rust
// crates/client/src/world_object.rs:126-152
/// Attaches the vox mesh as a child entity if `VisualKind::Vox` is present.
fn attach_visual(
    commands: &mut Commands,
    entity: Entity,
    def: &WorldObjectDef,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    default_material: &DefaultVoxModelMaterial,
) {
    let visual_kind = def
        .components
        .iter()
        .find_map(|c| c.try_downcast_ref::<VisualKind>());

    match visual_kind {
        Some(VisualKind::Vox(path)) => {
            attach_vox_mesh(...);
        }
        _ => {
            trace!("World object entity {entity:?} has no Vox visual, skipping mesh attachment");
        }
    }
}
```

- When replicated `VisualKind` changes, the client deletes old visual children and rebuilds collider/visuals locally.

```rust
// crates/client/src/world_object.rs:157-181
/// Rebuilds visuals and collider when VisualKind changes via replication (e.g. tree→stump).
pub fn on_visual_kind_changed(
    mut commands: Commands,
    query: Query<(Entity, &VisualKind), Changed<VisualKind>>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
    default_material: Res<DefaultVoxModelMaterial>,
    children_query: Query<&Children>,
) {
    for (entity, visual) in &query {
        // Despawn old visual children
        if let Ok(children) = children_query.get(entity) {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }

        // Remove old collider and rebuild from new visual
        commands.entity(entity).remove::<Collider>();
        if let VisualKind::Vox(path) = visual {
            if let Some(collider) =
                vox_trimesh_collider_from_path(path, &vox_registry, &vox_assets, &meshes)
            {
                commands.entity(entity).insert(collider);
            }
```

## Q8: How are world objects persisted and restored across chunk eviction, periodic saves, shutdown saves, and map reloads, and what distinguishes per-chunk entity files from map-level entity persistence?

**Direct answer:** Chunk-generated world objects persist through per-chunk entity files storing `Vec<WorldObjectSpawn>`; eviction, periodic save, shutdown save, and reload all operate on `ChunkEntityRef`/`WorldObjectSpawn`; map-level entity persistence is separate and currently stores `SavedEntity` records such as respawn points, not chunk world objects.

### Evidence

- Per-chunk persistence uses the same `WorldObjectSpawn` shape shown in Q3.

```rust
// crates/voxel_map_engine/src/config.rs:33-47
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldObjectSpawn {
    pub object_id: String,
    pub position: Vec3,
    /// RON-serialized persisted components. Empty for fresh spawns.
    #[serde(default)]
    pub persisted_components: Vec<PersistedComponent>,
}

/// A single persisted component: type path + RON data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedComponent {
    pub type_path: String,
    pub ron_data: String,
}
```

- Spawn materialization saves generated spawn lists immediately when a chunk entity store exists.

```rust
// crates/server/src/chunk_entities.rs:39-47
for (map_entity, map_id, mut pending, store, mut ops) in &mut map_query {
    for (chunk_pos, spawns) in pending.0.drain(..) {
        if spawns.is_empty() {
            continue;
        }

        if let (Some(store), Some(ref mut ops)) = (&store, &mut ops) {
            ops.spawn_save(&store.0, chunk_pos, spawns.clone());
        }
```

- Eviction serializes persisted components and writes the entity back as `WorldObjectSpawn`, then despawns.

```rust
// crates/server/src/chunk_entities.rs:128-151
let persisted = serialize_persisted(active_transform, health);

by_chunk
    .entry((chunk_ref.map_entity, chunk_ref.chunk_pos))
    .or_default()
    .push((
        entity,
        WorldObjectSpawn {
            object_id: obj_id.0.clone(),
            position: Vec3::from(pos.0),
            persisted_components: persisted,
        },
    ));
...
if let Ok((store, mut ops)) = store_query.get_mut(map_entity) {
    ops.spawn_save(&store.0, chunk_pos, spawns);
}

for (entity, _) in entities {
    commands.entity(entity).despawn();
}
```

- The currently serialized persisted components are limited in the observed chunk-entity code to `ActiveTransformation` and `protocol::Health` (per subagent trace of `crates/server/src/chunk_entities.rs:257-339`); restore applies transformations before reinserting persisted component state.

### Persistence layer distinction

| Layer | Key/value | File shape observed by code | Contents |
|---|---|---|---|
| Chunk terrain | `Store<IVec3, ChunkFileEnvelope>` | `map_dir/terrain/chunk_x_y_z.bin` | Voxel chunk data and metadata. |
| Per-chunk world objects | `Store<IVec3, Vec<WorldObjectSpawn>>` | `map_dir/entities/chunk_x_y_z.entities.bin` | World-object id, position, persisted component snapshots. |
| Map-level entities | `Store<(), Vec<SavedEntity>>` | `map_dir/entities.bin` | Map-level saved entities such as respawn points. |

## Q9: What automated tests or reusable test harnesses currently cover dev plugins, world-object replication, voxel/map persistence, chunk sync, map transitions, and multi-client behavior, and what scenarios do they exercise?

**Direct answer:** Tests exist for voxel/map persistence, voxel engine lifecycle/API, chunk sync, room routing, map transitions, and plugin observer registration; no direct dev spawn panel test or direct world-object replication integration test was observed by the research agents.

### Evidence inventory

| Area | Observed coverage |
|---|---|
| Dev plugins | No `crates/dev/tests` directory was observed. Adjacent plugin harnesses exist in `crates/client/tests/plugin.rs` for client network plugin observer registration. |
| World-object replication | No direct test file for `spawn_world_object` or `WorldObjectId` replication was observed. Adjacent room-routing tests cover `MapInstanceId` observer behavior in `crates/server/tests/rooms.rs`. |
| Per-chunk world-object persistence | `crates/voxel_map_engine/src/persistence/mod.rs` contains chunk entity store roundtrip/missing/empty tests using `WorldObjectSpawn`. |
| Voxel/map persistence | `crates/server/tests/voxel_persistence.rs`, `crates/server/tests/world_persistence.rs`, and persistence module tests cover dirty chunk saves, restarts, homebase/overworld stores, and map-level entity persistence. |
| Chunk sync/client lifecycle | `crates/client/tests/chunk_sync.rs` covers `ChunkDataSync`, `UnloadColumn`, mesh despawn after unload, and server-pushed data without local generation. |
| Voxel engine lifecycle/API | `crates/voxel_map_engine/tests/lifecycle.rs` and `crates/voxel_map_engine/tests/api.rs` cover chunk loading/unloading, tickets, map isolation, voxel set/get, dirty/remesh state, and raycasts. |
| Map transitions/rooms | `crates/server/tests/map_transition.rs`, `crates/server/tests/rooms.rs`, and `crates/client/tests/map_transition.rs` cover server-side transition markers/rooms; the client transition test file is noted by the agent as having removed tests pending a full Lightyear message pipeline. |
| Multi-client behavior | `crates/server/tests/rooms.rs` covers room client membership, same-frame room transfer, and excluding unrelated rooms; no full multi-client network session test was observed. |

## Cross-Cutting Observations

1. `MapInstanceId` is the common map-scope component for characters, world objects, chunk streaming, room routing, and client hydration.
2. `WorldObjectId` is the replicated key; object definitions are loaded locally on both server and client and applied with reflection.
3. Runtime/generated world objects are server-authored and room-scoped; current dev-panel spawns are explicitly client-local at origin.
4. Voxel edits are the only observed client-originated authoritative world mutation with request/ack/reject semantics.
5. Per-chunk world-object persistence is separate from map-level entity persistence.
6. Client cursor-to-world targeting exists for voxel edits and uses camera ray plus voxel raycast; no world-object placement preview entity was observed in the inspected paths.

## Open Areas

- Direct tests for dev spawn panel behavior and world-object network replication were not observed by the agents; adjacent plugin, room, persistence, and chunk-sync tests were identified instead.
- No world-object client-to-server placement/request message or ack/reject protocol was observed in the inspected protocol/server/client world-object paths.
- This research did not run build/test commands because the QRSPI research phase is documentation-only and the gathered deliverable is `research.md`.
