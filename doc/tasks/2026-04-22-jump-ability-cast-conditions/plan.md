# Implementation Plan

## Overview

Replace inline jump in `apply_movement` with a data-driven `jump.ability.ron` that activates through the existing ability pipeline gated by a new `ConditionalEffects` component evaluated against an `IsGrounded` sparse-set marker, while migrating `AbilityEffect::ApplyForce` onto avian's mass-scaled `forces.apply_linear_impulse`.

---

## Phase 1: Migrate `AbilityEffect::ApplyForce` to `apply_linear_impulse`

### Changes

#### 1. `apply_on_hit_effects` `ApplyForce` arm

**File**: `crates/protocol/src/hit_detection/effects.rs`
**Action**: modify (signature + arm body)

Replace the `Option<&mut LinearVelocity>` slot in `target_query` with avian's `Forces` `QueryData`. Add a separate `Position` lookup for the resolve_force_frame source position (since `Forces` does not carry position).

```rust
// crates/protocol/src/hit_detection/effects.rs
target_query: &mut Query<(
    &Position,
    Forces,
    &mut Health,
    Option<&Invulnerable>,
)>,
```

Rewrite the `ApplyForce` arm:

```rust
AbilityEffect::ApplyForce { force, frame, target } => {
    let entity = resolve_on_hit_target(target, victim, on_hit);
    if let Ok((target_pos, mut forces, _, _)) = target_query.get_mut(entity) {
        let world_force = resolve_force_frame(
            *force, frame, source_pos, target_pos.0,
            on_hit.caster, entity, rotation_query,
        );
        forces.apply_linear_impulse(world_force);
    } else {
        warn!("ApplyForce target {:?} not a rigid body", entity);
    }
}
```

The `Damage` arm above also calls `target_query.get_mut(entity)` and binds the second tuple slot (was `_` for velocity). Update its destructure to match the new tuple shape (`(_, _, mut health, invulnerable)` already discards it — check the arm at `effects.rs:100` after the type change and adjust if needed).

#### 2. Update all caller sites of `target_query`

**File**: `crates/protocol/src/hit_detection/systems.rs`
**Action**: modify (callers that build the query)

Search for any `Query<(&Position, Option<&mut LinearVelocity>, &mut Health, Option<&Invulnerable>)>` and replace with the new `Forces`-based shape. Confirm with `grep -rn "Option<&mut LinearVelocity>" crates/protocol/src/hit_detection/`.

#### 3. Rescale ApplyForce values in all ability RONs

**Files**:
- `assets/abilities/punch.ability.ron`
- `assets/abilities/punch2.ability.ron`
- `assets/abilities/punch3.ability.ron`
- `assets/abilities/shield_bash.ability.ron`
- `assets/abilities/ground_pound.ability.ron`
- `assets/abilities/shockwave.ability.ron`
- `assets/abilities/teleport_burst.ability.ron`
- `assets/abilities/dive_kick.ability.ron`
- `assets/abilities/fireball.ability.ron`
- `assets/abilities/blink_strike.ability.ron`
- `assets/abilities/uppercut.ability.ron`

**Action**: modify (rescale `force` vector by victim mass)

Character `ComputedMass` is derived from `Collider::capsule(radius=2.0, height=2.0)` × default `ColliderDensity(1.0)`. Capsule volume = π·r²·h + (4/3)·π·r³ = 8π + 32π/3 = 56π/3 ≈ **58.64**. Mass ≈ 58.64.

```ron
ApplyForce(force: (0.0, 52.78, 29.32), frame: RelativePosition, target: Victim),
```
(was `(0.0, 0.9, 0.5)` × 58.64.)

