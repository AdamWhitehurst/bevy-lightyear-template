# Research Findings

## Q1: How is the development plugin assembled and feature-gated, and how do its existing panels manage UI state, keyboard toggles, `egui` rendering, and access to ECS resources?

**Direct answer:** `dev` always adds Avian physics debug + `DevInspectorState`; optional `inspector`, `world-inspector`, and `spawn-panel` features
add `bevy_egui`, `bevy-inspector-egui`, root menu, and panels.

### Evidence

- Cargo features gate inspector dependencies and panels.

```toml
# crates/dev/Cargo.toml:6-16
[features]
inspector = ["dep:bevy_egui", "dep:bevy-inspector-egui", "dep:protocol"]
world-inspector = ["inspector"]
spawn-panel = ["inspector"]

[dependencies]
avian3d = { workspace = true, features = ["debug-plugin"] }
bevy = { workspace = true, default-features = true }
bevy_egui = { version = "0.39", optional = true }
bevy-inspector-egui = { version = "0.36", optional = true }
protocol = { workspace = true, optional = true }
```

- `DevPlugin` assembly: physics debug, state resource, F3/F4 update systems, and `EguiPrimaryContextPass` root menu only under `inspector`.

```rust
// crates/dev/src/lib.rs:17-36
impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsDebugPlugin::default())
            .init_resource::<DevInspectorState>()
            .add_systems(Startup, hide_physics_debug)
            .add_systems(Update, (toggle_physics_debug, toggle_dev_inspector));

        #[cfg(feature = "inspector")]
        {
            use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
            app.add_plugins(EguiPlugin::default()).add_systems(
                EguiPrimaryContextPass,
                draw_root_menu.run_if(inspector_enabled),
            );

            #[cfg(feature = "world-inspector")]
            app.add_plugins(panels::world_inspector::WorldInspectorPanelPlugin);

            #[cfg(feature = "spawn-panel")]
            app.add_plugins(panels::spawn::SpawnPanelPlugin);
        }
    }
}
```

- Keyboard toggles: F3 toggles `PhysicsGizmos`, F4 master inspector, F5 world inspector, F6 spawn panel.

```rust
// crates/dev/src/lib.rs:48-59
fn toggle_physics_debug(keys: Res<ButtonInput<KeyCode>>, mut store: ResMut<GizmoConfigStore>) {
    if keys.just_pressed(KeyCode::F3) {
        let (config, _) = store.config_mut::<PhysicsGizmos>();
        config.enabled = !config.enabled;
    }
}

fn toggle_dev_inspector(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<DevInspectorState>) {
    if keys.just_pressed(KeyCode::F4) {
        state.enabled = !state.enabled;
    }
}
```

```rust
// crates/dev/src/panels/world_inspector.rs:10-23
impl Plugin for WorldInspectorPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(WorldInspectorPlugin::new().run_if(world_inspector_enabled))
            .add_systems(Update, toggle_world_inspector);
    }
}

fn toggle_world_inspector(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<DevInspectorState>) {
    if keys.just_pressed(KeyCode::F5) {
        state.panels.world_inspector = !state.panels.world_inspector;
    }
}
```

```rust
// crates/dev/src/panels/spawn.rs:68-85
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

fn toggle_spawn_panel(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<DevInspectorState>) {
    if keys.just_pressed(KeyCode::F6) {
        state.panels.spawn_panel = !state.panels.spawn_panel;
    }
}
```

- Root menu handles missing primary egui context with traced early return and uses checkboxes for enabled panels.

```rust
// crates/dev/src/lib.rs:68-79
fn draw_root_menu(mut state: ResMut<DevInspectorState>, mut contexts: bevy_egui::EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else {
        // EguiContexts not yet attached to the primary window.
        trace!("draw_root_menu: EguiContexts not ready, skipping frame");
        return;
    };
    bevy_egui::egui::TopBottomPanel::top("dev_inspector_root").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Dev Inspector");
            ui.separator();
            #[cfg(feature = "world-inspector")]
            ui.checkbox(&mut state.panels.world_inspector, "World");
```

- Spawn panel state stores selected object, armed/pending placement requests, last rejection, and free-form reflected component selections.

```rust
// crates/dev/src/panels/spawn.rs:32-55
pub struct SpawnPanelUi {
    tab: SpawnTab,
    pub selected_object: Option<WorldObjectId>,
    pub placement: WorldObjectPlacementUi,
    selected_freeform: Vec<String>,
}

#[derive(Default)]
pub struct WorldObjectPlacementUi {
    pub armed: bool,
    pub next_sequence: u32,
    pub pending: Vec<PendingWorldObjectPlacement>,
    pub last_reject: Option<WorldObjectPlacementRejectReason>,
}

pub struct PendingWorldObjectPlacement {
    pub sequence: u32,
    pub object_id: WorldObjectId,
    pub base_position: Vec3,
    pub accepted_final_position: Option<Vec3>,
}
```

- Panel ECS access is via system params: `Option<Res<WorldObjectDefRegistry>>` for async loading, `Res<AppTypeRegistry>`, and `Commands`.

```rust
// crates/dev/src/panels/spawn.rs:89-116
fn draw_spawn_panel(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<SpawnPanelUi>,
    // Optional because definitions load during startup; the panel renders a loading label until ready.
    world_objects: Option<Res<WorldObjectDefRegistry>>,
    type_registry: Res<AppTypeRegistry>,
    mut commands: Commands,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("draw_spawn_panel: EguiContexts not ready, skipping frame");
        return;
    };
    egui::Window::new("Spawn").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut ui_state.tab, SpawnTab::DefDriven, "Def-driven");
            ui.selectable_value(&mut ui_state.tab, SpawnTab::FreeForm, "Free-form");
        });
        match ui_state.tab {
            SpawnTab::DefDriven => draw_def_tab(ui, &mut ui_state, world_objects.as_deref()),
            SpawnTab::FreeForm => {
                draw_freeform_tab(ui, &mut ui_state, &type_registry, &mut commands)
            }
        }
```

- Def-driven tab enumerates registry keys, arms/cancels placement, displays pending/accepted/reject state.

```rust
// crates/dev/src/panels/spawn.rs:126-180
if let Some(reg) = world_objects {
    egui::ComboBox::from_id_salt("world_object_picker")
        .selected_text(
            ui_state
                .selected_object
                .as_ref()
                .map(|i| i.0.as_str())
                .unwrap_or("(pick)"),
        )
        .show_ui(ui, |ui| {
            let mut ids: Vec<&WorldObjectId> = reg.objects.keys().collect();
            ids.sort_by(|a, b| a.0.cmp(&b.0));
            for id in ids {
                ui.selectable_value(&mut ui_state.selected_object, Some(id.clone()), &id.0);
            }
        });
    let has_selection = ui_state.selected_object.is_some();
    if ui.add_enabled(has_selection && !ui_state.placement.armed, egui::Button::new("Arm placement")).clicked() {
        ui_state.placement.armed = true;
        ui_state.placement.last_reject = None;
    }
    if ui_state.placement.armed && ui.button("Cancel placement").clicked() {
        ui_state.placement.armed = false;
    }
    ui.label(format!("Pending placement requests: {}", ui_state.placement.pending.len()));
    if let Some(reason) = &ui_state.placement.last_reject {
        ui.label(format!("Last placement rejected: {reason:?}"));
    }
} else {
    ui.label("(WorldObjectDefRegistry not yet loaded)");
}
```

