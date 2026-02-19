# Health, Respawn & Billboard Health Bar Implementation Plan

## Overview

Add a server-authoritative `Health` component with `base_damage` on abilities, instant respawn at `RespawnPoint` entities with post-respawn invulnerability, and client-side billboard health bars above all characters.

## Current State Analysis

- Hit detection exists but only applies knockback (`hit_detection.rs:130-146`)
- `AbilityEffect::Melee` and `Projectile` carry `knockback_force` but no damage
- No health, death, respawn, or world-space UI systems exist
- UI is screen-space only (`crates/ui/`)
- Single `Camera3d` with offset `(0, 9, -18)` following player (`render/lib.rs:98-112`)
- Characters spawn at `Position(Vec3::new(x, 30.0, z))` on server (`server/gameplay.rs:85-107`)
- DummyTarget spawns at `(3.0, 30.0, 0.0)` (`server/gameplay.rs:20-33`)

### Key Discoveries:
- `MeleeHitboxActive` already carries `knockback_force` — adding `base_damage` follows the same pattern
- `KnockbackForce` component on projectiles carries force — needs a parallel `DamageAmount` component
- Hit detection runs in shared code (both server and client for prediction) — Health should be predicted so damage applies naturally in both contexts, same as knockback
- `add_character_cosmetics` in `render/lib.rs:73-96` is the natural place to also spawn health bar children

## Desired End State

- Abilities with `Melee` or `Projectile` effects deal `base_damage` to target `Health`
- Characters and DummyTargets spawn with `Health { current: 100.0, max: 100.0 }`
- When `Health.current <= 0`, server instantly resets position to nearest `RespawnPoint` and restores full health
- 1-second (64 tick) invulnerability after respawn
- All characters display a billboard health bar (two overlapping quads) above their heads
- Health bar foreground scales with `current / max`

### Verification:
- `cargo server` + two `cargo client` instances
- Hit a character with punch/fireball, observe health bar decrease
- Reduce health to 0, observe instant respawn at RespawnPoint
- Confirm invulnerability prevents damage for ~1 second after respawn
- DummyTarget takes damage and respawns

## What We're NOT Doing

- Vitality stat integration (flat 100 HP for now)
- Death animation or delay
- Kill/death tracking or scoring
- Damage numbers or hit feedback UI
- Health regeneration
- Dash damage

## Implementation Approach

Damage values flow through the same marker-component pattern as knockback: ability definitions → effect markers → hit detection. Health is **predicted with rollback** — hit detection already runs in shared code and knockback is predicted, so health changes are a direct consequence of the same predicted hit events. This means damage application works identically on server and client with no special gating. The server remains authoritative; mispredictions get corrected via rollback alongside position/velocity. Respawn and invulnerability are server-only systems.

---

## Phase 1: Health Component & Damage Pipeline

### Overview
Add `Health` component, `base_damage` to ability effects, wire damage through markers into hit detection.

### Changes Required:

#### 1. Health component
**File**: `crates/protocol/src/lib.rs`

Add after `DummyTarget` component (line 63):
```rust
/// Health component. Predicted with rollback — hit detection is shared code
/// so damage must apply on both server and client for consistent prediction.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    pub fn new(max: f32) -> Self {
        Self { current: max, max }
    }

    pub fn apply_damage(&mut self, damage: f32) {
        self.current = (self.current - damage).max(0.0);
    }

    pub fn is_dead(&self) -> bool {
        self.current <= 0.0
    }

    pub fn restore_full(&mut self) {
        self.current = self.max;
    }
}
```

Register in `ProtocolPlugin::build` (after `CharacterMarker`):
```rust
app.register_component::<Health>().add_prediction();
```

Add to `pub use` exports and update `CharacterMarker` spawn bundles.

#### 2. Invulnerable marker
**File**: `crates/protocol/src/lib.rs`

```rust
/// Post-respawn invulnerability. Prevents damage while present.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Invulnerable {
    pub expires_at: Tick,
}
```

Register in `ProtocolPlugin::build` (predicted so hit detection can check it on client):
```rust
app.register_component::<Invulnerable>().add_prediction();
```

#### 3. Add `base_damage` to AbilityEffect variants
**File**: `crates/protocol/src/ability.rs`

Update the `AbilityEffect` enum (line 36):
```rust
pub enum AbilityEffect {
    Melee { knockback_force: f32, base_damage: f32 },
    Projectile { speed: f32, lifetime_ticks: u16, knockback_force: f32, base_damage: f32 },
    Dash { speed: f32 },
}
```

#### 4. Pipe `base_damage` through effect markers
**File**: `crates/protocol/src/ability.rs`

