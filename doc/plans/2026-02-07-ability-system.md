# Ability System Implementation Plan

## Overview

Add a full ability system: hotkey-driven (1-4), configurable ability slots per character, asset-loaded ability definitions (RON files), a phase-based state machine (startup/active/recovery), multi-step combos, tick-based cooldowns, and projectile spawning using lightyear's PreSpawned/DisableRollback pattern. All predicted and rollback-compatible.

## Current State Analysis

- `PlayerActions` enum has Move, Jump, PlaceVoxel, RemoveVoxel — no ability actions
- No asset loading infrastructure (no RON, no `bevy_common_assets`, no `assets/` dir)
- Component registration pattern established: `register_component::<T>().add_prediction()`
- Lightyear's leafwing `InputPlugin<PlayerActions>` already set up with `rebroadcast_inputs: true`
- Server spawns characters with `Replicate`, `PredictionTarget`, `ControlledBy`, physics bundle
- Client attaches `InputMap` on `Added<Replicated>` + `Controlled`, physics on `Added<Predicted>`
- Shared `apply_movement` in FixedUpdate handles prediction on both sides
- Tick access: `Single<&LocalTimeline, Without<ClientOf>>` → `timeline.tick()`

### Key Discoveries:
- Multiple `Actionlike` enums supported but single enum is simpler (research decision)
- `Tick(u16)` with wrapping arithmetic, `tick - other_tick` returns `i16`
- `PreSpawned::default()` hashes (archetype + tick); `default_with_salt(u64)` adds disambiguation
- `DisableRollback` excludes children from rollback resimulation
- Spaceships demo `Weapon` component: `add_prediction()` with no custom should_rollback (uses PartialEq)
- Direction-only projectile pattern: replicated parent `ProjectileSpawn` + local child bullets via `BulletOf` relationship

## Desired End State

Characters have 4 ability slots (keys 1-4) mapped to abilities defined in `assets/abilities.ron`. Pressing a hotkey activates the slotted ability: it progresses through startup → active → recovery phases (tick-counted). Multi-step abilities allow re-pressing during a window to chain hits. Projectile-type abilities spawn predicted entities using PreSpawned. All state (active ability, cooldowns) participates in lightyear rollback.

### Verification:
- `cargo test-all` passes
- `cargo server` + `cargo client -c 1`: press 1-4 to activate abilities, see phase transitions in logs
- Projectile abilities spawn entities visible to both players
- Rollback correctly resimulates ability state on mismatch

## What We're NOT Doing

- Actual damage/health/hitbox systems (no combat damage yet)
- Stat requirement checks (abilities are equippable without stat gates for now)
- Visual effects, animations, or UI for abilities
- Ability copying (Sonic Battle inspiration — future)
- Comeback gauge
- AI-controlled ability usage
- Alignment-based ability restrictions

## Implementation Approach

New code goes primarily in `crates/protocol/src/` (shared types, systems, asset loading) with thin wiring in `crates/server/src/gameplay.rs` and `crates/client/src/gameplay.rs`. Ability systems are shared (like `apply_movement`) and run in `FixedUpdate` for both server and client prediction.

---

## Phase 1: Data Types & Asset Loading

### Overview
Define the ability data model and load definitions from RON files.

### Changes Required:

#### 1. Add workspace dependencies
**File**: `Cargo.toml`
**Changes**: Add `ron` and `bevy_common_assets` to workspace dependencies.

```toml
# Add to [workspace.dependencies]:
ron = "0.9"
bevy_common_assets = { version = "0.13", features = ["ron"] }
```

#### 2. Add protocol crate dependencies
**File**: `crates/protocol/Cargo.toml`
**Changes**: Add `ron` and `bevy_common_assets`.

```toml
# Add to [dependencies]:
ron = { workspace = true }
bevy_common_assets = { workspace = true }
```

#### 3. Create ability types module
**File**: `crates/protocol/src/ability.rs` (new)
**Changes**: Define all ability-related types.

