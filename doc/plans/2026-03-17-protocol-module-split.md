# Protocol Module Split

Split four flat `.rs` files in `crates/protocol/src/` into submodule directories following the
existing `world_object/` pattern. The goal is maintainability — each submodule file has a single
area of concern.

## What We're NOT Doing

- Not changing any public API. `lib.rs` re-exports everything; callers outside `protocol` are
  unaffected.
- Not splitting `app_state.rs` (48 lines) or `physics.rs` (23 lines).
- Not moving `PlayerActions`, `PROTOCOL_ID`, `PRIVATE_KEY`, or `FIXED_TIMESTEP_HZ` — they stay
  in `lib.rs`.
- Not renaming any types, functions, or modules.
- Not changing system scheduling or plugin structure.
- Not adding or removing any behavior.

## Reference: `world_object/` Pattern

`world_object/mod.rs` declares most subfiles as `mod` (private), then selectively re-exports what
the rest of the crate needs. Public submodules like `loading` and `plugin` use `pub mod` to allow
direct downstream access. Internal files import each other with `super::` or `crate::`. This is
the exact pattern each new submodule follows.

---

## Phase 1 — Split `hit_detection.rs` into `hit_detection/`

This is the simplest split and a good warm-up. `hit_detection.rs` has 361 lines with clear
boundaries:

- Lines 14–73: constants, `GameLayer`, and collision-layer fns
- Lines 75–220: hitbox and projectile systems
- Lines 221–361: private effect-application helpers

### Files to Create

```
crates/protocol/src/hit_detection/
  mod.rs
  layers.rs
  systems.rs
  effects.rs
```

**`layers.rs`** — everything from `hit_detection.rs` lines 14–73:
- `GameLayer` enum
- `character_collision_layers()`, `terrain_collision_layers()`, `projectile_collision_layers()`,
  `hitbox_collision_layers()`, `damageable_collision_layers()`

Imports needed:
```rust
use avian3d::prelude::*;
```

**`systems.rs`** — lines 75–220:
- `update_hitbox_positions`
- `process_hitbox_hits`
- `cleanup_hitbox_entities`
- `process_projectile_hits`

Imports needed:
```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::reflect::TypeRegistryArc;
use lightyear::prelude::{ControlledBy, LocalTimeline, Tick};
use crate::ability::{
    facing_direction, spawn_sub_ability, AbilityAsset, AbilityBulletOf, AbilityDefs,
    AbilityPhase, ActiveAbility, ActiveBuffs, ActiveShield, HitTargets, HitboxOf,
    MeleeHitbox, OnHitEffects,
};
use crate::{Health, Invulnerable, PlayerId};
use super::effects::apply_on_hit_effects;
use super::layers::{MELEE_HITBOX_OFFSET, MELEE_HITBOX_HALF_EXTENTS};
```

**`effects.rs`** — lines 221–361:
- `resolve_on_hit_target` (private)
- `apply_damage_buffs` (private)
- `resolve_force_frame` (private)
- `apply_on_hit_effects` — promote to `pub(crate)` (called by `systems.rs` and also directly by
  `ability/effects.rs` if needed in future)

Imports needed:
```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::reflect::TypeRegistryArc;
use lightyear::prelude::{ControlledBy, Tick};
use crate::ability::{
    spawn_sub_ability, AbilityAsset, AbilityDefs, AbilityEffect, ActiveBuffs, ActiveShield,
    EffectTarget, ForceFrame, OnHitEffects,
};
use crate::{Health, Invulnerable, PlayerId};
```

**`mod.rs`**: keep the two constants directly in `layers.rs` with their original names and
re-export through `mod.rs`:

```rust
// layers.rs
pub const MELEE_HITBOX_OFFSET: f32 = 3.0;
pub const MELEE_HITBOX_HALF_EXTENTS: Vec3 = Vec3::new(1.5, 2.0, 1.0);
```

```rust
// mod.rs
pub use layers::{
    character_collision_layers, damageable_collision_layers, hitbox_collision_layers,
    projectile_collision_layers, terrain_collision_layers, GameLayer,
    MELEE_HITBOX_OFFSET, MELEE_HITBOX_HALF_EXTENTS,
};
```