Update `MeleeHitboxActive` (line 158):
```rust
pub struct MeleeHitboxActive {
    pub knockback_force: f32,
    pub base_damage: f32,
}
```

Update `dispatch_while_active_markers` (line 414) to pass `base_damage`:
```rust
AbilityEffect::Melee { knockback_force, base_damage } => {
    commands.entity(entity).insert(MeleeHitboxActive {
        knockback_force: *knockback_force,
        base_damage: *base_damage,
    });
}
```

Update `ProjectileSpawnAbilityEffect` (line 151) to include `base_damage`:
```rust
pub struct ProjectileSpawnAbilityEffect {
    pub speed: f32,
    pub lifetime_ticks: u16,
    pub knockback_force: f32,
    pub base_damage: f32,
}
```

Update `dispatch_on_cast_markers` (line 430):
```rust
AbilityEffect::Projectile { speed, lifetime_ticks, knockback_force, base_damage } => {
    commands.entity(entity).insert(ProjectileSpawnAbilityEffect {
        speed: *speed,
        lifetime_ticks: *lifetime_ticks,
        knockback_force: *knockback_force,
        base_damage: *base_damage,
    });
}
```

Update `AbilityProjectileSpawn` (line 169) to include `base_damage`.

#### 5. Add DamageAmount to projectile bullets
**File**: `crates/protocol/src/hit_detection.rs`

Add component:
```rust
/// Damage stored on a projectile entity.
#[derive(Component, Clone, Debug)]
pub struct DamageAmount(pub f32);
```

**File**: `crates/protocol/src/ability.rs`

In `handle_ability_projectile_spawn` (line 518), add `DamageAmount(spawn_info.base_damage)` to the bullet spawn.

#### 6. Wire damage into hit detection
**File**: `crates/protocol/src/hit_detection.rs`

Since Health is predicted, damage applies in shared code just like knockback. Add `&mut Health` and `Option<&Invulnerable>` to the target query.

Refactor hit application into an `apply_hit` function that handles both knockback and damage:

```rust
fn apply_hit(
    target_query: &mut Query<
        (&Position, &mut LinearVelocity, &mut Health, Option<&Invulnerable>),
        With<CharacterMarker>,
    >,
    target: Entity,
    source_pos: Vec3,
    knockback_force: f32,
    damage: f32,
) {
    let Ok((target_pos, mut velocity, mut health, invulnerable)) = target_query.get_mut(target) else {
        return;
    };
    let horizontal = (target_pos.0 - source_pos).with_y(0.0);
    let direction = if horizontal.length() > 0.01 {
        (horizontal.normalize() + Vec3::Y * 0.3).normalize()
    } else {
        Vec3::Y
    };
    velocity.0 += direction * knockback_force;
    if invulnerable.is_none() {
        health.apply_damage(damage);
    }
}
```

Update `process_melee_hits` target query to `(&Position, &mut LinearVelocity, &mut Health, Option<&Invulnerable>)` and call `apply_hit` instead of `apply_knockback`, passing `hitbox.base_damage`.

Update `process_projectile_hits` target query similarly, reading `DamageAmount` from the bullet and passing it to `apply_hit`. Projectiles always despawn on contact with a character — invulnerable targets consume the bullet but take no damage (knockback still applies via `apply_hit`).

Remove the old `apply_knockback` function.

#### 7. Update abilities.ron
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
            effect: Melee(
                knockback_force: 5.0,
                base_damage: 10.0,
            ),
        ),
        "dash": (
            startup_ticks: 2,
            active_ticks: 8,
            recovery_ticks: 4,
            cooldown_ticks: 32,
            steps: 1,
            step_window_ticks: 0,
            effect: Dash(
                speed: 15.0,
            ),
        ),
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
                knockback_force: 8.0,
                base_damage: 25.0,
            ),
        ),
    },
)
```

#### 8. Add Health to character spawns
**File**: `crates/server/src/gameplay.rs`

Add `Health::new(100.0)` to both `handle_connected` spawn (line 85) and `spawn_dummy_target` (line 21).

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` passes
- [x] `cargo server` starts without errors

#### Manual Verification:
- [x] Server logs show no warnings about Health
- [x] Abilities.ron loads correctly (check startup logs)

---

## Phase 2: Death & Respawn

### Overview
Server-side death detection, RespawnPoint entities, instant respawn with invulnerability.

### Changes Required:

#### 1. RespawnPoint component
**File**: `crates/protocol/src/lib.rs`

```rust
/// Marks a respawn location. Server-only, not replicated.
#[derive(Component, Clone, Debug)]
pub struct RespawnPoint;
```

No replication registration needed — server-only.