```rust
use bevy::prelude::*;
use lightyear::prelude::Tick;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// String-based ability identifier, matching keys in abilities.ron.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct AbilityId(pub String);

/// What an ability does when it enters the Active phase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub enum AbilityEffect {
    /// No spawned entity — melee or self-buff.
    Melee,
    /// Spawns a projectile in the facing direction.
    Projectile {
        speed: f32,
        lifetime_ticks: u16,
    },
    /// Dashes the character forward.
    Dash {
        speed: f32,
    },
}

/// Data definition for a single ability, loaded from RON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct AbilityDef {
    pub startup_ticks: u16,
    pub active_ticks: u16,
    pub recovery_ticks: u16,
    pub cooldown_ticks: u16,
    /// Number of steps in a combo chain. 1 = single hit.
    pub steps: u8,
    /// Ticks the player has to press again to advance to next step.
    /// Only meaningful when steps > 1.
    pub step_window_ticks: u16,
    pub effect: AbilityEffect,
}

/// Collection of all ability definitions, loaded as a Bevy asset from RON.
#[derive(Asset, TypePath, Debug, Clone, Serialize, Deserialize)]
pub struct AbilityDefsAsset {
    pub abilities: HashMap<String, AbilityDef>,
}

/// Resource holding ability definitions after loading is complete.
#[derive(Resource, Debug, Clone)]
pub struct AbilityDefs {
    pub abilities: HashMap<AbilityId, AbilityDef>,
}

impl AbilityDefs {
    pub fn get(&self, id: &AbilityId) -> Option<&AbilityDef> {
        self.abilities.get(id)
    }
}

/// Ability slots on a character — maps slot index (0-3) to an ability.
/// None means the slot is empty.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
pub struct AbilitySlots(pub [Option<AbilityId>; 4]);

impl Default for AbilitySlots {
    fn default() -> Self {
        Self([None, None, None, None])
    }
}

/// Current ability execution state on a character.
/// Present only while an ability is active; removed when ability completes.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ActiveAbility {
    pub ability_id: AbilityId,
    pub phase: AbilityPhase,
    pub phase_start_tick: Tick,
    /// Current step in a multi-hit combo (0-indexed).
    pub step: u8,
    pub total_steps: u8,
    /// Whether the player pressed the key again during this step's window.
    pub chain_input_received: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum AbilityPhase {
    Startup,
    Active,
    Recovery,
}

/// Per-slot cooldown tracking. Each entry is the tick the slot was last activated.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AbilityCooldowns {
    pub last_used: [Option<Tick>; 4],
}

impl Default for AbilityCooldowns {
    fn default() -> Self {
        Self { last_used: [None; 4] }
    }
}

/// Marker on a ProjectileSpawn entity — stores spawn parameters.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
pub struct AbilityProjectileSpawn {
    pub spawn_tick: Tick,
    pub position: bevy::math::Vec3,
    pub direction: bevy::math::Vec3,
    pub speed: f32,
    pub lifetime_ticks: u16,
    pub ability_id: AbilityId,
    pub shooter: Entity,
}

/// Relationship: child bullet → parent ProjectileSpawn.
#[derive(Component, Debug)]
#[relationship(relationship_target = AbilityBullets)]
pub struct AbilityBulletOf(pub Entity);

/// Relationship target on the ProjectileSpawn entity.
#[derive(Component, Debug)]
#[relationship_target(relationship = AbilityBulletOf, linked_spawn)]
pub struct AbilityBullets(Vec<Entity>);
```

#### 4. Create sample abilities asset
**File**: `assets/abilities.ron` (new)

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
            effect: Melee,
        ),
        "dash": (
            startup_ticks: 2,
            active_ticks: 8,
            recovery_ticks: 4,
            cooldown_ticks: 64,
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
            cooldown_ticks: 96,
            steps: 1,
            step_window_ticks: 0,
            effect: Projectile(
                speed: 20.0,
                lifetime_ticks: 192,
            ),
        ),
    },
)
```

#### 5. Asset loading plugin
**File**: `crates/protocol/src/ability.rs` (append to the same file)

```rust
use bevy_common_assets::ron::RonAssetPlugin;

pub struct AbilityPlugin;

impl Plugin for AbilityPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RonAssetPlugin::<AbilityDefsAsset>::new(&["abilities.ron"]));
        app.init_resource::<AbilityDefsHandle>();
        app.add_systems(Startup, load_ability_defs);
        app.add_systems(Update, insert_ability_defs_resource.run_if(not(resource_exists::<AbilityDefs>)));
    }
}

#[derive(Resource, Default)]
struct AbilityDefsHandle(Option<Handle<AbilityDefsAsset>>);

