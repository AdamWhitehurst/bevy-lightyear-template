---
date: 2026-02-22T14:21:20-0800
researcher: Claude Opus 4.6
git_commit: e98449d4fde33758e451ac37166f60771688d69b
branch: master
repository: bevy-lightyear-template
topic: "What remains to implement from the ability-effect-primitives design"
tags: [research, codebase, ability, effects, OnHit, OnEnd, OnInput, Damage, ApplyForce, Buff, Shield, Teleport, Grab, AreaOfEffect, Ability, Summon, hitbox-entities]
status: complete
last_updated: 2026-02-22
last_updated_by: Claude Opus 4.6
---

# Research: Remaining Ability Effect Primitives Implementation

**Date**: 2026-02-22T14:21:20-0800
**Git Commit**: e98449d4fde33758e451ac37166f60771688d69b
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

The foundation plan ([doc/plans/2026-02-21-ability-entity-foundation.md](doc/plans/2026-02-21-ability-entity-foundation.md)) has been implemented. What remains to fully implement the design doc ([doc/design/2026-02-13-ability-effect-primitives.md](doc/design/2026-02-13-ability-effect-primitives.md))?

## Summary

The foundation plan delivered: `ActiveAbility` as a prespawned/predicted entity, `Vec<EffectTrigger>` on `AbilityDef`, `OnCast`/`WhileActive` triggers, `MapEntities`, `prediction_despawn()`, and the three existing abilities (punch, dash, fireball) migrated. What remains is 3 trigger types, 9 effect variants, 4 new components, hitbox entity spawning, and a refactoring of how damage/knockback flows through the system.

---

## Implemented vs. Remaining

### EffectTrigger Variants

