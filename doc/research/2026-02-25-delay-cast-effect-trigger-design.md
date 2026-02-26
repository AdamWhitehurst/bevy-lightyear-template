---
date: 2026-02-25T18:06:52-0800
researcher: Claude Opus 4.6
git_commit: 6ea3cb67c02cc89aece911924ed7f8bea1427f72
branch: master
repository: bevy-lightyear-template
topic: "How to implement delayed/sequenced effect firing within an ability's Active phase"
tags: [research, codebase, ability, effects, EffectTrigger, OnCast, OnTick, delay, blink_strike, dispatch]
status: complete
last_updated: 2026-02-25
last_updated_by: Claude Opus 4.6
last_updated_note: "Decision: Remove OnCast, replace with OnTick (tick defaults to 0 via serde(default))"
---

# Research: Delayed Effect Firing Within an Ability's Active Phase

**Date**: 2026-02-25T18:06:52-0800
**Researcher**: Claude Opus 4.6
**Git Commit**: 6ea3cb67c02cc89aece911924ed7f8bea1427f72
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to implement `EffectTrigger::DelayCast(AbilityEffect)` (or equivalent) so that abilities can sequence effects across different ticks within the Active phase. Motivating example: `blink_strike` should fire `Teleport` on Active tick 0 and `Melee` on Active tick 1.

## Summary

Currently all `OnCast` effects fire simultaneously on the first Active tick. The codebase already has all the machinery to compute tick offsets within a phase (`tick - phase_start_tick`). **Decision: Remove `OnCast` entirely and replace with `OnTick { tick: u16, effect: AbilityEffect }`.** The `tick` field defaults to 0 via `#[serde(default)]`, so `OnTick(effect: Melee())` is equivalent to the old `OnCast(Melee())` with no ergonomic loss. This subsumes `OnCast` while adding tick-offset sequencing.

---

## Current Behavior: Why All OnCast Effects Fire Simultaneously

### The dispatch chain