fn load_ability_defs(
    asset_server: Res<AssetServer>,
    mut handle: ResMut<AbilityDefsHandle>,
) {
    handle.0 = Some(asset_server.load("abilities.ron"));
}

fn insert_ability_defs_resource(
    mut commands: Commands,
    handle: Res<AbilityDefsHandle>,
    assets: Res<Assets<AbilityDefsAsset>>,
) {
    let Some(ref h) = handle.0 else { return };
    let Some(asset) = assets.get(h) else { return };
    let defs = AbilityDefs {
        abilities: asset.abilities.iter()
            .map(|(k, v)| (AbilityId(k.clone()), v.clone()))
            .collect(),
    };
    info!("Loaded {} ability definitions", defs.abilities.len());
    commands.insert_resource(defs);
}
```

#### 6. Wire into protocol
**File**: `crates/protocol/src/lib.rs`
**Changes**: Add `pub mod ability;` and re-export types. Add `AbilityPlugin` to `SharedGameplayPlugin`. Register ability components in `ProtocolPlugin`.

Add to imports and module declaration:
```rust
pub mod ability;
pub use ability::{
    AbilityId, AbilityDef, AbilityDefs, AbilityDefsAsset, AbilityEffect,
    AbilitySlots, ActiveAbility, AbilityPhase, AbilityCooldowns, AbilityPlugin,
    AbilityProjectileSpawn, AbilityBulletOf, AbilityBullets,
};
```

In `ProtocolPlugin::build`, add component registrations:
```rust
// Ability components
app.register_component::<AbilitySlots>();  // replicated, no prediction
app.register_component::<ActiveAbility>()
    .add_prediction();
app.register_component::<AbilityCooldowns>()
    .add_prediction();
app.register_component::<AbilityProjectileSpawn>();  // replicated, no prediction (PreSpawned handles matching)
```

In `SharedGameplayPlugin::build`, add:
```rust
app.add_plugins(AbilityPlugin);
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` compiles
- [ ] `cargo test-all` passes
- [x] `cargo server` starts without errors (loads abilities.ron)
- [ ] `cargo client -c 1` starts without errors (loads abilities.ron)

#### Manual Verification:
- [x] Log message "Loaded 3 ability definitions" appears on server

---

## Phase 2: Input & Slot System

### Overview
Add ability hotkeys (1-4) to the input system and attach ability slots to characters.

### Changes Required:

#### 1. Add ability actions to PlayerActions
**File**: `crates/protocol/src/lib.rs`
**Changes**: Add `Ability1`–`Ability4` variants.

```rust
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy, Hash, Reflect)]
pub enum PlayerActions {
    Move,
    Jump,
    PlaceVoxel,
    RemoveVoxel,
    Ability1,
    Ability2,
    Ability3,
    Ability4,
}

impl Actionlike for PlayerActions {
    fn input_control_kind(&self) -> InputControlKind {
        match self {
            Self::Move => InputControlKind::DualAxis,
            _ => InputControlKind::Button,
        }
    }
}
```

#### 2. Add keybindings on client
**File**: `crates/client/src/gameplay.rs`
**Changes**: Add Digit1–4 and gamepad bindings to InputMap.

```rust
commands.entity(entity).insert(
    InputMap::new([(PlayerActions::Jump, KeyCode::Space)])
        .with(PlayerActions::Jump, GamepadButton::South)
        .with_dual_axis(PlayerActions::Move, GamepadStick::LEFT)
        .with_dual_axis(PlayerActions::Move, VirtualDPad::wasd())
        .with(PlayerActions::PlaceVoxel, MouseButton::Left)
        .with(PlayerActions::RemoveVoxel, MouseButton::Right)
        .with(PlayerActions::Ability1, KeyCode::Digit1)
        .with(PlayerActions::Ability2, KeyCode::Digit2)
        .with(PlayerActions::Ability3, KeyCode::Digit3)
        .with(PlayerActions::Ability4, KeyCode::Digit4),
);
```

#### 3. Spawn characters with ability slots
**File**: `crates/server/src/gameplay.rs`
**Changes**: Add `AbilitySlots` and `AbilityCooldowns` to character spawn bundle. Give the first character a default loadout.

```rust
// In handle_connected, add to the spawn tuple:
AbilitySlots([
    Some(AbilityId("punch".into())),
    Some(AbilityId("dash".into())),
    Some(AbilityId("fireball".into())),
    None,
]),
AbilityCooldowns::default(),
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` compiles
- [ ] `cargo test-all` passes

#### Manual Verification:
- [ ] `cargo server` + `cargo client -c 1`: pressing 1/2/3/4 shows `just_pressed` in action state (verify with a temporary log)
- [ ] AbilitySlots component replicates to client (visible in logs or debugger)

---

## Phase 3: Ability State Machine & Cooldowns

### Overview
Implement ability activation, phase transitions, multi-step combos, and cooldowns. All systems run in FixedUpdate and are shared between server and client.

### Changes Required:

#### 1. Shared ability systems
**File**: `crates/protocol/src/ability.rs` (append)
**Changes**: Add systems for activation, phase advancement, and combo chaining.

```rust
use lightyear::prelude::Tick;
use lightyear::timeline::LocalTimeline;
use lightyear::connection::client::ClientOf;

