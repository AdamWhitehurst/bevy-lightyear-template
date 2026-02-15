# Hit Detection + Knockback Implementation Plan

## Overview

Implement general-purpose hit detection for melee and projectile abilities, plus knockback as the first hit-reactive effect. Enables sumo mode gameplay.

## Current State Analysis

- `AbilityEffect::Melee` exists but is a complete no-op — no hitbox, no collision, no effect
- Bullets have `Collider::sphere(0.25)` + `RigidBody::Kinematic` but nothing detects their collisions
- No collision layers exist — characters, bullets, and terrain all collide in the default layer
- No Health/Damage/Knockback components exist anywhere

### Key Discoveries:
- Ability marker dispatch pattern at [ability.rs:370-416](crates/protocol/src/ability.rs#L370-L416) — `dispatch_while_active_markers` (persistent) and `dispatch_on_cast_markers` (one-shot)
- `DashAbilityEffect` is re-inserted every tick during Active phase ([ability.rs:395-401](crates/protocol/src/ability.rs#L395-L401))
- Bullet spawn uses two-entity pattern with `DisableRollback` on bullets ([ability.rs:484-501](crates/protocol/src/ability.rs#L484-L501))
- `SpatialQuery` is already used for jump ground detection ([lib.rs:228-231](crates/protocol/src/lib.rs#L228-L231))
- `CharacterPhysicsBundle` at [lib.rs:58-75](crates/protocol/src/lib.rs#L58-L75) has no collision layers

## Desired End State

- Melee punch creates a hitbox during Active phase that detects overlapping characters
- Projectile bullets detect character hits and despawn on contact
- Both melee and projectile hits apply knockback impulse to targets
- Collision layers prevent nonsensical interactions (hitbox vs terrain, bullet vs bullet)
- Hit deduplication prevents multi-frame double-hits
- All hit detection runs as shared prediction (client + server)

### Verification:
1. Start server + client, walk to dummy target, punch → dummy gets knocked back
2. Fireball hits dummy → dummy gets knocked back, bullet despawns
3. Punch doesn't knock back the caster
4. Bullet doesn't hit the shooter
5. Multi-tick Active phase only hits the same target once

## What We're NOT Doing

- EffectTrigger refactor (`effect` → `effects: Vec<EffectTrigger>`)
- Health/Damage system
- Lag compensation (`LagCompensationSpatialQuery`)
- Grab, Pull, Shield, Buff, or other effect primitives
- Terrain collision for bullets (they already fly through terrain)
- Visual hit feedback (particles, animations, sounds)

## Implementation Approach

**Melee: `SpatialQuery::shape_intersections` instead of sensor entities.** While we initially discussed sensor entities, `SpatialQuery` is better for V1 because:
- No entity lifecycle to manage during lightyear rollback resimulation
- `MeleeHitboxActive` lives on the character (which IS rolled back via `ActiveAbility`)
- One-shot overlap test each tick — no `CollisionEventsEnabled`, no sensor spawn/despawn
- Upgrade path to sensor entities exists if we later need persistent multi-hitbox attacks

**Projectiles: `Sensor` + `CollidingEntities` on bullet entities.** Bullets already have `DisableRollback` and their own entity lifecycle, so sensor approach works cleanly here.

**Knockback: applied directly in hit processing systems.** No intermediate event — keeps rollback behavior simple.

---

## Phase 1: Collision Layers + Module Structure

### Overview
Define collision layers and apply them to all existing physics entities. Create the `hit_detection` module.

### Changes Required:

#### 1. New module: `crates/protocol/src/hit_detection.rs`

```rust
use avian3d::prelude::*;
use bevy::prelude::*;

#[derive(PhysicsLayer, Default)]
pub enum GameLayer {
    #[default]
    Default,
    Character,
    Hitbox,
    Projectile,
    Terrain,
}

/// Collision layer config for characters.
pub fn character_collision_layers() -> CollisionLayers {
    CollisionLayers::new(
        GameLayer::Character,
        [GameLayer::Character, GameLayer::Terrain, GameLayer::Hitbox, GameLayer::Projectile],
    )
}

/// Collision layer config for terrain.
pub fn terrain_collision_layers() -> CollisionLayers {
    CollisionLayers::new(GameLayer::Terrain, [GameLayer::Character])
}

/// Collision layer config for projectiles.
pub fn projectile_collision_layers() -> CollisionLayers {
    CollisionLayers::new(GameLayer::Projectile, [GameLayer::Character])
}
```

#### 2. Update `CharacterPhysicsBundle`
**File**: `crates/protocol/src/lib.rs`

Add `CollisionLayers` to the bundle:

```rust
#[derive(Bundle)]
pub struct CharacterPhysicsBundle {
    pub collider: Collider,
    pub rigid_body: RigidBody,
    pub locked_axes: LockedAxes,
    pub friction: Friction,
    pub collision_layers: CollisionLayers,
}

impl Default for CharacterPhysicsBundle {
    fn default() -> Self {
        Self {
            // ... existing fields ...
            collision_layers: hit_detection::character_collision_layers(),
        }
    }
}
```

#### 3. Update terrain chunk colliders
**File**: `crates/protocol/src/map.rs`

Add collision layers when inserting terrain colliders:

```rust
commands
    .entity(entity)
    .insert((collider, RigidBody::Static, hit_detection::terrain_collision_layers()));
```

#### 4. Register module
**File**: `crates/protocol/src/lib.rs`

Add `pub mod hit_detection;` and re-export.

#### 5. Spawn dummy target for testing
**File**: `crates/server/src/gameplay.rs`

Add a `Startup` system that spawns a static dummy character at a known position. This provides a hit target for testing with a single client.

```rust
/// Marker to distinguish dummy targets from player characters.
#[derive(Component)]
pub struct DummyTarget;

fn spawn_dummy_target(mut commands: Commands) {
    commands.spawn((
        Name::new("DummyTarget"),
        Position(Vec3::new(5.0, 30.0, 0.0)),
        Rotation::default(),
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
        CharacterPhysicsBundle::default(),
        ColorComponent(css::GRAY.into()),
        CharacterMarker,
        DummyTarget,
    ));
}
```

Register in `ServerGameplayPlugin::build`:
```rust
app.add_systems(Startup, spawn_dummy_target);
```

The dummy has `CharacterMarker` + `CharacterPhysicsBundle` so it participates in collision layers and is detected by hit systems. It has no `ActionState`, `AbilitySlots`, or `ControlledBy` — it just stands there and gets hit.

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all`
- [x] `cargo test-all`

#### Manual Verification:
- [x] `cargo server` + `cargo client` — characters still collide with terrain and each other
- [x] Movement and jumping still work normally
- [x] Dummy target is visible and standing at (5, 30, 0)

---

## Phase 2: Melee Hit Detection + Knockback

### Overview
Implement melee hitbox detection using `SpatialQuery::shape_intersections` and apply knockback impulse to hit targets.

### Changes Required:

#### 1. Update `AbilityEffect::Melee` with parameters
**File**: `crates/protocol/src/ability.rs`

```rust
pub enum AbilityEffect {
    Melee { knockback_force: f32 },
    Projectile { speed: f32, lifetime_ticks: u16 },
    Dash { speed: f32 },
}
```

#### 2. Add melee marker components
**File**: `crates/protocol/src/ability.rs`

```rust
/// Present during Active phase of a Melee ability. Removed on phase exit.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct MeleeHitboxActive {
    pub knockback_force: f32,
}

/// Tracks entities already hit during this melee active window.
/// Separate from MeleeHitboxActive to avoid overwrite on re-insert.
#[derive(Component, Clone, Debug, Default)]
pub struct MeleeHitTargets(pub EntityHashSet);
```

#### 3. Update dispatch functions
**File**: `crates/protocol/src/ability.rs`

`dispatch_while_active_markers` — add Melee branch:
```rust
fn dispatch_while_active_markers(commands: &mut Commands, entity: Entity, def: &AbilityDef) {
    match &def.effect {
        AbilityEffect::Dash { speed } => {
            commands.entity(entity).insert(DashAbilityEffect { speed: *speed });
        }
        AbilityEffect::Melee { knockback_force } => {
            commands.entity(entity).insert(MeleeHitboxActive {
                knockback_force: *knockback_force,
            });
        }
        _ => {}
    }
}
```

`remove_while_active_markers` — add Melee cleanup:
```rust
fn remove_while_active_markers(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).remove::<DashAbilityEffect>();
    commands.entity(entity).remove::<MeleeHitboxActive>();
    commands.entity(entity).remove::<MeleeHitTargets>();
}
```

`cleanup_effect_markers_on_removal` — add Melee markers:
```rust
pub fn cleanup_effect_markers_on_removal(trigger: On<Remove, ActiveAbility>, mut commands: Commands) {
    if let Ok(mut cmd) = commands.get_entity(trigger.entity) {
        cmd.remove::<DashAbilityEffect>();
        cmd.remove::<ProjectileSpawnAbilityEffect>();
        cmd.remove::<MeleeHitboxActive>();
        cmd.remove::<MeleeHitTargets>();
    }
}
```

#### 4. Add melee hit detection system
**File**: `crates/protocol/src/hit_detection.rs`

Constants:
```rust
const MELEE_HITBOX_OFFSET: f32 = 1.5;
const MELEE_HITBOX_HALF_EXTENTS: Vec3 = Vec3::new(0.75, 1.0, 0.5);
```

```rust
/// Insert MeleeHitTargets for characters that have MeleeHitboxActive but no targets yet.
pub fn ensure_melee_hit_targets(
    mut commands: Commands,
    query: Query<Entity, (With<MeleeHitboxActive>, Without<MeleeHitTargets>)>,
) {
    for entity in &query {
        commands.entity(entity).insert(MeleeHitTargets::default());
    }
}

/// Detect melee hits using one-shot spatial query each tick.
pub fn process_melee_hits(
    spatial_query: SpatialQuery,
    mut query: Query<
        (Entity, &MeleeHitboxActive, &mut MeleeHitTargets, &Position, &Rotation),
        With<CharacterMarker>,
    >,
    mut target_query: Query<(&Position, Forces), With<CharacterMarker>>,
) {
    for (entity, hitbox, mut hit_targets, pos, rot) in &mut query {
        let direction = facing_direction(rot);
        let hitbox_pos = pos.0 + direction * MELEE_HITBOX_OFFSET;

        let filter = SpatialQueryFilter {
            mask: GameLayer::Character.into(),
            excluded_entities: EntityHashSet::from_iter([entity]),
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
                continue; // already hit
            }
            apply_knockback(
                &mut target_query,
                target,
                pos.0,
                hitbox.knockback_force,
            );
        }
    }
}

fn apply_knockback(
    target_query: &mut Query<(&Position, Forces), With<CharacterMarker>>,
    target: Entity,
    source_pos: Vec3,
    force: f32,
) {
    let Ok((target_pos, mut forces)) = target_query.get_mut(target) else {
        return;
    };
    let horizontal = (target_pos.0 - source_pos).with_y(0.0);
    let direction = if horizontal.length() > 0.01 {
        (horizontal.normalize() + Vec3::Y * 0.3).normalize()
    } else {
        Vec3::Y
    };
    forces.apply_linear_impulse(direction * force);
}
```

#### 5. Register melee systems
**File**: `crates/protocol/src/lib.rs`

Add after the existing ability chain in `SharedGameplayPlugin`:
```rust
app.add_systems(
    FixedUpdate,
    (
        hit_detection::ensure_melee_hit_targets,
        hit_detection::process_melee_hits,
    )
        .chain()
        .after(ability::dispatch_effect_markers),
);
```

#### 6. Update abilities.ron
**File**: `assets/abilities.ron`

```ron
"punch": (
    startup_ticks: 4,
    active_ticks: 3,
    recovery_ticks: 6,
    cooldown_ticks: 16,
    steps: 3,
    step_window_ticks: 20,
    effect: Melee(
        knockback_force: 15.0,
    ),
),
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all`
- [x] `cargo test-all`

#### Manual Verification:
- [x] `cargo server` + `cargo client` — walk to dummy target and punch, dummy gets knocked back
- [x] Punching doesn't knock back the caster
- [x] 3-hit combo: each step can hit independently (hit_targets reset per step via marker removal/re-insertion)
- [x] Knockback direction is away from the caster

---

## Phase 3: Projectile Hit Detection + Knockback

### Overview
Add collision detection to bullet entities so fireballs hit characters and apply knockback.

### Changes Required:

#### 1. Update `AbilityEffect::Projectile` with knockback
**File**: `crates/protocol/src/ability.rs`

```rust
AbilityEffect::Projectile { speed: f32, lifetime_ticks: u16, knockback_force: f32 }
```

Propagate `knockback_force` through `AbilityProjectileSpawn` and `ProjectileSpawnAbilityEffect`.

#### 2. Add sensor + collision components to bullets
**File**: `crates/protocol/src/ability.rs` in `handle_ability_projectile_spawn`

Add to bullet entity spawn:
```rust
commands.spawn((
    // ... existing components ...
    Sensor,
    CollisionEventsEnabled,
    CollidingEntities::default(),
    hit_detection::projectile_collision_layers(),
    KnockbackForce(spawn_info.knockback_force),
    ProjectileOwner(spawn_info.shooter),
));
```

#### 3. Add projectile hit components
**File**: `crates/protocol/src/hit_detection.rs`

```rust
/// Knockback force stored on a hitbox or projectile entity.
#[derive(Component, Clone, Debug)]
pub struct KnockbackForce(pub f32);

/// Who shot this projectile (to prevent self-hits).
#[derive(Component, Clone, Debug)]
pub struct ProjectileOwner(pub Entity);
```

#### 4. Add projectile hit processing system
**File**: `crates/protocol/src/hit_detection.rs`

```rust
/// Detect projectile hits via CollidingEntities and apply knockback.
pub fn process_projectile_hits(
    mut commands: Commands,
    bullet_query: Query<
        (Entity, &CollidingEntities, &KnockbackForce, &ProjectileOwner, &Position),
        With<Sensor>,
    >,
    mut target_query: Query<(&Position, Forces), With<CharacterMarker>>,
) {
    for (bullet, colliding, knockback, owner, bullet_pos) in &bullet_query {
        for &target in colliding.iter() {
            if target == owner.0 {
                continue; // skip shooter
            }
            if target_query.get(target).is_err() {
                continue; // not a character
            }
            apply_knockback(&mut target_query, target, bullet_pos.0, knockback.0);
            commands.entity(bullet).try_despawn();
            break; // bullet hits one target
        }
    }
}
```

#### 5. Register projectile hit system
**File**: `crates/protocol/src/lib.rs`

Add to the hit detection system set:
```rust
app.add_systems(
    FixedUpdate,
    (
        hit_detection::ensure_melee_hit_targets,
        hit_detection::process_melee_hits,
        hit_detection::process_projectile_hits,
    )
        .chain()
        .after(ability::dispatch_effect_markers),
);
```

#### 6. Update abilities.ron
**File**: `assets/abilities.ron`

```ron
"fireball": (
    startup_ticks: 6,
    active_ticks: 2,
    recovery_ticks: 8,
    cooldown_ticks: 42,
    steps: 1,
    step_window_ticks: 0,
    effect: Projectile(
        speed: 20.0,
        lifetime_ticks: 192,
        knockback_force: 20.0,
    ),
),
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all`
- [x] `cargo test-all`

#### Manual Verification:
- [x] `cargo server` + `cargo client` — fireball hits dummy target, dummy gets knocked back
- [x] Fireball despawns on hit
- [x] Fireball doesn't hit the shooter
- [x] Fireball that misses still despawns after lifetime expires

---

## Testing Strategy

### Unit Tests:
- `MeleeHitTargets` deduplication: same entity not processed twice
- `apply_knockback` direction calculation: horizontal away from source with slight upward component
- Collision layer interactions: verify layer masks are correct

### Integration Tests:
- Melee ability enters Active phase → `MeleeHitboxActive` inserted → removed on Recovery
- Combo chain: hit_targets reset between steps (marker removed and re-inserted)

### Manual Testing Steps:
1. Walk to dummy target, punch — dummy knocked back
2. Punch combo all 3 steps on dummy — three knockback applications
3. Fireball from distance hits dummy — knockback + bullet despawn
4. Fireball shot at empty space — expires via lifetime, no crash
5. Self-hit impossible for both melee and projectile

## Performance Considerations

- `SpatialQuery::shape_intersections` runs one shape cast per character with `MeleeHitboxActive` per tick. With active_ticks=3 and few simultaneous melee users, this is negligible.
- `CollidingEntities` on bullets is maintained by avian's narrow phase — no extra cost beyond what avian already computes.
- Collision layers reduce broad-phase pair count by filtering irrelevant combinations.

## References

- Research: [doc/research/2026-02-13-hit-detection-system.md](doc/research/2026-02-13-hit-detection-system.md)
- Design (future scope): [doc/design/2026-02-13-ability-effect-primitives.md](doc/design/2026-02-13-ability-effect-primitives.md)
- Lightyear projectiles example: [git/lightyear/examples/projectiles/src/shared.rs:291-367](git/lightyear/examples/projectiles/src/shared.rs#L291-L367)
- Existing ability dispatch: [crates/protocol/src/ability.rs:370-416](crates/protocol/src/ability.rs#L370-L416)
- SpatialQuery usage for jump: [crates/protocol/src/lib.rs:228-231](crates/protocol/src/lib.rs#L228-L231)
