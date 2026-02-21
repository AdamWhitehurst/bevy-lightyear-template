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
    pub steps: u8,
    pub step_window_ticks: u16,
    // BEFORE: pub effect: AbilityEffect,
    pub effects: Vec<EffectTrigger>,
}
```

### EffectTarget enum

Specifies who receives the effect, relevant for `OnHit` and `Ability` references.

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum EffectTarget {
    Caster,
    Victim,
}
```

### Triggered sub-abilities

`Melee`, `Projectile`, `AreaOfEffect`, and `Ability` variants all activate another `AbilityDef` by ID, which then runs through its full phase cycle (Startup → Active → Recovery) normally. A max recursion depth (e.g. 4) prevents infinite loops.

The `id` on `Melee`/`Projectile`/`AreaOfEffect` is the **exclusive** hit handler — parent `OnHit` effects do not fire for contacts made by child hitboxes or projectiles. Only the sub-ability specified by `id` runs on contact.

`Projectile` is a first-class ability host: the projectile entity itself is the `caster` of the sub-ability. `EffectTarget::Caster` resolves to the original shooter; `EffectTarget::Victim` resolves to the hit entity.

## AbilityEffect Variants

### Existing (refactored into new model)

| Variant | Trigger | Description |
|---------|---------|-------------|
| `Melee { id, target }` | `OnCast` | Spawns a hitbox in front of caster; on contact, activates ability `id` on `target` (full phase cycle) |
| `Projectile { id, speed, lifetime_ticks }` | `OnCast` | Spawns a projectile that hosts ability `id`; projectile is the caster, hit entity is the victim |
| `SetVelocity { speed, target }` | `WhileActive` | Sets velocity each tick. `Caster` uses facing direction; `Victim` uses away-from-caster direction. |

### New primitives

| Variant | Typical trigger | Description |
|---------|----------------|-------------|
| `ApplyForce { force, target }` | `OnHit` | Applies an impulse to `target` along the caster→target axis. Positive = away (knockback), negative = toward (pull). |
| `AreaOfEffect { id, target, radius }` | `OnCast` | Spawns a hitbox sphere around caster position; on each contact, activates ability `id` on `target` (full phase cycle). Ground pounds, shockwaves. |
| `Grab` | `OnHit` | Locks victim position to caster. Next combo step acts as throw. |
| `Buff { stat, multiplier, duration_ticks }` | `OnCast` | Temporary stat modifier on target. Enables support moves. |
| `Shield { absorb }` | `OnCast` | Damage absorption during active phase. Defensive counterplay. |
| `Teleport { distance }` | `OnCast` | Instant reposition in facing direction (no collision during transit). |
| `Summon { entity_type, lifetime_ticks }` | `OnCast` | Spawns a persistent entity (turret, trap, decoy). Requires entity behavior definitions — implement last. |
| `Ability { id, target }` | any | Activates ability `id` on `target` (full phase cycle). Max depth: 4. |

### Full enum

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum AbilityEffect {
    Melee { id: String, target: EffectTarget },
    Projectile { id: String, speed: f32, lifetime_ticks: u16 },
    SetVelocity { speed: f32, target: EffectTarget },
    ApplyForce { force: f32, target: EffectTarget },
    AreaOfEffect { id: String, target: EffectTarget, radius: f32 },
    Grab,
    Buff { stat: String, multiplier: f32, duration_ticks: u16 },
    Shield { absorb: f32 },
    Teleport { distance: f32 },
    Summon { entity_type: String, lifetime_ticks: u16 },
    Ability { id: String, target: EffectTarget },
}
```

## RON Examples

### Current punch (migrated)
```ron
"punch": (
    startup_ticks: 4,
    active_ticks: 3,
    recovery_ticks: 6,
    cooldown_ticks: 16,
    steps: 3,
    step_window_ticks: 20,
    effects: [
        OnCast(Melee),
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
    steps: 1,
    step_window_ticks: 0,
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
    steps: 1,
    step_window_ticks: 0,
    effects: [
        OnCast(Melee),
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
    steps: 1,
    step_window_ticks: 0,
    effects: [
        OnCast(AreaOfEffect(id: "ground_pound_hit", radius: 3.0)),
        OnHit(ApplyForce(force: 15.0, target: Victim)),
    ],
),
```

### Grab + throw combo
```ron
"grab": (
    startup_ticks: 3,
    active_ticks: 4,
    recovery_ticks: 6,
    cooldown_ticks: 48,
    steps: 2,
    step_window_ticks: 30,
    effects: [
        OnHit(Grab),
        // Step 2 effects resolve as throw based on Grab state
        OnCast(ApplyForce(force: 20.0, target: Victim)),
    ],
),
```

## System Changes

### Trigger dispatch

Current systems (`ability_projectile_spawn`, `ability_dash_effect`) check the single `effect` field. Refactor to iterate `effects` and dispatch by trigger type:

- **`apply_on_cast_effects`** — runs once when `phase` becomes `Active` (`phase_start_tick == tick`). Handles: `Melee`, `Projectile`, `AreaOfEffect`, `Buff`, `Shield`, `Teleport`, `Summon`, nested `Ability`.
- **`apply_while_active_effects`** — runs every tick where `phase == Active`. Handles: `SetVelocity`.
- **`apply_on_hit_effects`** — called from hit detection (not phase system). Handles: `ApplyForce`, `Grab`, nested `Ability`.
- **`apply_on_end_effects`** — runs once when `phase` transitions from `Active` to `Recovery`. Handles same effect set as `apply_on_cast_effects`.

### Ability activation (for `Ability`, `Melee`, `Projectile`, `AreaOfEffect` variants)

Triggered abilities are spawned as independent entities rather than inserted onto the caster or target (which may already have an `ActiveAbility`). Each activation spawns a new entity carrying:

```rust
struct ActiveAbility {
    def_id: AbilityId,
    caster: Entity,
    target: Entity,  // may equal caster for self-targeted effects
    phase: AbilityPhase,
    phase_start_tick: Tick,
    depth: u8,       // incremented per trigger chain; capped at 4
}
```

Effect systems query these entities directly, resolving `caster` and `target` from the component rather than from the entity identity.

### New components needed

| Component | Purpose |
|-----------|---------|
| `ActiveBuff { stat, multiplier, expires_tick }` | Tracks temporary stat modifiers |
| `ActiveShield { remaining }` | Tracks damage absorption |
| `GrabbedBy(Entity)` | Marks a grabbed victim, locks position |
| `Grabbing(Entity)` | Marks the grabber |

## Implementation Order

1. Refactor `effect` → `effects: Vec<EffectTrigger>` and migrate existing abilities
2. Update trigger dispatch systems
3. `ApplyForce` (impulse-based, covers knockback and pull via sign)
4. `AreaOfEffect` (hitbox spawning variant of Melee)
5. `Buff` / `Shield` (new components + tick-based expiry)
6. `Teleport` (position set, no physics sweep)
7. `Grab` (position lock + combo interaction)
8. `Ability { id, target }` (recursive resolution)
9. `Summon` (deferred — needs entity behavior system)
