# Research Findings

## Q1: How does the current filesystem persistence flow handle map metadata, terrain chunks, per-chunk entity spawns, and map-level saved entities from startup/load through edit/save, including `None` versus authoritative-empty semantics?

**Direct answer:** Filesystem persistence is split into four stores: map metadata (`map.meta.bin`), chunk terrain (`terrain/chunk_*.bin`), per-chunk world-object spawns (`entities/chunk_*.entities.bin`), and map-level entities (`entities.bin`); missing files return `None`, per-chunk empty entity files are authoritative `Some(vec![])`, while map-level empty entity files are loaded as `None`.

### Evidence

- Map metadata schema and save directory routing:

```rust
// crates/server/src/persistence/mod.rs:13-20
/// Metadata for a single map instance, saved to `map.meta.bin`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MapMeta {
    pub version: u32,
    pub seed: u64,
    pub generation_version: u32,
    pub spawn_points: Vec<Vec3>,
}
```

- Overworld startup creates all four backend components and begins in `AwaitingMeta`:

```rust
// crates/server/src/map.rs:136-158
let map = commands
    .spawn((
        MapInstanceId::Overworld,
        protocol::map::Owner(owner),
        MapLoadState::AwaitingMeta,
        Transform::default(),
        StoreBackend::new(FsMapMetaStore { map_dir: map_dir.clone() }),
        PendingStoreOps::<(), MapMeta>::default(),
        StoreBackend::new(FsMapEntitiesStore { map_dir: map_dir.clone() }),
        PendingStoreOps::<(), Vec<SavedEntity>>::default(),
        StoreBackend::new(FsChunkEntitiesStore { map_dir: map_dir.clone() }),
        PendingStoreOps::<IVec3, Vec<WorldObjectSpawn>>::default(),
        StoreBackend::new(FsChunkStore { map_dir: map_dir.clone() }),
        PendingStoreOps::<IVec3, ChunkFileEnvelope>::default(),
    ))
```

- Metadata missing/error behavior: `None` uses default seed; errors warn and also use defaults:

```rust
// crates/server/src/map.rs:190-229
if let Some((_, meta_opt)) = ops.completed_loads.pop() {
    let (seed, gen_version) = match meta_opt {
        Some(meta) => (meta.seed, meta.generation_version),
        None => (DEFAULT_OVERWORLD_SEED, GENERATION_VERSION),
    };
    configure_map_from_meta(...);
    *state = MapLoadState::AwaitingEntities;
}
for (_, e) in ops.load_errors.drain(..) {
    warn!("Failed to load map meta: {e}, using defaults");
    configure_map_from_meta(... DEFAULT_OVERWORLD_SEED, GENERATION_VERSION, ...);
    *state = MapLoadState::AwaitingEntities;
}
```

- Dirty save writes chunks, metadata, and map-level entities after debounce; map-level entities save only if `by_map.get(map_id)` exists:

```rust
// crates/server/src/map.rs:385-402
enqueue_dirty_chunks(&mut instance, &mut pending_saves);
let meta = MapMeta {
    version: 1,
    seed: config.seed,
    generation_version: config.generation_version,
    spawn_points,
};
meta_ops.spawn_save(&meta_store.0, (), meta);

if let Some(entities) = by_map.get(map_id) {
    entity_ops.spawn_save(&entity_store.0, (), entities.clone());
}
```

- Dirty chunks are drained into versioned terrain envelopes:

```rust
// crates/server/src/map.rs:409-424
fn enqueue_dirty_chunks(instance: &mut VoxelMapInstance, pending_saves: &mut PendingSaves) {
    let chunk_size = instance.chunk_size;
    let dirty: Vec<IVec3> = instance.dirty_chunks.drain().collect();
    for chunk_pos in dirty {
        if let Some(chunk_data) = instance.get_chunk_data(chunk_pos) {
            pending_saves.queue.push_back(lifecycle::PendingSave {
                position: chunk_pos,
                envelope: ChunkFileEnvelope { version: CHUNK_SAVE_VERSION, chunk_size, data: chunk_data.clone() },
            });
        }
    }
}
```

- Chunk terrain file format/path:

```rust
// crates/voxel_map_engine/src/persistence/mod.rs:12-31
pub const CHUNK_SAVE_VERSION: u32 = 4;
pub struct ChunkFileEnvelope {
    pub version: u32,
    pub chunk_size: u32,
    pub data: ChunkData,
}
pub fn chunk_file_path(map_dir: &Path, chunk_pos: IVec3) -> PathBuf {
    map_dir.join("terrain").join(format!("chunk_{}_{}_{}.bin", chunk_pos.x, chunk_pos.y, chunk_pos.z))
}
```

- Per-chunk entity file format/path:

```rust
// crates/voxel_map_engine/src/persistence/mod.rs:46-58
pub(crate) const ENTITY_SAVE_VERSION: u32 = 3;
pub(crate) struct EntityFileEnvelope {
    pub version: u32,
    pub spawns: Vec<WorldObjectSpawn>,
}
pub fn entity_file_path(map_dir: &Path, chunk_pos: IVec3) -> PathBuf {
    map_dir.join("entities").join(format!("chunk_{}_{}_{}.entities.bin", chunk_pos.x, chunk_pos.y, chunk_pos.z))
```

- Missing per-chunk entities return `None`; existing empty entity file returns `Some(empty)`:

```rust
// crates/voxel_map_engine/src/persistence/fs_chunk_entities.rs:52-70
fn load(&self, key: &IVec3) -> Result<Option<Vec<WorldObjectSpawn>>, PersistenceError> {
    let path = entity_file_path(&self.map_dir, *key);
    if !path.exists() { return Ok(None); }
    ...
    let envelope: EntityFileEnvelope = bincode::deserialize(&bytes)?;
    if envelope.version != ENTITY_SAVE_VERSION { ... }
```

```rust
// crates/voxel_map_engine/src/persistence/mod.rs:276-282
fn empty_entities_file_is_authoritative_empty() {
    let store = test_entity_store(dir.path());
    store.save(&IVec3::ZERO, &Vec::new()).unwrap();
    let loaded = store.load(&IVec3::ZERO).unwrap();
    assert!(matches!(loaded, Some(spawns) if spawns.is_empty()));
}
```

- Map-level entities invert empty to `None`:

```rust
// crates/server/src/persistence/fs_map_entities.rs:35-55
fn load(&self, _key: &()) -> Result<Option<Vec<SavedEntity>>, PersistenceError> {
    let path = self.map_dir.join("entities.bin");
    if !path.exists() { return Ok(None); }
    ...
    if envelope.entities.is_empty() {
        Ok(None)
    } else {
        Ok(Some(envelope.entities))
    }
}
```