/// Maps a PlayerActions ability variant to a slot index.
pub fn ability_action_to_slot(action: &PlayerActions) -> Option<usize> {
    match action {
        PlayerActions::Ability1 => Some(0),
        PlayerActions::Ability2 => Some(1),
        PlayerActions::Ability3 => Some(2),
        PlayerActions::Ability4 => Some(3),
        _ => None,
    }
}

/// Activate an ability when a hotkey is pressed and no ability is currently active.
pub fn ability_activation(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut query: Query<
        (Entity, &ActionState<PlayerActions>, &AbilitySlots, &mut AbilityCooldowns),
        Without<ActiveAbility>,
    >,
) {
    let tick = timeline.tick();

    for (entity, action_state, slots, mut cooldowns) in &mut query {
        for action in [
            PlayerActions::Ability1,
            PlayerActions::Ability2,
            PlayerActions::Ability3,
            PlayerActions::Ability4,
        ] {
            if !action_state.just_pressed(&action) {
                continue;
            }
            let Some(slot_idx) = ability_action_to_slot(&action) else { continue };
            let Some(ref ability_id) = slots.0[slot_idx] else { continue };
            let Some(def) = ability_defs.get(ability_id) else { continue };

            // Check cooldown
            if let Some(last_used) = cooldowns.last_used[slot_idx] {
                let elapsed = tick - last_used;
                if elapsed.abs() <= def.cooldown_ticks as i16 {
                    continue;
                }
            }

            // Activate
            cooldowns.last_used[slot_idx] = Some(tick);
            commands.entity(entity).insert(ActiveAbility {
                ability_id: ability_id.clone(),
                phase: AbilityPhase::Startup,
                phase_start_tick: tick,
                step: 0,
                total_steps: def.steps,
                chain_input_received: false,
            });
            break; // only one ability at a time
        }
    }
}

