---
date: 2026-02-07T12:14:13-08:00
researcher: Claude
git_commit: 4646d7f27a75b8e399e027d85bb5f395a35dab82
branch: master
repository: bevy-lightyear-template
topic: "Ability system architecture: hotkeys, configurable slots, prediction, multi-step abilities, projectile spawning"
tags: [research, codebase, abilities, input, prediction, projectiles, leafwing, lightyear]
status: complete
last_updated: 2026-02-07T13:15:00-08:00
last_updated_by: Claude
revision: 2
revision_note: "Added follow-up research on multi-step ability state machines and rollback registration"
---

# Research: Ability System Architecture

**Date**: 2026-02-07T12:14:13 PST
**Git Commit**: 4646d7f
**Branch**: master

## Research Question

How to add an ability system for players that enables them to press a hotkey and activate an ability. Requirements:
- Hotkeys hardcoded to 1, 2, 3, 4
- Abilities are configurable/swappable per player
- Server authoritatively triggers abilities; clients optimistically execute
- Dedicated systems for multi-step behavior (e.g. pressing key multiple times during a flourish)
- Use PreSpawned, ProjectileSpawn, and DisableRollback for generated projectiles
- Use Ticks for ability duration, not f32 time
- Use leafwing-input-manager, not bevy_enhanced_input

## Summary

The codebase already has the foundational patterns needed: leafwing input via `PlayerActions` enum, lightyear's `InputPlugin` for network replication, `PreSpawned`/`DisableRollback` in the local lightyear fork, and tick-based timing via `LocalTimeline`. This document maps all relevant existing code and lightyear APIs.

## Detailed Findings

### 1. Existing Input System

The current input system uses a single `PlayerActions` enum in `crates/protocol/src/lib.rs:23-38`:

```rust
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy, Hash, Reflect)]
pub enum PlayerActions {
    Move,
    Jump,
    PlaceVoxel,
    RemoveVoxel,
}

impl Actionlike for PlayerActions {
    fn input_control_kind(&self) -> InputControlKind {
        match self {
            Self::Move => InputControlKind::DualAxis,
            Self::Jump | Self::PlaceVoxel | Self::RemoveVoxel => InputControlKind::Button,
        }
    }
}
```

Registered with lightyear in `crates/protocol/src/lib.rs:75-80`:
```rust
app.add_plugins(InputPlugin::<PlayerActions> {
    config: InputConfig::<PlayerActions> {
        rebroadcast_inputs: true,
        ..default()
    },
});
```

Client-side `InputMap` is attached in `crates/client/src/gameplay.rs:30-36`:
```rust
commands.entity(entity).insert(
    InputMap::new([(PlayerActions::Jump, KeyCode::Space)])
        .with_dual_axis(PlayerActions::Move, VirtualDPad::wasd())
        .with(PlayerActions::PlaceVoxel, MouseButton::Left)
        .with(PlayerActions::RemoveVoxel, MouseButton::Right),
);
```

### 2. Lightyear's Leafwing Integration

**Source**: `git/lightyear/lightyear_inputs_leafwing/src/plugin.rs`

Lightyear's `InputPlugin<A>` wraps leafwing's `InputManagerPlugin<A>` and adds networking. Key behavior:
- On client: adds `InputManagerPlugin::<A>::default()` + `ClientInputPlugin` for sending inputs to server
- On server: adds `ServerInputPlugin` for receiving/rebroadcasting inputs
- `InputManagerPlugin` is added automatically - do NOT add it separately

**Multiple Actionlike enums are supported.** Each `InputPlugin::<A>` registration is independent. You can register `InputPlugin::<PlayerActions>` and `InputPlugin::<AbilityActions>` separately. Each gets its own `ActionState<A>` component and `InputMap<A>`.

**LeafwingUserAction trait** (auto-implemented) requires: `Serialize + DeserializeOwned + Clone + PartialEq + Send + Sync + Debug + 'static + Copy + Actionlike + GetTypeRegistration`

### 3. Tick System

**Tick type**: `lightyear_core::tick::Tick` - a wrapping u16 created via `wrapping_id!(Tick)` macro.