## Q2: How are map instances, owners, and map types represented across `protocol`, server map spawning, client map registries, and save directory layout, and where are overworld and homebase behavior already differentiated?

**Direct answer:** Map identity is `MapInstanceId::{Overworld, Homebase { owner: NostrPublicKey }}`, ownership is a separate `Owner(NostrPublicKey)` component, and both server/client maintain local `MapRegistry`; overworld and homebase differ in save directory, terrain def name, bounds/dimensions, seed derivation, and spawn path.

### Evidence

- Protocol map types:

```rust
// crates/protocol/src/map/types.rs:14-22
pub enum MapInstanceId {
    Overworld,
    Homebase { owner: NostrPublicKey },
}
#[derive(Resource, Default)]
pub struct MapRegistry(pub HashMap<MapInstanceId, Entity>);
```

- Owner component:

```rust
// crates/protocol/src/map/mod.rs:23-25
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[type_path = "protocol::map"]
pub struct Owner(pub NostrPublicKey);
```

- Save directory layout:

```rust
// crates/server/src/persistence/mod.rs:32-40
pub fn map_save_dir(base: &Path, map_id: &MapInstanceId) -> PathBuf {
    match map_id {
        MapInstanceId::Overworld => base.join("overworld"),
        MapInstanceId::Homebase { owner } => base.join(format!(
            "homebase_{}", nostr_client::npub_from_nostr_public_key(*owner)
        )),
    }
}
```

- Homebase spawn uses owner, `homebase` terrain def, finite bounds-derived spawning distance, and loads map entities synchronously:

```rust
// crates/server/src/map.rs:2157-2179
fn spawn_homebase(... owner: NostrPublicKey, ... map_id: &MapInstanceId, ...) -> (Entity, MapTransitionParams) {
    let map_dir = Arc::new(map_save_dir(&save_path.0, map_id));
    let seed = load_homebase_seed(&map_dir, owner);
    let terrain_def = terrain_registry.get("homebase").expect("homebase.terrain.ron must be loaded");
    let dimensions = terrain_def.map_dimensions().expect("homebase.terrain.ron must contain MapDimensions");
    let bounds = dimensions.bounds;
    let spawning_distance = bounds_to_spawning_distance(bounds.unwrap_or(IVec3::ONE));
```

- Client terrain type mapping:

```rust
// crates/client/src/transition.rs:158-162
fn terrain_def_name(map_id: &MapInstanceId) -> &'static str {
    match map_id {
        MapInstanceId::Overworld => "overworld",
        MapInstanceId::Homebase { .. } => "homebase",
    }
}
```

- Terrain assets differ: homebase is bounded with chunk size 32; overworld is unbounded with procedural terrain/placement rules.

```ron
// assets/terrain/homebase.terrain.ron:1-7
{
    "voxel_map_engine::config::MapDimensions": (
        chunk_size: 32,
        column_y_range: (-4, 4),
        tree_height: 3,
        bounds: Some((4, 4, 4)),
    ),
}
```

```ron
// assets/terrain/overworld.terrain.ron:1-8
{
    "voxel_map_engine::config::MapDimensions": (
        chunk_size: 64,
        column_y_range: (-2, 2),
        tree_height: 5,
        bounds: None,
    ),
```

## Q3: How does the server map lifecycle advance through `MapLoadState`, store-operation polling, chunk generation, entity spawning, and readiness today, and which systems assume a map is immediately usable once its entity exists?

**Direct answer:** Only overworld has an explicit `MapLoadState` (`AwaitingMeta -> AwaitingEntities -> Ready`); chunk generation begins after `VoxelGenerator` exists and engine helper components are auto-inserted, while several systems query `MapRegistry`/map components and `expect` them once a map entity is registered.

### Evidence

- Load state enum and polling registrations:

```rust
// crates/server/src/map.rs:120-124
pub enum MapLoadState {
    AwaitingMeta,
    AwaitingEntities,
    Ready,
}
```

```rust
// crates/server/src/map.rs:662-687
.add_systems(OnEnter(AppState::Ready), init_overworld_entity)
.add_systems(Update, (
    poll_map_meta.run_if(in_state(AppState::Ready)),
    poll_map_entities.run_if(in_state(AppState::Ready)),
    ...
    push_chunks_to_clients,
    save_dirty_chunks_debounced,
    handle_map_switch_requests.run_if(resource_exists::<TerrainDefRegistry>),
    crate::transition::complete_map_transition,
```

- Map entities become chunk-pipeline participants when they have `VoxelMapInstance + VoxelGenerator`:

```rust
// crates/voxel_map_engine/src/lifecycle.rs:282-315
for entity in &chunks_query { commands.entity(entity).insert(PendingChunks::default()); }
for entity in &remesh_query { commands.entity(entity).insert(PendingRemeshes::default()); }
for entity in &propagator_query { commands.entity(entity).insert(TicketLevelPropagator::default()); }
for entity in &budget_query { commands.entity(entity).insert(ChunkWorkBudget::default()); }
for entity in &gen_queue_query { commands.entity(entity).insert(GenQueue::default()); }
for entity in &pending_saves_query { commands.entity(entity).insert(PendingSaves::default()); }
for entity in &tracker_query { commands.entity(entity).insert(ChunkWorkTracker::default()); }
```

- Chunk update inserts loaded columns, drains generation queue, and passes optional stores:

```rust
// crates/voxel_map_engine/src/lifecycle.rs:369-405
for &col in &diff.unloaded { remove_column_chunks(&mut instance, &mut pending_saves, col, y_min, y_max); }
for &(col, level) in &diff.loaded {
    if is_column_within_bounds(col, dimensions.bounds) { instance.chunk_levels.insert(col, level); }
}
if config.generates_chunks {
    enqueue_new_chunks(...);
    let (entity_store, chunk_store) = store_query.get(map_entity)
        .map(|(es, cs)| (es.map(|sb| &sb.0), cs.map(|sb| &sb.0)))
        .unwrap_or((None, None));
    drain_gen_queue(..., entity_store, chunk_store);
}
```

- Disk load falls back to generation on missing/error; disk-loaded chunks are not marked dirty:

```rust
// crates/voxel_map_engine/src/generation.rs:60-91
match store.load(&pos) {
    Ok(Some(envelope)) => { ... return ChunkGenResult { position: pos, mesh, chunk_data: Some(chunk_data), entity_spawns, from_disk: true }; }
    Ok(None) => {}
    Err(e) => { bevy::log::warn!("Failed to load chunk at {pos}: {e}, regenerating"); }
}
generate_terrain(pos, &*generator)
```

