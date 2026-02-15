---
date: 2026-02-13T18:16:50-08:00
researcher: Claude
git_commit: 87990383498159c89729b5229116ce452a74d31e
branch: master
repository: bevy-lightyear-template
topic: "General-purpose hit detection system that supports abilities but is not tied to them"
tags: [research, codebase, hit-detection, collision, avian3d, abilities, lightyear, sensor]
status: complete
last_updated: 2026-02-13
last_updated_by: Claude
---

# Research: General-Purpose Hit Detection System

**Date**: 2026-02-13T18:16:50 PST
**Git Commit**: 8799038
**Branch**: master

## Research Question

How to implement a general-purpose hit detection system that supports abilities but is not necessarily tied to them.

## Summary

The codebase has an ability system with three effect types (`Melee`, `Projectile`, `Dash`), but **no hit detection exists**. `AbilityEffect::Melee` is defined but is a complete no-op at runtime — no hitbox spawning, no collision detection, no damage. The project uses avian3d for physics, which provides all the primitives needed: `Sensor` for trigger volumes, `CollisionStart`/`CollisionEnd` events, `SpatialQuery::shape_intersections` for overlap tests, and `ShapeCaster` for sweep detection. Lightyear's `fps` example demonstrates two hit detection patterns: prediction-based `Collisions` system param polling, and server-authoritative `SpatialQuery::cast_ray` with lag compensation.

## Detailed Findings

### 1. Current State: No Hit Detection Exists

