# Research Findings

## Q1: DevPlugin wiring and Bevy version constraints

### Findings
- `crates/dev/src/lib.rs:1-30` — 30 lines total. Defines `pub struct DevPlugin` (`lib.rs:8`); adds `avian3d::PhysicsDebugPlugin::default()`, `hide_physics_debug` (`Startup`), `toggle_physics_debug` (`Update`) at `lib.rs:10-16`. No resources/components/events.
- `DevPlugin` added at `crates/client/src/main.rs:55` and `crates/web/src/main.rs:41`. **Not added** in `crates/server/src/main.rs` — `server` has no `dev` dependency.
- Bevy version: `bevy = { version = "0.18", default-features = false }` at root `Cargo.toml:43`. Resolved from crates.io (no `[patch.crates-io]` entry for bevy).
- Vendored `git/bevy`, `git/lightyear`, `git/avian` are under `[workspace].exclude` at `Cargo.toml:14-31`. Only `lightyear` is pulled in via local path at `Cargo.toml:46`; its own workspace also targets `bevy 0.18` (`git/lightyear/Cargo.toml:190`). `avian3d = "0.5.0"` from crates.io. `[patch.crates-io]` patches only `ndshape`, `ndcopy`, `leafwing-input-manager`.
- `bevy_egui` / `bevy-inspector-egui` do **not** appear in any of this project's Cargo.toml files. They only appear inside the vendored lightyear examples: `git/lightyear/Cargo.toml:232` (`bevy-inspector-egui = "0.36"`), `Cargo.toml:238` (`bevy_egui = "0.39"`).

## Q2: voxel_map_engine chunk lifecycle

### Findings
- Canonical chunk stage enum `ChunkStatus` at `crates/voxel_map_engine/src/types.rs:32-38`: `Empty → Terrain → Features → Mesh → Full`. Stored in `ChunkData.status` (`types.rs:70`) inside `VoxelMapInstance.tree`.
- Orthogonal simulation-intensity enum `LoadState` at `ticket.rs:93-103` (`EntityTicking/BlockTicking/Border/Inaccessible`). Derived from an integer level via `LoadState::from_level(u32)` (`ticket.rs:107`). Max status per level via `ChunkStatus::max_for_level` (`types.rs:55-62`).
- No marker components for state. State is held by: (1) `ChunkData.status` in the octree, (2) `VoxelMapInstance.chunk_levels: HashMap<IVec2, u32>` (`instance.rs:31`), (3) `TicketLevelPropagator.levels: HashMap<IVec2, u32>` (`propagator.rs:37`).
- Coordinates: chunk `IVec3`, column `IVec2`. Conversions at `api.rs:189`, `lifecycle.rs:974`, `ticket.rs:147`, `ticket.rs:142`.
- `api.rs` only exposes `VoxelWorld` `SystemParam` with `get_voxel`, `chunk_size`, `set_voxel`, `raycast` (`api.rs:14-71`). **No direct "force load chunk" function.**
- **Tickets drive loading.** `ChunkTicket { map_entity, ticket_type, radius }` at `ticket.rs:37`. `TicketType::{Player(lvl 0,r4), Npc(lvl 1,r1), MapTransition(lvl 2,r4)}` at `ticket.rs:5-12`. Constructors at `ticket.rs:61-85`.
- `LOAD_LEVEL_THRESHOLD` = 20 (prod) / 4 (test) (`ticket.rs:134-135`). Columns at level ≤ threshold are loaded.
- Driver systems in `VoxelPlugin::build` (`lib.rs:46-58`, chained in `Update`): `ensure_pending_chunks`, `collect_tickets`, `update_chunks`, `poll_chunk_tasks`, `despawn_out_of_range_chunks`, `drain_pending_saves`, `spawn_remesh_tasks`, `poll_remesh_tasks`, `reset_chunk_budgets` (client-only).
- `collect_tickets` (`lifecycle.rs:414`) reads `ChunkTicket` + `GlobalTransform`, calls `prop.set_source(entity, column, base_level, radius)`. On removal, `prop.remove_source(entity)` (`lifecycle.rs:445`).
- `LevelDiff` (`propagator.rs:58-66`) with `loaded/changed/unloaded: Vec<(IVec2, u32)|IVec2>` drives transitions. Unload path: `remove_column_chunks` (`lifecycle.rs:513`) serializes dirty chunks to `PendingSave`, removes octree data, removes `chunk_levels` entry; `despawn_out_of_range_chunks` (`lifecycle.rs:983`) kills the `VoxelChunk` mesh entity.
- **No ECS events anywhere in `voxel_map_engine`** — all transitions synchronous via chained systems.
- **Programmatic force-load**: spawn a headless entity with `ChunkTicket::new(map_entity, TicketType::Player, 0)` + `GlobalTransform` at the target position. **Force-evict**: despawn the ticket-holding entity.

