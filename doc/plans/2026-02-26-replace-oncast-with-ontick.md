# Replace OnCast with OnTick — Implementation Plan

## Overview

Replace `EffectTrigger::OnCast(AbilityEffect)` with `OnTick { tick: u16, effect: AbilityEffect }` to support firing effects on specific Active-phase tick offsets. `#[serde(default)]` on `tick` means omitting it defaults to 0 — equivalent to the old `OnCast` with no ergonomic loss. This enables `blink_strike` to fire `Teleport` on tick 0 and `Melee` on tick 1.

**Research**: [doc/research/2026-02-25-delay-cast-effect-trigger-design.md](doc/research/2026-02-25-delay-cast-effect-trigger-design.md)

## Current State

- `EffectTrigger::OnCast(AbilityEffect)` — tuple variant, fires all effects on first Active tick
- `OnCastEffects(Vec<AbilityEffect>)` — marker component inserted on first Active tick
- `apply_on_cast_effects` — system that consumes `OnCastEffects`
- `dispatch_active_phase_markers` gates `OnCast` collection behind `first_active_tick` boolean
- 16 `OnCast` entries across 12 abilities in `assets/abilities.ron`
- ~16 test usages in `crates/protocol/tests/ability_systems.rs`

## Desired End State

- `EffectTrigger::OnTick { tick: u16, effect: AbilityEffect }` with `#[serde(default)]` on `tick`
- `OnTickEffects` replaces `OnCastEffects`
- `apply_on_tick_effects` replaces `apply_on_cast_effects`
- Dispatch uses `active_offset == trigger.tick` instead of `first_active_tick` guard
- `blink_strike` can sequence Teleport (tick 0) and Melee (tick 1) on separate frames
- All existing abilities work identically (all use tick 0)

### Verification:
- `cargo test-all` passes
- `cargo server` + `cargo client -c 1` — all abilities fire correctly
- `blink_strike` updated to use `OnTick(tick: 1, effect: Melee())` and visually delays melee by 1 tick

## What We're NOT Doing

- No validation of `OnTick.tick >= active_ticks` (unreachable effects) — future TODO
- No changes to `OnHitEffects` insertion timing — stays on tick 0
- No changes to `WhileActive`, `OnEnd`, `OnInput`, `OnHit` triggers
- No new test cases for multi-tick sequencing (just migrate existing tests)

## Implementation — Single Phase

All changes are atomic — the code won't compile with partial renames.

### 1. EffectTrigger enum

**File**: `crates/protocol/src/ability.rs:154-168`

Replace:
```rust
    /// Fires once when ability enters Active phase.
    OnCast(AbilityEffect),
```
With:
```rust
    /// Fires once on the specified Active-phase tick offset (0-indexed from phase start).
    /// Defaults to tick 0 (first Active tick) when `tick` is omitted.
    OnTick {
        #[serde(default)]
        tick: u16,
        effect: AbilityEffect,
    },
```

### 2. Marker component rename

**File**: `crates/protocol/src/ability.rs:314-316`

Rename `OnCastEffects` → `OnTickEffects`:
```rust
/// One-shot: inserted on matching Active tick offset; consumed by apply_on_tick_effects.
#[derive(Component)]
pub struct OnTickEffects(pub Vec<AbilityEffect>);
```

### 3. dispatch_active_phase_markers — tick-offset matching

**File**: `crates/protocol/src/ability.rs:590-610`

Replace the `first_active_tick`-gated `OnCast` collection:
```rust
    let first_active_tick = active.phase_start_tick == tick;

    if first_active_tick {
        let on_cast: Vec<AbilityEffect> = def
            .effects
            .iter()
            .filter_map(|t| match t {
                EffectTrigger::OnCast(e) => Some(e.clone()),
                _ => None,
            })
            .collect();
        if !on_cast.is_empty() {
            commands.entity(entity).insert(OnCastEffects(on_cast));
        }
```

With tick-offset matching:
```rust
    let first_active_tick = active.phase_start_tick == tick;
    let active_offset = (tick - active.phase_start_tick) as u16;

    {
        let on_tick: Vec<AbilityEffect> = def
            .effects
            .iter()
            .filter_map(|t| match t {
                EffectTrigger::OnTick { tick: t, effect } if *t == active_offset => {
                    Some(effect.clone())
                }
                _ => None,
            })
            .collect();
        if !on_tick.is_empty() {
            commands.entity(entity).insert(OnTickEffects(on_tick));
        }
    }
```