Apply the same scalar to every file in the list. After rescaling, remove the temporary `trace!`.

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test-native` passes

#### Manual
- [ ] User runs `cargo server` then `cargo client`. Activate punch (`Digit1`) on a target. Knockback distance/feel matches a pre-Phase-1 build. If off, ask user to record actual mass and rescale.
- [ ] Jump (Space) still works via the inline path — unchanged in this phase.

---

## Phase 2: `IsGrounded` sparse-set marker + `detect_grounded` system

### Changes

#### 1. New `IsGrounded` component

**File**: `crates/protocol/src/character/types.rs`
**Action**: modify (append at bottom of file)

```rust
/// Local-only marker present when the character's ground ray cast hits.
/// Toggled each FixedUpdate tick by `detect_grounded`. SparseSet storage avoids
/// archetype churn during jumps. Not registered for replication or prediction —
/// derived deterministically from already-replicated `Position` + colliders.
#[derive(Component, Debug)]
#[component(storage = "SparseSet")]
pub struct IsGrounded;
```

Add `IsGrounded` to the `pub use types::{...}` list in `crates/protocol/src/character/mod.rs`.

#### 2. New `detect_grounded` system

**File**: `crates/protocol/src/character/movement.rs`
**Action**: modify (append new fn)

```rust
/// Maintains the `IsGrounded` marker on character entities by ray casting
/// downward from the capsule center each tick. Must run before
/// `handle_character_movement` and `ability_activation` so consumers see a
/// fresh marker.
pub fn detect_grounded(
    mut commands: Commands,
    spatial_query: SpatialQuery,
    map_ids: Query<&MapInstanceId>,
    characters: Query<
        (Entity, &Position, Option<&MapInstanceId>, Has<IsGrounded>),
        With<CharacterMarker>,
    >,
) {
    for (entity, position, player_map_id, has_grounded) in &characters {
        let filter = SpatialQueryFilter::from_excluded_entities([entity]);
        let hit = spatial_query
            .cast_ray_predicate(
                position.0,
                Dir3::NEG_Y,
                4.0,
                false,
                &filter,
                &|hit_entity| match (player_map_id, map_ids.get(hit_entity).ok()) {
                    (Some(a), Some(b)) => a == b,
                    _ => true,
                },
            )
            .is_some();
        match (hit, has_grounded) {
            (true, false) => {
                commands.entity(entity).insert(IsGrounded);
            }
            (false, true) => {
                commands.entity(entity).remove::<IsGrounded>();
            }
            _ => {}
        }
    }
}
```

Add `use super::types::{CharacterMarker, IsGrounded};` to the imports.

Re-export `detect_grounded` from `crates/protocol/src/character/mod.rs`:
```rust
pub use movement::{apply_movement, detect_grounded, update_facing};
```

#### 3. Schedule `detect_grounded` in client and server

**File**: `crates/client/src/gameplay.rs`
**Action**: modify

Replace `app.add_systems(FixedUpdate, handle_character_movement);` with:
```rust
// detect_grounded must run before handle_character_movement and
// ability_activation so the IsGrounded gate sees fresh state.
app.add_systems(
    FixedUpdate,
    (
        protocol::detect_grounded,
        handle_character_movement,
    )
        .chain()
        .before(protocol::ability::ability_activation),
);
```

**File**: `crates/server/src/gameplay.rs`
**Action**: modify (line 32)

Same pattern as client. Confirm `protocol::detect_grounded` and `protocol::ability::ability_activation` are exported.

If `ability_activation` is not currently `pub`, change it to `pub` in `crates/protocol/src/ability/activation.rs` (it already is — `pub fn ability_activation`) and re-export at `crates/protocol/src/ability/mod.rs` if not already.

### Verification

#### Automated
- [x] `cargo check-all` passes

#### Manual
- [ ] Add a temporary `trace!("grounded={}", hit);` line at the start of each branch in `detect_grounded`. Run `cargo server` + `cargo client`. Confirm the marker toggles to `false` mid-jump and back to `true` on landing. Remove the `trace!` before committing.

---

## Phase 3: Expand `AbilitySlots` to 5 entries + bind `Jump` to slot 4

### Changes

#### 1. Widen `AbilitySlots` and `AbilityCooldowns` arrays

**File**: `crates/protocol/src/ability/types.rs`
**Action**: modify

```rust
// types.rs:178-186
pub struct AbilitySlots(pub [Option<AbilityId>; 5]);

impl Default for AbilitySlots {
    fn default() -> Self {
        Self([None, None, None, None, None])
    }
}