## Q3: Ability RON pipeline

### Findings
- RON files at `assets/abilities/*.ability.ron` (~15 files). WASM manifest: `assets/abilities.manifest.ron`. Default loadout: `assets/default.ability_slots.ron`.
- `AbilityAssetLoader` at `crates/protocol/src/ability/loader.rs:59-62`; `FromWorld` clones `world.resource::<AppTypeRegistry>().0` at `loader.rs:64-70`. Extension `"ability.ron"` (`loader.rs:78`).
- `AssetLoader::load` (`loader.rs:81-92`) reads bytes, `self.type_registry.read()`, calls `crate::reflect_loader::deserialize_component_map(&bytes, &registry)` → `AbilityAsset { components: Vec<Box<dyn PartialReflect>> }`.
- Registered at `crates/protocol/src/ability/plugin.rs:47-48` via `init_asset::<AbilityAsset>()` + `init_asset_loader::<AbilityAssetLoader>()`.
- `ComponentMapDeserializer` at `crates/protocol/src/reflect_loader.rs:9-51`. `visit_map` iterates entries: key → `TypeRegistrationDeserializer::new(registry)` (line 37), value → `TypedReflectDeserializer::new(registration, registry)` (line 40), optional `ReflectFromReflect` upgrade (lines 42-47). Entry point `deserialize_component_map` at `reflect_loader.rs:56-64`.
- Component application: `apply_ability_archetype` at `loader.rs:24-56`. Queues world command that for each component: `registry.get_with_type_path(type_path)` → `registration.data::<ReflectComponent>()` → `reflect_component.insert(&mut entity_mut, component, &registry)`. Called from `activation.rs:103` and `spawn.rs:90`.
- Hot-reload: `bevy/file_watcher` feature enabled in client (`crates/client/Cargo.toml:7-8` via `default`) and server (`Cargo.toml:17` explicit). `AssetPlugin` configured only with `file_path` at `client/src/main.rs:38-41` and `server/src/main.rs:25-28` — no explicit `watch_for_changes_override`. **Web crate has no `file_watcher`.**
- `reload_ability_defs` (native `loading.rs:151-177`, WASM `loading.rs:179-219`) reads `MessageReader<AssetEvent<AbilityAsset>>`; on `Modified`, rebuilds `AbilityDefs` (`HashMap<AbilityId, Handle<AbilityAsset>>`).
- **Already-spawned `ActiveAbility` entities are NOT re-applied** — only future activations pick up changes.

## Q4: World-object RON pipeline

### Findings
- RON files at `assets/objects/*.object.ron` (currently `tree_circle`, `stump_circle`). WASM manifest at `assets/objects.manifest.ron`.
- `WorldObjectLoader` at `crates/protocol/src/world_object/loader.rs:11-13`; `FromWorld` identical pattern (`loader.rs:16-21`). Extension `"object.ron"` (`loader.rs:29`).
- `AssetLoader::load` (`loader.rs:32-43`) delegates to `deserialize_world_object` (`loader.rs:55-61`) which calls the **same shared `crate::reflect_loader::deserialize_component_map`**.
- Asset `WorldObjectDef { components: Vec<Box<dyn PartialReflect>> }` at `types.rs:97-101`. Custom `Clone` via `reflect_clone()` at `types.rs:103-117`. Error wrapper `WorldObjectLoadError` at `types.rs:136` with `From<ReflectLoadError>` at `types.rs:183-190`.
- Native loading: `load_world_object_defs` (`loading.rs:35-43`) → `asset_server.load_folder("objects")`. WASM: loads manifest then per-ID `asset_server.load` (`loading.rs:46-83`).
- `collect_object_defs` (`loading.rs:88-107`) builds `HashMap<WorldObjectId, WorldObjectDef>` and inserts `WorldObjectDefRegistry` resource (`registry.rs:12-13`).
- Component application: `apply_object_components` at `world_object/spawn.rs:8-21` — same `ReflectComponent::insert` loop as ability pipeline.
- Hot-reload: `reload_world_object_defs` at `loading.rs:164-185`. On `AssetEvent::Modified`, iterates `Assets<WorldObjectDef>` and overwrites `WorldObjectDefRegistry.objects`.
- **Already-spawned world-object entities are NOT re-patched.** Only `on_visual_kind_changed` (`client/src/world_object.rs:158`) reacts to `Changed<VisualKind>` — but that fires on replication change, not RON reload.

