# Implementation Plan

## Overview

Extend the `dev` crate's `DevPlugin` with six independently-toggleable `bevy-inspector-egui` panels, each gated by a Cargo feature and a runtime `bool` in `DevInspectorState`. The umbrella `inspector` Cargo feature pulls `bevy_egui 0.39` + `bevy-inspector-egui 0.36`; six per-panel features layer on top. Phases 5 and 6 also extend the existing `reload_*_defs` hot-reload systems to re-patch already-spawned entities.

## Conventions used throughout

- **Sequential cargo builds only.** CLAUDE.md mandates: no parallel `cargo build`/`check`/`test`. All verification commands below are single, serial invocations.
- **F-key map.** F4 = root menu / `enabled`; F5 = world-inspector; F6 = spawn-panel; F7 = netviz; F8 = chunk-debug; F9 = ability-editor; F10 = world-object-editor. (F3 keeps physics gizmos.)
- **Run-condition gating.** Panels are added with `.run_if(panel_enabled)` rather than bare-return in their systems — keeps CLAUDE.md's "no bare return without trace" rule clean.
- **Egui schedule.** All UI systems are added to `bevy_egui::EguiPrimaryContextPass`, matching lightyear's launcher/lobby examples.
- **WASM F-key caveat.** Browsers intercept F5 (refresh), F11 (fullscreen), F12 (devtools). On wasm the egui top menu (mouse-clickable checkboxes) is the primary toggle path; F-keys are best-effort.
- **Default-on for debug builds.** Each panel feature is also appended to `client`'s `[features] default = [...]` list so `cargo client` (debug) enables the inspector + all shipped panels. Release builds opt out via `cargo client --release --no-default-features --features file_watcher`. Web stays opt-in (no `default` list) to keep release WASM bundles lean.

---

## Phase 0: Inspector Foundation

### Changes

#### 1. `crates/dev/Cargo.toml`
**Action**: modify

Add `[features]` section and optional deps. Final file:

```toml
[package]
name = "dev"
version = "0.1.0"
edition = "2021"

[features]
inspector = ["dep:bevy_egui", "dep:bevy-inspector-egui"]

[dependencies]
avian3d = { workspace = true, features = ["debug-plugin"] }
bevy = { workspace = true, default-features = true }
bevy_egui = { version = "0.39", optional = true }
bevy-inspector-egui = { version = "0.36", optional = true }
```

#### 2. `crates/dev/src/state.rs`
**Action**: create

```rust
//! Runtime toggle state for `DevPlugin` debug panels.

use bevy::prelude::*;

/// Master toggle + per-panel toggles. F4 flips `enabled`; per-panel F-keys
/// flip the matching field in `panels`.
#[derive(Resource, Default)]
pub struct DevInspectorState {
    pub enabled: bool,
    pub panels: PanelFlags,
}

/// One `bool` per debug panel. A panel is drawn iff `state.enabled && state.panels.<field>`.
#[derive(Default)]
pub struct PanelFlags {
    pub world_inspector: bool,
    pub spawn_panel: bool,
    pub netviz: bool,
    pub chunk_debugger: bool,
    pub ability_editor: bool,
    pub world_object_editor: bool,
}
```

#### 3. `crates/dev/src/lib.rs`
**Action**: modify

Add module decls, state init, F4 toggle, and the inspector-feature-gated egui plugin + root menu. Final file:

```rust
//! Development-only tooling: physics debug rendering, runtime toggles, and
//! optional `bevy-inspector-egui` panels (behind the `inspector` Cargo feature).

use avian3d::prelude::{PhysicsDebugPlugin, PhysicsGizmos};
use bevy::gizmos::config::GizmoConfigStore;
use bevy::prelude::*;

mod state;
pub use state::{DevInspectorState, PanelFlags};

#[cfg(feature = "inspector")]
mod panels;

pub struct DevPlugin;

impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsDebugPlugin::default())
            .init_resource::<DevInspectorState>()
            .add_systems(Startup, hide_physics_debug)
            .add_systems(Update, (toggle_physics_debug, toggle_dev_inspector));

        #[cfg(feature = "inspector")]
        {
            use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
            app.add_plugins(EguiPlugin::default())
                .add_systems(EguiPrimaryContextPass, draw_root_menu.run_if(inspector_enabled));
        }
    }
}

fn hide_physics_debug(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<PhysicsGizmos>();
    config.enabled = false;
}

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

#[cfg(feature = "inspector")]
fn inspector_enabled(state: Res<DevInspectorState>) -> bool {
    state.enabled
}

#[cfg(feature = "inspector")]
fn draw_root_menu(
    mut state: ResMut<DevInspectorState>,
    mut contexts: bevy_egui::EguiContexts,
) {
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
            #[cfg(feature = "spawn-panel")]
            ui.checkbox(&mut state.panels.spawn_panel, "Spawn");
            #[cfg(feature = "netviz")]
            ui.checkbox(&mut state.panels.netviz, "Netviz");
            #[cfg(feature = "chunk-debug")]
            ui.checkbox(&mut state.panels.chunk_debugger, "Chunks");
            #[cfg(feature = "ability-editor")]
            ui.checkbox(&mut state.panels.ability_editor, "Abilities");
            #[cfg(feature = "world-object-editor")]
            ui.checkbox(&mut state.panels.world_object_editor, "Objects");
        });
    });
}
```

#### 4. `crates/dev/src/panels/mod.rs`
**Action**: create

```rust
//! Per-panel modules. Each is gated by its own Cargo feature so disabled
//! panels pay zero compile + zero runtime cost.

#[cfg(feature = "world-inspector")]
pub mod world_inspector;

#[cfg(feature = "spawn-panel")]
pub mod spawn;

#[cfg(feature = "netviz")]
pub mod netviz;

#[cfg(feature = "chunk-debug")]
pub mod chunk_debug;

#[cfg(feature = "ability-editor")]
pub mod ability_editor;

#[cfg(feature = "world-object-editor")]
pub mod world_object_editor;
```

(In Phase 0 only the `mod.rs` is created with all `mod` lines; the referenced files are added in Phases 1–6. The `cfg`-gates make each `mod` line inert until its feature lands.)

#### 5. `crates/client/Cargo.toml`
**Action**: modify

Add forwarding features under `[features]`:

```toml
[features]
default = ["file_watcher"]
file_watcher = ["bevy/file_watcher"]
tracy = ["bevy/trace_tracy", "tracy-client/enable"]
inspector = ["dev/inspector"]
```

