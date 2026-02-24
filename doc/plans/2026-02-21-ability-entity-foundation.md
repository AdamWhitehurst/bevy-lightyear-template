# Ability Entity Foundation — Implementation Plan

## Overview

Refactor `ActiveAbility` from a component on the character entity to a prespawned/predicted entity. Migrate `AbilityDef.effect` to `AbilityDef.effects: Vec<EffectTrigger>`. Remove combo step mechanics. Migrate existing 3 abilities (punch, dash, fireball) to the new architecture.

## Current State Analysis

- `ActiveAbility` is a component on character entities, allowing one ability at a time
- `AbilityDef` has a single `effect: AbilityEffect` with 3 variants: `Melee`, `Projectile`, `Dash`
- Combo chaining uses `steps`, `step_window_ticks`, `step`, `total_steps`, `chain_input_received`
- Effect dispatch inserts typed markers on the character: `DashAbilityEffect`, `MeleeHitboxActive`, `ProjectileSpawnAbilityEffect`
- Movement globally suppressed via `Without<ActiveAbility>` in both [server](crates/server/src/gameplay.rs#L55) and [client](crates/client/src/gameplay.rs#L69)
- `PlayerId(PeerId)` already on character entities ([lib.rs:63](crates/protocol/src/lib.rs#L63), [gameplay.rs:158](crates/server/src/gameplay.rs#L158))

### Key Discoveries:
- Prespawn pattern at [ability.rs:504-516](crates/protocol/src/ability.rs#L504) with `Query<&ControlledBy>` server-detection is the model to follow
- `prediction_despawn()` is an extension method on `EntityCommands` via `PredictionDespawnCommandsExt` ([despawn.rs:64-69](git/lightyear/lightyear_prediction/src/despawn.rs#L64))
- Lightyear's `.add_map_entities()` chains on component registration; requires `MapEntities` trait impl ([registry.rs:486-493](git/lightyear/lightyear_replication/src/registry/registry.rs#L486))
- No `MapEntities` impls in project code yet; lightyear examples show the pattern ([protocol.rs:153-157](git/lightyear/examples/replication_groups/src/protocol.rs#L153))

## Desired End State

- `ActiveAbility` entities are prespawned with compound salt, predicted, and cleaned up via `prediction_despawn()`
- Multiple abilities can be active concurrently for the same caster
- `AbilityDef` uses `effects: Vec<EffectTrigger>` with `OnCast` and `WhileActive` triggers
- Effect markers live on `ActiveAbility` entities; effect systems resolve caster via `ActiveAbility.caster`
- Movement is no longer globally suppressed; dash uses `WhileActive(SetVelocity)` to override movement
- Punch is a single melee hit (combo restored in future OnInput work)

### Verification:
- All three abilities (punch, dash, fireball) function correctly at runtime
- Client-side prediction works without double-spawning or rollback glitches
- Multiple abilities can be activated concurrently (e.g. fireball then dash)

## What We're NOT Doing

- New effect primitives (Damage, ApplyForce, Buff, Shield, Teleport, Grab, AreaOfEffect, Summon)
- OnHit, OnEnd, OnInput trigger types
- Hitbox entity spawning (melee keeps spatial query approach for now)
- Combo chaining via OnInput (punch loses its 3-step combo)
- ActiveAbilityOf/ActiveAbilities custom relationship (deferred until hitbox entities needed)

## Behavior Changes

- **Punch loses combo**: Steps removed. Punch becomes a single melee hit.
- **Movement no longer globally suppressed**: Characters can move during abilities. Dash overrides movement via SetVelocity. Punch and fireball allow movement during cast.
- **Multiple simultaneous abilities**: Activation gated only by cooldowns.

## Implementation Approach

All changes are tightly coupled and must land together. Steps are ordered for clarity but form one atomic change.

---

## Phase 1: Data Model + Entity Architecture + Dispatch Migration

### Step 1: Type Definitions

**File**: `crates/protocol/src/ability.rs`

**1a. New enums**:

```rust
/// Specifies who receives an effect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum EffectTarget {
    Caster,
    Victim,
    OriginalCaster,
}

/// Controls when an effect fires.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum EffectTrigger {
    /// Fires once when ability enters Active phase.
    OnCast(AbilityEffect),
    /// Fires every tick during Active phase.
    WhileActive(AbilityEffect),
}
```

**1b. Modified `AbilityEffect`** — rename `Dash` to `SetVelocity`, add `EffectTarget`:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
pub enum AbilityEffect {
    Melee { knockback_force: f32, base_damage: f32 },
    Projectile { speed: f32, lifetime_ticks: u16, knockback_force: f32, base_damage: f32 },
    SetVelocity { speed: f32, target: EffectTarget },
}
```

**1c. Modified `AbilityDef`** — `effect` → `effects`, remove `steps`/`step_window_ticks`:

```rust
pub struct AbilityDef {
    pub startup_ticks: u16,
    pub active_ticks: u16,
    pub recovery_ticks: u16,
    pub cooldown_ticks: u16,
    pub effects: Vec<EffectTrigger>,
}
```

**1d. Modified `ActiveAbility`** — entity form with entity references:

```rust
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveAbility {
    pub def_id: AbilityId,
    pub caster: Entity,
    pub original_caster: Entity,
    pub target: Entity,
    pub phase: AbilityPhase,
    pub phase_start_tick: Tick,
    pub ability_slot: u8,
    pub depth: u8,
}
```

Add `MapEntities` impl:

```rust
impl MapEntities for ActiveAbility {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.caster = entity_mapper.get_mapped(self.caster);
        self.original_caster = entity_mapper.get_mapped(self.original_caster);
        self.target = entity_mapper.get_mapped(self.target);
    }
}
```

Remove `has_more_steps()` impl block.

**1e. New marker components** (on ActiveAbility entities):

```rust
/// One-shot: inserted on first Active tick; consumed by apply_on_cast_effects.
#[derive(Component)]
pub struct OnCastEffects(pub Vec<AbilityEffect>);

/// Persistent: present every Active tick; removed when phase exits Active.
#[derive(Component)]
pub struct WhileActiveEffects(pub Vec<AbilityEffect>);
```

**1f. Remove old types**:
- Remove `DashAbilityEffect` struct
- Remove `ProjectileSpawnAbilityEffect` struct (keep but move to ActiveAbility entities as `ProjectileSpawnEffect`)
- Keep `MeleeHitboxActive` (moves to ActiveAbility entities)
- Keep `MeleeHitTargets` (moves to ActiveAbility entities)

Actually — replace `ProjectileSpawnAbilityEffect` with a simpler marker since the data now comes from `OnCastEffects`:

```rust
/// One-shot: inserted by apply_on_cast_effects when processing Projectile.
/// Consumed by ability_projectile_spawn.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ProjectileSpawnEffect {
    pub speed: f32,
    pub lifetime_ticks: u16,
    pub knockback_force: f32,
    pub base_damage: f32,
}
```

### Step 2: ActiveAbility Entity Spawning

**File**: `crates/protocol/src/ability.rs`

Rewrite `ability_activation` to spawn entities:

```rust
pub fn ability_activation(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut query: Query<(
        Entity,
        &ActionState<PlayerActions>,
        &AbilitySlots,
        &mut AbilityCooldowns,
        &PlayerId,
    )>,
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();

    for (entity, action_state, slots, mut cooldowns, player_id) in &mut query {
        for (slot_idx, action) in ABILITY_ACTIONS.iter().enumerate() {
            if !action_state.just_pressed(action) {
                continue;
            }
            let Some(ref ability_id) = slots.0[slot_idx] else {
                continue;
            };
            let Some(def) = ability_defs.get(ability_id) else {
                warn!("Ability {:?} not found in defs", ability_id);
                continue;
            };
            if cooldowns.is_on_cooldown(slot_idx, tick, def.cooldown_ticks) {
                continue;
            }

            cooldowns.last_used[slot_idx] = Some(tick);

            let salt = (player_id.0.to_bits() as u64) << 32
                     | (slot_idx as u64) << 16
                     | 0u64; // depth = 0

            let mut cmd = commands.spawn((
                ActiveAbility {
                    def_id: ability_id.clone(),
                    caster: entity,
                    original_caster: entity,
                    target: entity,
                    phase: AbilityPhase::Startup,
                    phase_start_tick: tick,
                    ability_slot: slot_idx as u8,
                    depth: 0,
                },
                PreSpawned::default_with_salt(salt),
                Name::new("ActiveAbility"),
            ));

            // Server-only: add replication
            if let Ok(controlled_by) = server_query.get(entity) {
                cmd.insert((
                    Replicate::to_clients(NetworkTarget::All),
                    PredictionTarget::to_clients(NetworkTarget::All),
                    *controlled_by,
                ));
            }
        }
    }
}
```

Key changes:
- No `Without<ActiveAbility>` filter — multiple abilities can activate concurrently
- Spawns a new entity with `PreSpawned` compound salt (client_id + slot + depth)
- Server-only `Replicate`/`PredictionTarget`/`ControlledBy` via existing pattern
- Queries `&PlayerId` on the character for salt computation
- The `break` after first activation is removed — multiple abilities can activate per frame

### Step 3: Phase Management

**File**: `crates/protocol/src/ability.rs`

Rewrite `update_active_abilities` to query ActiveAbility entities (not characters):

```rust
pub fn update_active_abilities(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut query: Query<(Entity, &mut ActiveAbility)>,
) {
    let tick = timeline.tick();

    for (entity, mut active) in &mut query {
        let Some(def) = ability_defs.get(&active.def_id) else {
            warn!("Ability {:?} not found", active.def_id);
            commands.entity(entity).prediction_despawn();
            continue;
        };

        advance_ability_phase(&mut commands, entity, &mut active, def, tick);
    }
}
```

Simplify `advance_ability_phase` — remove all combo logic:

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

    if !phase_complete {
        return;
    }

    match active.phase {
        AbilityPhase::Startup => {
            active.phase = AbilityPhase::Active;
            active.phase_start_tick = tick;
        }
        AbilityPhase::Active => {
            active.phase = AbilityPhase::Recovery;
            active.phase_start_tick = tick;
        }
        AbilityPhase::Recovery => {
            commands.entity(entity).prediction_despawn();
        }
    }
}
```

Remove:
- `set_chain_input_received` function
- All combo-related fields and logic

Import `PredictionDespawnCommandsExt` from lightyear.

### Step 4: Trigger Dispatch

**File**: `crates/protocol/src/ability.rs`

Rewrite `dispatch_effect_markers` to work with `Vec<EffectTrigger>` on ActiveAbility entities:

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
        }
    }
}

fn dispatch_active_phase_markers(
    commands: &mut Commands,
    entity: Entity,
    active: &ActiveAbility,
    def: &AbilityDef,
    tick: Tick,
) {
    let first_active_tick = active.phase_start_tick == tick;

    if first_active_tick {
        let on_cast: Vec<AbilityEffect> = def.effects.iter().filter_map(|t| match t {
            EffectTrigger::OnCast(e) => Some(e.clone()),
            _ => None,
        }).collect();
        if !on_cast.is_empty() {
            commands.entity(entity).insert(OnCastEffects(on_cast));
        }
    }

    let while_active: Vec<AbilityEffect> = def.effects.iter().filter_map(|t| match t {
        EffectTrigger::WhileActive(e) => Some(e.clone()),
        _ => None,
    }).collect();
    if !while_active.is_empty() {
        commands.entity(entity).insert(WhileActiveEffects(while_active));
    }
}

fn remove_active_phase_markers(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).remove::<OnCastEffects>();
    commands.entity(entity).remove::<WhileActiveEffects>();
    commands.entity(entity).remove::<MeleeHitboxActive>();
    commands.entity(entity).remove::<MeleeHitTargets>();
}
```

### Step 5: Effect Systems

**File**: `crates/protocol/src/ability.rs`

**5a. `apply_on_cast_effects`** — processes `OnCastEffects`, spawns sub-markers:

```rust
pub fn apply_on_cast_effects(
    mut commands: Commands,
    query: Query<(Entity, &OnCastEffects)>,
) {
    for (entity, effects) in &query {
        for effect in &effects.0 {
            match effect {
                AbilityEffect::Melee { knockback_force, base_damage } => {
                    commands.entity(entity).insert(MeleeHitboxActive {
                        knockback_force: *knockback_force,
                        base_damage: *base_damage,
                    });
                }
                AbilityEffect::Projectile { speed, lifetime_ticks, knockback_force, base_damage } => {
                    commands.entity(entity).insert(ProjectileSpawnEffect {
                        speed: *speed,
                        lifetime_ticks: *lifetime_ticks,
                        knockback_force: *knockback_force,
                        base_damage: *base_damage,
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

**5b. `apply_while_active_effects`** — processes `WhileActiveEffects`, applies SetVelocity:

```rust
pub fn apply_while_active_effects(
    mut commands: Commands,
    query: Query<(Entity, &WhileActiveEffects, &ActiveAbility)>,
    mut caster_query: Query<(&Rotation, &mut LinearVelocity)>,
) {
    for (_entity, effects, active) in &query {
        for effect in &effects.0 {
            match effect {
                AbilityEffect::SetVelocity { speed, target } => {
                    let caster_entity = match target {
                        EffectTarget::Caster => active.caster,
                        EffectTarget::OriginalCaster => active.original_caster,
                        _ => {
                            warn!("SetVelocity target {:?} not supported yet", target);
                            continue;
                        }
                    };
                    if let Ok((rotation, mut velocity)) = caster_query.get_mut(caster_entity) {
                        let direction = facing_direction(rotation);
                        velocity.x = direction.x * speed;
                        velocity.z = direction.z * speed;
                    }
                }
                _ => {
                    warn!("Unhandled WhileActive effect: {:?}", effect);
                }
            }
        }
    }
}
```

Remove `ability_dash_effect` (replaced by `apply_while_active_effects`).

**5c. Rewrite `ability_projectile_spawn`** to query ActiveAbility entities:

```rust
pub fn ability_projectile_spawn(
    mut commands: Commands,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    query: Query<(Entity, &ProjectileSpawnEffect, &ActiveAbility)>,
    caster_query: Query<(&Position, &Rotation)>,
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();

    for (ability_entity, request, active) in &query {
        let Ok((position, rotation)) = caster_query.get(active.caster) else {
            warn!("Projectile spawn: caster {:?} missing Position/Rotation", active.caster);
            continue;
        };
        let direction = facing_direction(rotation);
        let spawn_info = AbilityProjectileSpawn {
            spawn_tick: tick,
            position: position.0 + direction * PROJECTILE_SPAWN_OFFSET,
            direction,
            speed: request.speed,
            lifetime_ticks: request.lifetime_ticks,
            knockback_force: request.knockback_force,
            base_damage: request.base_damage,
            ability_id: active.def_id.clone(),
            shooter: active.caster,
        };

        let salt = (active.ability_slot as u64) << 8 | (active.depth as u64);
        let mut cmd = commands.spawn((
            spawn_info,
            PreSpawned::default_with_salt(salt),
            Name::new("AbilityProjectileSpawn"),
        ));

        if let Ok(controlled_by) = server_query.get(active.caster) {
            cmd.insert((
                Replicate::to_clients(NetworkTarget::All),
                PredictionTarget::to_clients(NetworkTarget::All),
                *controlled_by,
            ));
        }

        commands.entity(ability_entity).remove::<ProjectileSpawnEffect>();
    }
}
```

### Step 6: Melee Hit Detection Migration

**File**: `crates/protocol/src/hit_detection.rs`

Melee hit detection now queries ActiveAbility entities instead of characters.

**6a. `ensure_melee_hit_targets`**:

```rust
pub fn ensure_melee_hit_targets(
    mut commands: Commands,
    query: Query<Entity, (With<MeleeHitboxActive>, Without<MeleeHitTargets>)>,
) {
    for entity in &query {
        commands.entity(entity).insert(MeleeHitTargets::default());
    }
}
```

No change needed — already queries by marker, not by `CharacterMarker`. Works for both character-hosted (old) and ability-entity-hosted (new) markers.

**6b. `process_melee_hits`**:

```rust
pub fn process_melee_hits(
    spatial_query: SpatialQuery,
    mut ability_query: Query<(
        &MeleeHitboxActive,
        &mut MeleeHitTargets,
        &ActiveAbility,
    )>,
    caster_query: Query<(&Position, &Rotation)>,
    mut target_query: Query<
        (&Position, &mut LinearVelocity, &mut Health, Option<&Invulnerable>),
        With<CharacterMarker>,
    >,
) {
    for (hitbox, mut hit_targets, active) in &mut ability_query {
        let Ok((pos, rot)) = caster_query.get(active.caster) else {
            continue;
        };
        let direction = facing_direction(rot);
        let hitbox_pos = pos.0 + direction * MELEE_HITBOX_OFFSET;

        let filter = SpatialQueryFilter {
            mask: GameLayer::Character.into(),
            excluded_entities: EntityHashSet::from_iter([active.caster]),
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
            if !hit_targets.0.insert(target) {
                continue;
            }
            apply_hit(&mut target_query, target, pos.0, hitbox.knockback_force, hitbox.base_damage);
        }
    }
}
```

Key change: queries `ActiveAbility` entities with `MeleeHitboxActive`, resolves caster position via `active.caster`. Excludes `active.caster` from spatial query filter (not `entity`).

### Step 7: Movement Changes

**File**: `crates/server/src/gameplay.rs`

Remove `Without<ActiveAbility>` from `handle_character_movement`:

```rust
fn handle_character_movement(
    time: Res<Time>,
    spatial_query: SpatialQuery,
    mut query: Query<
        (Entity, &ActionState<PlayerActions>, &ComputedMass, &Position, Forces),
        With<CharacterMarker>,
    >,
) { /* ... unchanged body ... */ }
```

**File**: `crates/client/src/gameplay.rs`

Same removal:

```rust
fn handle_character_movement(
    time: Res<Time>,
    spatial_query: SpatialQuery,
    mut query: Query<
        (Entity, &ActionState<PlayerActions>, &ComputedMass, &Position, Forces),
        (With<Predicted>, With<CharacterMarker>),
    >,
) { /* ... unchanged body ... */ }
```

Add explicit ordering so movement runs before ability effects (SetVelocity overrides movement):

**File**: `crates/protocol/src/lib.rs`

See Step 8 for full schedule.

### Step 8: System Schedule + Registration

**File**: `crates/protocol/src/lib.rs`

**8a. Lightyear registration** — add `MapEntities`, rename re-exports:

```rust
// In ProtocolPlugin::build:
app.register_component::<ActiveAbility>()
    .add_prediction()
    .add_map_entities();
```

Remove `PlayerId` from plain `register_component` — it's already registered without prediction, which is correct (replicate-only, no rollback).

**8b. System schedule** — updated chain:

```rust
// In SharedGameplayPlugin::build:

app.add_systems(
    FixedUpdate,
    (
        ability::ability_activation,
        ability::update_active_abilities,
        ability::dispatch_effect_markers,
        ability::apply_on_cast_effects,
        ability::ability_projectile_spawn,
        ability::apply_while_active_effects,
    )
        .chain()
        .after(handle_character_movement_label)
        .run_if(ready.clone()),
);

app.add_systems(
    FixedUpdate,
    (
        hit_detection::ensure_melee_hit_targets,
        hit_detection::process_melee_hits,
        hit_detection::process_projectile_hits,
    )
        .chain()
        .after(ability::apply_on_cast_effects)
        .run_if(ready.clone()),
);
```

Note: `handle_character_movement` is registered in server/client crates, not protocol. We need an ordering anchor. Options:
- Export a system set label from protocol for movement systems
- Or simply ensure movement systems use `.before(ability::ability_activation)` in server/client crates

The simpler approach: add `.before(ability::ability_activation)` to the movement system registrations in server and client crates.

**8c. Re-exports** — update `pub use` in `lib.rs`:

Remove: `DashAbilityEffect`, `ProjectileSpawnAbilityEffect`
Add: `EffectTarget`, `EffectTrigger`, `OnCastEffects`, `WhileActiveEffects`, `ProjectileSpawnEffect`

**8d. Observer update** — `cleanup_effect_markers_on_removal`:

```rust
pub fn cleanup_effect_markers_on_removal(
    trigger: On<Remove, ActiveAbility>,
    mut commands: Commands,
) {
    if let Ok(mut cmd) = commands.get_entity(trigger.entity) {
        cmd.remove::<OnCastEffects>();
        cmd.remove::<WhileActiveEffects>();
        cmd.remove::<ProjectileSpawnEffect>();
        cmd.remove::<MeleeHitboxActive>();
        cmd.remove::<MeleeHitTargets>();
    }
}
```

### Step 9: RON Migration

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
                OnCast(Melee(
                    knockback_force: 5.0,
                    base_damage: 10.0,
                )),
            ],
        ),
        "dash": (
            startup_ticks: 2,
            active_ticks: 8,
            recovery_ticks: 4,
            cooldown_ticks: 32,
            effects: [
                WhileActive(SetVelocity(
                    speed: 15.0,
                    target: Caster,
                )),
            ],
        ),
        "fireball": (
            startup_ticks: 6,
            active_ticks: 2,
            recovery_ticks: 8,
            cooldown_ticks: 42,
            effects: [
                OnCast(Projectile(
                    speed: 20.0,
                    lifetime_ticks: 192,
                    knockback_force: 8.0,
                    base_damage: 25.0,
                )),
            ],
        ),
    },
)
```

### Step 10: Test Updates

**File**: `crates/protocol/tests/ability_systems.rs`

All tests need updating for entity-based `ActiveAbility`. Key changes:

- `test_defs()` — remove `steps`/`step_window_ticks`, use `effects: vec![...]`
- `spawn_character()` — add `PlayerId(PeerId::default())` (or a test PeerId)
- Tests that insert `ActiveAbility` on characters must instead spawn a separate entity
- Remove combo-specific tests: `combo_chain_advances_step`, `combo_window_expires`
- `activation_blocked_by_active` — remove or convert (multiple abilities now allowed)
- `dash_applies_velocity_active` — needs ActiveAbility entity with WhileActiveEffects
- Phase transition tests — query ActiveAbility entities instead of characters

The `ability_activation` system now spawns entities, so tests must check for spawned entities rather than components on the character.

### Step 11: Cleanup

- Remove `set_chain_input_received` function
- Remove `has_more_steps` method
- Remove `ability_dash_effect` function
- Remove `DashAbilityEffect` struct
- Remove `dispatch_while_active_markers`, `dispatch_on_cast_markers`, `remove_while_active_markers` helper functions
- Update `Cargo.toml` if any new dependencies needed (likely none)
- Update README.md if ability system documentation is affected

---

## Success Criteria

### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Workspace compiles: `cargo check-all`
- [ ] Server builds and runs: `cargo server`
- [ ] Client builds and runs: `cargo client -c 1`

### Manual Verification:
- [ ] Punch activates and hits target (single hit, no combo)
- [ ] Dash activates and moves character at speed for 8 ticks
- [ ] Fireball spawns projectile that travels and hits with damage/knockback
- [ ] Character can move during punch and fireball casting
- [ ] Multiple abilities can be activated in rapid succession
- [ ] Client prediction works without visible double-spawns or rubber-banding
- [ ] Two clients can simultaneously use abilities without prespawn hash collisions

## Performance Considerations

- `dispatch_effect_markers` now iterates `Vec<EffectTrigger>` each tick per active ability. With 3 effects per ability and ~10 concurrent abilities max, this is negligible.
- Spatial query for melee hits now resolves caster position via an extra query. One additional ECS lookup per active melee ability per tick — negligible.

## References

- Design doc: [doc/design/2026-02-13-ability-effect-primitives.md](doc/design/2026-02-13-ability-effect-primitives.md)
- Research doc: [doc/research/2026-02-21-ability-effect-primitives-lightyear-hierarchy.md](doc/research/2026-02-21-ability-effect-primitives-lightyear-hierarchy.md)
- Existing prespawn pattern: [ability.rs:504-516](crates/protocol/src/ability.rs#L504)
- Lightyear `prediction_despawn`: [despawn.rs:64-69](git/lightyear/lightyear_prediction/src/despawn.rs#L64)
- Lightyear `MapEntities` example: [replication_groups/protocol.rs:153-157](git/lightyear/examples/replication_groups/src/protocol.rs#L153)
- Lightyear `add_map_entities` API: [registry.rs:486-493](git/lightyear/lightyear_replication/src/registry/registry.rs#L486)