- `git/lightyear/lightyear_core/src/tick.rs:10` - `wrapping_id!(Tick)` generates the type
- `Tick(pub u16)` with wrapping arithmetic, Ord, Hash, etc.
- Supports `tick - other_tick` returning i16 (wrapping difference)

**Getting current tick**: Via the `LocalTimeline` component (not a global resource).

```rust
// Query the timeline
timeline: Single<&LocalTimeline, Without<ClientOf>>,
// Get tick
let tick = timeline.tick();
```

`LocalTimeline` is a component on the server entity and client entity, incremented every `FixedFirst`.

`git/lightyear/lightyear_core/src/timeline.rs:106-113`:
```rust
pub struct LocalTimeline(Timeline<Local>);

pub(crate) fn increment_local_tick(mut query: Query<&mut LocalTimeline>) {
    query.iter_mut().for_each(|mut t| {
        t.apply_delta(TickDelta::from_i16(1));
    })
}
```

**TickDuration resource**: `Res<TickDuration>` wraps `Duration`, gives the wall-time per tick. At 64Hz: ~15.625ms per tick.

**No TickManager**: The old `TickManager` resource was replaced with `LocalTimeline` component in recent lightyear versions.

### 4. PreSpawned Component

**Source**: `git/lightyear/lightyear_replication/src/prespawn.rs:187-206`

```rust
#[derive(Component)]
pub struct PreSpawned {
    pub hash: Option<u64>,       // Auto-computed from archetype + tick if None
    pub user_salt: Option<u64>,  // Extra salt for hash disambiguation
    pub receiver: Option<Entity>,
}
```

**How it works**:
1. Client spawns entity with `PreSpawned::default()` in `FixedMain` schedule
2. A component hook auto-computes `hash` from the entity's archetype (component types) + current tick
3. Server spawns the same entity with `PreSpawned::default()` + `Replicate` + `PredictionTarget`
4. When server entity replicates to client, lightyear matches by hash to the prespawned entity instead of creating a new one
5. Unmatched prespawned entities are cleaned up after ~50 ticks

**Usage pattern** (from `projectiles` example):
```rust
// Client side:
commands.spawn((bullet_bundle, PreSpawned::default()));

// Server side:
commands.spawn((
    bullet_bundle,
    PreSpawned::default(),
    Replicate::to_clients(NetworkTarget::All),
    PredictionTarget::to_clients(NetworkTarget::Single(shooter_id)),
    controlled_by.clone(),
));
```

Use `PreSpawned::default_with_salt(salt)` when multiple entities could share the same archetype and tick (e.g., shotgun pellets).

### 5. DisableRollback Component

**Source**: `git/lightyear/lightyear_prediction/src/rollback.rs:235-238`

```rust
/// Marker component to indicate that the entity will be completely excluded from rollbacks.
/// It won't be part of rollback checks, and it won't be rolled back to a past state if a rollback happens.
#[derive(Component, Debug)]
pub struct DisableRollback;
```

During rollback, entities with `DisableRollback` get a `DisabledDuringRollback` component added, making them invisible to all queries during the rollback resimulation.

**When to use**: For child entities spawned from a predicted parent. The parent `ProjectileSpawn` entity is predicted/prespawned and handles rollback matching. The child bullet entity gets `DisableRollback` so it doesn't interfere with rollback physics - it just exists in the world and isn't resimulated.

### 6. ProjectileSpawn Pattern (Direction-Only Replication)

**Source**: `git/lightyear/examples/projectiles/src/shared.rs:1090-1401`

The `direction_only` module demonstrates a bandwidth-efficient projectile pattern:

1. A `ProjectileSpawn` component stores spawn parameters (tick, position, rotation, speed, weapon type, shooter)
2. This entity IS replicated and predicted (with `PreSpawned`)
3. A system (`handle_projectile_spawn`) watches for new `ProjectileSpawn` entities and spawns child bullet entities
4. Child bullets get `DisableRollback` and a `BulletOf(parent)` relationship
5. When a child bullet despawns, the parent `ProjectileSpawn` also despawns