/// Advance ability phases based on tick counts. Handle multi-step combo chaining.
pub fn ability_phase_advance(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    mut query: Query<(Entity, &mut ActiveAbility, &ActionState<PlayerActions>, &AbilitySlots)>,
) {
    let tick = timeline.tick();

    for (entity, mut active, action_state, slots) in &mut query {
        let Some(def) = ability_defs.get(&active.ability_id) else {
            commands.entity(entity).remove::<ActiveAbility>();
            continue;
        };

        let elapsed = (tick - active.phase_start_tick) as i16;

        // Check for combo chain input (re-pressing the same ability key)
        if !active.chain_input_received && active.step + 1 < active.total_steps {
            // Find which slot this ability is in
            for (slot_idx, slot) in slots.0.iter().enumerate() {
                if slot.as_ref() == Some(&active.ability_id) {
                    let action = match slot_idx {
                        0 => PlayerActions::Ability1,
                        1 => PlayerActions::Ability2,
                        2 => PlayerActions::Ability3,
                        3 => PlayerActions::Ability4,
                        _ => continue,
                    };
                    if action_state.just_pressed(&action) {
                        active.chain_input_received = true;
                    }
                    break;
                }
            }
        }

        match active.phase {
            AbilityPhase::Startup => {
                if elapsed >= def.startup_ticks as i16 {
                    active.phase = AbilityPhase::Active;
                    active.phase_start_tick = tick;
                }
            }
            AbilityPhase::Active => {
                if elapsed >= def.active_ticks as i16 {
                    active.phase = AbilityPhase::Recovery;
                    active.phase_start_tick = tick;
                }
            }
            AbilityPhase::Recovery => {
                if elapsed >= def.recovery_ticks as i16 {
                    // Check for combo chain
                    if active.chain_input_received && active.step + 1 < active.total_steps {
                        // Advance to next step
                        active.step += 1;
                        active.phase = AbilityPhase::Startup;
                        active.phase_start_tick = tick;
                        active.chain_input_received = false;
                    } else {
                        // Ability complete
                        commands.entity(entity).remove::<ActiveAbility>();
                    }
                } else if !active.chain_input_received
                    && active.step + 1 < active.total_steps
                    && elapsed >= def.step_window_ticks as i16
                {
                    // Window expired without chain input — end ability
                    commands.entity(entity).remove::<ActiveAbility>();
                }
            }
        }
    }
}
```

#### 2. Register systems in SharedGameplayPlugin
**File**: `crates/protocol/src/lib.rs`
**Changes**: Add ability systems to FixedUpdate.

```rust
// In SharedGameplayPlugin::build, add:
app.add_systems(FixedUpdate, (
    ability::ability_activation,
    ability::ability_phase_advance,
).chain());
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` compiles
- [ ] `cargo test-all` passes

#### Manual Verification:
- [ ] Press 1: "punch" ability activates, progresses through startup→active→recovery phases (add temporary info! logs to verify)
- [ ] Press 1 again during recovery: combo advances to step 1, then step 2
- [ ] Press 2 during cooldown: nothing happens
- [ ] After cooldown expires: ability can be used again

---

## Phase 4: Projectile Spawning

### Overview
When a projectile-type ability enters its Active phase, spawn a `AbilityProjectileSpawn` entity with `PreSpawned`. A shared system then spawns a child bullet with `DisableRollback`.

### Changes Required:

#### 1. Projectile spawn system
**File**: `crates/protocol/src/ability.rs` (append)
**Changes**: System that detects Active phase transition for projectile abilities and spawns the PreSpawned entity.

```rust
use avian3d::prelude::*;
use lightyear::prelude::*;
use lightyear::replication::prespawn::PreSpawned;
use lightyear::prediction::rollback::DisableRollback;

/// Spawn a ProjectileSpawn entity when a projectile ability enters Active phase.
/// Runs on both client and server for prediction.
pub fn ability_projectile_spawn(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    query: Query<(Entity, &ActiveAbility, &Position, &Rotation), With<CharacterMarker>>,
    // Server-only: need to know if we should add Replicate
    server_query: Query<&ControlledBy>,
) {
    let tick = timeline.tick();

    for (entity, active, position, rotation) in &query {
        // Only spawn on the tick the ability enters Active phase
        if active.phase != AbilityPhase::Active || active.phase_start_tick != tick {
            continue;
        }

        let Some(def) = ability_defs.get(&active.ability_id) else { continue };

        let AbilityEffect::Projectile { speed, lifetime_ticks } = &def.effect else { continue };

        // Compute forward direction from rotation
        let direction = rotation.0 * Vec3::NEG_Z;

        let spawn_info = AbilityProjectileSpawn {
            spawn_tick: tick,
            position: position.0 + direction * 1.5, // spawn slightly in front
            direction: direction.normalize(),
            speed: *speed,
            lifetime_ticks: *lifetime_ticks,
            ability_id: active.ability_id.clone(),
            shooter: entity,
        };

        let mut spawn_cmd = commands.spawn((
            spawn_info,
            PreSpawned::default_with_salt(active.step as u64),
            Name::new("AbilityProjectileSpawn"),
        ));

        // If server, add replication components
        if let Ok(controlled_by) = server_query.get(entity) {
            spawn_cmd.insert((
                Replicate::to_clients(NetworkTarget::All),
                PredictionTarget::to_clients(NetworkTarget::Single(controlled_by.owner)),
                controlled_by.clone(),
            ));
        }
    }
}

/// Spawn child bullet entities from AbilityProjectileSpawn parents.
/// Runs in PreUpdate so it catches newly spawned ProjectileSpawn entities.
pub fn handle_ability_projectile_spawn(
    mut commands: Commands,
    spawn_query: Query<
        (Entity, &AbilityProjectileSpawn),
        Without<AbilityBullets>,
    >,
) {
    for (spawn_entity, spawn_info) in &spawn_query {
        commands.spawn((
            Position(spawn_info.position),
            Rotation::default(),
            LinearVelocity(spawn_info.direction * spawn_info.speed),
            RigidBody::Kinematic,
            Collider::sphere(0.25),
            AbilityBulletOf(spawn_entity),
            DisableRollback,
            Name::new("AbilityBullet"),
        ));
    }
}