- Free-form tab enumerates all registered `ReflectComponent`s, requires `ReflectDefault`, spawns local `DevSpawned`, and inserts reflected components.

```rust
// crates/dev/src/panels/spawn.rs:190-235
let registry = type_registry.read();
let mut component_paths: Vec<String> = registry
    .iter()
    .filter(|reg| reg.data::<ReflectComponent>().is_some())
    .map(|reg| reg.type_info().type_path().to_string())
    .collect();
component_paths.sort();
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

## Q2: How does the current world-object definition pipeline load, register, clone, and apply reflected components from `.object.ron` assets on both server and client?

**Direct answer:** `WorldObjectPlugin` registers a custom `.object.ron` asset loader that deserializes a flat type-path map into
`Vec<Box<dyn PartialReflect>>`, aggregates assets into `WorldObjectDefRegistry` by filename stem, clones component boxes with `reflect_clone`, and
applies them via `ReflectComponent::insert` on server spawns and client replication.

### Evidence

- Definition data is type-erased reflected components; clone requires `reflect_clone`.

```rust
// crates/protocol/src/world_object/types.rs:133-155
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
```

- Loader extension and RON shape are `.object.ron` and flat map from type path to component data.

```rust
// crates/protocol/src/world_object/loader.rs:24-41
impl AssetLoader for WorldObjectLoader {
    type Asset = WorldObjectDef;
    type Settings = ();
    type Error = WorldObjectLoadError;

    fn extensions(&self) -> &[&str] {
        &["object.ron"]
    }

    async fn load(...) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let registry = self.type_registry.read();
        deserialize_world_object(&bytes, &registry)
    }
}
```

````rust
// crates/protocol/src/world_object/loader.rs:45-60
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
    registry: &bevy::reflect::TypeRegistry,
) -> Result<WorldObjectDef, WorldObjectLoadError> {
    let components = reflect_loader::deserialize_component_map(bytes, registry)?;
    Ok(WorldObjectDef { components })
}
````

- Shared reflect map deserializer uses `TypeRegistrationDeserializer` and `TypedReflectDeserializer`.

```rust
// crates/protocol/src/reflect_loader.rs:56-63
pub fn deserialize_component_map(
    bytes: &[u8],
    registry: &TypeRegistry,
) -> Result<Vec<Box<dyn PartialReflect>>, ReflectLoadError> {
    let mut deserializer = ron::de::Deserializer::from_bytes(bytes)?;
    let components = ComponentMapDeserializer { registry }.deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(components)
}
```

- Plugin initializes asset/loader, native folder load or WASM manifest, inserts registry once, hot reloads in Ready, registers deserializable types.

```rust
// crates/protocol/src/world_object/plugin.rs:21-58
impl Plugin for WorldObjectPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<WorldObjectDef>();
        app.init_asset_loader::<WorldObjectLoader>();
        ...
        app.add_systems(Startup, load_world_object_defs);
        ...
        app.add_systems(
            Update,
            insert_world_object_defs.run_if(not(resource_exists::<WorldObjectDefRegistry>)),
        );
        app.add_systems(
            Update,
            reload_world_object_defs.run_if(in_state(AppState::Ready)),
        );

        // Register types for RON reflect-based component deserialization.
        app.register_type::<Health>();
        app.register_type::<crate::RespawnTimerConfig>();
        app.register_type::<ObjectCategory>();
        app.register_type::<VisualKind>();
        app.register_type::<ColliderConstructor>();
        app.register_type::<super::types::PlacementOffset>();
        app.register_type::<super::types::OnDeathEffects>();
        app.register_type::<super::types::DeathEffect>();
        app.register_type::<super::types::ActiveTransformation>();
    }
}
```

- Registry aggregation derives `WorldObjectId` from asset path by stripping `.object.ron` and clones defs into
  `HashMap<WorldObjectId, WorldObjectDef>`.

```rust
// crates/protocol/src/world_object/loading.rs:88-106
fn collect_object_defs(
    ids: impl Iterator<Item = AssetId<WorldObjectDef>>,
    object_assets: &Assets<WorldObjectDef>,
    asset_server: &AssetServer,
) -> HashMap<WorldObjectId, WorldObjectDef> {
    let mut objects = HashMap::new();
    for id in ids {
        let Some(def) = object_assets.get(id) else { continue; };
        let Some(path) = asset_server.get_path(id) else { continue; };
        let Some(obj_id) = object_id_from_path(&path) else { continue; };
        objects.insert(obj_id, def.clone());
    }
    objects
}
```

```rust
// crates/protocol/src/world_object/loading.rs:187-190
pub(super) fn object_id_from_path(path: &AssetPath) -> Option<WorldObjectId> {
    let name = path.path().file_name()?.to_str()?;
    Some(WorldObjectId(name.strip_suffix(".object.ron")?.to_string()))
}
```

- Component application queues a world command and uses `ReflectComponent::insert`.

```rust
// crates/protocol/src/world_object/spawn.rs:8-18
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

- Server spawns and client replication both clone def components, filter `ColliderConstructor` when a vox trimesh collider exists, then call
  `apply_object_components`.

```rust
// crates/server/src/world_object.rs:46-53
let vox_collider = vox_trimesh_collider(def, vox_registry, vox_assets, meshes);
let use_vox_collider = vox_collider.is_some();

let components = clone_def_components(def, use_vox_collider);
apply_object_components(commands, entity, components, type_registry.0.clone());

if let Some(collider) = vox_collider {
    commands.entity(entity).insert(collider);
}
```

```rust
// crates/client/src/world_object.rs:128-137
let vox_collider = vox_trimesh_collider(def, &vox_registry, &vox_assets, &meshes);
let has_vox_collider = vox_collider.is_some();

let components = clone_def_components(def, has_vox_collider);
apply_object_components(&mut commands, entity, components, type_registry.0.clone());

if let Some(collider) = vox_collider {
    commands.entity(entity).insert(collider);
}
```

## Q3: What is the full flow for def-driven world-object placement, from client UI state through terrain interaction and network messages to authoritative server spawning and client replication?

**Direct answer:** The spawn panel arms selected `WorldObjectId`; client PostUpdate intercepts `PlaceVoxel`, raycasts terrain to produce base
position, sends `WorldObjectPlacementRequest`, tracks pending preview; server validates against controlled character/map/loaded chunk, spawns
replicated chunk entity, sends ack/reject; client updates pending state and despawns matching local preview when replicated entity arrives.

### Evidence

- Network channel and messages are ordered reliable and registered bidirectional/request/ack/reject.

