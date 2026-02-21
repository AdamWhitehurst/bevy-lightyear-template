# Ability Effect Primitives Implementation Plan

## Overview

Refactor the ability system from a single `AbilityEffect` per ability to a composable `Vec<EffectTrigger>` model, migrate `ActiveAbility` from a
character component to a standalone spawned entity, add `Damage`/`ApplyForce`/`OnInput` primitives, and convert the punch combo from step-based
chaining to separate abilities linked via `OnInput`.

## Current State Analysis

- **3 ability effects**: `Melee` (inline knockback+damage), `Projectile` (inline knockback+damage), `Dash` (speed)
- **Single `effect` field** on `AbilityDef` with `steps`/`step_window_ticks` for combo chaining
- **`ActiveAbility` is a component** on the character entity; `Without<ActiveAbility>` filter prevents simultaneous abilities
- **Marker-based dispatch**: `DashAbilityEffect`, `MeleeHitboxActive`, `ProjectileSpawnAbilityEffect` inserted/removed by `dispatch_effect_markers`
- **Hardcoded hit effects**: `apply_hit` in [hit_detection.rs:134](crates/protocol/src/hit_detection.rs#L134) applies knockback + damage directly
- **5-system chain** in FixedUpdate: activation → update → dispatch → projectile_spawn → dash_effect
- **14 tests** covering activation, phase transitions, combo chaining, dash velocity, bullet lifetime

### Key Discoveries:

- `dispatch_effect_markers` at [ability.rs:392](crates/protocol/src/ability.rs#L392) delegates to 3 helpers based on single `def.effect` match — needs
  full rewrite for `Vec<EffectTrigger>`
- `ability_activation` at [ability.rs:252](crates/protocol/src/ability.rs#L252) uses `Without<ActiveAbility>` filter — entity migration removes this
  constraint
- `process_melee_hits` at [hit_detection.rs:67](crates/protocol/src/hit_detection.rs#L67) reads `MeleeHitboxActive.knockback_force`/`base_damage` —
  these move to `OnHitEffects`
- `ability_projectile_spawn` at [ability.rs:473](crates/protocol/src/ability.rs#L473) reads `ProjectileSpawnAbilityEffect` fields including
  knockback/damage — these move to the projectile's sub-ability `OnHitEffects`
- `PreSpawned::default_with_salt(active.step as u64)` at [ability.rs:506](crates/protocol/src/ability.rs#L506) — salt needs new source after step
  removal
- Tests use hardcoded `AbilityDef` structs with old fields — all need migration

## Desired End State

After this plan:

- `AbilityDef` uses `effects: Vec<EffectTrigger>` instead of `effect: AbilityEffect`
- `ActiveAbility` is a standalone spawned entity with `caster`, `original_caster`, `target`, `depth` fields
- Multiple abilities can be active simultaneously per character (gated only by `AbilityCooldowns`)
- `Damage` and `ApplyForce` are composable effect primitives fired via `OnHit` triggers
- Punch is a 3-ability chain using `OnInput` triggers instead of `steps`/`step_window_ticks`
- `steps`, `step_window_ticks`, `step`, `total_steps`, `chain_input_received` are fully removed
- All old per-variant marker components (`DashAbilityEffect`, `MeleeHitboxActive`, `ProjectileSpawnAbilityEffect`) replaced by generic trigger markers

### Verification:

- All tests pass with new data model
- `cargo server` + `cargo client` works: punch combo (3 hits), dash, fireball all function correctly
- Punch hits apply damage and knockback via `Damage`/`ApplyForce` primitives
- Fireball hits apply damage and knockback via sub-ability `OnHit` effects
- Dash moves character via `WhileActive(SetVelocity(...))`
- Punch combo chains via `OnInput` — pressing Ability1 during active window triggers next punch

## What We're NOT Doing

- `AreaOfEffect`, `Buff`, `Shield`, `Teleport`, `Grab`, `Summon` effect variants (future plan)
- `OnEnd` trigger implementation (defined but not wired — no current abilities use it)
- `Ability { id, target }` recursive sub-ability activation (partially needed for OnInput, but full recursive chain is future)
- Movement suppression via `WhileActive(SetVelocity { speed: 0.0, ... })`
- `EffectTarget::OriginalCaster` resolution (defined but only `Caster`/`Victim` used in this scope)

---

## Phase 1: Data Model + Entity Migration

### Overview

Change all type definitions, migrate RON, convert `ActiveAbility` to a standalone entity, and update activation/phase-management systems. Keep combo
behavior working temporarily via old fields (removed in Phase 4).

### Changes Required:

#### 1. New enums in ability.rs

**File**: `crates/protocol/src/ability.rs`

Add `EffectTarget` and `EffectTrigger` enums after the existing `AbilityEffect`:

```rust
/// Who receives the effect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum EffectTarget {
    Caster,
    Victim,
    OriginalCaster,
}

/// Controls when an effect fires during an ability's lifecycle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum EffectTrigger {
    OnCast(AbilityEffect),
    WhileActive(AbilityEffect),
    OnHit(AbilityEffect),
    OnEnd(AbilityEffect),
    OnInput { action: PlayerActions, effect: AbilityEffect },
}
```

#### 2. Expand AbilityEffect enum

**File**: `crates/protocol/src/ability.rs`

Replace the current 3-variant enum:

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

Note: All variants defined now for RON forward-compatibility; only `Melee`, `Projectile`, `SetVelocity`, `Damage`, `ApplyForce`, `Ability` are
implemented in this plan.

#### 3. Update AbilityDef

**File**: `crates/protocol/src/ability.rs`

```rust
pub struct AbilityDef {
    pub startup_ticks: u16,
    pub active_ticks: u16,
    pub recovery_ticks: u16,
    pub cooldown_ticks: u16,
    // TEMPORARY: kept until Phase 4 removes combo step logic
    #[serde(default = "default_steps")]
    pub steps: u8,
    #[serde(default)]
    pub step_window_ticks: u16,
    pub effects: Vec<EffectTrigger>,
}

fn default_steps() -> u8 { 1 }
```

The `serde(default)` annotations allow new RON abilities (punch2, punch3) to omit `steps`/`step_window_ticks` while keeping them for backward compat
during Phase 1-3.

#### 4. Update ActiveAbility

**File**: `crates/protocol/src/ability.rs`

```rust
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveAbility {
    pub ability_id: AbilityId,
    pub phase: AbilityPhase,
    pub phase_start_tick: Tick,
    pub caster: Entity,
    pub original_caster: Entity,
    pub target: Entity,
    pub depth: u8,
    // TEMPORARY: kept until Phase 3 removes combo step logic
    pub step: u8,
    pub total_steps: u8,
    pub chain_input_received: bool,
}
```

#### 5. Update ability_activation to spawn entities

**File**: `crates/protocol/src/ability.rs`

Replace the current `ability_activation` system. Key changes:

- Remove `Without<ActiveAbility>` filter — characters can have multiple active abilities
- Spawn a standalone entity instead of inserting a component
- Add Lightyear replication components on the spawned entity

```rust
pub fn ability_activation(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut query: Query<
        (Entity, &ActionState<PlayerActions>, &AbilitySlots, &mut AbilityCooldowns),
        With<CharacterMarker>,
    >,
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();

    for (entity, action_state, slots, mut cooldowns) in &mut query {
        for (slot_idx, action) in ABILITY_ACTIONS.iter().enumerate() {
            if !action_state.just_pressed(action) {
                continue;
            }
            let Some(ref ability_id) = slots.0[slot_idx] else { continue };
            let Some(def) = ability_defs.get(ability_id) else {
                warn!("Ability {:?} not found in defs", ability_id);
                continue;
            };
            if cooldowns.is_on_cooldown(slot_idx, tick, def.cooldown_ticks) {
                continue;
            }

            cooldowns.last_used[slot_idx] = Some(tick);

            let mut cmd = commands.spawn((
                ActiveAbility {
                    ability_id: ability_id.clone(),
                    phase: AbilityPhase::Startup,
                    phase_start_tick: tick,
                    caster: entity,
                    original_caster: entity,
                    target: entity,
                    depth: 0,
                    step: 0,
                    total_steps: def.steps,
                    chain_input_received: false,
                },
                PreSpawned::default_with_salt(slot_idx as u64),
                Name::new("ActiveAbility"),
            ));

            if let Ok(controlled_by) = server_query.get(entity) {
                cmd.insert((
                    Replicate::to_clients(NetworkTarget::All),
                    PredictionTarget::to_clients(NetworkTarget::All),
                    *controlled_by,
                ));
            }

            break;
        }
    }
}
```

#### 6. Update update_active_abilities for standalone entities

**File**: `crates/protocol/src/ability.rs`

The system now queries `ActiveAbility` entities directly. It looks up the caster's `ActionState` and `AbilitySlots` via a second query:

```rust
pub fn update_active_abilities(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut query: Query<(Entity, &mut ActiveAbility)>,
    character_query: Query<(&ActionState<PlayerActions>, &AbilitySlots), With<CharacterMarker>>,
) {
    let tick = timeline.tick();

    for (entity, mut active) in &mut query {
        let Some(def) = ability_defs.get(&active.ability_id) else {
            warn!("Ability {:?} not found", active.ability_id);
            commands.entity(entity).try_despawn();
            continue;
        };

        if let Ok((action_state, slots)) = character_query.get(active.caster) {
            set_chain_input_received(&mut active, action_state, slots);
        }
        advance_ability_phase(&mut commands, entity, &mut active, def, tick);
    }
}
```

#### 7. Update advance_ability_phase for entity despawn

**File**: `crates/protocol/src/ability.rs`

In `advance_ability_phase`, replace `commands.entity(entity).remove::<ActiveAbility>()` with `commands.entity(entity).try_despawn()` (two
occurrences: recovery complete and chain window expired).

#### 8. Migrate abilities.ron

**File**: `assets/abilities.ron`

```ron
(
    abilities: {
        "punch": (
            startup_ticks: 4,
            active_ticks: 3,
            recovery_ticks: 6,
            cooldown_ticks: 16,
            steps: 3,
            step_window_ticks: 20,
            effects: [
                OnCast(Melee(id: None, target: Caster)),
                OnHit(Damage(amount: 10.0, target: Victim)),
                OnHit(ApplyForce(force: 5.0, target: Victim)),
            ],
        ),
        "dash": (
            startup_ticks: 2,
            active_ticks: 8,
            recovery_ticks: 4,
            cooldown_ticks: 32,
            effects: [
                WhileActive(SetVelocity(speed: 15.0, target: Caster)),
            ],
        ),
        "fireball": (
            startup_ticks: 6,
            active_ticks: 2,
            recovery_ticks: 8,
            cooldown_ticks: 42,
            effects: [
                OnCast(Projectile(id: "fireball_hit", speed: 20.0, lifetime_ticks: 192)),
            ],
        ),
        "fireball_hit": (
            startup_ticks: 0,
            active_ticks: 1,
            recovery_ticks: 0,
            cooldown_ticks: 0,
            effects: [
                OnHit(Damage(amount: 25.0, target: Victim)),
                OnHit(ApplyForce(force: 8.0, target: Victim)),
            ],
        ),
    },
)
```

Note: `punch` temporarily keeps `steps: 3` and `step_window_ticks: 20` until Phase 3 migrates it to 3 separate abilities. `dash` and `fireball` omit
these fields (using `serde(default)`). `fireball_hit` is a new sub-ability for projectile on-hit effects.

#### 9. Update dispatch_effect_markers to read Vec<EffectTrigger>

**File**: `crates/protocol/src/ability.rs`

The dispatch system needs to extract effects from the trigger list. During this phase, keep the old marker components but populate them from the new
format:

```rust
pub fn dispatch_effect_markers(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    query: Query<(Entity, &ActiveAbility)>,
) {
    let tick = timeline.tick();

    for (entity, active) in &query {
        let Some(def) = ability_defs.get(&active.ability_id) else {
            warn!("Ability {:?} not found in defs", active.ability_id);
            continue;
        };

        if active.phase == AbilityPhase::Active {
            dispatch_while_active_markers(&mut commands, entity, active, def);
            if active.phase_start_tick == tick {
                dispatch_on_cast_markers(&mut commands, entity, active, def);
            }
        } else {
            remove_while_active_markers(&mut commands, active.caster);
        }
    }
}
```

Update `dispatch_while_active_markers` to iterate `def.effects`:

```rust
fn dispatch_while_active_markers(
    commands: &mut Commands,
    entity: Entity,
    active: &ActiveAbility,
    def: &AbilityDef,
) {
    for trigger in &def.effects {
        match trigger {
            EffectTrigger::WhileActive(AbilityEffect::SetVelocity { speed, .. }) => {
                commands.entity(active.caster).insert(DashAbilityEffect { speed: *speed });
            }
            EffectTrigger::OnCast(AbilityEffect::Melee { .. }) | EffectTrigger::WhileActive(AbilityEffect::Melee { .. }) => {
                let (damage, force) = extract_on_hit_damage_force(&def.effects, active.ability_id.as_ref());
                commands.entity(active.caster).insert(MeleeHitboxActive {
                    knockback_force: force,
                    base_damage: damage,
                });
            }
            _ => {}
        }
    }
}
```

Add a temporary helper to extract damage/force from OnHit triggers (removed in Phase 2):

```rust
fn extract_on_hit_damage_force(effects: &[EffectTrigger], ability_id: &str) -> (f32, f32) {
    let mut damage = 0.0;
    let mut force = 0.0;
    for trigger in effects {
        match trigger {
            EffectTrigger::OnHit(AbilityEffect::Damage { amount, .. }) => damage = *amount,
            EffectTrigger::OnHit(AbilityEffect::ApplyForce { force: f, .. }) => force = *f,
            _ => {}
        }
    }
    if damage == 0.0 && force == 0.0 {
        warn!("No damage or force found for {}", ability_id);
    }
    (damage, force)
}
```

Update `dispatch_on_cast_markers` similarly for `Projectile`:

```rust
fn dispatch_on_cast_markers(
    commands: &mut Commands,
    entity: Entity,
    active: &ActiveAbility,
    def: &AbilityDef,
) {
    for trigger in &def.effects {
        if let EffectTrigger::OnCast(AbilityEffect::Projectile { speed, lifetime_ticks, .. }) = trigger {
            let (damage, force) = extract_on_hit_damage_force(&def.effects, active.ability_id.as_ref());
            commands.entity(active.caster).insert(ProjectileSpawnAbilityEffect {
                speed: *speed,
                lifetime_ticks: *lifetime_ticks,
                knockback_force: force,
                base_damage: damage,
            });
        }
    }
}
```

Update `remove_while_active_markers` — it now takes `caster: Entity`:

```rust
fn remove_while_active_markers(commands: &mut Commands, caster: Entity) {
    commands.entity(caster).remove::<DashAbilityEffect>();
    commands.entity(caster).remove::<MeleeHitboxActive>();
    commands.entity(caster).remove::<MeleeHitTargets>();
}
```

#### 10. Update ability_projectile_spawn for entity model

**File**: `crates/protocol/src/ability.rs`

The query changes since `ActiveAbility` is no longer on the character entity. The system needs to look up position/rotation from the caster:

```rust
pub fn ability_projectile_spawn(
    mut commands: Commands,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    caster_query: Query<
        (Entity, &ProjectileSpawnAbilityEffect, &Position, &Rotation),
        With<CharacterMarker>,
    >,
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();

    for (entity, request, position, rotation) in &caster_query {
        let direction = facing_direction(rotation);
        let spawn_info = AbilityProjectileSpawn {
            spawn_tick: tick,
            position: position.0 + direction * PROJECTILE_SPAWN_OFFSET,
            direction,
            speed: request.speed,
            lifetime_ticks: request.lifetime_ticks,
            knockback_force: request.knockback_force,
            base_damage: request.base_damage,
            ability_id: AbilityId("fireball".into()), // temporary: hardcoded until Phase 2
            shooter: entity,
        };

        let mut cmd = commands.spawn((
            spawn_info,
            PreSpawned::default_with_salt(tick.0 as u64),
            Name::new("AbilityProjectileSpawn"),
        ));

        if let Ok(controlled_by) = server_query.get(entity) {
            cmd.insert((
                Replicate::to_clients(NetworkTarget::All),
                PredictionTarget::to_clients(NetworkTarget::All),
                *controlled_by,
            ));
        }

        commands.entity(entity).remove::<ProjectileSpawnAbilityEffect>();
    }
}
```

#### 11. Phase 1 cleanup: scan-based stale marker removal

**File**: `crates/protocol/src/ability.rs`

In Phase 1, multiple `ActiveAbility` entities may reference the same caster simultaneously. The old `cleanup_effect_markers_on_removal` observer
cannot tell if another active ability still needs the marker. Replace it with a scan-based system:

```rust
pub fn cleanup_stale_effect_markers(
    mut commands: Commands,
    marker_query: Query<Entity, (With<CharacterMarker>, Or<(With<DashAbilityEffect>, With<MeleeHitboxActive>)>)>,
    active_query: Query<&ActiveAbility>,
) {
    for character in &marker_query {
        let has_active = active_query.iter().any(|a| a.caster == character && a.phase == AbilityPhase::Active);
        if !has_active {
            remove_while_active_markers(&mut commands, character);
        }
    }
}
```

Register this as a system running after `dispatch_effect_markers`. Remove the `cleanup_effect_markers_on_removal` observer.

#### 12. Update lib.rs exports and registration

**File**: `crates/protocol/src/lib.rs`

- Add to exports: `EffectTarget`, `EffectTrigger`
- Keep `ActiveAbility` registered with `.add_prediction()`

Update the system schedule:

```rust
app.add_systems(
    FixedUpdate,
    (
        ability::ability_activation,
        ability::update_active_abilities,
        ability::dispatch_effect_markers,
        ability::cleanup_stale_effect_markers,
        ability::ability_projectile_spawn,
        ability::ability_dash_effect,
    )
        .chain()
        .run_if(ready.clone()),
);
```

Remove the `cleanup_effect_markers_on_removal` observer registration.

#### 13. Update tests

**File**: `crates/protocol/tests/ability_systems.rs`

All `AbilityDef` constructors need `effects: Vec<EffectTrigger>` instead of `effect: AbilityEffect`. All `ActiveAbility` constructors need `caster`,
`original_caster`, `target`, `depth` fields.

Key test changes:

- `test_defs()`: migrate all 4 ability definitions (punch, dash, fireball, fireball_hit)
- Tests that insert `ActiveAbility` directly: must spawn a **separate entity** instead of inserting on the character
- `activation_blocked_by_active` test: behavior changes — simultaneous abilities are now allowed. Replace with a test verifying cooldown-only gating
- `test_app()`: add `cleanup_stale_effect_markers` to the chain
- Dash velocity tests: dispatch now inserts `DashAbilityEffect` on `active.caster`, so the `ActiveAbility` entity must have `caster` pointing to the
  character

Example `test_defs()` migration for punch:

```rust
AbilityDef {
    startup_ticks: 4,
    active_ticks: 3,
    recovery_ticks: 6,
    cooldown_ticks: 16,
    steps: 3,
    step_window_ticks: 20,
    effects: vec![
        EffectTrigger::OnCast(AbilityEffect::Melee { id: None, target: EffectTarget::Caster }),
        EffectTrigger::OnHit(AbilityEffect::Damage { amount: 10.0, target: EffectTarget::Victim }),
        EffectTrigger::OnHit(AbilityEffect::ApplyForce { force: 5.0, target: EffectTarget::Victim }),
    ],
}
```

Example test helper for spawning ActiveAbility entities:

```rust
fn spawn_active_ability(world: &mut World, active: ActiveAbility) -> Entity {
    world.spawn(active).id()
}
```

Tests that check `app.world().get::<ActiveAbility>(char_entity)` must instead query for `ActiveAbility` entities whose `caster == char_entity`.

### Success Criteria:

#### Automated Verification:

- [ ] `cargo check-all` compiles
- [ ] `cargo test -p protocol` — all tests pass (with updated assertions)

#### Manual Verification:

- [ ] `cargo server` + `cargo client`: punch, dash, fireball all work as before
- [ ] Punch combo chains 3 hits correctly (step-based chaining still active)

---

## Phase 2: Trigger Dispatch Rewrite

### Overview

Replace the per-variant marker components (`DashAbilityEffect`, `MeleeHitboxActive`, `ProjectileSpawnAbilityEffect`) with generic trigger-type
markers (`OnCastEffects`, `WhileActiveEffects`, `OnHitEffects`). Rewrite dispatch and effect systems.

**Melee hitbox design**: `MeleeHitbox` is spawned as a child entity of the `ActiveAbility` entity (not the character). Bevy's hierarchy ensures the
hitbox despawns automatically when `ActiveAbility` is despawned. The `dispatch_effect_markers` else-branch calls
`commands.entity(entity).despawn_descendants()` for explicit cleanup when phase exits Active. Position for the spatial query is still computed from
the caster's `Position` + `Rotation` components at runtime — `GlobalTransform` propagation timing in `FixedUpdate` is unreliable for spatial queries.

### Changes Required:

#### 1. New marker components

**File**: `crates/protocol/src/ability.rs`

```rust
/// One-shot: inserted on first Active tick, consumed by apply_on_cast_effects.
#[derive(Component, Clone, Debug)]
pub struct OnCastEffects(pub Vec<AbilityEffect>);

/// Persistent: present every Active tick, removed on phase exit.
#[derive(Component, Clone, Debug)]
pub struct WhileActiveEffects(pub Vec<AbilityEffect>);

/// Persistent: present every Active tick, removed on phase exit.
/// Each entry is (action, effect); system checks just_pressed on caster.
#[derive(Component, Clone, Debug)]
pub struct OnInputEffects(pub Vec<(PlayerActions, AbilityEffect)>);

/// One-shot: inserted on Active→Recovery transition, consumed by apply_on_end_effects.
#[derive(Component, Clone, Debug)]
pub struct OnEndEffects(pub Vec<AbilityEffect>);

/// Carried on MeleeHitbox and bullet entities for hit detection.
#[derive(Component, Clone, Debug)]
pub struct OnHitEffects {
    pub effects: Vec<AbilityEffect>,
    pub caster: Entity,
    pub original_caster: Entity,
}

/// Marker on a melee hitbox entity (child of ActiveAbility entity).
#[derive(Component, Clone, Debug)]
pub struct MeleeHitbox {
    pub caster: Entity,
}

/// Tracks entities already hit during this melee active window.
#[derive(Component, Clone, Debug, Default)]
pub struct MeleeHitTargets(pub EntityHashSet);
```

#### 2. Rewrite dispatch_effect_markers

**File**: `crates/protocol/src/ability.rs`

```rust
pub fn dispatch_effect_markers(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    query: Query<(Entity, &ActiveAbility)>,
) {
    let tick = timeline.tick();

    for (entity, active) in &query {
        let Some(def) = ability_defs.get(&active.ability_id) else {
            warn!("Ability {:?} not found in defs", active.ability_id);
            continue;
        };

        if active.phase == AbilityPhase::Active {
            let first_tick = active.phase_start_tick == tick;

            if first_tick {
                let on_cast = collect_effects(&def.effects, |t| matches!(t, EffectTrigger::OnCast(_)));
                if !on_cast.is_empty() {
                    commands.entity(entity).insert(OnCastEffects(on_cast));
                }
            }

            let while_active = collect_effects(&def.effects, |t| matches!(t, EffectTrigger::WhileActive(_)));
            if !while_active.is_empty() {
                commands.entity(entity).insert(WhileActiveEffects(while_active));
            }

            let on_input = collect_on_input_effects(&def.effects);
            if !on_input.is_empty() {
                commands.entity(entity).insert(OnInputEffects(on_input));
            }
        } else {
            commands.entity(entity)
                .remove::<WhileActiveEffects>()
                .remove::<OnInputEffects>()
                .despawn_descendants(); // removes MeleeHitbox child if present
        }
    }
}

fn collect_effects(effects: &[EffectTrigger], pred: impl Fn(&EffectTrigger) -> bool) -> Vec<AbilityEffect> {
    effects.iter().filter(|t| pred(t)).map(|t| match t {
        EffectTrigger::OnCast(e) | EffectTrigger::WhileActive(e)
        | EffectTrigger::OnHit(e) | EffectTrigger::OnEnd(e) => e.clone(),
        EffectTrigger::OnInput { effect, .. } => effect.clone(),
    }).collect()
}

fn collect_on_input_effects(effects: &[EffectTrigger]) -> Vec<(PlayerActions, AbilityEffect)> {
    effects.iter().filter_map(|t| match t {
        EffectTrigger::OnInput { action, effect } => Some((*action, effect.clone())),
        _ => None,
    }).collect()
}
```

#### 3. apply_on_cast_effects system

**File**: `crates/protocol/src/ability.rs`

```rust
pub fn apply_on_cast_effects(
    mut commands: Commands,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    ability_defs: Res<AbilityDefs>,
    query: Query<(Entity, &OnCastEffects, &ActiveAbility)>,
    caster_query: Query<(&Position, &Rotation), With<CharacterMarker>>,
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();

    for (entity, on_cast, active) in &query {
        let Ok((pos, rot)) = caster_query.get(active.caster) else {
            warn!("Caster {:?} missing for ability {:?}", active.caster, active.ability_id);
            continue;
        };
        let Some(def) = ability_defs.get(&active.ability_id) else {
            warn!("Ability {:?} not found in defs", active.ability_id);
            continue;
        };
        let direction = facing_direction(rot);

        for effect in &on_cast.0 {
            match effect {
                AbilityEffect::Melee { .. } => {
                    let on_hit = collect_effects(&def.effects, |t| matches!(t, EffectTrigger::OnHit(_)));
                    spawn_melee_hitbox(&mut commands, entity, active, on_hit);
                }
                AbilityEffect::Projectile { id, speed, lifetime_ticks } => {
                    spawn_projectile(
                        &mut commands, tick, active, pos, direction,
                        id, *speed, *lifetime_ticks, &server_query,
                    );
                }
                _ => {}
            }
        }

        commands.entity(entity).remove::<OnCastEffects>();
    }
}
```

#### 4. Melee hitbox entity

**File**: `crates/protocol/src/ability.rs`

`MeleeHitbox` is spawned as a child of the `ActiveAbility` entity. Bevy's entity hierarchy automatically despawns it when the `ActiveAbility` entity
is despawned — either via the explicit `despawn_descendants()` call in `dispatch_effect_markers` (when phase exits Active) or via cascade when
`ActiveAbility.try_despawn()` fires at recovery end. No follow system is needed and no back-reference to the ability entity is needed.

Position for the spatial query is computed from the caster's `Position` + `Rotation` at query time. `GlobalTransform` propagation runs in
`PostUpdate` and is not reliable for `FixedUpdate` spatial queries.

```rust
fn spawn_melee_hitbox(
    commands: &mut Commands,
    active_ability_entity: Entity,
    active: &ActiveAbility,
    on_hit_effects: Vec<AbilityEffect>,
) {
    commands.entity(active_ability_entity).with_children(|parent| {
        parent.spawn((
            MeleeHitbox { caster: active.caster },
            OnHitEffects {
                effects: on_hit_effects,
                caster: active.caster,
                original_caster: active.original_caster,
            },
            MeleeHitTargets::default(),
            Name::new("MeleeHitbox"),
        ));
    });
}
```

#### 5. Rewrite process_melee_hits

**File**: `crates/protocol/src/hit_detection.rs`

```rust
pub fn process_melee_hits(
    spatial_query: SpatialQuery,
    mut hitbox_query: Query<(&MeleeHitbox, &mut MeleeHitTargets, &OnHitEffects)>,
    caster_query: Query<(&Position, &Rotation), With<CharacterMarker>>,
    mut target_query: Query<(&Position, &mut LinearVelocity, &mut Health, Option<&Invulnerable>), With<CharacterMarker>>,
) {
    for (hitbox, mut hit_targets, on_hit) in &mut hitbox_query {
        let Ok((pos, rot)) = caster_query.get(hitbox.caster) else {
            warn!("Caster {:?} missing for melee hitbox", hitbox.caster);
            continue;
        };
        let direction = facing_direction(rot);
        let hitbox_pos = pos.0 + direction * MELEE_HITBOX_OFFSET;

        let filter = SpatialQueryFilter {
            mask: GameLayer::Character.into(),
            excluded_entities: EntityHashSet::from_iter([hitbox.caster]),
        };

        let hits = spatial_query.shape_intersections(
            &Collider::cuboid(
                MELEE_HITBOX_HALF_EXTENTS.x,
                MELEE_HITBOX_HALF_EXTENTS.y,
                MELEE_HITBOX_HALF_EXTENTS.z,
            ),
            hitbox_pos,
            rot.0,
            &filter,
        );

        for target in hits {
            if !hit_targets.0.insert(target) { continue; }
            apply_on_hit_effects(&on_hit.effects, &mut target_query, target, pos.0);
        }
    }
}
```

#### 6. apply_on_hit_effects helper

**File**: `crates/protocol/src/hit_detection.rs`

Replaces `apply_hit`. Dispatches each `AbilityEffect` in the effects list:

```rust
fn apply_on_hit_effects(
    effects: &[AbilityEffect],
    target_query: &mut Query<(&Position, &mut LinearVelocity, &mut Health, Option<&Invulnerable>), With<CharacterMarker>>,
    target: Entity,
    source_pos: Vec3,
) {
    let Ok((target_pos, mut velocity, mut health, invulnerable)) = target_query.get_mut(target) else {
        return;
    };

    for effect in effects {
        match effect {
            AbilityEffect::Damage { amount, .. } => {
                if invulnerable.is_none() {
                    health.apply_damage(*amount);
                }
            }
            AbilityEffect::ApplyForce { force, .. } => {
                let direction = knockback_direction(target_pos.0, source_pos);
                velocity.0 += direction * *force;
            }
            _ => {}
        }
    }
}

fn knockback_direction(target_pos: Vec3, source_pos: Vec3) -> Vec3 {
    let horizontal = (target_pos - source_pos).with_y(0.0);
    if horizontal.length() > 0.01 {
        (horizontal.normalize() + Vec3::Y * 0.3).normalize()
    } else {
        Vec3::Y
    }
}
```

#### 7. Update process_projectile_hits and handle_ability_projectile_spawn

**File**: `crates/protocol/src/hit_detection.rs` and `crates/protocol/src/ability.rs`

When `handle_ability_projectile_spawn` spawns a bullet, look up the sub-ability's `OnHit` effects from `AbilityDefs` and attach `OnHitEffects` to
the bullet. `process_projectile_hits` then reads `OnHitEffects` instead of `KnockbackForce`/`DamageAmount`.

Update `handle_ability_projectile_spawn`:

```rust
pub fn handle_ability_projectile_spawn(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    spawn_query: Query<(Entity, &AbilityProjectileSpawn), Without<AbilityBullets>>,
) {
    for (spawn_entity, spawn_info) in &spawn_query {
        let on_hit_effects = ability_defs
            .get(&spawn_info.ability_id)
            .map(|def| collect_effects(&def.effects, |t| matches!(t, EffectTrigger::OnHit(_))))
            .unwrap_or_else(|| {
                warn!("Sub-ability {:?} not found for projectile on-hit effects", spawn_info.ability_id);
                vec![]
            });

        commands.spawn((
            Position(spawn_info.position),
            Rotation::default(),
            LinearVelocity(spawn_info.direction * spawn_info.speed),
            RigidBody::Kinematic,
            Collider::sphere(BULLET_COLLIDER_RADIUS),
            Sensor,
            CollisionEventsEnabled,
            CollidingEntities::default(),
            crate::hit_detection::projectile_collision_layers(),
            OnHitEffects {
                effects: on_hit_effects,
                caster: spawn_info.shooter,
                original_caster: spawn_info.shooter,
            },
            crate::hit_detection::ProjectileOwner(spawn_info.shooter),
            AbilityBulletOf(spawn_entity),
            DisableRollback,
            Name::new("AbilityBullet"),
        ));
    }
}
```

Update `process_projectile_hits`:

```rust
pub fn process_projectile_hits(
    mut commands: Commands,
    bullet_query: Query<
        (Entity, &CollidingEntities, &OnHitEffects, &ProjectileOwner, &Position),
        With<Sensor>,
    >,
    mut target_query: Query<(&Position, &mut LinearVelocity, &mut Health, Option<&Invulnerable>), With<CharacterMarker>>,
) {
    for (bullet, colliding, on_hit, owner, bullet_pos) in &bullet_query {
        for &target in colliding.iter() {
            if target == owner.0 { continue; }
            if target_query.get(target).is_err() { continue; }
            apply_on_hit_effects(&on_hit.effects, &mut target_query, target, bullet_pos.0);
            commands.entity(bullet).try_despawn();
            break;
        }
    }
}
```

#### 8. apply_while_active_effects system

**File**: `crates/protocol/src/ability.rs`

Replaces `ability_dash_effect`:

```rust
pub fn apply_while_active_effects(
    query: Query<(&WhileActiveEffects, &ActiveAbility)>,
    mut caster_query: Query<(&Rotation, &mut LinearVelocity), With<CharacterMarker>>,
) {
    for (effects, active) in &query {
        let Ok((rotation, mut velocity)) = caster_query.get_mut(active.caster) else {
            warn!("Caster {:?} missing for ability {:?}", active.caster, active.ability_id);
            continue;
        };

        for effect in &effects.0 {
            match effect {
                AbilityEffect::SetVelocity { speed, .. } => {
                    let direction = facing_direction(rotation);
                    velocity.x = direction.x * *speed;
                    velocity.z = direction.z * *speed;
                }
                _ => {}
            }
        }
    }
}
```

#### 9. Remove old components and systems

**File**: `crates/protocol/src/ability.rs` — delete:

- `DashAbilityEffect` struct
- `ProjectileSpawnAbilityEffect` struct
- `MeleeHitboxActive` struct
- `dispatch_while_active_markers` (old version)
- `dispatch_on_cast_markers` (old version)
- `remove_while_active_markers` (old version)
- `extract_on_hit_damage_force`
- `ability_dash_effect`
- `cleanup_effect_markers_on_removal`
- `cleanup_stale_effect_markers`

**File**: `crates/protocol/src/hit_detection.rs` — delete:

- `KnockbackForce` component
- `DamageAmount` component
- `apply_hit` function
- `ensure_melee_hit_targets` system (`MeleeHitTargets` is now inserted by `spawn_melee_hitbox`)

**File**: `crates/protocol/src/ability.rs` — remove `knockback_force` and `base_damage` from `AbilityProjectileSpawn`:

```rust
pub struct AbilityProjectileSpawn {
    pub spawn_tick: Tick,
    pub position: Vec3,
    pub direction: Vec3,
    pub speed: f32,
    pub lifetime_ticks: u16,
    pub ability_id: AbilityId,
    pub shooter: Entity,
}
```

#### 10. Update system schedule

**File**: `crates/protocol/src/lib.rs`

```rust
app.add_systems(
    FixedUpdate,
    (
        ability::ability_activation,
        ability::update_active_abilities,
        ability::dispatch_effect_markers,
        ability::apply_on_cast_effects,
        ability::apply_while_active_effects,
    )
        .chain()
        .run_if(ready.clone()),
);

app.add_systems(
    FixedUpdate,
    (
        hit_detection::process_melee_hits,
        hit_detection::process_projectile_hits,
    )
        .chain()
        .after(ability::dispatch_effect_markers)
        .run_if(ready.clone()),
);
```

Remove `ensure_melee_hit_targets`, `ability_projectile_spawn`, and `ability_dash_effect` from schedule.

#### 11. Update lib.rs exports

**File**: `crates/protocol/src/lib.rs`

- Remove: `DashAbilityEffect`, `ProjectileSpawnAbilityEffect`, `MeleeHitboxActive`
- Add: `OnCastEffects`, `WhileActiveEffects`, `OnHitEffects`, `OnInputEffects`, `OnEndEffects`, `MeleeHitbox`, `MeleeHitTargets`

### Success Criteria:

#### Automated Verification:

- [ ] `cargo check-all` compiles
- [ ] `cargo test -p protocol` — all tests pass

#### Manual Verification:

- [ ] `cargo server` + `cargo client`: punch hits deal damage and knockback
- [ ] Dash moves character correctly
- [ ] Fireball projectile hits deal damage and knockback
- [ ] Punch combo still chains 3 hits (step-based)

---

## Phase 3: OnInput + Punch Combo Migration

### Overview

Implement `OnInput` trigger, migrate punch from step-based chaining to 3 separate abilities linked via `OnInput`, and remove all step/chain
infrastructure.

### Changes Required:

#### 1. apply_on_input_effects system

**File**: `crates/protocol/src/ability.rs`

```rust
pub fn apply_on_input_effects(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    query: Query<(Entity, &OnInputEffects, &ActiveAbility)>,
    caster_query: Query<&ActionState<PlayerActions>, With<CharacterMarker>>,
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();

    for (entity, on_input, active) in &query {
        let Ok(action_state) = caster_query.get(active.caster) else {
            warn!("Caster {:?} missing for ability {:?}", active.caster, active.ability_id);
            continue;
        };

        for (action, effect) in &on_input.0 {
            if !action_state.just_pressed(action) { continue; }

            if let AbilityEffect::Ability { id, target } = effect {
                let Some(_def) = ability_defs.get(&AbilityId(id.clone())) else {
                    warn!("Ability {:?} not found in defs", id);
                    continue;
                };
                let target_entity = resolve_target(target, active);

                let mut cmd = commands.spawn((
                    ActiveAbility {
                        ability_id: AbilityId(id.clone()),
                        phase: AbilityPhase::Startup,
                        phase_start_tick: tick,
                        caster: target_entity,
                        original_caster: active.original_caster,
                        target: target_entity,
                        depth: active.depth + 1,
                    },
                    PreSpawned::default_with_salt(tick.0 as u64 + id.len() as u64),
                    Name::new("ActiveAbility"),
                ));

                if let Ok(controlled_by) = server_query.get(active.caster) {
                    cmd.insert((
                        Replicate::to_clients(NetworkTarget::All),
                        PredictionTarget::to_clients(NetworkTarget::All),
                        *controlled_by,
                    ));
                }

                // Despawn current ability — OnInput consumed it
                commands.entity(entity).try_despawn();
                break;
            }
        }
    }
}

fn resolve_target(target: &EffectTarget, active: &ActiveAbility) -> Entity {
    match target {
        EffectTarget::Caster => active.caster,
        EffectTarget::Victim => active.target,
        EffectTarget::OriginalCaster => active.original_caster,
    }
}
```

#### 2. Migrate punch to 3 abilities

**File**: `assets/abilities.ron`

```ron
"punch": (
    startup_ticks: 4,
    active_ticks: 20,
    recovery_ticks: 0,
    cooldown_ticks: 16,
    effects: [
        OnCast(Melee(id: None, target: Caster)),
        OnHit(Damage(amount: 10.0, target: Victim)),
        OnHit(ApplyForce(force: 5.0, target: Victim)),
        OnInput(action: Ability1, effect: Ability(id: "punch2", target: Caster)),
    ],
),
"punch2": (
    startup_ticks: 4,
    active_ticks: 20,
    recovery_ticks: 0,
    cooldown_ticks: 0,
    effects: [
        OnCast(Melee(id: None, target: Caster)),
        OnHit(Damage(amount: 12.0, target: Victim)),
        OnHit(ApplyForce(force: 6.0, target: Victim)),
        OnInput(action: Ability1, effect: Ability(id: "punch3", target: Caster)),
    ],
),
"punch3": (
    startup_ticks: 4,
    active_ticks: 6,
    recovery_ticks: 10,
    cooldown_ticks: 0,
    effects: [
        OnCast(Melee(id: None, target: Caster)),
        OnHit(Damage(amount: 15.0, target: Victim)),
        OnHit(ApplyForce(force: 10.0, target: Victim)),
    ],
),
```

`active_ticks` on punch/punch2 covers both the hitbox window and the combo-continue window. `punch3` has recovery and no `OnInput`.

#### 3. Remove step/chain infrastructure

**File**: `crates/protocol/src/ability.rs`

From `AbilityDef`, remove: `steps`, `step_window_ticks`, `default_steps()`

From `ActiveAbility`, remove: `step`, `total_steps`, `chain_input_received`

Delete: `has_more_steps()`, `set_chain_input_received()`, all combo branches in `advance_ability_phase()`

Simplified `advance_ability_phase`:

```rust
fn advance_ability_phase(
    commands: &mut Commands,
    entity: Entity,
    active: &mut ActiveAbility,
    def: &AbilityDef,
    tick: Tick,
) {
    let elapsed = tick - active.phase_start_tick;
    let phase_complete = elapsed >= def.phase_duration(&active.phase) as i16;

    match active.phase {
        AbilityPhase::Startup if phase_complete => {
            active.phase = AbilityPhase::Active;
            active.phase_start_tick = tick;
        }
        AbilityPhase::Active if phase_complete => {
            active.phase = AbilityPhase::Recovery;
            active.phase_start_tick = tick;
        }
        AbilityPhase::Recovery if phase_complete => {
            commands.entity(entity).try_despawn();
        }
        _ => {}
    }
}
```

Simplified `update_active_abilities` (no longer needs `ActionState`/`AbilitySlots`):

```rust
pub fn update_active_abilities(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut query: Query<(Entity, &mut ActiveAbility)>,
) {
    let tick = timeline.tick();

    for (entity, mut active) in &mut query {
        let Some(def) = ability_defs.get(&active.ability_id) else {
            warn!("Ability {:?} not found", active.ability_id);
            commands.entity(entity).try_despawn();
            continue;
        };
        advance_ability_phase(&mut commands, entity, &mut active, def, tick);
    }
}
```

#### 4. Update system schedule

**File**: `crates/protocol/src/lib.rs`

Add `apply_on_input_effects` to the chain:

```rust
app.add_systems(
    FixedUpdate,
    (
        ability::ability_activation,
        ability::update_active_abilities,
        ability::dispatch_effect_markers,
        ability::apply_on_cast_effects,
        ability::apply_while_active_effects,
        ability::apply_on_input_effects,
    )
        .chain()
        .run_if(ready.clone()),
);
```

#### 5. Update tests

**File**: `crates/protocol/tests/ability_systems.rs`

- Remove `steps`/`step_window_ticks` from all `AbilityDef` constructors
- Remove `step`/`total_steps`/`chain_input_received` from all `ActiveAbility` constructors
- Delete `combo_chain_advances_step` and `combo_window_expires` tests
- Add `punch2` and `punch3` to `test_defs()`
- Add new tests:
  - `on_input_triggers_sub_ability`: punch in Active phase + just_pressed Ability1 → punch despawned, punch2 spawned
  - `on_input_no_press_no_trigger`: punch in Active phase without press → no new ability spawned
  - `punch_combo_full_chain`: simulate press sequences through punch → punch2 → punch3

### Success Criteria:

#### Automated Verification:

- [ ] `cargo check-all` compiles
- [ ] `cargo test -p protocol` — all tests pass

#### Manual Verification:

- [ ] `cargo server` + `cargo client`: punch combo chains 3 hits via re-pressing Ability1
- [ ] Punch combo terminates after punch3
- [ ] Dash and fireball unchanged
- [ ] No `step`/`chain` fields remain in codebase

---

## Testing Strategy

### Unit Tests (in `crates/protocol/tests/ability_systems.rs`):

- Activation spawns standalone entity with correct fields
- Phase transitions (Startup→Active→Recovery→despawn)
- Cooldown blocking
- Dispatch inserts correct marker components on ActiveAbility entity
- `SetVelocity` applies velocity to caster during Active phase
- `OnInput` fires sub-ability on `just_pressed`
- `OnInput` does not fire without press
- Multiple active abilities can coexist (cooldown-only gating)
- `MeleeHitbox` child entity is despawned when ActiveAbility phase exits Active

### Integration Tests (manual):

- Full punch combo: hit dummy target, verify damage/knockback on each hit
- Dash: verify movement speed and duration
- Fireball: verify projectile spawn, flight, hit damage/knockback
- Combo window: verify re-press during active window chains; no re-press lets ability expire

## Performance Considerations

- `ActiveAbility` entities are short-lived (tens of ticks) — no accumulation concern
- `process_melee_hits` queries `MeleeHitbox` children — same cost as querying characters directly
- `collect_effects` allocates `Vec` per dispatch tick — acceptable for 3-5 item lists; optimize with `SmallVec` later if needed
- `despawn_descendants()` in else-branch of `dispatch_effect_markers` is a no-op when no children exist

## Migration Notes

- Phase 1 maintains behavioral compatibility via temporary shims (`extract_on_hit_damage_force`, keeping `steps` fields with `serde(default)`)
- Phase 2 removes shims and old markers — point of no return for the old dispatch architecture
- Phase 3 removes step/chain infrastructure — point of no return for the old combo system
- Each phase should be a separate commit for clean revert capability

## References

- Design doc: `doc/design/2026-02-13-ability-effect-primitives.md`
- Research: `doc/research/2026-02-20-ability-effect-primitives-implementation-analysis.md`
- Current ability system: [ability.rs](crates/protocol/src/ability.rs)
- Current hit detection: [hit_detection.rs](crates/protocol/src/hit_detection.rs)
- System schedule: [lib.rs:239-268](crates/protocol/src/lib.rs#L239)
- RON definitions: [abilities.ron](assets/abilities.ron)
- Tests: [ability_systems.rs](crates/protocol/tests/ability_systems.rs)
