---
date: 2026-02-25T17:58:29-08:00
researcher: Claude Sonnet 4.6
git_commit: 6ea3cb67c02cc89aece911924ed7f8bea1427f72
branch: master
repository: bevy-lightyear-template
topic: "Hot-reloadable AbilitySlots asset — fallback for players without own AbilitySlots component"
tags: [research, codebase, ability-slots, hot-reload, assets, ron, player-spawning]
status: complete
last_updated: 2026-02-25
last_updated_by: Claude Sonnet 4.6
last_updated_note: "Resolved open questions via user follow-up"
---

# Research: Hot-reloadable AbilitySlots Asset

**Date**: 2026-02-25T17:58:29-08:00 **Researcher**: Claude Sonnet 4.6 **Git Commit**: `6ea3cb67c02cc89aece911924ed7f8bea1427f72` **Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

Add a hot-reloadable `AbilitySlots` asset that is read from (not inserted on) when players don't have their own `AbilitySlots` component inserted on
them. Remove the default `AbilitySlots` insertion on Players.

This must enable updating the ron asset file and having players without their own `AbilitySlots` component have their abilities updated without server
or client reloading.

---

## Summary

`AbilitySlots` is currently a `Component` hardcoded into the server's player spawn call with three specific ability IDs. There is no `AbilitySlots`
`.ron` asset or asset type. The codebase has exactly one established pattern for hot-reloadable RON assets — the `AbilityDefs` system — which uses a
separate asset type, a handle-holding `Resource` newtype, and `AssetEvent`-based reload detection. The `ability_activation` system is the only
consumer of `AbilitySlots`. The client never directly spawns or inserts `AbilitySlots`; it receives it via Lightyear replication from the server.

---

## Detailed Findings

### `AbilitySlots` — Current Definition