```rust
// crates/protocol/src/lib.rs:130-143
app.add_channel::<WorldObjectPlacementChannel>(ChannelSettings {
    mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
    ..default()
})
.add_direction(NetworkDirection::Bidirectional);

app.register_message::<WorldObjectPlacementRequest>()
    .add_direction(NetworkDirection::ClientToServer);
app.register_message::<WorldObjectPlacementAck>()
    .add_direction(NetworkDirection::ServerToClient);
app.register_message::<WorldObjectPlacementReject>()
    .add_direction(NetworkDirection::ServerToClient);
```

- Message structs carry `sequence`, object id, base/final position, and explicit reject reasons.

```rust
// crates/protocol/src/world_object/types.rs:24-58
pub struct WorldObjectPlacementRequest {
    pub sequence: u32,
    pub object_id: WorldObjectId,
    /// Un-offset placement base point. Server applies `PlacementOffset` exactly once.
    pub base_position: Vec3,
}

pub struct WorldObjectPlacementAck {
    pub sequence: u32,
    pub object_id: WorldObjectId,
    pub final_position: Vec3,
}

pub struct WorldObjectPlacementReject {
    pub sequence: u32,
    pub reason: WorldObjectPlacementRejectReason,
}

pub enum WorldObjectPlacementRejectReason {
    NoControlledCharacter,
    UnknownObject,
    NonFinitePosition,
    OutOfBounds,
```

- Client placement input requires armed UI, `PlaceVoxel`, selected object, and current raycast target; it sends request and pushes pending.

```rust
// crates/client/src/map.rs:432-489
fn handle_world_object_placement_input(
    mut ui_state: ResMut<SpawnPanelUi>,
    action_query: Query<&ActionState<PlayerActions>, With<Controlled>>,
    player_query: Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    mut voxel_world: VoxelWorld,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    mut message_sender: Query<&mut MessageSender<WorldObjectPlacementRequest>>,
) {
    if !ui_state.placement.armed { trace!("handle_world_object_placement_input: placement is not armed"); return; }
    let Ok(action_state) = action_query.single() else { trace!("handle_world_object_placement_input: no entity with ActionState + Controlled"); return; };
    if !action_state.just_pressed(&PlayerActions::PlaceVoxel) { trace!("handle_world_object_placement_input: place action not pressed"); return; }
    let Some(object_id) = ui_state.selected_object.clone() else { trace!("handle_world_object_placement_input: placement armed without selected object"); return; };
    let Some(target) = current_placement_target(&player_query, &mut voxel_world, &camera_query, &window_query) else { trace!("handle_world_object_placement_input: no placement target"); return; };

    let sequence = ui_state.placement.next_sequence();
    let request = WorldObjectPlacementRequest { sequence, object_id: object_id.clone(), base_position: target.base_position };
    for mut sender in message_sender.iter_mut() {
        sender.send::<WorldObjectPlacementChannel>(request.clone());
    }
    ui_state.placement.pending.push(PendingWorldObjectPlacement {
        sequence,
        object_id,
```

- When placement is armed, normal voxel edit input is skipped.

```rust
// crates/client/src/map.rs:308-321
fn handle_voxel_input(
    ...
    #[cfg(feature = "spawn-panel")] placement_ui: Option<Res<SpawnPanelUi>>,
) {
    #[cfg(feature = "spawn-panel")]
    if placement_ui.as_ref().is_some_and(|ui| ui.placement.armed) {
        trace!("handle_voxel_input: world object placement armed; skipping voxel input");
        return;
    }
```

- Placement target comes from predicted controlled player's `ChunkTicket`, camera cursor ray, solid voxel raycast, and adjacent normal.

```rust
// crates/client/src/map.rs:50-76
pub fn current_placement_target(
    player_query: &Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>,
    voxel_world: &mut VoxelWorld,
    camera_query: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: &Query<&Window, With<PrimaryWindow>>,
) -> Option<PlacementTarget> {
    let Ok(chunk_ticket) = player_query.single() else { trace!("current_placement_target: no predicted player with ChunkTicket"); return None; };
    let Some(ray) = camera_ray(camera_query, window_query) else { trace!("current_placement_target: no camera ray"); return None; };
    let Some(hit) = voxel_world.raycast(chunk_ticket.map_entity, ray, RAYCAST_MAX_DISTANCE, |v| {
        matches!(v, WorldVoxel::Solid(_))
    }) else { trace!("current_placement_target: raycast hit nothing"); return None; };
    let Some(normal) = hit.normal else { trace!("current_placement_target: hit has no normal"); return None; };
    let hit_normal = normal.as_ivec3();
    Some(PlacementTarget {
        base_position: (hit.position + hit_normal).as_vec3(),
```

- Server system is gated by world object and vox registries, resolves client map, validates, spawns, then sends ack/reject.

```rust
// crates/server/src/map.rs:645-659
handle_world_object_placement_requests.run_if(
    resource_exists::<WorldObjectDefRegistry>
        .and(resource_exists::<VoxModelRegistry>),
),
...
crate::chunk_entities::spawn_chunk_entities
    .after(lifecycle::poll_chunk_tasks)
    .run_if(
        resource_exists::<WorldObjectDefRegistry>
            .and(resource_exists::<VoxModelRegistry>),
    ),
```

```rust
// crates/server/src/map.rs:756-824
pub fn handle_world_object_placement_requests(
    mut receivers: Query<(Entity, &mut MessageReceiver<WorldObjectPlacementRequest>)>,
    mut ack_senders: Query<&mut MessageSender<WorldObjectPlacementAck>>,
    mut reject_senders: Query<&mut MessageSender<WorldObjectPlacementReject>>,
    controlled_query: Query<(&ControlledBy, &MapInstanceId), With<CharacterMarker>>,
    map_registry: Res<MapRegistry>,
    map_query: Query<(&VoxelMapInstance, &MapDimensions)>,
    defs: Res<WorldObjectDefRegistry>,
    ...
) {
    for (client_entity, mut receiver) in &mut receivers {
        for request in receiver.receive() {
            let Some((map_entity, map_id)) = resolve_player_map(client_entity, &controlled_query, &map_registry) else {
                send_placement_reject(client_entity, request.sequence, WorldObjectPlacementRejectReason::NoControlledCharacter, &mut reject_senders);
                continue;
            };
            let (instance, dimensions) = map_query.get(map_entity).expect("resolved map entity must have VoxelMapInstance and MapDimensions");
            match validate_world_object_placement(&request, instance, dimensions, &defs) {
                Ok((def, final_position, _)) => {
                    crate::world_object::spawn_placed_world_object(...);
                    send_placement_ack(client_entity, WorldObjectPlacementAck { sequence: request.sequence, object_id: request.object_id, final_position }, &mut ack_senders);
                }
                Err(reason) => { send_placement_reject(client_entity, request.sequence, reason, &mut reject_senders); continue; }
            }
        }
    }
}
```