### Changes to Other Files

- Delete `crates/protocol/src/hit_detection.rs`.
- `lib.rs` already declares `pub mod hit_detection;` — no change needed.
- `ability.rs` already imports `crate::hit_detection::{hitbox_collision_layers,
  MELEE_HITBOX_HALF_EXTENTS, MELEE_HITBOX_OFFSET}` — no change needed (re-exported from mod.rs).
- `map.rs` already calls `crate::hit_detection::terrain_collision_layers()` — no change needed.

### Success Criteria (Phase 1)

```
cargo check-all
```
Must pass with zero errors.

---

## Phase 2 — Split `map.rs` into `map/`

`map.rs` has 219 lines. Five concern areas with no internal cross-dependencies (except
`attach_chunk_colliders` which uses types from `types.rs`).

### Files to Create

```
crates/protocol/src/map/
  mod.rs
  types.rs
  voxel.rs
  transition.rs
  chunk.rs
  persistence.rs
  colliders.rs
```

**`types.rs`** — `MapInstanceId`, `MapRegistry`, `MapSwitchTarget`, and the `#[cfg(test)]` block
(lines 12–65 of `map.rs`):
```rust
use std::collections::HashMap;
use avian3d::prelude::ActiveCollisionHooks;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
```
Keep the `#[cfg(test)]` module in this file.

**`voxel.rs`** — `VoxelChannel` + all voxel edit messages (lines 8–9, 67–101):
- `VoxelChannel`
- `VoxelEditRequest`, `VoxelEditBroadcast`, `VoxelEditAck`, `VoxelEditReject`, `SectionBlocksUpdate`

```rust
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
pub use voxel_map_engine::prelude::{PalettedChunk, VoxelChunk, VoxelType};
```

**`transition.rs`** — `MapChannel` + all transition messages/components (lines 103–143):
- `MapChannel`
- `PlayerMapSwitchRequest`, `PendingTransition`, `MapTransitionStart`, `MapTransitionReady`,
  `MapTransitionEnd`, `TransitionReadySent`

```rust
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use super::types::{MapInstanceId, MapSwitchTarget};
```

**`chunk.rs`** — `ChunkChannel` + chunk streaming messages (lines 145–165):
- `ChunkChannel`
- `ChunkDataSync`, `ChunkRequest`, `ChunkUnload`

```rust
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use voxel_map_engine::prelude::PalettedChunk;
```

**`persistence.rs`** — save/load types (lines 167–182):
- `MapSaveTarget`, `SavedEntityKind`, `SavedEntity`

```rust
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
```

**`colliders.rs`** — `attach_chunk_colliders` system (lines 184–219):
```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use voxel_map_engine::prelude::VoxelChunk;
use crate::hit_detection::terrain_collision_layers;
use super::types::MapInstanceId;
```

**`mod.rs`**:
```rust
mod chunk;
mod colliders;
mod persistence;
mod transition;
mod types;
mod voxel;

pub use voxel_map_engine::prelude::{VoxelChunk, VoxelType};

pub use chunk::{ChunkChannel, ChunkDataSync, ChunkRequest, ChunkUnload};
pub use colliders::attach_chunk_colliders;
pub use persistence::{MapSaveTarget, SavedEntity, SavedEntityKind};
pub use transition::{
    MapChannel, MapTransitionEnd, MapTransitionReady, MapTransitionStart, PendingTransition,
    PlayerMapSwitchRequest, TransitionReadySent,
};
pub use types::{MapInstanceId, MapRegistry, MapSwitchTarget};
pub use voxel::{
    SectionBlocksUpdate, VoxelChannel, VoxelEditAck, VoxelEditBroadcast, VoxelEditReject,
    VoxelEditRequest,
};
```

Note: `VoxelChunk` and `VoxelType` are currently re-exported from `map.rs` via
`pub use voxel_map_engine::prelude::{PalettedChunk, VoxelChunk, VoxelType}`. Keep those re-exports
in `mod.rs`. `PalettedChunk` is used in `ChunkDataSync` but is not listed in `lib.rs`'s re-exports,
so it only needs to be accessible within the module.