(Per-panel forwarding features added in their respective phases.)

#### 6. `crates/web/Cargo.toml`
**Action**: modify

Web has no `[features]` block today. Add one (placed above `[dependencies]`):

```toml
[features]
inspector = ["dev/inspector"]
```

### Verification

#### Automated
- [x] `cargo check-all` passes (no features changed by default; baseline still green).
- [x] `cargo build -p dev` compiles with no `bevy_egui` or `bevy-inspector-egui` in the crate's dep graph (`cargo tree -p dev | grep -E 'egui'` returns nothing).
- [x] `cargo check -p dev --features inspector` passes (native).
- [x] `cargo check -p dev --features inspector --target wasm32-unknown-unknown` passes.

#### Manual
- [x] `cargo client` (debug, defaults = `file_watcher`, `inspector`) — F3 toggles physics; F4 shows empty top-bar "Dev Inspector"; F4 again hides it.
- [ ] `cargo client --release --no-default-features --features file_watcher` — F3 toggles physics; F4 does nothing visible (inspector compiled out).

---

## Phase 1: World Inspector Panel

### Changes

#### 1. `crates/dev/Cargo.toml`
**Action**: modify

Append to `[features]`:

```toml
world-inspector = ["inspector"]
```

#### 2. `crates/dev/src/panels/world_inspector.rs`
**Action**: create

```rust
//! World-tree inspector panel. Wraps `bevy_inspector_egui::quick::WorldInspectorPlugin`
//! with a runtime toggle.

use crate::state::DevInspectorState;
use bevy::prelude::*;
use bevy_inspector_egui::quick::WorldInspectorPlugin;

pub struct WorldInspectorPanelPlugin;

impl Plugin for WorldInspectorPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(WorldInspectorPlugin::new().run_if(world_inspector_enabled))
            .add_systems(Update, toggle_world_inspector);
    }
}

fn world_inspector_enabled(state: Res<DevInspectorState>) -> bool {
    state.enabled && state.panels.world_inspector
}

fn toggle_world_inspector(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<DevInspectorState>,
) {
    if keys.just_pressed(KeyCode::F5) {
        state.panels.world_inspector = !state.panels.world_inspector;
    }
}
```

#### 3. `crates/dev/src/lib.rs`
**Action**: modify

Inside the `#[cfg(feature = "inspector")]` block in `DevPlugin::build`, append:

```rust
#[cfg(feature = "world-inspector")]
app.add_plugins(panels::world_inspector::WorldInspectorPanelPlugin);
```

#### 4. `crates/client/Cargo.toml` and `crates/web/Cargo.toml`
**Action**: modify

Append to each `[features]` block:

```toml
world-inspector = ["dev/world-inspector"]
```

### Verification

#### Automated
- [x] `cargo check -p dev --features world-inspector` passes (native).
- [x] `cargo check -p dev --features world-inspector --target wasm32-unknown-unknown` passes.
- [x] `cargo check-all` still passes.

#### Manual
- [ ] `cargo client --features dev/world-inspector` — F4 shows root, F5 opens the world tree window; can drill into entities and view their components.

---

## Phase 2: Spawn Panel (Dual-Mode)

### Changes

#### 1. `crates/dev/Cargo.toml`
**Action**: modify

Append to `[features]`:

```toml
spawn-panel = ["inspector"]
```

#### 2. `crates/dev/src/panels/spawn.rs`
**Action**: create