// types.rs:219-230
pub struct AbilityCooldowns {
    pub last_used: [Option<Tick>; 5],
}

impl Default for AbilityCooldowns {
    fn default() -> Self {
        Self {
            last_used: [None; 5],
        }
    }
}
```

#### 2. Extend `ABILITY_ACTIONS` with `Jump` at index 4

**File**: `crates/protocol/src/ability/activation.rs`
**Action**: modify (lines 17-22 and doc comments at 24, 29)

```rust
const ABILITY_ACTIONS: [PlayerActions; 5] = [
    PlayerActions::Ability1,
    PlayerActions::Ability2,
    PlayerActions::Ability3,
    PlayerActions::Ability4,
    PlayerActions::Jump,
];

/// Maps a `PlayerActions` ability variant to a slot index (0-4).
pub fn ability_action_to_slot(action: &PlayerActions) -> Option<usize> {
    ABILITY_ACTIONS.iter().position(|a| a == action)
}

/// Maps a slot index (0-4) to its corresponding `PlayerActions` variant.
pub fn slot_to_ability_action(slot: usize) -> Option<PlayerActions> {
    ABILITY_ACTIONS.get(slot).copied()
}
```

#### 3. Default slots RON file gains a 5th `None` entry

**File**: `assets/default.ability_slots.ron`
**Action**: modify

```ron
#![enable(implicit_some)]
(
    (
        AbilityId("punch"),
        AbilityId("speed_burst"),
        AbilityId("ground_pound"),
        AbilityId("blink_strike"),
        None,
    )
)
```

(`None` in slot 4 — Phase 5 will replace with `AbilityId("jump")`.)

#### 4. Sanity-check Lightyear `Serialize` round-trip

`AbilitySlots` and `AbilityCooldowns` use `Serialize`/`Deserialize` from `serde`. Fixed-size arrays of `Option<T>` round-trip out of the box, so no schema bump is required. `register_component::<AbilitySlots>()` and `register_component::<AbilityCooldowns>()` at `crates/protocol/src/lib.rs:186,190` need no edit.

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test-native` passes

#### Manual
- [ ] Run `cargo server` + `cargo client`. Activate `Digit1`–`Digit4` and confirm each Ability1–4 still fires correctly. Press Space — jump still works via the inline path (slot 4 is `None`).

---

## Phase 4: `ConditionalEffects` component + activation-time evaluation

### Changes

#### 1. New reflected types

**File**: `crates/protocol/src/ability/types.rs`
**Action**: modify (append after `OnHitEffectDefs`)

```rust
/// Caster-state condition evaluated at ability activation.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[type_path = "protocol::ability"]
pub enum Condition {
    Grounded,
    Airborne,
}

/// One conditional branch: if `condition` matches the caster at activation,
/// `effect` is appended to the spawned ability's `OnTickEffects` at tick 0.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[type_path = "protocol::ability"]
pub struct ConditionalEffect {
    pub condition: Condition,
    pub effect: AbilityEffect,
}

/// If present and no entry's condition matches at activation, the ability
/// is refused (no spawn, no cooldown consumption). All matching entries fire.
#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[type_path = "protocol::ability"]
#[reflect(Component, Serialize, Deserialize)]
pub struct ConditionalEffects(pub Vec<ConditionalEffect>);
```

Re-export from `crates/protocol/src/ability/mod.rs` next to existing `OnTickEffects` etc.

#### 2. Typed extractors

**File**: `crates/protocol/src/ability/loader.rs`
**Action**: modify (append after `extract_phases`)