- Validation rejects non-finite, unknown object, out-of-bounds, and unavailable chunk; success returns def/final position/chunk.

```rust
// crates/server/src/map.rs:836-868
pub fn validate_world_object_placement<'a>(
    request: &WorldObjectPlacementRequest,
    instance: &VoxelMapInstance,
    dimensions: &MapDimensions,
    defs: &'a WorldObjectDefRegistry,
) -> Result<(&'a WorldObjectDef, Vec3, IVec3), WorldObjectPlacementRejectReason> {
    if !request.base_position.is_finite() { return Err(WorldObjectPlacementRejectReason::NonFinitePosition); }
    let Some(def) = defs.get(&request.object_id) else { return Err(WorldObjectPlacementRejectReason::UnknownObject); };
    let final_position = crate::world_object::final_placed_world_object_position(def, request.base_position);
    if !final_position.is_finite() { return Err(WorldObjectPlacementRejectReason::NonFinitePosition); }
    let chunk_pos = crate::chunk_entities::chunk_pos_for_world_position(final_position, dimensions.chunk_size);
    if !placement_chunk_in_bounds(chunk_pos, dimensions) { return Err(WorldObjectPlacementRejectReason::OutOfBounds); }
    let column = voxel_map_engine::prelude::chunk_to_column(chunk_pos);
    if !instance.chunk_levels.contains_key(&column) || instance.get_chunk_data(chunk_pos).is_none() {
        return Err(WorldObjectPlacementRejectReason::ChunkUnavailable);
    }
    Ok((def, final_position, chunk_pos))
}
```

- Client ack/reject mutate pending state; replication reconciliation removes matching preview by `WorldObjectId` and position.

```rust
// crates/client/src/map.rs:692-730
fn handle_world_object_placement_ack(
    mut receivers: Query<&mut MessageReceiver<WorldObjectPlacementAck>>,
    mut ui_state: ResMut<SpawnPanelUi>,
) {
    for mut receiver in &mut receivers {
        for ack in receiver.receive() {
            let Some(pending) = ui_state.placement.pending.iter_mut().find(|pending| pending.sequence == ack.sequence) else {
                trace!("handle_world_object_placement_ack: ack seq={} had no pending placement", ack.sequence);
                continue;
            };
            pending.accepted_final_position = Some(ack.final_position);
            ui_state.placement.last_reject = None;
        }
    }
}

fn handle_world_object_placement_reject(...) {
    ...
    ui_state.placement.pending.retain(|pending| pending.sequence != reject.sequence);
```

```rust
// crates/client/src/map.rs:637-662
pub fn reconcile_placement_preview_on_replication(
    mut commands: Commands,
    mut ui_state: ResMut<SpawnPanelUi>,
    replicated_query: Query<(&WorldObjectId, &Position), Added<Replicated>>,
    preview_query: Query<(Entity, &WorldObjectPlacementPreview, &Transform)>,
) {
    for (replicated_id, replicated_position) in &replicated_query {
        let replicated_position = Vec3::from(replicated_position.0);
        for (preview_entity, preview, preview_transform) in &preview_query {
            let Some(sequence) = preview.sequence else { continue; };
            if &preview.object_id != replicated_id { continue; }
            if positions_match(preview_transform.translation, replicated_position) {
                commands.entity(preview_entity).despawn();
                ui_state.placement.pending.retain(|pending| pending.sequence != sequence);
            }
        }
    }
}
```

## Q4: How are world-object entities identified after spawning, including `WorldObjectId`, map/chunk ownership, replication markers, visual children, physics components, and any stable or persisted entity references?

**Direct answer:** Server world objects are identified by replicated `WorldObjectId`, `MapInstanceId`, `Replicate`, `NetworkVisibility`, and usually
`ChunkEntityRef`; clients use replicated `WorldObjectId` to reapply defs, attach local visual children, rebuild colliders, and despawn stale map
objects.

### Evidence

- `WorldObjectId` is explicitly documented as the single replicated component used to identify definition lookup.

```rust
// crates/protocol/src/world_object/types.rs:9-14
/// Unique identifier for a world object definition. Derived from the `.object.ron` filename.
///
/// Also used as a replicated ECS component — the single component Lightyear sends to clients
/// to identify which definition to look up in `WorldObjectDefRegistry`.
#[derive(Component, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct WorldObjectId(pub String);
```

- Base server spawn components: id, rotation, map id, replication target, network visibility.

