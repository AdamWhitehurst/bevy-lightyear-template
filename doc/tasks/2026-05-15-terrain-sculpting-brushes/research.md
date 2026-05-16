# Research Findings

## Q1: How does the Dev plugin structure editing modes, panel state, and panel modules, and where is terrain-specific UI or input currently routed relative to world-object placement and selection?

**Direct answer:** `DevPlugin` owns the shared `EditingMode` resource and panel toggles; the spawn panel renders the Terrain tab but terrain input is routed in `client/src/map.rs` through `handle_voxel_input` only when `EditingMode::Terrain` is active.

### Evidence

- Dev state has four modes; `Terrain` is default.

```rust
// crates/dev/src/state.rs:5-13
/// Active dev editing mode used to route terrain and world-object input.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditingMode {
    #[default]
    Terrain,
    PlaceDefinition,
    PlaceFreeForm,
    SelectEdit,
}
```

- `DevPlugin` initializes state and conditionally registers panel plugins.

```rust
// crates/dev/src/lib.rs:17-38
impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsDebugPlugin::default())
            .init_resource::<DevInspectorState>()
            .init_resource::<EditingMode>()
            .add_systems(Startup, hide_physics_debug)
            .add_systems(Update, (toggle_physics_debug, toggle_dev_inspector));
...
            #[cfg(feature = "spawn-panel")]
            app.add_plugins(panels::spawn::SpawnPanelPlugin);
```

- Spawn panel owns object placement/selection UI and selects the shared editing mode.

```rust
// crates/dev/src/panels/spawn.rs:1-6
//! Spawn panel. Selects the active `EditingMode` for terrain editing, authoritative
//! definition-driven world-object placement, free-form client-local spawning, and
//! existing world-object selection/editing.
//!
//! Free-form spawns are client-local (no `Replicate`) at the world origin and
//! carry a `DevSpawned` marker.
```

- Terrain panel body is currently empty.

```rust
// crates/dev/src/panels/spawn.rs:227-229
fn draw_terrain_tab(ui: &mut egui::Ui) {
    draw_section(ui, "TERRAIN", |_| {});
}
```

- Client input systems are gated by the selected editing mode.

```rust
// crates/client/src/map.rs:165-185
.add_systems(
    PostUpdate,
    (
        handle_voxel_input.run_if(in_editing_mode(EditingMode::Terrain)),
        #[cfg(feature = "spawn-panel")]
        update_world_object_nearby_selection
            .run_if(in_editing_mode(EditingMode::SelectEdit)),
...
        #[cfg(feature = "spawn-panel")]
        handle_world_object_placement_input
            .run_if(in_editing_mode(EditingMode::PlaceDefinition)),
```

## Q2: What conventions exist for keeping Dev plugin panels split across files, registering their resources/systems, and exposing panel state through `lib.rs` or `mod.rs`?

**Direct answer:** Panel modules live under `crates/dev/src/panels/`, are feature-gated in `panels/mod.rs`, register their own resources/systems in a panel plugin, and shared state types are re-exported from `dev/src/lib.rs`.

### Evidence

- Public exports and feature-gated panels.

```rust
// crates/dev/src/lib.rs:8-12
mod state;
pub use state::{DevInspectorState, EditingMode, PanelFlags};

#[cfg(feature = "inspector")]
pub mod panels;
```

- Panel module split convention.

```rust
// crates/dev/src/panels/mod.rs:1-8
//! Per-panel modules. Each is gated by its own Cargo feature so disabled
//! panels pay zero compile + zero runtime cost.

#[cfg(feature = "world-inspector")]
pub mod world_inspector;

#[cfg(feature = "spawn-panel")]
pub mod spawn;
```

- Panel plugin registers resource plus toggle/draw systems.

```rust
// crates/dev/src/panels/spawn.rs:151-162
pub struct SpawnPanelPlugin;

impl Plugin for SpawnPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnPanelUi>()
            .init_resource::<EditingMode>()
            .add_systems(Update, toggle_spawn_panel)
            .add_systems(
                EguiPrimaryContextPass,
                draw_spawn_panel.run_if(spawn_panel_enabled),
            );
    }
}
```

## Q3: How do client-side systems currently create, send, acknowledge, reject, and locally track voxel edit requests, including sequence numbers and pending state?

**Direct answer:** Client terrain input raycasts, computes a single target voxel, locally applies prediction, stores a `VoxelPrediction { sequence, position, old_voxel, new_voxel }`, sends `VoxelEditRequest`, clears pending entries on ack `<= sequence`, and on reject writes the server-provided voxel and removes the rejected sequence.