### Changes to Other Files

- Delete `crates/protocol/src/map.rs`.
- `lib.rs` `pub mod map;` declaration — no change.
- `lib.rs` `pub use map::{...}` — verify the list still matches what `mod.rs` exports. The current
  re-export list in `lib.rs` (lines 30–36) is:
  ```rust
  pub use map::{
      attach_chunk_colliders, ChunkChannel, ChunkDataSync, ChunkRequest, ChunkUnload, MapChannel,
      MapInstanceId, MapRegistry, MapSaveTarget, MapSwitchTarget, MapTransitionEnd,
      MapTransitionReady, MapTransitionStart, PendingTransition, PlayerMapSwitchRequest, SavedEntity,
      SavedEntityKind, SectionBlocksUpdate, TransitionReadySent, VoxelChannel, VoxelChunk,
      VoxelEditAck, VoxelEditBroadcast, VoxelEditReject, VoxelEditRequest, VoxelType,
  };
  ```
  All of these must be re-exported from `map/mod.rs`. Cross-check against the list above.

### Success Criteria (Phase 2)

```
cargo check-all
```
Must pass with zero errors.

---

## Phase 3 — Split `ability.rs` into `ability/`

`ability.rs` is 1739 lines and is the most complex split. There are six concern areas plus a plugin
wiring file.

### Files to Create

```
crates/protocol/src/ability/
  mod.rs
  types.rs
  loader.rs
  loading.rs
  activation.rs
  effects.rs
  spawn.rs
  lifecycle.rs
  plugin.rs
```

### File Contents

**`types.rs`** — all type/struct/enum definitions plus module-level constants. Lines 30–577 of
`ability.rs` (excluding `AbilityPlugin` at line 579 which belongs in `plugin.rs`):
- `PROJECTILE_SPAWN_OFFSET`, `BULLET_COLLIDER_RADIUS`, `ABILITY_ACTIONS` (lines 30–38)
- `facing_direction` (line 40–42) — geometry helper; a section header in the file clarifies it is
  not a type definition
- `AbilityId`, `EffectTarget`, `ForceFrame`, `AbilityEffect`, `EffectTrigger`
- `AbilityDef` + impl
- `AbilityPhases` + impl
- `AbilityManifest`, `AbilityDefs` + impl
- `AbilitySlots` + Default impl
- `AbilityPhase`
- `ActiveAbility` + `MapEntities` impl
- `AbilityCooldowns`
- `ProjectileSpawnEffect`
- `HitboxOf`, `ActiveAbilityHitboxes`
- `MeleeHitbox`, `AoEHitbox`
- `HitTargets`
- `OnHitEffects`, `TickEffect`, `OnTickEffects`, `WhileActiveEffects`, `OnEndEffects`
- `InputEffect`, `OnInputEffects`, `OnHitEffectDefs`
- `AbilityAsset`
- `ActiveShield`
- `ActiveBuff`, `ActiveBuffs`
- `AbilityProjectileSpawn`
- `AbilityBulletOf`, `AbilityBullets`

Imports needed:
```rust
use avian3d::prelude::Rotation;
use bevy::ecs::entity::{EntityMapper, MapEntities};
use bevy::prelude::*;
use bevy::reflect::PartialReflect;
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::Tick;
use lightyear::utils::collections::EntityHashSet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use crate::PlayerActions;
```

**`loader.rs`** — `AbilityAssetLoader`, `extract_phases`, `apply_ability_archetype`
(find by searching `impl AssetLoader` and surrounding helpers):
```rust
use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, AssetPath, LoadContext};
use bevy::prelude::*;
use bevy::reflect::{PartialReflect, TypeRegistryArc};
use super::types::{AbilityAsset, AbilityPhases};
use crate::reflect_loader;
```

Note: `apply_ability_archetype` is called by `loading.rs` and `activation.rs` — promote to
`pub(crate)` in `loader.rs` and re-export from `mod.rs`.