```rust
//! Spawn panel. Two tabs:
//!   * **Def-driven**: pick a registered `WorldObjectId` / `AbilityId` and spawn via the
//!     existing `apply_object_components` / `apply_ability_archetype` pipelines.
//!   * **Free-form**: pick any reflected `Component` from the `AppTypeRegistry` and
//!     instantiate via `ReflectDefault`.
//! All spawns are client-local (no `Replicate`) at the world origin and carry a
//! `DevSpawned` marker.

use crate::state::DevInspectorState;
use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::reflect::{ReflectFromReflect, TypeRegistration};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use protocol::ability::{AbilityDefs, AbilityId};
use protocol::ability::loader::apply_ability_archetype;
use protocol::ability::types::AbilityAsset;
use protocol::world_object::registry::WorldObjectDefRegistry;
use protocol::world_object::spawn::apply_object_components;
use protocol::world_object::types::WorldObjectId;

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
    selected_ability: Option<AbilityId>,
    selected_freeform: Vec<String>, // type paths
}

pub struct SpawnPanelPlugin;

impl Plugin for SpawnPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnPanelUi>()
            .add_systems(Update, toggle_spawn_panel)
            .add_systems(EguiPrimaryContextPass, draw_spawn_panel.run_if(spawn_panel_enabled));
    }
}

fn spawn_panel_enabled(state: Res<DevInspectorState>) -> bool {
    state.enabled && state.panels.spawn_panel
}

fn toggle_spawn_panel(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<DevInspectorState>) {
    if keys.just_pressed(KeyCode::F6) {
        state.panels.spawn_panel = !state.panels.spawn_panel;
    }
}

fn draw_spawn_panel(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<SpawnPanelUi>,
    world_objects: Option<Res<WorldObjectDefRegistry>>,
    abilities: Option<Res<AbilityDefs>>,
    type_registry: Res<AppTypeRegistry>,
    ability_assets: Res<Assets<AbilityAsset>>,
    mut commands: Commands,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("draw_spawn_panel: EguiContexts not ready, skipping frame");
        return;
    };
    egui::Window::new("Spawn (client-local)").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut ui_state.tab, SpawnTab::DefDriven, "Def-driven");
            ui.selectable_value(&mut ui_state.tab, SpawnTab::FreeForm, "Free-form");
        });
        ui.separator();
        ui.label("Spawned at world origin; client-local (no Replicate).");
        ui.separator();
        match ui_state.tab {
            SpawnTab::DefDriven => draw_def_tab(
                ui,
                &mut ui_state,
                world_objects.as_deref(),
                abilities.as_deref(),
                &type_registry,
                &ability_assets,
                &mut commands,
            ),
            SpawnTab::FreeForm => draw_freeform_tab(
                ui,
                &mut ui_state,
                &type_registry,
                &mut commands,
            ),
        }
    });
}

fn draw_def_tab(
    ui: &mut egui::Ui,
    ui_state: &mut SpawnPanelUi,
    world_objects: Option<&WorldObjectDefRegistry>,
    abilities: Option<&AbilityDefs>,
    type_registry: &AppTypeRegistry,
    ability_assets: &Assets<AbilityAsset>,
    commands: &mut Commands,
) {
    ui.label("World Object");
    if let Some(reg) = world_objects {
        egui::ComboBox::from_id_source("world_object_picker")
            .selected_text(
                ui_state.selected_object.as_ref().map(|i| i.0.as_str()).unwrap_or("(pick)"),
            )
            .show_ui(ui, |ui| {
                for id in reg.objects.keys() {
                    ui.selectable_value(&mut ui_state.selected_object, Some(id.clone()), &id.0);
                }
            });
        if ui.button("Spawn world object").clicked() {
            if let Some(id) = &ui_state.selected_object {
                if let Some(def) = reg.objects.get(id) {
                    let entity = commands
                        .spawn((
                            id.clone(),
                            Transform::default(),
                            DevSpawned,
                            Name::new(format!("dev:{}", id.0)),
                        ))
                        .id();
                    let components = def.components.iter().map(|c| c.reflect_clone().unwrap()).collect();
                    apply_object_components(commands, entity, components, type_registry.0.clone());
                }
            }
        }
    } else {
        ui.label("(WorldObjectDefRegistry not yet loaded)");
    }
    ui.separator();
    ui.label("Ability");
    if let Some(defs) = abilities {
        egui::ComboBox::from_id_source("ability_picker")
            .selected_text(
                ui_state.selected_ability.as_ref().map(|i| i.0.as_str()).unwrap_or("(pick)"),
            )
            .show_ui(ui, |ui| {
                for id in defs.abilities.keys() {
                    ui.selectable_value(&mut ui_state.selected_ability, Some(id.clone()), &id.0);
                }
            });
        if ui.button("Spawn ability").clicked() {
            if let Some(id) = &ui_state.selected_ability {
                if let Some(handle) = defs.get(id) {
                    if let Some(asset) = ability_assets.get(handle) {
                        let entity = commands
                            .spawn((Transform::default(), DevSpawned, Name::new(format!("dev:{}", id.0))))
                            .id();
                        apply_ability_archetype(commands, entity, asset, type_registry.0.clone());
                    }
                }
            }
        }
    } else {
        ui.label("(AbilityDefs not yet loaded)");
    }
}

fn draw_freeform_tab(
    ui: &mut egui::Ui,
    ui_state: &mut SpawnPanelUi,
    type_registry: &AppTypeRegistry,
    commands: &mut Commands,
) {
    let registry = type_registry.read();
    let mut component_paths: Vec<&str> = registry
        .iter()
        .filter(|reg| reg.data::<ReflectComponent>().is_some())
        .map(|reg| reg.type_info().type_path())
        .collect();
    component_paths.sort();
    ui.label("Pick reflected Components (multi-select):");
    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
        for path in &component_paths {
            let mut checked = ui_state.selected_freeform.iter().any(|p| p == path);
            if ui.checkbox(&mut checked, *path).changed() {
                if checked {
                    ui_state.selected_freeform.push(path.to_string());
                } else {
                    ui_state.selected_freeform.retain(|p| p != path);
                }
            }
        }
    });
    if ui.button("Spawn with selected components").clicked() && !ui_state.selected_freeform.is_empty() {
        let entity = commands
            .spawn((Transform::default(), DevSpawned, Name::new("dev:freeform")))
            .id();
        let mut components: Vec<Box<dyn bevy::reflect::PartialReflect>> = Vec::new();
        for path in &ui_state.selected_freeform {
            if let Some(reg) = registry.get_with_type_path(path) {
                if let Some(default) = reg.data::<bevy::reflect::ReflectDefault>() {
                    components.push(default.default().into_partial_reflect());
                } else {
                    warn!("freeform spawn: type {path} has no ReflectDefault, skipping");
                }
            }
        }
        // Reuse the world-object component-application path; it's a generic
        // "insert these reflected components on entity" loop.
        apply_object_components(commands, entity, components, type_registry.0.clone());
    }
}
```

Notes:
- `apply_object_components` and `apply_ability_archetype` are imported from `protocol`. The structure said `apply_*` are visible; `apply_ability_archetype` is currently `pub(crate)` — bumped to `pub` in Phase 5 (its own change), but since spawn-panel needs it now, **it's bumped to `pub` in Phase 2**. Note this in the Phase 5 changes.
- `WorldObjectDefRegistry`/`AbilityDefs` may not exist yet during `AppState::Loading` — the panel handles `Option<Res<_>>` per CLAUDE.md exception (loading-time absence). Comment explains in code via the "(not yet loaded)" UI message.
- Free-form spawns reuse `apply_object_components` because that function is a generic "insert this list of reflected components on this entity" loop — no world-object semantics intrinsic to it.

#### 3. `crates/protocol/src/ability/loader.rs`
**Action**: modify

Change `apply_ability_archetype` visibility from `pub(crate)` to `pub`:

```rust
pub fn apply_ability_archetype(commands: &mut Commands, entity: Entity, asset: &AbilityAsset, registry: TypeRegistryArc)
```

#### 4. `crates/dev/Cargo.toml`
**Action**: modify

Add `protocol` and `bevy_egui` (already optional via `inspector`) as deps. The `inspector` feature already pulls `bevy_egui`. Add:

```toml
protocol = { workspace = true, optional = true }
```

And update the feature line to include `protocol`:

```toml
[features]
inspector = ["dep:bevy_egui", "dep:bevy-inspector-egui", "dep:protocol"]
```

(`protocol` is required by spawn-panel and later editor panels; pulling it under `inspector` keeps the umbrella the gate.)

#### 5. `crates/dev/src/lib.rs`
**Action**: modify

Inside the `#[cfg(feature = "inspector")]` block, append:

```rust
#[cfg(feature = "spawn-panel")]
app.add_plugins(panels::spawn::SpawnPanelPlugin);
```

#### 6. `crates/client/Cargo.toml` and `crates/web/Cargo.toml`
**Action**: modify

Append to each `[features]` block:

```toml
spawn-panel = ["dev/spawn-panel"]
```

### Verification

#### Automated
- [x] `cargo check -p dev --features spawn-panel` passes.
- [x] `cargo check -p dev --features spawn-panel --target wasm32-unknown-unknown` passes.
- [x] `cargo check-all` passes.