### Differences vs ability pipeline
- **Registry storage**: `WorldObjectDefRegistry` stores cloned `WorldObjectDef` values eagerly; `AbilityDefs` stores `Handle<AbilityAsset>` lazily (asset stays in `Assets<AbilityAsset>`).
- **apply fn location**: `apply_object_components` in separate `spawn.rs` file; `apply_ability_archetype` lives in loader file.
- **`clone_def_components` helper is duplicated** between `server/src/world_object.rs:134` and `client/src/world_object.rs:90` (identical impls). No ability equivalent.
- **Collider filtering**: world-object spawn filters `ColliderConstructor` when a vox mesh is present (`server/src/world_object.rs:47`, `client/src/world_object.rs:61`). No ability equivalent.
- **Error wrapper**: world-object has `WorldObjectLoadError`; ability uses `ReflectLoadError` directly.
- Hot-reload data sources differ: world-object reads `Assets<WorldObjectDef>` directly; ability re-queries the `LoadedFolder`.

## Q5: Server interest management

### Findings
- `RoomRegistry(pub HashMap<MapInstanceId, Entity>)` at `crates/server/src/map.rs:45-56`. `init_resource` at `map.rs:628`. `get_or_create` lazily spawns `Room::default()` (`map.rs:49-55`).
- Lightyear `Room { clients: EntityHashSet, entities: EntityHashSet }` at `git/lightyear/lightyear_replication/src/visibility/room.rs:62-67`.
- `get_or_create` callers: `on_map_instance_id_added` observer (`map.rs:606`), `handle_connected` (`gameplay.rs:420`), `start_map_transition` (`transition.rs:35-36`).
- Entity-room assignment: observer `on_map_instance_id_added` (`map.rs:596-612`) fires when any entity gains `MapInstanceId`. Inserts `NetworkVisibility` + triggers `RoomEvent { target: RoomTarget::AddEntity(entity) }`. All terrain objects and characters go through this.
- Client-room assignment: clients (sender entities / `ClientOf`) added via `RoomTarget::AddSender` only at `transition.rs:133-136` inside `complete_map_transition` (Phase 2). Initial connect goes through same path via `TransitionPending`.
- Client-room removal: `RoomTarget::RemoveSender` at `transition.rs:39-43` (Phase 1); disconnects cleaned by `RoomPlugin::handle_disconnect` observer (`room.rs:81-89`).
- Per-sender visibility actually stored in Lightyear's `ReplicationState.per_sender_state: EntityIndexMap<PerSenderReplicationState>` (`send/components.rs:614-619`). Application code **does not** call `replicate_to` directly — all visibility flows through `RoomEvent` triggers.
- Write path: `RoomPlugin::handle_room_event` observer (`room.rs:93-192`) updates `Room.clients/entities` and records gain/lose into `RoomEvents` with `shared_counts: EntityHashMap<EntityHashMap<u8>>` refcount. `RoomPlugin::apply_room_events` in `PostUpdate` (`room.rs:194-251`) calls `vis.gain_visibility(sender)` / `vis.lose_visibility(sender)` on each entity's `ReplicationState`.
- **No radius/AABB visibility.** Visibility is purely room-membership. Chunk streaming `push_chunks_to_clients` (`map.rs:886-940`) uses Chebyshev-distance radius (`compute_loaded_columns`, `map.rs:943-964`) for chunk data delivery, separate from entity replication.
- Character replication: `Replicate::to_clients(NetworkTarget::All)` + `PredictionTarget::to_clients(NetworkTarget::All)` at `gameplay.rs:398-399`. `NetworkTarget::All` = candidates; room membership filters the actual set. Map/world-object entities have no explicit `Replicate` — they're added via room observer.
- `ProtocolPlugin` (`protocol/src/lib.rs:157-208`) configures `add_prediction`, `add_should_rollback`, `add_linear_correction_fn`, `add_linear_interpolation` — rollback/interp, not visibility.

## Q6: AppTypeRegistry registrations and reflect pattern