**`loading.rs`** — all asset-loading systems plus handle resources:
- `AbilityFolderHandle` (cfg native), `AbilityManifestHandle` / `PendingAbilityHandles` (cfg wasm)
- `DefaultAbilitySlotsHandle`, `DefaultAbilitySlots`
- `load_ability_defs`, `trigger_individual_ability_loads`, `insert_ability_defs`,
  `reload_ability_defs`
- `ability_id_from_path`, `collect_ability_handles_from_folder`
- `load_default_ability_slots`, `sync_default_ability_slots`

```rust
use bevy::prelude::*;
use std::collections::HashMap;
use crate::app_state::TrackedAssets;
use super::types::{AbilityAsset, AbilityDefs, AbilityId, AbilityManifest, AbilitySlots};
```

Note: `DefaultAbilitySlots` is re-exported publicly from `mod.rs` (it appears in `lib.rs`'s
re-export list). `DefaultAbilitySlotsHandle` is internal — stays `pub(crate)` or private.

**`activation.rs`** — ability activation and phase-advance systems:
- `ability_action_to_slot` (pub — in `lib.rs` re-exports)
- `ability_activation`
- `advance_ability_phase`
- `update_active_abilities`

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::{
    ControlledBy, DisableRollback, NetworkTarget, PreSpawned, PredictionDespawnCommandsExt,
    PredictionTarget, Replicate, Tick,
};
use crate::{PlayerActions, PlayerId};
use super::loader::apply_ability_archetype;
use super::types::{
    AbilityDefs, AbilityPhase, AbilityPhases, AbilitySlots, ActiveAbility, AbilityCooldowns,
    OnHitEffects, OnHitEffectDefs,
};
```

**`effects.rs`** — effect dispatch systems:
- `apply_on_tick_effects`
- `apply_while_active_effects`
- `apply_on_end_effects`
- `apply_on_input_effects`
- `resolve_caster_target`
- `compute_sub_ability_salt`
- `apply_teleport`
- `apply_buff`

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::{ControlledBy, LocalTimeline, Tick};
use crate::{PlayerActions, PlayerId};
use super::spawn::spawn_sub_ability;
use super::types::{
    AbilityDefs, AbilityAsset, AbilityEffect, ActiveAbility, ActiveBuff, ActiveBuffs,
    EffectTarget, InputEffect, OnEndEffects, OnInputEffects, OnTickEffects, WhileActiveEffects,
};
```

**`spawn.rs`** — spawn helper functions and projectile systems:
- `spawn_sub_ability` — `pub(crate)`
- `spawn_melee_hitbox`, `spawn_aoe_hitbox` — `pub(crate)`
- `ability_projectile_spawn` (FixedUpdate system)
- `handle_ability_projectile_spawn` (PreUpdate system)
- `despawn_ability_projectile_spawn` (observer)

```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::reflect::TypeRegistryArc;
use lightyear::prelude::{
    ControlledBy, DisableRollback, NetworkTarget, PreSpawned, PredictionTarget, Replicate,
    PredictionDespawnCommandsExt, Tick,
};
use crate::{PlayerId};
use crate::hit_detection::{hitbox_collision_layers, MELEE_HITBOX_HALF_EXTENTS, MELEE_HITBOX_OFFSET};
use super::loader::apply_ability_archetype;
use super::types::{
    AbilityAsset, AbilityBulletOf, AbilityBullets, AbilityDefs, AbilityId, AbilityProjectileSpawn,
    ActiveAbility, AoEHitbox, HitboxOf, ActiveAbilityHitboxes, MeleeHitbox, OnHitEffects,
    OnHitEffectDefs, ProjectileSpawnEffect,
};
```

**`lifecycle.rs`** — cleanup and expiry systems:
- `aoe_hitbox_lifetime`
- `ability_bullet_lifetime`
- `expire_buffs`
- `cleanup_effect_markers_on_removal`