| Variant | Design Signature | Status |
|---|---|---|
| `OnCast(AbilityEffect)` | Same | **Done** ([ability.rs:56](crates/protocol/src/ability.rs#L56)) |
| `WhileActive(AbilityEffect)` | Same | **Done** ([ability.rs:58](crates/protocol/src/ability.rs#L58)) |
| `OnHit(AbilityEffect)` | Same | **Not implemented** |
| `OnEnd(AbilityEffect)` | Same | **Not implemented** |
| `OnInput { action: PlayerActions, effect: AbilityEffect }` | Same | **Not implemented** |

### AbilityEffect Variants

| Variant | Design Signature | Current State |
|---|---|---|
| `Melee` | `{ id: Option<String>, target: EffectTarget }` | **Exists but different**: `{ knockback_force: f32, base_damage: f32 }` ([ability.rs:47](crates/protocol/src/ability.rs#L47)). Bakes damage/knockback inline instead of composing via `OnHit(Damage)` + `OnHit(ApplyForce)`. |
| `Projectile` | `{ id: String, speed: f32, lifetime_ticks: u16 }` | **Exists but different**: `{ speed, lifetime_ticks, knockback_force, base_damage }` ([ability.rs:48](crates/protocol/src/ability.rs#L48)). Same issue — damage/knockback baked in. No sub-ability `id`. |
| `SetVelocity` | `{ speed: f32, target: EffectTarget }` | **Done** ([ability.rs:49](crates/protocol/src/ability.rs#L49)) |
| `Damage` | `{ amount: f32, target: EffectTarget }` | **Not implemented** |
| `ApplyForce` | `{ force: f32, target: EffectTarget }` | **Not implemented** |
| `AreaOfEffect` | `{ id: Option<String>, target: EffectTarget, radius: f32 }` | **Not implemented** |
| `Grab` | (no fields) | **Not implemented** |
| `Buff` | `{ stat: String, multiplier: f32, duration_ticks: u16, target: EffectTarget }` | **Not implemented** |
| `Shield` | `{ absorb: f32 }` | **Not implemented** |
| `Teleport` | `{ distance: f32 }` | **Not implemented** |
| `Summon` | `{ entity_type: String, lifetime_ticks: u16 }` | **Not implemented** (design says "implement last") |
| `Ability` | `{ id: String, target: EffectTarget }` | **Not implemented** |

### Marker Components

| Component | Location | Status |
|---|---|---|
| `OnCastEffects(Vec<AbilityEffect>)` | ActiveAbility entity | **Done** ([ability.rs:186](crates/protocol/src/ability.rs#L186)) |
| `WhileActiveEffects(Vec<AbilityEffect>)` | ActiveAbility entity | **Done** ([ability.rs:190](crates/protocol/src/ability.rs#L190)) |
| `OnInputEffects(Vec<(PlayerActions, AbilityEffect)>)` | ActiveAbility entity | **Not implemented** |
| `OnEndEffects(Vec<AbilityEffect>)` | ActiveAbility entity | **Not implemented** |
| `OnHitEffects { effects, caster, original_caster, depth }` | Hitbox/AoE entity | **Not implemented** |

### Gameplay Components

| Component | Purpose | Status |
|---|---|---|
| `ActiveBuff { stat, multiplier, expires_tick }` | Temp stat modifier on target | **Not implemented** |
| `ActiveShield { remaining }` | Damage absorption | **Not implemented** |
| `GrabbedBy(Entity)` | On victim, references grabber | **Not implemented** |
| `Grabbing(Entity)` | On grabber, references victim | **Not implemented** |

### Effect Processing Systems

| System | Status |
|---|---|
| `apply_on_cast_effects` | **Done** ([ability.rs:443](crates/protocol/src/ability.rs#L443)) |
| `apply_while_active_effects` | **Done** ([ability.rs:474](crates/protocol/src/ability.rs#L474)) |
| `apply_on_hit_effects` | **Not implemented** — runs from hit detection |
| `apply_on_end_effects` | **Not implemented** |
| `apply_on_input_effects` | **Not implemented** |

---

## Detailed Analysis of Remaining Work

### 1. The Damage/Knockback Refactoring Problem

The central architectural gap: the design wants damage and knockback to be **composable effects** (`Damage`, `ApplyForce`) that fire via `OnHit` trigger. The current implementation hardcodes them:

**Current flow (Melee)**:
```
AbilityDef.effects → OnCast(Melee { knockback_force, base_damage })
  → MeleeHitboxActive { knockback_force, base_damage }
    → process_melee_hits → apply_hit(knockback_force, base_damage)
```

**Current flow (Projectile)**:
```
AbilityDef.effects → OnCast(Projectile { speed, lifetime_ticks, knockback_force, base_damage })
  → AbilityProjectileSpawn { ..., knockback_force, base_damage }
    → bullet entity with KnockbackForce + DamageAmount components
      → process_projectile_hits → apply_hit(knockback_force, base_damage)
```

**Design target flow**:
```
AbilityDef.effects → [OnCast(Melee { id: None }), OnHit(Damage { amount, target: Victim }), OnHit(ApplyForce { force, target: Victim })]
  → hitbox entity with OnHitEffects { effects: [Damage, ApplyForce], caster, original_caster, depth }
    → hit detection → apply_on_hit_effects processes each effect
```

This refactoring requires: `OnHit` trigger, `OnHitEffects` component, `Damage` variant, `ApplyForce` variant, and changes to `Melee`/`Projectile` signatures.

### 2. OnHit Trigger

The `OnHit` trigger is fundamentally different from `OnCast`/`WhileActive`/`OnEnd`/`OnInput`:

- `OnCast`/`WhileActive`/`OnEnd`/`OnInput` are dispatched as marker components on the **ActiveAbility entity** by `dispatch_effect_markers`.
- `OnHit` effects are carried on the **hitbox/projectile entity** as `OnHitEffects`, populated at spawn time from the ability's trigger list. When a hit occurs, the hit detection system reads `OnHitEffects` and applies each effect to the victim.

The `OnHitEffects` component needs entity references for resolution:
```rust
struct OnHitEffects {
    effects: Vec<AbilityEffect>,
    caster: Entity,           // immediate caster
    original_caster: Entity,  // top-level character
    depth: u8,                // for recursion limit
}
```

The design says `apply_on_hit_effects` "runs synchronously from within hit detection since it needs the hit contact information (attacker position, victim entity) available at that point."

### 3. Hitbox Entity Spawning

Currently, melee uses ephemeral spatial queries ([hit_detection.rs:68-109](crates/protocol/src/hit_detection.rs#L68)) — no hitbox entity exists. The design envisions spawned hitbox entities with:

- `Collider` (cuboid for melee, sphere for AoE)
- `Sensor` + `CollidingEntities` + `CollisionEventsEnabled` (same as bullets)
- `GameLayer::Hitbox` collision layer (already defined at [hit_detection.rs:28](crates/protocol/src/hit_detection.rs#L28), characters already interact with it at [hit_detection.rs:40](crates/protocol/src/hit_detection.rs#L40))
- `DisableRollback` (same as bullets, per research doc's resolved question 8)
- `OnHitEffects` carrying the `OnHit` effects from the ability definition
- A custom relationship to the `ActiveAbility` entity (like `AbilityBulletOf`/`AbilityBullets`)
- Lifetime tied to the Active phase of the ability

The research doc ([doc/research/2026-02-21-ability-effect-primitives-lightyear-hierarchy.md](doc/research/2026-02-21-ability-effect-primitives-lightyear-hierarchy.md), section "Hitbox Entity Spawning Strategy", resolved question 8) decided: **local-only with `DisableRollback`**, using a custom relationship (not `ChildOf`).

A `hitbox_collision_layers()` helper is needed (currently missing).

### 4. OnEnd Trigger

Fires once when the ability transitions from Active → Recovery. `dispatch_effect_markers` needs to detect this transition and insert `OnEndEffects` on the ActiveAbility entity. The `apply_on_end_effects` system consumes and removes it.

Detection mechanism: `advance_ability_phase` at [ability.rs:335-362](crates/protocol/src/ability.rs#L335) already handles the Active → Recovery transition. The dispatch system runs after `update_active_abilities`, so it can detect that `active.phase == Recovery && active.phase_start_tick == tick` to know the transition just happened.

### 5. OnInput Trigger

Fires during Active phase when a specific `PlayerActions` input is `just_pressed`. Used for combo chaining (e.g., pressing Ability1 during punch's Active window triggers `Ability(id: "punch2")`).

Needs:
- `OnInputEffects(Vec<(PlayerActions, AbilityEffect)>)` marker on ActiveAbility entity
- `apply_on_input_effects` system that reads caster's `ActionState<PlayerActions>` and checks `just_pressed` for each action in the list
- Dispatched by `dispatch_effect_markers` every tick during Active phase (like `WhileActiveEffects`)

### 6. Ability { id, target } — Recursive Sub-Ability Dispatch

Spawns a new `ActiveAbility` entity for ability `id`, targeting `target`. The `ActiveAbility` struct already has `depth: u8` for recursion tracking ([ability.rs:129](crates/protocol/src/ability.rs#L129)). The system caps at depth 4.

This variant can appear in any trigger context:
- `OnCast(Ability { id, target })` — spawn sub-ability when entering Active phase
- `OnHit(Ability { id, target })` — spawn sub-ability per hit
- `OnInput(action, Ability { id, target })` — spawn sub-ability on button press (combo chaining)
- `OnEnd(Ability { id, target })` — spawn sub-ability when ability ends

The sub-ability entity needs the same prespawn pattern as top-level abilities, with a salt that encodes depth to prevent hash collisions between parent and child abilities.

### 7. AreaOfEffect

Similar to `Melee` but uses a sphere hitbox around the caster rather than a cuboid in front of the caster. Needs:
- Spawns a hitbox entity with `Collider::sphere(radius)` at caster position
- `GameLayer::Hitbox` collision layer
- `OnHitEffects` from the ability's `OnHit` triggers
- Optional sub-ability `id` like `Melee`

### 8. Buff, Shield, Teleport, Grab

These are new effect variant implementations:

**Buff**: Inserts `ActiveBuff { stat, multiplier, expires_tick }` on the target entity. Needs a tick-based expiry system and a way for stat queries to incorporate active buffs. Needs lightyear prediction registration (`.add_prediction()`).

**Shield**: Inserts `ActiveShield { remaining }` on the caster. The `apply_hit` function (or its replacement) must check for `ActiveShield` before applying damage: absorb up to `remaining`, then apply overflow to `Health`. Needs prediction registration.

**Teleport**: Instantly repositions the caster by `distance` in facing direction. Needs collision checking to avoid teleporting through walls (or explicitly no collision as design says "no collision during transit").

**Grab**: Complex two-part system:
1. `OnHit(Grab)` inserts `GrabbedBy(caster)` on victim and `Grabbing(victim)` on caster
2. A tick system drives grabbed victim's `Position` to trail the caster
3. Victim swaps to `RigidBody::Kinematic` while grabbed
4. On ability end (or victim death), restores `RigidBody::Dynamic` and removes both components
5. `throw` is a separate ability that resolves `Victim` by querying `GrabbedBy` — needs special `EffectTarget::Victim` resolution for abilities without a hitbox

Both `GrabbedBy` and `Grabbing` need `MapEntities` + `.add_prediction()` for rollback correctness.

---

## Current System Schedule

```
FixedUpdate (chained, run_if AppState::Ready):
  1. ability_activation
  2. update_active_abilities
  3. dispatch_effect_markers
  4. apply_on_cast_effects
  5. apply_while_active_effects
  6. ability_projectile_spawn

FixedUpdate (chained, .after(apply_on_cast_effects), run_if AppState::Ready):
  1. ensure_melee_hit_targets
  2. process_melee_hits
  3. process_projectile_hits

FixedUpdate (unordered, run_if AppState::Ready):
  - update_facing
  - ability_bullet_lifetime

PreUpdate:
  - handle_ability_projectile_spawn

Observers:
  - despawn_ability_projectile_spawn
  - cleanup_effect_markers_on_removal
```

Note: `handle_character_movement` (server + client) has **no explicit ordering** relative to ability systems. Both run in `FixedUpdate` but no `.before()`/`.after()` is declared.

### Target Schedule (with all triggers)

```
FixedUpdate (chained):
  1. ability_activation
  2. update_active_abilities
  3. dispatch_effect_markers          // inserts OnCast, WhileActive, OnInput, OnEnd markers
  4. apply_on_cast_effects            // processes Melee, Projectile, AoE, Shield, Buff, Teleport, Ability
  5. apply_while_active_effects       // processes SetVelocity, Grab position tracking
  6. apply_on_input_effects           // processes OnInput (combo chaining)
  7. apply_on_end_effects             // processes OnEnd effects
  8. ability_projectile_spawn         // spawns projectile entities from ProjectileSpawnEffect

FixedUpdate (chained, after apply_on_cast_effects):
  1. ensure_melee_hit_targets / spawn hitbox entities
  2. process_melee_hits               // reads OnHitEffects from hitbox entities
  3. process_projectile_hits          // reads OnHitEffects from projectile entities
  4. apply_on_hit_effects             // or inlined into hit detection systems
```

---

## Existing Infrastructure That Supports Remaining Work

### Already in place:
- `ActiveAbility.depth` field for recursion tracking ([ability.rs:129](crates/protocol/src/ability.rs#L129))
- `EffectTarget` enum with `Caster`, `Victim`, `OriginalCaster` ([ability.rs:38-42](crates/protocol/src/ability.rs#L38))
- `GameLayer::Hitbox` defined and characters configured to interact with it ([hit_detection.rs:28,40](crates/protocol/src/hit_detection.rs#L28))
- `AbilityBulletOf`/`AbilityBullets` relationship pattern with `linked_spawn` ([ability.rs:207-214](crates/protocol/src/ability.rs#L207))
- `prediction_despawn()` used correctly on ActiveAbility entities ([ability.rs:359,376](crates/protocol/src/ability.rs#L359))
- `MapEntities` impl on `ActiveAbility` ([ability.rs:132-138](crates/protocol/src/ability.rs#L132))
- `PlayerId` on character entities for prespawn salt ([lib.rs:63](crates/protocol/src/lib.rs#L63))
- `cleanup_effect_markers_on_removal` observer ([ability.rs:505-516](crates/protocol/src/ability.rs#L505))
- `dispatch_effect_markers` / `dispatch_active_phase_markers` / `remove_active_phase_markers` pattern ([ability.rs:385-440](crates/protocol/src/ability.rs#L385))
- `apply_hit` shared function ([hit_detection.rs:135-155](crates/protocol/src/hit_detection.rs#L135))
- Projectile entity spawn pattern with `Sensor` + `CollidingEntities` + `DisableRollback` ([ability.rs:572-588](crates/protocol/src/ability.rs#L572))

### Not yet in place:
- `hitbox_collision_layers()` helper function
- `ActiveAbilityOf`/`ActiveAbilities` custom relationship (deferred in foundation plan)
- `HitboxOf`/`ActiveAbilityHitboxes` custom relationship for hitbox entities
- Buff/Shield/Grab stat and state systems
- `RigidBody` swapping mechanism (for Grab)
- `EffectTarget::Victim` resolution outside of hit context (for throw)

---

## Dependency Graph for Implementation

```
                     OnHit trigger
                    /      |       \
                   v       v        v
              Damage  ApplyForce  Ability{id}
                 \       /            |
                  v     v             v
           Refactor Melee/Proj    OnInput trigger
           to use OnHit comp.     (combo chaining)
                   |
                   v
           Hitbox entity spawning
                   |
                   v
              AreaOfEffect
```

Independent tracks (no dependencies on OnHit):
- `OnEnd` trigger
- `Teleport`
- `Buff` / `Shield` (need new components + expiry system)
- `Grab` (needs new components + RigidBody swap + Victim resolution)
- `Summon` (deferred)

### Suggested implementation phases:

**Phase 2**: `OnHit` trigger + `Damage` + `ApplyForce` + `OnHitEffects` component. Refactor `Melee`/`Projectile` signatures. Migrate `apply_hit` to be driven by `OnHitEffects`. This is the largest single change.

**Phase 3**: `OnEnd` trigger (independent, small). `OnEndEffects` marker + `apply_on_end_effects` system.

**Phase 4**: `Ability { id, target }` variant (recursive sub-ability spawning). Needed before OnInput becomes useful.

**Phase 5**: `OnInput` trigger + combo chaining. Punch gets its 3-step combo back.

**Phase 6**: Hitbox entity spawning + `AreaOfEffect`. Replace spatial query melee with entity-based hitboxes.

**Phase 7**: `Buff` / `Shield` / `Teleport` (independent of each other).

**Phase 8**: `Grab` (complex, needs Buff/Shield patterns established first).

**Phase 9**: `Summon` (needs entity behavior system — deferred).

---

## Test Infrastructure

Tests live at [crates/protocol/tests/ability_systems.rs](crates/protocol/tests/ability_systems.rs) (347 lines, 9 tests). No lightyear mocking — pure ECS with `MinimalPlugins` and manual `LocalTimeline`. Two tests are `#[ignore]` because `PreSpawned` hooks require a full lightyear runtime. Tests spawn `ActiveAbility` as separate entities (matching new architecture). Test helpers: `test_defs()`, `test_app()`, `spawn_timeline()`, `advance_timeline()`, `spawn_character()`, `find_active_ability()`.

Each new effect variant and trigger type will need tests following these patterns.

## Code References

- [crates/protocol/src/ability.rs](crates/protocol/src/ability.rs) — all ability types, systems, entity spawning
- [crates/protocol/src/hit_detection.rs](crates/protocol/src/hit_detection.rs) — melee spatial query, projectile collision, `apply_hit`
- [crates/protocol/src/lib.rs:248-278](crates/protocol/src/lib.rs#L248) — system schedule
- [crates/protocol/src/lib.rs:166-206](crates/protocol/src/lib.rs#L166) — lightyear component registration
- [crates/server/src/gameplay.rs:128-181](crates/server/src/gameplay.rs#L128) — character spawn with `PlayerId`
- [crates/client/src/gameplay.rs:55-80](crates/client/src/gameplay.rs#L55) — client movement (no `Without<ActiveAbility>`)
- [crates/protocol/tests/ability_systems.rs](crates/protocol/tests/ability_systems.rs) — test infrastructure
- [assets/abilities.ron](assets/abilities.ron) — 3 ability definitions (punch, dash, fireball)

## Related Research

- [doc/research/2026-02-21-ability-effect-primitives-lightyear-hierarchy.md](doc/research/2026-02-21-ability-effect-primitives-lightyear-hierarchy.md) — lightyear replication/prediction patterns, resolved architectural questions
- [doc/plans/2026-02-21-ability-entity-foundation.md](doc/plans/2026-02-21-ability-entity-foundation.md) — phase 1 implementation plan (completed)
- [doc/design/2026-02-13-ability-effect-primitives.md](doc/design/2026-02-13-ability-effect-primitives.md) — full design vision

## Resolved Questions

1. **Melee/Projectile signature change**: All at once — change Melee/Projectile signatures atomically with OnHit trigger in the same change.

2. **Hitbox entity spawning vs spatial query**: Deferred until AreaOfEffect needs them. Keep spatial query for melee; introduce hitbox entities in Phase 6.

3. **Movement ordering**: No explicit ordering needed. Current accidental ordering (SetVelocity runs after movement) is sufficient.

## Open Questions

None remaining.