#### Manual
- [ ] `cargo client --features dev/spawn-panel` — F4, F6, Tab "Def-driven": pick `tree_circle`, click Spawn → tree appears at origin in world inspector; entity has `DevSpawned` + `WorldObjectId`.
- [ ] Same session, Tab "Free-form": pick `protocol::world_object::types::Health`, click Spawn → new entity with only `Health` + `Transform` + `DevSpawned`.

---

## Phase 3: Network Entity Viewer

### Changes

> **Deviation note**: Structure says display `Replicated::from`. Actual lightyear field is `Replicated::receiver: Entity` (the local entity holding the `ReplicationReceiver`). Same semantic — "which connection is this replicated through". Plan uses `receiver`.

#### 1. `crates/dev/Cargo.toml`
**Action**: modify

Append to `[features]`:

```toml
netviz = ["inspector"]
```

Add lightyear dep (needed for `Replicated`/`Predicted`/`Interpolated`/`Controlled` markers — not currently a `dev` dep):

```toml
lightyear = { workspace = true, optional = true, features = ["client", "replication"] }
```

Update `inspector` feature to also pull lightyear when netviz is on — but lightyear is heavyweight; instead gate it under `netviz` only:

```toml
[features]
netviz = ["inspector", "dep:lightyear"]
```

#### 2. `crates/dev/src/panels/netviz.rs`
**Action**: create

```rust
//! Network entity viewer. Read-only table of replicated entities and their markers.

use crate::state::DevInspectorState;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use lightyear::prelude::{Controlled, Interpolated, Predicted, Replicated};

pub struct NetvizPanelPlugin;

impl Plugin for NetvizPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, toggle_netviz)
            .add_systems(EguiPrimaryContextPass, draw_netviz_panel.run_if(netviz_enabled));
    }
}

fn netviz_enabled(state: Res<DevInspectorState>) -> bool {
    state.enabled && state.panels.netviz
}

fn toggle_netviz(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<DevInspectorState>) {
    if keys.just_pressed(KeyCode::F7) {
        state.panels.netviz = !state.panels.netviz;
    }
}

fn draw_netviz_panel(
    mut contexts: EguiContexts,
    q: Query<(
        Entity,
        &Replicated,
        Has<Predicted>,
        Has<Interpolated>,
        Has<Controlled>,
    )>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("draw_netviz_panel: EguiContexts not ready, skipping frame");
        return;
    };
    egui::Window::new("Netviz").show(ctx, |ui| {
        egui::Grid::new("netviz_grid")
            .striped(true)
            .num_columns(5)
            .show(ui, |ui| {
                ui.label("Entity");
                ui.label("Receiver");
                ui.label("Predicted");
                ui.label("Interpolated");
                ui.label("Controlled");
                ui.end_row();
                for (entity, replicated, predicted, interpolated, controlled) in &q {
                    ui.label(format!("{entity:?}"));
                    ui.label(format!("{:?}", replicated.receiver));
                    ui.label(if predicted { "Y" } else { "" });
                    ui.label(if interpolated { "Y" } else { "" });
                    ui.label(if controlled { "Y" } else { "" });
                    ui.end_row();
                }
            });
    });
}
```

#### 3. `crates/dev/src/lib.rs`
**Action**: modify

Append inside the `#[cfg(feature = "inspector")]` block:

```rust
#[cfg(feature = "netviz")]
app.add_plugins(panels::netviz::NetvizPanelPlugin);
```

#### 4. `crates/client/Cargo.toml` and `crates/web/Cargo.toml`
**Action**: modify

Append to each `[features]` block:

```toml
netviz = ["dev/netviz"]
```

### Verification

#### Automated
- [ ] `cargo check -p dev --features netviz` passes.
- [ ] `cargo check -p dev --features netviz --target wasm32-unknown-unknown` passes.
- [ ] `cargo check-all` passes.

#### Manual
- [ ] Sequentially: `cargo server` in one terminal, then `cargo client --features dev/netviz` in another. F4, F7 → table lists local character (Predicted+Controlled = Y) and replicated world-objects (Interpolated = Y); `Receiver` column shows the same entity ID across rows.

---

## Phase 4: Chunk Debugger + `TicketType::Dev`

### Changes

#### 1. `crates/voxel_map_engine/src/ticket.rs`
**Action**: modify

Add `Dev` variant + ctor + doc-comment the threshold invariant.

```rust
pub enum TicketType {
    Player,
    Npc,
    MapTransition,
    /// Dev-only force-load. Uses level 0 (Player-equivalent) so it always
    /// passes `LOAD_LEVEL_THRESHOLD` and actually triggers chunk loading.
    /// Default radius is bounded (2 columns) to limit blast radius from a
    /// runaway dev pin. See `LOAD_LEVEL_THRESHOLD` invariant note in the
    /// constants section below.
    Dev,
}
```

In the existing `impl TicketType` block, extend both arms:

```rust
pub fn base_level(self) -> u32 {
    match self {
        Player => 0,
        Npc => 1,
        MapTransition => 2,
        Dev => 0, // see TicketType::Dev doc-comment
    }
}
pub fn default_radius(self) -> u32 {
    match self {
        Player => 4,
        Npc => 1,
        MapTransition => 4,
        Dev => 2,
    }
}
```

In `impl ChunkTicket`, add:

```rust
/// Dev-only force-load ticket. Spawn an entity carrying this + `GlobalTransform`
/// to pin a column; despawn the entity to evict.
pub fn dev(map_entity: Entity, radius: u32) -> Self {
    Self::new(map_entity, TicketType::Dev, radius)
}
```

Add a doc-comment block near the existing `LOAD_LEVEL_THRESHOLD` constants:

```rust
/// Columns whose effective level is `<= LOAD_LEVEL_THRESHOLD` are loaded.
/// `TicketType::Dev` deliberately chooses level 0 so a dev pin always
/// satisfies this threshold — choosing a level above the threshold would
/// silently no-op.
pub const LOAD_LEVEL_THRESHOLD: u32 = 20;
```

(Keep the `#[cfg(test)]` variant and `MAX_LEVEL` unchanged.)

#### 2. `crates/dev/Cargo.toml`
**Action**: modify

Append to `[features]`:

```toml
chunk-debug = ["inspector", "dep:voxel_map_engine"]
```

Add dep:

```toml
voxel_map_engine = { workspace = true, optional = true }
```

#### 3. `crates/dev/src/panels/chunk_debug.rs`
**Action**: create