```rust
// crates/server/src/world_object.rs:36-43
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

- Placed objects add `Position` and `ChunkEntityRef { chunk_pos, map_entity }`.

```rust
// crates/server/src/world_object.rs:104-109
let chunk_pos = crate::chunk_entities::chunk_pos_for_world_position(final_position, chunk_size);
commands.entity(entity).insert((
    Position(final_position),
    protocol::map::ChunkEntityRef {
        chunk_pos,
        map_entity,
```

- `ChunkEntityRef` stores an ECS `Entity` map reference and chunk position for lifecycle.

```rust
// crates/protocol/src/map/mod.rs:34-39
/// Tags an entity as belonging to a specific chunk on a specific map.
/// Used to save/despawn entities when their chunk is evicted.
#[derive(Component, Clone, Debug)]
pub struct ChunkEntityRef {
    pub chunk_pos: IVec3,
    pub map_entity: Entity,
}
```

- Client replication handler uses `Added<Replicated>`, `WorldObjectId`, `Position`, `Rotation`; despawns stale objects if their `MapInstanceId` is not
  in local `MapRegistry`.

```rust
// crates/client/src/world_object.rs:103-127
pub fn on_world_object_replicated(
    query: Query<(Entity, &WorldObjectId, Option<&Position>, Option<&Rotation>), Added<Replicated>>,
    registry: Res<WorldObjectDefRegistry>,
    map_registry: Res<MapRegistry>,
    map_id_query: Query<&MapInstanceId>,
    ...
) {
    for (entity, id, pos, rot) in &query {
        if let Ok(entity_mid) = map_id_query.get(entity) {
            if !map_registry.0.contains_key(entity_mid) {
                trace!("Despawning stale world object {entity:?} from map {entity_mid:?}");
                commands.entity(entity).despawn();
                continue;
            }
        }
        let Some(def) = registry.get(id) else {
            warn!("Replicated world object has unknown id: {:?}", id.0);
            continue;
        };
```

- Client inserts `Transform` from physics position/rotation before attaching visual child.

```rust
// crates/client/src/world_object.rs:139-149
// Insert Transform matching Position so children (Mesh3d) have a parent
// with GlobalTransform. PhysicsTransformPlugin is disabled, so Position
// does not auto-require Transform; lightyear's add_transform only runs
// in PostUpdate, after children are already attached.
let transform = transform_from_physics(pos, rot);
commands.entity(entity).insert(transform);

attach_visual(
    &mut commands,
    entity,
```

- Vox visuals are child `Mesh3d`/`MeshMaterial3d`; server/client physics collider is trimesh from vox mesh or reflected fallback
  `ColliderConstructor`.

```rust
// crates/client/src/world_object.rs:280-299
fn attach_vox_mesh(
    commands: &mut Commands,
    entity: Entity,
    vox_path: &str,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    default_material: &DefaultVoxModelMaterial,
) {
    let Some(asset_handle) = vox_registry.get(vox_path) else { warn!("Vox model not found in registry: {vox_path}"); return; };
    let Some(asset) = vox_assets.get(asset_handle) else { warn!("VoxModelAsset not yet loaded: {vox_path}"); return; };
    let Some(mesh_handle) = asset.lod_meshes.first() else { warn!("VoxModelAsset has no LOD meshes: {vox_path}"); return; };

    commands
        .entity(entity)
        .insert(Visibility::default())
        .with_child((
            Mesh3d(mesh_handle.clone()),
            MeshMaterial3d(default_material.0.clone()),
        ));
}
```

- Stable persistence data is not ECS entity id; per-chunk spawns store object id, final/base position kind, and RON serialized components.

```rust
// crates/voxel_map_engine/src/config.rs:40-54
/// Spawn data for a world object placed during the Features stage.
///
/// Uses bare `String` for `object_id` (not `WorldObjectId`) because `WorldObjectId`
/// lives in the `protocol` crate, and `voxel_map_engine` must not depend on it.
/// The server spawn system converts to `WorldObjectId` at the boundary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldObjectSpawn {
    pub object_id: String,
    pub position: Vec3,
    #[serde(default)]
    pub position_kind: WorldObjectPositionKind,
    /// RON-serialized persisted components. Empty means no component snapshots;
    /// it is not a fresh-vs-reload signal.
    #[serde(default)]
    pub persisted_components: Vec<PersistedComponent>,
}
```

## Q5: What existing patterns remove, despawn, transform, or replace world-object-related entities or components, and how do those paths handle children, colliders, replicated state, and chunk/map bookkeeping?

**Direct answer:** Existing world-object lifecycle mutations are chunk eviction despawn-with-save, client stale preview/object despawn,
visual/collider rebuild on `VisualKind` changes, and server death-effect transformation via reflected component diff; no direct world-object deletion
request path exists.

### Evidence

- Transforming world objects removes reflected components absent from new def, always removes old `Collider`, applies new def components, then inserts
  new vox collider if available.

```rust
// crates/server/src/world_object.rs:120-154
pub fn apply_transformation(
    commands: &mut Commands,
    entity: Entity,
    current_def: &WorldObjectDef,
    source_def: &WorldObjectDef,
    ...
) {
    let source_type_paths: HashSet<&str> = source_def.components.iter().map(|c| c.reflect_type_path()).collect();

    remove_absent_components(commands, entity, current_def, &source_type_paths, type_registry);

    // Always remove the old collider — it may have been built from a vox trimesh
    // (not present in the def's component list) and won't be caught by
    // remove_absent_components.
    commands.entity(entity).remove::<Collider>();

    let vox_collider = vox_trimesh_collider(source_def, vox_registry, vox_assets, meshes);
    let use_vox_collider = vox_collider.is_some();
    let components = clone_def_components(source_def, use_vox_collider);
    apply_object_components(commands, entity, components, type_registry.0.clone());
```

```rust
// crates/server/src/world_object.rs:160-184
fn remove_absent_components(...) {
    let registry = type_registry.read();
    for component in &current_def.components {
        let path = component.reflect_type_path();
        if keep_paths.contains(path) { continue; }
        let Some(registration) = registry.get_with_type_path(path) else { continue; };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else { continue; };
        let reflect_component = reflect_component.clone();
        commands.queue(move |world: &mut World| {
            if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
                reflect_component.remove(&mut entity_mut);
            }
        });
    }
}
```

- Death effects call transformation and mark `ActiveTransformation`; timed reversion applies original def and removes marker.

```rust
// crates/server/src/gameplay.rs:175-216
fn on_death_effects(
    mut commands: Commands,
    mut events: MessageReader<DeathEvent>,
    effect_query: Query<(&OnDeathEffects, &WorldObjectId)>,
    defs: Res<WorldObjectDefRegistry>,
    ...
) {
    for event in events.read() {
        let Ok((effects, obj_id)) = effect_query.get(event.entity) else { continue; };
        for effect in &effects.0 {
            match effect {
                DeathEffect::TransformInto { source, revert_after_ticks } => {
                    let source_id = WorldObjectId(source.clone());
                    let Some(source_def) = defs.get(&source_id) else { warn!("Unknown transformation source '{source}'"); continue; };
                    let Some(current_def) = defs.get(obj_id) else { warn!("Unknown current def '{}'", obj_id.0); continue; };
                    crate::world_object::apply_transformation(...);
                    commands.entity(event.entity).insert(ActiveTransformation {
                        source: source.clone(),
                        ticks_remaining: *revert_after_ticks,
                    });
```

```rust
// crates/server/src/gameplay.rs:225-263
fn tick_active_transformations(
    mut commands: Commands,
    mut query: Query<(Entity, &mut ActiveTransformation, &WorldObjectId)>,
    defs: Res<WorldObjectDefRegistry>,
    ...
) {
    for (entity, mut transform, obj_id) in &mut query {
        let Some(ref mut remaining) = transform.ticks_remaining else { continue; };
        *remaining = remaining.saturating_sub(1);
        if *remaining > 0 { continue; }
        ...
        crate::world_object::apply_transformation(...);
        commands.entity(entity).remove::<ActiveTransformation>();
    }
}
```

- Chunk eviction saves current object spawns grouped by map/chunk, then despawns entity. This is map/chunk bookkeeping path.

```rust
// crates/server/src/chunk_entities.rs:101-156
/// Saves and despawns chunk entities when their chunk is evicted (column unloaded).
pub fn evict_chunk_entities(
    mut commands: Commands,
    entity_query: Query<(Entity, &ChunkEntityRef, &WorldObjectId, &Position, Option<&ActiveTransformation>, Option<&protocol::Health>)>,
    map_query: Query<&VoxelMapInstance>,
    mut store_query: Query<(&StoreBackend<IVec3, Vec<WorldObjectSpawn>, FsChunkEntitiesStore>, &mut PendingStoreOps<IVec3, Vec<WorldObjectSpawn>>)>,
) {
    let mut by_chunk: HashMap<(Entity, IVec3), Vec<(Entity, WorldObjectSpawn)>> = HashMap::new();
    for (entity, chunk_ref, obj_id, pos, active_transform, health) in &entity_query {
        let Ok(instance) = map_query.get(chunk_ref.map_entity) else { continue; };
        let col = chunk_to_column(chunk_ref.chunk_pos);
        if instance.chunk_levels.contains_key(&col) { continue; }
        let persisted = serialize_persisted(active_transform, health);
        by_chunk.entry((chunk_ref.map_entity, chunk_ref.chunk_pos)).or_default().push((entity, WorldObjectSpawn {
            object_id: obj_id.0.clone(),
            position: pos.0,
            position_kind: WorldObjectPositionKind::Final,
            persisted_components: persisted,
        }));
    }
    ...
    for (entity, _) in entities {
        commands.entity(entity).despawn();
    }
}
```

- Persistence currently serializes only `ActiveTransformation` and `Health` despite presence of generic marker type data.

```rust
// crates/server/src/chunk_entities.rs:267-284
fn serialize_persisted(
    active_transform: Option<&ActiveTransformation>,
    health: Option<&protocol::Health>,
) -> Vec<PersistedComponent> {
    let mut result = Vec::new();
    if let Some(at) = active_transform {
        if let Ok(ron_data) = ron::to_string(at) {
            result.push(PersistedComponent {
                type_path: std::any::type_name::<ActiveTransformation>().to_string(),
                ron_data,
            });
        }
    }
    if let Some(h) = health {
```

- Reload restore reapplies transformation source def and inserts persisted marker/health.

```rust
// crates/server/src/chunk_entities.rs:296-337
fn restore_persisted(
    commands: &mut Commands,
    entity: Entity,
    persisted: &[PersistedComponent],
    base_def: &protocol::world_object::WorldObjectDef,
    defs: &WorldObjectDefRegistry,
    ...
) {
    let at_type = std::any::type_name::<ActiveTransformation>();
    let health_type = std::any::type_name::<protocol::Health>();
    ...
    if let Some(at) = active_transform {
        let source_id = WorldObjectId(at.source.clone());
        if let Some(source_def) = defs.get(&source_id) {
            crate::world_object::apply_transformation(commands, entity, base_def, source_def, ...);
        }
        commands.entity(entity).insert(at);
    }

    if let Some(health) = persisted_health {
        commands.entity(entity).insert(health);
    }
}
```

- Client visual change path despawns old visual children, removes collider, optionally rebuilds collider and child mesh.

```rust
// crates/client/src/world_object.rs:228-259
pub fn on_visual_kind_changed(
    mut commands: Commands,
    query: Query<(Entity, &VisualKind), Changed<VisualKind>>,
    ...
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
            if let Some(collider) = vox_trimesh_collider_from_path(path, &vox_registry, &vox_assets, &meshes) {
                commands.entity(entity).insert(collider);
            }
            attach_vox_mesh(...);
```

- Local placement preview despawns stale sequence previews and hover previews; matching replicated object despawns corresponding preview.

```rust
// crates/client/src/map.rs:513-537
let still_pending = ui_state
    .placement
    .pending
    .iter()
    .any(|pending| pending.sequence == sequence);
if !still_pending {
    trace!("update_world_object_placement_preview: despawning stale sequence preview {}", sequence);
    commands.entity(entity).despawn();
}
...
if !ui_state.placement.armed {
    for (entity, _, preview) in &mut preview_query {
        if preview.sequence.is_none() {
            trace!("update_world_object_placement_preview: despawning disarmed hover preview");
            commands.entity(entity).despawn();
        }
    }
```

- None found: no codebase path named `delete_world_object`, `remove_world_object`, or `WorldObjectRemoval*`; nearest removal concepts are above plus
  voxel remove input and chunk eviction.

## Q6: What runtime editing mechanisms already exist through reflection, the world inspector, or component insertion/removal helpers, and which world-object components are registered or marked specially for reflection, persistence, or spawn-only behavior?

**Direct answer:** Runtime editing exists via `bevy_inspector_egui` world inspector, spawn-panel free-form reflected component insertion, and server
reflection helpers for insertion/removal during transformations; special world-object reflect markers include `SpawnOnly` on `PlacementOffset`,
`Persist` on `ActiveTransformation`, and marker type-data structs `ReflectPersist` / `ReflectSpawnOnly`.

### Evidence

- World inspector wraps `bevy_inspector_egui::quick::WorldInspectorPlugin` behind toggle/run_if.

```rust
// crates/dev/src/panels/world_inspector.rs:1-13
//! World-tree inspector panel. Wraps `bevy_inspector_egui::quick::WorldInspectorPlugin`
//! with a runtime toggle.

use bevy_inspector_egui::quick::WorldInspectorPlugin;

pub struct WorldInspectorPanelPlugin;

impl Plugin for WorldInspectorPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(WorldInspectorPlugin::new().run_if(world_inspector_enabled))
            .add_systems(Update, toggle_world_inspector);
    }
}
```

- Free-form spawn is reflection-driven insertion through `ReflectDefault` + `apply_object_components`.

```rust
// crates/dev/src/panels/spawn.rs:190-235
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
apply_object_components(commands, entity, components, type_registry.0.clone());
```

- Runtime reflected removal helper uses `ReflectComponent::remove`; insertion helper uses `ReflectComponent::insert`.

```rust
// crates/server/src/world_object.rs:173-183
let Some(registration) = registry.get_with_type_path(path) else { continue; };
let Some(reflect_component) = registration.data::<ReflectComponent>() else { continue; };
let reflect_component = reflect_component.clone();
commands.queue(move |world: &mut World| {
    if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
        reflect_component.remove(&mut entity_mut);
    }
});
```

```rust
// crates/protocol/src/world_object/spawn.rs:20-34
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

- World-object component reflection and special markers:

| Component/type         | Marker evidence                                            | Meaning in code                                                 |
| ---------------------- | ---------------------------------------------------------- | --------------------------------------------------------------- |
| `PlacementOffset`      | `#[reflect(Component, Serialize, Deserialize, SpawnOnly)]` | offset applied at spawn; marked spawn-only                      |
| `ObjectCategory`       | `#[reflect(Component, Serialize, Deserialize)]`            | reflected serializable component                                |
| `VisualKind`           | `#[reflect(Component, Serialize, Deserialize)]`            | reflected serializable component; drives client visual/collider |
| `OnDeathEffects`       | `#[reflect(Component)]`                                    | reflected death behavior component                              |
| `ActiveTransformation` | `#[reflect(Component, Persist)]`                           | persistent transform marker serialized on chunk eviction        |
| `ReflectPersist`       | `FromType<T>`                                              | type data marker definition                                     |
| `ReflectSpawnOnly`     | `FromType<T>`                                              | type data marker definition                                     |

```rust
// crates/protocol/src/world_object/types.rs:20-22
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
#[reflect(Component, Serialize, Deserialize, SpawnOnly)]
pub struct PlacementOffset(pub Vec3);
```

```rust
// crates/protocol/src/world_object/types.rs:105-111
/// Tracks an active transformation on a world object. Persisted across chunk eviction.
#[derive(Component, Reflect, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[reflect(Component, Persist)]
pub struct ActiveTransformation {
    pub source: String,
    pub ticks_remaining: Option<u16>,
}
```

```rust
// crates/protocol/src/world_object/types.rs:113-130
/// Reflect type data: marks a component for serialization during chunk eviction.
#[derive(Clone)]
pub struct ReflectPersist;

impl<T: Reflect> bevy::reflect::FromType<T> for ReflectPersist {
    fn from_type() -> Self { ReflectPersist }
}

/// Reflect type data: marks a component as spawn-only (skipped on reload).
#[derive(Clone)]
pub struct ReflectSpawnOnly;
```

- Registered world-object related reflected types in `WorldObjectPlugin`: `Health`, `RespawnTimerConfig`, `ObjectCategory`, `VisualKind`,
  `ColliderConstructor`, `PlacementOffset`, `OnDeathEffects`, `DeathEffect`, `ActiveTransformation`.

```rust
// crates/protocol/src/world_object/plugin.rs:50-58
app.register_type::<Health>();
app.register_type::<crate::RespawnTimerConfig>();
app.register_type::<ObjectCategory>();
app.register_type::<VisualKind>();
app.register_type::<ColliderConstructor>();
app.register_type::<super::types::PlacementOffset>();
app.register_type::<super::types::OnDeathEffects>();
app.register_type::<super::types::DeathEffect>();
app.register_type::<super::types::ActiveTransformation>();
```

## Q7: How do input focus, terrain raycast/picking, camera context, and UI interaction currently determine which world position or entity is being targeted in client gameplay and dev tooling?

**Direct answer:** Targeting is cursor-camera-ray to voxel terrain only; it uses the single `Camera3d`, primary window cursor adjusted for camera
viewport, `VoxelWorld::raycast` against solid voxels, and Leafwing `PlayerActions`; no entity picking path for world objects exists in the client/dev
code.

### Evidence

- Camera ray source: exactly one camera and primary window; cursor position adjusted by viewport rect, then `viewport_to_world`.

```rust
// crates/client/src/map.rs:673-689
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

    camera.viewport_to_world(camera_transform, viewport_pos).ok()
}
```

- Terrain targeting uses voxel raycast, not entity picking.

```rust
// crates/voxel_map_engine/src/api.rs:71-103
pub fn raycast(
    &self,
    map: Entity,
    ray: Ray3d,
    max_distance: f32,
    filter: impl Fn(WorldVoxel) -> bool,
) -> Option<VoxelRaycastResult> {
    let Ok((instance, generator)) = self.maps.get(map) else {
        warn!("raycast: entity {map:?} has no VoxelMapInstance");
        return None;
    };

    let start = ray.origin;
    let end = ray.origin + *ray.direction * max_distance;
    ...
    voxel_line_traversal(start, end, |voxel_pos, t, face| {
        let voxel = lookup_voxel(...);
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
```

- Placement target consumes raycast result and chooses adjacent face position.

```rust
// crates/client/src/map.rs:64-76
let Some(hit) = voxel_world.raycast(chunk_ticket.map_entity, ray, RAYCAST_MAX_DISTANCE, |v| {
    matches!(v, WorldVoxel::Solid(_))
}) else {
    trace!("current_placement_target: raycast hit nothing");
    return None;
};
let Some(normal) = hit.normal else {
    trace!("current_placement_target: hit has no normal");
    return None;
};
let hit_normal = normal.as_ivec3();
Some(PlacementTarget {
    base_position: (hit.position + hit_normal).as_vec3(),
```

- Voxel input uses same raycast; remove targets hit voxel, place targets adjacent normal. Input action source is `ActionState<PlayerActions>` on
  `Controlled` entity.

```rust
// crates/client/src/map.rs:333-357
let removing = action_state.just_pressed(&PlayerActions::RemoveVoxel);
let placing = action_state.just_pressed(&PlayerActions::PlaceVoxel);
if !removing && !placing {
    trace!("handle_voxel_input: no voxel edit action pressed");
    return;
}

let Some(ray) = camera_ray(&camera_query, &window_query) else {
    warn!("handle_voxel_input: no camera ray (no cursor position?)");
    return;
};

let Some(hit) = voxel_world.raycast(chunk_ticket.map_entity, ray, RAYCAST_MAX_DISTANCE, |v| {
    matches!(v, WorldVoxel::Solid(_))
}) else { ... };

let (position, voxel) = if removing {
    (hit.position, VoxelType::Air)
} else if let Some(normal) = hit.normal {
    (hit.position + normal.as_ivec3(), VoxelType::Solid(0))
} else {
    trace!("handle_voxel_input: place hit has no normal");
    return;
};
```

- UI interaction affects gameplay targeting only by state: when placement is armed, voxel input early-outs; no egui focus check found in these paths.

```rust
// crates/client/src/map.rs:318-321
if placement_ui.as_ref().is_some_and(|ui| ui.placement.armed) {
    trace!("handle_voxel_input: world object placement armed; skipping voxel input");
    return;
}
```

## Q8: What tests cover world-object placement, rejection, replication, persistence, and lifecycle changes, and what test utilities or app setup patterns do they use?

**Direct answer:** Server tests cover accepted placement spawn/query and validation rejection reasons; client feature-gated tests cover placement UI
sequencing, visual-only previews, and replication reconciliation; voxel-map persistence tests cover chunk entity storage shape. No direct test found
for server death-effect transformation or full network roundtrip placement.

### Evidence

- Server accepted placement test uses `App`, `MinimalPlugins`, `ReplicationSendPlugin`, registers `PlacementOffset`, inserts `VoxModelRegistry` and
  asset resources, then runs `spawn_placed_world_object` via `RunSystemOnce`.

```rust
// crates/server/tests/world_object_placement.rs:66-126
fn accepted_placement_spawns_replicated_chunk_entity() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(ReplicationSendPlugin);
    app.register_type::<PlacementOffset>();
    app.insert_resource(VoxModelRegistry { models: HashMap::new() });
    app.init_resource::<Assets<VoxModelAsset>>();
    app.init_resource::<Assets<Mesh>>();
    ...
    app.world_mut().run_system_once(move |mut commands: Commands, type_registry: Res<AppTypeRegistry>, vox_registry: Res<VoxModelRegistry>, vox_assets: Res<Assets<VoxModelAsset>>, meshes: Res<Assets<Mesh>>| {
        server::world_object::spawn_placed_world_object(...);
    }).unwrap();

    let mut query = app.world_mut().query::<(&WorldObjectId, &MapInstanceId, &Position, &ChunkEntityRef, &Replicate, &NetworkVisibility)>();
    let objects: Vec<_> = query.iter(app.world()).collect();
    assert_eq!(objects.len(), 1);
    assert_eq!(spawned_id, &object_id());
    assert_eq!(spawned_map_id, &MapInstanceId::Overworld);
    assert_eq!(position.0, final_position);
    assert_eq!(chunk_ref.map_entity, map_entity);
```

- Server validation test confirms final position/chunk after offset.

```rust
// crates/server/tests/world_object_placement.rs:131-148
fn accepted_placement_validation_returns_final_position_and_chunk() {
    let base_position = Vec3::new(1.0, 2.0, 3.0);
    let expected_final = Vec3::new(1.0, 3.5, 3.0);
    let chunk_pos = voxel_map_engine::lifecycle::world_to_chunk_pos(expected_final, CHUNK_SIZE);
    let instance = loaded_instance(chunk_pos);
    let dims = dimensions(None);
    let registry = test_registry();

    let (_, final_position, validated_chunk) = server::map::validate_world_object_placement(
        &placement_request(base_position), &instance, &dims, &registry,
    ).expect("placement should be valid");

    assert_eq!(final_position, expected_final);
    assert_eq!(validated_chunk, chunk_pos);
}
```

- Server rejection test covers `UnknownObject`, `NonFinitePosition`, `OutOfBounds`, `ChunkUnavailable`, plus enum presence for
  `NoControlledCharacter`.

```rust
// crates/server/tests/world_object_placement.rs:152-194
fn rejected_placement_spawns_no_entity() {
    ...
    let unknown = WorldObjectPlacementRequest { object_id: WorldObjectId("missing".to_string()), ..placement_request(base_position) };
    assert_eq!(server::map::validate_world_object_placement(&unknown, &loaded, &dims, &registry).unwrap_err(), WorldObjectPlacementRejectReason::UnknownObject);

    let non_finite = placement_request(Vec3::new(f32::NAN, 0.0, 0.0));
    assert_eq!(server::map::validate_world_object_placement(&non_finite, &loaded, &dims, &registry).unwrap_err(), WorldObjectPlacementRejectReason::NonFinitePosition);

    let out_of_bounds = placement_request(Vec3::new(200.0, 0.0, 0.0));
    assert_eq!(server::map::validate_world_object_placement(&out_of_bounds, &loaded, &dims, &registry).unwrap_err(), WorldObjectPlacementRejectReason::OutOfBounds);

    let unavailable = placement_request(base_position + Vec3::new(32.0, 0.0, 0.0));
    assert_eq!(server::map::validate_world_object_placement(&unavailable, &loaded, &dims, &registry).unwrap_err(), WorldObjectPlacementRejectReason::ChunkUnavailable);
}
```

- Client UI state test covers sequence increments and pending ack final position.

```rust
// crates/client/tests/plugin.rs:10-30
#[cfg(feature = "spawn-panel")]
#[test]
fn world_object_placement_ui_sequences_and_pending_ack() {
    let mut ui = WorldObjectPlacementUi::default();
    assert_eq!(ui.next_sequence(), 0);
    assert_eq!(ui.next_sequence(), 1);

    ui.pending.push(PendingWorldObjectPlacement {
        sequence: 1,
        object_id: WorldObjectId("test:crate".to_string()),
        base_position: Vec3::new(1.0, 2.0, 3.0),
        accepted_final_position: None,
    });
    ui.pending[0].accepted_final_position = Some(Vec3::new(1.0, 3.5, 3.0));
```

- Client preview test asserts local preview has preview marker and transform but no collider, physics position, map id, replicated marker, or
  authoritative `WorldObjectId` component.

```rust
// crates/client/tests/plugin.rs:81-88
let entity_ref = app.world().entity(entity);
assert!(entity_ref.contains::<WorldObjectPlacementPreview>());
assert!(entity_ref.contains::<Transform>());
assert!(!entity_ref.contains::<Collider>());
assert!(!entity_ref.contains::<Position>());
assert!(!entity_ref.contains::<MapInstanceId>());
assert!(!entity_ref.contains::<Replicated>());
assert!(!entity_ref.contains::<protocol::world_object::WorldObjectId>());
```

- Client replication reconciliation test spawns one matching and one other preview plus a replicated object; after running system, matching preview is
  gone, other remains, pending list empty.

```rust
// crates/client/tests/plugin.rs:93-154
fn replicated_object_reconciles_matching_preview_only() {
    ...
    ui.placement.pending.push(PendingWorldObjectPlacement {
        sequence: 7,
        object_id: matching_id.clone(),
        base_position: Vec3::ZERO,
        accepted_final_position: Some(Vec3::new(1.0, 2.0, 3.0)),
    });
    let matched_preview = app.world_mut().spawn((WorldObjectPlacementPreview { sequence: Some(7), object_id: matching_id.clone() }, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)))).id();
    let other_preview = app.world_mut().spawn((WorldObjectPlacementPreview { sequence: Some(8), object_id: other_id }, Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)))).id();
    app.world_mut().spawn((matching_id, Position(Vec3::new(1.0, 2.0, 3.0).into()), Replicated { receiver }));

    app.world_mut().run_system_once(reconcile_placement_preview_on_replication).expect("reconciliation system should run");
    app.update();

    assert!(app.world().get_entity(matched_preview).is_err());
    assert!(app.world().get_entity(other_preview).is_ok());
    assert!(app.world().resource::<SpawnPanelUi>().placement.pending.is_empty());
}
```

- Voxel-map persistence tests include chunk entity position-kind/persisted-components shape.

```rust
// crates/voxel_map_engine/src/persistence/mod.rs:225-240
fn chunk_entities_preserve_final_position_kind_without_persisted_components() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsChunkEntitiesStore { map_dir: dir.path().to_path_buf() };
    let spawns = vec![WorldObjectSpawn {
        object_id: "tree".to_string(),
        position: Vec3::new(1.0, 2.0, 3.0),
        position_kind: WorldObjectPositionKind::Final,
        persisted_components: Vec::new(),
    }];
    store.save(&IVec3::ZERO, &spawns).unwrap();
    let loaded = store.load(&IVec3::ZERO).unwrap().unwrap();
    assert_eq!(loaded[0].position_kind, WorldObjectPositionKind::Final);
    assert!(loaded[0].persisted_components.is_empty());
}
```

## Cross-Cutting Observations

- Reflected component data is the core data path: `.object.ron` assets, terrain defs, dev free-form spawning, server transformations, and client
  replication all converge on `AppTypeRegistry` + `ReflectComponent` insertion/removal.
- Server authority boundary: client can only request placement by `WorldObjectId` + base position; server resolves controlled map, validates loaded
  chunk, applies `PlacementOffset`, spawns replicated entity, and separately persists chunk entity data.
- Client visuals are derived, not authoritative: replicated entity identity/components arrive from Lightyear, then client locally attaches
  `Transform`, mesh child, material, and collider from `WorldObjectDefRegistry`/`VoxModelRegistry`.
- Persistence is chunk-oriented: `ChunkEntityRef` connects a spawned entity to `(map_entity, chunk_pos)`; persisted file payload uses
  `WorldObjectSpawn`, not stable ECS entity IDs.
- Expected missing state is consistently handled with `trace!` early returns in UI, targeting, and lifecycle paths.

## Open Areas

- Subagents were spawned per process requirement, but their returned outputs were unusable; all evidence above was verified directly from code.
- No direct code path found for authoritative world-object removal/deletion requests or entity picking of world-object entities. Adjacent existing
  paths are chunk eviction despawn, stale client despawn, preview despawn, visual child despawn/rebuild, death-effect transform, and voxel removal
  input.
- No direct test found for `on_death_effects`, `tick_active_transformations`, `restore_persisted` with `ActiveTransformation`, or full client-server
  Lightyear placement roundtrip.
