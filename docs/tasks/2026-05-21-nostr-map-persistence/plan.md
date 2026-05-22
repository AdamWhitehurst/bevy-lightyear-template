# Implementation Plan

## Overview

Map/layout persistence should choose the latest visible valid save chain across local filesystem and Nostr/Blossom without weakening server-authoritative gameplay. Remote data is fetched and validated into backend-agnostic save data, materialized into the existing filesystem layout, and then loaded by the current map/chunk pipeline.

## Global Implementation Rules

- Before any `cargo build`, `cargo check`, or `cargo test`, verify no other build/check/test is running:

```bash
if pgrep -af 'cargo (build|check|test|make)|cargo-make|rustc' | grep -v pgrep; then
  echo 'A Rust build/check/test is already running; wait or stop it first.' >&2
  exit 1
fi
```

- Keep `voxel_map_engine` free of Nostr/Blossom/protocol types.
- Keep all remote transport and signed Nostr/Blossom event/blob helpers in `crates/nostr_client`.
- Do not invent new persistence backend traits or task wrappers until first trying `bevy-persistence::Store`, `StoreBackend`, and `PendingStoreOps`; any exception must document the exact gap that `Store<K, V>` cannot express.
- Keep domain DTOs separate from persistence plumbing: map revisions, manifests, descriptor roots, journals, and validation policies are domain types; load/save/poll/backend/publisher/transport seams should generally be `Store` adapters.
- Treat unavailable remote services as graceful fallback, but treat invalid, incomplete, or divergent available chains as blocked/quarantined states.
- Add `trace!` before every expected wait/early-out. Use `panic!`, `expect`, or explicit rejection for impossible/invalid state.
- Do not change runtime gameplay authority: edits are still server-validated, server-applied, acked/replicated, and then persisted.

## Type Minimization Result

- Keep ECS/game-flow types that express gameplay or transition state: `MapLoadState`, `PendingMapSwitchPreflight`, `MapTransitionParams`, `MapPreparation`, and `RemoteMapPublishWorker`.
- Keep domain DTOs that express persistence data or protocol semantics: `MapRevision`, `ValidatedMapSave`, `ValidatedMapDelta`, `PayloadSlotState`, Nostr manifests/descriptors, blob refs, publish journal entries, and quarantine records.
- Use `MapPersistenceRejection` as the single detailed failure type; do not add parallel failure-kind or block-reason enums unless a caller needs a separate stable public code.
- Replace custom task/backend/transport/publisher traits with `Store<K, V>` adapters plus `StoreBackend`/`PendingStoreOps`: accepted head, preflight, fake remote heads/manifests/payloads, real Nostr/Blossom reads, publish journals, blob upload, and manifest publication.

---

## Phase 1: Preflight Persistence Before Transitions

### Changes

#### 1. Split server map module before adding persistence logic

**Files**: `crates/server/src/map.rs`, `crates/server/src/map/mod.rs`, `crates/server/src/map/types.rs`, `crates/server/src/map/preflight.rs`, `crates/server/src/map/preparation.rs`, `crates/server/src/map/switching.rs`  
**Action**: rename/create/modify

Do not add the new persistence types and systems to the already-large `map.rs`. First convert `map.rs` into a `map/` module tree:

- rename `crates/server/src/map.rs` to `crates/server/src/map/mod.rs`; `crates/server/src/lib.rs` can keep `pub mod map;` unchanged;
- keep `ServerMapPlugin`, resource registration, system scheduling, and broad compatibility re-exports in `map/mod.rs`;
- move shared map lifecycle/transition-preparation types into `map/types.rs` and re-export them from `map/mod.rs` with `pub use types::*;`;
- put async persistence preflight task spawning/polling in `map/preflight.rs`;
- put `ensure_map_exists`, `spawn_homebase`, and homebase seed/materialization helpers in `map/preparation.rs`;
- put `handle_map_switch_requests` and `resolve_switch_target` in `map/switching.rs`.

Keep existing voxel-edit, world-object-edit, room, chunk-push, and save systems in `map/mod.rs` for Phase 1 unless moving a function is required by the new module boundaries. Later phases should add focused modules such as `map/remote_publish.rs`, `map/homebase_publication.rs`, and `map/diagnostics.rs`; `map/mod.rs` should schedule and re-export systems, not become the dumping ground.

#### 2. Server map persistence state and preflight types

**File**: `crates/server/src/map/types.rs`  
**Action**: create

Move existing `MapLoadState` and `MapTransitionParams` from the old `map.rs` into this file, then add transition-preflight data structures beside them. Re-export these types from `map/mod.rs`.

```rust
/// Tracks a map entity's server-side load/persistence lifecycle.
#[derive(Component, Clone, Debug)]
pub enum MapLoadState {
    CheckingPersistence,
    AwaitingMeta,
    AwaitingEntities,
    Blocked(MapPersistenceRejection),
    Ready,
}

/// Describes the backend choice made by map persistence preflight.
#[derive(Clone, Debug)]
pub enum MapPersistencePreflightDecision {
    UseFilesystem(MapMeta),
    UseRemote(ValidatedMapSave),
    Missing,
    RemoteUnavailable,
    Blocked(MapPersistenceRejection),
}

/// Captures a player's map-switch intent while persistence preflight runs.
#[derive(Clone, Debug)]
pub struct PendingMapSwitchPreflight {
    pub client_entity: Entity,
    pub player_entity: Entity,
    pub current_map_id: MapInstanceId,
    pub target_map_id: MapInstanceId,
    pub requested_at: f64,
}

/// Completed persistence preflight result loaded through `PendingStoreOps`.
#[derive(Clone, Debug)]
pub struct MapPersistencePreflightResult {
    pub target_map_id: MapInstanceId,
    pub decision: MapPersistencePreflightDecision,
}

/// Seed, generation version, and bounds for a map transition message.
pub struct MapTransitionParams {
    pub seed: u64,
    pub generation_version: u32,
    pub bounds: Option<IVec3>,
    pub chunk_size: u32,
    pub column_y_range: (i32, i32),
}

/// Indicates whether a map is ready for transition commit, still loading, or blocked.
pub enum MapPreparation {
    Ready { entity: Entity, params: MapTransitionParams },
    Pending,
    Blocked(MapPersistenceRejection),
}
```

In Phase 1, `ValidatedMapSave` and `MapPersistenceRejection` may be minimal placeholders in `crates/server/src/persistence/mod.rs`; Phase 2 fills them in. Do not add a custom task component for preflight; use a `Store<PendingMapSwitchPreflight, MapPersistencePreflightResult>` adapter plus `StoreBackend`/`PendingStoreOps` so the polling path matches existing map metadata/chunk persistence.