```rust
/// Extract `ConditionalEffects` from an `AbilityAsset`'s reflected components.
pub fn extract_conditional_effects(asset: &AbilityAsset) -> Option<&ConditionalEffects> {
    let target_id = std::any::TypeId::of::<ConditionalEffects>();
    for reflected in &asset.components {
        let info = reflected
            .get_represented_type_info()
            .expect("AbilityAsset should have type info");
        if info.type_id() == target_id {
            return reflected.try_downcast_ref::<ConditionalEffects>();
        }
    }
    None
}

/// Extract `OnTickEffects` from an `AbilityAsset`'s reflected components.
pub fn extract_on_tick_effects(asset: &AbilityAsset) -> Option<&OnTickEffects> {
    let target_id = std::any::TypeId::of::<OnTickEffects>();
    for reflected in &asset.components {
        let info = reflected
            .get_represented_type_info()
            .expect("AbilityAsset should have type info");
        if info.type_id() == target_id {
            return reflected.try_downcast_ref::<OnTickEffects>();
        }
    }
    None
}
```

Imports gain `use super::types::{AbilityAsset, AbilityPhases, ConditionalEffects, OnTickEffects};`.

#### 3. Archetype helper that overrides `OnTickEffects` on insert

**File**: `crates/protocol/src/ability/loader.rs`
**Action**: modify

Add a sibling fn that mirrors `apply_ability_archetype` but substitutes the asset's `OnTickEffects` with a caller-supplied value (cloned + extra entries appended). Keep the original `apply_ability_archetype` for non-conditional callers.

```rust
/// Like `apply_ability_archetype`, but replaces the asset's `OnTickEffects`
/// component with `override_on_tick` during insertion. If the asset had no
/// `OnTickEffects`, `override_on_tick` is inserted as a new component.
pub(crate) fn apply_ability_archetype_with_on_tick_override(
    commands: &mut Commands,
    entity: Entity,
    asset: &AbilityAsset,
    registry: TypeRegistryArc,
    override_on_tick: OnTickEffects,
) {
    let target_id = std::any::TypeId::of::<OnTickEffects>();
    let mut components: Vec<Box<dyn PartialReflect>> = asset
        .components
        .iter()
        .filter(|c| {
            c.get_represented_type_info()
                .map(|i| i.type_id() != target_id)
                .unwrap_or(true)
        })
        .map(|c| {
            c.reflect_clone()
                .expect("ability component must be cloneable")
                .into_partial_reflect()
        })
        .collect();
    components.push(Box::new(override_on_tick) as Box<dyn PartialReflect>);

    commands.queue(move |world: &mut World| {
        let registry = registry.read();
        let mut entity_mut = world.entity_mut(entity);
        for component in &components {
            let type_path = component.reflect_type_path();
            let Some(registration) = registry.get_with_type_path(type_path) else {
                warn!("Ability component type not registered: {type_path}");
                continue;
            };
            let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                warn!("Type missing #[reflect(Component)]: {type_path}");
                continue;
            };
            reflect_component.insert(&mut entity_mut, component.as_ref(), &registry);
        }
    });
}
```

Note: `OnTickEffects` already derives `Reflect` and is `register_type`'d in `ability/plugin.rs:35`. `Box<OnTickEffects>` becomes `Box<dyn PartialReflect>` via the `Reflect: PartialReflect` blanket.

#### 4. Evaluate `ConditionalEffects` in `ability_activation`

**File**: `crates/protocol/src/ability/activation.rs`
**Action**: modify

Add `IsGrounded` access to the system signature and gate the spawn:

```rust
use super::loader::{
    apply_ability_archetype, apply_ability_archetype_with_on_tick_override,
    extract_conditional_effects, extract_on_tick_effects, extract_phases,
};
use super::types::{
    AbilityAsset, AbilityCooldowns, AbilityDefs, AbilityPhase, AbilityPhases, AbilitySlots,
    ActiveAbility, Condition, ConditionalEffects, OnHitEffectDefs, OnHitEffects, OnTickEffects,
    TickEffect,
};
use crate::character::IsGrounded;
```

Then in the per-action loop, after the cooldown check and before the `commands.spawn`:

```rust
// Evaluate ConditionalEffects against the caster's current state. If the
// asset declares conditions but none match, refuse the cast: no spawn, no
// cooldown consumption (cooldowns.last_used was not yet written this tick).
let conditional = extract_conditional_effects(asset);
let matched: Vec<crate::ability::AbilityEffect> = if let Some(ce) = conditional {
    let grounded = grounded_query.contains(entity);
    ce.0.iter()
        .filter(|c| match c.condition {
            Condition::Grounded => grounded,
            Condition::Airborne => !grounded,
        })
        .map(|c| c.effect.clone())
        .collect()
} else {
    Vec::new()
};
if conditional.is_some() && matched.is_empty() {
    trace!(
        "Ability {:?} refused: no ConditionalEffects condition matched (entity {:?})",
        ability_id, entity
    );
    continue;
}

cooldowns.last_used[slot_idx] = Some(tick);
```

(Move the existing `cooldowns.last_used[slot_idx] = Some(tick);` line down to after the gate.)

Replace the `apply_ability_archetype` call with branching:

```rust
if conditional.is_some() {
    let mut on_tick = extract_on_tick_effects(asset).cloned().unwrap_or_default();
    for effect in matched {
        on_tick.0.push(TickEffect { tick: 0, effect });
    }
    apply_ability_archetype_with_on_tick_override(
        &mut commands, entity_id, asset, registry.0.clone(), on_tick,
    );
} else {
    apply_ability_archetype(&mut commands, entity_id, asset, registry.0.clone());
}
```

Add the `IsGrounded` query to the system params:

```rust
pub fn ability_activation(
    // ... existing params ...
    grounded_query: Query<(), With<IsGrounded>>,
)
```

#### 5. Register the new types

**File**: `crates/protocol/src/ability/plugin.rs`
**Action**: modify

Imports gain `Condition, ConditionalEffect, ConditionalEffects`. Append to the `app.register_type::<...>()` chain at lines 34-45:

```rust
.register_type::<Condition>()
.register_type::<ConditionalEffect>()
.register_type::<ConditionalEffects>();
```

#### 6. Unit test

**File**: `crates/protocol/src/ability/loader.rs` (or a new `#[cfg(test)] mod` therein)
**Action**: modify (add tests)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ability::types::{
        AbilityAsset, AbilityEffect, Condition, ConditionalEffect, ConditionalEffects,
        EffectTarget, ForceFrame,
    };
    use bevy::prelude::*;

    fn synth_asset(ce: ConditionalEffects) -> AbilityAsset {
        AbilityAsset {
            components: vec![Box::new(ce) as Box<dyn bevy::reflect::PartialReflect>],
        }
    }

    #[test]
    fn extracts_conditional_effects() {
        let ce = ConditionalEffects(vec![ConditionalEffect {
            condition: Condition::Grounded,
            effect: AbilityEffect::ApplyForce {
                force: Vec3::Y * 2000.0,
                frame: ForceFrame::World,
                target: EffectTarget::Caster,
            },
        }]);
        let asset = synth_asset(ce.clone());
        let extracted = extract_conditional_effects(&asset).unwrap();
        assert_eq!(extracted, &ce);
    }

    #[test]
    fn grounded_filter_selects_matching_entries() {
        let ce = ConditionalEffects(vec![
            ConditionalEffect {
                condition: Condition::Grounded,
                effect: AbilityEffect::ApplyForce {
                    force: Vec3::Y,
                    frame: ForceFrame::World,
                    target: EffectTarget::Caster,
                },
            },
            ConditionalEffect {
                condition: Condition::Airborne,
                effect: AbilityEffect::ApplyForce {
                    force: -Vec3::Y,
                    frame: ForceFrame::World,
                    target: EffectTarget::Caster,
                },
            },
        ]);
        let pick = |grounded: bool| -> Vec<AbilityEffect> {
            ce.0.iter()
                .filter(|c| match c.condition {
                    Condition::Grounded => grounded,
                    Condition::Airborne => !grounded,
                })
                .map(|c| c.effect.clone())
                .collect()
        };
        assert_eq!(pick(true).len(), 1);
        assert_eq!(pick(false).len(), 1);
        assert_ne!(pick(true), pick(false));
    }
}
```

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test-native` passes (new tests included)

#### Manual
- [ ] No production ability uses `ConditionalEffects` yet — Ability1–4 behavior unchanged.

---

## Phase 5: Jump as a data-defined ability + remove inline jump