```rust
// crates/voxel_map_engine/src/lifecycle.rs:893-900
let completed_status = if let Some(chunk_data) = result.chunk_data {
    let status = chunk_data.status;
    if !result.from_disk { instance.dirty_chunks.insert(result.position); }
    instance.insert_chunk_data(result.position, chunk_data);
    status
```

- Examples of immediate usability assumptions:

```rust
// crates/server/src/map.rs:2124-2127
if let Some(&entity) = registry.0.get(map_id) {
    let (config, dimensions) = map_params_query
        .get(entity)
        .expect("Existing map entity must have VoxelMapConfig + MapDimensions");
```

```rust
// crates/server/src/map.rs:834-836
let (instance, dimensions) = map_query
    .get(map_entity)
    .expect("resolved map entity must have VoxelMapInstance and MapDimensions");
```

## Q4: How does the map transition flow process client switch requests, create or find target maps, relocate/freeze players, send transition messages, and wait for client readiness, including all places that require seed, dimensions, or chunk data before progress continues?

**Direct answer:** Server maps `MapSwitchTarget` to `MapInstanceId`, creates/fetches target map and transition params, moves/freezes the player, sends `MapTransitionStart`; client spawns a local map and waits for chunk levels/remesh/colliders before `MapTransitionReady`, after which server unfreezes and sends `MapTransitionEnd`.

### Evidence

- Switch request resolution and duplicate/current guards:

```rust
// crates/server/src/map.rs:2046-2089
for request in receiver.receive() {
    let (player_entity, _controlled_by, current_map_id) = controlled_query.iter()
        .find(|(_, ctrl, _)| ctrl.owner == client_entity)
        .unwrap_or_else(|| panic!("No character entity found for client {client_entity:?} during map switch"));
    if pending.get(player_entity).is_ok() { warn!("Player {player_entity:?} already transitioning, ignoring request"); continue; }
    let identity = player_identities.get(client_entity).expect("Authenticated client must have PlayerIdentity before map switch");
    let target_map_id = resolve_switch_target(&request.target, identity.0);
    if *current_map_id == target_map_id { warn!("Player {player_entity:?} already on target map {target_map_id:?}"); continue; }
    crate::transition::start_map_transition(...);
}
```

- Server transition start removes sender from old room, freezes, ensures map, attaches `ChunkTicket`, and sends seed/dimensions/readiness params:

```rust
// crates/server/src/transition.rs:35-99
let old_room = room_registry.get_or_create(current_map_id, commands);
let new_room = room_registry.get_or_create(target_map_id, commands);
commands.trigger(RoomEvent { room: old_room, target: RoomTarget::RemoveSender(client_entity) });
let spawn_position = respawn_query.iter().find(|(_, mid)| *mid == target_map_id).map(|(pos, _)| pos.0).unwrap_or(crate::gameplay::DEFAULT_SPAWN_POS);
relocation::relocate_remove(commands, player_entity, old_room, target_map_id, Some(spawn_position));
commands.entity(player_entity).insert((DisableRollback, ColliderDisabled, RigidBodyDisabled, PendingTransition(target_map_id.clone())));
let (map_entity, params) = ensure_map_exists(...);
commands.entity(player_entity).insert(ChunkTicket::player(map_entity));
sender.send::<MapChannel>(MapTransitionStart { target: target_map_id.clone(), seed: params.seed, generation_version: params.generation_version, bounds: params.bounds, spawn_position, chunk_size: params.chunk_size, column_y_range: params.column_y_range, readiness_radius: TRANSITION_READINESS_RADIUS });
```

- Client handles start before chunk sync, spawns local map if absent, and starts state machine:

```rust
// crates/client/src/transition.rs:88-109
if !registry.0.contains_key(&transition.target) {
    let map_entity = spawn_map_from_transition(&mut commands, &transition, &terrain_registry);
    registry.insert(transition.target.clone(), map_entity);
}
let map_entity = registry.get(&transition.target);
if let Ok(player) = player_query.single() {
    commands.entity(player).insert(ChunkTicket::map_transition(map_entity));
}
transition_state.begin(&transition);
```

- Client transition scheduling explicitly prevents same-frame chunk-drop:

```rust
// crates/client/src/transition.rs:420-434
// Transition handler must flush before chunk sync runs so the
// newly spawned map entity is in the registry. Without this,
// ChunkDataSync arriving on the same frame as MapTransitionStart
// would be silently dropped (registry lookup fails) and the
// server never re-sends them.
app.add_systems(Update, (
    on_transition_start_received.run_if(resource_exists::<TerrainDefRegistry>),
    ApplyDeferred,
    (crate::map::handle_chunk_data_sync, crate::map::handle_unload_column, crate::map::attach_chunk_ticket_to_player, protocol::attach_chunk_colliders),
```

- Client readiness gates require registered map, `VoxelMapInstance`, `ChunkWorkTracker`, chunk levels, no remesh, children, and colliders:

```rust
// crates/client/src/transition.rs:303-374
let Some(&map_entity) = registry.0.get(target) else { return false; };
let Ok((instance, tracker, dimensions)) = instance_query.get(map_entity) else { return false; };
let Some(tracker) = tracker else { return false; };
...
if !instance.chunk_levels.contains_key(&col) { return false; }
...
if instance.chunks_needing_remesh.contains(&pos) || tracker.remeshing.contains(&pos) { return false; }
...
let Ok(children) = children_query.get(map_entity) else { return false; };
...
if dx <= radius && dz <= radius && !has_collider { return false; }
true
```

## Q5: How are voxel edits and world-object edits validated, acknowledged, replicated, marked dirty, and persisted, and what guarantees exist that client-visible state reflects server-authoritative applied changes?

**Direct answer:** Server validates map/request shape and loaded chunks for brush/concrete/world-object edits, applies authoritative state before ack/broadcast, marks voxel chunks dirty, and persists world objects per affected chunk; clients keep predictions until ack or roll back on reject, while replicated world-object changes/despawns arrive separately.

### Evidence

- Voxel concrete/brush validation and map matching:

```rust
// crates/server/src/map.rs:1741-1759
fn request_map_matches_player(request_map_id: &MapInstanceId, player_map_id: &MapInstanceId) -> bool { request_map_id == player_map_id }
fn validate_voxel_concrete_edit(request: &VoxelConcreteEditRequest) -> bool {
    !request.changes.is_empty() && request.changes.len() <= MAX_BRUSH_VOXELS
}
fn validate_voxel_brush_edit(request: &VoxelBrushEditRequest) -> bool {
    if request.width == 0 || request.height == 0 { return false; }
    brush_footprint(request.anchor, request.shape, request.width, request.height).len() <= MAX_BRUSH_VOXELS
}
```