```rust
// ProjectileSpawn entity (predicted/replicated):
commands.spawn((spawn_info, PreSpawned::default()));

// Child bullet (not predicted, local only):
commands.spawn((
    bullet_bundle,
    BulletOf(spawn_entity),
    DisableRollback,
    Name::new("Bullet"),
));
```

### 7. Existing Character Spawn & Movement Pattern

**Server** (`crates/server/src/gameplay.rs:46-85`):
- Spawns character on `Connected` event with `Replicate`, `PredictionTarget`, `ControlledBy`, `ActionState::<PlayerActions>::default()`
- Movement in `FixedUpdate` queries `ActionState<PlayerActions>`

**Client** (`crates/client/src/gameplay.rs:16-76`):
- On `Added<Replicated>` + `Controlled`: attaches `InputMap` to owned character
- On `Added<Predicted>`: adds `CharacterPhysicsBundle`
- Movement in `FixedUpdate` queries `(With<Predicted>, With<CharacterMarker>)`

**Shared** (`crates/protocol/src/lib.rs:170-214`):
- `apply_movement()` is the shared movement function used by both server and client

### 8. Component Registration Pattern

In `crates/protocol/src/lib.rs:102-128`:
```rust
app.register_component::<CharacterMarker>();
app.register_component::<LinearVelocity>()
    .add_prediction()
    .add_should_rollback(linear_velocity_should_rollback);
app.register_component::<Position>()
    .add_prediction()
    .add_should_rollback(position_should_rollback)
    .add_linear_correction_fn()
    .add_linear_interpolation();
```

Any new ability-related components that need prediction/replication must follow this pattern.

### 9. KeyCode for Number Keys

In Bevy 0.17 / leafwing 0.19, number keys use `KeyCode::Digit1`, `KeyCode::Digit2`, `KeyCode::Digit3`, `KeyCode::Digit4`.

### 10. ActionState Methods

From leafwing-input-manager:
- `action_state.just_pressed(&action)` - true on the tick the button was pressed
- `action_state.pressed(&action)` - true while held
- `action_state.just_released(&action)` - true on the tick released
- `action_state.clamped_value(&action)` - f32 for axis
- `action_state.axis_pair(&action)` - Vec2 for dual-axis

### 11. Vision Document Context

From `VISION.md` and `doc/scratch/stats.md`:
- Abilities have **stat requirements** (min stat thresholds to equip)
- 5 primary stats: Power, Agility, Stamina, Focus, Vitality
- Different stats unlock different ability categories (heavy strikes need Power, multi-hit strings need Agility, etc.)
- Brawlers are the characters, not the player directly
- Alignment (good/evil) can affect ability access

## Code References

- `crates/protocol/src/lib.rs:23-38` - PlayerActions enum
- `crates/protocol/src/lib.rs:71-128` - ProtocolPlugin (component registration, InputPlugin)
- `crates/protocol/src/lib.rs:170-214` - apply_movement shared function
- `crates/server/src/gameplay.rs:19-44` - Server movement system
- `crates/server/src/gameplay.rs:46-85` - Server character spawn
- `crates/client/src/gameplay.rs:16-49` - Client character setup (InputMap, physics)
- `crates/client/src/gameplay.rs:51-76` - Client movement system
- `git/lightyear/lightyear_inputs_leafwing/src/plugin.rs:18-82` - InputPlugin internals
- `git/lightyear/lightyear_inputs_leafwing/src/action_state.rs:9-38` - LeafwingUserAction trait
- `git/lightyear/lightyear_replication/src/prespawn.rs:187-233` - PreSpawned component
- `git/lightyear/lightyear_prediction/src/rollback.rs:235-243` - DisableRollback component
- `git/lightyear/lightyear_core/src/tick.rs:10` - Tick type definition
- `git/lightyear/lightyear_core/src/timeline.rs:106-113` - LocalTimeline
- `git/lightyear/examples/projectiles/src/shared.rs:236-346` - Weapon shooting with ticks
- `git/lightyear/examples/projectiles/src/shared.rs:1090-1401` - direction_only ProjectileSpawn pattern

## Architecture Documentation