### Findings
- **Ability plugin** (`crates/protocol/src/ability/plugin.rs:34-45`):
  - Components: `AbilityPhases`, `OnTickEffects`, `WhileActiveEffects`, `OnHitEffectDefs`, `OnEndEffects`, `OnInputEffects` (all `#[reflect(Component)]` in `ability/types.rs:141-325`).
  - Data-only: `TickEffect` (`types.rs:282`), `InputEffect` (`types.rs:308`), `AbilityEffect` (`types.rs:45`), `EffectTarget` (`types.rs:23`), `ForceFrame` (`types.rs:32`), `PlayerActions` (`lib.rs:59`).
- **World-object plugin** (`crates/protocol/src/world_object/plugin.rs:50-58`):
  - Components: `Health`, `RespawnTimerConfig`, `ObjectCategory`, `VisualKind`, `ColliderConstructor` (avian), `PlacementOffset`, `OnDeathEffects`, `ActiveTransformation`.
  - Data-only: `DeathEffect` (`world_object/types.rs:55`).
- **voxel_map_engine** (`crates/voxel_map_engine/src/lib.rs:32-41`):
  - Components: `MapDimensions`, `HeightMap`, `MoistureMap`, `BiomeRules`, `PlacementRules` (all `#[reflect(Component)]` in `config.rs`/`terrain.rs`).
  - Data-only: `BiomeRule`, `NoiseDef`, `NoiseType`, `FractalType`, `PlacementRule`.
- **Transition plugin** (`crates/protocol/src/transition/plugin.rs:11`): `TransitionPhase` (data-only enum).
- **`crates/protocol/src/lib.rs`** itself has no `register_type` calls — all come from sub-plugins.
- **Reflect → live entity pattern** (uniform across both pipelines):
  1. `TypeRegistrationDeserializer` resolves type path → `TypeRegistration` (`reflect_loader.rs:37`).
  2. `TypedReflectDeserializer` produces `Box<dyn PartialReflect>` (`reflect_loader.rs:40`).
  3. Optional `ReflectFromReflect` upgrade (`reflect_loader.rs:42-47`).
  4. Queue world command; for each component: `registry.get_with_type_path` → `.data::<ReflectComponent>()` → `.insert(&mut entity_mut, component, &registry)` (`loader.rs:40-55`, `spawn.rs:14-37`).
  - Requires type registered **and** `#[reflect(Component)]`.

## Q7: Cargo features and debug toggles

### Findings
- Workspace features: `client` has `default=["file_watcher"]`, `file_watcher`, `tracy` (`client/Cargo.toml:7-9`); `server` has `tracy` (`server/Cargo.toml:13-14`); `protocol` has marker `test_utils` (`protocol/Cargo.toml:7-8`); `voxel_map_engine` has `tracy` (`voxel_map_engine/Cargo.toml:7-8`).
- **No `[features]` in `dev`, `render`, `ui`, `web`, `sprite_rig`, `persistence`.**
- **No `#[cfg]` or `cfg!` gates in the `dev` crate** — it's wholly unconditional.
- `hide_physics_debug` (`crates/dev/src/lib.rs:19-22`): `store.config_mut::<PhysicsGizmos>()` → `config.enabled = false` in `Startup`.
- `toggle_physics_debug` (`lib.rs:25-30`): `keys.just_pressed(KeyCode::F3)` → `config.enabled = !config.enabled` every `Update`.
- State manipulated is Bevy's `GizmoConfigStore` (built-in) keyed by `PhysicsGizmos` marker. No custom resource/component introduced.
- `crates/dev/Cargo.toml:7` pulls `avian3d` with `debug-plugin` feature in regular `[dependencies]` — not gated. No `[dev-dependencies]`.

## Q8: Client-side replication markers