```rust
use bevy::prelude::*;
use lightyear::prelude::{LocalTimeline, PredictionDespawnCommandsExt};
use super::types::{
    AbilityBulletOf, AbilityProjectileSpawn, ActiveAbility, ActiveBuffs, AoEHitbox,
    HitboxOf, MeleeHitbox, OnEndEffects, OnHitEffectDefs, OnHitEffects, OnInputEffects,
    OnTickEffects, WhileActiveEffects,
};
```

**`plugin.rs`** — `AbilityPlugin` only:
```rust
use bevy::prelude::*;
use crate::app_state::AppState;
use super::loader::AbilityAssetLoader;
use super::loading::{
    insert_ability_defs, load_ability_defs, load_default_ability_slots,
    reload_ability_defs, sync_default_ability_slots,
};
use super::activation::{ability_activation, update_active_abilities};
use super::effects::{
    apply_on_end_effects, apply_on_input_effects, apply_on_tick_effects,
    apply_while_active_effects,
};
use super::spawn::{
    ability_projectile_spawn, despawn_ability_projectile_spawn, handle_ability_projectile_spawn,
};
use super::lifecycle::{
    ability_bullet_lifetime, aoe_hitbox_lifetime, cleanup_effect_markers_on_removal, expire_buffs,
};
use super::types::{AbilityAsset, AbilityManifest, AbilitySlots};
```

For WASM-gated systems (`trigger_individual_ability_loads`), wrap with `#[cfg(target_arch = "wasm32")]`
exactly as in the original.

**`mod.rs`**:
```rust
mod activation;
mod effects;
mod lifecycle;
mod loader;
mod spawn;
mod types;

pub mod loading;
pub mod plugin;

pub(crate) use loader::apply_ability_archetype;
pub(crate) use spawn::spawn_sub_ability;

pub use activation::ability_action_to_slot;
pub use loading::DefaultAbilitySlots;
pub use plugin::AbilityPlugin;
pub use types::{
    AbilityAsset, AbilityBulletOf, AbilityBullets, AbilityCooldowns, AbilityDef, AbilityDefs,
    AbilityEffect, AbilityId, AbilityManifest, AbilityPhase, AbilityPhases,
    AbilityProjectileSpawn, AbilitySlots, ActiveAbility, ActiveBuff, ActiveBuffs, ActiveShield,
    AoEHitbox, EffectTarget, EffectTrigger, ForceFrame, HitTargets, HitboxOf,
    ActiveAbilityHitboxes, InputEffect, MeleeHitbox, OnEndEffects, OnHitEffectDefs, OnHitEffects,
    OnInputEffects, OnTickEffects, ProjectileSpawnEffect, TickEffect, WhileActiveEffects,
    facing_direction,
};
```

The list above must exactly cover what `lib.rs` currently imports from `crate::ability` (lines
17–24 of `lib.rs`) plus what `hit_detection/` imports.

### Changes to Other Files

- Delete `crates/protocol/src/ability.rs`.
- `lib.rs` `pub mod ability;` — no change.
- `lib.rs` `pub use ability::{...}` — verify list matches `mod.rs` exports.
- `hit_detection/systems.rs` and `hit_detection/effects.rs` import from `crate::ability::*` — these
  paths are all re-exported from `ability/mod.rs`, so no path changes needed.

### Success Criteria (Phase 3)

```
cargo check-all
```
Must pass with zero errors.

---

## Phase 4 — Split `lib.rs` character types into `character/`

`lib.rs` contains character types and movement logic that are unrelated to protocol registration.
These move into a new `character/` submodule.

### Files to Create

```
crates/protocol/src/character/
  mod.rs
  types.rs
  movement.rs
```

**`types.rs`** — character type definitions. Extract from `lib.rs` lines 43–147, but keep
`PlayerActions` (lines 46–65) in `lib.rs` as specified in "What We're NOT Doing":
- `CHARACTER_CAPSULE_RADIUS`, `CHARACTER_CAPSULE_HEIGHT`
- `PlayerId`, `CharacterMarker`, `DummyTarget`, `CharacterType`
- `RespawnPoint`
- `Health` + impl
- `Invulnerable`
- `ColorComponent`
- `CharacterPhysicsBundle` + Default impl

