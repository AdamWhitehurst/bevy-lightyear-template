# Ability Effect Primitives Plan

## Overview

Refactor the ability system from a single `AbilityEffect` per ability to a composable `Vec<EffectTrigger>` model, and add new effect primitives for a brawler moveset.

## Architecture Changes

### EffectTrigger wrapper

Controls *when* an effect fires, replacing the current single `effect` field on `AbilityDef`.

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum EffectTrigger {
    /// Fires once when ability enters Active phase
    OnCast(AbilityEffect),
    /// Fires every tick during Active phase
    WhileActive(AbilityEffect),
    /// Fires when the ability's hitbox connects with an entity
    OnHit(AbilityEffect),
    /// Fires once when ability exits Active phase (enters Recovery)
    OnEnd(AbilityEffect),
    /// Fires once during Active phase when the specified input is just-pressed.
    /// `action` must be a `PlayerActions` variant (serializable for RON).
    /// Replaces the old `steps` / `step_window_ticks` combo mechanism.
    OnInput { action: PlayerActions, effect: AbilityEffect },
}
```

### AbilityDef change

```rust
pub struct AbilityDef {
    pub id: String,
    pub startup_ticks: u16,
    pub active_ticks: u16,
    pub recovery_ticks: u16,
    pub cooldown_ticks: u16,
    // BEFORE: pub steps: u8, pub step_window_ticks: u16, pub effect: AbilityEffect,
    pub effects: Vec<EffectTrigger>,
}
```

### EffectTarget enum

Specifies who receives the effect. Resolution depends on the `ActiveAbility` entity's fields:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum EffectTarget {
    /// The `caster` field of the current `ActiveAbility` entity.
    /// For top-level abilities: the character. For projectile sub-abilities: the projectile entity.
    Caster,
    /// The entity hit by this ability's hitbox. Undefined in `OnCast`/`OnEnd` contexts.
    Victim,
    /// The `original_caster` field — the top-level character that initiated the full chain.
    /// Always the player entity regardless of chain depth.
    OriginalCaster,
}
```

### Triggered sub-abilities

`Melee`, `Projectile`, `AreaOfEffect`, and `Ability` variants all activate another `AbilityDef` by ID, which then runs through its full phase cycle (Startup → Active → Recovery) normally. A max recursion depth (e.g. 4) prevents infinite loops.

Parent `OnHit` effects **always fire** when a hitbox connects, regardless of whether the spawning effect has an `id`. If `id` is present on `Melee`/`AreaOfEffect`, the named sub-ability also activates on that contact — both run additively.

`Projectile` is a first-class ability host: the projectile entity is the `caster` of the sub-ability. `EffectTarget::Caster` resolves to the projectile entity; `EffectTarget::OriginalCaster` resolves to the original shooter. `original_caster` is propagated unchanged through every level of the chain.

## AbilityEffect Variants

### Existing (refactored into new model)

| Variant | Trigger | Description |
|---------|---------|-------------|
| `Melee { id, target }` | `OnCast` | Spawns a hitbox in front of caster. Parent `OnHit` effects always fire on contact. If `id` is `Some`, also activates that sub-ability on contact (additive). |
| `Projectile { id, speed, lifetime_ticks }` | `OnCast` | Spawns a projectile that hosts ability `id`; projectile is the caster, hit entity is the victim. |
| `SetVelocity { speed, target }` | `WhileActive` | Sets velocity each tick. `Caster` uses facing direction; `Victim` uses away-from-caster direction. |

### New primitives

| Variant | Typical trigger | Description |
|---------|----------------|-------------|
| `Damage { amount, target }` | `OnHit` | Reduces `target`'s health. Intercepted by `ActiveShield` before reaching `Health`. |
| `ApplyForce { force, target }` | `OnHit` | Applies an impulse to `target` along the caster→target axis. Positive = away (knockback), negative = toward (pull). |
| `AreaOfEffect { id, target, radius }` | `OnCast` | Spawns a hitbox sphere around caster position. Parent `OnHit` effects always fire on each contact. If `id` is `Some`, also activates that sub-ability per contact (additive). |
| `Grab` | `OnHit` | Swaps victim to `RigidBody::Kinematic` and drives their `Position` to trail the caster each tick. On grab release (ability ends or victim dies), restores `RigidBody::Dynamic`. Throw is a separate ability that reads `GrabbedBy` to resolve its victim. |
| `Buff { stat, multiplier, duration_ticks, target }` | `OnCast` | Temporary stat modifier on target. Enables support moves. |
| `Shield { absorb }` | `OnCast` | Damage absorption during active phase. Defensive counterplay. |
| `Teleport { distance }` | `OnCast` | Instant reposition in facing direction (no collision during transit). |
| `Summon { entity_type, lifetime_ticks }` | `OnCast` | Spawns a persistent entity (turret, trap, decoy). Requires entity behavior definitions — implement last. |
| `Ability { id, target }` | any | Activates ability `id` on `target` (full phase cycle). Max depth: 4. |