#### 3. Start and poll preflight before committing transitions

**Files**: `crates/server/src/map/preflight.rs`, `crates/server/src/map/switching.rs`, `crates/server/src/map/mod.rs`  
**Action**: create/modify

Replace direct calls to `crate::transition::start_map_transition` inside `handle_map_switch_requests` with a preflight load spawned through bevy-persistence. The request handler must only resolve player identity/current map and store intent. It must not relocate, freeze, attach `ChunkTicket`, remove room senders, or send `MapTransitionStart`.

```rust
#[derive(Clone)]
pub struct MapPreflightStore {
    pub save_root: PathBuf,
}

impl Store<PendingMapSwitchPreflight, MapPersistencePreflightResult> for MapPreflightStore {
    fn save(
        &self,
        _key: &PendingMapSwitchPreflight,
        _value: &MapPersistencePreflightResult,
    ) -> Result<(), PersistenceError> {
        Err(PersistenceError::Serialize("map preflight store is read-only".into()))
    }

    fn load(
        &self,
        request: &PendingMapSwitchPreflight,
    ) -> Result<Option<MapPersistencePreflightResult>, PersistenceError> {
        let map_dir = map_save_dir(&self.save_root, &request.target_map_id);
        let meta_store = FsMapMetaStore { map_dir: Arc::new(map_dir) };
        let decision = match meta_store.load(&()) {
            Ok(Some(meta)) => MapPersistencePreflightDecision::UseFilesystem(meta),
            Ok(None) => MapPersistencePreflightDecision::Missing,
            Err(err) => MapPersistencePreflightDecision::Blocked(
                MapPersistenceRejection::Filesystem(err.to_string()),
            ),
        };
        Ok(Some(MapPersistencePreflightResult {
            target_map_id: request.target_map_id.clone(),
            decision,
        }))
    }
}

fn spawn_map_persistence_preflight(
    commands: &mut Commands,
    request: PendingMapSwitchPreflight,
    save_path: &WorldSavePath,
) {
    let mut ops = PendingStoreOps::<PendingMapSwitchPreflight, MapPersistencePreflightResult>::default();
    let store = MapPreflightStore { save_root: save_path.0.clone() };
    ops.spawn_load(&store, request);
    commands.spawn((StoreBackend::new(store), ops));
}
```

This Phase 1 implementation is a filesystem-only scaffold because the fake/real remote store adapters are introduced later. Do not preserve it as backend priority. Once Phase 2/3 remote lookup exists, preflight must treat filesystem state as the local baseline/accepted head, query remote before selecting a final decision, and choose filesystem only when remote is disabled, missing, unavailable, or has no accepted newer descendant.

Add `poll_map_persistence_preflight` to poll `PendingStoreOps` and drain completed loads/errors. It should:

- leave the task entity in place and `trace!` while `ops.has_pending()` is true;
- despawn the task entity after its load result or error is drained;
- materialize remote data only for `UseRemote` once Phase 2 exists;
- use filesystem/default metadata only for `UseFilesystem`, `Missing`, or `RemoteUnavailable`, where `UseFilesystem` means remote comparison has already decided local state wins;
- set/keep `MapLoadState::Blocked(_)` for `MapPersistencePreflightDecision::Blocked(_)`;
- convert unexpected `PendingStoreOps::load_errors` into a loud `MapPersistenceRejection::Filesystem(...)` block;
- call the new transition commit function only when `ensure_map_exists(...)` returns `MapPreparation::Ready`.

Register the poll system in `ServerMapPlugin::build` before `handle_map_switch_requests` or directly after it, but before `complete_map_transition`.

#### 4. Make map existence distinct from map usability

**File**: `crates/server/src/map/preparation.rs`  
**Action**: create/modify

Change `ensure_map_exists(...) -> (Entity, MapTransitionParams)` to `ensure_map_exists(...) -> MapPreparation`.

Rules:

- Existing registered maps with `MapLoadState::Ready` and `VoxelMapConfig + MapDimensions` return `Ready`.
- Existing registered maps with `CheckingPersistence`, `AwaitingMeta`, or `AwaitingEntities` return `Pending` and log `trace!`.
- Existing registered maps with `Blocked(reason)` return `Blocked(reason.clone())`.
- Missing overworld still `panic!`s because overworld must be registered at `AppState::Ready`.
- Missing homebase is spawned from selected/preflight metadata and starts in `AwaitingEntities` or `Ready` only after map entities are loaded.

```rust
pub fn ensure_map_exists(...) -> MapPreparation {
    if let Some(&entity) = registry.0.get(map_id) {
        let state = map_state_query
            .get(entity)
            .expect("registered map entity must have MapLoadState");
        match state {
            MapLoadState::Ready => {
                let (config, dimensions) = map_params_query
                    .get(entity)
                    .expect("ready map entity must have VoxelMapConfig + MapDimensions");
                return MapPreparation::Ready { entity, params: transition_params(config, dimensions) };
            }
            MapLoadState::Blocked(reason) => return MapPreparation::Blocked(reason.clone()),
            MapLoadState::CheckingPersistence | MapLoadState::AwaitingMeta | MapLoadState::AwaitingEntities => {
                trace!(?map_id, ?state, "map exists but is not ready for transition yet");
                return MapPreparation::Pending;
            }
        }
    }
    // existing spawn path, returning MapPreparation instead of tuple
}
```

#### 5. Split transition start into preflight and commit

**File**: `crates/server/src/transition.rs`  
**Action**: modify

Rename the existing `start_map_transition` body to `commit_map_transition` and take already-prepared map information.

```rust
#[allow(clippy::too_many_arguments)]
pub fn commit_map_transition(
    commands: &mut Commands,
    player_entity: Entity,
    client_entity: Entity,
    current_map_id: &MapInstanceId,
    target_map_id: &MapInstanceId,
    prepared_map: Entity,
    params: MapTransitionParams,
    room_registry: &mut RoomRegistry,
    senders: &mut Query<&mut MessageSender<MapTransitionStart>>,
    respawn_query: &Query<(&Position, &MapInstanceId), With<RespawnPoint>>,
) {
    // Same relocation/freeze/send logic as current start_map_transition.
    // Do not call ensure_map_exists here.
    commands.entity(player_entity).insert(ChunkTicket::player(prepared_map));
}
```

The only public transition function that relocates/freezes/sends `MapTransitionStart` should be `commit_map_transition`. It must be called only from `poll_map_persistence_preflight` after map preparation is ready.

#### 6. Transition tests

**File**: `crates/server/tests/map_transition.rs`  
**Action**: modify

