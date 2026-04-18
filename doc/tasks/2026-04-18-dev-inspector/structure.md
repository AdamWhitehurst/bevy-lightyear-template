# Structure Outline

## Approach

Umbrella `inspector` Cargo feature in `crates/dev` pulls `bevy_egui 0.39` + `bevy-inspector-egui 0.36`. Six per-panel features (`world-inspector`, `spawn-panel`, `netviz`, `chunk-debug`, `ability-editor`, `world-object-editor`) each depend on `inspector` and each honor a runtime `bool` in `DevInspectorState`, flipped by F-keys and by a top egui menu. Panels ship one-per-phase; each phase crosses Cargo → plugin wiring → panel UI → runtime verification.

## Phase 0: Inspector Foundation

Scaffolds deps + runtime state + root menu (F4). No panels yet — validates that egui compiles on native and `wasm32-unknown-unknown` and that zero-feature builds pull no new deps.

**Files**: `crates/dev/Cargo.toml`, `crates/dev/src/lib.rs`, `crates/dev/src/state.rs` (new), `crates/client/Cargo.toml`, `crates/web/Cargo.toml`

**Key changes**:
- `[features] inspector = ["dep:bevy_egui", "dep:bevy-inspector-egui"]` — optional deps
- `#[derive(Resource, Default)] struct DevInspectorState { enabled: bool, panels: PanelFlags }` with one `bool` per panel
- `fn toggle_dev_inspector(keys: Res<ButtonInput<KeyCode>>, state: ResMut<DevInspectorState>)` — F4 flips `enabled`
- `#[cfg(feature = "inspector")] fn draw_root_menu(state: ResMut<DevInspectorState>, mut ctx: EguiContexts)` — top bar with one checkbox per panel
- Client + web forward `inspector` to `dev/inspector`

**Verify**: `cargo check-all` passes; `cargo build -p dev` (no features) has no egui in `cargo tree`; `cargo check -p dev --features inspector --target wasm32-unknown-unknown` passes; `cargo client --features dev/inspector` → F3 still toggles physics, F4 shows empty top bar.

---

## Phase 1: World Inspector Panel

Drops in `WorldInspectorPlugin` behind `world-inspector` feature, gated by `state.panels.world_inspector`. Simplest possible slice — proves the toggle + run-condition pattern end-to-end.

**Files**: `crates/dev/Cargo.toml`, `crates/dev/src/panels/mod.rs` (new), `crates/dev/src/panels/world_inspector.rs` (new), `crates/dev/src/lib.rs`

**Key changes**:
- `[features] world-inspector = ["inspector"]`
- `fn world_inspector_enabled(state: Res<DevInspectorState>) -> bool`
- `WorldInspectorPlugin::default().run_if(world_inspector_enabled)`
- F5 flips `state.panels.world_inspector`

**Verify**: `cargo client --features dev/world-inspector` → F4 root, F5 opens the Bevy world tree, entity drill-down works.

---

## Phase 2: Spawn Panel (Dual-Mode)

Two tabs inside one panel. **Tab A** (def-driven): dropdowns over `WorldObjectDefRegistry` + `AbilityDefs` → spawn via existing `apply_object_components` / `apply_ability_archetype`. **Tab B** (free-form): walk `AppTypeRegistry`, filter to registrations exposing `ReflectComponent`, user picks N, instantiate via `ReflectDefault` + insert. Both spawn client-local entities (no `Replicate`) with a clearly-labeled "client-local dev spawn" badge. Shipped early because it exercises the reflect → live-entity path that ability/world-object editors build on in later phases.

**Files**: `crates/dev/Cargo.toml`, `crates/dev/src/panels/spawn.rs` (new), `crates/dev/src/lib.rs`

**Key changes**:
- `[features] spawn-panel = ["inspector"]`
- `enum SpawnTab { DefDriven, FreeForm }` + per-tab ui state
- `fn draw_spawn_def_tab(world_objects: Res<WorldObjectDefRegistry>, abilities: Res<AbilityDefs>, commands: Commands, ...)` — dropdown + `[Spawn at cursor]`
- `fn draw_spawn_freeform_tab(type_registry: Res<AppTypeRegistry>, commands: Commands, ...)` — filtered list, multi-select, spawn button
- F6 flips `state.panels.spawn_panel`

**Verify**: `cargo client --features dev/spawn-panel` — Tab A: pick `tree_circle`, click spawn → tree appears in world tree (client-local); Tab B: pick `Health`, click spawn → world inspector shows new entity carrying only `Health` component.

---

## Phase 3: Network Entity Viewer

Read-only panel: list entities with `Replicated`, showing `Replicated::from` + `Has<Predicted>`/`Has<Interpolated>`/`Has<Controlled>`. Pure observation, no mutation path.

**Files**: `crates/dev/Cargo.toml`, `crates/dev/src/panels/netviz.rs` (new), `crates/dev/src/lib.rs`

**Key changes**:
- `[features] netviz = ["inspector"]`
- `fn draw_netviz_panel(q: Query<(Entity, &Replicated, Has<Predicted>, Has<Interpolated>, Has<Controlled>)>, state: Res<DevInspectorState>, ctx: EguiContexts)` — egui table
- F7 flips `state.panels.netviz`

**Verify**: `cargo server &` + `cargo client --features dev/netviz` → F7 table lists local predicted character + interpolated world-objects with correct marker booleans; `Replicated::from` matches server sender entity.

---

## Phase 4: Chunk Debugger + `TicketType::Dev`

New `TicketType::Dev` variant (`level=0`, bounded radius) + `ChunkTicket::dev` constructor. Panel lists columns from `VoxelMapInstance.chunk_levels`, shows octree `ChunkStatus`, pins via a hidden dev-ticket entity, unpins via despawn.