### Full enum

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum AbilityEffect {
    Melee { id: Option<String>, target: EffectTarget },
    Projectile { id: String, speed: f32, lifetime_ticks: u16 },
    SetVelocity { speed: f32, target: EffectTarget },
    Damage { amount: f32, target: EffectTarget },
    ApplyForce { force: f32, target: EffectTarget },
    AreaOfEffect { id: Option<String>, target: EffectTarget, radius: f32 },
    Grab,
    Buff { stat: String, multiplier: f32, duration_ticks: u16, target: EffectTarget },
    Shield { absorb: f32 },
    Teleport { distance: f32 },
    Summon { entity_type: String, lifetime_ticks: u16 },
    Ability { id: String, target: EffectTarget },
}
```

## RON Examples

### Current punch (migrated)

Three-step combo via `OnInput`. Each step is a separate ability; `OnInput` fires during the Active window when the player presses the same slot button again. The caster entity's `ActionState<PlayerActions>` is queried for `just_pressed`.

```ron
"punch": (
    startup_ticks: 4,
    active_ticks: 20,    // active window covers both the hitbox window and the combo-continue window
    recovery_ticks: 0,
    cooldown_ticks: 16,
    effects: [
        OnCast(Melee()),
        OnHit(Damage(amount: 5.0, target: Victim)),
        OnHit(ApplyForce(force: 3.0, target: Victim)),
        OnInput(action: Ability1, effect: Ability(id: "punch2", target: Caster)),
    ],
),
"punch2": (
    startup_ticks: 4,
    active_ticks: 20,
    recovery_ticks: 0,
    cooldown_ticks: 0,
    effects: [
        OnCast(Melee()),
        OnHit(Damage(amount: 6.0, target: Victim)),
        OnHit(ApplyForce(force: 3.5, target: Victim)),
        OnInput(action: Ability1, effect: Ability(id: "punch3", target: Caster)),
    ],
),
"punch3": (
    startup_ticks: 4,
    active_ticks: 6,
    recovery_ticks: 10,
    cooldown_ticks: 0,
    effects: [
        OnCast(Melee()),
        OnHit(Damage(amount: 10.0, target: Victim)),
        OnHit(ApplyForce(force: 8.0, target: Victim)),
    ],
),
```

### Dash with shield
```ron
"dash_shield": (
    startup_ticks: 2,
    active_ticks: 8,
    recovery_ticks: 4,
    cooldown_ticks: 32,
    effects: [
        WhileActive(SetVelocity(speed: 15.0, target: Caster)),
        OnCast(Shield(absorb: 20.0)),
    ],
),
```

### Flame punch (melee that triggers fireball on hit)
```ron
"flame_punch": (
    startup_ticks: 4,
    active_ticks: 3,
    recovery_ticks: 8,
    cooldown_ticks: 48,
    effects: [
        OnCast(Melee()),
        OnHit(Ability(id: "fireball", target: Victim)),
    ],
),
```

### Ground pound (AoE + knockback)
```ron
"ground_pound": (
    startup_ticks: 8,
    active_ticks: 2,
    recovery_ticks: 12,
    cooldown_ticks: 64,
    effects: [
        OnCast(AreaOfEffect(id: "ground_pound_hit", radius: 3.0)),
        OnHit(ApplyForce(force: 15.0, target: Victim)),
    ],
),
```

### Grab + throw combo

Grab and throw are separate abilities. `Grab` installs `GrabbedBy(caster)` on the victim; `throw` queries for that component to resolve its victim. The player presses the throw slot button separately — no `OnInput` needed on `grab` itself since throw is an independent ability activation.

```ron
"grab": (
    startup_ticks: 3,
    active_ticks: 4,
    recovery_ticks: 6,
    cooldown_ticks: 48,
    effects: [
        OnCast(Melee()),
        OnHit(Grab),
    ],
),
"throw": (
    startup_ticks: 2,
    active_ticks: 1,
    recovery_ticks: 10,
    cooldown_ticks: 16,
    effects: [
        OnCast(ApplyForce(force: 20.0, target: Victim)),
    ],
),
```

`throw`'s `apply_on_cast_effects` resolves `Victim` by querying for `GrabbedBy(caster_entity)` on any character entity. If no entity is grabbed, the ability is a no-op.

## System Changes

### Activation gating

Ability activation is gated **solely on `AbilityCooldowns`** — no marker component prevents simultaneous activations. Multiple `ActiveAbility` entities for the same caster may coexist (e.g. a dash with an active shield).

Movement is not globally suppressed during casting. Abilities that restrict caster movement use `WhileActive(SetVelocity { speed: 0.0, target: Caster })` to override movement input each tick.

### Trigger dispatch — marker component architecture

`dispatch_effect_markers` partitions `Vec<EffectTrigger>` into **per-trigger-type marker components** on the `ActiveAbility` entity each tick. Downstream effect systems query only their own marker type, with no knowledge of `EffectTrigger` — preserving the same parallelism guarantee as the current approach.

#### Marker components

```rust
/// One-shot: inserted on the first Active tick; consumed (removed) by apply_on_cast_effects.
#[derive(Component)]
struct OnCastEffects(Vec<AbilityEffect>);