### Evidence

- Pending prediction state.

```rust
// crates/client/src/map.rs:105-118
/// Tracks pending predictions for block edits awaiting server acknowledgment.
#[derive(Resource, Default)]
pub struct VoxelPredictionState {
    pub next_sequence: u32,
    pub pending: Vec<VoxelPrediction>,
}

/// A single pending block edit prediction awaiting server acknowledgment.
pub struct VoxelPrediction {
    pub sequence: u32,
    pub position: IVec3,
    pub old_voxel: VoxelType,
    pub new_voxel: VoxelType,
}
```

- Sequence allocation.

```rust
// crates/client/src/map.rs:120-126
impl VoxelPredictionState {
    /// Returns the next sequence number, incrementing the counter.
    pub fn next(&mut self) -> u32 {
        let seq = self.next_sequence;
        self.next_sequence += 1;
        seq
    }
}
```

- Request creation, local prediction, and send.

```rust
// crates/client/src/map.rs:401-430
let (position, voxel) = if removing {
    (hit.position, VoxelType::Air)
} else if let Some(normal) = hit.normal {
    (hit.position + normal.as_ivec3(), VoxelType::Solid(0))
} else {
    trace!("handle_voxel_input: place hit has no normal");
    return;
};

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
```

- Ack clears all predictions up to acknowledged sequence.

```rust
// crates/client/src/map.rs:1568-1583
/// Processes server acknowledgments, clearing confirmed predictions.
fn handle_voxel_edit_ack(
    mut receivers: Query<&mut MessageReceiver<VoxelEditAck>>,
    mut prediction_state: ResMut<VoxelPredictionState>,
) {
...
            prediction_state
                .pending
                .retain(|p| p.sequence > ack.sequence);
```

- Reject applies `correct_voxel` and removes only that sequence.

```rust
// crates/client/src/map.rs:1587-1613
/// Processes server rejections, rolling back the predicted voxel to the correct value.
fn handle_voxel_edit_reject(
...
            voxel_world.set_voxel(
                chunk_ticket.map_entity,
                reject.position,
                WorldVoxel::from(reject.correct_voxel),
            );
            prediction_state
                .pending
                .retain(|p| p.sequence != reject.sequence);
```

## Q4: How does the server resolve a client's active map, validate voxel edits, apply them to `VoxelWorld`, mark persistence state dirty, and queue broadcasts?

**Direct answer:** The server resolves the controlled character's `MapInstanceId`, looks up its map entity in `MapRegistry`, currently validates all voxel edits as true, applies via `VoxelWorld::set_voxel`, updates `WorldDirtyState`, sends ack, and queues `PendingVoxelEdit` grouped by chunk.

### Evidence

- Active map resolution follows `ControlledBy.owner` on character entities.