**Files**: `crates/voxel_map_engine/src/ticket.rs`, `crates/dev/Cargo.toml`, `crates/dev/src/panels/chunk_debug.rs` (new), `crates/dev/src/lib.rs`

**Key changes**:
- `enum TicketType { Player, Npc, MapTransition, Dev }` — `base_level()` returns 0 for `Dev`; doc-comment the `LOAD_LEVEL_THRESHOLD` invariant
- `impl ChunkTicket { pub fn dev(map_entity: Entity, position: Vec3, radius: u32) -> impl Bundle }`
- `[features] chunk-debug = ["inspector"]`
- `#[derive(Component)] struct DevChunkPin { column: IVec2, ticket: Entity }` — one per user-pinned column
- `fn draw_chunk_panel(instance: Query<(Entity, &VoxelMapInstance)>, pins: Query<&DevChunkPin>, commands: Commands, ctx: EguiContexts)` — table of columns + `[Pin]`/`[Unpin]` buttons
- F8 flips `state.panels.chunk_debugger`

**Verify**: `cargo test -p voxel_map_engine` (ticket tests still pass); `cargo client --features dev/chunk-debug` → F8 lists active columns with `ChunkStatus`; `[Pin]` on an off-radius column → status reaches `Full` within one propagator cycle; `[Unpin]` → mesh entity despawns within one tick; F3 gizmos unaffected.

---

## Phase 5: Ability Editor + Re-Patch

Live RON editor per loaded `Handle<AbilityAsset>`. Save path: **native** writes bytes to source path and lets existing `bevy/file_watcher` fire `reload_ability_defs`; **wasm32** mutates `Assets<AbilityAsset>` and emits `AssetEvent::Modified`. Extend `reload_ability_defs` to re-run `apply_ability_archetype` on entities carrying the changed `AbilityId`.

**Files**: `crates/dev/Cargo.toml`, `crates/dev/src/panels/ability_editor.rs` (new), `crates/protocol/src/ability/loading.rs`, `crates/protocol/src/ability/loader.rs`, `crates/dev/src/lib.rs`

**Key changes**:
- `[features] ability-editor = ["inspector"]`
- In `ability/loader.rs`: `pub fn serialize_ability(asset: &AbilityAsset, registry: &TypeRegistry) -> Result<String, SerializeError>` via `ReflectSerializer`
- Panel: `HashMap<AbilityId, String>` buffer (one per loaded handle); on `[Save]`, parse via existing deserializer → on success: native `std::fs::write(source_path, bytes)`, wasm replace `Assets<_>` entry + `asset_events.write(AssetEvent::Modified { id })`
- Extend `reload_ability_defs`: after registry refresh, for each modified `AbilityId`, query entities spawned from that id and re-apply components via existing `ReflectComponent::insert` loop
- F9 flips `state.panels.ability_editor`

**Verify**: `cargo client --features dev/ability-editor` — edit `dash.ability.ron` cooldown in panel, save; activate dash on a character that existed before the edit → new cooldown takes effect (no respawn). `cargo web` build — same edit applies in-session; panel shows "unsaved (web)".

---

## Phase 6: World-Object Editor + Re-Patch

Mirror of Phase 5 against `Assets<WorldObjectDef>`. Extend `reload_world_object_defs` to re-patch. Research checkpoint inside this phase: confirm whether spawned world-object entities already carry their `WorldObjectId`; if not, add a `SpawnedFromDef(WorldObjectId)` marker in `protocol/src/world_object/` and insert on spawn in the existing server + client paths. If `clone_def_components` has to move to shared code for the re-patch, extract it into `protocol/src/world_object/` once (per design note).

**Files**: `crates/dev/Cargo.toml`, `crates/dev/src/panels/world_object_editor.rs` (new), `crates/protocol/src/world_object/loading.rs`, `crates/protocol/src/world_object/loader.rs`, `crates/protocol/src/world_object/mod.rs` (if marker/helper added), `crates/server/src/world_object.rs`, `crates/client/src/world_object.rs`, `crates/dev/src/lib.rs`

**Key changes**:
- `[features] world-object-editor = ["inspector"]`
- In `world_object/loader.rs`: `pub fn serialize_world_object(def: &WorldObjectDef, registry: &TypeRegistry) -> Result<String, SerializeError>`
- Panel: text buffer per def, same save fork (native disk, wasm in-memory + `AssetEvent`)
- Extend `reload_world_object_defs`: after registry refresh, re-apply components via `apply_object_components` to entities spawned from the changed def
- F10 flips `state.panels.world_object_editor`

**Verify**: `cargo server &` + `cargo client --features dev/world-object-editor` — edit `tree_circle.object.ron` health field, save; already-spawned trees update `Health` without despawn. `cargo web` build — in-session edit applies.

---

## Testing Checkpoints

- **After Phase 0**: no-feature `cargo build -p dev` unchanged in dep tree; `--features inspector` compiles native + wasm32; F4 shows empty bar; F3 physics unaffected.
- **After Phase 1**: `--features world-inspector` → F5 opens world tree; non-enabled panels still absent.
- **After Phase 2**: `--features spawn-panel` → both tabs spawn; free-form entities flagged client-local; validates reflect → live-entity path that Phases 5–6 reuse.
- **After Phase 3**: `--features netviz` → F7 table shows correct markers and sender.
- **After Phase 4**: `--features chunk-debug` → `TicketType::Dev` force-load reaches `Full`; despawn evicts within one tick; voxel tests green.
- **After Phase 5**: ability edits live-update pre-spawned entities on native + wasm; existing hot-reload for non-dev users unaffected.
- **After Phase 6**: world-object edits live-update pre-spawned entities; all six panels independently feature-gated and runtime-toggleable.