```rust
//! Chunk debug panel. Lists loaded columns and lets the user pin (force-load)
//! or unpin (despawn) a column via a `TicketType::Dev` ticket entity.

use crate::state::DevInspectorState;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use voxel_map_engine::instance::VoxelMapInstance;
use voxel_map_engine::ticket::ChunkTicket;

/// Marker for entities the chunk panel spawns to pin a column. Also tracks
/// the column so we can find the right entity to despawn on unpin.
#[derive(Component)]
pub struct DevChunkPin {
    pub column: IVec2,
}

pub struct ChunkDebugPanelPlugin;

impl Plugin for ChunkDebugPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, toggle_chunk_debug)
            .add_systems(EguiPrimaryContextPass, draw_chunk_panel.run_if(chunk_debug_enabled));
    }
}

fn chunk_debug_enabled(state: Res<DevInspectorState>) -> bool {
    state.enabled && state.panels.chunk_debugger
}

fn toggle_chunk_debug(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<DevInspectorState>) {
    if keys.just_pressed(KeyCode::F8) {
        state.panels.chunk_debugger = !state.panels.chunk_debugger;
    }
}

const CHUNK_SIZE_F: f32 = 16.0; // matches default chunk size; only used to derive a world position for GlobalTransform

fn draw_chunk_panel(
    mut contexts: EguiContexts,
    instances: Query<(Entity, &VoxelMapInstance)>,
    pins: Query<(Entity, &DevChunkPin)>,
    mut commands: Commands,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("draw_chunk_panel: EguiContexts not ready, skipping frame");
        return;
    };
    egui::Window::new("Chunks").show(ctx, |ui| {
        for (map_entity, instance) in &instances {
            ui.collapsing(format!("map {map_entity:?}"), |ui| {
                egui::Grid::new(("chunk_grid", map_entity))
                    .num_columns(3)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("Column");
                        ui.label("Level");
                        ui.label("Action");
                        ui.end_row();
                        let mut columns: Vec<(IVec2, u32)> =
                            instance.chunk_levels.iter().map(|(c, l)| (*c, *l)).collect();
                        columns.sort_by_key(|(c, _)| (c.x, c.y));
                        for (column, level) in columns {
                            ui.label(format!("({}, {})", column.x, column.y));
                            ui.label(level.to_string());
                            let pinned = pins.iter().find(|(_, p)| p.column == column);
                            if let Some((pin_entity, _)) = pinned {
                                if ui.button("Unpin").clicked() {
                                    commands.entity(pin_entity).despawn();
                                }
                            } else if ui.button("Pin").clicked() {
                                let world_pos = Vec3::new(
                                    column.x as f32 * CHUNK_SIZE_F,
                                    0.0,
                                    column.y as f32 * CHUNK_SIZE_F,
                                );
                                commands.spawn((
                                    ChunkTicket::dev(map_entity, 1),
                                    Transform::from_translation(world_pos),
                                    GlobalTransform::from_translation(world_pos),
                                    DevChunkPin { column },
                                    Name::new(format!("dev_chunk_pin {column:?}")),
                                ));
                            }
                            ui.end_row();
                        }
                    });
            });
        }
    });
}
```

Note: `instance.chunk_size` is the authoritative chunk size; could be threaded in. For dev-purpose pin placement, the static `CHUNK_SIZE_F = 16.0` is acceptable — `collect_tickets` reads `GlobalTransform.translation()` and converts to a column via the instance's chunk size, so even an off-by-one world position lands the right column as long as it's within bounds. Acceptable simplification.

#### 4. `crates/dev/src/lib.rs`
**Action**: modify

Append inside `#[cfg(feature = "inspector")]` block:

```rust
#[cfg(feature = "chunk-debug")]
app.add_plugins(panels::chunk_debug::ChunkDebugPanelPlugin);
```

#### 5. `crates/client/Cargo.toml` and `crates/web/Cargo.toml`
**Action**: modify

Append to each `[features]` block:

```toml
chunk-debug = ["dev/chunk-debug"]
```

### Verification

#### Automated
- [ ] `cargo test -p voxel_map_engine` passes (existing ticket tests must still pass with the new `Dev` variant).
- [ ] `cargo check -p dev --features chunk-debug` passes.
- [ ] `cargo check -p dev --features chunk-debug --target wasm32-unknown-unknown` passes.
- [ ] `cargo check-all` passes.

#### Manual
- [ ] `cargo server` then `cargo client --features dev/chunk-debug`. F4, F8 → table lists active columns with their level. Click "Pin" on an off-radius column → after one or two propagator cycles, the column appears with a level matching `Dev`'s base; the world inspector shows a new mesh entity in that column. Click "Unpin" → mesh entity despawns within one tick.
- [ ] F3 physics gizmo toggle still works independently.

---

## Phase 5: Ability Editor + Re-Patch

### Changes

#### 1. `crates/dev/Cargo.toml`
**Action**: modify

Append to `[features]`:

```toml
ability-editor = ["inspector"]
```

(`protocol` already pulled via `inspector`. No new deps.)

#### 2. `crates/protocol/src/ability/loader.rs`
**Action**: modify

Add a serializer counterpart to the existing deserializer. Append to the file:

```rust
use bevy::reflect::serde::ReflectSerializer;
use ron::ser::PrettyConfig;

#[derive(Debug, thiserror::Error)]
pub enum AbilitySerializeError {
    #[error("ron serialize: {0}")]
    Ron(#[from] ron::Error),
}

/// Re-emit an `AbilityAsset` as RON in the same `{ "type::Path": value, ... }` shape
/// the deserializer accepts. Output may not be byte-identical to a hand-written
/// file — first save will reformat. Acceptable for dev use.
pub fn serialize_ability(
    asset: &AbilityAsset,
    registry: &TypeRegistry,
) -> Result<String, AbilitySerializeError> {
    use ron::ser::to_string_pretty;
    let mut entries: Vec<(String, ron::Value)> = Vec::with_capacity(asset.components.len());
    for component in &asset.components {
        let type_path = component.reflect_type_path().to_string();
        let serializer = ReflectSerializer::new(component.as_ref(), registry);
        let value: ron::Value = ron::from_str(&to_string_pretty(&serializer, PrettyConfig::default())?)?;
        // Strip the outer `{ "type": value }` wrapper that ReflectSerializer adds.
        let inner = match value {
            ron::Value::Map(map) => {
                let mut iter = map.iter();
                iter.next().map(|(_, v)| v.clone()).ok_or_else(|| {
                    ron::Error::Message("ReflectSerializer produced empty map".into())
                })?
            }
            _ => value,
        };
        entries.push((type_path, inner));
    }
    let mut map = ron::Map::new();
    for (k, v) in entries {
        map.insert(ron::Value::String(k), v);
    }
    Ok(format!(
        "#![enable(implicit_some)]\n{}",
        to_string_pretty(&ron::Value::Map(map), PrettyConfig::default())?
    ))
}
```