#### 2. Spawn default RespawnPoint on server
**File**: `crates/server/src/gameplay.rs`

Add to `ServerGameplayPlugin::build`:
```rust
app.add_systems(Startup, spawn_respawn_points);
```

```rust
fn spawn_respawn_points(mut commands: Commands) {
    commands.spawn((
        RespawnPoint,
        Position(Vec3::new(0.0, 30.0, 0.0)),
    ));
}
```

#### 3. Death detection & respawn system (server-only)
**File**: `crates/server/src/gameplay.rs`

```rust
fn check_death_and_respawn(
    mut commands: Commands,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut dead_query: Query<(Entity, &mut Health, &mut Position, &mut LinearVelocity), With<CharacterMarker>>,
    respawn_query: Query<&Position, (With<RespawnPoint>, Without<CharacterMarker>)>,
) {
    let tick = timeline.tick();
    for (entity, mut health, mut position, mut velocity) in &mut dead_query {
        if !health.is_dead() { continue; }

        let respawn_pos = find_nearest_respawn_point(&position, &respawn_query);
        position.0 = respawn_pos;
        velocity.0 = Vec3::ZERO;
        health.restore_full();
        commands.entity(entity).insert(Invulnerable {
            expires_at: tick + 64, // 1 second at 64hz
        });
    }
}

fn find_nearest_respawn_point(
    current_pos: &Position,
    respawn_query: &Query<&Position, (With<RespawnPoint>, Without<CharacterMarker>)>,
) -> Vec3 {
    respawn_query
        .iter()
        .min_by(|a, b| {
            a.0.distance_squared(current_pos.0)
                .partial_cmp(&b.0.distance_squared(current_pos.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| p.0)
        .unwrap_or(Vec3::new(0.0, 30.0, 0.0))
}
```

Register in `ServerGameplayPlugin::build`:
```rust
app.add_systems(
    FixedUpdate,
    check_death_and_respawn
        .after(hit_detection::process_projectile_hits),
);
```

#### 4. Invulnerability expiry (server-only)
**File**: `crates/server/src/gameplay.rs`

```rust
fn expire_invulnerability(
    mut commands: Commands,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    query: Query<(Entity, &Invulnerable)>,
) {
    let tick = timeline.tick();
    for (entity, invuln) in &query {
        if tick >= invuln.expires_at {
            commands.entity(entity).remove::<Invulnerable>();
        }
    }
}
```

Register in `ServerGameplayPlugin::build`:
```rust
app.add_systems(FixedUpdate, expire_invulnerability);
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` passes
- [x] `cargo server` starts without errors

#### Manual Verification:
- [ ] Hit DummyTarget repeatedly until health reaches 0 — it teleports to respawn point with full health
- [ ] Hit a player character to 0 — instant respawn at RespawnPoint
- [ ] Immediately after respawn, attacks deal no damage for ~1 second
- [ ] After invulnerability expires, damage works normally again

---

## Phase 3: Billboard Health Bar

### Overview
Client-side health bar rendered as two child mesh quads (background + foreground) above each character, with a billboard system to face the camera.

### Changes Required:

#### 1. Health bar components
**File**: `crates/render/src/lib.rs`

```rust
/// Marker for the health bar root entity (child of character).
#[derive(Component)]
struct HealthBarRoot;

/// Marker for the foreground (colored) bar that scales with HP.
#[derive(Component)]
struct HealthBarForeground;

/// Marker for billboard entities that always face the camera.
#[derive(Component)]
struct Billboard;
```

#### 2. Spawn health bar children
**File**: `crates/render/src/lib.rs`

In `add_character_cosmetics`, after inserting `Mesh3d` + `MeshMaterial3d`, spawn health bar children:

```rust
// Health bar constants
const HEALTH_BAR_WIDTH: f32 = 1.5;
const HEALTH_BAR_HEIGHT: f32 = 0.15;
const HEALTH_BAR_Y_OFFSET: f32 = 2.5; // above capsule top

let bg_mesh = meshes.add(Plane3d::new(Vec3::Z, Vec2::new(HEALTH_BAR_WIDTH / 2.0, HEALTH_BAR_HEIGHT / 2.0)));
let fg_mesh = meshes.add(Plane3d::new(Vec3::Z, Vec2::new(HEALTH_BAR_WIDTH / 2.0, HEALTH_BAR_HEIGHT / 2.0)));
let bg_material = materials.add(StandardMaterial {
    base_color: Color::srgba(0.2, 0.2, 0.2, 0.8),
    unlit: true,
    alpha_mode: AlphaMode::Blend,
    ..default()
});
let fg_material = materials.add(StandardMaterial {
    base_color: Color::srgb(0.1, 0.9, 0.1),
    unlit: true,
    alpha_mode: AlphaMode::Blend,
    ..default()
});

commands.entity(entity).with_children(|parent| {
    parent.spawn((
        HealthBarRoot,
        Billboard,
        Transform::from_translation(Vec3::Y * HEALTH_BAR_Y_OFFSET),
    )).with_children(|bar| {
        // Background
        bar.spawn((
            Mesh3d(bg_mesh),
            MeshMaterial3d(bg_material),
            Transform::from_translation(Vec3::Z * -0.01), // slightly behind
        ));
        // Foreground
        bar.spawn((
            HealthBarForeground,
            Mesh3d(fg_mesh),
            MeshMaterial3d(fg_material),
            Transform::default(),
        ));
    });
});
```