/// When a child bullet's AbilityBulletOf relationship is removed (bullet despawned),
/// also despawn the parent ProjectileSpawn.
pub fn despawn_ability_projectile_spawn(
    trigger: On<Remove, AbilityBulletOf>,
    bullet_query: Query<&AbilityBulletOf>,
    mut commands: Commands,
) {
    if let Ok(bullet_of) = bullet_query.get(trigger.entity) {
        if let Ok(mut c) = commands.get_entity(bullet_of.0) {
            c.try_despawn();
        }
    }
}

/// Despawn bullets after their lifetime expires (tick-based).
pub fn ability_bullet_lifetime(
    mut commands: Commands,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    query: Query<(Entity, &AbilityBulletOf)>,
    spawn_query: Query<&AbilityProjectileSpawn>,
) {
    let tick = timeline.tick();
    for (entity, bullet_of) in &query {
        if let Ok(spawn_info) = spawn_query.get(bullet_of.0) {
            let elapsed = tick - spawn_info.spawn_tick;
            if elapsed.abs() >= spawn_info.lifetime_ticks as i16 {
                commands.entity(entity).try_despawn();
            }
        }
    }
}
```

#### 2. Dash effect system
**File**: `crates/protocol/src/ability.rs` (append)

```rust
/// Apply dash velocity while a dash ability is in Active phase.
pub fn ability_dash_effect(
    ability_defs: Res<AbilityDefs>,
    mut query: Query<(&ActiveAbility, &Rotation, Forces), With<CharacterMarker>>,
) {
    for (active, rotation, mut forces) in &mut query {
        if active.phase != AbilityPhase::Active {
            continue;
        }
        let Some(def) = ability_defs.get(&active.ability_id) else { continue };
        let AbilityEffect::Dash { speed } = &def.effect else { continue };

        let direction = rotation.0 * Vec3::NEG_Z;
        let dash_velocity = direction.normalize() * *speed;
        // Override horizontal velocity during dash
        let current = forces.linear_velocity();
        let diff = Vec3::new(
            dash_velocity.x - current.x,
            0.0,
            dash_velocity.z - current.z,
        );
        forces.apply_linear_impulse(diff);
    }
}
```

#### 3. Register projectile systems
**File**: `crates/protocol/src/lib.rs`
**Changes**: Add projectile and dash systems to appropriate schedules.

```rust
// In SharedGameplayPlugin::build, add:
app.add_systems(FixedUpdate, (
    ability::ability_activation,
    ability::ability_phase_advance,
    ability::ability_projectile_spawn,
    ability::ability_dash_effect,
).chain());

app.add_systems(PreUpdate, ability::handle_ability_projectile_spawn);
app.add_systems(FixedUpdate, ability::ability_bullet_lifetime);
app.add_observer(ability::despawn_ability_projectile_spawn);
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` compiles
- [ ] `cargo test-all` passes

#### Manual Verification:
- [ ] Press 3 (fireball): after startup ticks, a projectile entity spawns and moves forward
- [ ] Projectile is visible to both server and connected clients (PreSpawned matching works)
- [ ] Projectile despawns after lifetime expires
- [ ] Press 2 (dash): character moves forward rapidly during Active phase
- [ ] On rollback, ability state and projectiles resimulate correctly

---

## Phase 5: Server/Client Integration & Cleanup

### Overview
Final wiring, imports, ensure server headless asset loading works, update README.

### Changes Required:

#### 1. Server asset loading
**File**: `crates/server/src/main.rs`
**Changes**: The server uses `MinimalPlugins` without `DefaultPlugins`, so `AssetPlugin` is already added. The `AbilityPlugin` (added via `SharedGameplayPlugin`) will use it. Ensure the `assets/` directory is accessible. If needed, configure `AssetPlugin` path.

No code change expected — `AssetPlugin::default()` is already present at line 18. Verify at build time that the server can load `abilities.ron`.

#### 2. Update server character spawn imports
**File**: `crates/server/src/gameplay.rs`
**Changes**: Import ability types.

```rust
use protocol::{AbilitySlots, AbilityId, AbilityCooldowns};
```

#### 3. Ensure client imports
**File**: `crates/client/src/gameplay.rs`
**Changes**: Import PlayerActions variants (already imported via `protocol::*`). No changes needed if `use protocol::*` is used.

#### 4. Movement suppression during abilities
**File**: `crates/protocol/src/lib.rs`
**Changes**: Modify `apply_movement` to skip movement when an active ability is in progress (optional — prevents movement during ability execution). This should be called from the movement systems which would need to query `Option<&ActiveAbility>`.

Alternatively, add `Without<ActiveAbility>` to the movement query filters in both server and client. This is simpler:

**File**: `crates/server/src/gameplay.rs`
```rust
// Add Without<ActiveAbility> to movement query:
mut query: Query<
    (Entity, &ActionState<PlayerActions>, &ComputedMass, &Position, Forces),
    (With<CharacterMarker>, Without<ActiveAbility>),