- Applying accepted voxel changes marks dirty and updates timestamp:

```rust
// crates/server/src/map.rs:1681-1700
fn apply_voxel_changes(changes: &[VoxelChange], map_entity: Entity, voxel_world: &mut VoxelWorld, dirty_state: &mut WorldDirtyState, time: &Time) {
    voxel_world.set_voxels(map_entity, changes.iter().map(|change| (change.position, WorldVoxel::from(change.voxel))));
    let now = time.elapsed_secs_f64();
    if !dirty_state.is_dirty { dirty_state.first_dirty_time = Some(now); }
    dirty_state.is_dirty = true;
    dirty_state.last_edit_time = now;
}
```

- Ack and broadcast are sent after apply:

```rust
// crates/server/src/map.rs:1615-1642
apply_voxel_changes(&request.changes, map_entity, &mut voxel_world, &mut dirty_state, &time);
send_brush_edit_ack(client_entity, request.sequence, player_map_id.clone(), request.changes.clone(), &mut ack_senders);
...
for change in request.changes {
    queue_edit_broadcast(PendingVoxelEdit { position: change.position, voxel: change.voxel, originator: client_entity, map_id: player_map_id.clone() }, chunk_size, &mut pending_broadcasts);
}
```

- Broadcast excludes originators and sends single/batched changes to room clients:

```rust
// crates/server/src/map.rs:1797-1828
let originators: bevy::ecs::entity::EntityHashSet = edits.iter().map(|e| e.originator).collect();
let targets: bevy::ecs::entity::EntityHashSet = room.clients.iter().filter(|e| !originators.contains(*e)).copied().collect();
if edits.len() == 1 {
    sender.send_to_entities::<_, VoxelChannel>(&VoxelEditBroadcast { map_id: map_id.clone(), position: edit.position, voxel: edit.voxel }, &targets).ok();
} else {
    sender.send_to_entities::<_, VoxelChannel>(&SectionBlocksUpdate { map_id: map_id.clone(), chunk_pos, changes }, &targets).ok();
}
```

- Client skips broadcasts at positions with pending predictions; ack clears prediction; reject rolls back:

```rust
// crates/client/src/map.rs:400-419
let has_pending_prediction = has_pending_prediction_at(&prediction_state, &broadcast.map_id, broadcast.position);
if has_pending_prediction { continue; }
voxel_world.set_voxel(map_entity, broadcast.position, WorldVoxel::from(broadcast.voxel));
```

```rust
// crates/client/src/map.rs:2123-2135
let Some(index) = prediction_state.pending.iter().position(|p| p.sequence == ack.sequence && p.map_id == ack.map_id) else { continue; };
let prediction = prediction_state.pending.remove(index);
```

```rust
// crates/client/src/map.rs:2386-2399
if rollback_prediction(reject.sequence, &reject.map_id, &mut prediction_state, &mut voxel_world, map_entity) { continue; }
voxel_world.set_voxel(map_entity, reject.position, WorldVoxel::from(reject.correct_voxel));
```

- World-object placement validates known def, finite/bounds/loaded chunk, then spawns and acks:

```rust
// crates/server/src/map.rs:889-916
if !request.base_position.is_finite() { return Err(WorldObjectPlacementRejectReason::NonFinitePosition); }
let Some(def) = defs.get(&request.object_id) else { return Err(WorldObjectPlacementRejectReason::UnknownObject); };
let final_position = crate::world_object::final_placed_world_object_position(def, request.base_position);
if !placement_chunk_in_bounds(chunk_pos, dimensions) { return Err(WorldObjectPlacementRejectReason::OutOfBounds); }
if !instance.chunk_levels.contains_key(&column) || instance.get_chunk_data(chunk_pos).is_none() {
    return Err(WorldObjectPlacementRejectReason::ChunkUnavailable);
}
Ok((def, final_position, chunk_pos))
```

- Move/rotate validations enforce same player map and loaded chunks:

```rust
// crates/server/src/map.rs:1057-1083
let Ok((_id, object_map_id, chunk_ref)) = object_query.get(request.target) else { return Err(WorldObjectEditRejectReason::MissingTarget); };
if object_map_id != player_map_id || chunk_ref.map_entity != player_map_entity { return Err(WorldObjectEditRejectReason::ForeignMap); }
...
if !instance.chunk_levels.contains_key(&column) || instance.get_chunk_data(new_chunk_pos).is_none() {
    return Err(if new_chunk_pos == chunk_ref.chunk_pos { WorldObjectEditRejectReason::ChunkUnavailable } else { WorldObjectEditRejectReason::DestinationChunkUnavailable });
}
```

## Q6: What identity and signing infrastructure already exists in `nostr_client` and `protocol` for client identities, server identities, auth proofs, relay pool readiness, event publication, subscription, filtering, and error handling?

**Direct answer:** `protocol` defines public-key/auth proof resources/messages; `nostr_client` manages encrypted identities, server keys, Nostr-signed auth proofs, relay pool readiness via EOSE, and server announcement publish/subscribe/filtering with string/error enums.

### Evidence

- Protocol identity/auth types:

```rust
// crates/protocol/src/auth/mod.rs:4-33
pub struct NostrPublicKey(pub [u8; 32]);
impl NostrPublicKey { pub fn client_id_prefix(self) -> u64 { u64::from_le_bytes(self.0[0..8].try_into().expect("NostrPublicKey has 32 bytes")) } }
pub struct IdentityChallenge { pub nonce: [u8; 32], }
pub struct IdentityProof { pub pubkey: NostrPublicKey, pub signed_event_json: String, }
pub struct PlayerIdentity(pub NostrPublicKey);
```

- Client/server identity resources:

```rust
// crates/nostr_client/src/identity.rs:16-25
pub struct ClientIdentity {
    pub secret: SecretKey,
    pub public: PublicKey,
}
pub struct ServerIdentity {
    pub keys: Keys,
}
```

- Signed auth proof and verification:

```rust
// crates/nostr_client/src/auth.rs:8-24
pub fn build_identity_proof(identity: &ClientIdentity, nonce: [u8; 32]) -> Result<IdentityProof, String> {
    let keys = nostr_sdk::Keys::new(identity.secret.clone());
    let event = EventBuilder::new(Kind::Custom(NOSTR_KIND_AUTH), "")
        .tag(Tag::custom(TagKind::custom("challenge"), [hex::encode(nonce)]))
        .sign_with_keys(&keys)?;
    Ok(IdentityProof { pubkey: NostrPublicKey(*identity.public.as_bytes()), signed_event_json: event.as_json() })
}
```

