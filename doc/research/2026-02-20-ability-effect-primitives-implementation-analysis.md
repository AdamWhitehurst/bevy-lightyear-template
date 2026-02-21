---
date: 2026-02-20T14:29:45-0800
researcher: Claude Sonnet 4.6
git_commit: 83c1c6a7b81ae8df6f3b15e94b7d507f6ff679db
branch: master
repository: bevy-lightyear-template
topic: "How to implement ability-effect-primitives design"
tags: [research, codebase, ability, effects, lightyear, avian3d, hit-detection]
status: complete
last_updated: 2026-02-20
last_updated_by: Claude Sonnet 4.6
---

# Research: How to Implement Ability Effect Primitives

**Date**: 2026-02-20T14:29:45-0800
**Git Commit**: 83c1c6a7b81ae8df6f3b15e94b7d507f6ff679db
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to implement the changes described in `doc/design/2026-02-13-ability-effect-primitives.md`.

## Summary

The design doc introduces a composable `Vec<EffectTrigger>` model replacing the current single `effect: AbilityEffect` field. The existing codebase has three ability effects (`Melee`, `Projectile`, `Dash`), a marker-component dispatch system, and `ActiveAbility` as a component on character entities. The implementation requires: (1) a data model refactor, (2) a major architectural shift in how `ActiveAbility` works (component → spawned entity), (3) expanding the hit detection pipeline to dispatch effect lists, and (4) new components for buffs, shields, and grabs.

---

## Detailed Findings

### Current Data Model

**File**: [crates/protocol/src/ability.rs](crates/protocol/src/ability.rs)

```rust
// ability.rs:36-52 — current state
pub enum AbilityEffect {
    Melee { knockback_force: f32, base_damage: f32 },
    Projectile { speed: f32, lifetime_ticks: u16, knockback_force: f32, base_damage: f32 },
    Dash { speed: f32 },
}

pub struct AbilityDef {
    pub startup_ticks: u16,
    pub active_ticks: u16,
    pub recovery_ticks: u16,
    pub cooldown_ticks: u16,
    pub steps: u8,
    pub step_window_ticks: u16,
    pub effect: AbilityEffect,  // ← single field, to become Vec<EffectTrigger>
}
```

`AbilityDefs` is a `Resource` (not replicated) loaded from `assets/abilities.ron`. Current RON has three abilities: `punch`, `dash`, `fireball`.

### Current ActiveAbility Architecture