If `ron` doesn't expose `Map`/`Value` insertion the way above suggests, fall back to: build the type-keyed string by iterating entries, calling `to_string_pretty(&ReflectSerializer::new(...))` for each, and concatenating with `"<TypePath>": <value>,` lines wrapped in `{ ... }`. This preserves the on-disk format used in `assets/abilities/dash.ability.ron`.

(`ron 0.12` API surface — confirm `ron::Value` API at implementation time. If the typed-value approach is unwieldy, the string-concatenation fallback is plain enough to inline.)

Add `thiserror = "1"` to `protocol`'s deps if not already present (check `crates/protocol/Cargo.toml` first; if absent, use a hand-rolled `enum` with `Display`/`Error` impls instead — no new workspace dep needed).

#### 3. `crates/protocol/src/ability/loading.rs`
**Action**: modify

Extend both native and wasm `reload_ability_defs` to re-patch live entities. After the existing registry-refresh code, append (in both variants):

```rust
fn repatch_active_abilities(
    commands: &mut Commands,
    asset_server: &AssetServer,
    ability_assets: &Assets<AbilityAsset>,
    type_registry: TypeRegistryArc,
    active: &Query<(Entity, &ActiveAbility)>,
    changed_id: &AbilityId,
) {
    let Some(handle) = /* defs.get(changed_id) */ ... else { return; };
    let Some(asset) = ability_assets.get(handle) else { return; };
    for (entity, active_ability) in active.iter() {
        if &active_ability.def_id == changed_id {
            crate::ability::loader::apply_ability_archetype(
                commands,
                entity,
                asset,
                type_registry.clone(),
            );
        }
    }
}
```

Concretely: change `reload_ability_defs`'s signature to additionally take `Query<(Entity, &ActiveAbility)>`, `Res<Assets<AbilityAsset>>`, and `Res<AppTypeRegistry>`; on each `AssetEvent::Modified`, look up the changed `AbilityId` (via `asset_server.get_path(*id).and_then(ability_id_from_path)`), then iterate `ActiveAbility` entities and call `apply_ability_archetype` for each match. `apply_ability_archetype` is `pub` (bumped in Phase 2).

#### 4. `crates/dev/src/panels/ability_editor.rs`
**Action**: create

```rust
//! Ability RON editor. One text buffer per loaded `Handle<AbilityAsset>`.
//! Save path:
//!   * native: writes bytes to the asset's source file; `bevy/file_watcher`
//!     fires `reload_ability_defs`, which re-patches live entities.
//!   * wasm: mutates `Assets<AbilityAsset>` directly and emits
//!     `AssetEvent::Modified` so the same reload path runs.

use crate::state::DevInspectorState;
use bevy::asset::AssetEvent;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use protocol::ability::types::{AbilityAsset, AbilityId};
use protocol::ability::AbilityDefs;
use protocol::ability::loader::{deserialize_ability_components, serialize_ability};
use std::collections::HashMap;

#[derive(Resource, Default)]
struct AbilityEditorUi {
    buffers: HashMap<AbilityId, String>,
    selected: Option<AbilityId>,
    last_status: HashMap<AbilityId, String>,
}

pub struct AbilityEditorPlugin;

impl Plugin for AbilityEditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AbilityEditorUi>()
            .add_systems(Update, toggle_ability_editor)
            .add_systems(EguiPrimaryContextPass, draw_ability_editor.run_if(ability_editor_enabled));
    }
}

fn ability_editor_enabled(state: Res<DevInspectorState>) -> bool {
    state.enabled && state.panels.ability_editor
}

fn toggle_ability_editor(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<DevInspectorState>) {
    if keys.just_pressed(KeyCode::F9) {
        state.panels.ability_editor = !state.panels.ability_editor;
    }
}

fn draw_ability_editor(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<AbilityEditorUi>,
    defs: Option<Res<AbilityDefs>>,
    mut ability_assets: ResMut<Assets<AbilityAsset>>,
    type_registry: Res<AppTypeRegistry>,
    asset_server: Res<AssetServer>,
    mut events: MessageWriter<AssetEvent<AbilityAsset>>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        trace!("draw_ability_editor: EguiContexts not ready, skipping frame");
        return;
    };
    let Some(defs) = defs.as_deref() else {
        egui::Window::new("Ability Editor").show(ctx, |ui| {
            ui.label("(AbilityDefs not loaded yet)");
        });
        return;
    };
    egui::Window::new("Ability Editor").show(ctx, |ui| {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_source("ability_editor_picker")
                .selected_text(
                    ui_state.selected.as_ref().map(|i| i.0.as_str()).unwrap_or("(pick)"),
                )
                .show_ui(ui, |ui| {
                    for id in defs.abilities.keys() {
                        ui.selectable_value(&mut ui_state.selected, Some(id.clone()), &id.0);
                    }
                });
            #[cfg(target_arch = "wasm32")]
            ui.label(egui::RichText::new("unsaved (web)").color(egui::Color32::YELLOW));
        });
        let Some(id) = ui_state.selected.clone() else { return; };
        let Some(handle) = defs.get(&id) else { return; };
        // Lazily populate buffer with serialized current asset on first open.
        if !ui_state.buffers.contains_key(&id) {
            if let Some(asset) = ability_assets.get(handle) {
                let registry = type_registry.read();
                match serialize_ability(asset, &registry) {
                    Ok(s) => { ui_state.buffers.insert(id.clone(), s); }
                    Err(e) => { ui_state.last_status.insert(id.clone(), format!("serialize error: {e}")); }
                }
            }
        }
        let buffer = ui_state.buffers.entry(id.clone()).or_default();
        egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(buffer).code_editor().desired_rows(20));
        });
        if let Some(status) = ui_state.last_status.get(&id) {
            ui.label(status);
        }
        if ui.button("Save").clicked() {
            let registry = type_registry.read();
            match deserialize_ability_components(buffer.as_bytes(), &registry) {
                Ok(components) => {
                    let new_asset = AbilityAsset { components };
                    let asset_id = handle.id();
                    drop(registry);
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if let Some(path) = asset_server.get_path(asset_id) {
                            match std::fs::write(path.path(), buffer.as_bytes()) {
                                Ok(()) => {
                                    ui_state.last_status.insert(id.clone(), "saved (file_watcher will reload)".into());
                                }
                                Err(e) => {
                                    ui_state.last_status.insert(id.clone(), format!("write failed: {e}"));
                                }
                            }
                        } else {
                            ui_state.last_status.insert(id.clone(), "no source path for asset".into());
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        ability_assets.insert(asset_id, new_asset);
                        events.write(AssetEvent::Modified { id: asset_id });
                        ui_state.last_status.insert(id.clone(), "applied in-memory (session only)".into());
                    }
                    let _ = (ability_assets, asset_server, asset_id, new_asset, events); // suppress unused on platform
                }
                Err(e) => {
                    ui_state.last_status.insert(id.clone(), format!("parse error: {e}"));
                }
            }
        }
    });
}
```