System execution order ([lib.rs:250-264](crates/protocol/src/lib.rs#L250-L264)), all `.chain()`ed in `FixedUpdate`:

```
ability_activation → update_active_abilities → dispatch_effect_markers →
apply_on_cast_effects → apply_while_active_effects → apply_on_end_effects →
apply_on_input_effects → ability_projectile_spawn
```

### First Active tick detection

In `dispatch_active_phase_markers` ([ability.rs:585](crates/protocol/src/ability.rs#L585)):

```rust
let first_active_tick = active.phase_start_tick == tick;
```

`phase_start_tick` is set to the current tick when `update_active_abilities` transitions Startup → Active ([ability.rs:516-517](crates/protocol/src/ability.rs#L516-L517)). Since `dispatch_effect_markers` runs immediately after in the chain, it sees `phase_start_tick == tick` as `true` on exactly one tick.

### All OnCast effects are collected into one marker

On that first Active tick, **all** `EffectTrigger::OnCast(e)` effects are collected into a single `OnCastEffects(Vec<AbilityEffect>)` component ([ability.rs:587-598](crates/protocol/src/ability.rs#L587-L598)). `apply_on_cast_effects` then iterates the vec and processes every effect in definition order within one system invocation, then removes the component ([ability.rs:845](crates/protocol/src/ability.rs#L845)).

### blink_strike concrete timing

For `blink_strike` (`startup_ticks: 3, active_ticks: 4`):

| Tick | Event |
|------|-------|
| T+0 | Spawn `ActiveAbility(phase=Startup, phase_start_tick=T)` |
| T+1, T+2 | Still Startup (elapsed < 3) |
| T+3 | Transition to Active. `dispatch_effect_markers` inserts `OnCastEffects([Teleport, Melee])`. `apply_on_cast_effects` processes both: Teleport mutates Position, then Melee spawns hitbox at the new position. Both fire on the **same tick**. |
| T+4-T+6 | Remaining Active ticks (WhileActive/OnInput dispatched, OnCast never re-inserted) |
| T+7 | Transition to Recovery |

Within `apply_on_cast_effects`, the Teleport executes first (vec order matches RON order), mutating the caster's `Position` via the shared `&mut Query<(&mut Position, &Rotation)>`. The Melee hitbox spawn then reads the post-teleport position. So the hitbox **does** appear at the teleported location -- but both happen on the same tick with no frame separation.

---

## Existing Tick-Offset Patterns in the Codebase

The codebase already computes tick offsets in several places:

| Location | Pattern | Expression |
|----------|---------|------------|
| [ability.rs:507](crates/protocol/src/ability.rs#L507) | Phase elapsed | `tick - active.phase_start_tick` as `i16` |
| [ability.rs:585](crates/protocol/src/ability.rs#L585) | First tick detect | `active.phase_start_tick == tick` (exact match) |
| [ability.rs:571](crates/protocol/src/ability.rs#L571) | First Recovery tick | `active.phase == Recovery && active.phase_start_tick == tick` |
| [ability.rs:252-254](crates/protocol/src/ability.rs#L252-L254) | Cooldown check | `(current_tick - last).unsigned_abs() <= cooldown_ticks` |
| [ability.rs:1252](crates/protocol/src/ability.rs#L1252) | AoE lifetime | `tick - aoe.spawn_tick >= duration_ticks as i16` |
| [ability.rs:1269](crates/protocol/src/ability.rs#L1269) | Bullet lifetime | `tick - spawn_info.spawn_tick >= lifetime_ticks as i16` |
| [ability.rs:1118](crates/protocol/src/ability.rs#L1118) | Buff expiry | `b.expires_tick - tick > 0` (absolute target tick) |

The expression `(tick - active.phase_start_tick)` gives the zero-indexed tick offset within the current phase. This is already computed in `advance_ability_phase` but not exposed to `dispatch_active_phase_markers`.

No existing pattern dispatches different effects on different ticks within the same phase.

---

## Design Options

### Option A (chosen): `OnTick` replaces `OnCast`

Remove `OnCast` entirely. `OnTick` with `#[serde(default)]` on `tick` covers both cases:

```rust
enum EffectTrigger {
    /// Fires once on the specified Active-phase tick offset. Defaults to tick 0 (first Active tick).
    OnTick {
        #[serde(default)]
        tick: u16,
        effect: AbilityEffect,
    },
    WhileActive(AbilityEffect),
    OnHit(AbilityEffect),
    OnEnd(AbilityEffect),
    OnInput { action: PlayerActions, effect: AbilityEffect },
}
```

RON examples:
```ron
// tick defaults to 0 — equivalent to old OnCast:
OnTick(effect: Teleport(distance: 6.0)),

// explicit tick offset:
OnTick(tick: 1, effect: Melee()),
```

RON for blink_strike:
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

**Implementation**: In `dispatch_active_phase_markers`, compute `let active_offset = (tick - active.phase_start_tick) as u16;`. Collect `OnTick` effects where `trigger.tick == active_offset`. Insert them as `OnTickEffects` (renamed from `OnCastEffects`). `apply_on_tick_effects` (renamed from `apply_on_cast_effects`) processes them identically. `OnTickEffects` can be inserted on any Active tick via exact tick match.

**Why this wins**:
- No redundancy — one trigger for all Active-phase one-shot effects
- `#[serde(default)]` means omitting `tick` defaults to 0, so common case is just as clean as old `OnCast`
- The dispatch check is `==` (exact tick match), inherently rollback-safe
- Reuses existing marker/apply pattern entirely
- All existing `OnCast(X)` in RON become `OnTick(effect: X)` — mechanical find-replace

### Option B (rejected): `delay_ticks` Field on `OnCast`

Change `OnCast` from tuple variant to struct variant:

```rust
OnCast { effect: AbilityEffect, #[serde(default)] delay_ticks: u16 }
```

RON:
```ron
OnCast(effect: Teleport(distance: 6.0)),
OnCast(effect: Melee(), delay_ticks: 1),
```

**Pros**: No new trigger variant.

**Cons**:
- Every existing `OnCast(Melee())` in RON becomes `OnCast(effect: Melee())` -- pervasive file change
- Turns the one-shot dispatch into a multi-tick retention problem: `OnCastEffects` must persist across ticks, tracking which effects have fired
- Or: split into separate markers per delay tick, adding complexity to dispatch
- Semantically awkward: "OnCast" with a delay of 5 ticks is no longer "on cast"

### Option C: `delay_ticks` Field on Every `EffectTrigger`

Add delay to the trigger wrapper:

```rust
struct TimedEffect { delay_ticks: u16, effect: AbilityEffect }
enum EffectTrigger {
    OnCast(TimedEffect),
    WhileActive(TimedEffect),  // delay on WhileActive is odd
    OnHit(TimedEffect),        // delay on OnHit is odd
    ...
}
```

**Pros**: Universal -- any trigger can be delayed.

**Cons**: Delay is meaningless for most triggers (`WhileActive` fires every tick, `OnHit` is event-driven). Adds noise to every trigger definition.

### Option D: Full Timeline/Keyframe System

Replace triggers with frame-keyed event lists:

```ron
timeline: [
    (frame: 0, events: [Teleport(distance: 6.0)]),
    (frame: 1, events: [SpawnHitbox(melee)]),
]
```

**Pros**: Maximum expressiveness, how fighting games work internally.

**Cons**: Replaces the entire trigger/effect model. Overkill for this codebase where most abilities have 1-2 cast effects. Reactive triggers (OnHit, OnInput, WhileActive) don't fit a keyframe model.

---

## Analysis

### `OnTick` replaces `OnCast` — implementation sketch

`dispatch_active_phase_markers` replaces the `first_active_tick`-gated `OnCast` collection with a tick-offset match:

```rust
let active_offset = (tick - active.phase_start_tick) as u16;

let on_tick: Vec<AbilityEffect> = def.effects.iter().filter_map(|t| match t {
    EffectTrigger::OnTick { tick: t, effect } if *t == active_offset => Some(effect.clone()),
    _ => None,
}).collect();

if !on_tick.is_empty() {
    commands.entity(entity).insert(OnTickEffects(on_tick));
}
```

`OnHitEffects` insertion stays on the first Active tick (`active_offset == 0`) since hit effects are a property of the ability, not tied to a specific `OnTick`.

**Rollback safety**: Exact tick match (`==`) fires on exactly one tick per simulation. No "fired" flags needed. `phase_start_tick` is predicted and restored correctly.

**Prespawn salt**: Hitboxes use `DisableRollback` so no salt issue. For `OnTick` spawning a Projectile, the salt at [ability.rs:1176](crates/protocol/src/ability.rs#L1176) should incorporate the tick offset.

**Sub-ability latency**: `OnTick(tick: 1, effect: Ability(id: "x"))` spawns sub-ability `"x"` on Active tick 1. The sub-ability then needs at least 1 more tick for `update_active_abilities` to advance it. So sub-ability effects fire at earliest on tick 2 (if `startup_ticks: 0`). For same-tick sequencing, use `OnTick` directly on the parent ability.

---

## Code References

- [crates/protocol/src/ability.rs:140-156](crates/protocol/src/ability.rs#L140-L156) -- `EffectTrigger` enum definition
- [crates/protocol/src/ability.rs:578-643](crates/protocol/src/ability.rs#L578-L643) -- `dispatch_active_phase_markers` (where OnTick dispatch would be added)
- [crates/protocol/src/ability.rs:585](crates/protocol/src/ability.rs#L585) -- `first_active_tick` computation
- [crates/protocol/src/ability.rs:500-527](crates/protocol/src/ability.rs#L500-L527) -- `advance_ability_phase` (tick offset computation)
- [crates/protocol/src/ability.rs:749-846](crates/protocol/src/ability.rs#L749-L846) -- `apply_on_cast_effects` (reused for OnTick effects)
- [assets/abilities.ron:72-83](assets/abilities.ron#L72-L83) -- `blink_strike` definition
- [crates/protocol/src/lib.rs:250-264](crates/protocol/src/lib.rs#L250-L264) -- system chain ordering

## Related Research

- [doc/research/2026-02-22-remaining-ability-effect-primitives.md](doc/research/2026-02-22-remaining-ability-effect-primitives.md) -- full remaining work inventory
- [doc/design/2026-02-13-ability-effect-primitives.md](doc/design/2026-02-13-ability-effect-primitives.md) -- design vision
- [doc/plans/2026-02-22-ability-effect-primitives-phases-2-7.md](doc/plans/2026-02-22-ability-effect-primitives-phases-2-7.md) -- implementation plan (phases 2-7)

## Resolved Questions

1. **`OnCast` is removed.** `OnTick` with `#[serde(default)] tick: u16` replaces it. `OnTick(effect: X)` (tick defaults to 0) is the new way to express "fire on first Active tick." No activation-time trigger exists yet; if needed in the future (e.g., shield during startup windup), it would be a new trigger variant.

2. **No validation for now.** `OnTick.tick >= active_ticks` (unreachable effects) should eventually be a load-time warning, but skip for initial implementation. TODO.

3. **Per-OnTick OnHit effects via sub-abilities.** If different `OnTick` effects need different `OnHitEffects`, use `OnTick(tick: 1, effect: Ability(id: "blink_melee"))`. The sub-ability carries its own `OnHit` effects. Note: sub-abilities add at least 1 tick of latency (need one tick for `update_active_abilities` to advance them).

4. **RON migration**: All `OnCast(X)` become `OnTick(effect: X)`. Mechanical find-replace across `abilities.ron` and test files.

5. **Sub-ability latency is an architectural constraint.** `Ability { id, target }` spawns an entity via `Commands`; the processing systems (`update_active_abilities → dispatch_effect_markers → apply_on_tick_effects`) have already run this tick. The sub-ability won't be processed until the next tick — minimum 1-tick latency (~15.6ms at 64Hz, sub-frame at 60fps). This cannot be eliminated without breaking the ECS pattern. **Guidance: use `OnTick` directly on the parent ability for same-tick sequencing. Reserve sub-abilities for when you need independent phase cycles or different `OnHit` effects.**