>,
```

**File**: `crates/client/src/gameplay.rs`
```rust
// Same filter:
mut query: Query<
    (Entity, &ActionState<PlayerActions>, &ComputedMass, &Position, Forces),
    (With<Predicted>, With<CharacterMarker>, Without<ActiveAbility>),
>,
```

#### 5. Update README
**File**: `README.md`
**Changes**: Add section about the ability system, hotkeys, and ability definitions file.

Add a section:
```markdown
## Ability System

Abilities are defined in `assets/abilities.ron` and loaded at startup. Each character has 4 ability slots mapped to keys 1-4.

### Hotkeys
- `1` - Ability slot 1
- `2` - Ability slot 2
- `3` - Ability slot 3
- `4` - Ability slot 4

### Defining Abilities

Edit `assets/abilities.ron` to add or modify abilities. Each ability has:
- Phase durations (startup, active, recovery) in ticks (64 ticks = 1 second)
- Cooldown in ticks
- Combo steps and chain window
- Effect type: `Melee`, `Dash`, or `Projectile`
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` compiles
- [ ] `cargo test-all` passes
- [x] `cargo server` starts and loads abilities.ron
- [ ] `cargo client -c 1` starts and loads abilities.ron

#### Manual Verification:
- [ ] Connect 2 clients: both characters have ability slots replicated
- [ ] Press 1 (punch): melee phases execute, movement is suppressed during ability
- [ ] Press 1 repeatedly: combo chains through 3 steps
- [ ] Press 2 (dash): character dashes forward
- [ ] Press 3 (fireball): projectile spawns after startup, flies forward, despawns after lifetime
- [ ] Second client sees the projectile (PreSpawned matching)
- [ ] Cooldowns prevent re-use until expired
- [ ] No regressions in basic movement (WASD, jump)

---

## Testing Strategy

### Unit Tests:
- `AbilityId` serialization round-trip (RON format)
- `AbilityDef` deserialization from RON string
- `AbilityCooldowns` default values
- `ability_action_to_slot` mapping correctness

### Integration Tests:
- Load `abilities.ron` and verify all 3 abilities parse correctly
- Ability activation inserts `ActiveAbility` component
- Phase advancement progresses through all phases within correct tick counts
- Cooldown blocks re-activation

### Manual Testing Steps:
1. Start server + 1 client, press each ability key, verify log output
2. Test combo: press 1, wait for Active phase, press 1 again, verify step increments
3. Test cooldown: press 1, try pressing 1 again immediately, verify blocked
4. Test projectile: press 3, verify entity spawns and moves
5. Test with 2 clients: verify projectile visible to both
6. Disconnect and reconnect: verify ability state recovers

## Performance Considerations

- `AbilityDefs` is a `Resource`, not queried per-entity — no per-tick allocation
- `ActiveAbility` only exists on entities with active abilities — sparse queries
- Projectile spawn uses `PreSpawned` hash matching — no extra bandwidth for projectile position each tick
- `DisableRollback` prevents unnecessary resimulation of bullet physics

## References

- Research document: `doc/research/2026-02-07-ability-system-architecture.md`
- Lightyear projectiles example: `git/lightyear/examples/projectiles/src/shared.rs`
- Lightyear spaceships Weapon pattern: `git/lightyear/demos/spaceships/src/protocol.rs:119-134`
- Existing protocol: `crates/protocol/src/lib.rs`
- Existing movement: `crates/protocol/src/lib.rs:170-214`
- Stats design: `doc/scratch/stats.md`