```rust
// crates/nostr_client/src/auth.rs:27-58
pub fn verify_identity_proof(proof: &IdentityProof, expected_nonce: [u8; 32], expected_client_id: u64) -> Result<PlayerIdentity, String> {
    let event = Event::from_json(&proof.signed_event_json)?;
    if event.kind != Kind::Custom(NOSTR_KIND_AUTH) { return Err(...); }
    if !event.verify_signature() { return Err("identity proof signature verification failed".to_string()); }
    let event_pubkey = NostrPublicKey(*event.pubkey.as_bytes());
    if event_pubkey != proof.pubkey { return Err("identity proof pubkey does not match signed event pubkey".to_string()); }
    if !event_has_nonce(&event, expected_nonce) { return Err("identity proof nonce tag mismatch".to_string()); }
```

- Relay readiness waits for EOSE on announcement filter:

```rust
// crates/nostr_client/src/relay_pool.rs:49-70
let filter = Filter::new()
    .kind(Kind::Custom(NOSTR_KIND_SERVER_ANNOUNCEMENT))
    .identifier(SERVER_ANNOUNCEMENT_IDENTIFIER)
    .limit(1);
let subscription = match client.subscribe(filter, None).await { ... };
...
Ok(RelayPoolNotification::Message { relay_url, message: RelayMessage::EndOfStoredEvents(id), }) if id.as_ref() == &subscription_id => {
    let _ = ready_tx.send(()).await;
    break;
}
```

- Relay ready resource is polled into `RelayPoolReady`:

```rust
// crates/nostr_client/src/relay_pool.rs:88-99
pub fn poll_relay_pool_ready(mut ready: ResMut<RelayPoolReady>, pool: Option<Res<RelayPool>>) {
    let Some(pool) = pool else { trace!("poll_relay_pool_ready: RelayPool not inserted yet"); return; };
    while pool.ready_rx.try_recv().is_ok() {
        if !ready.0 { info!("Nostr relay pool reached EOSE on at least one relay"); }
        ready.0 = true;
    }
}
```

- Server announcements are signed/published and subscription-filtered:

```rust
// crates/nostr_client/src/announcement.rs:71-101
pub fn server_announcement_builder(announcement: &ServerAnnouncement) -> Result<EventBuilder, serde_json::Error> {
    let content = serde_json::to_string(announcement)?;
    let expiration = Timestamp::now() + Duration::from_secs(SERVER_ANNOUNCEMENT_TTL_SECS);
    Ok(EventBuilder::new(Kind::Custom(NOSTR_KIND_SERVER_ANNOUNCEMENT), content)
        .tag(Tag::identifier(SERVER_ANNOUNCEMENT_IDENTIFIER))
        .tag(Tag::expiration(expiration)))
}
pub async fn publish_server_announcement(client: Client, identity: ServerIdentity, announcement: ServerAnnouncement) -> Result<String, String> {
    let event = server_announcement_builder(&announcement)?.sign_with_keys(&identity.keys)?;
    let output = client.send_event(&event).await?;
    if output.success.is_empty() { return Err(format!("publish announcement: no relay accepted event {}; failures={:?}", event.id, output.failed)); }
```

## Q7: What persisted entity kinds, world-object definitions, component reflection paths, allowlists, bounds checks, quotas, and schema/version checks currently exist for accepting or rejecting loaded map data?

**Direct answer:** Map-level persistence only supports `SavedEntityKind::RespawnPoint`; chunk entities persist `WorldObjectSpawn` with object id/position/persisted components, restore only specific persisted component type paths, and validate runtime edits against known `WorldObjectDefRegistry`, map bounds, finite values, and loaded chunk availability; version checks exist in each store.

### Evidence

- Map-level saved entity kind is a single enum variant:

```rust
// crates/protocol/src/map/persistence.rs:6-15
pub struct MapSaveTarget;
pub enum SavedEntityKind {
    RespawnPoint,
}
pub struct SavedEntity {
    pub kind: SavedEntityKind,
    pub position: Vec3,
}
```

- `collect_entities_by_map` only saves respawn points and asserts on unknown `MapSaveTarget`:

```rust
// crates/server/src/map.rs:532-546
for (_marker, map_id, position, respawn) in entity_query.iter() {
    let kind = if respawn.is_some() { SavedEntityKind::RespawnPoint } else {
        debug_assert!(false, "Entity with MapSaveTarget has no recognized SavedEntityKind");
        continue;
    };
    by_map.entry(map_id.clone()).or_default().push(SavedEntity { kind, position: position.0 });
}
```

- Chunk entity persisted schema and protocol-free object IDs:

```rust
// crates/voxel_map_engine/src/config.rs:40-58
pub struct WorldObjectSpawn {
    pub object_id: String,
    pub position: Vec3,
    #[serde(default)]
    pub position_kind: WorldObjectPositionKind,
    #[serde(default)]
    pub persisted_components: Vec<PersistedComponent>,
}
/// A single persisted component: type path + RON data.
#[derive(Clone, Debug, Serialize, Deserialize)]
```

- World object definitions use reflect component map RON:

````rust
// crates/protocol/src/world_object/loader.rs:45-52
/// The RON format is a flat map of type paths to component data:
/// ```ron
/// {
///     "protocol::world_object::ObjectCategory": Scenery,
///     "protocol::world_object::VisualKind": Vox("models/trees/tree.vox"),
///     "protocol::Health": (current: 50.0, max: 50.0),
````

- Existing object asset example:

```ron
// assets/objects/tree_circle.object.ron:1-12
{
    "protocol::world_object::types::ObjectCategory": Scenery,
    "protocol::world_object::types::VisualKind": Vox("models/trees/tree_circle.vox"),
    "avian3d::collision::collider::constructor::ColliderConstructor": Cylinder(radius: 0.5, height: 3.0),
    "protocol::Health": (
        current: 50.0,
        max: 50.0,
    ),
    "protocol::RespawnTimerConfig": (duration_ticks: 384),
```

- Persisted components restored are explicitly recognized by type path:

```rust
// crates/server/src/chunk_entities.rs:430-459
fn serialize_persisted(active_transform: Option<&ActiveTransformation>, health: Option<&protocol::Health>, rotation: Option<&Rotation>) -> Vec<PersistedComponent> {
    let mut result = Vec::new();
    if let Some(at) = active_transform { result.push(PersistedComponent { type_path: std::any::type_name::<ActiveTransformation>().to_string(), ron_data, }); }
    if let Some(h) = health { result.push(PersistedComponent { type_path: std::any::type_name::<protocol::Health>().to_string(), ron_data, }); }
    if let Some(rotation) = rotation { result.push(PersistedComponent { type_path: std::any::type_name::<WorldObjectRotationSnapshot>().to_string(), ron_data, }); }
    result
}
```

- Loaded unknown generated object IDs are skipped, not fatal:

```rust
// crates/server/src/chunk_entities.rs:75-83
let id = WorldObjectId(spawn.object_id.clone());
let Some(def) = defs.get(&id) else {
    warn!("Unknown world object '{}' in placement rules", spawn.object_id);
    continue;
};
```

- Version mismatch variants in stores:

```rust
// git/bevy-persistence/src/store.rs:3-11
pub enum PersistenceError {
    Io(std::io::Error),
    Serialize(String),
    Deserialize(String),
    VersionMismatch { expected: u32, actual: u32 },
}
```

## Q8: How do chunk terrain files, chunk entity files, map metadata files, and map-level entity files encode versioning, dimensions, generation data, content completeness, and corruption/version-mismatch failures?

**Direct answer:** Terrain envelopes encode version + chunk size + chunk data; chunk entity envelopes encode version + spawn list; map meta encodes version + seed + generation version + spawn points; map-level entities encode version + entity list; all deserialize through bincode, terrain/chunk entities zstd-compressed, and version mismatches/corruption become `PersistenceError` load errors.

### Evidence

| File class | Path | Version field | Data fields | Missing | Empty semantics |
| --- | --- | --: | --- | --- | --- |
| Map meta | `map.meta.bin` | `MapMeta.version` | `seed`, `generation_version`, `spawn_points` | `Ok(None)` | N/A |
| Terrain chunk | `terrain/chunk_x_y_z.bin` | `ChunkFileEnvelope.version` | `chunk_size`, `ChunkData` | `Ok(None)` | `ChunkData.fill_type/status` |
| Chunk entities | `entities/chunk_x_y_z.entities.bin` | `EntityFileEnvelope.version` | `Vec<WorldObjectSpawn>` | `Ok(None)` | `Some(vec![])` authoritative empty |
| Map entities | `entities.bin` | `EntityFileEnvelope.version` | `Vec<SavedEntity>` | `Ok(None)` | `Ok(None)` if empty |

- Terrain save uses temp+zstd+rename and rejects version mismatch:

```rust
// crates/voxel_map_engine/src/persistence/fs_chunk.rs:26-42
let bytes = bincode::serialize(value)?;
let tmp_path = path.with_extension("bin.tmp");
let file = fs::File::create(&tmp_path)?;
let mut encoder = zstd::Encoder::new(file, ZSTD_COMPRESSION_LEVEL)?;
encoder.write_all(&bytes)?;
encoder.finish()?;
fs::rename(&tmp_path, &path)?;
```

```rust
// crates/voxel_map_engine/src/persistence/fs_chunk.rs:60-65
let envelope: ChunkFileEnvelope = bincode::deserialize(&bytes)?;
if envelope.version != CHUNK_SAVE_VERSION {
    return Err(PersistenceError::VersionMismatch { expected: CHUNK_SAVE_VERSION,
```

- Map meta version mismatch/corruption path:

```rust
// crates/server/src/persistence/fs_map_meta.rs:35-42
let bytes = fs::read(&path).map_err(|e| PersistenceError::Deserialize(format!("read meta: {e}")))?;
let meta: MapMeta = bincode::deserialize(&bytes).map_err(|e| PersistenceError::Deserialize(format!("deserialize meta: {e}")))?;
if meta.version != META_VERSION {
    return Err(PersistenceError::VersionMismatch { expected: META_VERSION, actual: meta.version });
```

- Store trait reserves `None` for absence and errors for invalid data:

```rust
// git/bevy-persistence/src/store.rs:49-53
/// Load the value for `key`.
///
/// Return `Ok(None)` when the key is absent. Reserve errors for failed IO,
/// invalid serialized data, version mismatches, or backend-specific failures.
fn load(&self, key: &K) -> Result<Option<V>, PersistenceError>;
```

## Q9: What tests currently cover map persistence, map transitions, filesystem fallback, identity ownership, voxel/world-object edits, and Nostr relay behavior, and what test utilities exist for simulating failures or alternate stores?

**Direct answer:** Tests cover filesystem store roundtrips/corruption/version mismatch, server world persistence, transition rooms/markers/homebase identity, client transition/chunk sync, voxel prediction ack/reject helpers, world-object placement/edit persistence, Nostr auth/identity/announcement parsing; no alternate persistence backend test store was found in the application crates, but temp dirs and hand-written corrupted files are used.

### Evidence

- Representative persistence tests:

```text
// rg output
crates/server/tests/voxel_persistence.rs:45:fn dirty_chunks_saved_on_debounce()
crates/server/tests/voxel_persistence.rs:67:fn clean_chunks_not_saved()
crates/server/tests/voxel_persistence.rs:88:fn terrain_persists_across_save_load()
crates/server/tests/voxel_persistence.rs:132:fn evicted_dirty_chunk_saved_before_removal()
crates/server/tests/voxel_persistence.rs:173:fn load_chunk_with_mismatched_chunk_size_errors()
crates/server/tests/world_persistence.rs:61:fn terrain_persists_across_server_restart()
crates/server/tests/world_persistence.rs:230:fn multiple_maps_save_independently()
crates/server/tests/world_persistence.rs:262:fn homebase_metadata_roundtrip()
crates/server/tests/world_persistence.rs:345:fn entities_persist_across_server_restart()
```

- Transition/identity ownership tests:

```text
// rg output
crates/server/tests/map_transition.rs:13:fn room_registry_creates_separate_rooms_for_different_maps()
crates/server/tests/map_transition.rs:89:fn pending_transition_marker_can_be_added_and_removed()
crates/server/tests/map_transition.rs:108:fn different_homebase_owners_produce_distinct_map_ids()
crates/client/tests/map_transition.rs:* (file exists; client transition coverage)
crates/ui/tests/map_transition_state.rs:43:fn transitioning_state_spawns_loading_screen()
```

- World-object edit tests enumerate validation and persistence cases:

```text
// rg output
crates/server/tests/world_object_edit.rs:70:fn delete_validation_accepts_loaded_world_object_on_player_map()
crates/server/tests/world_object_edit.rs:174:fn delete_save_writes_empty_chunk_file()
crates/server/tests/world_object_edit.rs:257:fn move_same_chunk_validation_accepts_loaded_target()
crates/server/tests/world_object_edit.rs:446:fn cross_chunk_move_saves_empty_source_and_destination_with_moved_object()
crates/server/tests/world_object_edit.rs:703:fn rotate_persists_through_chunk_entity_save_restore_payload()
```

- Nostr/auth tests exist in module tests:

```text
// rg output
crates/nostr_client/src/auth.rs:80:fn identity_proof_roundtrips()
crates/nostr_client/src/auth.rs:92:fn identity_proof_rejects_wrong_nonce()
crates/nostr_client/src/announcement.rs:271..419: multiple #[test] cases for builder/filter/parser/list behavior
crates/nostr_client/src/identity.rs:317..468: multiple #[test] cases for identity paths, encryption, profiles, load/save errors
```

- Failure simulation by corrupt file is direct filesystem write:

```rust
// crates/server/src/persistence/mod.rs:188-193
#[test]
fn corrupt_entities_file_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("entities.bin"), b"garbage data").unwrap();
    let store = test_entity_store(dir.path());
    assert!(store.load(&()).is_err());
}
```

## Q10: How are errors and degraded states surfaced today for persistence loads/saves, relay availability, identity readiness, and map transitions, and are there existing enums/resources/events suitable for stale, divergent, missing, or unavailable state?

**Direct answer:** Persistence errors are drained from `PendingStoreOps` into logs or fallback behavior; relay/identity readiness is boolean resources gating `AppState::Ready`; transitions use marker components/resources and trace/warn logs, but no explicit stale/divergent/missing/unavailable map-state enum was found beyond existing reject reasons, readiness booleans, and `MapLoadState`.

### Evidence

- Pending store ops store load/save errors separately:

```rust
// git/bevy-persistence/src/ops.rs:33-40
pub struct PendingStoreOps<K: Send + Sync + 'static, V: Send + Sync + 'static> {
    tasks: Vec<Task<StoreOp<K, V>>>,
    pub completed_loads: Vec<(K, Option<V>)>,
    pub load_errors: Vec<(K, PersistenceError)>,
    pub save_errors: Vec<(K, PersistenceError)>,
}
```

- Save errors are logged and drained:

```rust
// crates/server/src/map.rs:52-61
fn log_save_errors<K, V>(ops: &mut PendingStoreOps<K, V>, context: &str) {
    for (key, error) in ops.save_errors.drain(..) {
        error!("Failed to save {context} at {key:?}: {error}");
    }
}
```

- Load errors fallback: meta defaults; entities proceed with none:

```rust
// crates/server/src/map.rs:218-229
for (_, e) in ops.load_errors.drain(..) {
    warn!("Failed to load map meta: {e}, using defaults");
    configure_map_from_meta(... DEFAULT_OVERWORLD_SEED, GENERATION_VERSION, ...);
    *state = MapLoadState::AwaitingEntities;
}
```

```rust
// crates/server/src/map.rs:619-621
for (_, e) in ops.load_errors.drain(..) {
    warn!("Failed to load entities for {map_id:?}: {e}, proceeding with none");
    *state = MapLoadState::Ready;
}
```

- Startup gates for relay/identity readiness:

```rust
// crates/protocol/src/app_state.rs:42-58
fn check_assets_loaded(... relay_ready: Res<RelayPoolReady>, identity_ready: Res<IdentityLoadComplete>, ...) {
    let assets_loaded = tracked.0.iter().all(|handle| asset_server.is_loaded_with_dependencies(handle));
    if !assets_loaded { trace!("check_assets_loaded: tracked assets still loading"); return; }
```

```rust
// crates/protocol/src/app_state.rs:59-68
if !relay_ready.0 { trace!("check_assets_loaded: waiting for Nostr relay EOSE"); return; }
if !identity_ready.0 { trace!("check_assets_loaded: waiting for identity store load"); return; }
info!("Startup gates complete, transitioning to AppState::Ready");
next_state.set(AppState::Ready);
```

- Existing transition states/markers are `PendingTransition`, `TransitionPending`, client `TransitionPhase`; server warns on impossible/missing pending cases:

```rust
// crates/server/src/transition.rs:119-127
for _ready in receiver.receive() {
    let Some((player_entity, pending)) = transition_query.iter().find(|(_, p)| p.client_entity == client_entity) else {
        warn!("MapTransitionReady from {client_entity:?} but no TransitionPending");
        continue;
    };
```

- Existing reject enums cover edit validation, not persistence availability:

```rust
// crates/protocol/src/world_object/types.rs:55-64
pub enum WorldObjectPlacementRejectReason {
    NoControlledCharacter,
    UnknownObject,
    NonFinitePosition,
    OutOfBounds,
    ChunkUnavailable,
}
```

## Q11: How is `nostr_client` wired into server and client crates today, what optional features or dependency boundaries exist, and where would backend-agnostic persistence interfaces need to remain free of Nostr/Blossom types?

**Direct answer:** `nostr_client` is a workspace path dependency of server/client/ui/web; server map code uses it for server identity and npub save dirs, while the generic `bevy-persistence` `Store<K,V>` interface and `voxel_map_engine` chunk spawn schema are otherwise backend-agnostic and contain no Nostr/Blossom types.

### Evidence

- Workspace dependency and crate use:

```toml
// Cargo.toml:39-42
persistence = { package = "bevy-persistence", git = "https://github.com/AdamWhitehurst/bevy-persistence.git" }
protocol = { path = "crates/protocol" }
nostr_client = { path = "crates/nostr_client" }
```

```text
// grep output
crates/server/Cargo.toml:23:nostr_client = { workspace = true }
crates/client/Cargo.toml:29:nostr_client = { workspace = true }
crates/ui/Cargo.toml:10:nostr_client = { workspace = true }
crates/web/Cargo.toml:53:nostr_client = { workspace = true }
```

- Server map code directly depends on `ServerIdentity` for overworld owner:

```rust
// crates/server/src/map.rs:127-134
fn init_overworld_entity(... server_identity: Res<nostr_client::ServerIdentity>,) {
    let map_dir = Arc::new(map_save_dir(&save_path.0, &MapInstanceId::Overworld));
    let owner = NostrPublicKey(*server_identity.keys.public_key().as_bytes());
```

- Generic persistence interface is only key/value/store and `PersistenceError`:

```rust
// git/bevy-persistence/src/store.rs:35-53
pub trait Store<K, V>: Send + Sync + Clone + 'static {
    fn save(&self, key: &K, value: &V) -> Result<(), PersistenceError>;
    fn load(&self, key: &K) -> Result<Option<V>, PersistenceError>;
}
```