**What's defined but unimplemented:**
- [ability.rs:36](crates/protocol/src/ability.rs#L36) — `AbilityEffect::Melee` variant exists
- [ability.rs:395-416](crates/protocol/src/ability.rs#L395-L416) — `dispatch_while_active_markers` handles only `Dash`; `dispatch_on_cast_markers` handles only `Projectile`. No branch for `Melee`.
- [ability_systems.rs:429](crates/protocol/tests/ability_systems.rs#L429) — Test asserts melee "should not change velocity", confirming it's intentionally a no-op for now.

**Components that do NOT exist anywhere in the codebase:**
`Hitbox`, `Hurtbox`, `HitDetection`, `DamageEvent`, `Health`, `Damage`, `MeleeAbilityEffect`

### 2. Avian3d Collision/Sensor APIs

The project uses avian3d from local source at `git/avian/`.

#### Sensor Component
[git/avian/src/collision/collider/mod.rs:396-429](git/avian/src/collision/collider/mod.rs#L396-L429)

```rust
#[derive(Reflect, Clone, Component, Debug, Default, PartialEq, Eq)]
pub struct Sensor;
```

Makes a collider detect overlaps without applying physics forces. Entities pass through sensor colliders.

#### CollisionStart / CollisionEnd Events
[git/avian/src/collision/collision_events.rs:169-189](git/avian/src/collision/collision_events.rs#L169-L189)

```rust
#[derive(EntityEvent, Message, Clone, Copy, Debug, PartialEq)]
pub struct CollisionStart {
    #[event_target]
    pub collider1: Entity,
    pub collider2: Entity,
    pub body1: Option<Entity>,
    pub body2: Option<Entity>,
}
```

Dual consumption modes:
- **MessageReader**: `MessageReader<CollisionStart>` for bulk processing in systems
- **Observer**: `On<CollisionStart>` for per-entity observers

**Requirement**: Entities must have `CollisionEventsEnabled` component to generate events ([collision_events.rs:292-297](git/avian/src/collision/collision_events.rs#L292-L297)).

#### Event Triggering Pipeline
[git/avian/src/collision/narrow_phase/mod.rs:308-377](git/avian/src/collision/narrow_phase/mod.rs#L308-L377)

Events fire in `CollisionEventSystems` set inside `PhysicsStepSystems::Finalize` (after solver). The `trigger_collision_events` system checks each contact pair — if either entity has `CollisionEventsEnabled`, an event is triggered with that entity as `collider1` (the `#[event_target]`).

#### CollidingEntities Component
[git/avian/src/collision/collider/mod.rs:608-652](git/avian/src/collision/collider/mod.rs#L608-L652)

```rust
#[derive(Reflect, Clone, Component, Debug, Default, Deref, DerefMut, PartialEq, Eq)]
pub struct CollidingEntities(pub EntityHashSet);
```

Live set of currently-overlapping entities. Must be added manually; updated automatically.

#### SpatialQuery Intersection Tests
[git/avian/src/spatial_query/system_param.rs:931-940](git/avian/src/spatial_query/system_param.rs#L931-L940)

```rust
pub fn shape_intersections(
    &self,
    shape: &Collider,
    shape_position: Vector,
    shape_rotation: RotationValue,
    filter: &SpatialQueryFilter,
) -> Vec<Entity>
```

One-shot overlap test — no persistent entity needed. Checks all colliders intersecting a given shape at a position. Useful for melee hitbox checks without spawning a sensor entity.

#### ShapeCaster Component
[git/avian/src/spatial_query/shape_caster.rs:63-144](git/avian/src/spatial_query/shape_caster.rs#L63-L144)

Persistent shape cast that runs every physics frame. Results in `ShapeHits` component. Useful for continuous sweep detection (e.g., a sword swing arc).

#### SpatialQueryFilter
[git/avian/src/spatial_query/query_filter.rs:35-39](git/avian/src/spatial_query/query_filter.rs#L35-L39)

```rust
pub struct SpatialQueryFilter {
    pub mask: LayerMask,
    pub excluded_entities: EntityHashSet,
}
```

Controls which colliders participate in queries. `LayerMask` enables collision layer filtering. `excluded_entities` can exclude the caster from their own hitbox.

### 3. Lightyear Hit Detection Patterns

#### Pattern A: Collisions System Param (Predicted, Client+Server)
[git/lightyear/examples/projectiles/src/shared.rs:291-367](git/lightyear/examples/projectiles/src/shared.rs#L291-L367)

The `projectiles` example uses avian's `Collisions` system param to poll contact pairs in `FixedUpdate`:

```rust
fn check_hits(
    mut commands: Commands,
    collisions: Collisions,
    bullet_q: Query<(&Bullet, &ColorComponent, &Position), Without<Player>>,
    player_q: Query<&Player>,
    mut hit_ev_writer: MessageWriter<BulletHitMessage>,
) {
    for contacts in collisions.iter() {
        if !contacts.is_sensor && contacts.manifolds.iter().any(|m| m.num_active_contacts() > 0) {
            if let Ok((bullet, col, bullet_pos)) = bullet_q.get(contacts.collider1) {
                // Skip self-hits
                if let Ok(owner) = player_q.get(contacts.collider2)
                    && bullet.owner == owner.client_id { continue; }
                commands.entity(contacts.collider1).prediction_despawn();
                hit_ev_writer.write(BulletHitMessage { ... });
            }
        }
    }
}
```

Key details:
- Runs on both client (predicted) and server
- Uses `prediction_despawn()` for bullet cleanup
- Writes `BulletHitMessage` (a lightyear message) for hit effects
- Checks `contacts.is_sensor` and active contact count

#### Pattern B: Server-Authoritative Ray/Shape Cast with Lag Compensation
[git/lightyear/examples/fps/src/server.rs:122-175](git/lightyear/examples/fps/src/server.rs#L122-L175)

The `fps` example uses two server-only hit detection approaches:

**Lag-compensated ray cast** (for interpolated targets):
```rust
fn compute_hit_lag_compensation(
    query: LagCompensationSpatialQuery,
    bullets: Query<(Entity, &PlayerId, &Position, &LinearVelocity, &ControlledBy), With<BulletMarker>>,
    client_query: Query<&InterpolationDelay, With<ClientOf>>,
) {
    if let Some(hit_data) = query.cast_ray(
        *delay,          // InterpolationDelay from client
        position.0,
        direction,
        BULLET_COLLISION_DISTANCE_CHECK,
        false,
        &mut SpatialQueryFilter::default(),
    ) { /* despawn bullet, increment score */ }
}
```

**Standard ray cast** (for predicted targets):
```rust
fn compute_hit_prediction(
    query: SpatialQuery,
    ...
) {
    if let Some(hit_data) = query.cast_ray_predicate(
        position.0, direction, distance, false,
        &SpatialQueryFilter::default(),
        &|entity| bot_query.get(entity).is_ok(),  // filter predicate
    ) { /* despawn bullet, increment score */ }
}
```

Key details:
- `LagCompensationSpatialQuery` rewinds collider positions to match what the client saw
- Requires `LagCompensationPlugin` and `LagCompensationHistory::default()` on target entities
- Runs in `PhysicsSchedule` in `LagCompensationSystems::Collisions` set
- No `Health`/`Damage` component in the example — just increments a `Score(u32)`

### 4. Trigger Volume Pattern (Sensor Entity)

From avian's sensor example ([git/avian/crates/avian2d/examples/sensor.rs:58-74](git/avian/crates/avian2d/examples/sensor.rs#L58-L74)):

```rust
commands.spawn((
    RigidBody::Static,
    Collider::rectangle(100.0, 100.0),
    Sensor,                       // pass-through collider
    CollisionEventsEnabled,       // enables CollisionStart/CollisionEnd
    CollidingEntities::default(), // live overlap tracking
));
```

### 5. Existing Design Document

[doc/design/2026-02-13-ability-effect-primitives.md](doc/design/2026-02-13-ability-effect-primitives.md) proposes:
- Refactoring `effect` → `effects: Vec<EffectTrigger>` with `OnCast`, `WhileActive`, `OnHit` triggers
- `Melee` as an `OnCast` trigger that spawns a hitbox
- `OnHit` trigger type that invokes effects when a hitbox connects (e.g., `Knockback`, `Pull`, `Grab`)
- This design document is **not yet implemented**

### 6. Current Ability System Architecture

The ability phase machine in [ability.rs](crates/protocol/src/ability.rs) uses the marker component pattern:
1. `ActiveAbility` component tracks phase (Startup → Active → Recovery)
2. `dispatch_effect_markers` inserts/removes marker components based on phase
3. Dedicated systems query markers (`DashAbilityEffect`, `ProjectileSpawnAbilityEffect`)
4. Markers are one-shot (consumed by the effect system) or duration-based (present during Active phase)

This pattern naturally extends to hit detection: a `MeleeHitboxMarker` (or similar) would be inserted during Active phase, and a hit detection system would query for it.

### 7. Two Approaches for Melee Hit Detection

**Approach A: Spawn a Sensor entity (persistent hitbox)**
- Spawn a child entity with `Sensor + Collider + CollisionEventsEnabled` during Active phase
- Use `CollisionStart` observer or `CollidingEntities` to detect overlaps
- Despawn when phase exits Active
- Works with avian's built-in narrow phase
- Natural for hitboxes that persist across multiple ticks (e.g., a spinning attack)

**Approach B: One-shot SpatialQuery (instant check)**
- Call `spatial_query.shape_intersections(shape, position, rotation, filter)` during Active phase
- No entity spawning, no cleanup
- Runs once per tick, returns `Vec<Entity>` of hits
- Natural for instantaneous checks (e.g., a single-frame punch)

Both approaches are "general-purpose" in that they detect collisions between any entities — not specific to abilities. An ability triggers the check, but the hit detection system itself operates on generic components.

## Code References

- [crates/protocol/src/ability.rs:36](crates/protocol/src/ability.rs#L36) — `AbilityEffect::Melee` (unimplemented)
- [crates/protocol/src/ability.rs:143-153](crates/protocol/src/ability.rs#L143-L153) — `DashAbilityEffect`, `ProjectileSpawnAbilityEffect` marker components
- [crates/protocol/src/ability.rs:370-431](crates/protocol/src/ability.rs#L370-L431) — `dispatch_effect_markers`, `dispatch_while_active_markers`, `dispatch_on_cast_markers`
- [crates/protocol/src/lib.rs:186-203](crates/protocol/src/lib.rs#L186-L203) — `SharedGameplayPlugin` ability system registration
- [crates/server/src/gameplay.rs:30-31](crates/server/src/gameplay.rs#L30-L31) — Movement excludes entities with `ActiveAbility`
- [git/avian/src/collision/collider/mod.rs:396](git/avian/src/collision/collider/mod.rs#L396) — `Sensor` component
- [git/avian/src/collision/collision_events.rs:169-297](git/avian/src/collision/collision_events.rs#L169-L297) — `CollisionStart`, `CollisionEnd`, `CollisionEventsEnabled`
- [git/avian/src/collision/collider/mod.rs:608](git/avian/src/collision/collider/mod.rs#L608) — `CollidingEntities`
- [git/avian/src/spatial_query/system_param.rs:931](git/avian/src/spatial_query/system_param.rs#L931) — `SpatialQuery::shape_intersections`
- [git/avian/src/spatial_query/shape_caster.rs:63](git/avian/src/spatial_query/shape_caster.rs#L63) — `ShapeCaster` component
- [git/lightyear/examples/projectiles/src/shared.rs:291-367](git/lightyear/examples/projectiles/src/shared.rs#L291-L367) — `Collisions` polling pattern
- [git/lightyear/examples/fps/src/server.rs:122-175](git/lightyear/examples/fps/src/server.rs#L122-L175) — Lag-compensated ray cast pattern

## Architecture Documentation

### Avian3d Hit Detection Primitives Available

| API | Type | Use Case |
|-----|------|----------|
| `Sensor` component | Marker | Makes collider pass-through (trigger volume) |
| `CollisionEventsEnabled` | Marker | Required for `CollisionStart`/`CollisionEnd` events |
| `CollidingEntities` | Component | Live set of currently-overlapping entities |
| `CollisionStart` / `CollisionEnd` | Event | Entity-targeted events via observer or MessageReader |
| `Collisions` | SystemParam | Query all touching contact pairs this frame |
| `SpatialQuery::shape_intersections` | Method | One-shot overlap test at a position |
| `SpatialQuery::cast_ray` / `cast_shape` | Method | Directional sweep test |
| `ShapeCaster` | Component | Persistent per-frame shape cast |
| `SpatialQueryFilter` | Struct | Layer mask + entity exclusion for all queries |
| `LagCompensationSpatialQuery` | SystemParam | Server-only rewound collision queries (lightyear) |

### Lightyear Prediction Considerations

- Hit detection systems in `FixedUpdate` run during rollback resimulation
- `Sensor` entities that are children of predicted entities participate in rollback if registered
- `DisableRollback` on hitbox entities prevents them from being resimulated (same pattern as projectile bullets)
- `prediction_despawn()` is the correct way to remove entities during prediction (schedules cleanup)
- Server-authoritative hit confirmation via `LagCompensationSpatialQuery` handles interpolated targets

## Historical Context (from doc/)

- [doc/design/2026-02-13-ability-effect-primitives.md](doc/design/2026-02-13-ability-effect-primitives.md) — Proposes composable `EffectTrigger` model with `OnHit` triggers for hit-reactive effects
- [doc/research/2026-02-07-ability-system-architecture.md](doc/research/2026-02-07-ability-system-architecture.md) — Original ability system research; covers phase machine, cooldowns, projectile spawning
- [doc/scratch/stats.md](doc/scratch/stats.md) — Stat system design with ability requirements
- [doc/research/2025-09-30-sonic-battle-chao-design-research.md](doc/research/2025-09-30-sonic-battle-chao-design-research.md) — Early research mentioning hypothetical `AttackHitbox`, `DamageEvent`

## Open Questions

1. **Sensor entity vs SpatialQuery for melee**: Should melee hitboxes be persistent sensor entities (good for multi-tick active windows, natural parent-child cleanup) or one-shot `shape_intersections` calls (simpler, no entity management)?
2. **Hit deduplication**: How to prevent the same hitbox from hitting the same target multiple times across ticks during the Active phase? (Common approach: `HitTargets(HashSet<Entity>)` component on the hitbox)
3. **Lag compensation for melee**: The fps example uses `LagCompensationSpatialQuery` for server hit confirmation. Is this needed for melee, or is the shared prediction model (both client and server run the same hitbox check) sufficient?
4. **Where does hit detection live?**: Should it be in `protocol` (shared, predicted) or split between server-authoritative confirmation and client prediction?
5. **Damage/Health system**: No `Health` or `Damage` components exist. Hit detection needs *something* to apply hits to. Is this a separate system or part of the hit detection design?