Imports needed:
```rust
use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::{PeerId, Tick};
use serde::{Deserialize, Serialize};
use crate::hit_detection::character_collision_layers;
use crate::map::MapSaveTarget;
```

`CharacterPhysicsBundle::default()` references `character_collision_layers()` — import from
`crate::hit_detection`.

**`movement.rs`** — movement functions (lib.rs lines 317–390):
- `apply_movement`
- `update_facing`

Imports needed:
```rust
use avian3d::prelude::{forces::ForcesItem, *};
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use crate::map::MapInstanceId;
use crate::PlayerActions;
use super::types::CharacterMarker;
```

**`mod.rs`**:
```rust
pub mod movement;
pub mod types;

pub use movement::{apply_movement, update_facing};
pub use types::{
    CharacterMarker, CharacterPhysicsBundle, CharacterType, ColorComponent, DummyTarget,
    Health, Invulnerable, PlayerId, RespawnPoint,
    CHARACTER_CAPSULE_HEIGHT, CHARACTER_CAPSULE_RADIUS,
};
```

### Changes to `lib.rs`

1. Add at the top of `lib.rs` (alongside the existing `pub mod` declarations):
   ```rust
   pub mod character;
   ```

2. Replace the character type definitions in `lib.rs` (lines 43–147) with re-exports:
   ```rust
   pub use character::{
       CharacterMarker, CharacterPhysicsBundle, CharacterType, ColorComponent, DummyTarget,
       Health, Invulnerable, PlayerId, RespawnPoint,
       CHARACTER_CAPSULE_HEIGHT, CHARACTER_CAPSULE_RADIUS,
   };
   ```

3. Remove `apply_movement` and `update_facing` definitions from `lib.rs` (lines 317–390). Add
   re-export:
   ```rust
   pub use character::{apply_movement, update_facing};
   ```

4. The `pub use character::{apply_movement, update_facing}` re-export added in step 3 keeps
   `update_facing` in scope unqualified within `lib.rs`. No change to `SharedGameplayPlugin::build`
   is needed.

5. `ProtocolPlugin` references `PlayerId`, `ColorComponent`, `CharacterMarker`, etc. — these are
   all still in scope via the `pub use character::{...}` re-exports in `lib.rs`, so no changes
   needed inside `ProtocolPlugin`.

6. `CharacterPhysicsBundle::default()` calls `hit_detection::character_collision_layers()`. In
   `character/types.rs` this becomes `crate::hit_detection::character_collision_layers()`.

### Changes to Other Crates

No other crate imports `CHARACTER_CAPSULE_RADIUS` or `CHARACTER_CAPSULE_HEIGHT` directly from
`protocol` at the module level — they go through `pub use` in `lib.rs`. Verify with:
```
grep -r "CHARACTER_CAPSULE" --include="*.rs" .
```
If any crate imports `protocol::CHARACTER_CAPSULE_*` directly, the re-export in `lib.rs` covers it.

### Success Criteria (Phase 4)

```
cargo check-all
```
Must pass with zero errors.

---

## Final Verification

After all four phases complete:

1. **Automated:**
   ```
   cargo check-all
   ```

2. **Runtime:**
   ```
   cargo server
   ```
   Server must start without panics or asset-loading errors. Test that the basic gameplay loop
   works (character movement, abilities, voxel map loading).