### Current Plugin Structure
```
protocol/    SharedGameplayPlugin -> ProtocolPlugin (InputPlugin, component registration, physics)
server/      ServerGameplayPlugin (movement system, character spawn)
client/      ClientGameplayPlugin (InputMap, predicted movement)
```

### Current Prediction Flow
1. Client reads `ActionState<PlayerActions>` from leafwing
2. Lightyear buffers and sends inputs to server each tick
3. Both client (`With<Predicted>`) and server run `apply_movement` in `FixedUpdate`
4. Server's authoritative `Position`/`LinearVelocity` replicate back
5. Client rolls back and resimulates on mismatch (threshold-based `should_rollback`)

### ProjectileSpawn Prediction Pattern (from lightyear examples)
1. Shoot action triggers on both client and server (via replicated inputs)
2. Both spawn `(ProjectileSpawnComponent, PreSpawned::default())`
3. Server also adds `Replicate`, `PredictionTarget`, `ControlledBy`
4. A shared observer/system spawns child bullet with `DisableRollback`
5. On mismatch, the prespawned entity is despawned (taking children with it)

## Historical Context (from doc/)

- `doc/scratch/stats.md` - Detailed stat system design (Power, Agility, Stamina, Focus, Vitality) with ability requirement thresholds per stat
- `doc/scratch/vision-theorycrafting.md` - Brawler abilities have stat requirements; alignment affects ability access
- `VISION.md` - "Stats unlock abilities (stat requirements for moves)" and varied game modes beyond just combat

## Decisions (Resolved)

1. **Use `PlayerActions` enum.** Add `Ability1`–`Ability4` variants (all `InputControlKind::Button`) to the existing enum. Single enum is simpler and already proven in the codebase.