### Findings
- Lightyear markers used in `crates/client/src/`: `Replicated`, `Predicted`, `Interpolated`, `Controlled` (imported at `gameplay.rs:5`, `map.rs:3`). `ReplicationTarget`, `ClientId` not queried on client.
- `handle_new_character` (`gameplay.rs:35-81`): `Query<(Entity, Has<Controlled>), (Added<Replicated>, With<CharacterMarker>)>` decides owner vs. remote; `Query<..., Or<(Added<Predicted>, Added<Interpolated>)>>` adds physics bundle.
- `handle_character_movement` (`gameplay.rs:83-116`): movement only on `With<Predicted>` character.
- `sync_camera_yaw_to_input` (`gameplay.rs:167-178`): `Query<..., With<Predicted>>` for local player input.
- `map.rs` voxel handlers all use `Query<&ChunkTicket, (With<Predicted>, With<Controlled>, With<CharacterMarker>)>`.
- `attach_chunk_ticket_to_player` (`map.rs:74-92`): `(With<Predicted>, With<CharacterMarker>, Without<ChunkTicket>)`.
- Transitions: `Query<Entity, (With<Predicted>, With<CharacterMarker>, With<Controlled>)>` at `transition.rs:52`. `cleanup_stale_map_entities` (`transition.rs:397-412`) uses `With<Replicated>`.
- **Authoritative server mapping** of "entity → clients": stored in Lightyear-internal `ReplicationState.per_sender_state` (`pub(crate)`, not app-accessible). Application-accessible proxy is `Room.clients` + `Room.entities` (`pub` fields, `room.rs:62-67`) reachable via `Res<RoomRegistry>` → `Query<&Room>`. `flush_voxel_broadcasts` (`map.rs:838-842`) already uses this pattern to compute broadcast targets.

## Q9: Startup and plugin order

### Findings
- **`AppState`** at `crates/protocol/src/app_state.rs:5-9`: two variants `Loading` (default) / `Ready`. `TrackedAssets(Vec<UntypedHandle>)` resource at `app_state.rs:13`. `check_assets_loaded` (`app_state.rs:34-48`) runs `Update` while `Loading`, checks `asset_server.is_loaded_with_dependencies` on every handle, calls `next_state.set(AppState::Ready)` when all pass.
- **Client `add_plugins` order** (`client/main.rs`): `DefaultPlugins` → `ClientPlugins` (lightyear) → `SharedGameplayPlugin` → `ClientNetworkPlugin` → `ClientGameplayPlugin` → `ClientMapPlugin` → `ClientTransitionPlugin` → `RenderPlugin` → `UiPlugin` → **`DevPlugin` (position 10)** → `SharedDiagnosticsPlugin` → `ClientDiagnosticsPlugin`.
- **Server order** (`server/main.rs`): `MinimalPlugins` → `TerminalCtrlCHandlerPlugin` → `StatesPlugin` → `LogPlugin` → `AssetPlugin` → `TransformPlugin` → `ScenePlugin` → `ServerPlugins` → `SharedGameplayPlugin` → `ServerNetworkPlugin` → `ServerGameplayPlugin` → `ServerMapPlugin` → `SharedDiagnosticsPlugin` → `ServerDiagnosticsPlugin`. **No `DevPlugin`.**
- **Web order** (`web/main.rs`): `DefaultPlugins` (with `WindowPlugin` override) → `ClientPlugins` → `SharedGameplayPlugin` → `WebClientPlugin` → `ClientGameplayPlugin` → `ClientMapPlugin` → `RenderPlugin` → `UiPlugin` → **`DevPlugin` (position 9)**.
- On client/web, `DevPlugin` runs **after** all voxel/ability/network/UI plugins have been composed — any debug UI has full access to their resources and state.

## Q10: WASM constraints in web crate

### Findings
- `crates/web/src/main.rs:19` uses `DefaultPlugins` with no `AssetPlugin` override. `crates/web/Cargo.toml:15-38` does **not** include `file_watcher`. No `watch_for_changes_override` anywhere. **File watching is inactive on WASM.**
- Asset loading: `crates/web/index.html:24` has `<link data-trunk rel="copy-dir" href="../../assets"/>` — Trunk copies `assets/` into `dist/`; browser fetches over HTTP same-origin. Built via Bevy CLI wrapping Trunk (`Makefile.toml:70-81`). `crates/web/Cargo.toml:70-72` sets `rustflags = ["--cfg", "getrandom_backend=\"wasm_js\""]`.
- **No `IndexedDB`, `localStorage`, `FileSystem` usage anywhere** in the workspace. No browser-side RON persistence.
- `#[cfg(target_arch = "wasm32")]` gates all concentrated in `crates/protocol/`:
  - `ability/loading.rs`, `ability/plugin.rs` — `AbilityManifest` + `RonAssetPlugin` + per-ID handle-loading path.
  - `world_object/loading.rs`, `world_object/plugin.rs` — `WorldObjectManifest` + per-ID path.
  - `vox_model/loading.rs`, `vox_model/plugin.rs` — `VoxModelManifest`. **No WASM reload variant for vox_model** (native-only `reload_vox_models` at `vox_model/loading.rs:170`).
  - `terrain/loading.rs`, `terrain/plugin.rs` — `TerrainManifest`.