### Changes

#### 1. New jump ability asset

**File**: `assets/abilities/jump.ability.ron`
**Action**: create

```ron
#![enable(implicit_some)]
{
    "protocol::ability::AbilityPhases": (startup: 0, active: 1, recovery: 0, cooldown: 0),
    "protocol::ability::ConditionalEffects": ([
        (
            condition: Grounded,
            effect: ApplyForce(force: (0.0, 2000.0, 0.0), frame: World, target: Caster),
        ),
    ]),
}
```

Force value `(0.0, 2000.0, 0.0)` matches the existing inline impulse `Vec3::new(0.0, 2000.0, 0.0)` at `movement.rs:44` — no rescaling needed because that call was already `apply_linear_impulse`.

#### 2. Default slots RON: slot 4 = jump

**File**: `assets/default.ability_slots.ron`
**Action**: modify (replace `None` from Phase 3)

```ron
#![enable(implicit_some)]
(
    (
        AbilityId("punch"),
        AbilityId("speed_burst"),
        AbilityId("ground_pound"),
        AbilityId("blink_strike"),
        AbilityId("jump"),
    )
)
```

#### 3. Add `ApplyForce` arm to `apply_on_tick_effects`

**File**: `crates/protocol/src/ability/effects.rs`
**Action**: modify

The current `apply_on_tick_effects` has no arm for `AbilityEffect::ApplyForce` — it falls through to the `_ => warn!(...)` branch. Add a caster-target impulse arm. Switch `caster_query` to expose `Forces` for impulse application:

```rust
mut caster_query: Query<(&mut Position, &Rotation, &MapInstanceId, Forces)>,
```

Note: `Forces` is a `QueryData`, not a `&mut`. Existing arms (`Melee`, `AreaOfEffect`, `Teleport`) read `(&mut Position, &Rotation, &MapInstanceId)` — they continue to bind `(_, _, _, _)` ignoring the `Forces` slot. Update each `caster_query.get_mut(...)` destructure accordingly.

Add the new arm:

```rust
AbilityEffect::ApplyForce { force, frame, target } => {
    let target_entity = resolve_caster_target(target, active);
    let Ok((position, rotation, _, mut forces)) = caster_query.get_mut(target_entity) else {
        warn!("ApplyForce target {:?} not a rigid body", target_entity);
        continue;
    };
    // Caster-relative resolution: source = caster pos, target = same entity.
    // For `frame: World`, world_force == force; for Caster/RelativeRotation,
    // rotation is the caster's. RelativePosition collapses (caster == target)
    // and falls back to forward-Z; document as not meaningful for self-target.
    let world_force = match frame {
        ForceFrame::World => *force,
        ForceFrame::Caster | ForceFrame::RelativeRotation => rotation.0 * *force,
        ForceFrame::Victim | ForceFrame::RelativePosition => {
            warn!("ApplyForce frame {:?} not meaningful for caster target", frame);
            *force
        }
    };
    forces.apply_linear_impulse(world_force);
    let _ = position; // silence unused if not needed
}
```

Imports gain `ForceFrame` and `avian3d::prelude::Forces` (already imported via `use avian3d::prelude::*;`).

`apply_teleport`'s helper also needs its query type aligned — it currently takes `&mut Query<(&mut Position, &Rotation, &MapInstanceId)>`. Update to the new tuple shape so the caller still type-checks.

#### 4. Delete inline jump from `apply_movement`

**File**: `crates/protocol/src/character/movement.rs`
**Action**: modify (delete lines 25-46)

Remove the entire `if action_state.just_pressed(&PlayerActions::Jump) { ... }` block. The `Jump` action is now consumed by `ability_activation` via slot 4.

After deletion, `apply_movement`'s body no longer references `entity`, `spatial_query`, `position`, `player_map_id`, or `map_ids`. Remove those parameters:

```rust
pub fn apply_movement(
    mass: &ComputedMass,
    delta_secs: f32,
    action_state: &ActionState<PlayerActions>,
    forces: &mut ForcesItem,
)
```

Update both callers:

**File**: `crates/client/src/gameplay.rs`
**Action**: modify (`handle_character_movement` query + call)

Drop `Entity`, `&Position`, `Option<&MapInstanceId>` from the query, and drop `spatial_query`/`map_ids` system params. Call becomes:
```rust
apply_movement(mass, time.delta_secs(), action_state, &mut forces);
```

**File**: `crates/server/src/gameplay.rs`
**Action**: modify (same)

Same pruning. Note: Phase 2 added `detect_grounded` as a separate system that owns `SpatialQuery`/`MapInstanceId` — the movement system no longer needs them.

#### 5. Confirm `Jump` key binding remains

**File**: `crates/client/src/gameplay.rs:51-62`
**Action**: no change

`InputMap::new([(PlayerActions::Jump, KeyCode::Space)])` is unchanged — `ability_activation` reads `just_pressed(&PlayerActions::Jump)` via the slot lookup.

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test-native` passes

#### Manual (user-driven — test with `cargo server` + `cargo client`)
- [ ] Hold/tap Space on ground → repeated jumps with same arc and cadence as the pre-Phase-5 build (compare side by side).
- [ ] Press Space mid-air → no impulse, no `ActiveAbility` entity spawned (check via `bevy-inspector` or temporary `trace!` in `ability_activation`'s "refused" branch), no Y-velocity bump.
- [ ] Edit `assets/abilities/jump.ability.ron`, swap `Grounded` → `Airborne`, save (asset hot reload). Verify Space now only fires mid-air.
- [ ] Restore `Grounded`, then delete the `ConditionalEffects` line entirely. Verify Space fires unconditionally (proves the gate is data-driven).
- [ ] Activate punch (`Digit1`) on a target — knockback unchanged from Phase 1 baseline.

---

## Notes on deviations from `structure.md`

- Structure mentions only `punch1.ability.ron`; the actual file is `punch.ability.ron`, and **11** ability RON files use `ApplyForce`. Structure already says "and any other ability RON using `ApplyForce`" — all 11 are listed in Phase 1 above.
- The example value `(0.0, 0.9, 2.85)` cited in structure does not match any current file's value (real punch is `(0.0, 0.9, 0.5)`). The plan rescales each file by the same mass factor rather than chasing the structure's placeholder.
- `apply_on_tick_effects` currently has no `ApplyForce` arm at all — Phase 5 adds it (rather than "reusing the helper from Phase 1", since Phase 1's site is `apply_on_hit_effects`, a different system with a different query shape). The shared element is the `forces.apply_linear_impulse` call, not a function helper.

## Addendum: post-Phase-4 simplification of `apply_ability_archetype`

The originally specified `apply_ability_archetype_with_on_tick_override` (Phase 4 §3) was deleted. Instead, `apply_ability_archetype` was extended with one extra parameter:

```rust
pub(crate) fn apply_ability_archetype(
    commands: &mut Commands,
    entity: Entity,
    asset: &AbilityAsset,
    registry: TypeRegistryArc,
    extra_tick_effects: Vec<TickEffect>,
)
```

After the existing reflect-insert closure runs, if `extra_tick_effects` is non-empty the function merges them with the asset's `OnTickEffects` (or starts a default if absent) and queues a typed `commands.entity(entity).insert(merged)`. Because commands flush in queue order, this typed insert overwrites the reflected `OnTickEffects` from the same asset.

Consequences:
- The two-branch `if conditional.is_some() { override } else { plain }` in `ability_activation` collapses to a single call site.
- One redundant filter+re-clone pass over the asset's components is eliminated; in exchange we accept one wasted reflect-insert when both an asset `OnTickEffects` and a `ConditionalEffects` match are present (one tiny `Vec` is inserted then immediately overwritten).
- `extract_on_tick_effects` becomes private to `loader.rs` — only the merge logic uses it.
- `spawn_sub_ability` (the only other caller) now passes `Vec::new()`.

Rationale was the second `commands.queue(move |world: &mut World| ...)` block in the override variant — a code smell, since the typed-insert path expresses the "overwrite OnTickEffects" intent more directly.