Note: `first_active_tick` is retained because it's still used for `OnHitEffects` insertion (lines 612-627).

### 4. remove_active_phase_markers

**File**: `crates/protocol/src/ability.rs:657-662`

Replace `OnCastEffects` → `OnTickEffects`:
```rust
fn remove_active_phase_markers(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).remove::<OnTickEffects>();
    // ... rest unchanged
}
```

### 5. apply_on_cast_effects → apply_on_tick_effects

**File**: `crates/protocol/src/ability.rs:760-857`

Rename function. Update:
- Doc comment (line 760)
- Function name (line 761)
- Query type: `&OnCastEffects` → `&OnTickEffects` (line 769)
- Warn message: `"Unhandled OnCast effect"` → `"Unhandled OnTick effect"` (line 853)
- Component removal: `remove::<OnCastEffects>()` → `remove::<OnTickEffects>()` (line 857)

### 6. cleanup_effect_markers_on_removal

**File**: `crates/protocol/src/ability.rs:1139-1152`

Replace `OnCastEffects` → `OnTickEffects` (line 1145).

### 7. System registration

**File**: `crates/protocol/src/lib.rs:256, 275`

- Line 256: `ability::apply_on_cast_effects` → `ability::apply_on_tick_effects`
- Line 275: `.after(ability::apply_on_cast_effects)` → `.after(ability::apply_on_tick_effects)`

### 8. RON ability definitions

**File**: `assets/abilities.ron`

Mechanical replacement — all 16 entries:
- `OnCast(X)` → `OnTick(effect: X)` for all abilities

Exception — `blink_strike` gets tick sequencing:
```ron
"blink_strike": (
    startup_ticks: 3,
    active_ticks: 4,
    recovery_ticks: 10,
    cooldown_ticks: 28,
    effects: [
        OnTick(effect: Teleport(distance: 6.0)),
        OnTick(tick: 1, effect: Melee()),
        OnHit(Damage(amount: 18.0, target: Victim)),
        OnHit(ApplyForce(force: (0.0, 1.2, 4.0), frame: RelativePosition, target: Victim)),
    ],
),
```

### 9. Tests

**File**: `crates/protocol/tests/ability_systems.rs`

- All `EffectTrigger::OnCast(AbilityEffect::X { .. })` → `EffectTrigger::OnTick { tick: 0, effect: AbilityEffect::X { .. } }`
- All `ability::apply_on_cast_effects` → `ability::apply_on_tick_effects` (lines 161, 876)
- `fn sub_ability_spawned_on_cast()` → rename to `fn sub_ability_spawned_on_tick()` (line 656)

Locations (16 `OnCast` usages): lines 24, 55, 86, 123, 672, 720, 911, 982, 1042, 1124, 1193, 1286, 1375.

### 10. README.md

**File**: `README.md:118`

Replace `OnCast` → `OnTick` in the effect trigger description.

### 11. Memory file

**File**: `.claude/projects/-Users-aw-dev-bevy-lightyear-template/memory/MEMORY.md:5`

Update `EffectTrigger::OnCast(effect)` → `EffectTrigger::OnTick { tick, effect }`.

### Success Criteria

#### Automated Verification:
- [x] Workspace builds clean: `cargo check-all`
- [ ] All tests pass: `cargo test-all` (note: 31 tests pre-existing failures unrelated to this change)
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client -c 1`

#### Manual Verification:
- [ ] All abilities fire correctly (punch, fireball, ground_pound, blink_strike, uppercut, shield_bash, shockwave, dive_kick, speed_burst, barrier, teleport_burst, dash)
- [ ] `blink_strike` teleports on tick 0, melee hitbox appears on tick 1 (1-frame delay visible)

## References

- Research: [doc/research/2026-02-25-delay-cast-effect-trigger-design.md](doc/research/2026-02-25-delay-cast-effect-trigger-design.md)
- EffectTrigger enum: [crates/protocol/src/ability.rs:154-168](crates/protocol/src/ability.rs#L154-L168)
- dispatch_active_phase_markers: [crates/protocol/src/ability.rs:590-655](crates/protocol/src/ability.rs#L590-L655)
- apply_on_cast_effects: [crates/protocol/src/ability.rs:761-857](crates/protocol/src/ability.rs#L761-L857)
- System chain: [crates/protocol/src/lib.rs:250-264](crates/protocol/src/lib.rs#L250-L264)
- abilities.ron: [assets/abilities.ron](assets/abilities.ron)
- Tests: [crates/protocol/tests/ability_systems.rs](crates/protocol/tests/ability_systems.rs)