**File**: [crates/protocol/src/ability.rs:197-205](crates/protocol/src/ability.rs#L197-L205)

```rust
/// Per-character ability loadout (up to 4 slots).
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AbilitySlots(pub [Option<AbilityId>; 4]);

impl Default for AbilitySlots {
    fn default() -> Self {
        Self([None, None, None, None])
    }
}
```

- Derives `Component`, `Clone`, `Debug`, `PartialEq`, `Serialize`, `Deserialize`.
- Does **not** derive `Reflect` or implement `Asset`.
- Registered as a Lightyear replicated component (no prediction) at [crates/protocol/src/lib.rs:186](crates/protocol/src/lib.rs#L186):
  ```rust
  app.register_component::<AbilitySlots>();
  ```

### `AbilitySlots` — Current Insertion (Server)

**File**: [crates/server/src/gameplay.rs:156-180](crates/server/src/gameplay.rs#L156-L180)

Inserted inline in `handle_connected` observer (fires on `On<Add, Connected>`):

```rust
commands.spawn((
    // ...
    AbilitySlots([
        Some(AbilityId("dive_kick".into())),
        Some(AbilityId("speed_burst".into())),
        Some(AbilityId("shockwave".into())),
        None,
    ]),
    AbilityCooldowns::default(),
));
```

This is the only live-code insertion site. No post-spawn observer or system adds `AbilitySlots` after this point.

### `AbilitySlots` — Consumer

**File**: [crates/protocol/src/ability.rs:439-469](crates/protocol/src/ability.rs#L439-L469)

Only one system reads `AbilitySlots`:

```rust
pub fn ability_activation(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut query: Query<(
        Entity,
        &ActionState<PlayerActions>,
        &AbilitySlots,
        &mut AbilityCooldowns,
        &PlayerId,
    )>,
    server_query: Query<&ControlledBy>,
) { ... }
```

`slots.0[slot_idx]` is read at line 459 to resolve which `AbilityId` corresponds to a pressed input action.

---

### Hot-Reload Pattern — `AbilityDefs` (Established Codebase Pattern)

The only hot-reloadable RON asset in the project is `AbilityDefsAsset`. Its implementation is the template for any new hot-reloadable asset.

#### Asset Type

**File**: [crates/protocol/src/ability.rs:178-189](crates/protocol/src/ability.rs#L178-L189)

```rust
#[derive(Clone, Debug, Serialize, Deserialize, Asset, TypePath)]
#[type_path = "protocol::ability"]
pub struct AbilityDefsAsset {
    pub abilities: HashMap<String, AbilityDef>,
}

#[derive(Resource, Clone, Debug)]
pub struct AbilityDefs {
    pub abilities: HashMap<AbilityId, AbilityDef>,
}
```

Two types: the raw deserialized asset (`AbilityDefsAsset` implements `Asset`) and the usable game resource (`AbilityDefs`).

#### Handle Storage

**File**: [crates/protocol/src/ability.rs:359-360](crates/protocol/src/ability.rs#L359-L360)

```rust
#[derive(Resource)]
struct AbilityDefsHandle(Handle<AbilityDefsAsset>);
```

A newtype `Resource` wrapping `Handle<T>`.

#### Plugin Registration

**File**: [crates/protocol/src/ability.rs:364-370](crates/protocol/src/ability.rs#L364-L370)

```rust
impl Plugin for AbilityPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RonAssetPlugin::<AbilityDefsAsset>::new(&["abilities.ron"]));
        app.add_systems(Startup, load_ability_defs);
        app.add_systems(Update, (insert_ability_defs, reload_ability_defs));
    }
}
```

`RonAssetPlugin` is from `bevy_common_assets`. The extension filter `"abilities.ron"` is the full filename (not just the extension suffix).

#### Load System

**File**: [crates/protocol/src/ability.rs:372-380](crates/protocol/src/ability.rs#L372-L380)

```rust
fn load_ability_defs(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<crate::app_state::TrackedAssets>,
) {
    let handle = asset_server.load::<AbilityDefsAsset>("abilities.ron");
    tracked.add(handle.clone());
    commands.insert_resource(AbilityDefsHandle(handle));
}
```

The handle is cloned into both `TrackedAssets` (for load-gating) and `AbilityDefsHandle` (for event comparison).

#### First-Time Insert System

**File**: [crates/protocol/src/ability.rs:382-402](crates/protocol/src/ability.rs#L382-L402)

```rust
fn insert_ability_defs(
    mut commands: Commands,
    handle: Option<Res<AbilityDefsHandle>>,
    assets: Res<Assets<AbilityDefsAsset>>,
    existing: Option<Res<AbilityDefs>>,
) {
    if existing.is_some() { return; }
    let Some(handle) = handle else { return };
    let Some(asset) = assets.get(&handle.0) else { return };
    // ...build HashMap and insert_resource(AbilityDefs {...})
}
```

Short-circuits if `AbilityDefs` already exists.

#### Hot-Reload System — `AssetEvent`

**File**: [crates/protocol/src/ability.rs:404-426](crates/protocol/src/ability.rs#L404-L426)

```rust
fn reload_ability_defs(
    mut commands: Commands,
    handle: Option<Res<AbilityDefsHandle>>,
    assets: Res<Assets<AbilityDefsAsset>>,
    mut events: MessageReader<AssetEvent<AbilityDefsAsset>>,
) {
    let Some(handle) = handle else { return };
    for event in events.read() {
        if event.is_modified(&handle.0) {
            // warn! if asset not available, else insert_resource(AbilityDefs {...})
        }
    }
}
```

Detection: `AssetEvent<T>` via `MessageReader`. Filter: `event.is_modified(&handle.0)`. Apply: `commands.insert_resource(...)` overwrites existing
resource.

#### Load-Gating Infrastructure

**File**: [crates/protocol/src/app_state.rs](crates/protocol/src/app_state.rs)

```rust
#[derive(Resource, Default)]
pub struct TrackedAssets(Vec<UntypedHandle>);
```

`abilities.ron` is the only handle currently tracked. `check_assets_loaded` blocks `AppState::Ready` until all tracked handles are loaded.

---

### Client-Side — No `AbilitySlots` Insertion

**File**: [crates/client/src/gameplay.rs:16-53](crates/client/src/gameplay.rs#L16-L53)

The client's `handle_new_character` system runs on `Added<Replicated>`/`Added<Predicted>`/`Added<Interpolated>` + `With<CharacterMarker>`. It inserts:

- `InputMap` (controlled entity only)
- `CharacterPhysicsBundle` (predicted/interpolated entities)

It does **not** insert `AbilitySlots`. The client receives `AbilitySlots` from the server via Lightyear replication.

---

## Architecture Documentation

### Current `AbilitySlots` Data Flow

```
Server spawn (handle_connected)
  └─ commands.spawn((..., AbilitySlots([...hardcoded...]), ...))
        │
        └─ Lightyear replication → Client receives AbilitySlots as component
                                        │
                                        └─ ability_activation system reads &AbilitySlots
```

### Established Hot-Reload Pattern (from `AbilityDefs`)

```
Startup:
  asset_server.load("abilities.ron") → Handle<AbilityDefsAsset>
  ├─ stored in AbilityDefsHandle (Resource) — for event matching
  └─ added to TrackedAssets — for load-gating

Update (every frame):
  insert_ability_defs: if AbilityDefs missing AND asset loaded → insert_resource(AbilityDefs)
  reload_ability_defs: on AssetEvent::Modified → insert_resource(AbilityDefs) (overwrites)
```

### Key Types Summary

| Role                          | Type                                          | Kind                 |
| ----------------------------- | --------------------------------------------- | -------------------- |
| Per-entity ability loadout    | `AbilitySlots([Option<AbilityId>; 4])`        | `Component`          |
| Raw deserialized ability defs | `AbilityDefsAsset`                            | `Asset` + `TypePath` |
| Usable ability defs resource  | `AbilityDefs`                                 | `Resource`           |
| Handle for event comparison   | `AbilityDefsHandle(Handle<AbilityDefsAsset>)` | `Resource` (newtype) |
| Load-gate tracker             | `TrackedAssets(Vec<UntypedHandle>)`           | `Resource`           |

---

## Key Change Points for Implementation

### What needs to change in `crates/protocol/src/ability.rs`

1. **Add `Asset, TypePath` to `AbilitySlots`** — no wrapper type needed; `AbilitySlots` holds the final form directly.
2. **New handle resource** `DefaultAbilitySlotsHandle(Handle<AbilitySlots>)` analogous to `AbilityDefsHandle`.
3. **New resource** `DefaultAbilitySlots(AbilitySlots)` holding the loaded default, analogous to `AbilityDefs`.
4. **New load system** — `asset_server.load("ability_slots.ron")`, adds to `TrackedAssets`, stores in handle resource.
5. **New insert system** — first-time insert of `DefaultAbilitySlots` resource once asset is loaded.
6. **New hot-reload system** on `AssetEvent<AbilitySlots>` — overwrites `DefaultAbilitySlots` resource on `Modified`.
7. **New RON file** `assets/ability_slots.ron` for the default slots.
8. **Register** `RonAssetPlugin::<AbilitySlots>::new(&["ability_slots.ron"])` in `AbilityPlugin`.

### What needs to change in `crates/server/src/gameplay.rs`

- Remove the hardcoded `AbilitySlots([...])` from the `commands.spawn(...)` call at lines 173–178.
- No `AbilitySlots` insertion at all — the entity simply won't have one.

### What needs to change in `ability_activation`

- Query signature must change: `&AbilitySlots` becomes optional or the system must handle entities without it.
- For entities lacking `AbilitySlots`, it falls back to the default slots loaded from the asset resource.

---

## Related Research

- [2026-02-20-ability-effect-primitives-implementation-analysis.md](2026-02-20-ability-effect-primitives-implementation-analysis.md)
- [2026-02-22-remaining-ability-effect-primitives.md](2026-02-22-remaining-ability-effect-primitives.md)

## Resolved Questions

**Q: Single global default or per-class?** Single global default. All players share one `AbilitySlots` loaded from the asset.

**Q: Separate wrapper type or add `Asset + TypePath` directly to `AbilitySlots`?** Add directly to `AbilitySlots`. The
`AbilityDefsAsset`/`AbilityDefs` split exists because the raw asset uses `HashMap<String, AbilityDef>` and the usable resource uses
`HashMap<AbilityId, AbilityDef>` — a post-processing conversion is required. `AbilitySlots` holds `[Option<AbilityId>; 4]` which is already the final
form. No transformation is needed. Bevy allows a type to implement both `Component` and `Asset`. Adding `Asset, TypePath` derives to `AbilitySlots`
directly is sufficient.

**Q: Must the fallback resource also exist on the client?** Yes. Since players without their own `AbilitySlots` component won't have one replicated
from the server either, the client must also load the asset and maintain the same fallback `Resource` so `ability_activation` can function if it runs
client-side.
