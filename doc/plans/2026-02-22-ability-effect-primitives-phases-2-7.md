# Ability Effect Primitives — Phases 2-7 Implementation Plan

## Overview

Implement the remaining ability effect primitives from the [design doc](../design/2026-02-13-ability-effect-primitives.md), building on the Phase 1 foundation (commit `e98449d`). This covers: OnHit/OnEnd/OnInput triggers, Damage/ApplyForce/AreaOfEffect/Buff/Shield/Teleport/Ability effects, hitbox entity spawning, and the Melee/Projectile refactoring to decouple damage from hitbox definitions.

## Current State Analysis

Phase 1 delivered:
- `ActiveAbility` as prespawned/predicted entities ([ability.rs:120-130](crates/protocol/src/ability.rs#L120))
- `Vec<EffectTrigger>` on `AbilityDef` with `OnCast`/`WhileActive` triggers ([ability.rs:52-59](crates/protocol/src/ability.rs#L52))
- `dispatch_effect_markers` → `apply_on_cast_effects` → `apply_while_active_effects` pipeline
- Three abilities: punch (melee), dash (SetVelocity), fireball (projectile)

### Key Discoveries:
- `Melee { knockback_force, base_damage }` bakes damage/knockback inline; design wants composable `OnHit(Damage)` + `OnHit(ApplyForce)` ([ability.rs:47](crates/protocol/src/ability.rs#L47))
- `Projectile { speed, lifetime_ticks, knockback_force, base_damage }` same issue ([ability.rs:48](crates/protocol/src/ability.rs#L48))
- `KnockbackForce`, `DamageAmount`, `ProjectileOwner` components on bullet entities will be replaced by `OnHitEffects` ([hit_detection.rs:9-15](crates/protocol/src/hit_detection.rs#L9))
- `apply_hit` function handles both damage and knockback atomically ([hit_detection.rs:135-155](crates/protocol/src/hit_detection.rs#L135))
- Melee uses ephemeral spatial queries — no hitbox entity ([hit_detection.rs:68-109](crates/protocol/src/hit_detection.rs#L68))
- `GameLayer::Hitbox` and character interaction with it already defined ([hit_detection.rs:28,40](crates/protocol/src/hit_detection.rs#L28))
- `AbilityBulletOf`/`AbilityBullets` relationship with `linked_spawn` is the pattern for hitbox entities ([ability.rs:207-214](crates/protocol/src/ability.rs#L207))
- `handle_character_movement` has no explicit ordering relative to ability systems ([gameplay.rs:44-69](crates/server/src/gameplay.rs#L44))

## Desired End State

After all phases:
- Damage and knockback flow through composable `OnHit(Damage)` + `OnHit(ApplyForce)` effects
- Melee hitboxes are entity-based with unified collision detection
- `OnEnd` and `OnInput` triggers enable cleanup effects and combo chaining
- `Ability { id, target }` enables recursive sub-ability spawning (punch combo chain)
- `AreaOfEffect` spawns sphere hitboxes for AoE abilities
- `Buff`, `Shield`, `Teleport` provide defensive and utility effect primitives
- All abilities defined in RON match the design doc's format

### Verification:
- Punch does damage and knockback via `OnHit` effects
- Fireball projectile does damage and knockback via `OnHit` effects
- Punch 3-step combo works via `OnInput` → `Ability` chain
- AreaOfEffect abilities create sphere hitboxes that detect collisions
- Buff/Shield/Teleport effects work at runtime
- All tests pass, server/client build and run

## What We're NOT Doing

- `Grab` / `Grabbing(Entity)` / `GrabbedBy(Entity)` — Phase 8
- `Summon { entity_type, lifetime_ticks }` — Phase 9, needs entity behavior system
- `ActiveAbilityOf`/`ActiveAbilities` custom relationship (character → ability) — deferred until needed
- Stat system integration for `Buff` beyond simple multiplier tracking
- Projectile-as-sub-ability-host (projectile entity becomes `caster` of a sub-ability) — deferred to future work when needed

---

## Phase 2: OnHit + Damage + ApplyForce + Melee/Projectile Refactor

### Overview

The core architectural change: decouple damage/knockback from hitbox definitions. Introduce `OnHit` trigger, `Damage` and `ApplyForce` effect variants, and the `OnHitEffects` component. Refactor `Melee`/`Projectile` to remove baked-in damage/knockback fields.

### Changes Required

#### 1. EffectTarget Default
**File**: `crates/protocol/src/ability.rs`

Add `Default` derive to `EffectTarget`:
```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect, Default)]
pub enum EffectTarget {
    #[default]
    Caster,
    Victim,
    OriginalCaster,
}
```

#### 2. AbilityEffect Variants
**File**: `crates/protocol/src/ability.rs`

Replace current `AbilityEffect` enum:
```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum AbilityEffect {
    Melee {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        target: EffectTarget,
    },
    Projectile {
        #[serde(default)]
        id: Option<String>,
        speed: f32,
        lifetime_ticks: u16,
    },
    SetVelocity { speed: f32, target: EffectTarget },
    Damage { amount: f32, target: EffectTarget },
    ApplyForce { force: f32, target: EffectTarget },
}
```

- `Melee`: removes `knockback_force`/`base_damage`, adds `id` (unused until Phase 4) and `target` (unused until Phase 6). Both `#[serde(default)]` so RON `Melee()` works.
- `Projectile`: removes `knockback_force`/`base_damage`, adds `id` (unused until Phase 4). `#[serde(default)]` on `id`.
- `Damage` and `ApplyForce`: new variants.

#### 3. EffectTrigger — OnHit
**File**: `crates/protocol/src/ability.rs`

Add `OnHit` variant:
```rust
pub enum EffectTrigger {
    OnCast(AbilityEffect),
    WhileActive(AbilityEffect),
    /// Fires when a hitbox/projectile spawned by this ability hits a target.
    OnHit(AbilityEffect),
}
```

#### 4. OnHitEffects Component
**File**: `crates/protocol/src/ability.rs`

```rust
/// Carried on ActiveAbility entities (for melee) and bullet entities (for projectiles).
/// Hit detection systems read this to determine what effects to apply on contact.
#[derive(Component, Clone, Debug)]
pub struct OnHitEffects {
    pub effects: Vec<AbilityEffect>,
    pub caster: Entity,
    pub original_caster: Entity,
    pub depth: u8,
}
```

#### 5. MeleeHitboxActive — Unit Marker
**File**: `crates/protocol/src/ability.rs`

Change from struct with fields to unit marker:
```rust
/// Marker: this ActiveAbility entity has an active melee hitbox.
#[derive(Component, Clone, Debug)]
pub struct MeleeHitboxActive;
```

#### 6. ProjectileSpawnEffect — Remove Damage Fields
**File**: `crates/protocol/src/ability.rs`

```rust
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ProjectileSpawnEffect {
    pub speed: f32,
    pub lifetime_ticks: u16,
}
```

#### 7. AbilityProjectileSpawn — Remove Damage Fields
**File**: `crates/protocol/src/ability.rs`

Remove `knockback_force` and `base_damage` fields:
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

#### 8. dispatch_effect_markers — Dispatch OnHitEffects
**File**: `crates/protocol/src/ability.rs`

Update `dispatch_active_phase_markers` to also collect OnHit effects:
```rust
fn dispatch_active_phase_markers(
    commands: &mut Commands,
    entity: Entity,
    active: &ActiveAbility,
    def: &AbilityDef,
    tick: Tick,
) {
    let first_active_tick = active.phase_start_tick == tick;

    if first_active_tick {
        // OnCast effects (existing)
        let on_cast: Vec<AbilityEffect> = def.effects.iter().filter_map(|t| match t {
            EffectTrigger::OnCast(e) => Some(e.clone()),
            _ => None,
        }).collect();
        if !on_cast.is_empty() {
            commands.entity(entity).insert(OnCastEffects(on_cast));
        }

        // OnHit effects — persist for entire Active phase
        let on_hit: Vec<AbilityEffect> = def.effects.iter().filter_map(|t| match t {
            EffectTrigger::OnHit(e) => Some(e.clone()),
            _ => None,
        }).collect();
        if !on_hit.is_empty() {
            commands.entity(entity).insert(OnHitEffects {
                effects: on_hit,
                caster: active.caster,
                original_caster: active.original_caster,
                depth: active.depth,
            });
        }
    }

    // WhileActive effects (existing, unchanged)
    // ...
}
```

Update `remove_active_phase_markers` to also remove `OnHitEffects`:
```rust
fn remove_active_phase_markers(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).remove::<OnCastEffects>();
    commands.entity(entity).remove::<WhileActiveEffects>();
    commands.entity(entity).remove::<OnHitEffects>();
    commands.entity(entity).remove::<MeleeHitboxActive>();
    commands.entity(entity).remove::<MeleeHitTargets>();
}
```

#### 9. apply_on_cast_effects — Updated Match Arms
**File**: `crates/protocol/src/ability.rs`

```rust
pub fn apply_on_cast_effects(
    mut commands: Commands,
    query: Query<(Entity, &OnCastEffects)>,
) {
    for (entity, effects) in &query {
        for effect in &effects.0 {
            match effect {
                AbilityEffect::Melee { .. } => {
                    commands.entity(entity).insert(MeleeHitboxActive);
                }
                AbilityEffect::Projectile { speed, lifetime_ticks, .. } => {
                    commands.entity(entity).insert(ProjectileSpawnEffect {
                        speed: *speed,
                        lifetime_ticks: *lifetime_ticks,
                    });
                }
                _ => {
                    warn!("Unhandled OnCast effect: {:?}", effect);
                }
            }
        }
        commands.entity(entity).remove::<OnCastEffects>();
    }
}
```

#### 10. ability_projectile_spawn — Propagate OnHitEffects
**File**: `crates/protocol/src/ability.rs`

Read `OnHitEffects` from ActiveAbility entity, insert clone on spawn entity:
```rust
pub fn ability_projectile_spawn(
    mut commands: Commands,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    query: Query<(Entity, &ProjectileSpawnEffect, &ActiveAbility, Option<&OnHitEffects>)>,
    caster_query: Query<(&Position, &Rotation)>,
    server_query: Query<&ControlledBy>,
) {
    // ... existing spawn logic ...
    // After spawning AbilityProjectileSpawn entity:
    if let Some(on_hit) = on_hit_effects {
        cmd.insert(on_hit.clone());
    }
    // ...
}
```

#### 11. handle_ability_projectile_spawn — Transfer OnHitEffects to Bullet
**File**: `crates/protocol/src/ability.rs`

Read `OnHitEffects` from spawn entity, insert on bullet entity. Remove `KnockbackForce`, `DamageAmount`, `ProjectileOwner`:
```rust
pub fn handle_ability_projectile_spawn(
    mut commands: Commands,
    spawn_query: Query<(Entity, &AbilityProjectileSpawn, Option<&OnHitEffects>), Without<AbilityBullets>>,
) {
    for (spawn_entity, spawn_info, on_hit_effects) in &spawn_query {
        let mut bullet_cmd = commands.spawn((
            Position(spawn_info.position),
            Rotation::default(),
            LinearVelocity(spawn_info.direction * spawn_info.speed),
            RigidBody::Kinematic,
            Collider::sphere(BULLET_COLLIDER_RADIUS),
            Sensor,
            CollisionEventsEnabled,
            CollidingEntities::default(),
            crate::hit_detection::projectile_collision_layers(),
            AbilityBulletOf(spawn_entity),
            DisableRollback,
            Name::new("AbilityBullet"),
        ));
        if let Some(on_hit) = on_hit_effects {
            bullet_cmd.insert(on_hit.clone());
        }
    }
}
```

#### 12. apply_on_hit_effects Function
**File**: `crates/protocol/src/hit_detection.rs`

Replace `apply_hit` with composable effect processing:
```rust
fn resolve_on_hit_target(target: &EffectTarget, victim: Entity, on_hit: &OnHitEffects) -> Entity {
    match target {
        EffectTarget::Victim => victim,
        EffectTarget::Caster => on_hit.caster,
        EffectTarget::OriginalCaster => on_hit.original_caster,
    }
}

fn apply_on_hit_effects(
    on_hit: &OnHitEffects,
    victim: Entity,
    source_pos: Vec3,
    target_query: &mut Query<
        (&Position, &mut LinearVelocity, &mut Health, Option<&Invulnerable>),
        With<CharacterMarker>,
    >,
) {
    for effect in &on_hit.effects {
        match effect {
            AbilityEffect::Damage { amount, target } => {
                let entity = resolve_on_hit_target(target, victim, on_hit);
                if let Ok((_, _, mut health, invulnerable)) = target_query.get_mut(entity) {
                    if invulnerable.is_none() {
                        health.apply_damage(*amount);
                    }
                } else {
                    warn!("Damage target {:?} not found", entity);
                }
            }
            AbilityEffect::ApplyForce { force, target } => {
                let entity = resolve_on_hit_target(target, victim, on_hit);
                if let Ok((target_pos, mut velocity, _, _)) = target_query.get_mut(entity) {
                    let horizontal = (target_pos.0 - source_pos).with_y(0.0);
                    let direction = if horizontal.length() > 0.01 {
                        (horizontal.normalize() + Vec3::Y * 0.3).normalize()
                    } else {
                        Vec3::Y
                    };
                    velocity.0 += direction * *force;
                }
            }
            _ => {
                warn!("Unhandled OnHit effect: {:?}", effect);
            }
        }
    }
}
```

#### 13. process_melee_hits — Use OnHitEffects
**File**: `crates/protocol/src/hit_detection.rs`

```rust
pub fn process_melee_hits(
    spatial_query: SpatialQuery,
    mut ability_query: Query<(
        &MeleeHitboxActive,
        &mut MeleeHitTargets,
        &ActiveAbility,
        &OnHitEffects,
    )>,
    caster_query: Query<(&Position, &Rotation)>,
    mut target_query: Query<
        (&Position, &mut LinearVelocity, &mut Health, Option<&Invulnerable>),
        With<CharacterMarker>,
    >,
) {
    for (_hitbox, mut hit_targets, active, on_hit) in &mut ability_query {
        let Ok((pos, rot)) = caster_query.get(active.caster) else {
            warn!("Melee hit: caster {:?} missing Position/Rotation", active.caster);
            continue;
        };
        // ... existing spatial query logic unchanged ...
        for target in hits {
            if !hit_targets.0.insert(target) {
                continue;
            }
            apply_on_hit_effects(on_hit, target, pos.0, &mut target_query);
        }
    }
}
```

#### 14. process_projectile_hits — Use OnHitEffects
**File**: `crates/protocol/src/hit_detection.rs`

```rust
pub fn process_projectile_hits(
    mut commands: Commands,
    bullet_query: Query<
        (Entity, &CollidingEntities, &OnHitEffects, &Position),
        With<Sensor>,
    >,
    mut target_query: Query<
        (&Position, &mut LinearVelocity, &mut Health, Option<&Invulnerable>),
        With<CharacterMarker>,
    >,
) {
    for (bullet, colliding, on_hit, bullet_pos) in &bullet_query {
        for &target in colliding.iter() {
            if target == on_hit.original_caster {
                continue; // don't hit self
            }
            if target_query.get(target).is_err() {
                continue;
            }
            apply_on_hit_effects(on_hit, target, bullet_pos.0, &mut target_query);
            commands.entity(bullet).try_despawn();
            break;
        }
    }
}
```

#### 15. Remove Old Components
**File**: `crates/protocol/src/hit_detection.rs`

Remove `KnockbackForce`, `DamageAmount`, `ProjectileOwner`, and `apply_hit`.

#### 16. Update Exports
**File**: `crates/protocol/src/lib.rs`

Update `pub use hit_detection::` — remove `DamageAmount`, add `OnHitEffects` (if needed externally). Keep `GameLayer`, `character_collision_layers`, `projectile_collision_layers`, `terrain_collision_layers`.

#### 17. Update cleanup_effect_markers_on_removal
**File**: `crates/protocol/src/ability.rs`

Add `OnHitEffects` to cleanup:
```rust
cmd.try_remove::<OnHitEffects>();
```

#### 18. RON Migration
**File**: `assets/abilities.ron`

```ron
(
    abilities: {
        "punch": (
            startup_ticks: 4,
            active_ticks: 3,
            recovery_ticks: 6,
            cooldown_ticks: 16,
            effects: [
                OnCast(Melee()),
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
                OnCast(Projectile(speed: 20.0, lifetime_ticks: 192)),
                OnHit(Damage(amount: 25.0, target: Victim)),
                OnHit(ApplyForce(force: 8.0, target: Victim)),
            ],
        ),
    },
)
```

#### 19. Test Updates
**File**: `crates/protocol/tests/ability_systems.rs`

- Update `test_defs()` to use new `Melee` and `Projectile` signatures plus `OnHit` triggers
- Add test: `on_hit_effects_dispatched_on_first_active_tick` — spawn ActiveAbility, advance to Active, verify `OnHitEffects` component is present with correct effects
- Add test: `on_hit_effects_removed_on_recovery` — advance past Active, verify `OnHitEffects` removed
- Add test: `melee_hitbox_active_is_unit_marker` — verify `MeleeHitboxActive` inserted without fields

### Success Criteria

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Workspace compiles: `cargo check-all`
- [x] Server builds and runs: `cargo server`
- [x] Client builds and runs: `cargo client`

#### Manual Verification:
- [ ] Punch hits target and applies damage + knockback
- [ ] Fireball projectile hits target and applies damage + knockback
- [ ] Dash still works (WhileActive SetVelocity unchanged)
- [ ] No regressions in prediction/rollback behavior

---

## Phase 3: OnEnd Trigger

### Overview

Add `OnEnd` trigger that fires once when ability transitions from Active to Recovery. Small, independent change.

### Changes Required

#### 1. EffectTrigger — OnEnd
**File**: `crates/protocol/src/ability.rs`

```rust
pub enum EffectTrigger {
    OnCast(AbilityEffect),
    WhileActive(AbilityEffect),
    OnHit(AbilityEffect),
    /// Fires once when ability exits Active phase (enters Recovery).
    OnEnd(AbilityEffect),
}
```

#### 2. OnEndEffects Component
**File**: `crates/protocol/src/ability.rs`

```rust
/// One-shot: inserted when Active → Recovery transition happens.
/// Consumed by apply_on_end_effects.
#[derive(Component)]
pub struct OnEndEffects(pub Vec<AbilityEffect>);
```

#### 3. dispatch_effect_markers — Detect Active→Recovery
**File**: `crates/protocol/src/ability.rs`

After the existing Active-phase dispatch, add Recovery-phase detection. When `active.phase == Recovery && active.phase_start_tick == tick`, the transition just happened — insert `OnEndEffects`.

Update `dispatch_effect_markers`:
```rust
pub fn dispatch_effect_markers(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    query: Query<(Entity, &ActiveAbility)>,
) {
    let tick = timeline.tick();
    for (entity, active) in &query {
        let Some(def) = ability_defs.get(&active.def_id) else {
            warn!("dispatch_effect_markers: ability {:?} not found", active.def_id);
            continue;
        };

        if active.phase == AbilityPhase::Active {
            dispatch_active_phase_markers(&mut commands, entity, active, def, tick);
        } else {
            remove_active_phase_markers(&mut commands, entity);
            // Detect Active→Recovery transition
            if active.phase == AbilityPhase::Recovery && active.phase_start_tick == tick {
                dispatch_on_end_markers(&mut commands, entity, def);
            }
        }
    }
}

fn dispatch_on_end_markers(commands: &mut Commands, entity: Entity, def: &AbilityDef) {
    let on_end: Vec<AbilityEffect> = def.effects.iter().filter_map(|t| match t {
        EffectTrigger::OnEnd(e) => Some(e.clone()),
        _ => None,
    }).collect();
    if !on_end.is_empty() {
        commands.entity(entity).insert(OnEndEffects(on_end));
    }
}
```

#### 4. apply_on_end_effects System
**File**: `crates/protocol/src/ability.rs`

```rust
/// Process OnEnd effects — handles effects that fire when ability ends.
pub fn apply_on_end_effects(
    mut commands: Commands,
    query: Query<(Entity, &OnEndEffects, &ActiveAbility)>,
    mut caster_query: Query<(&Rotation, &mut LinearVelocity)>,
) {
    for (entity, effects, active) in &query {
        for effect in &effects.0 {
            match effect {
                AbilityEffect::SetVelocity { speed, target } => {
                    let target_entity = resolve_caster_target(target, active);
                    if let Ok((rotation, mut velocity)) = caster_query.get_mut(target_entity) {
                        let direction = facing_direction(rotation);
                        velocity.x = direction.x * speed;
                        velocity.z = direction.z * speed;
                    }
                }
                _ => {
                    warn!("Unhandled OnEnd effect: {:?}", effect);
                }
            }
        }
        commands.entity(entity).remove::<OnEndEffects>();
    }
}
```

Note: `resolve_caster_target` is a helper that resolves `EffectTarget` to entity using `ActiveAbility` fields (Caster/OriginalCaster only — Victim is invalid in OnEnd context). Extract this as a shared helper from `apply_while_active_effects`.

#### 5. System Schedule
**File**: `crates/protocol/src/lib.rs`

Add `apply_on_end_effects` to the chained system set, after `apply_while_active_effects`:
```rust
(
    ability::ability_activation,
    ability::update_active_abilities,
    ability::dispatch_effect_markers,
    ability::apply_on_cast_effects,
    ability::apply_while_active_effects,
    ability::apply_on_end_effects,
    ability::ability_projectile_spawn,
).chain()
```

#### 6. Cleanup
**File**: `crates/protocol/src/ability.rs`

Add `OnEndEffects` to `cleanup_effect_markers_on_removal`.

#### 7. Tests
- `on_end_effects_dispatched_on_active_to_recovery` — spawn ActiveAbility in Active phase, advance past active_ticks, verify `OnEndEffects` inserted on same tick as Recovery transition

### Success Criteria

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Workspace compiles: `cargo check-all`

#### Manual Verification:
- [ ] No regressions (punch, dash, fireball still work)

---

## Phase 4: Ability { id, target } (Recursive Sub-Abilities)

### Overview

Add `Ability { id, target }` effect variant that spawns a new `ActiveAbility` entity for a named ability. Enables composable ability chains (a melee that triggers a fireball on hit, combo chaining, etc.).

### Changes Required

#### 1. AbilityEffect — Ability Variant
**File**: `crates/protocol/src/ability.rs`

Add to `AbilityEffect` enum:
```rust
Ability { id: String, target: EffectTarget },
```

#### 2. Shared Sub-Ability Spawn Function
**File**: `crates/protocol/src/ability.rs`

Extract a `spawn_sub_ability` function usable from any effect processing system:
```rust
fn spawn_sub_ability(
    commands: &mut Commands,
    ability_defs: &AbilityDefs,
    id: &str,
    target_entity: Entity,
    parent_caster: Entity,
    original_caster: Entity,
    parent_slot: u8,
    parent_depth: u8,
    tick: Tick,
    server_query: &Query<&ControlledBy>,
) {
    if parent_depth >= 4 {
        warn!("Ability recursion depth exceeded for {:?}", id);
        return;
    }
    let ability_id = AbilityId(id.to_string());
    let Some(_def) = ability_defs.get(&ability_id) else {
        warn!("Sub-ability {:?} not found in defs", id);
        return;
    };
    let depth = parent_depth + 1;
    let salt = compute_sub_ability_salt(original_caster, parent_slot, depth, id);

    let mut cmd = commands.spawn((
        ActiveAbility {
            def_id: ability_id,
            caster: target_entity,
            original_caster,
            target: target_entity,
            phase: AbilityPhase::Startup,
            phase_start_tick: tick,
            ability_slot: parent_slot,
            depth,
        },
        PreSpawned::default_with_salt(salt),
        Name::new("ActiveAbility"),
    ));

    if let Ok(controlled_by) = server_query.get(original_caster) {
        cmd.insert((
            Replicate::to_clients(NetworkTarget::All),
            PredictionTarget::to_clients(NetworkTarget::All),
            *controlled_by,
        ));
    }
}

fn compute_sub_ability_salt(original_caster: Entity, slot: u8, depth: u8, id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    original_caster.hash(&mut hasher);
    slot.hash(&mut hasher);
    depth.hash(&mut hasher);
    id.hash(&mut hasher);
    hasher.finish()
}
```

Note: The salt uses a hash of (original_caster, slot, depth, id) to ensure determinism across client/server. `original_caster` entity ID is consistent because it's a replicated entity matched via prespawning.

#### 3. Handle Ability Variant in Effect Systems

Add `Ability { id, target }` handling to `apply_on_cast_effects`, `apply_on_end_effects`, and the `apply_on_hit_effects` function. Each needs access to `AbilityDefs`, `LocalTimeline`, `Query<&ControlledBy>`.

For `apply_on_cast_effects`:
```rust
pub fn apply_on_cast_effects(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    server_query: Query<&ControlledBy>,
    query: Query<(Entity, &OnCastEffects, &ActiveAbility)>,
) {
    let tick = timeline.tick();
    for (entity, effects, active) in &query {
        for effect in &effects.0 {
            match effect {
                AbilityEffect::Melee { .. } => { /* existing */ }
                AbilityEffect::Projectile { .. } => { /* existing */ }
                AbilityEffect::Ability { id, target } => {
                    let target_entity = resolve_caster_target(target, active);
                    spawn_sub_ability(
                        &mut commands, &ability_defs, id, target_entity,
                        active.caster, active.original_caster,
                        active.ability_slot, active.depth, tick, &server_query,
                    );
                }
                _ => { warn!("Unhandled OnCast effect: {:?}", effect); }
            }
        }
        commands.entity(entity).remove::<OnCastEffects>();
    }
}
```

Similarly for `apply_on_end_effects` and within `apply_on_hit_effects` (which becomes a system or receives additional parameters).

For `apply_on_hit_effects`, since it runs inline from hit detection and now needs to spawn entities: either pass `Commands` and `AbilityDefs` through, or convert to an event-based approach. The simplest path: pass `Commands`, `AbilityDefs`, `Tick`, and `Query<&ControlledBy>` as parameters to `apply_on_hit_effects`. This makes the hit detection systems' signatures larger but keeps the inline execution model.

#### 4. Tests
- `sub_ability_spawned_on_cast` — define ability with `OnCast(Ability(id: "punch", target: Caster))`, verify sub-ability entity spawned
- `sub_ability_depth_limited` — verify depth >= 4 prevents spawning
- `sub_ability_phase_management` — verify sub-ability goes through normal phase cycle

### Success Criteria

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Workspace compiles: `cargo check-all`

#### Manual Verification:
- [ ] An ability with `OnCast(Ability(...))` correctly triggers the sub-ability
- [ ] Sub-ability goes through full phase cycle independently

---

## Phase 5: OnInput Trigger + Combo Chaining

### Overview

Add `OnInput { action, effect }` trigger that fires during Active phase when a specific `PlayerActions` input is `just_pressed`. Combined with Phase 4's `Ability` variant, this restores the punch 3-step combo.

### Changes Required

#### 1. EffectTrigger — OnInput
**File**: `crates/protocol/src/ability.rs`

```rust
pub enum EffectTrigger {
    OnCast(AbilityEffect),
    WhileActive(AbilityEffect),
    OnHit(AbilityEffect),
    OnEnd(AbilityEffect),
    /// Fires during Active phase when the specified input is just-pressed.
    OnInput { action: PlayerActions, effect: AbilityEffect },
}
```

#### 2. OnInputEffects Component
**File**: `crates/protocol/src/ability.rs`

```rust
/// Persistent: present every Active tick. Each entry is (action, effect).
/// System checks just_pressed on caster's ActionState.
#[derive(Component)]
pub struct OnInputEffects(pub Vec<(PlayerActions, AbilityEffect)>);
```

#### 3. dispatch_effect_markers — Dispatch OnInputEffects
**File**: `crates/protocol/src/ability.rs`

In `dispatch_active_phase_markers`, add OnInput collection (every Active tick, like WhileActive):
```rust
let on_input: Vec<(PlayerActions, AbilityEffect)> = def.effects.iter().filter_map(|t| match t {
    EffectTrigger::OnInput { action, effect } => Some((*action, effect.clone())),
    _ => None,
}).collect();
if !on_input.is_empty() {
    commands.entity(entity).insert(OnInputEffects(on_input));
}
```

Update `remove_active_phase_markers` to remove `OnInputEffects`.

#### 4. apply_on_input_effects System
**File**: `crates/protocol/src/ability.rs`

```rust
pub fn apply_on_input_effects(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    server_query: Query<&ControlledBy>,
    query: Query<(Entity, &OnInputEffects, &ActiveAbility)>,
    action_query: Query<&ActionState<PlayerActions>>,
) {
    let tick = timeline.tick();
    for (_entity, effects, active) in &query {
        let Ok(action_state) = action_query.get(active.caster) else {
            continue;
        };
        for (action, effect) in &effects.0 {
            if !action_state.just_pressed(action) {
                continue;
            }
            match effect {
                AbilityEffect::Ability { id, target } => {
                    let target_entity = resolve_caster_target(target, active);
                    spawn_sub_ability(
                        &mut commands, &ability_defs, id, target_entity,
                        active.caster, active.original_caster,
                        active.ability_slot, active.depth, tick, &server_query,
                    );
                }
                _ => {
                    warn!("Unhandled OnInput effect: {:?}", effect);
                }
            }
        }
    }
}
```

#### 5. System Schedule
**File**: `crates/protocol/src/lib.rs`

Add `apply_on_input_effects` to chain after `apply_on_end_effects`:
```rust
(
    ability::ability_activation,
    ability::update_active_abilities,
    ability::dispatch_effect_markers,
    ability::apply_on_cast_effects,
    ability::apply_while_active_effects,
    ability::apply_on_end_effects,
    ability::apply_on_input_effects,
    ability::ability_projectile_spawn,
).chain()
```

#### 6. RON — Punch Combo
**File**: `assets/abilities.ron`

Add punch2, punch3 definitions. Update punch to include OnInput for combo chaining:
```ron
"punch": (
    startup_ticks: 4,
    active_ticks: 20,
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

#### 7. Cleanup + Tests
Add `OnInputEffects` to `cleanup_effect_markers_on_removal`.
- Test: `on_input_effects_dispatched_during_active` — verify OnInputEffects present on Active ticks
- Test: `on_input_effects_removed_on_recovery` — verify removed on phase exit

### Success Criteria

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Workspace compiles: `cargo check-all`

#### Manual Verification:
- [ ] Pressing 1 during punch's Active window chains to punch2
- [ ] Pressing 1 again during punch2's Active window chains to punch3
- [ ] punch3 has recovery (can't chain further)
- [ ] Combo prediction works without desyncs

---

## Phase 6: Hitbox Entity Spawning + AreaOfEffect

### Overview

Replace melee's spatial query approach with spawned hitbox entities. Add `AreaOfEffect` variant that spawns sphere hitboxes. Unify hit detection: melee hitboxes, AoE hitboxes, and projectiles all use `CollidingEntities` + `OnHitEffects`.

### Changes Required

#### 1. AbilityEffect — AreaOfEffect Variant
**File**: `crates/protocol/src/ability.rs`

```rust
AreaOfEffect {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    target: EffectTarget,
    radius: f32,
},
```

#### 2. HitboxOf / ActiveAbilityHitboxes Relationship
**File**: `crates/protocol/src/ability.rs`

```rust
/// Relationship: hitbox entity belongs to an ActiveAbility entity.
#[derive(Component, Debug)]
#[relationship(relationship_target = ActiveAbilityHitboxes)]
pub struct HitboxOf(#[entities] pub Entity);

/// Relationship target: ActiveAbility's hitbox entities.
#[derive(Component, Debug, Default)]
#[relationship_target(relationship = HitboxOf, linked_spawn)]
pub struct ActiveAbilityHitboxes(Vec<Entity>);
```

#### 3. hitbox_collision_layers
**File**: `crates/protocol/src/hit_detection.rs`

```rust
pub fn hitbox_collision_layers() -> CollisionLayers {
    CollisionLayers::new(GameLayer::Hitbox, [GameLayer::Character])
}
```

#### 4. Spawn Hitbox Entities from apply_on_cast_effects

Replace `MeleeHitboxActive` marker insertion with hitbox entity spawning. For `Melee`:
```rust
AbilityEffect::Melee { .. } => {
    spawn_melee_hitbox(&mut commands, entity, active, on_hit_effects);
}
```

Where `spawn_melee_hitbox` creates an entity with:
- `Collider::cuboid(MELEE_HITBOX_HALF_EXTENTS)` — positioned at caster + facing * offset
- `Sensor`, `CollisionEventsEnabled`, `CollidingEntities::default()`
- `hitbox_collision_layers()`
- `OnHitEffects` cloned from ActiveAbility entity
- `HitboxOf(ability_entity)`
- `DisableRollback`

For `AreaOfEffect { radius, .. }`:
- Same pattern but `Collider::sphere(radius)` at caster position

`apply_on_cast_effects` needs access to caster `Position`/`Rotation` for hitbox positioning.

#### 5. Hitbox Position Tracking System

Melee hitboxes need to track the caster's position each tick:
```rust
pub fn update_hitbox_positions(
    hitbox_query: Query<(&HitboxOf, &mut Position), With<MeleeHitboxMarker>>,
    ability_query: Query<&ActiveAbility>,
    caster_query: Query<(&Position, &Rotation)>,
) {
    // Update melee hitbox position to caster.position + facing * offset
}
```

AoE hitboxes are static (spawned at caster position, don't move).

#### 6. Unified Hit Detection System

Replace `process_melee_hits` (spatial query) with collision-based detection:
```rust
pub fn process_hitbox_hits(
    mut commands: Commands,
    mut hitbox_query: Query<(
        &CollidingEntities,
        &OnHitEffects,
        &mut HitTargets,  // replaces MeleeHitTargets
        &Position,
    ), With<Sensor>>,
    mut target_query: Query<...>,
) {
    // Same CollidingEntities iteration as process_projectile_hits
    // but with HitTargets dedup (melee hitboxes persist, unlike bullets)
}
```

Note: projectiles still have their own detection because they despawn on hit. Hitboxes persist and use `HitTargets` for dedup.

#### 7. Remove Old Melee Components and Systems

Remove: `MeleeHitboxActive`, `MeleeHitTargets`, `ensure_melee_hit_targets`, `process_melee_hits` (spatial query version), melee-related constants (keep for hitbox sizing).

#### 8. Tests
- `melee_hitbox_entity_spawned` — verify entity with Collider + OnHitEffects spawned
- `aoe_hitbox_entity_spawned` — verify sphere collider spawned
- `hitbox_despawned_on_ability_end` — verify linked_spawn cleanup

### Success Criteria

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Workspace compiles: `cargo check-all`
- [x] Server builds and runs: `cargo server`
- [x] Client builds and runs: `cargo client`

#### Manual Verification:
- [ ] Melee punch still hits targets (entity-based detection)
- [ ] AreaOfEffect ability hits all entities within radius
- [ ] Hitbox entities despawn when ability ends
- [ ] No collision detection regressions

---

## Phase 7: Buff / Shield / Teleport

### Overview

Three independent effect variants. Can be implemented in any order.

### 7A: Teleport

Simplest of the three. Instant reposition in facing direction.

#### Changes Required

**File**: `crates/protocol/src/ability.rs`

Add to `AbilityEffect`:
```rust
Teleport { distance: f32 },
```

Handle in `apply_on_cast_effects`:
```rust
AbilityEffect::Teleport { distance } => {
    if let Ok((rotation, mut position)) = caster_pos_query.get_mut(active.caster) {
        let direction = facing_direction(rotation);
        position.0 += direction * *distance;
    }
}
```

`apply_on_cast_effects` needs access to caster `Position` + `Rotation` (mutable `Position` for teleport).

#### Tests
- `teleport_moves_caster` — verify caster Position changes by distance * facing

### 7B: Shield

Damage absorption. Insert component on caster, intercept damage before it reaches Health.

#### Changes Required

**File**: `crates/protocol/src/ability.rs`

Add to `AbilityEffect`:
```rust
Shield { absorb: f32 },
```

New component:
```rust
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveShield {
    pub remaining: f32,
}
```

Handle in `apply_on_cast_effects`:
```rust
AbilityEffect::Shield { absorb } => {
    commands.entity(active.caster).insert(ActiveShield { remaining: *absorb });
}
```

**File**: `crates/protocol/src/hit_detection.rs`

Update `apply_on_hit_effects` Damage handling to check for `ActiveShield`:
```rust
AbilityEffect::Damage { amount, target } => {
    let entity = resolve_on_hit_target(target, victim, on_hit);
    // Check shield first
    if let Ok(mut shield) = shield_query.get_mut(entity) {
        if shield.remaining >= *amount {
            shield.remaining -= *amount;
            continue; // fully absorbed
        }
        let overflow = *amount - shield.remaining;
        shield.remaining = 0.0;
        // Apply overflow to health
        if let Ok((_, _, mut health, invulnerable)) = target_query.get_mut(entity) {
            if invulnerable.is_none() {
                health.apply_damage(overflow);
            }
        }
    } else if let Ok((_, _, mut health, invulnerable)) = target_query.get_mut(entity) {
        if invulnerable.is_none() {
            health.apply_damage(*amount);
        }
    }
}
```

`apply_on_hit_effects` needs a `shield_query: &mut Query<&mut ActiveShield>` parameter. This ripples to `process_melee_hits` and `process_projectile_hits`.

Cleanup: remove `ActiveShield` when `remaining <= 0.0` or when the ability that granted it ends (via `OnEnd` trigger or manual cleanup).

**File**: `crates/protocol/src/lib.rs`

Register component:
```rust
app.register_component::<ActiveShield>().add_prediction();
```

#### Tests
- `shield_absorbs_damage` — apply Damage to entity with ActiveShield, verify Health unchanged and shield reduced
- `shield_overflow_damages_health` — damage exceeds shield, verify remainder reaches Health

### 7C: Buff

Temporary stat modifier. Needs tick-based expiry.

#### Changes Required

**File**: `crates/protocol/src/ability.rs`

Add to `AbilityEffect`:
```rust
Buff { stat: String, multiplier: f32, duration_ticks: u16, target: EffectTarget },
```

New component (on target entity):
```rust
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveBuffs(pub Vec<ActiveBuff>);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveBuff {
    pub stat: String,
    pub multiplier: f32,
    pub expires_tick: Tick,
}
```

Handle in `apply_on_cast_effects` (or whichever trigger context):
```rust
AbilityEffect::Buff { stat, multiplier, duration_ticks, target } => {
    let target_entity = resolve_caster_target(target, active);
    // Insert or append to ActiveBuffs
}
```

New expiry system:
```rust
pub fn expire_buffs(
    mut commands: Commands,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut query: Query<(Entity, &mut ActiveBuffs)>,
) {
    let tick = timeline.tick();
    for (entity, mut buffs) in &mut query {
        buffs.0.retain(|b| b.expires_tick > tick);
        if buffs.0.is_empty() {
            commands.entity(entity).remove::<ActiveBuffs>();
        }
    }
}
```

**File**: `crates/protocol/src/lib.rs`

Register:
```rust
app.register_component::<ActiveBuffs>().add_prediction();
```

Add `expire_buffs` to schedule.

Note: The stat query integration (how game systems read active buffs to modify behavior) depends on what stats exist. For now, `ActiveBuffs` is a data container. Systems that care about specific stats query `ActiveBuffs` and compute effective values. This is deferred until specific stats are needed.

#### Tests
- `buff_inserted_on_target` — verify ActiveBuffs component inserted with correct fields
- `buff_expires_after_duration` — verify buff removed when tick exceeds expires_tick

### Success Criteria (all of Phase 7)

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Workspace compiles: `cargo check-all`

#### Manual Verification:
- [ ] Teleport repositions character correctly
- [ ] Shield absorbs damage visually (health bar doesn't decrease)
- [ ] Buff component is present during duration, absent after
- [ ] No regressions in existing abilities

---

## Testing Strategy

### Unit Tests (crates/protocol/tests/ability_systems.rs):
- Phase transition tests (existing, updated for new signatures)
- OnHitEffects dispatch/cleanup tests
- OnEndEffects dispatch timing test
- OnInputEffects dispatch during Active test
- Sub-ability spawn + depth limit tests
- Shield absorption math tests
- Buff expiry tests
- Teleport position change test

### Manual Testing Steps:
1. Start server (`cargo server`) + client (`cargo client`)
2. Approach dummy target, press 1 (punch) — target takes damage and knockback
3. Press 2 (dash) — character moves forward
4. Press 3 (fireball) — projectile spawns, travels, hits target with damage/knockback
5. (Phase 5) Press 1 during punch active window — chains to punch2
6. (Phase 5) Press 1 again — chains to punch3 with stronger hit
7. (Phase 6) AoE ability hits multiple targets in radius
8. (Phase 7) Teleport moves character forward, Shield absorbs damage

## Performance Considerations

- `OnHitEffects` component is cloned per-hitbox/per-bullet. With typical 2-3 effects per ability, this is negligible.
- Hitbox entity spawning (Phase 6) adds entities but removes the per-tick spatial query overhead from melee.
- `expire_buffs` iterates all entities with `ActiveBuffs` each tick. With typical ~5 buffed entities, negligible.

## References

- Design doc: [doc/design/2026-02-13-ability-effect-primitives.md](../design/2026-02-13-ability-effect-primitives.md)
- Research (remaining work): [doc/research/2026-02-22-remaining-ability-effect-primitives.md](../research/2026-02-22-remaining-ability-effect-primitives.md)
- Research (lightyear patterns): [doc/research/2026-02-21-ability-effect-primitives-lightyear-hierarchy.md](../research/2026-02-21-ability-effect-primitives-lightyear-hierarchy.md)
- Foundation plan (Phase 1): [doc/plans/2026-02-21-ability-entity-foundation.md](2026-02-21-ability-entity-foundation.md)