Implementation notes:
- `deserialize_ability_components` does not exist as a standalone fn today; the existing entry point is `crate::reflect_loader::deserialize_component_map`. Re-export it from `protocol::ability::loader` as `pub use crate::reflect_loader::deserialize_component_map as deserialize_ability_components;`, **or** call `deserialize_component_map` directly. The plan uses the re-export to keep the panel's import surface tidy.
- The `let _ = (...)` line suppresses the unused-variable lints on either platform after the cfg-fork.
- On wasm, `Assets::insert` already fires `AssetEvent::Modified`, but the explicit `events.write(...)` matches structure.md's specification (idempotent — duplicate `Modified` events are processed identically by the reload system).

#### 5. `crates/protocol/src/ability/loader.rs`
**Action**: modify (cont.)

Add the re-export near the top of the file:

```rust
pub use crate::reflect_loader::deserialize_component_map as deserialize_ability_components;
```

#### 6. `crates/dev/src/lib.rs`
**Action**: modify

Append inside `#[cfg(feature = "inspector")]` block:

```rust
#[cfg(feature = "ability-editor")]
app.add_plugins(panels::ability_editor::AbilityEditorPlugin);
```

#### 7. `crates/client/Cargo.toml` and `crates/web/Cargo.toml`
**Action**: modify

Append to each `[features]` block:

```toml
ability-editor = ["dev/ability-editor"]
```

### Verification

#### Automated
- [ ] `cargo check -p protocol` passes (loader changes).
- [ ] `cargo check -p dev --features ability-editor` passes.
- [ ] `cargo check -p dev --features ability-editor --target wasm32-unknown-unknown` passes.
- [ ] `cargo test -p protocol` passes (existing ability/world-object tests must still pass; assertions don't depend on serializer output format).
- [ ] `cargo check-all` passes.

#### Manual
- [ ] `cargo client --features dev/ability-editor` — F4, F9. Pick `dash`. Edit the cooldown number in the buffer. Click Save → status reads "saved (file_watcher will reload)". Activate dash on a character that existed before the edit → new cooldown takes effect, no respawn.
- [ ] `cargo web-build --features dev/ability-editor` then run via `cargo web` (Trunk serve). Open in browser, F4/F9 (or top-bar checkbox if F9 intercepted). Edit cooldown, Save → status reads "applied in-memory (session only)". Same in-session behavior change.

---

## Phase 6: World-Object Editor + Re-Patch

### Changes

#### 1. `crates/dev/Cargo.toml`
**Action**: modify

Append to `[features]`:

```toml
world-object-editor = ["inspector"]
```

#### 2. `crates/protocol/src/world_object/mod.rs`
**Action**: modify

Per design.md "no third copy of `clone_def_components`": extract the helper into `protocol::world_object`. Add a new module:

```rust
pub mod components;
```

#### 3. `crates/protocol/src/world_object/components.rs`
**Action**: create

```rust
//! Shared helpers for cloning the reflected component list from a `WorldObjectDef`.
//! Extracted from server + client copies to support the dev editor's re-patch path.

use crate::world_object::types::WorldObjectDef;
use avian3d::prelude::ColliderConstructor;
use bevy::reflect::PartialReflect;

/// Deep-clone the reflected components from a def. When `filter_collider_constructor`
/// is true, drops any `ColliderConstructor` entries (caller already has a vox-derived
/// collider).
pub fn clone_def_components(
    def: &WorldObjectDef,
    filter_collider_constructor: bool,
) -> Vec<Box<dyn PartialReflect>> {
    def.components
        .iter()
        .filter(|c| {
            if !filter_collider_constructor {
                return true;
            }
            c.reflect_type_path() != std::any::type_name::<ColliderConstructor>()
        })
        .map(|c| c.reflect_clone().unwrap())
        .collect()
}
```

(Confirm at implementation time: `ColliderConstructor`'s reflect type-path matches `std::any::type_name::<_>()`. If not, port the exact filter logic from `server/src/world_object.rs:134-149` verbatim.)

#### 4. `crates/server/src/world_object.rs`
**Action**: modify

Delete the local `clone_def_components` definition; replace its call sites with `protocol::world_object::components::clone_def_components(...)`.

#### 5. `crates/client/src/world_object.rs`
**Action**: modify

Same as #4: delete local def, replace call sites with the protocol-side helper.

#### 6. `crates/protocol/src/world_object/loader.rs`
**Action**: modify

Add `serialize_world_object` mirroring the ability serializer. Append:

```rust
use bevy::reflect::serde::ReflectSerializer;
use ron::ser::PrettyConfig;

pub fn serialize_world_object(
    def: &WorldObjectDef,
    registry: &TypeRegistry,
) -> Result<String, WorldObjectLoadError> {
    // Same shape as serialize_ability in ability/loader.rs — type-keyed map RON
    // wrapped in `#![enable(implicit_some)]`. See that fn's notes for the
    // string-concatenation fallback if the typed-value path is awkward in ron 0.12.
    todo!("mirror serialize_ability — emit type-keyed RON map of components")
}
```

Implementation note: the serializer body is parallel to `serialize_ability`. Lift the shared core into `crate::reflect_loader` as `serialize_component_map(components, registry) -> Result<String, ron::Error>` if duplication is bothersome — but not required for correctness.

Add re-export for the editor panel to import:

```rust
pub use crate::reflect_loader::deserialize_component_map as deserialize_world_object_components;
```

Extend `WorldObjectLoadError` with a `Serialize` variant if the serializer path needs distinct error reporting; otherwise reuse the existing `Ron` variant.

#### 7. `crates/protocol/src/world_object/loading.rs`
**Action**: modify

Extend `reload_world_object_defs` to re-patch. New signature:

```rust
pub(super) fn reload_world_object_defs(
    mut events: MessageReader<AssetEvent<WorldObjectDef>>,
    object_assets: Res<Assets<WorldObjectDef>>,
    asset_server: Res<AssetServer>,
    mut registry: ResMut<WorldObjectDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
    spawned: Query<(Entity, &WorldObjectId)>,
    mut commands: Commands,
)
```

After the existing registry-refresh loop, for each `Modified` event, derive the `WorldObjectId` from the asset path (mirror `object_id_from_path` already used in this file), then iterate `spawned` and for matches call:

```rust
let components = crate::world_object::components::clone_def_components(def, /* filter_collider_constructor: */ false);
crate::world_object::spawn::apply_object_components(&mut commands, entity, components, type_registry.0.clone());
```

The `false` for `filter_collider_constructor` means the re-patch keeps whatever collider was in the def — re-patch is intended for stat/component edits, not vox-collider swap. (Document inline.)

#### 8. `crates/dev/src/panels/world_object_editor.rs`
**Action**: create

Mirror `ability_editor.rs` byte-for-byte with:
- `AbilityDefs` → `WorldObjectDefRegistry` (note: registry stores owned defs, not handles; lookup of the `Handle<WorldObjectDef>` for native disk-write must go through `Assets<WorldObjectDef>::ids()` + `asset_server.get_path(id)` filtered to the matching `WorldObjectId`)
- `AbilityAsset` → `WorldObjectDef`
- `AbilityId` → `WorldObjectId`
- `serialize_ability` → `serialize_world_object`
- `deserialize_ability_components` → `deserialize_world_object_components`
- `F9` → `F10`
- Save action: parse buffer → `WorldObjectDef { components }` → native: write to source file by looking up the asset's `Handle<WorldObjectDef>` and resolving its path; wasm: `assets.insert(id, new_def)` + `events.write(AssetEvent::Modified { id })`.

Concrete handle-lookup helper for native write path (inside `draw_world_object_editor`):

```rust
fn handle_for_id<'a>(
    target_id: &WorldObjectId,
    object_assets: &'a Assets<WorldObjectDef>,
    asset_server: &AssetServer,
) -> Option<bevy::asset::AssetId<WorldObjectDef>> {
    object_assets.ids().find(|id| {
        asset_server
            .get_path(*id)
            .and_then(|p| object_id_from_path(&p))
            .as_ref() == Some(target_id)
    })
}
```

(`object_id_from_path` is `pub(super)` in `loading.rs`; bump to `pub` and re-export from `world_object::mod` for the panel's use, or duplicate the trivial path-suffix-stripping logic inline. Plan: bump to `pub`.)

#### 9. `crates/dev/src/lib.rs`
**Action**: modify

Append inside `#[cfg(feature = "inspector")]` block:

```rust
#[cfg(feature = "world-object-editor")]
app.add_plugins(panels::world_object_editor::WorldObjectEditorPlugin);
```

#### 10. `crates/client/Cargo.toml` and `crates/web/Cargo.toml`
**Action**: modify

Append to each `[features]` block:

```toml
world-object-editor = ["dev/world-object-editor"]
```

### Verification

#### Automated
- [ ] `cargo check -p protocol` passes (loader + extracted helper).
- [ ] `cargo check -p server` passes (helper extraction; call site swap).
- [ ] `cargo check -p client` passes (helper extraction; call site swap).
- [ ] `cargo check -p dev --features world-object-editor` passes.
- [ ] `cargo check -p dev --features world-object-editor --target wasm32-unknown-unknown` passes.
- [ ] `cargo test -p protocol` passes.
- [ ] `cargo test-all` passes.
- [ ] `cargo check-all` with all features enabled (`--features dev/world-inspector,dev/spawn-panel,dev/netviz,dev/chunk-debug,dev/ability-editor,dev/world-object-editor`) passes.

#### Manual
- [ ] `cargo server` then `cargo client --features dev/world-object-editor`. F4, F10. Pick `tree_circle`. Edit `Health.current` to `999`. Save → status reads "saved (file_watcher will reload)". Existing tree entities update `Health` in place (verify via world inspector panel if both features are on; otherwise via gameplay).
- [ ] `cargo web-build --features dev/world-object-editor` then `cargo web`. In browser, edit `tree_circle` health, Save → status reads "applied in-memory (session only)"; in-session change visible.
- [ ] All six panels enabled simultaneously (`cargo client --features dev/world-inspector,dev/spawn-panel,dev/netviz,dev/chunk-debug,dev/ability-editor,dev/world-object-editor`): F4 → top bar shows all six checkboxes; F5–F10 each toggle their respective panel; mouse-clicking a checkbox is equivalent.
- [ ] README.md updated to document the new Cargo features and F-key map (per CLAUDE.md "MUST review README.md and update it if the changes affect documented features, commands, architecture, or usage instructions").

---

## Cross-phase notes

### Codegen / fallback
No codegen step in this task. All RON serialization runs at runtime via `bevy_reflect::serde::ReflectSerializer`. If `ReflectSerializer` output format diverges in surprising ways from hand-written RON (open risk in design.md), the serializer keeps a string-concatenation fallback — see Phase 5 #2 implementation notes.

### Schema migrations
None. Asset schemas are unchanged. Existing tests that load `dash.ability.ron` etc. continue to pass.

### Final all-features verification
After Phase 6, before declaring done:

- [ ] `cargo check-all` passes.
- [ ] `cargo build-all` passes.
- [ ] `cargo test-all` passes.
- [ ] `cargo web-build` passes (no inspector features).
- [ ] `cargo web-build --features dev/world-inspector,dev/spawn-panel,dev/netviz,dev/chunk-debug,dev/ability-editor,dev/world-object-editor` passes.
- [ ] `cargo build -p dev` (no features) — `cargo tree -p dev` shows no `bevy_egui` / `bevy-inspector-egui` / `lightyear` (only `avian3d` + `bevy`).
