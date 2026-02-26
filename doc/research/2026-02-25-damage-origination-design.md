---
date: 2026-02-25T13:28:24-08:00
researcher: Claude Sonnet 4.6
git_commit: 0bceede37bc84227c8c92c185d696f3a0ab2709c
branch: master
repository: bevy-lightyear-template
topic: "Damage Tracking and Origination Design for Vision Requirements"
tags: [research, damage, combat, pvp, pve, hit-detection, ability-system, game-design]
status: complete
last_updated: 2026-02-25
last_updated_by: Claude Sonnet 4.6
last_updated_note: "Resolved all open questions with design decisions"
---

# Research: Damage Tracking and Origination Design

**Date**: 2026-02-25T13:28:24-08:00
**Researcher**: Claude Sonnet 4.6
**Git Commit**: 0bceede37bc84227c8c92c185d696f3a0ab2709c
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

Given an overworld with player interaction, future support is needed for:
- Players cannot hurt each other by default (no friendly fire)
- Players can use abilities to destroy world objects
- Players can challenge each other to duels, enabling mutual damage
- Players can harvest world objects (resources + XP from abilities)
- Players gain XP from abilities on world objects and other players

How should damage tracking and origination be designed to support these VISION.md requirements?

## Summary

The current system already tracks origination (`caster` + `original_caster` on `OnHitEffects`) but has no concept of target classification, interaction modes (PvE/PvP), or yield/reward tracking. Hit detection is hardcoded to filter for `CharacterMarker` only — world objects cannot currently receive hits. Damage application has no notion of whether the caster is "allowed" to damage a given target. The codebase provides clean extension points at three levels: the collision layer system, the hit detection filter, and `apply_on_hit_effects`. No systems need to be torn down — they need classification data added to entities and a gating layer inserted into the damage pipeline.

## Detailed Findings

### 1. Current Origination Tracking

