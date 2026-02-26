# Hot-Reloadable `AbilitySlots` Asset Implementation Plan

## Overview

Replace the hardcoded `AbilitySlots` insertion in the server's player spawn with a hot-reloadable RON asset (`default.ability_slots.ron`). Players without their own `AbilitySlots` component fall back to this global default. The asset follows the exact `AbilityDefs` hot-reload pattern already established in the codebase.

## Current State Analysis

- `AbilitySlots` is a `Component` with no `Asset` or `TypePath` derives (`ability.rs:198`)
- Hardcoded insertion at server spawn: `gameplay.rs:173-178`
- Only consumer: `ability_activation` at `ability.rs:439`, queries `&AbilitySlots` (required)
- `AbilityDefs` pattern in `ability.rs:364-426` is the exact template to follow
- No `default.ability_slots.ron` asset file exists yet
- `bevy_common_assets::ron::RonAssetPlugin` already imported in `ability.rs:4`

## Desired End State

- `assets/default.ability_slots.ron` contains the global default loadout
- Modifying `default.ability_slots.ron` at runtime immediately affects all players without their own `AbilitySlots` component — no server/client restart required
- Server no longer inserts `AbilitySlots` during player spawn
- `ability_activation` uses the entity's `AbilitySlots` if present, otherwise falls back to `DefaultAbilitySlots` resource

**Verification**: modify `default.ability_slots.ron` while the game runs; the active character's abilities change on the next press (since they have no personal `AbilitySlots`).

### Key Discoveries

- `AbilityId(pub String)` serializes transparently in RON (serde newtype default)
- `AbilitySlots(pub [Option<AbilityId>; 4])` serializes as its inner array
- Bevy allows a type to implement both `Component` and `Asset`
- `Asset` does NOT require `Reflect` in Bevy 0.17
- `TrackedAssets` at `app_state.rs` must receive the new handle to gate startup on load

## What We're NOT Doing

- Per-character or per-class slot assets — single global default only
- Removing `AbilityCooldowns` from the server spawn
- Changing how clients receive `AbilitySlots` via replication when a personal component is present

---

## Phase 1: Asset Infrastructure

### Overview

Make `AbilitySlots` a Bevy asset; add the RON file, handle/resource types, and load/insert/reload systems. Mirrors the `AbilityDefs` pattern precisely.

### Changes Required

#### 1. `assets/default.ability_slots.ron` (new file)

```ron
([
    Some("dive_kick"),
    Some("speed_burst"),
    Some("shockwave"),
    None,
])
```

`AbilitySlots` is a newtype struct; serde serializes it as its inner value. `AbilityId` is also a newtype struct over `String`, serializing transparently.

#### 2. `crates/protocol/src/ability.rs` — `AbilitySlots` derives

```rust
// Before:
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AbilitySlots(pub [Option<AbilityId>; 4]);

// After:
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, Asset, TypePath)]
#[type_path = "protocol::ability"]
pub struct AbilitySlots(pub [Option<AbilityId>; 4]);
```

#### 3. `crates/protocol/src/ability.rs` — new type (add after `AbilityDefsHandle`)

Unlike `AbilityDefs` (which transforms `AbilityDefsAsset`), `AbilitySlots` IS the asset — no intermediate type needed. The handle resource is the resource; consumers look up the asset directly via `Assets<AbilitySlots>`.

```rust
/// Resource holding the handle for the global default ability slots asset.
/// Consumers look up the current value via `Assets<AbilitySlots>`.
#[derive(Resource)]
pub struct DefaultAbilitySlots(pub Handle<AbilitySlots>);
```

#### 4. `crates/protocol/src/ability.rs` — `AbilityPlugin::build`

No insert/reload systems needed — hot-reload is handled by Bevy's asset system automatically.

```rust
impl Plugin for AbilityPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RonAssetPlugin::<AbilityDefsAsset>::new(&["abilities.ron"]));
        app.add_plugins(RonAssetPlugin::<AbilitySlots>::new(&["ability_slots.ron"]));
        app.add_systems(Startup, (load_ability_defs, load_default_ability_slots));
        app.add_systems(Update, (insert_ability_defs, reload_ability_defs));
    }
}
```

#### 5. `crates/protocol/src/ability.rs` — new system (add after `reload_ability_defs`)

```rust
fn load_default_ability_slots(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<crate::app_state::TrackedAssets>,
) {
    let handle = asset_server.load::<AbilitySlots>("default.ability_slots.ron");
    tracked.add(handle.clone());
    commands.insert_resource(DefaultAbilitySlots(handle));
}
```

### Success Criteria

#### Automated Verification
- [x] `cargo check-all` compiles without errors

---

## Phase 2: Update `ability_activation`

### Overview

Change the query from `&AbilitySlots` (required) to `Option<&AbilitySlots>`, falling back to the `DefaultAbilitySlots` handle + `Assets<AbilitySlots>` lookup when absent.

### Changes Required

#### `crates/protocol/src/ability.rs` — `ability_activation`

```rust
pub fn ability_activation(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    default_slots: Res<DefaultAbilitySlots>,
    ability_slots_assets: Res<Assets<AbilitySlots>>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut query: Query<(
        Entity,
        &ActionState<PlayerActions>,
        Option<&AbilitySlots>,
        &mut AbilityCooldowns,
        &PlayerId,
    )>,
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();
    let default = ability_slots_assets
        .get(&default_slots.0)
        .expect("default.ability_slots.ron not loaded");

    for (entity, action_state, slots_opt, mut cooldowns, player_id) in &mut query {
        let slots = slots_opt.unwrap_or(default);
        // ... remainder unchanged
    }
}
```

`TrackedAssets` gating ensures the asset is loaded before this system runs, so the `expect` will not panic in practice. The rest of the loop body is unchanged.

### Success Criteria

#### Automated Verification
- [x] `cargo check-all` compiles without errors

---

## Phase 3: Remove Hardcoded Server Insertion

### Overview

Remove `AbilitySlots` from the `commands.spawn(...)` call in the server's player spawn handler. `AbilityCooldowns` remains.

### Changes Required

#### `crates/server/src/gameplay.rs:173-178`

Remove:
```rust
AbilitySlots([
    Some(AbilityId("dive_kick".into())),
    Some(AbilityId("speed_burst".into())),
    Some(AbilityId("shockwave".into())),
    None,
]),
```

Check if `AbilitySlots` import is still needed in `server/src/gameplay.rs` after removal — remove it if unused.

### Success Criteria

#### Automated Verification
- [x] `cargo check-all` compiles without errors
- [x] `cargo server` starts without panic

#### Manual Verification
- [ ] Connect a client; character can activate abilities using the default slots from `default.ability_slots.ron`
- [ ] Modify `default.ability_slots.ron` while server + client are running; character's available abilities update without restart
- [ ] No warnings about missing `AbilitySlots` or `DefaultAbilitySlots` in logs

---

## References

- Research: `doc/research/2026-02-25-ability-slots-hot-reload-asset.md`
- `AbilityDefs` pattern (template): `crates/protocol/src/ability.rs:359-426`
- Server spawn site: `crates/server/src/gameplay.rs:156-181`
- Consumer: `crates/protocol/src/ability.rs:439-497`