- Pattern: native `load_folder` → `LoadedFolder`; WASM `.manifest.ron` + explicit per-ID `asset_server.load`. Ability + world_object have WASM reload variants; vox_model does not.
- **No `"web"` Cargo feature** anywhere. WASM-specific code gated by `cfg(target_arch = "wasm32")`.
- `crates/web/.cargo/config.toml:1-3` and `.cargo/config.toml:22-28` set `web_sys_unstable_apis` + `target-feature=+reference-types` for `wasm32-unknown-unknown`.
- `[profile.wasm-test]` at `Cargo.toml:84-89` (`inherits=test`, `debug=false`, `strip=true`, `lto="thin"`). No `[profile.wasm-release]`.

## Q11: bevy-inspector-egui wasm constraints (current)

### Findings
- Bevy 0.18 released 2026-01-13. Compatible versions:
  - `bevy-inspector-egui 0.36.0` (2026-01-14) — targets Bevy 0.18, pulls `bevy_egui 0.39`.
  - `bevy_egui 0.39.1` (2026-02-06) — targets Bevy 0.18.
- Both officially support `wasm32-unknown-unknown`. `bevy_egui` ships WASM demo and has gated `web-sys`/`wasm-bindgen` deps.
- Open WASM-specific issues in `vladbat00/bevy_egui`:
  - **#246** — consolidates clipboard paste, non-QWERTY layout, emoji picker (open since Jan 2024).
  - **#247** — emoji picker positioning on WASM (open).
  - **#169** — non-QWERTY keyboard layout wrong on WASM (open).
  - **#196** — keyboard input issues (open).
- No open WASM issues in `jakobhellermann/bevy-inspector-egui` at time of research.
- Clipboard was previously forced-on and broken on WASM (inspector-egui #209, bevy_egui #113) — both closed; fix made `manage_clipboard` opt-in.
- No file dialog feature in either crate. No panic/compile failure reports on WASM at 0.36/0.39.
- **Required flags for WASM**: if enabling `manage_clipboard`, add `rustflags = ["--cfg=web_sys_unstable_apis"]` for `wasm32-unknown-unknown`. Workspace already sets `web_sys_unstable_apis` (`.cargo/config.toml:22-28`). `getrandom` `wasm_js` feature handled internally by `bevy_egui`. No `wayland` / `winit/web` flags required.

## Cross-Cutting Observations

- **Shared reflect infrastructure**: both ability and world-object loaders use `crate::reflect_loader::deserialize_component_map` and the same `ReflectComponent::insert` pattern. Clean symmetry except for registry-storage choice (eager clone vs. lazy handle) and helper duplication.
- **Hot-reload is half-wired**: both pipelines detect `AssetEvent::Modified` and refresh their registries, but neither re-applies components to already-spawned entities. Dev workflow has to despawn/respawn.
- **File watcher is native-only**: `bevy/file_watcher` feature enabled in client + server, absent from web crate. WASM cannot observe file changes.
- **Visibility model is purely room-based**: no radius/AABB visibility on entity replication. Chunk streaming has its own Chebyshev-distance mechanism independent of replication rooms.
- **`RoomRegistry.0`, `Room.clients`, `Room.entities` are all `pub`** — an on-server dev UI can read "which clients see this entity" via a single query with no Lightyear-internal access.
- **`DevPlugin` is last in `add_plugins`** on both client and web — debug systems run after all gameplay/voxel/network systems have registered. `DevPlugin` is absent on the server.
- **No existing feature flag or cfg gate in `DevPlugin`** — it's compiled unconditionally in client and web binaries today.

## Open Areas

- No ECS events for chunk state transitions exist — any "observe chunk became loaded" signal would want events, instead of polling or derived from `VoxelMapInstance.chunk_levels` changes.
- Lightyear's per-entity-per-sender `ReplicationState.per_sender_state` is `pub(crate)`; a dev UI on the server has to work with `Room` membership as a proxy (not strictly identical to what the sender will actually replicate, but in this codebase room-membership is the only filter above `NetworkTarget::All`).
- The `WorldObjectManifest` / `AbilityManifest` WASM load path is not exercised by any existing dev tooling — unknown whether individual RON files in `assets/abilities/` are enumerable on WASM without manifest regeneration.