Add focused unit/system tests for the transition gate:

- pending preflight does not insert `PendingTransition`, `ColliderDisabled`, `RigidBodyDisabled`, or `ChunkTicket`;
- pending preflight does not send `MapTransitionStart`;
- completed filesystem preflight commits transition and inserts the expected markers;
- invalid/divergent preflight records `MapLoadState::Blocked(_)` and leaves the player on the current map;
- waiting paths are explicit and observable through state, not implied by map entity existence.

Use direct systems/resources where Lightyear senders are hard to drive; assert ECS state before/after `app.update()`.

#### 7. Persistence tests for fallback/block states

**File**: `crates/server/tests/world_persistence.rs`  
**Action**: modify

Add tests for:

- local valid `map.meta.bin` gives `UseFilesystem`;
- no local meta gives `Missing` and homebase seed fallback remains deterministic;
- simulated remote unavailable gives filesystem/default fallback;
- invalid preflight does not overwrite local files.

### Verification

#### Automated

- [ ] `if pgrep -af 'cargo (build|check|test)|cargo-make|rustc' | grep -v pgrep; then echo busy >&2; exit 1; fi`
- [ ] `cargo test -p server map_transition`
- [ ] `cargo test -p server world_persistence`

#### Manual

- [ ] Start `cargo server` and `cargo client`, request a homebase switch, and confirm the player is not frozen/relocated until the server has selected seed/dimensions.
- [ ] With remote disabled/unavailable, confirm valid filesystem worlds still transition normally.

---

## Phase 2: Fake Remote Restore Materialization

### Changes

#### 1. Backend-agnostic accepted save bundle and revision metadata

**File**: `crates/server/src/persistence/mod.rs`  
**Action**: modify

Add the concrete accepted remote-save data model. It must use existing backend types only: `MapMeta`, `ChunkFileEnvelope`, `WorldObjectSpawn`, `SavedEntity`, `MapInstanceId`, `IVec3`.

```rust
pub type ManifestHash = [u8; 32];
pub type BlobHash = [u8; 32];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MapRevision {
    pub revision: u64,
    pub previous_hash: Option<ManifestHash>,
    pub manifest_hash: ManifestHash,
}

#[derive(Clone, Debug)]
pub struct ValidatedMapSave {
    pub meta: MapMeta,
    pub chunks: Vec<(IVec3, ChunkFileEnvelope)>,
    pub chunk_entities: Vec<(IVec3, Vec<WorldObjectSpawn>)>,
    pub map_entities: Option<Vec<SavedEntity>>,
    pub revision: MapRevision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapPersistenceRejection {
    Filesystem(String),
    Invalid(String),
    Incomplete(String),
    Divergent(String),
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PayloadSlotState<T> {
    Present(T),
    Empty,
    Absent,
    Tombstoned,
}
```

#### 2. Accepted head store and legacy bootstrap

**File**: `crates/server/src/persistence/mod.rs`  
**Action**: modify

Add an accepted-head file under each map directory, e.g. `accepted_head.bin`. The file is updated only after materialization succeeds. Implement it as a bevy-persistence store, not a bespoke load/save API.

```rust
#[derive(Clone, Debug)]
pub struct FsAcceptedMapHeadStore {
    pub map_dir: Arc<PathBuf>,
}

impl Store<(), MapRevision> for FsAcceptedMapHeadStore {
    fn load(&self, _key: &()) -> Result<Option<MapRevision>, PersistenceError> {
        // bincode load from accepted_head.bin; Ok(None) if absent.
    }

    fn save(&self, _key: &(), revision: &MapRevision) -> Result<(), PersistenceError> {
        // tmp-write + rename accepted_head.bin.
    }
}

impl FsAcceptedMapHeadStore {
    pub fn path(&self) -> PathBuf { self.map_dir.join("accepted_head.bin") }
}

pub fn bootstrap_filesystem_revision(
    save_dir: &Path,
    map_id: &MapInstanceId,
) -> Result<MapRevision, PersistenceError> {
    // Hash map id + existing meta/entities/chunk file bytes in sorted path order.
    // Use revision 0 and previous_hash None for legacy filesystem state.
}
```

The bootstrap hash must be deterministic: sort relative file paths lexicographically and hash `(relative_path, bytes)` for `map.meta.bin`, `entities.bin`, `terrain/*.bin`, and `entities/*.entities.bin`.

#### 3. Materialize accepted remote data crash-safely

**File**: `crates/server/src/persistence/mod.rs`  
**Action**: modify

Add file writers that reuse the same serialized formats as existing stores. Do not rewrite the existing filesystem store APIs.

```rust
pub fn cleanup_materialization_tmps(save_dir: &Path) -> Result<(), PersistenceError> {
    // Remove files ending in .tmp under map save dir.
}

pub fn materialize_validated_map_save(
    save_dir: &Path,
    save: &ValidatedMapSave,
) -> Result<(), PersistenceError> {
    cleanup_materialization_tmps(save_dir)?;
    write_meta_tmp_validate_rename(save_dir, &save.meta)?;
    write_map_entities_tmp_validate_rename(save_dir, save.map_entities.as_ref())?;
    for (pos, chunk) in &save.chunks {
        write_chunk_tmp_validate_rename(save_dir, *pos, chunk)?;
    }
    for (pos, spawns) in &save.chunk_entities {
        write_chunk_entities_tmp_validate_rename(save_dir, *pos, spawns)?;
    }
    FsAcceptedMapHeadStore { map_dir: Arc::new(save_dir.to_path_buf()) }
        .save(&(), &save.revision)?;
    Ok(())
}
```

Each writer must:

1. reuse existing `Store` serialization logic where practical;
2. write the target contents to the exact target path with `.tmp` suffix;
3. load/deserialize the tmp file through the same format used by the real store;
4. rename tmp into place;
5. leave `accepted_head.bin` untouched until all data files are committed.

#### 4. Fake remote store adapters and fixture model

**File**: `crates/server/src/persistence/mod.rs`  
**Action**: modify