2. **Use `AbilitySlots([AbilityId; 4])` component.** Lives on the character entity, maps slot index to ability type. Replicated so the server knows what each slot maps to. Registered with `register_component::<AbilitySlots>()` (no prediction needed — slots don't change during gameplay).

---

## Follow-Up Research: Multi-Step Abilities & Rollback

### 12. Multi-Step Ability State Machine Design

**Fighting game standard**: Every attack has three phases — **Startup** (windup, before the hitbox is active), **Active** (hitbox active, can hit), **Recovery** (returning to neutral, vulnerable). Duration of each phase is measured in frames/ticks.

**Recommended component**: A single `ActiveAbility` component on the character entity, tracking the current ability execution state:

```rust
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ActiveAbility {
    pub ability_id: AbilityId,
    pub phase: AbilityPhase,
    pub phase_start_tick: Tick,
    pub step: u8,          // for multi-hit: which hit in the sequence (0-indexed)
    pub total_steps: u8,   // total hits in the sequence
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum AbilityPhase {
    Startup,
    Active,
    Recovery,
}
```

**Phase durations are data, not code.** Each ability definition specifies tick counts per phase:

```rust
pub struct AbilityDef {
    pub startup_ticks: u16,
    pub active_ticks: u16,
    pub recovery_ticks: u16,
    pub steps: u8,              // 1 for single-hit, >1 for multi-step
    pub step_window_ticks: u16, // ticks the player has to press again for next step
}
```

**Phase transitions** happen in a `FixedUpdate` system that checks `tick - phase_start_tick >= phase_duration`:

```rust
fn advance_ability_phase(
    mut query: Query<&mut ActiveAbility>,
    timeline: Single<&LocalTimeline, Without<ClientOf>>,
    ability_defs: Res<AbilityDefs>,
) {
    let tick = timeline.tick();
    for mut active in query.iter_mut() {
        let def = &ability_defs[active.ability_id];
        let elapsed = tick - active.phase_start_tick; // returns i16 (wrapping)
        match active.phase {
            AbilityPhase::Startup if elapsed >= def.startup_ticks as i16 => {
                active.phase = AbilityPhase::Active;
                active.phase_start_tick = tick;
            }
            AbilityPhase::Active if elapsed >= def.active_ticks as i16 => {
                active.phase = AbilityPhase::Recovery;
                active.phase_start_tick = tick;
            }
            AbilityPhase::Recovery if elapsed >= def.recovery_ticks as i16 => {
                // Ability complete — remove component or transition
            }
            _ => {}
        }
    }
}
```

**Multi-step (combo) abilities**: When `step < total_steps` and the player presses the ability key again during the `Active` or `Recovery` phase (within `step_window_ticks`), increment `step` and reset to `Startup` of the next step. If the window expires without a press, the ability ends.

**No external state machine crate needed.** `seldom_state` (0.15, Bevy 0.17 compatible) provides component-based state machines, but it's overkill here — the ability state is a simple enum with tick-driven transitions in a single system. More importantly, `seldom_state` isn't designed for rollback-compatible state management. A plain component with `PartialEq`-based rollback is the correct approach for lightyear.

**Bevy's `States`/`SubStates` are app-wide, not per-entity.** They cannot be used for per-character ability states.

### 13. Rollback Registration for Ability State

**Key insight from lightyear source**: Any component that participates in prediction needs the `SyncComponent` trait bound:

```
SyncComponent = Component + Clone + PartialEq + Debug + Serialize + Deserialize
```

**`ActiveAbility` registration** — use `register_component` + `add_prediction()`:

```rust
app.register_component::<ActiveAbility>()
    .add_prediction();
// No add_should_rollback needed — default uses PartialEq::ne, which works
// No add_linear_correction_fn — discrete state, not continuous
// No add_linear_interpolation — not visually interpolated
```

**Default rollback comparison uses `PartialEq::ne`**. When the server's `Confirmed<ActiveAbility>` differs from the client's predicted `ActiveAbility` at the confirmed tick, a rollback triggers. For an enum-based state, this is exactly correct — any difference in phase, step, or tick means a mismatch.

**Precedent**: The `Weapon` component in lightyear's spaceships demo uses this exact pattern:

```rust
// demos/spaceships/src/protocol.rs:119-124
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct Weapon {
    pub(crate) last_fire_tick: Tick,
    pub(crate) cooldown: u16,
    pub(crate) bullet_speed: f32,
}

// Registration:
app.register_component::<Weapon>().add_prediction();
```

No custom `should_rollback`, no correction function, no interpolation — just `add_prediction()`. The `Weapon` component stores tick-based cooldown state and participates in rollback via simple equality comparison.

**`AbilitySlots` does NOT need prediction.** It's a configuration component that doesn't change during gameplay. Register it with `register_component::<AbilitySlots>()` only (for replication, not prediction).

**`app.add_rollback::<C>()`** is for non-networked components that still need rollback (e.g., physics components in deterministic replication mode). Ability state IS networked, so use `register_component` + `add_prediction()` instead.

**History buffer**: `add_prediction()` registers a `PredictionHistory<C>` that stores per-tick component snapshots. On rollback, lightyear pops history to the confirmed tick, compares with the server's `Confirmed<C>`, and resimulates forward if they differ.

### 14. Cooldown Tracking

Two approaches, both tick-based. We will use **Option A**: 

**Option A: Field on `ActiveAbility`** — Track `last_used_tick: Tick` per ability slot. Check `tick - last_used_tick >= cooldown_ticks` before allowing activation. This lives on a separate component since `ActiveAbility` is removed when no ability is active.

**Option B: Dedicated `AbilityCooldowns` component**:

```rust
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AbilityCooldowns {
    pub last_used: [Tick; 4], // one per slot
}
```

Register with `add_prediction()` so cooldown state participates in rollback.

## Additional Code References (Follow-Up)

- `git/lightyear/demos/spaceships/src/protocol.rs:119-134` - Weapon component (tick-based cooldown with prediction)
- `git/lightyear/demos/spaceships/src/protocol.rs:191` - `app.register_component::<Weapon>().add_prediction()`
- `git/lightyear/lightyear_prediction/src/registry.rs` - PredictionRegistrationExt trait, SyncComponent bound
- `git/lightyear/lightyear_prediction/src/predicted_history.rs` - PredictionHistory<C> buffer
- `git/lightyear/lightyear_replication/src/components.rs:109` - `Confirmed<C>(pub C)` wrapper
- `git/lightyear/examples/deterministic_replication/src/protocol.rs:99-115` - `add_rollback` for non-networked components