**`OnHitEffects`** — [`crates/protocol/src/ability.rs:294-300`](crates/protocol/src/ability.rs#L294)

```rust
pub struct OnHitEffects {
    pub effects: Vec<AbilityEffect>,
    pub caster: Entity,          // proximate caster (the immediate ability entity's caster)
    pub original_caster: Entity, // top-level player who first triggered the chain
    pub depth: u8,
}
```

Both `caster` and `original_caster` are propagated through the entire ability chain — through `spawn_melee_hitbox`, `spawn_aoe_hitbox`, and `handle_ability_projectile_spawn`. By the time `apply_on_hit_effects` fires, `original_caster` always identifies the root player entity, regardless of sub-ability nesting depth.

This means **the attacker entity is always known at hit time**. There is no ambiguity about who dealt damage.

What does NOT exist:
- No persistent kill/damage credit record
- No per-hit event emitted (damage is applied inline in `apply_on_hit_effects`)
- No origination data attached to the `Health` change itself

### 2. Current Hit Detection Filter

**`process_hitbox_hits`** — [`crates/protocol/src/hit_detection.rs:75-128`](crates/protocol/src/hit_detection.rs#L75)
**`process_projectile_hits`** — [`crates/protocol/src/hit_detection.rs:148-198`](crates/protocol/src/hit_detection.rs#L148)

Both systems skip a target if `target_query.get(target).is_err()`, where `target_query` is `With<CharacterMarker>`. This is the **only target classification** in the system. Only `CharacterMarker` entities can receive on-hit effects.

Currently this captures:
- Player characters (spawned in `handle_connected`)
- `DummyTarget` entity (spawned in `spawn_dummy_target` — it also has `CharacterMarker`)

What cannot currently receive hits:
- Terrain (voxel chunks have no `CharacterMarker`)
- Any future world object without `CharacterMarker`

### 3. Current Damage Application Gate

**`apply_on_hit_effects`** — [`crates/protocol/src/hit_detection.rs:249-336`](crates/protocol/src/hit_detection.rs#L249)

The only gate before applying damage is:
1. `ActiveShield` check (absorb before health)
2. `Option<&Invulnerable>` check (skip if invulnerable)

There is no check for:
- Whether the caster and victim are on the same "team"
- Whether the victim has opted into PvP (duel mode)
- Whether the caster has permission to damage this entity type

### 4. Collision Layers

**`GameLayer`** — [`crates/protocol/src/hit_detection.rs:17-53`](crates/protocol/src/hit_detection.rs#L17)

```rust
pub enum GameLayer {
    Default,
    Character,  // players + dummy
    Hitbox,
    Projectile,
    Terrain,
}
```

Currently `Hitbox` and `Projectile` only collide with `Character`. To enable hitting world objects, a new layer (e.g. `WorldObject`) would need to be added, and hitboxes/projectiles would need to list it as a collision target. This is controlled per-hitbox via `hitbox_collision_layers()` / `projectile_collision_layers()` — both are functions returning `CollisionLayers` values, called at hitbox/bullet spawn time.

The collision layer system does not currently encode PvP vs PvE intent — it only encodes physics geometry grouping.

### 5. Damage Effect in Ability Definitions

**`AbilityEffect::Damage`** — [`crates/protocol/src/ability.rs:98`](crates/protocol/src/ability.rs#L98)

```rust
Damage {
    amount: f32,
    target: EffectTarget,  // Victim, Caster, or OriginalCaster
}
```

The damage effect has no target-type qualifier. It applies `amount` to whatever the resolved entity is. There is no field like `target_category: TargetCategory` or `flags: DamageFlags` that could conditionally gate application.

### 6. Health Component

**`Health`** — [`crates/protocol/src/lib.rs:77-98`](crates/protocol/src/lib.rs#L77)

```rust
pub struct Health {
    pub current: f32,
    pub max: f32,
}
```

`apply_damage` is a bare mutable method — no context, no event, no callback. Any future harvest/XP logic reading damage dealt would need to be added to the call site in `apply_on_hit_effects`, or triggered by observing `Health` component changes.

### 7. Player Entity Identity

**`PlayerId(pub PeerId)`** — [`crates/protocol/src/lib.rs:62-63`](crates/protocol/src/lib.rs#L62)

Player entities carry `PlayerId`. The dummy target entity has `CharacterMarker + Health` but no `PlayerId`. This distinction already exists and could be used to discriminate player vs non-player targets in hit detection — `Has<PlayerId>` vs absence thereof.

### 8. No Faction, Duel, or Interaction Mode

A grep across the entire codebase for `team`, `faction`, `duel`, `pvp`, `pve`, `alliance`, `interactable` returns zero matches. There is no infrastructure for:
- Opt-in combat between players
- Per-entity interaction permissions
- Social state (challenged, dueling, allied)

## Architecture Documentation

### Current Data Flow (Damage)

```
PlayerInput → ability_activation → ActiveAbility spawned
→ dispatch_effect_markers → OnHitEffects set on ability entity
  (carries: caster, original_caster, Vec<AbilityEffect>)
→ apply_on_cast_effects → hitbox/projectile entities spawned
  (OnHitEffects copied to hitbox/bullet, collision layers set)
→ [physics tick: CollidingEntities populated]
→ process_hitbox_hits / process_projectile_hits
  Filter: target must have CharacterMarker
  Filter: target is not caster/original_caster (self-hit)
  → apply_on_hit_effects(victim, source_pos, on_hit)
    Gate: ActiveShield absorb
    Gate: Invulnerable skip
    → health.apply_damage(amount)
```

### What "originator" data is available at hit time

At the moment `apply_on_hit_effects` is called:
- `on_hit.original_caster` — the player entity who cast the ability
- `on_hit.caster` — the proximate caster (usually same as original for non-sub-abilities)
- `victim` — the entity being hit (passed as argument)
- `source_pos: Vec3` — world position of the hitbox/projectile at moment of contact

All four pieces of context are available inline. Any reward/harvest/XP logic has access to both attacker and defender identities at the hit site.

### Key Extension Points

Three locations in the pipeline are natural extension points:

1. **Collision layer gating** (`hitbox_collision_layers`, `projectile_collision_layers`) — controls what physics geometry hitboxes can touch
2. **Hit detection filter** (`process_hitbox_hits:109`, `process_projectile_hits:174`) — the `target_query` `With<CharacterMarker>` filter determines what entity types enter `apply_on_hit_effects`
3. **Damage gate** (`apply_on_hit_effects:249`) — the `ActiveShield`/`Invulnerable` checks; a PvP-opt-in or faction check would insert here

## Relating Findings to Vision Requirements

### Requirement: Players cannot hurt each other by default

Currently all players CAN hurt each other — there is no gate. The `original_caster` is excluded (self-hit), but all other `CharacterMarker` entities are valid targets. A PvP gate is absent.

The existing `Invulnerable` component demonstrates the gate pattern: a component present on the target causes `apply_on_hit_effects` to skip the damage call. A similar component (e.g. `NoPvp`) present on the victim (or checked against both caster and victim) could gate player-to-player damage without affecting player-to-world-object damage.

### Requirement: Players can destroy world objects

World objects do not exist yet, but the pattern is clear: they need `Health`, a collision layer visible to hitboxes/projectiles, and inclusion in the `target_query` filter. Currently the filter is `With<CharacterMarker>` — world objects would need either `CharacterMarker` (unlikely, semantically wrong) or a trait-like alternative (a `Damageable` marker component, or a query union).

### Requirement: Duel opt-in enables mutual damage

The existing `Invulnerable` component and the `original_caster` exclusion show that the system supports per-entity, per-tick damage gating. A `Dueling { opponent: Entity }` component on both participants, checked in `apply_on_hit_effects`, would allow damage to flow only between paired players.

### Requirement: Harvest resources and XP from world objects

Harvesting requires knowing (a) who hit, (b) what was hit, (c) how much damage was dealt. All three are present at the `apply_on_hit_effects` call site. Currently `health.apply_damage` returns `()` — it doesn't report actual damage dealt (clamped at 0). The actual damage number would need to be computed before calling apply_damage, or `apply_damage` would need to return the effective amount. An event (`HitEvent { attacker, victim, damage }`) emitted from that site could fan out to separate systems for XP, loot drops, and analytics.

### Requirement: XP from abilities on both world objects and players

This is the same as harvest: an event fired from `apply_on_hit_effects` with attacker + victim + damage is sufficient. A separate system reads those events and credits XP to the attacker, with XP scaling rules that can vary by victim type (world object vs player, dead vs alive, etc.).

## Code References

- [`crates/protocol/src/ability.rs:294-300`](crates/protocol/src/ability.rs#L294) — `OnHitEffects` struct (caster, original_caster)
- [`crates/protocol/src/ability.rs:81-138`](crates/protocol/src/ability.rs#L81) — `AbilityEffect` enum (all variants)
- [`crates/protocol/src/ability.rs:42`](crates/protocol/src/ability.rs#L42) — `EffectTarget` enum
- [`crates/protocol/src/hit_detection.rs:17-53`](crates/protocol/src/hit_detection.rs#L17) — `GameLayer` enum + collision layer factories
- [`crates/protocol/src/hit_detection.rs:75-128`](crates/protocol/src/hit_detection.rs#L75) — `process_hitbox_hits` (includes CharacterMarker filter at line 109)
- [`crates/protocol/src/hit_detection.rs:148-198`](crates/protocol/src/hit_detection.rs#L148) — `process_projectile_hits`
- [`crates/protocol/src/hit_detection.rs:249-336`](crates/protocol/src/hit_detection.rs#L249) — `apply_on_hit_effects` (damage + force application)
- [`crates/protocol/src/lib.rs:62-63`](crates/protocol/src/lib.rs#L62) — `PlayerId` component
- [`crates/protocol/src/lib.rs:77-98`](crates/protocol/src/lib.rs#L77) — `Health` component
- [`crates/protocol/src/lib.rs:101-104`](crates/protocol/src/lib.rs#L101) — `Invulnerable` component
- [`crates/server/src/gameplay.rs:28-42`](crates/server/src/gameplay.rs#L28) — `spawn_dummy_target` (existing non-player CharacterMarker target)
- [`crates/server/src/gameplay.rs:75-98`](crates/server/src/gameplay.rs#L75) — `check_death_and_respawn`

## Related Research

- [`doc/research/2026-02-13-hit-detection-system.md`](doc/research/2026-02-13-hit-detection-system.md) — detailed hit detection architecture
- [`doc/research/2026-02-07-ability-system-architecture.md`](doc/research/2026-02-07-ability-system-architecture.md) — ability system full architecture
- [`doc/research/2026-02-22-remaining-ability-effect-primitives.md`](doc/research/2026-02-22-remaining-ability-effect-primitives.md) — effect primitive implementations
- [`doc/research/2026-02-16-health-respawn-billboard-ui.md`](doc/research/2026-02-16-health-respawn-billboard-ui.md) — Health + Invulnerable + respawn

## Open Questions

Resolved 2026-02-25.

## Follow-up Research 2026-02-25

Resolving all five open questions based on design decisions provided.

### Q1 — Rollback implications of duel/social state components (elaborated)

**Decision**: Duel state lives as components on player entities.

**The core constraint**: `Health` is registered with `.add_prediction()` ([`crates/protocol/src/lib.rs:173`](crates/protocol/src/lib.rs#L173)). This means Lightyear snapshots `Health` every tick and will trigger a full rollback + re-simulation if the client's predicted `Health` diverges from the server's authoritative value.

During re-simulation, all `FixedUpdate` systems run again from the rollback tick, including `apply_on_hit_effects`. If `apply_on_hit_effects` reads a gate component (like `Invulnerable`, or a future `Dueling`) to decide whether to call `health.apply_damage()`, that gate component **must also be predicted**, or the re-simulation will produce a different result than the original simulation — causing a cascade of spurious `Health` rollbacks.

**Evidence from the existing pattern**: `Invulnerable` is registered with `.add_prediction()` ([`crates/protocol/src/lib.rs:174`](crates/protocol/src/lib.rs#L174)) even though it is entirely server-controlled (inserted and removed only in `check_death_and_respawn` and `expire_invulnerability` on the server). The client never predicts its insertion — but it must be in the prediction system so rollback re-simulation can see it when re-running the damage gate check.

**Implication for duel state**: A `Dueling { opponent: Entity }` component (or a `PvpEnabled` marker) that gates player-to-player damage in `apply_on_hit_effects` must be registered with `.add_prediction()`. Without it:

- Client simulates tick N: player A hits player B, `Dueling` is absent → damage skipped → Health unchanged
- Server simulates tick N: same hit, `Dueling` is present → damage applied → Health decremented
- Replication arrives: `Health` mismatch → rollback triggered
- Re-simulation: without `Dueling` in the snapshot, re-simulation still skips damage → mismatch never resolves → rollback loops

**The non-predicted replicated components** (`PlayerId`, `AbilitySlots`, `Name`) are safe precisely because none of them gate any predicted component mutations. They are write-once or slowly-changing metadata that the client reads only for display, not for simulation logic.

**Conclusion**: Any component that conditionally affects a mutation of a predicted component (`Health`, `LinearVelocity`, `Position`) must itself be predicted. For duel state specifically — both the `Dueling` component and any PvP opt-in marker must use `.add_prediction()`.

---

### Q2 — World object target classification

**Decision**: Use a `Damageable` marker component.

`CharacterMarker` is semantically specific to player-controlled characters and the training dummy. World objects (destructible terrain features, resource nodes, interactive props) should carry `Damageable` instead.

The hit detection filter in `process_hitbox_hits:109` and `process_projectile_hits:174` currently uses `With<CharacterMarker>` on `target_query`. Replacing it with `With<Damageable>` — and adding `Damageable` to player entities — admits both character and world-object entities through the same damage pipeline uniformly.

`CharacterMarker` remains as the discriminator for systems that are character-specific: `check_death_and_respawn`, `handle_character_movement`, `ability_activation`. World objects don't move or cast abilities, so they don't need `CharacterMarker`.

**Collision layer change needed**: World objects would need a `GameLayer::WorldObject` (or equivalent) layer, and `hitbox_collision_layers()` / `projectile_collision_layers()` would need to list `WorldObject` alongside `Character` as a valid target. Currently those layers only collide with `Character`.

**World object `Health` registration**: World objects are not client-controlled and have no input prediction, so their `Health` does not need `.add_prediction()` — replicated-only is appropriate. This differs from player `Health`. If this distinction matters at the damage gate, the `Damageable` component could carry a flag or a separate `WorldObjectMarker` component can be queried to route different post-hit logic (harvesting vs respawn).

---

### Q3 — Damage event replication

**Decision**: Replicate it — client UI needs to show it.

A `HitEvent` emitted from `apply_on_hit_effects` carries `{ attacker: Entity, victim: Entity, damage: f32 }`. Since this must reach the client for UI feedback (floating damage numbers, XP gain popups, harvest notifications), it must be a Lightyear replicated message, not a plain Bevy `Event`.

In Lightyear, server-to-client messages are sent via `server.send_message_to_target::<Channel, Msg>(msg, target)`. The message needs to be registered on the channel in `ProtocolPlugin`. The client reads it via `client.read_event::<Msg>()` (or the Bevy event reader pattern Lightyear provides).

**Channel considerations**: Hit feedback is not rollback-sensitive — it's informational. An unordered, unreliable channel is appropriate. Missing one floating damage number is acceptable; re-ordering is acceptable. This avoids head-of-line blocking.

**XP/harvest logic itself** runs server-side only, reading the event data. The replicated message to the client is purely for feedback display, not for authoritative state.

---

### Q4 — Duel initiation protocol

**Decision**: Components on player entities.

Two components on the involved player entities:

- `DuelRequest { from: Entity, expires_at: Tick }` — inserted on the challenged player when a challenge is sent; removed on accept, decline, or timeout
- `Dueling { opponent: Entity }` — inserted on both players when the duel begins; removed when the duel ends (one player dies, concedes, or leaves range)

Both must be registered with `.add_prediction()` (per Q1 — `Dueling` gates damage, `DuelRequest` may gate ability use or UI state that feeds into predicted logic).

The challenge/accept flow is server-authoritative: the client sends a `ChallengeRequest { target: Entity }` message; the server validates (both players in overworld, not already dueling, within range), inserts `DuelRequest` on the target, and replicates. The target's client shows a UI prompt; accept sends `ChallengeAccept`; the server inserts `Dueling` on both entities and removes `DuelRequest`.

Duel end is detected by the server's `check_death_and_respawn` system (or a dedicated `check_duel_end` system) — when a `Dueling` player dies or the duel timer expires, the server removes `Dueling` from both entities.

**Why not a dedicated "duel session" entity**: Player entity components are simpler to replicate (existing replication config applies), simpler to query in `apply_on_hit_effects` (already queries the victim entity directly), and components are removed atomically with the player entity if they disconnect.

---

### Q5 — `apply_damage` return value

**Decision**: Return the damage dealt.

Current signature:
```rust
pub fn apply_damage(&mut self, damage: f32) {
    self.current = (self.current - damage).max(0.0);
}
```

Changed to return the actual damage applied (clamped by remaining health):
```rust
pub fn apply_damage(&mut self, damage: f32) -> f32 {
    let actual = damage.min(self.current);
    self.current -= actual;
    actual
}
```

`actual` is the value that feeds into the `HitEvent { attacker, victim, damage: actual }` emitted after the damage call. XP and harvest rewards scale from `actual`, not from `amount`, so overkill damage does not inflate rewards. The shield absorption path in `apply_on_hit_effects` similarly benefits — the shield already computes the split, so the health damage portion is already the correct clamped value by the time `apply_damage` is called.