#### 3. Billboard system
**File**: `crates/render/src/lib.rs`

```rust
fn billboard_face_camera(
    camera_query: Query<&GlobalTransform, With<Camera3d>>,
    mut billboard_query: Query<(&GlobalTransform, &mut Transform), With<Billboard>>,
) {
    let Ok(camera_gt) = camera_query.single() else { return };
    let camera_pos = camera_gt.translation();
    for (global_transform, mut transform) in &mut billboard_query {
        let billboard_pos = global_transform.translation();
        let direction = (camera_pos - billboard_pos).with_y(0.0);
        if direction.length_squared() > 0.001 {
            transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction.normalize());
        }
    }
}
```

Note: Using `NEG_Z` as forward since `Plane3d::new(Vec3::Z, ...)` faces +Z. The rotation arc aligns the quad toward the camera while staying upright (no Y component in direction).

#### 4. Health bar update system
**File**: `crates/render/src/lib.rs`

```rust
fn update_health_bars(
    health_query: Query<&Health, With<CharacterMarker>>,
    bar_root_query: Query<(&ChildOf, &Children), With<HealthBarRoot>>,
    mut fg_query: Query<&mut Transform, With<HealthBarForeground>>,
) {
    for (child_of, children) in &bar_root_query {
        let Ok(health) = health_query.get(child_of.parent()) else { continue };
        let ratio = (health.current / health.max).clamp(0.0, 1.0);
        for &child in children.iter() {
            if let Ok(mut transform) = fg_query.get_mut(child) {
                transform.scale.x = ratio;
                // Shift left so bar drains from right to left
                let offset = (1.0 - ratio) * HEALTH_BAR_WIDTH * -0.5;
                transform.translation.x = offset;
            }
        }
    }
}
```

Note: `ChildOf` is Bevy 0.17's component for accessing a child's parent (renamed from `Parent` in 0.15).

#### 5. Register systems
**File**: `crates/render/src/lib.rs`

Update `RenderPlugin::build` (line 25):
```rust
app.add_systems(Update, (add_character_cosmetics, follow_player, billboard_face_camera, update_health_bars));
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` passes
- [ ] `cargo server` starts
- [ ] `cargo client` starts and connects

#### Manual Verification:
- [ ] Green health bar visible above all characters and DummyTarget
- [ ] Health bar always faces the camera regardless of character orientation
- [ ] Hitting a target causes the green bar to shrink from right to left
- [ ] On respawn, health bar returns to full
- [ ] Health bar is visible from different camera angles
- [ ] Two clients see each other's health bars updating

---

## Testing Strategy

### Manual Testing Steps:
1. Start server, connect two clients
2. Client A punches Client B — verify health bar decreases (~10 damage per punch hit)
3. Client A fireballs Client B — verify health bar decreases more (~25 damage)
4. Punch DummyTarget to 0 — verify it respawns at RespawnPoint
5. Kill Client B — verify instant teleport to RespawnPoint, full health bar
6. Immediately attack respawned Client B — verify no damage for ~1 second
7. Wait 1 second, attack again — verify damage works
8. Move camera around — verify health bars always face camera

## Performance Considerations

- Billboard system runs per-frame but only on `HealthBarRoot` entities (one per character) — negligible cost
- Health bar update runs per-frame on all characters — negligible cost at expected entity counts
- No additional network bandwidth beyond the replicated `Health` and `Invulnerable` components

## References

- Research: `doc/research/2026-02-16-health-respawn-billboard-ui.md`
- Hit detection: `crates/protocol/src/hit_detection.rs`
- Ability effects: `crates/protocol/src/ability.rs:36-52`
- Character spawn: `crates/server/src/gameplay.rs:62-108`
- Render setup: `crates/render/src/lib.rs:73-96`
- Protocol registration: `crates/protocol/src/lib.rs:97-161`