**File**: [crates/protocol/src/ability.rs:103-113](crates/protocol/src/ability.rs#L103)

`ActiveAbility` is a **component on the character entity**, not a standalone entity:

```rust
pub struct ActiveAbility {
    pub ability_id: AbilityId,
    pub phase: AbilityPhase,
    pub phase_start_tick: Tick,>
    pub step: u8,
    pub total_steps: u8,
    pub chain_input_received: bool,
}
```

The `Without<ActiveAbility>` filter on `ability_activation` ([ability.rs:263](crates/protocol/src/ability.rs#L263)) prevents simultaneous abilities. The design doc changes this: multiple `ActiveAbility` **entities** can coexist per character, gated only by `AbilityCooldowns`.

The new `ActiveAbility` structure in the design adds: `caster`, `original_caster`, `target`, `depth`. The existing fields (`ability_id`, `phase`, `phase_start_tick`, `step`, `total_steps`, `chain_input_received`) remain.

### Current Dispatch Architecture

**File**: [crates/protocol/src/ability.rs:392-470](crates/protocol/src/ability.rs#L392)

Current flow uses **marker components** as an indirection layer:

```
dispatch_effect_markers
  ├── While Active: insert DashAbilityEffect / MeleeHitboxActive
  ├── On Active start: insert ProjectileSpawnAbilityEffect (consumed by ability_projectile_spawn)
  └── On exit Active: remove DashAbilityEffect / MeleeHitboxActive / MeleeHitTargets
```

Effect systems (`ability_dash_effect`, `ability_projectile_spawn`) query the markers, not `ActiveAbility` directly. The new design replaces these per-variant marker components with four generic dispatch functions that iterate `Vec<EffectTrigger>`.

The system chain in [lib.rs:239-268](crates/protocol/src/lib.rs#L239):

```
FixedUpdate:
  ability_activation
  → update_active_abilities
  → dispatch_effect_markers
  → ability_projectile_spawn
  → ability_dash_effect
  (then after dispatch_effect_markers:)
  → ensure_melee_hit_targets
  → process_melee_hits
  → process_projectile_hits
```

### Current Hit Detection

**File**: [crates/protocol/src/hit_detection.rs](crates/protocol/src/hit_detection.rs)

Damage and knockback are **hardcoded** in `apply_hit` ([hit_detection.rs:134](crates/protocol/src/hit_detection.rs#L134)) — pulled from `MeleeHitboxActive.base_damage / knockback_force` or `DamageAmount / KnockbackForce` components on projectile entities.

```rust
fn apply_hit(...) {
    velocity.0 += direction * knockback_force;
    if invulnerable.is_none() {
        health.apply_damage(damage);
    }
}
```

The new design routes through `apply_on_hit_effects` instead, iterating `OnHit(...)` entries from the `ActiveAbility` entity's `effects` list. The `apply_hit` function's logic (damage → health, impulse → velocity) maps to the new `Damage` and `ApplyForce` primitives.

Physics: **avian3d** (`ExternalImpulse` available). Current code modifies `LinearVelocity` directly (`velocity.0 += ...`) rather than using `ExternalImpulse`.

### Lightyear Registration

**File**: [crates/protocol/src/lib.rs:178-183](crates/protocol/src/lib.rs#L178)

| Component                | Registration        |
| ------------------------ | ------------------- |
| `AbilitySlots`           | replicate-only      |
| `ActiveAbility`          | `.add_prediction()` |
| `AbilityCooldowns`       | `.add_prediction()` |
| `AbilityProjectileSpawn` | replicate-only      |

New components (`ActiveBuff`, `ActiveShield`, `GrabbedBy`, `Grabbing`) will need registration. Components that affect predicted state (e.g., `ActiveShield` intercepting damage, `GrabbedBy` moving a character) should use `.add_prediction()`.

Spawned `ActiveAbility` **entities** (new arch): follow the pattern at [ability.rs:504-516](crates/protocol/src/ability.rs#L504) — `PreSpawned`, `Replicate`, `PredictionTarget`, `ControlledBy` if the owner is a controlled client.

---

## Architecture Documentation

### Delta: Single Effect → Trigger List

| Aspect                                   | Current                                                      | New                                                                                                |
| ---------------------------------------- | ------------------------------------------------------------ | -------------------------------------------------------------------------------------------------- |
| `AbilityDef.effect`                      | `AbilityEffect`                                              | `Vec<EffectTrigger>`                                                                               |
| `AbilityDef.steps` / `step_window_ticks` | present                                                      | **removed** — replaced by `OnInput` trigger                                                        |
| Effect variants                          | 3 (`Melee`, `Projectile`, `Dash`)                            | 12 total                                                                                           |
| `Dash`                                   | standalone variant                                           | replaced by `WhileActive(SetVelocity(...))`                                                        |
| `Melee` fields                           | `knockback_force`, `base_damage` inline                      | `id: Option<String>` only; damage/knockback become `OnHit(Damage(...))` / `OnHit(ApplyForce(...))` |
| `Projectile` fields                      | carries damage/knockback                                     | `id`, `speed`, `lifetime_ticks` only; sub-ability handles effects                                  |
| Dispatch                                 | marker components per variant                                | 5 generic dispatch fns iterate trigger list                                                        |
| Combo chaining                           | `steps` + `step_window_ticks` + `chain_input_received` field | `OnInput { action: PlayerActions, effect }` trigger, fires during Active phase on `just_pressed`   |

### Delta: ActiveAbility Architecture

| Aspect      | Current                                                                                  | New                                                                                                               |
| ----------- | ---------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Storage     | Component on character entity                                                            | Standalone spawned entity                                                                                         |
| Concurrency | One at a time (filter)                                                                   | Multiple per character allowed                                                                                    |
| Gating      | `Without<ActiveAbility>`                                                                 | `AbilityCooldowns` only                                                                                           |
| Fields      | `ability_id`, `phase`, `phase_start_tick`, `step`, `total_steps`, `chain_input_received` | `def_id`, `phase`, `phase_start_tick`, `caster`, `original_caster`, `target`, `depth` — step/chain fields removed |

### Effect Dispatch Map

`dispatch_effect_markers` partitions `Vec<EffectTrigger>` into per-trigger-type marker components on the `ActiveAbility` entity. Downstream systems query only their own marker type — same parallelism guarantee as the current architecture.

| System                       | Marker queried                                                    | When                                       |
| ---------------------------- | ----------------------------------------------------------------- | ------------------------------------------ |
| `apply_on_cast_effects`      | `OnCastEffects` (one-shot, consumed)                              | First Active tick                          |
| `apply_while_active_effects` | `WhileActiveEffects` (persistent while Active)                    | Every Active tick                          |
| `apply_on_input_effects`     | `OnInputEffects` (persistent while Active) + caster `ActionState` | Every Active tick, fires on `just_pressed` |
| `apply_on_hit_effects`       | `OnHitEffects` on hitbox entity                                   | From hit detection                         |
| `apply_on_end_effects`       | `OnEndEffects` (one-shot, consumed)                               | Active → Recovery transition tick          |

### New Components Required

| Component                                       | Purpose                 | Lightyear reg       |
| ----------------------------------------------- | ----------------------- | ------------------- |
| `ActiveBuff { stat, multiplier, expires_tick }` | Temp stat modifier      | `.add_prediction()` |
| `ActiveShield { remaining }`                    | Damage absorption       | `.add_prediction()` |
| `GrabbedBy(Entity)`                             | Victim locked to caster | `.add_prediction()` |
| `Grabbing(Entity)`                              | Bidirectional mirror    | `.add_prediction()` |

---

## Implementation Order (from design doc)

1. **Refactor `effect` → `effects: Vec<EffectTrigger>`; remove `steps` / `step_window_ticks`; remove `step` / `total_steps` / `chain_input_received` from `ActiveAbility`; remove `set_chain_input_received` and `has_more_steps`**
   - Add `EffectTrigger` enum and `EffectTarget` enum to `ability.rs`
   - Change `AbilityDef.effect: AbilityEffect` → `AbilityDef.effects: Vec<EffectTrigger>`
   - Rewrite `AbilityEffect` variants: rename `Dash` → `SetVelocity { speed, target }`, strip damage/knockback from `Melee` and `Projectile`
   - Migrate `assets/abilities.ron`: `punch` → `[OnCast(Melee()), OnHit(Damage(...)), OnHit(ApplyForce(...))]`; `dash` → `[WhileActive(SetVelocity(...))]`; `fireball` → keep as `OnCast(Projectile(...))` with sub-ability holding hit effects
   - Update tests in `crates/protocol/tests/ability_systems.rs`

2. **Update trigger dispatch systems**
   - Remove `dispatch_while_active_markers`, `dispatch_on_cast_markers`, `remove_while_active_markers`
   - Rewrite `dispatch_effect_markers` to partition `Vec<EffectTrigger>` into `OnCastEffects`, `WhileActiveEffects`, `OnInputEffects`, `OnEndEffects` marker components on the `ActiveAbility` entity
   - `OnHitEffects` is placed on spawned hitbox/AoE entities (not the `ActiveAbility` entity)
   - Write `apply_on_cast_effects`, `apply_while_active_effects`, `apply_on_input_effects`, `apply_on_end_effects`, each querying only its own marker type
   - `MeleeHitTargets` moves to the spawned hitbox entity; `MeleeHitboxActive` is replaced by `OnHitEffects` on that entity
   - Rewrite `ability_dash_effect` → `apply_while_active_effects` handling `SetVelocity` from `WhileActiveEffects`
   - `apply_on_input_effects` needs a second query for the caster's `ActionState<PlayerActions>` (looked up via `active.caster`)

3. **`ApplyForce`**
   - `apply_on_hit_effects` handles `ApplyForce { force, target }`: compute direction, apply `LinearVelocity` impulse (or `ExternalImpulse`)
   - Replaces the hardcoded knockback in `apply_hit`

4. **`AreaOfEffect`**
   - Similar to `Melee` but spherical spatial query, fires for each entity in radius
   - Needs its own active marker component (like `MeleeHitboxActive`) or reuse spatial query pattern inline

5. **`Buff` / `Shield`**
   - Insert `ActiveBuff` / `ActiveShield` on target when `apply_on_cast_effects` fires
   - Tick-expiry system for `ActiveBuff`; `ActiveShield` absorbs incoming `Damage` in `apply_on_hit_effects`
   - Register both with `.add_prediction()`

6. **`Teleport`**
   - `apply_on_cast_effects`: set `Position` directly to `caster_pos + facing_dir * distance`

7. **`Grab`**
   - `apply_on_hit_effects`: insert `GrabbedBy(caster)` on victim, `Grabbing(victim)` on caster
   - A system each tick drives victim `Position` to trail caster; swaps `RigidBody::Kinematic`
   - On `ActiveAbility` removal or victim death, restore `RigidBody::Dynamic` and remove both components
   - `throw` ability: `apply_on_cast_effects` for `ApplyForce`, with `Victim` resolved via `GrabbedBy` query

8. **`Ability { id, target }` (recursive)**
   - `apply_on_cast_effects` / `apply_on_hit_effects`: spawn new `ActiveAbility` entity for the sub-ability
   - Propagate `original_caster`; increment `depth`; cap at 4

9. **`Summon` (deferred)**
   - Requires entity behavior system not yet defined; skip

---

## Code References

- [crates/protocol/src/ability.rs](crates/protocol/src/ability.rs) — all ability types and systems
- [crates/protocol/src/ability.rs:36-52](crates/protocol/src/ability.rs#L36) — `AbilityEffect` and `AbilityDef` (the root of the refactor)
- [crates/protocol/src/ability.rs:103-113](crates/protocol/src/ability.rs#L103) — `ActiveAbility` (becomes a standalone entity)
- [crates/protocol/src/ability.rs:252-295](crates/protocol/src/ability.rs#L252) — `ability_activation` (remove `Without<ActiveAbility>` filter)
- [crates/protocol/src/ability.rs:392-470](crates/protocol/src/ability.rs#L392) — `dispatch_effect_markers` and helpers (full rewrite)
- [crates/protocol/src/hit_detection.rs:134-154](crates/protocol/src/hit_detection.rs#L134) — `apply_hit` (replace with `apply_on_hit_effects`)
- [crates/protocol/src/lib.rs:178-183](crates/protocol/src/lib.rs#L178) — Lightyear component registration
- [crates/protocol/src/lib.rs:239-268](crates/protocol/src/lib.rs#L239) — system schedule (add new dispatch fns, preserve ordering)
- [assets/abilities.ron](assets/abilities.ron) — RON definitions to migrate
- [crates/protocol/tests/ability_systems.rs](crates/protocol/tests/ability_systems.rs) — tests to update

---

## Open Questions

- **`throw` victim resolution**: The design resolves `Victim` in `apply_on_cast_effects` for `throw` by querying `GrabbedBy(caster_entity)`. This means `apply_on_cast_effects` needs to handle `ApplyForce` specially when target is `Victim` in a grab context (vs. the normal hit-context meaning of `Victim`). The design says "resolves via `GrabbedBy`" but the exact query + entity lookup plumbing is not specified.
- **`AreaOfEffect` per-tick vs. on-cast**: The design marks `AreaOfEffect` as `OnCast`, but if it should re-fire each tick (like `Melee` does continuously via `MeleeHitboxActive`), it may need a `WhileActive` trigger variant instead.
- **`ActiveAbility` entity replication**: Spawned `ActiveAbility` entities need a Lightyear replication strategy. The existing pattern for `AbilityProjectileSpawn` ([ability.rs:504-516](crates/protocol/src/ability.rs#L504)) using `PreSpawned` + salt is the likely model, but the deterministic salt for arbitrary sub-abilities needs design.
- **`ActiveAbility` component registration**: Currently `ActiveAbility` is a component registered with `.add_prediction()`. In the new model it becomes the identity of a spawned entity — it may need to stay as a replicated component on that entity (same registration) or transition to a different pattern.