- `voxel_map_engine` intentionally keeps chunk object spawn IDs as `String`, not protocol/Nostr types:

```rust
// crates/voxel_map_engine/src/config.rs:40-45
/// Uses bare `String` for `object_id` (not `WorldObjectId`) because `WorldObjectId`
/// lives in the `protocol` crate, and `voxel_map_engine` must not depend on it.
/// The server spawn system converts to `WorldObjectId` at the boundary.
pub struct WorldObjectSpawn {
```

- Nostr client plugin is normal Bevy plugin, with relay config and readiness systems:

```rust
// crates/nostr_client/src/plugin.rs:50-66
impl Plugin for NostrClientPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config.clone())
            .init_resource::<RelayPoolReady>()
            .init_resource::<IdentityLoadComplete>()
            .init_resource::<ServerList>()
            .init_resource::<ServerAnnouncementSubscriptionStarted>()
            .add_systems(Startup, (mark_identity_load_complete, spawn_relay_pool))
            .add_systems(Update, (poll_relay_pool_ready, spawn_server_announcement_subscription, poll_server_announcements))
```

## Q12: What local Blossom/Nostr source or documentation is present under `git/`, and what event kinds, Blossom endpoints, authorization events, server-list events, content hashes, and filtering patterns are relevant to the current crate versions?

**Direct answer:** `git/blossom/` contains Blossom BUD docs; no local Nostr protocol source was found under `git/`; the current Nostr Rust crate is `nostr-sdk 0.44.1`; existing code uses custom kind `30078` for server announcements, while Blossom docs define kind `24242` authorization and kind `10063` user server lists plus sha256-addressed blob endpoints.

### Evidence

- Local Blossom docs exist; Nostr source does not:

```text
// `find git -name '*nostr*'` result
No files found matching pattern
```

```text
// git/blossom/README.md:21-31
- [BUD-02: Blob upload and management](./buds/02.md)
- [BUD-03: User Server List](./buds/03.md)
...
- [BUD-11: Nostr Authorization](./buds/11.md)
- [BUD-12: Blob management endpoints](./buds/12.md)
```

- Current crate version:

```toml
// Cargo.lock:5862-5865
[[package]]
name = "nostr-sdk"
version = "0.44.1"
source = "registry+https://github.com/rust-lang/crates.io-index"
```

- Blossom basics: blobs are sha256-addressed, server endpoints include get/head/upload/list/delete/mirror:

```md
// git/blossom/README.md:7-11 Blossom is a specification for a set of HTTP endpoints that allow users to store blobs of data on publicly accessible servers ... Blobs are packs of binary data addressed by their sha256 hash
```

```md
// git/blossom/README.md:37-43

- `GET /<sha256>` (optional file `.ext`) [BUD-01]
- `HEAD /<sha256>` (optional file `.ext`) [BUD-01]
- `PUT /upload` [BUD-02]
- `HEAD /upload` [BUD-06]
- `GET /list/<pubkey>` [BUD-12] _(unrecommended)_
- `DELETE /<sha256>` [BUD-12]
- `PUT /mirror` [BUD-04]
```

- Blossom upload descriptor and content hash semantics:

```md
// git/blossom/buds/02.md:15-23 A blob descriptor is a JSON object containing `url`, `sha256`, `size`, `type`, and `uploaded` fields

- `url` A publicly accessible URL to the ... `GET /<sha256>` endpoint with a file extension
- `sha256` The sha256 hash of the blob ... Servers MUST include a file extension in the URL in the `url` field...
```

```md
// git/blossom/buds/02.md:41-44 The `PUT /upload` endpoint MUST accept binary data in the request body. The server MUST NOT modify the blob in any way and MUST compute the sha256 hash over the exact bytes received. Clients ... MAY provide an `X-SHA-256` header containing the lowercase hex-encoded sha256 of the request body.
```

- Blossom user server list is replaceable kind `10063` with `server` tags:

```md
// git/blossom/buds/03.md:7-13 Defines a replaceable event using `kind:10063` to advertise the blossom servers a user uses to host their blobs. The event MUST include at least one `server` tag containing the full server URL including the `http://` or `https://`. The order of these tags is important... The `.content` field is not used.
```

- Blossom authorization tokens are Nostr events kind `24242`, base64url in `Authorization: Nostr ...`, with `t`, `expiration`, optional `server`, `x` tags:

```md
// git/blossom/buds/11.md:7-27 Defines the format of the authorization token ... Authorization tokens are signed ... events of kind `24242`...

- MUST have ... `expiration` tag...
- MUST have a `t` tag with a verb of `get`, `upload`, `list`, `delete`, or `media`. Authorization tokens MAY include `server` and `x` tags...
```

```md
// git/blossom/buds/11.md:48-50 Using the `Authorization` HTTP header, the authorization token MUST be encoded as Base64 URL-safe without padding ... and use the Authorization scheme Nostr
```

- Current code filtering pattern uses kind + identifier:

```rust
// crates/nostr_client/src/announcement.rs:105-109
fn server_announcement_filter() -> Filter {
    Filter::new()
        .kind(Kind::Custom(NOSTR_KIND_SERVER_ANNOUNCEMENT))
        .identifier(SERVER_ANNOUNCEMENT_IDENTIFIER)
}
```

## Cross-Cutting Observations

- Existing map persistence is ECS-component-local: map entities own their store components and pending ops; systems query those components rather than a global persistence service.
- `None` has two meanings depending on store: absent metadata/chunk/chunk-entity file, but map-level empty entity file is also collapsed to `None`.
- Server is authoritative for edits: client prediction exists only for terrain UX; server acks/rejects and replication/broadcast reconcile client state.
- Map IDs are semantic and network-safe; actual map entities are side-local and resolved through `MapRegistry`.
- Terrain definitions and world-object definitions are reflect/RON component maps loaded before `AppState::Ready`.
- Nostr identity is already part of auth and map ownership; Blossom is only present as local documentation, not code.

## Open Areas

- No application-level Blossom client/server implementation was found in `crates/`; only `git/blossom` docs are present.
- No explicit stale/divergent/missing/unavailable map persistence enum was found; closest existing constructs are `MapLoadState`, `RelayPoolReady`, `IdentityLoadComplete`, transition phases, and edit reject reasons.
- No alternate non-filesystem application store implementation was found; generic `Store<K,V>` supports it, but current map persistence uses filesystem stores.
- The parallel subagents did not return usable evidence; all claims above were verified directly against repository files and local `git/blossom` docs.