3. **Structure check:** confirm the following files exist and the originals are gone:
   ```
   crates/protocol/src/hit_detection/mod.rs
   crates/protocol/src/hit_detection/layers.rs
   crates/protocol/src/hit_detection/systems.rs
   crates/protocol/src/hit_detection/effects.rs
   crates/protocol/src/map/mod.rs
   crates/protocol/src/map/types.rs
   crates/protocol/src/map/voxel.rs
   crates/protocol/src/map/transition.rs
   crates/protocol/src/map/chunk.rs
   crates/protocol/src/map/persistence.rs
   crates/protocol/src/map/colliders.rs
   crates/protocol/src/ability/mod.rs
   crates/protocol/src/ability/types.rs
   crates/protocol/src/ability/loader.rs
   crates/protocol/src/ability/loading.rs
   crates/protocol/src/ability/activation.rs
   crates/protocol/src/ability/effects.rs
   crates/protocol/src/ability/spawn.rs
   crates/protocol/src/ability/lifecycle.rs
   crates/protocol/src/ability/plugin.rs
   crates/protocol/src/character/mod.rs
   crates/protocol/src/character/types.rs
   crates/protocol/src/character/movement.rs
   ```
   And the following flat files are gone:
   ```
   crates/protocol/src/hit_detection.rs  — deleted
   crates/protocol/src/map.rs            — deleted
   crates/protocol/src/ability.rs        — deleted
   ```
   `lib.rs` remains; it no longer contains character type definitions.

---

## Implementation Notes

- Do phases in order: Phase 1 → 2 → 3 → 4. Run `cargo check-all` after each before proceeding.
- The `ability/` split (Phase 3) is the largest. Work file by file: start with `types.rs` (no
  intra-ability imports), then `loader.rs`, `loading.rs`, `lifecycle.rs`, `spawn.rs`, `effects.rs`,
  `activation.rs`, `plugin.rs`, then wire `mod.rs` last.
- `spawn_sub_ability` must be `pub(crate)` in `spawn.rs` and re-exported as `pub(crate)` from
  `ability/mod.rs` so `hit_detection/effects.rs` can call `crate::ability::spawn_sub_ability`.
- `facing_direction` must be re-exported from `ability/mod.rs` as `pub` so
  `hit_detection/systems.rs` can call `crate::ability::facing_direction`.
- `apply_ability_archetype` is called in both `activation.rs` and `spawn.rs` — make it `pub(crate)`
  in `loader.rs`, re-export as `pub(crate)` from `mod.rs`.
- All `#[cfg(not(target_arch = "wasm32"))]` / `#[cfg(target_arch = "wasm32")]` gates in
  `ability.rs` must be preserved exactly in the split files.

## TypePath Preservation (CRITICAL)

Moving a type into a submodule changes its `TypePath`. Bevy derives `TypePath` from the definition
site, not re-export paths. RON asset files embed type paths as strings — if the path changes, asset
loading silently fails with "no registration found for `protocol::ability::AbilityPhases`".

**Every `#[derive(Reflect)]` type that moves to a submodule must carry a `#[type_path]` attribute
restoring its original stable path.** Place it on the line immediately after `#[derive(...)]` and
before any `#[reflect(...)]`.

```rust
#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[type_path = "protocol::ability"]   // ← restores pre-split path
#[reflect(Component, Serialize, Deserialize)]
pub struct AbilityPhases { ... }
```

### Required attributes by file

**`ability/types.rs`** — `#[type_path = "protocol::ability"]` on:
`AbilityId`, `EffectTarget`, `ForceFrame`, `AbilityEffect`, `EffectTrigger`, `AbilityDef`,
`AbilityPhases`, `AbilityPhase`, `TickEffect`, `OnTickEffects`, `WhileActiveEffects`,
`OnEndEffects`, `InputEffect`, `OnInputEffects`, `OnHitEffectDefs`, `AbilityProjectileSpawn`

(`AbilitySlots` already has it.)

**`character/types.rs`** — `#[type_path = "protocol"]` on:
`PlayerId`, `CharacterType`, `Health`

(Types were in `lib.rs`; their original paths were `protocol::TypeName`.)

**`map/types.rs`** — `#[type_path = "protocol::map"]` on:
`MapInstanceId`, `MapSwitchTarget`

**`map/transition.rs`** — `#[type_path = "protocol::map"]` on:
`PlayerMapSwitchRequest`, `MapTransitionStart`, `MapTransitionReady`, `MapTransitionEnd`

**`map/voxel.rs`** — `#[type_path = "protocol::map"]` on:
`VoxelEditRequest`, `VoxelEditBroadcast`, `VoxelEditAck`, `VoxelEditReject`, `SectionBlocksUpdate`