/// Persistent: present every Active tick; removed when phase exits Active.
#[derive(Component)]
struct WhileActiveEffects(Vec<AbilityEffect>);

/// Persistent: present every Active tick; removed when phase exits Active.
/// Each entry is (action, effect); system checks just_pressed on active.caster.
#[derive(Component)]
struct OnInputEffects(Vec<(PlayerActions, AbilityEffect)>);

/// One-shot: inserted on the tick Active → Recovery; consumed by apply_on_end_effects.
#[derive(Component)]
struct OnEndEffects(Vec<AbilityEffect>);

/// Carried on the spawned hitbox/AoE entity (not the ActiveAbility entity).
/// Hit detection reads this to know what to fire on contact.
#[derive(Component)]
struct OnHitEffects {
    effects: Vec<AbilityEffect>,
    caster: Entity,
    original_caster: Entity,
    depth: u8,
}
```

`dispatch_effect_markers` runs each tick and inserts/removes these components based on phase transitions. Effect systems are decoupled from `EffectTrigger` entirely:

| System | Marker queried | Parallelism note |
|--------|---------------|-----------------|
| `apply_on_cast_effects` | `&OnCastEffects` (removes after processing) | Spawns entities via `Commands`; no character component writes |
| `apply_while_active_effects` | `&WhileActiveEffects` | Writes `LinearVelocity` on caster; conflicts with physics, not with other effect systems |
| `apply_on_input_effects` | `&OnInputEffects` + caster `&ActionState` | Read-only on caster; spawns via `Commands` |
| `apply_on_hit_effects` | `&OnHitEffects` on hitbox entity | Writes `Health`, `LinearVelocity` on targets; runs from hit detection |
| `apply_on_end_effects` | `&OnEndEffects` (removes after processing) | Same access pattern as `apply_on_cast_effects` |

`apply_on_cast_effects` and `apply_on_end_effects` can run in parallel (disjoint markers, both only spawn via `Commands`). `apply_while_active_effects` and `apply_on_input_effects` both run during Active ticks and can also run in parallel — `WhileActive` writes velocity, `OnInput` only reads input and defers spawns.

`apply_on_hit_effects` runs synchronously from within hit detection since it needs the hit contact information (attacker position, victim entity) available at that point.

### Ability activation (for `Ability`, `Melee`, `Projectile`, `AreaOfEffect` variants)

Triggered abilities are spawned as independent entities rather than inserted onto the caster or target (which may already have an `ActiveAbility`). Each activation spawns a new entity carrying:

```rust
struct ActiveAbility {
    def_id: AbilityId,
    caster: Entity,          // immediate caster (may be a projectile or bomb entity)
    original_caster: Entity, // top-level initiator; propagated unchanged through every chain level
    target: Entity,          // may equal caster for self-targeted effects
    phase: AbilityPhase,
    phase_start_tick: Tick,
    depth: u8,               // incremented per trigger chain; capped at 4
}
```

Effect systems query these entities directly, resolving `caster`, `original_caster`, and `target` from the component rather than from the entity identity.

### New components needed

| Component | Purpose |
|-----------|---------|
| `ActiveBuff { stat, multiplier, expires_tick }` | Tracks temporary stat modifiers |
| `ActiveShield { remaining }` | Tracks damage absorption |
| `GrabbedBy(Entity)` | Inserted on grabbed victim. Triggers swap to `RigidBody::Kinematic`; removed on grab release, restoring `RigidBody::Dynamic`. Grab system drives victim `Position` to trail caster each tick while present. |
| `Grabbing(Entity)` | Mirrors `GrabbedBy` on the grabber for bidirectional lookup |

## Implementation Order

1. Refactor `effect` → `effects: Vec<EffectTrigger>` and migrate existing abilities; remove `steps` / `step_window_ticks` / `step` / `total_steps` / `chain_input_received` from all structs and systems
2. Update trigger dispatch systems; add `apply_on_input_effects`
3. `ApplyForce` (impulse-based, covers knockback and pull via sign)
4. `OnInput` (query caster `ActionState`; fire inner effect — depends on `Ability { id, target }` from step 8, but can be stubbed with a simple `Ability` dispatch first)
5. `AreaOfEffect` (hitbox spawning variant of Melee)
6. `Buff` / `Shield` (new components + tick-based expiry)
7. `Teleport` (position set, no physics sweep)
8. `Grab` (position lock + combo interaction)
9. `Ability { id, target }` (recursive resolution)
10. `Summon` (deferred — needs entity behavior system)