Add server-side fake remote stores with no Nostr/Blossom types. Do not add a new `RemoteMapPersistence` trait; model lookup through `Store<K, V>` adapters so tests and production use the same persistence orchestration shape.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RemoteHeadKey {
    pub map_id: MapInstanceId,
    pub accepted_head: Option<ManifestHash>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RemoteManifestKey {
    pub hash: ManifestHash,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RemotePayloadKey {
    pub hash: BlobHash,
}

#[cfg(test)]
#[derive(Clone, Default)]
pub struct FakeRemoteHeadStore {
    pub manifests: Arc<HashMap<ManifestHash, TestManifest>>,
    pub heads: Arc<HashMap<MapInstanceId, ManifestHash>>,
}

#[cfg(test)]
impl Store<RemoteHeadKey, TestManifest> for FakeRemoteHeadStore { /* latest visible head lookup */ }

#[cfg(test)]
#[derive(Clone, Default)]
pub struct FakeRemoteManifestStore {
    pub manifests: Arc<HashMap<ManifestHash, TestManifest>>,
}

#[cfg(test)]
impl Store<RemoteManifestKey, TestManifest> for FakeRemoteManifestStore { /* ancestor lookup */ }

#[cfg(test)]
#[derive(Clone, Default)]
pub struct FakeRemotePayloadStore {
    pub payloads: Arc<HashMap<BlobHash, Vec<u8>>>,
}

#[cfg(test)]
impl Store<RemotePayloadKey, Vec<u8>> for FakeRemotePayloadStore { /* blob bytes lookup */ }
```

`TestManifest` should describe map id, owner, revision, previous hash, and slot descriptors for meta/chunks/chunk entities/map entities. Missing data should use `Ok(None)`; invalid, incomplete, divergent, or unavailable data should be surfaced as explicit `MapPersistenceRejection` values by the assembly/preflight layer rather than hidden behind a parallel lookup enum.

#### 5. Chain assembly before materialization

**File**: `crates/server/src/persistence/mod.rs`  
**Action**: modify

Add assembly functions that fetch ancestors by hash, replay deltas from a base, and reject missing required slots before materialization.

```rust
pub enum SaveBase {
    Empty,
    Snapshot(ValidatedMapSave),
}

pub struct ValidatedMapDelta {
    pub revision: MapRevision,
    pub meta: PayloadSlotState<MapMeta>,
    pub chunks: HashMap<IVec3, PayloadSlotState<ChunkFileEnvelope>>,
    pub chunk_entities: HashMap<IVec3, PayloadSlotState<Vec<WorldObjectSpawn>>>,
    pub map_entities: PayloadSlotState<Vec<SavedEntity>>,
}

pub fn assemble_validated_map_save(
    base: SaveBase,
    chain: Vec<ValidatedMapDelta>,
) -> Result<ValidatedMapSave, MapPersistenceRejection> {
    // Validate each delta descends from the previous manifest hash.
    // Replay Present/Empty/Tombstoned/Absent semantics.
    // Reject if meta is absent after replay.
    // Reject if a required chunk/entity payload is referenced but missing.
}
```

Semantics:

- `Present(value)` writes/updates a slot;
- `Empty` writes an authoritative empty value for entity slots;
- `Tombstoned` deletes the slot from the assembled save;
- `Absent` means no change in that delta, not missing content.

#### 6. Integrate fake restore with map preflight

**Files**: `crates/server/src/map/preflight.rs`, `crates/server/src/map/preparation.rs`  
**Action**: modify

Teach `MapPreflightStore::load`/`poll_map_persistence_preflight` to select a backend in this order:

- load accepted head via `FsAcceptedMapHeadStore` or bootstrap filesystem revision as the local baseline, not as an automatic winner;
- query fake remote `Store` adapters when configured in tests before deciding `UseFilesystem`;
- compare visible remote descendant to filesystem accepted head;
- return `UseRemote(save)` after full chain assembly when remote has an accepted descendant;
- return `UseFilesystem` only when remote is missing/unavailable or has no accepted newer descendant;
- call `materialize_validated_map_save` before existing map load proceeds.

Keep production remote disabled in Phase 2 unless tests install fake `Store` adapters. `SaveBase` should use already-loaded local snapshot data, not filesystem paths, so assembly is backend-agnostic.

#### 7. Map meta store helper for tmp validation

**File**: `crates/server/src/persistence/fs_map_meta.rs`  
**Action**: modify

Add a small public/internal helper so materialization can validate a specific tmp meta file without duplicating version/deserialization logic, or factor private read/validate logic into reusable functions.

```rust
pub(crate) fn load_map_meta_file(path: &Path) -> Result<MapMeta, PersistenceError> {
    let bytes = fs::read(path).map_err(|e| PersistenceError::Deserialize(format!("read meta: {e}")))?;
    let meta: MapMeta = bincode::deserialize(&bytes)
        .map_err(|e| PersistenceError::Deserialize(format!("deserialize meta: {e}")))?;
    if meta.version != META_VERSION {
        return Err(PersistenceError::VersionMismatch { expected: META_VERSION, actual: meta.version });
    }
    Ok(meta)
}
```

#### 8. End-to-end restore tests

**File**: `crates/server/tests/world_persistence.rs`  
**Action**: modify

Add tests named with `remote_restore` in the test name so they can be filtered by `cargo test -p server remote_restore`:

- `remote_restore_missing_local_save_materializes_meta_chunks_and_entities`;
- `remote_restore_delta_chain_replays_from_filesystem_base`;
- `remote_restore_incomplete_slot_rejected`;
- `remote_restore_tombstone_removes_slot`;
- `remote_restore_accepted_head_written_after_files`;
- `remote_restore_tmp_cleanup_removes_interrupted_files`;
- `remote_restore_divergent_chain_preserves_filesystem`.

#### 9. Existing voxel persistence regression tests

**File**: `crates/server/tests/voxel_persistence.rs`  
**Action**: modify

Add or update tests to prove materialized chunks are loadable by `FsChunkStore` and dirty saves after restore still write through the normal filesystem store.

### Verification

#### Automated

- [ ] `if pgrep -af 'cargo (build|check|test)|cargo-make|rustc' | grep -v pgrep; then echo busy >&2; exit 1; fi`
- [ ] `cargo test -p server remote_restore`
- [ ] `cargo test -p server world_persistence`
- [ ] `cargo test -p server voxel_persistence`

#### Manual

- [ ] Create a temp `worlds/` directory with no local save, enable the fake remote test resource in a local harness, and confirm `map.meta.bin`, chunk files, entity files, and `accepted_head.bin` appear only after a complete valid chain.
- [ ] Interrupt a materialization run after tmp writes in a debugger/test harness and confirm the next startup removes leftover `.tmp` files before loading.

---

## Phase 3: Real Nostr/Blossom Read Path

### Changes

#### 1. Export the map persistence module

**File**: `crates/nostr_client/src/lib.rs`  
**Action**: modify

Add and re-export the new module.

```rust
pub mod map_persistence;

pub use map_persistence::{
    BlobRef, BlossomReadError, ManifestPayloadDescriptor, MapPersistencePolicy,
    NostrMapManifest, NostrMapQueryPolicy, VerifiedBlob,
};
```

#### 2. Add dependencies for hashing, URL parsing, and native HTTP blob fetches

**File**: `crates/nostr_client/Cargo.toml`  
**Action**: modify

Add direct dependencies already used conceptually by the structure outline:

```toml
sha2 = "0.10"
url = "2"
thiserror = "2"

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
```

Keep the production HTTP transport behind `#[cfg(not(target_arch = "wasm32"))]`; tests use fake transport and must not require real network.

#### 3. Signed manifest, payload descriptors, policy, and query policy

**File**: `crates/nostr_client/src/map_persistence.rs`  
**Action**: create

Add the Nostr/Blossom read model. The manifest content should serialize as JSON and remain independent of server ECS types except for protocol-safe identity/map types.

```rust
use protocol::{MapInstanceId, NostrPublicKey};

pub const NOSTR_KIND_MAP_MANIFEST: u16 = 30079;
pub const MAP_MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NostrMapManifest {
    pub map_id: MapInstanceId,
    pub owner: NostrPublicKey,
    pub revision: u64,
    pub previous_hash: Option<[u8; 32]>,
    pub payloads: Vec<ManifestPayloadDescriptor>,
    pub schema_version: u32,
    pub descriptor_root: [u8; 32],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum PayloadClass {
    MapMeta,
    TerrainChunk,
    ChunkEntities,
    MapEntities,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum PayloadKey {
    Singleton,
    Chunk { x: i32, y: i32, z: i32 },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestPayloadDescriptor {
    pub class: PayloadClass,
    pub key: PayloadKey,
    pub blob: BlobRef,
    pub schema_version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlobRef {
    pub sha256: [u8; 32],
    pub size: u64,
    pub content_type: String,
    pub urls: Vec<String>,
}
```

Add policy structs:

```rust
pub struct MapPersistencePolicy {
    pub max_blob_bytes: u64,
    pub max_manifest_bytes: usize,
    pub max_payloads: usize,
    pub allowed_payload_classes: BTreeSet<PayloadClass>,
    pub entity_allowlist: BTreeSet<String>,
    pub quota: MapPersistenceQuota,
    pub allowed_blossom_hosts: BTreeSet<String>,
}

pub struct NostrMapQueryPolicy {
    pub relays: Vec<String>,
    pub timeout: Duration,
    pub limit: usize,
    pub tie_break: ManifestTieBreak,
}
```

#### 4. Manifest verification and descriptor root

**File**: `crates/nostr_client/src/map_persistence.rs`  
**Action**: modify

Implement:

```rust
pub fn verify_manifest_event(
    event_json: &str,
    expected_owner: NostrPublicKey,
    expected_map_id: &MapInstanceId,
) -> Result<NostrMapManifest, ManifestVerificationError> {
    let event = nostr_sdk::Event::from_json(event_json)?;
    if event.kind != nostr_sdk::Kind::Custom(NOSTR_KIND_MAP_MANIFEST) { /* reject */ }
    if !event.verify_signature() { /* reject */ }
    if NostrPublicKey(*event.pubkey.as_bytes()) != expected_owner { /* reject */ }
    let manifest: NostrMapManifest = serde_json::from_str(&event.content)?;
    if manifest.owner != expected_owner || &manifest.map_id != expected_map_id { /* reject */ }
    if manifest.schema_version != MAP_MANIFEST_SCHEMA_VERSION { /* reject */ }
    verify_descriptor_root(&manifest)?;
    Ok(manifest)
}

pub fn verify_descriptor_root(manifest: &NostrMapManifest) -> Result<(), ManifestVerificationError> {
    // Sort descriptors by class/key/schema/blob hash.
    // Hash domain separator + class + key + blob sha256 + blob size + schema version.
}
```

Descriptor root hashing must be domain-separated, deterministic, and covered by tests that mutate one field at a time.

#### 5. Revision-chain verification and ancestor fetch

**File**: `crates/nostr_client/src/map_persistence.rs`  
**Action**: modify

Implement:

```rust
pub enum RevisionDecision {
    AtAcceptedHead,
    Descendant(Vec<NostrMapManifest>),
}

pub fn verify_revision_chain(
    candidate: &NostrMapManifest,
    accepted_head: Option<MapRevision>,
) -> Result<RevisionDecision, MapPersistenceRejection> {
    // Revision number may order candidates, but only previous_hash descent proves safety.
}

pub async fn fetch_manifest_ancestors(
    client: &nostr_sdk::Client,
    head: &NostrMapManifest,
    accepted_head: Option<MapRevision>,
) -> Result<Vec<NostrMapManifest>, RemotePersistenceError> {
    // Query previous_hash manifests until accepted head/genesis/base or reject as missing/divergent.
}
```

Reject rollback, missing ancestors, ambiguous forks, and divergent heads. Latestness is only relative to configured query policy and local accepted head.

#### 6. Blossom URL and byte verification

**File**: `crates/nostr_client/src/map_persistence.rs`  
**Action**: modify

Implement pure verifiers and HTTP fetch helpers. Do not add a separate `BlobTransport` trait unless the server-side `Store<BlobFetchRequest, VerifiedBlob>` adapter in Phase 3.8 cannot express the operation.

```rust
pub struct VerifiedBlob {
    pub sha256: [u8; 32],
    pub bytes: Vec<u8>,
}

pub fn verify_blob_url(url: &url::Url, policy: &MapPersistencePolicy) -> Result<(), BlossomReadError> {
    if url.scheme() != "https" { /* reject */ }
    let host = url.host_str().ok_or(BlossomReadError::MissingHost)?;
    if !policy.allowed_blossom_hosts.contains(host) { /* reject */ }
    Ok(())
}

pub fn verify_blob_bytes(
    expected_sha256: [u8; 32],
    expected_size: Option<u64>,
    bytes: Vec<u8>,
) -> Result<VerifiedBlob, BlossomReadError> {
    if expected_size.is_some_and(|size| size != bytes.len() as u64) { /* reject */ }
    let actual = sha2::Sha256::digest(&bytes);
    if actual.as_slice() != expected_sha256 { /* reject */ }
    Ok(VerifiedBlob { sha256: expected_sha256, bytes })
}
```

`fetch_and_verify_blob` must try policy-approved URLs only and accept the first byte body that matches hash and size.

#### 7. Remote helper functions

**File**: `crates/nostr_client/src/map_persistence.rs`  
**Action**: modify

Add pure/async helper functions around the existing `nostr_sdk::Client`. Do not add bevy-persistence dependencies to `nostr_client`; the server boundary wraps these helpers in `Store` adapters.

```rust
pub async fn latest_visible_manifest(
    client: &nostr_sdk::Client,
    owner: NostrPublicKey,
    map_id: &MapInstanceId,
    policy: NostrMapQueryPolicy,
) -> Result<Option<NostrMapManifest>, RemotePersistenceError> { /* query + verify + tie-break */ }

pub async fn download_payloads(
    manifest_chain: &[NostrMapManifest],
    policy: MapPersistencePolicy,
) -> Result<RawMapPayloads, RemotePersistenceError> { /* blob descriptors to verified bytes */ }

pub fn validate_remote_map_save(
    manifest_chain: Vec<NostrMapManifest>,
    payloads: RawMapPayloads,
    policy: MapPersistencePolicy,
) -> Result<ValidatedMapSave, MapPersistenceRejection> { /* decode + bounds/quota/schema/allowlist */ }
```

Keep manifest verification and chain-verification helpers separate from network helpers so they remain easy to unit test.

#### 8. Server boundary integration

**File**: `crates/server/src/persistence/mod.rs`  
**Action**: modify

Add conversion/validation helpers that turn `nostr_client::RawMapPayloads` into `ValidatedMapDelta`/`ValidatedMapSave` without leaking Nostr/Blossom types into voxel engine. Keep policy enforcement at the server boundary for map bounds, entity allowlists, quota, and class allowlists.

Also add server-side bevy-persistence adapters around the pure `nostr_client` helpers:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NostrManifestQuery {
    pub owner: NostrPublicKey,
    pub map_id: MapInstanceId,
    pub accepted_head: Option<ManifestHash>,
}

#[derive(Clone)]
pub struct ServerNostrManifestStore {
    pub client: nostr_sdk::Client,
    pub policy: NostrMapQueryPolicy,
}

impl Store<NostrManifestQuery, NostrMapManifest> for ServerNostrManifestStore { /* wraps latest_visible_manifest */ }

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BlobFetchRequest {
    pub blob: BlobRef,
    pub max_bytes: u64,
}

#[derive(Clone)]
pub struct ServerBlossomBlobStore {
    pub policy: MapPersistencePolicy,
}

impl Store<BlobFetchRequest, VerifiedBlob> for ServerBlossomBlobStore { /* wraps download/verify helpers */ }
```

These adapters may block on async `nostr_client` helpers internally because `PendingStoreOps` runs them off the main ECS schedule.

#### 9. Preflight uses real remote when configured

**File**: `crates/server/src/map/preflight.rs`  
**Action**: modify

Add server-side `Store` adapters that wrap the pure `nostr_client` helpers, then wire them into `MapPreflightStore` behind optional server resources/config. Preserve Phase 2 backend-selection semantics:

- remote disabled or no stores configured => filesystem/default behavior;
- remote configured => query remote stores before returning `UseFilesystem`;
- missing remote head => compare/load filesystem;
- timeout/unreachable => `RemoteUnavailable` and filesystem fallback;
- verified descendant => materialize then load;
- invalid/incomplete/divergent => blocked.

#### 10. Relay pool support

**File**: `crates/nostr_client/src/relay_pool.rs`  
**Action**: modify

Expose enough of the existing relay `Client` for the server-side Nostr manifest store adapter, without changing relay readiness semantics. If `RelayPool` already owns a `nostr_sdk::Client`, add a getter/clone helper.

```rust
impl RelayPool {
    pub fn client(&self) -> nostr_sdk::Client {
        self.client.clone()
    }
}
```

### Verification

#### Automated

- [ ] `if pgrep -af 'cargo (build|check|test)|cargo-make|rustc' | grep -v pgrep; then echo busy >&2; exit 1; fi`
- [ ] `cargo test -p nostr_client map_persistence`
- [ ] `cargo test -p server remote_restore`

#### Manual

- [ ] Review test fixtures to confirm they cover one-field-at-a-time tampering for signature, pubkey, map id, kind, tags, revision, previous hash, descriptor slot, blob hash, and blob size.
- [ ] Confirm no test requires an external relay, Blossom server, local HTTP server, or network access.

---

## Phase 4: Server-Owned Overworld Dual-Write

### Changes

#### 1. Publish journal and status model

**File**: `crates/server/src/persistence/mod.rs`  
**Action**: modify

Add persisted publish journal types.

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RemotePublishStatus {
    Pending,
    InFlight,
    Published,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemotePublishJournalEntry {
    pub map_id: MapInstanceId,
    pub local_revision: MapRevision,
    pub previous_remote_manifest_hash: Option<ManifestHash>,
    pub new_manifest_hash: ManifestHash,
    pub payloads: Vec<PublishPayloadRef>,
    pub status: RemotePublishStatus,
    pub retry_count: u32,
}

#[derive(Clone, Debug)]
pub struct FsRemotePublishJournalStore {
    pub save_root: PathBuf,
}

impl Store<MapInstanceId, Vec<RemotePublishJournalEntry>> for FsRemotePublishJournalStore {
    fn load(&self, map_id: &MapInstanceId) -> Result<Option<Vec<RemotePublishJournalEntry>>, PersistenceError> {
        // bincode load per-map journal; Ok(None) if absent.
    }

    fn save(
        &self,
        map_id: &MapInstanceId,
        entries: &Vec<RemotePublishJournalEntry>,
    ) -> Result<(), PersistenceError> {
        // tmp-write + rename per-map journal.
    }
}
```

Persist journal entries under server-controlled per-map journal files via `FsRemotePublishJournalStore` plus `StoreBackend`/`PendingStoreOps`.

#### 2. Publish helpers and server store adapters

**Files**: `crates/nostr_client/src/map_persistence.rs`, `crates/server/src/map/remote_publish.rs`  
**Action**: modify/create

Keep Nostr/Blossom signing and HTTP upload helpers in `nostr_client`, but keep bevy-persistence store adapters at the server boundary. Do not add generic `BlobPublisher`/`RemotePublisher` traits unless `Store<K, V>` cannot model the operation.

```rust
// crates/nostr_client/src/map_persistence.rs
pub async fn upload_blossom_blob(upload_url: &str, bytes: Vec<u8>) -> Result<BlobRef, RemotePersistenceError>;
pub async fn publish_manifest_event(client: &nostr_sdk::Client, event_json: String) -> Result<(), RemotePersistenceError>;
```

```rust
// crates/server/src/map/remote_publish.rs
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BlobPutKey {
    pub blob: BlobRef,
}

#[derive(Clone)]
pub struct ServerBlossomBlobPutStore {
    pub upload_url: String,
}

impl Store<BlobPutKey, Vec<u8>> for ServerBlossomBlobPutStore { /* wraps upload_blossom_blob; load is unsupported */ }

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ManifestPublishKey {
    pub hash: ManifestHash,
}

#[derive(Clone)]
pub struct ServerNostrManifestPublishStore {
    pub client: nostr_sdk::Client,
}

impl Store<ManifestPublishKey, String> for ServerNostrManifestPublishStore { /* wraps publish_manifest_event; load is unsupported */ }

pub fn publish_server_map_update(
    identity: &ServerIdentity,
    update: ValidatedMapDelta,
    blob_store: &impl Store<BlobPutKey, Vec<u8>>,
    manifest_store: &impl Store<ManifestPublishKey, String>,
) -> Result<MapRevision, RemotePersistenceError> {
    // Serialize payload slots, compute BlobRef values deterministically, save blobs,
    // build descriptor root, sign manifest, then save event JSON to manifest_store.
}
```

Use deterministic serialization and deterministic manifest hashes so retrying an entry republishes the same manifest hash. Reuse `ValidatedMapDelta`/payload slot semantics for publish input instead of inventing a second `ServerMapUpdate` shape.

#### 3. Server publish worker and save integration

**Files**: `crates/server/src/map/remote_publish.rs`, `crates/server/src/map/mod.rs`  
**Action**: create/modify

After existing filesystem saves succeed in `save_dirty_chunks_debounced`/chunk entity save paths, enqueue a publish journal entry for overworld map updates.

Add a `RemoteMapPublishWorker` resource/system:

```rust
#[derive(Resource, Default)]
pub struct RemoteMapPublishWorker {
    pub in_flight_by_map: HashSet<MapInstanceId>,
}

fn poll_remote_publish_journal(...) {
    // For each map, publish only the oldest Pending entry.
    // Mark InFlight before spawn; Published only after success.
    // Failed entries block later entries until retried or administratively cleared.
}
```

Rules:

- only one in-flight publish per map;
- later pending entries never publish past an earlier failed entry;
- remote head advances only after publish success;
- local filesystem saves continue even if remote publish fails;
- later pending entries may be squashed only if their `previous_remote_manifest_hash` is recomputed against the current remote head;
- overworld manifests must be signed by configured server identity.

#### 4. Server persistence journal helpers

**File**: `crates/server/src/persistence/mod.rs`  
**Action**: modify

Add journal load/save/recovery helpers. On startup, reset `InFlight` to `Pending` so interrupted publishes retry deterministically.

#### 5. Voxel publish tests

**File**: `crates/server/tests/voxel_persistence.rs`  
**Action**: modify

Add `remote_publish`-filtered tests:

- publish N fails while N+1 is queued;
- N+1 does not publish before N succeeds;
- retry of N uses the same deterministic manifest hash;
- remote already has manifest hash counts as success;
- local chunk file exists even when remote publish fails.

#### 6. World-object publish tests

**File**: `crates/server/tests/world_object_edit.rs`  
**Action**: modify

Add tests proving chunk-entity changes enqueue/publish alongside terrain and preserve authoritative empty chunk-entity files.

### Verification

#### Automated

- [ ] `if pgrep -af 'cargo (build|check|test)|cargo-make|rustc' | grep -v pgrep; then echo busy >&2; exit 1; fi`
- [ ] `cargo test -p server remote_publish`
- [ ] `cargo test -p server voxel_persistence`
- [ ] `cargo test -p server world_object_edit`

#### Manual

- [ ] Run a server with fake remote publisher, make two overworld edits quickly, and confirm logs show only one in-flight publish for `Overworld`.
- [ ] Force the first fake publish to fail and confirm later entries wait while local filesystem saves still advance.

---

## Phase 5: Player-Owned Homebase Publication

### Changes

#### 1. Protocol attestation type

**File**: `crates/protocol/src/map/persistence.rs`  
**Action**: modify

Add server-signed homebase publication attestation data. Keep it protocol-serializable and free of Nostr SDK types.

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct HomebasePublicationAttestation {
    pub owner: NostrPublicKey,
    pub map_id: MapInstanceId,
    pub server_revision: u64,
    pub previous_manifest_hash: Option<[u8; 32]>,
    pub descriptor_root: [u8; 32],
    pub payload_scope: HomebasePayloadScope,
    pub expires_at: u64,
    pub server_pubkey: NostrPublicKey,
    pub server_signature: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct HomebasePayloadScope {
    pub terrain_chunks: Vec<IVec3>,
    pub chunk_entities: Vec<IVec3>,
    pub includes_meta: bool,
    pub includes_map_entities: bool,
}
```

If `IVec3` cannot be serialized from `protocol` without adding a dependency or feature, use a small protocol-local serializable chunk coordinate struct and convert at boundaries.

#### 2. Client publication queue and completeness tracking

**File**: `crates/client/src/map.rs`  
**Action**: modify

Track authoritative replicated homebase data eligible for publication.

```rust
#[derive(Resource, Default)]
pub struct HomebasePublicationQueue {
    pub pending: VecDeque<ClientHomebaseUpdateDraft>,
}

#[derive(Clone, Debug, Default)]
pub struct HomebaseReplicaCompleteness {
    pub has_meta: bool,
    pub terrain_chunks: HashSet<IVec3>,
    pub chunk_entities: HashSet<IVec3>,
    pub has_map_entities: bool,
}
```

Update completeness only from authoritative server messages/acks/replication, not from speculative client prediction. Do not include progression-bearing state.

#### 3. Transition/start completeness hooks

**File**: `crates/client/src/transition.rs`  
**Action**: modify

When entering a homebase transition, initialize/reset the publication completeness tracker for that `MapInstanceId`. When transition readiness completes and authoritative chunks/entities are present, mark publishable slots.

#### 4. Client publish unit and attested publish

**Files**: `crates/nostr_client/src/map_persistence.rs`, `crates/client/src/map_publication.rs`  
**Action**: modify/create

Add client-owned homebase publish data and keep store orchestration at the client boundary. `nostr_client` should build/sign payloads and events; client code should use bevy-persistence store adapters for blob upload and manifest publication.

```rust
pub struct ClientHomebaseUpdate {
    pub owner: NostrPublicKey,
    pub map_id: MapInstanceId,
    pub payloads: Vec<ManifestPayloadDescriptor>,
    pub previous_revision: Option<MapRevision>,
    pub attestation: HomebasePublicationAttestation,
}

pub fn build_homebase_manifest_event(
    identity: &ClientIdentity,
    update: ClientHomebaseUpdate,
) -> Result<(ManifestHash, String), RemotePersistenceError> {
    // Sign with player identity and include attestation in manifest event JSON.
}
```

`crates/client/src/map_publication.rs` should reuse or mirror the Phase 4 `Store<BlobPutKey, Vec<u8>>` and `Store<ManifestPublishKey, String>` adapters, then enqueue/poll uploads through `PendingStoreOps`.

#### 5. Server attestation request/verification

**File**: `crates/server/src/map/homebase_publication.rs`  
**Action**: create/modify

Add the server-side logic to verify a descriptor root against authoritative homebase state and sign the attestation.

```rust
pub fn verify_homebase_publication_attestation_request(
    owner: NostrPublicKey,
    map_id: &MapInstanceId,
    descriptor_root: [u8; 32],
    payload_scope: &HomebasePayloadScope,
    authoritative_state: &AuthoritativeHomebaseState,
) -> Result<HomebasePublicationAttestation, MapPersistenceRejection> {
    // map_id must be Homebase { owner }
    // descriptor root must match authoritative meta/chunk/entity payload hashes at server revision
    // reject progression-bearing or entitlement-unsafe payloads
}
```

Only sign attestations for `MapInstanceId::Homebase { owner }`. Reject overworld and foreign-owner requests.

#### 6. Import validation for homebase manifests

**File**: `crates/nostr_client/src/map_persistence.rs`  
**Action**: modify

Import accepts player-owned homebase data only if:

- player signature is valid;
- manifest signer equals owner;
- map id is `Homebase { owner }`;
- server attestation signature is valid;
- attestation owner/map/revision/descriptor root matches manifest;
- revision descends from accepted head;
- payloads pass hash/schema/bounds/quota/allowlist validation.

#### 7. Server import policy rejects progression-bearing data

**File**: `crates/server/src/map/homebase_publication.rs`  
**Action**: modify

Add server policy checks that reject progression-bearing objects, earned inventory, character state, relationships, breeding state, rewards, unentitled furnishings/toys/eggs/rewards, and all client-published overworld data.

#### 8. Client publication tests

**File**: `crates/client/tests/map_transition.rs`  
**Action**: modify

Replace the existing placeholder-only file with tests named `homebase_publication_*` where practical, or add a new test module in the same file. Cover:

- homebase transition initializes completeness tracking;
- speculative prediction alone does not mark a slot publishable;
- ack/replicated authoritative state can mark a slot publishable;
- publish queue waits for attestation.

### Verification

#### Automated

- [ ] `if pgrep -af 'cargo (build|check|test)|cargo-make|rustc' | grep -v pgrep; then echo busy >&2; exit 1; fi`
- [ ] `cargo test -p client homebase_publication`
- [ ] `cargo test -p server remote_restore`

#### Manual

- [ ] Run server and client, edit a homebase, wait for server ack/replication, request server attestation, publish from client, then restart/import and confirm import succeeds only with both valid player signature and valid server attestation.
- [ ] Try to publish an overworld or foreign-owner homebase manifest from a client identity and confirm it is rejected.

---

## Phase 6: Quarantine, Rollback, and Diagnostics

### Changes

#### 1. Quarantine and runtime remote config

**File**: `crates/server/src/persistence/mod.rs`  
**Action**: modify

Add quarantine record/config types and filesystem helpers.

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuarantinedMapSave {
    pub map_id: MapInstanceId,
    pub owner: NostrPublicKey,
    pub reason: MapPersistenceRejection,
    pub manifest_hash: ManifestHash,
}

#[derive(Resource, Clone, Debug)]
pub struct RemoteMapPersistenceConfig {
    pub enabled: bool,
    pub fallback_timeout: Duration,
    pub quarantine_dir: PathBuf,
}

pub fn quarantine_rejected_map_save(
    config: &RemoteMapPersistenceConfig,
    record: &QuarantinedMapSave,
    raw_manifest: Option<&[u8]>,
) -> Result<(), PersistenceError> { /* tmp + rename record and optional manifest */ }
```

Default config should preserve current filesystem-only behavior when remote persistence is disabled.

#### 2. Server diagnostics and startup recovery

**Files**: `crates/server/src/map/diagnostics.rs`, `crates/server/src/map/mod.rs`  
**Action**: create/modify

Add startup/update systems that:

- remove leftover materialization `.tmp` files;
- reset publish journal `InFlight` entries to `Pending`;
- validate accepted-head references point to existing materialized files;
- quarantine/fallback if accepted head points to missing/invalid materialized data;
- emit structured logs for map id, owner, selected backend, revision, manifest hash, remote head, local accepted head, query policy, and failure class.

Every expected wait/fallback path must log with `trace!`; invalid/divergent/incomplete paths should use `warn!` or `error!` with the rejection reason.

#### 3. Nostr diagnostics classification

**File**: `crates/nostr_client/src/map_persistence.rs`  
**Action**: modify

Ensure remote errors classify distinctly as:

- unavailable/timeout;
- manifest verification failure;
- descriptor root mismatch;
- revision-chain divergence;
- incomplete payload set;
- Blossom URL policy rejection;
- blob hash/size mismatch.

These variants feed server quarantine/block decisions.

#### 4. README documentation

**File**: `README.md`  
**Action**: modify

Add a concise `Nostr/Blossom Map Persistence` subsection under Development or Nostr configuration covering:

- v1 scope: map/layout persistence for Overworld and Homebase only;
- latestness limitation: latest visible valid descendant under configured query policy and local accepted head, not global latest;
- remote disabled mode and filesystem fallback behavior;
- quarantine directory and what invalid/divergent means;
- accepted head file and safe rollback/manual recovery path;
- no progression-bearing client-published state in v1.

#### 5. Update task structure notes after implementation

**File**: `docs/tasks/2026-05-21-nostr-map-persistence/structure.md`  
**Action**: modify

After Phase 6 implementation, add a short implementation note or checklist result that records the final remote-disable, quarantine, rollback, and diagnostics behavior. Do not change the phase order or design decisions.

### Verification

#### Automated

- [ ] `if pgrep -af 'cargo (build|check|test)|cargo-make|rustc' | grep -v pgrep; then echo busy >&2; exit 1; fi`
- [ ] `cargo check-all`
- [ ] `cargo test-all`

#### Manual

- [ ] Existing filesystem-only worlds load unchanged with remote persistence disabled.
- [ ] Remote can be disabled without migration or save-directory changes.
- [ ] Divergent remote data is quarantined without overwriting valid filesystem state.
- [ ] Logs distinguish relay/Blossom unavailable, invalid data, incomplete data, and divergent chain.
- [ ] README recovery steps are sufficient to locate quarantine records, inspect accepted head, disable remote, and roll back to filesystem state.