```rust
// crates/server/src/map.rs:683-693
pub(crate) fn resolve_player_map(
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

- Validation/reject path.

```rust
// crates/server/src/map.rs:695-715
fn is_edit_valid(
    request: &VoxelEditRequest,
...
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
```

- Current validation is a stub accepting all edits.

```rust
// crates/server/src/map.rs:1407-1415
/// Validates a voxel edit request. Returns false if the edit should be rejected.
fn validate_voxel_edit(
    _request: &VoxelEditRequest,
    _map_entity: Entity,
    _voxel_world: &VoxelWorld,
) -> bool {
    // TODO: Add validation rules as needed (bounds, range, anti-cheat)
    true
}
```

- Apply marks dirty and timestamps dirty window.

```rust
// crates/server/src/map.rs:717-736
fn apply_voxel_edit(
    request: &VoxelEditRequest,
    map_entity: Entity,
    voxel_world: &mut VoxelWorld,
    dirty_state: &mut WorldDirtyState,
    time: &Time,
) {
    voxel_world.set_voxel(
        map_entity,
        request.position,
        WorldVoxel::from(request.voxel),
    );
    let now = time.elapsed_secs_f64();
    if !dirty_state.is_dirty {
        dirty_state.first_dirty_time = Some(now);
    }
    dirty_state.is_dirty = true;
    dirty_state.last_edit_time = now;
}
```

- Main request handler ties resolve/validate/apply/ack/queue together.

```rust
// crates/server/src/map.rs:1352-1402
pub fn handle_voxel_edit_requests(
...
    for (client_entity, mut receiver) in &mut receivers {
        for request in receiver.receive() {
            let Some((map_entity, player_map_id)) =
                resolve_player_map(client_entity, &controlled_query, &map_registry)
...
            apply_voxel_edit(
                &request,
                map_entity,
                &mut voxel_world,
                &mut dirty_state,
                &time,
            );
            send_edit_ack(client_entity, request.sequence, &mut ack_senders);
            let chunk_size = voxel_world
                .chunk_size(map_entity)
                .expect("map entity has VoxelMapInstance");
            queue_edit_broadcast(
                PendingVoxelEdit {
                    position: request.position,
                    voxel: request.voxel,
                    originator: client_entity,
                    map_id: player_map_id,
                },
```

## Q5: How do `VoxelWorld` and `VoxelMapInstance` locate chunk/local voxel coordinates, read existing voxel data, mutate voxel data, update boundary padding, mark chunks dirty, and schedule remeshing for single-voxel edits?

**Direct answer:** `VoxelWorld` is a `SystemParam` selecting a map entity, delegates `set_voxel` to `VoxelMapInstance`, which computes chunk/local/padded indices, writes `PalettedChunk`, marks `dirty_chunks` and `chunks_needing_remesh`, and updates neighbor padding/remesh for boundary voxels.

### Evidence

- `VoxelWorld` selects map entity per operation.

```rust
// crates/voxel_map_engine/src/api.rs:10-16
/// SystemParam for reading/writing voxels on any map instance.
///
/// Every operation takes a `map: Entity` parameter to select which map instance to operate on.
#[derive(SystemParam)]
pub struct VoxelWorld<'w, 's> {
    maps: Query<'w, 's, (&'static mut VoxelMapInstance, &'static VoxelGenerator)>,
}
```

- Read path: loaded chunk first, generator fallback.

```rust
// crates/voxel_map_engine/src/api.rs:22-42
pub fn get_voxel(&self, map: Entity, pos: IVec3) -> WorldVoxel {
    let Ok((instance, generator)) = self.maps.get(map) else {
        warn!("get_voxel: entity {map:?} has no VoxelMapInstance");
        return WorldVoxel::Unset;
    };

    let chunk_size = instance.chunk_size;
    let chunk_pos = voxel_to_chunk_pos(pos, chunk_size);
    if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
...
        return chunk_data.voxels.get(index);
    }

    evaluate_voxel_at(pos, generator, chunk_size, &instance.shape)
}
```

- World-to-chunk convention uses `div_euclid`, including negatives.

```rust
// crates/voxel_map_engine/src/api.rs:188-196
/// Converts a world-space voxel position to the chunk coordinate containing it.
pub fn voxel_to_chunk_pos(voxel_pos: IVec3, chunk_size: u32) -> IVec3 {
    let cs = chunk_size as i32;
    IVec3::new(
        voxel_pos.x.div_euclid(cs),
        voxel_pos.y.div_euclid(cs),
        voxel_pos.z.div_euclid(cs),
    )
}
```

- Single-voxel mutation and dirty/remesh marking.

```rust
// crates/voxel_map_engine/src/instance.rs:95-118
/// Mutate a voxel directly in the octree. Marks the chunk dirty and queues
/// it for async remesh. Also updates neighbor chunk padding for boundary voxels.
/// If the chunk is not loaded, the edit is silently dropped.
pub fn set_voxel(&mut self, world_pos: IVec3, voxel: WorldVoxel) {
    let chunk_pos = voxel_to_chunk_pos(world_pos, self.chunk_size);
    let local = world_pos - chunk_pos * self.chunk_size as i32;
...
    chunk_data.voxels.set(index, voxel);

    self.dirty_chunks.insert(chunk_pos);
    self.chunks_needing_remesh.insert(chunk_pos);

    self.update_neighbor_padding(chunk_pos, local, voxel);
}
```

- Boundary padding updates adjacent loaded chunks and marks neighbors for remesh.

```rust
// crates/voxel_map_engine/src/instance.rs:120-148
fn update_neighbor_padding(&mut self, chunk_pos: IVec3, local: IVec3, voxel: WorldVoxel) {
    let chunk_size = self.chunk_size as i32;
    for axis in 0..3 {
        let l = local[axis];
        if l == 0 {
            let mut neighbor = chunk_pos;
            neighbor[axis] -= 1;
...
            if let Some(nd) = self.get_chunk_data_mut(neighbor) {
                nd.voxels.set(idx, voxel);
            }
            self.chunks_needing_remesh.insert(neighbor);
        }
        if l == chunk_size - 1 {
```

## Q6: How do chunk lifecycle systems consume `chunks_needing_remesh`, generate replacement meshes, update spawned chunk entities, and persist dirty chunk data?

**Direct answer:** `VoxelPlugin` chains remesh spawn/poll after lifecycle systems; `spawn_remesh_tasks` drains eligible positions from `chunks_needing_remesh` into async `mesh_chunk_greedy` tasks, `poll_remesh_tasks` replaces/spawns/despawns chunk mesh entities, and server persistence periodically drains `dirty_chunks` into `PendingSaves`.

### Evidence

- Voxel lifecycle system order.

```rust
// crates/voxel_map_engine/src/lib.rs:45-58
app.add_systems(Startup, lifecycle::init_default_material);
app.add_systems(
    Update,
    (
        lifecycle::ensure_pending_chunks,
        lifecycle::collect_tickets,
        (lifecycle::update_chunks, lifecycle::poll_chunk_tasks).run_if(generation_enabled),
        lifecycle::reset_chunk_budgets.run_if(not(generation_enabled)),
        lifecycle::despawn_out_of_range_chunks,
        lifecycle::drain_pending_saves,
        lifecycle::spawn_remesh_tasks,
        lifecycle::poll_remesh_tasks,
    )
        .chain(),
);
```

- Remesh spawning consumes `chunks_needing_remesh` only after task creation/skips.

```rust
// crates/voxel_map_engine/src/lifecycle.rs:1040-1092
for &pos in instance.chunks_needing_remesh.iter() {
    let col = chunk_to_column(pos);
    heap.push(ChunkWork {
        position: pos,
        effective_level: 0,
        distance_to_source: propagator.min_distance_to_source(col),
    });
}
...
let voxels = {
    let _span = info_span!("expand_palette").entered();
    chunk_data.voxels.to_voxels()
};
let shape = instance.shape.clone();
let task = pool.spawn(async move { mesh_chunk_greedy(&voxels, &shape) });
pending.tasks.push(RemeshTask {
    chunk_pos: work.position,
    task,
});
tracker.remeshing.insert(work.position);
instance.chunks_needing_remesh.remove(&work.position);
```

- Remesh poll updates existing mesh, spawns new chunk entity, or despawns old entity.

```rust
// crates/voxel_map_engine/src/lifecycle.rs:1144-1181
let existing = chunk_query
    .iter()
    .find(|(_, vc, parent)| vc.position == remesh.chunk_pos && parent.0 == map_entity);

match (mesh_opt, existing) {
    (Some(mesh), Some((entity, _, _))) => {
        let handle = meshes.add(mesh);
        commands.entity(entity).insert(Mesh3d(handle));
    }
    (Some(mesh), None) => {
...
        let chunk_entity = commands
            .spawn((
                VoxelChunk {
                    position: remesh.chunk_pos,
                    lod_level: 0,
                },
...
    (None, Some((entity, _, _))) => {
        commands.entity(entity).despawn();
    }
```

- Server debounced persistence drains dirty chunks.

```rust
// crates/server/src/map.rs:393-409
/// Drain dirty chunks from an instance into the `PendingSaves` queue.
fn enqueue_dirty_chunks(instance: &mut VoxelMapInstance, pending_saves: &mut PendingSaves) {
    let chunk_size = instance.chunk_size;
    let dirty: Vec<IVec3> = instance.dirty_chunks.drain().collect();
    for chunk_pos in dirty {
        if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
            pending_saves.queue.push_back(lifecycle::PendingSave {
                position: chunk_pos,
                envelope: ChunkFileEnvelope {
                    version: CHUNK_SAVE_VERSION,
                    chunk_size,
                    data: chunk_data.clone(),
                },
            });
        }
    }
}
```

## Q7: How are voxel material IDs and terrain definitions represented across assets, `WorldVoxel`, `VoxelType`, meshing merge values, and any UI-visible registries?

**Direct answer:** Voxel material is currently an unlabelled `u8` carried in `WorldVoxel::Solid(u8)` and network `VoxelType::Solid(u8)`; terrain assets assign biome `surface_material`/`subsurface_material` u8s; the Terrain registry exposes loaded terrain defs by ID, but no UI-visible material registry was found.

### Evidence

- Runtime voxel representation.

```rust
// crates/voxel_map_engine/src/types.rs:6-12
/// Voxel data stored per position
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub enum WorldVoxel {
    Air,
    Unset,
    Solid(u8),
}
```

- Network representation omits `Unset`.

```rust
// crates/voxel_map_engine/src/types.rs:126-148
/// Network-serializable voxel type (mirrors WorldVoxel without Unset)
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Reflect)]
pub enum VoxelType {
    Air,
    Solid(u8),
}
...
impl From<WorldVoxel> for VoxelType {
    fn from(v: WorldVoxel) -> Self {
        match v {
            WorldVoxel::Air | WorldVoxel::Unset => VoxelType::Air,
            WorldVoxel::Solid(m) => VoxelType::Solid(m),
```

- Biome rules assign material IDs.

```rust
// crates/voxel_map_engine/src/terrain.rs:75-84
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

- Terrain generation writes `WorldVoxel::Solid(material)`.

```rust
// crates/voxel_map_engine/src/terrain.rs:247-255
if (world_y as f64) <= terrain_height {
    let material = pick_material(
        world_y,
        terrain_height,
        xz_index(px, pz, padded_size),
        moisture_cache.as_deref(),
        biome_rules,
    );
    voxels[i as usize] = WorldVoxel::Solid(material);
}
```

- Asset example uses material numbers 1,2,3.

```ron
// assets/terrain/overworld.terrain.ron:18-26
"voxel_map_engine::terrain::BiomeRules": ([
    (biome_id: "grassland", height_range: (-5.0, 5.0), moisture_range: (-0.3, 0.3),
     surface_material: 1, subsurface_material: 2, subsurface_depth: 3),
    (biome_id: "desert", height_range: (-5.0, 5.0), moisture_range: (-1.0, -0.3),
     surface_material: 3, subsurface_material: 3, subsurface_depth: 5),
```

- Terrain defs are registry entries by string ID, not material IDs.

```rust
// crates/protocol/src/terrain/registry.rs:7-20
/// All loaded terrain definitions, keyed by ID (e.g., "overworld", "homebase").
///
/// Populated during `AppState::Loading` via `TerrainPlugin` systems.
/// Available to both server and client after `AppState::Ready`.
#[derive(Resource, Clone, Debug)]
pub struct TerrainDefRegistry {
    pub terrains: HashMap<String, TerrainDef>,
}
```

## Q8: What semantics already exist in protocol/server code for sending multiple voxel changes from one update cycle, and how do they handle ordering, per-chunk grouping, originator exclusion, and cross-map room routing?

**Direct answer:** Multiple edits are grouped by `voxel_to_chunk_pos`; one edit sends `VoxelEditBroadcast`, 2+ edits in a chunk sends `SectionBlocksUpdate { chunk_pos, changes }`; the channel is ordered reliable; originators are excluded as a set; routing uses the first edit's `map_id` to find the Lightyear room.

### Evidence

- Protocol batch shape.

```rust
// crates/protocol/src/map/voxel.rs:41-47
/// Batched block changes for a single chunk, sent when 2+ changes happen in one tick.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct SectionBlocksUpdate {
    pub chunk_pos: IVec3,
    pub changes: Vec<(IVec3, VoxelType)>,
}
```

- Ordered reliable channel and message directions.

```rust
// crates/protocol/src/lib.rs:113-130
// Voxel channel
app.add_channel::<VoxelChannel>(ChannelSettings {
    mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
    ..default()
})
.add_direction(NetworkDirection::Bidirectional);

// Voxel messages
app.register_message::<VoxelEditRequest>()
    .add_direction(NetworkDirection::ClientToServer);
...
app.register_message::<SectionBlocksUpdate>()
    .add_direction(NetworkDirection::ServerToClient);
```

- Per-chunk grouping.

```rust
// crates/server/src/map.rs:751-759
/// Queues a voxel edit for batched broadcast.
fn queue_edit_broadcast(
    edit: PendingVoxelEdit,
    chunk_size: u32,
    pending: &mut PendingVoxelBroadcasts,
) {
    let chunk_pos = voxel_map_engine::prelude::voxel_to_chunk_pos(edit.position, chunk_size);
    pending.per_chunk.entry(chunk_pos).or_default().push(edit);
}
```

- Room routing and originator exclusion.

```rust
// crates/server/src/map.rs:1430-1450
for (chunk_pos, edits) in pending.per_chunk.drain() {
    let Some(first) = edits.first() else {
        continue;
    };
    let Some(&room_entity) = room_registry.0.get(&first.map_id) else {
        warn!("flush_voxel_broadcasts: no room for map {:?}", first.map_id);
        continue;
    };
...
    let originators: bevy::ecs::entity::EntityHashSet =
        edits.iter().map(|e| e.originator).collect();
    let targets: bevy::ecs::entity::EntityHashSet = room
        .clients
        .iter()
        .filter(|e| !originators.contains(*e))
        .copied()
        .collect();
```

- Single vs batched send.

```rust
// crates/server/src/map.rs:1452-1471
if edits.len() == 1 {
    let edit = &edits[0];
    sender
        .send_to_entities::<_, VoxelChannel>(
            &VoxelEditBroadcast {
                position: edit.position,
                voxel: edit.voxel,
            },
            &targets,
        )
        .ok();
} else {
    let changes: Vec<(IVec3, VoxelType)> =
        edits.iter().map(|e| (e.position, e.voxel)).collect();
    sender
        .send_to_entities::<_, VoxelChannel>(
            &SectionBlocksUpdate { chunk_pos, changes },
```

## Q9: What assumptions in tests and examples cover voxel editing, remeshing, map-instance isolation, boundary padding, and multi-change broadcasts, and where are the gaps for multi-voxel area/volume edits or cross-chunk edits?

**Direct answer:** Tests cover single-voxel dirty/remesh state, generator fallback, map instance isolation, client prediction filtering/reject bookkeeping, and broadcast grouping branches; no test found exercises one logical multi-voxel area/volume edit API, one request spanning multiple chunks, or cross-chunk server batching from a single logical operation.

### Evidence

- Single edit marks dirty and remesh.

```rust
// crates/voxel_map_engine/tests/api.rs:84-103
// Use VoxelWorld::set_voxel via a one-shot system
let edit_pos = IVec3::new(3, 5, 7);
app.world_mut()
    .run_system_once(move |mut vw: VoxelWorld| {
        vw.set_voxel(map, edit_pos, WorldVoxel::Solid(42));
    })
    .unwrap();

// Verify the edit is immediately visible in dirty_chunks
let instance = app.world().get::<VoxelMapInstance>(map).unwrap();
let chunk_pos = IVec3::ZERO; // edit_pos (3,5,7) is in chunk (0,0,0)
assert!(instance.dirty_chunks.contains(&chunk_pos));
assert!(instance.chunks_needing_remesh.contains(&chunk_pos));
```

- Map isolation.

```rust
// crates/voxel_map_engine/tests/api.rs:260-279
let edit_pos = IVec3::new(3, 5, 7);
app.world_mut()
    .run_system_once(move |mut vw: VoxelWorld| {
        vw.set_voxel(map_a, edit_pos, WorldVoxel::Solid(42));
    })
    .unwrap();

app.world_mut()
    .run_system_once(move |vw: VoxelWorld| {
        assert_eq!(
            vw.get_voxel(map_a, edit_pos),
            WorldVoxel::Solid(42),
            "map_a should have the written voxel"
        );
        assert_eq!(
            vw.get_voxel(map_b, edit_pos),
            WorldVoxel::Air,
```

- Multi-change server grouping tests are branch-level tests over `PendingVoxelBroadcasts`.

```rust
// crates/server/src/map.rs:1961-1970
#[test]
fn multiple_changes_in_same_chunk_takes_batched_path() {
    let mut pending = PendingVoxelBroadcasts::default();
    let entry = pending.per_chunk.entry(IVec3::ZERO).or_default();
    entry.push(make_edit(IVec3::new(1, 2, 3), VoxelType::Solid(1)));
    entry.push(make_edit(IVec3::new(4, 5, 6), VoxelType::Air));

    for (_, edits) in pending.per_chunk.drain() {
        assert_eq!(edits.len(), 2, "multi-edit should take batched update path");
    }
}
```

- Different chunks produce separate pending entries.

```rust
// crates/server/src/map.rs:1973-1995
#[test]
fn different_chunks_produce_separate_entries() {
    let mut pending = PendingVoxelBroadcasts::default();
...
    pending
        .per_chunk
        .entry(IVec3::ONE)
        .or_default()
        .push(make_edit(IVec3::new(17, 18, 19), VoxelType::Solid(2)));

    let chunks: Vec<_> = pending.per_chunk.drain().collect();
    assert_eq!(
        chunks.len(),
        2,
        "different chunks should produce separate entries"
```

## Q10: How could existing voxel mutation paths represent one logical edit over arbitrary world-space voxel positions, including edits that touch multiple chunks, without requiring callers to manage per-voxel chunk bookkeeping?

**Direct answer:** None — no current logical multi-voxel edit API exists; the nearest existing shape is repeated `VoxelWorld::set_voxel(map, pos, voxel)`, where each call computes chunk/local coordinates internally, and the server broadcast accumulator groups repeated `PendingVoxelEdit`s by chunk.

### Evidence

- Current public mutation API is single-position.

```rust
// crates/voxel_map_engine/src/api.rs:52-65
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
```

- Per-call bookkeeping is hidden inside `VoxelMapInstance::set_voxel`.

```rust
// crates/voxel_map_engine/src/instance.rs:98-115
pub fn set_voxel(&mut self, world_pos: IVec3, voxel: WorldVoxel) {
    let chunk_pos = voxel_to_chunk_pos(world_pos, self.chunk_size);
    let local = world_pos - chunk_pos * self.chunk_size as i32;
    let padded = [
        (local.x + 1) as u32,
        (local.y + 1) as u32,
        (local.z + 1) as u32,
    ];
    let index = self.shape.linearize(padded) as usize;
...
    self.dirty_chunks.insert(chunk_pos);
    self.chunks_needing_remesh.insert(chunk_pos);
```

- Existing grouping is in broadcast accumulation, not mutation input.

```rust
// crates/server/src/map.rs:97-100
/// Accumulates voxel edits per chunk during a tick for batching.
#[derive(Resource, Default)]
pub struct PendingVoxelBroadcasts {
    pub per_chunk: HashMap<IVec3, Vec<PendingVoxelEdit>>,
}
```

## Q11: How do raycast and cursor-to-world flows identify target voxels or adjacent placement positions, and what coordinate conventions distinguish changing an existing voxel from adding a voxel next to a hit surface?

**Direct answer:** Cursor position is converted to a `Ray3d`, raycast traverses integer voxel coordinates and returns the matching voxel plus entered-face normal; remove edits use `hit.position`, while placement uses `hit.position + normal.as_ivec3()`.

### Evidence

- Camera cursor to ray.

```rust
// crates/client/src/map.rs:1408-1424
fn camera_ray(
    camera_query: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: &Query<&Window, With<PrimaryWindow>>,
) -> Option<Ray3d> {
    let (camera, camera_transform) = camera_query.single().ok()?;
    let window = window_query.single().ok()?;
    let cursor_pos = window.cursor_position()?;
...
    camera
        .viewport_to_world(camera_transform, viewport_pos)
        .ok()
}
```

- Raycast result includes integer voxel position and optional normal.

```rust
// crates/voxel_map_engine/src/raycast.rs:32-39
/// Result of a voxel raycast.
#[derive(Clone, Debug)]
pub struct VoxelRaycastResult {
    pub position: IVec3,
    pub normal: Option<Vec3>,
    pub voxel: WorldVoxel,
    /// Normalized time along the ray [0, 1].
    pub t: f32,
}
```

- Existing terrain edit convention.

```rust
// crates/client/src/map.rs:394-405
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
```

- World-object placement uses the same adjacent voxel convention.

```rust
// crates/client/src/map.rs:86-100
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
    hit_normal,
})
```

## Q12: What undo/redo or reversible action patterns already exist in the codebase, and how do they store prior state, replay changes, and interact with authoritative server state?

**Direct answer:** None for general undo/redo or reversible action stacks; adjacent reversible-like patterns are client voxel prediction storing `old_voxel/new_voxel` and authoritative reject rollback, plus world-object pending previews reconciled by ack/replication or removed on reject.

### Evidence

- Voxel predictions store prior and new state.

```rust
// crates/client/src/map.rs:112-118
/// A single pending block edit prediction awaiting server acknowledgment.
pub struct VoxelPrediction {
    pub sequence: u32,
    pub position: IVec3,
    pub old_voxel: VoxelType,
    pub new_voxel: VoxelType,
}
```

- Reject path uses authoritative `correct_voxel`, not replay of local history.

```rust
// crates/protocol/src/map/voxel.rs:32-39
/// Server rejects a block edit — client must roll back.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Reflect, Message)]
#[type_path = "protocol::map"]
pub struct VoxelEditReject {
    pub sequence: u32,
    pub position: IVec3,
    pub correct_voxel: VoxelType,
}
```

- World-object panel has pending delete/move/rotation records.

```rust
// crates/dev/src/panels/spawn.rs:84-117
/// A pending authoritative world-object delete request.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingWorldObjectDelete {
    pub sequence: u32,
    pub target: Entity,
    pub accepted: bool,
}
...
pub struct PendingWorldObjectMove {
    pub sequence: u32,
    pub target: Entity,
    pub final_position: Vec3,
    pub old_chunk_pos: Option<IVec3>,
    pub new_chunk_pos: Option<IVec3>,
    pub accepted: bool,
}
```

- Rejection removes pending entries and records reason.

```rust
// crates/client/src/map.rs:1544-1564
fn handle_world_object_edit_reject(
    mut receivers: Query<&mut MessageReceiver<WorldObjectEditReject>>,
    mut ui_state: ResMut<SpawnPanelUi>,
) {
...
            ui_state
                .selection
                .pending_deletes
                .retain(|pending| pending.sequence != reject.sequence);
...
            ui_state.selection.last_reject = Some(reject.reason);
```

## Q13: How does this project separate authoritative server edits from client-local preview or dev tooling state, especially for admin overworld editing versus home-base editing?

**Direct answer:** Authoritative edits are server-mediated by map/room/controlled-character state; client-local dev previews are spawned without replication and reconciled against authoritative replication; map separation uses replicated `MapInstanceId` plus side-local `MapRegistry`, with Overworld/Homebase as semantic IDs.

### Evidence

- Semantic map IDs distinguish Overworld and per-owner Homebase.

```rust
// crates/protocol/src/map/types.rs:9-17
/// Identifies which map instance an entity belongs to.
/// Semantic enum — safe to replicate, no Entity references.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, Reflect)]
#[type_path = "protocol::map"]
#[require(ActiveCollisionHooks::FILTER_PAIRS)]
pub enum MapInstanceId {
    Overworld,
    Homebase { owner: NostrPublicKey },
}
```

- Entity map lookup is side-local.

```rust
// crates/protocol/src/map/types.rs:19-34
/// Maps semantic `MapInstanceId` to local `VoxelMapInstance` entities.
/// Each side (server/client) maintains independently.
#[derive(Resource, Default)]
pub struct MapRegistry(pub HashMap<MapInstanceId, Entity>);

impl MapRegistry {
    pub fn get(&self, id: &MapInstanceId) -> Entity {
        *self
            .0
            .get(id)
```

- Dev free-form state is explicitly client-local.

```rust
// crates/dev/src/panels/spawn.rs:1-6
//! Spawn panel. Selects the active `EditingMode` for terrain editing, authoritative
//! definition-driven world-object placement, free-form client-local spawning, and
//! existing world-object selection/editing.
//!
//! Free-form spawns are client-local (no `Replicate`) at the world origin and
//! carry a `DevSpawned` marker.
```

- Placement/edit previews are local-only marker components.

```rust
// crates/client/src/map.rs:55-61
#[cfg(feature = "spawn-panel")]
/// Marker for local-only world-object edit preview entities.
#[derive(Component)]
pub struct WorldObjectEditPreview {
    pub sequence: Option<u32>,
    pub target: Entity,
    pub object_id: WorldObjectId,
}
```

- Server chunk pushing is scoped by player's current `ChunkTicket.map_entity` and resets per tracked map.

```rust
// crates/server/src/map.rs:1505-1513
for (ticket, controlled_by, pos, mut visibility) in &mut player_query {
    if visibility.tracked_map != Some(ticket.map_entity) {
        visibility.sent_chunks.clear();
        visibility.sent_columns.clear();
        visibility.tracked_map = Some(ticket.map_entity);
    }

    let Ok((instance, dimensions, map_id)) = map_query.get(ticket.map_entity) else {
```

- Rooms are distinct per `MapInstanceId`.

```rust
// crates/server/tests/map_transition.rs:20-28
.run_system_once(
    |mut registry: ResMut<RoomRegistry>, mut commands: Commands| {
        let ow = registry.get_or_create(&MapInstanceId::Overworld, &mut commands);
        let hb = registry
            .get_or_create(&MapInstanceId::Homebase { owner: owner(42) }, &mut commands);
        assert_ne!(ow, hb, "Different maps should have different rooms");

        let ow2 = registry.get_or_create(&MapInstanceId::Overworld, &mut commands);
```

## Cross-Cutting Observations

- **Single-position mutation is the core primitive.** `VoxelEditRequest`, `VoxelEditBroadcast`, `VoxelWorld::set_voxel`, `VoxelMapInstance::set_voxel`, and client prediction all carry one `IVec3` and one voxel value.
- **Map instance identity is always explicit at system boundaries.** Client/server code carries `MapInstanceId`; voxel APIs take a map `Entity`; `MapRegistry` bridges the two locally.
- **Client prediction avoids duplicate application by position.** Single broadcasts and section updates both skip positions that match any pending prediction.
- **Batching currently exists only after server-side acceptance.** There is no batched client request type; `SectionBlocksUpdate` is server-to-client only.
- **Terrain material IDs are numeric with no discovered label registry.** Assets and generation carry `u8` material IDs, and UI currently hardcodes placement material `Solid(0)`.

## Open Areas

- Subagent execution did not return substantive findings; this document is based on direct code inspection.
- No external web concept research was included because all questions could be grounded in adjacent in-repo patterns; no current multi-voxel edit or undo/redo abstraction was found.
- `grep` searches did not find a UI-visible voxel material registry; only `TerrainDefRegistry` and biome material fields were found.
- Tests for boundary padding exist in `crates/voxel_map_engine/src/instance.rs`, but this pass did not enumerate each unit test body in detail.
