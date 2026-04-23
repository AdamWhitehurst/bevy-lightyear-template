# Research Findings

## Q1: Jump implementation in `crates/protocol/src/character/movement.rs`

**Direct answer:** `apply_movement` (a free function, not a system) is called by both client and server `handle_character_movement` systems. It detects `just_pressed(&PlayerActions::Jump)`, casts a 4.0-unit ray straight down from `Position.0` with a map-id predicate, and calls `forces.apply_linear_impulse(Vec3::new(0.0, 2000.0, 0.0))` on hit.

### Evidence

Input edge-detection:

```rust
// crates/protocol/src/character/movement.rs:25
if action_state.just_pressed(&PlayerActions::Jump) {
```

Inline ground check via avian `SpatialQuery::cast_ray_predicate`:

```rust
// crates/protocol/src/character/movement.rs:26-42
let ray_cast_origin = position.0;
let filter = SpatialQueryFilter::from_excluded_entities([entity]);

if spatial_query
    .cast_ray_predicate(
        ray_cast_origin,
        Dir3::NEG_Y,
        4.0,
        false,
        &filter,
        &|hit_entity| match (player_map_id, map_ids.get(hit_entity).ok()) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        },
    )
    .is_some()
```

- Origin is character-center `Position.0`, not feet.
- Direction: `Dir3::NEG_Y`.
- Max distance: `4.0`.
- Predicate filters by `MapInstanceId` — only hits entities in the same map instance (or any, if either side lacks a map id).
- `SpatialQueryFilter::from_excluded_entities([entity])` prevents self-hits.

Impulse application on hit:

```rust
// crates/protocol/src/character/movement.rs:44
forces.apply_linear_impulse(Vec3::new(0.0, 2000.0, 0.0));
```

`forces` is a `&mut ForcesItem` from avian's `Forces` QueryData. `apply_linear_impulse` computes `delta_vel = effective_inverse_mass * impulse` (accounting for locked axes) and immediately adds it to `LinearVelocity` that same tick (see `git/avian/src/dynamics/rigid_body/forces/query_data.rs:388-395`).

Horizontal movement uses a separate `apply_force` path:

```rust
// crates/protocol/src/character/movement.rs:64
forces.apply_force(required_acceleration * mass.value());
```

This accumulates into `VelocityIntegrationData.linear_increment` and is spread across solver substeps.

---

## Q2: End-to-end ability flow (input → effect)

**Direct answer:** Input presses of `Ability1`–`Ability4` are detected per-tick in `ability_activation`, which spawns an `ActiveAbility` entity carrying reflected effect components from the ability's `.ron` asset. `update_active_abilities` advances the entity through `Startup → Active → Recovery` phases. Four effect systems (`apply_on_tick_effects`, `apply_while_active_effects`, `apply_on_end_effects`, `apply_on_input_effects`) read the phase and fire per-variant side effects onto the world; `apply_on_hit_effects` is fired by the hit-detection chain.

### Evidence

Four hardcoded input slots (no remapping):

```rust
// crates/protocol/src/ability/activation.rs:17-22
const ABILITY_ACTIONS: [PlayerActions; 4] = [
    PlayerActions::Ability1,
    PlayerActions::Ability2,
    PlayerActions::Ability3,
    PlayerActions::Ability4,
];
```

`ability_activation` checks each slot for `just_pressed` and performs 6 ordered guards before spawning (`crates/protocol/src/ability/activation.rs:52-75`):

1. `action_state.just_pressed(action)` — skip if not pressed
2. `slots.0[slot_idx]` — skip if the slot is empty
3. `ability_defs.get(ability_id)` — `warn!` and skip if no handle
4. `ability_assets.get(handle)` — `warn!` and skip if asset not loaded
5. `extract_phases(asset)` — `warn!` and skip if asset lacks `AbilityPhases`
6. `cooldowns.is_on_cooldown(slot_idx, tick, phases.cooldown)` — silent skip

On pass, cooldown is stamped and an entity is spawned (`activation.rs:77-101`):

```rust
// crates/protocol/src/ability/activation.rs:86-101
let entity_id = commands
    .spawn((
        ActiveAbility { ... },
        PreSpawned::default_with_salt(salt),
        Name::new("ActiveAbility"),
    ))
    .id();
apply_ability_archetype(&mut commands, entity_id, ability_id, asset, &registry);
if let Ok(controlled_by) = server_query.get(entity) {
    commands.entity(entity_id).insert((
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
        *controlled_by,
    ));
}
```

`update_active_abilities` progresses state via `advance_ability_phase` (`crates/protocol/src/ability/activation.rs:116-143`):

| From | To | Side effect |
|---|---|---|
| `Startup` | `Active` | `phase_start_tick = tick`; insert `OnHitEffects` if `OnHitEffectDefs` present |
| `Active` | `Recovery` | `phase_start_tick = tick`; remove `OnHitEffects` |
| `Recovery` | despawn | `commands.entity(entity).prediction_despawn()` |

Four effect systems that read phase state (`crates/protocol/src/ability/effects.rs`):

| System | File:line | Fires when | Handles variants |
|---|---|---|---|
| `apply_on_tick_effects` | `effects.rs:28` | `Active` && `tick.elapsed == tick_effect.tick` | `Melee`, `AreaOfEffect`, `Projectile`, `Ability`, `Teleport`, `Shield`, `Buff` |
| `apply_while_active_effects` | `effects.rs:139` | every tick during `Active` | `SetVelocity` |
| `apply_on_end_effects` | `effects.rs:165` | exact tick of `Active → Recovery` | `SetVelocity`, `Ability`, `Teleport`, `Shield`, `Buff` |
| `apply_on_input_effects` | `effects.rs:248` | during `Active`, on caster's `just_pressed(variant)` | `Ability` (sub-spawn) |

`apply_on_hit_effects` is invoked by `process_hitbox_hits` (`hit_detection/systems.rs:34`) and `process_projectile_hits` (`hit_detection/systems.rs:110`), iterating `on_hit.effects` (`hit_detection/effects.rs:62`):

| Variant | Behavior |
|---|---|
| `Damage` | apply `ActiveBuffs("damage")` multiplier; drain `ActiveShield` first; decrement `Health`; emit `DeathEvent` on zero |
| `ApplyForce` | resolve `ForceFrame`; `velocity.0 += world_force` |
| `Ability` | `spawn_sub_ability` |

---

## Q3: Ability data schema and loading

**Direct answer:** Abilities are `.ability.ron` files with `#![enable(implicit_some)]`, deserialized into a flat `{ "type::path": (data) }` component map via `reflect_loader::deserialize_component_map`. `AbilityAsset` is literally `Vec<Box<dyn PartialReflect>>` — the schema is defined by which component types appear as keys.

### Evidence

`AbilityAsset` is type-erased:

```rust
// crates/protocol/src/ability/types.rs:330-332
#[derive(Asset, TypePath)]
pub struct AbilityAsset {
    pub components: Vec<Box<dyn PartialReflect>>,
}
```

Recognized component keys (registered in `crates/protocol/src/ability/plugin.rs:34-45`):

| RON key | Rust type | Fields |
|---|---|---|
| `protocol::ability::AbilityPhases` | `AbilityPhases` | `startup: u16, active: u16, recovery: u16, cooldown: u16` |
| `protocol::ability::OnTickEffects` | `OnTickEffects(Vec<TickEffect>)` | each `tick: u16, effect: AbilityEffect` |
| `protocol::ability::WhileActiveEffects` | `WhileActiveEffects(Vec<AbilityEffect>)` | list |
| `protocol::ability::OnHitEffectDefs` | `OnHitEffectDefs(Vec<AbilityEffect>)` | list |
| `protocol::ability::OnEndEffects` | `OnEndEffects(Vec<AbilityEffect>)` | list |
| `protocol::ability::OnInputEffects` | `OnInputEffects(Vec<InputEffect>)` | each `action: PlayerActions, effect: AbilityEffect` |

`AbilityPhases` is mandatory; all others are optional.

`AbilityEffect` variants (`crates/protocol/src/ability/types.rs:47-99`):

| Variant | Fields |
|---|---|
| `Melee` | `id: Option<String>`, `target: EffectTarget` |
| `Projectile` | `id: Option<String>`, `speed: f32`, `lifetime_ticks: u16` |
| `SetVelocity` | `speed: f32`, `target: EffectTarget` |
| `Damage` | `amount: f32`, `target: EffectTarget` |
| `ApplyForce` | `force: Vec3`, `frame: ForceFrame`, `target: EffectTarget` |
| `AreaOfEffect` | `id: Option<String>`, `target: EffectTarget`, `radius: f32`, `duration_ticks: Option<u16>` |
| `Ability` | `id: String`, `target: EffectTarget` |
| `Teleport` | `distance: f32` |
| `Shield` | `absorb: f32` |
| `Buff` | `stat: String`, `multiplier: f32`, `duration_ticks: u16`, `target: EffectTarget` |

Support enums: `EffectTarget { Caster, Victim, OriginalCaster }` (`types.rs:25-30`); `ForceFrame { World, Caster, Victim, RelativePosition, RelativeRotation }` (`types.rs:35-42`).

Full example:

```ron
// assets/abilities/punch.ability.ron
#![enable(implicit_some)]
{
    "protocol::ability::AbilityPhases": (startup: 20, active: 10, recovery: 100, cooldown: 16),
    "protocol::ability::OnTickEffects": ([(tick: 1, effect: Melee())]),
    "protocol::ability::OnHitEffectDefs": ([
        Damage(amount: 5.0, target: Victim),
        ApplyForce(force: (0.0, 0.9, 0.5), frame: RelativePosition, target: Victim),
    ]),
    "protocol::ability::OnInputEffects": ([(action: Ability1, effect: Ability(id: "punch2", target: Caster))]),
}
```

Loading flow:

1. `Startup`: `load_ability_defs` calls `asset_server.load_folder("abilities")` (native, `loading.rs:32`) or loads `abilities.manifest.ron` (WASM, `loading.rs:43`).
2. `Update`: `insert_ability_defs` builds `AbilityDefs { abilities: HashMap<AbilityId, Handle<AbilityAsset>> }` once loading finishes. `AbilityId` is `filename.strip_suffix(".ability.ron")` (`loading.rs:223`).
3. Per-file load runs through `AbilityAssetLoader` (`loader.rs:72`) calling `deserialize_component_map`:

```rust
// crates/protocol/src/reflect_loader.rs:7-8 (structural shape)
// Deserializes a { "type::path": (..fields..) } map into Vec<Box<dyn PartialReflect>>.
```

4. At activation, `apply_ability_archetype` (`loader.rs:24`) walks each reflected component, looks it up in `AppTypeRegistry`, and calls `reflect_component.insert(&mut entity_mut, ...)` (`loader.rs:53`).

All RON files under `assets/`:

| File | Role |
|---|---|
| `assets/abilities/punch.ability.ron` | slot 0 default |
| `assets/abilities/punch2.ability.ron` | sub-ability (chained from punch) |
| `assets/abilities/punch3.ability.ron` | sub-ability (chained from punch2) |
| `assets/abilities/speed_burst.ability.ron` | slot 1 default |
| `assets/abilities/ground_pound.ability.ron` | slot 2 default |
| `assets/abilities/blink_strike.ability.ron` | slot 3 default |
| `assets/abilities/uppercut.ability.ron` | bundled |
| `assets/abilities/shockwave.ability.ron` | bundled |
| `assets/abilities/dive_kick.ability.ron` | bundled |
| `assets/abilities/fireball.ability.ron` | bundled |
| `assets/abilities/teleport_burst.ability.ron` | bundled |
| `assets/abilities/shield_bash.ability.ron` | bundled |
| `assets/default.ability_slots.ron` | default slot-loadout resource (`.ability_slots.ron` via `RonAssetPlugin`, see `plugin.rs:50-51`) |

---

## Q4: Marker-style component conventions

**Direct answer:** No component uses `#[component(storage = "SparseSet")]` anywhere in the workspace — every marker uses Bevy's default table storage. There are no comments discussing storage strategy. Markers are declared as bare unit structs with `#[derive(Component)]`, inserted via spawn tuples or `commands.entity(…).insert((…))`, removed via `.remove::<(…)>()` or `.try_remove::<T>()`.

### Evidence

Grep for `SparseSet` / `storage = ` across the workspace returns zero hits. Every marker inspected is a plain `#[derive(Component)]` unit struct.

Marker inventory (selection; unit-struct markers):

| Type | File:line | Insert sites | Remove sites |
|---|---|---|---|
| `CharacterMarker` | `crates/protocol/src/character/types.rs:20` | `server/src/gameplay.rs:407` via spawn tuple | (none — lives for entity lifetime) |
| `DummyTarget` | `crates/protocol/src/character/types.rs:24` | commented at `server/src/gameplay.rs:101` | — |
| `RespawnPoint` | `crates/protocol/src/character/types.rs:38` (with `#[require(MapSaveTarget)]`) | `server/src/gameplay.rs:148`, `server/src/map.rs:551` | — |
| `MapSaveTarget` | `crates/protocol/src/map/persistence.rs:6` | auto via `#[require(…)]` on `RespawnPoint` | — |
| `TransitionReadySent` | `crates/protocol/src/map/transition.rs:47` | — | `client/src/transition.rs:275` |
| `MeleeHitbox` | `crates/protocol/src/ability/types.rs:258` | `ability/spawn.rs:133` via `apply_ability_archetype` + tuple | cleanup systems |
| `JointRoot` | `crates/sprite_rig/src/spawn.rs:42` | `sprite_rig/src/spawn.rs:177` | — |
| `HealthBarRoot` | `crates/render/src/health_bar.rs:8` | `render/src/health_bar.rs:76` | — |
| `HealthBarForeground` | `crates/render/src/health_bar.rs:11` | `render/src/health_bar.rs:88` | — |
| `ConnectButton` / `QuitButton` / `MainMenuButton` / `CancelButton` / `MapSwitchButton` | `crates/ui/src/components.rs:5,8,12,15,19` | spawn tuples in `ui/src/lib.rs` | — |
| `LoadingScreenText` | `crates/ui/src/lib.rs:547` | `ui/src/lib.rs:565` | — |

Insert / remove patterns seen in the codebase:

```rust
// server/src/gameplay.rs:391-416 (tuple insert on spawn)
commands.spawn((
    CharacterMarker,
    ...
))
```

```rust
// server/src/transition.rs:64-69 (post-spawn conditional insert, bulk)
commands.entity(player_entity).insert((
    DisableRollback,       // from lightyear
    ColliderDisabled,      // from avian3d
    RigidBodyDisabled,     // from avian3d
    PendingTransition(target_map_id.clone()),
));
```

```rust
// server/src/transition.rs:140-145 (bulk remove)
commands.entity(entity).remove::<(
    RigidBodyDisabled,
    ColliderDisabled,
    DisableRollback,
    PendingTransition,
)>();
```

```rust
// protocol/src/ability/lifecycle.rs:30-39 (panic-free remove in observer)
cmd.try_remove::<OnTickEffects>();
cmd.try_remove::<WhileActiveEffects>();
cmd.try_remove::<OnHitEffects>();
```

```rust
// client/src/gameplay.rs:119-148 (observer-driven toggle)
fn on_respawn_timer_added(trigger: On<Add, RespawnTimer>, mut commands: Commands, ...) {
    commands.entity(entity).insert((Visibility::Hidden, RigidBodyDisabled, ColliderDisabled));
}
fn on_respawn_timer_removed(trigger: On<Remove, RespawnTimer>, mut commands: Commands, ...) {
    commands.entity(entity).remove::<(RigidBodyDisabled, ColliderDisabled)>().insert(Visibility::Inherited);
}
```

Auto-insert via required components:

```rust
// protocol/src/character/types.rs:38-39
#[derive(Component, Clone, Debug)]
#[require(MapSaveTarget)]
pub struct RespawnPoint;
```

---

## Q5: Activation-time gates on abilities

**Direct answer:** Exactly one gameplay gate exists — the cooldown check at `activation.rs:73`. Five additional checks in `ability_activation` are data-integrity guards (slot empty / def missing / asset not loaded / `AbilityPhases` missing). There are no state checks (e.g., grounded/airborne), no resource costs, no targeting prereqs, and no health checks. The failure outcome for the cooldown gate is a silent `continue` — no event is emitted.

### Evidence

Cooldown gate:

```rust
// crates/protocol/src/ability/activation.rs:73
if cooldowns.is_on_cooldown(slot_idx, tick, phases.cooldown) {
    continue;
}
```

```rust
// crates/protocol/src/ability/types.rs:233-237
pub fn is_on_cooldown(&self, slot: usize, current_tick: Tick, cooldown_ticks: u16) -> bool {
    self.last_used[slot]
        .map(|last| (current_tick - last).unsigned_abs() <= cooldown_ticks)
        .unwrap_or(false)
}
```

`last_used: [Option<Tick>; 4]` starts as `[None; 4]` (`types.rs:226`), so the first press per slot is never blocked. On pass, `cooldowns.last_used[slot_idx] = Some(tick)` is written (`activation.rs:77`) before the spawn.

The remaining four guards (all warn-and-skip, not gameplay gates) are data-integrity:

| # | Check | File:line | Failure |
|---|---|---|---|
| 1 | `slots.0[slot_idx]` populated | `activation.rs:58` | silent `continue` |
| 2 | `ability_defs.get(ability_id)` | `activation.rs:61` | `warn!` + `continue` |
| 3 | `ability_assets.get(handle)` | `activation.rs:64` | `warn!` + `continue` |
| 4 | `extract_phases(asset)` returns Some | `activation.rs:68` | `warn!` + `continue` |

No other gates. No `AbilityActivationFailed` event or analogous signal exists. The ordered sequence at `activation.rs:54-75` (pressed → slot → def → asset → phases → cooldown) is the complete activation guard chain, and is the structural location where additional gates would sit.

---

## Q6: System ordering for physics-adjacent systems

**Direct answer:** Physics integration runs in `FixedPostUpdate` via the `LightyearAvianPlugin` (with `AvianReplicationMode::Position`), so every gameplay system in `FixedUpdate` — including `handle_character_movement` and the entire ability pipeline — executes before physics. Within `FixedUpdate`, `handle_character_movement` has **no declared ordering** relative to the ability pipeline. The ability pipeline is itself `.chain()`-ed.

### Evidence

Physics chain (from lightyear_avian):

```rust
// git/lightyear/lightyear_avian/src/plugin.rs:156-166
// FixedPostUpdate chain:
// PhysicsSystems::StepSimulation → PredictionSystems::UpdateHistory → FrameInterpolationSystems::Update
```

`PhysicsSystems::StepSimulation` expands to avian's full step (force-increment computation → substep integration → collision solve → position integration). The plugin is added at `crates/protocol/src/lib.rs:240`.

Ability pipeline chain (both client and server via `AbilityPlugin`):

```rust
// crates/protocol/src/ability/plugin.rs:82-94
// FixedUpdate chain with .run_if(ready):
//   ability_activation
// → update_active_abilities
// → apply_on_tick_effects
// → apply_while_active_effects
// → apply_on_end_effects
// → apply_on_input_effects
// → ability_projectile_spawn
```

Hit detection runs after `apply_on_tick_effects` (`plugin.rs:98-107`):

```
update_hitbox_positions → process_hitbox_hits → process_projectile_hits → cleanup_hitbox_entities
```

Lifetime / buff expiry chains after hit detection (`plugin.rs:109-115`).

Scheduling primitives inventory:

| Site | Primitive |
|---|---|
| `ability/plugin.rs:82-94` | `.chain()` inside ability pipeline |
| `ability/plugin.rs:98-107` | `.after(apply_on_tick_effects)` |
| `ability/plugin.rs:109-115` | `.after(process_hitbox_hits).after(process_projectile_hits)` |
| `lightyear_avian plugin.rs:156-166` | `.chain()` in `FixedPostUpdate` |
| `client/src/gameplay.rs:22-23` | `.before(InputSystems::BufferClientInputs)` for `sync_camera_yaw_to_input` (in `FixedPreUpdate`) |
| `server/src/gameplay.rs:37-53` | `.after(process_projectile_hits).after(process_hitbox_hits)` for death/respawn |

Client `handle_character_movement` is registered at `client/src/gameplay.rs:21` with no ordering constraint; server `handle_character_movement` at `server/src/gameplay.rs:26-54` similarly has none relative to the ability pipeline.

---

## Q7: Ground-contact APIs in use

**Direct answer:** The only ground-contact mechanism is the single `cast_ray_predicate` call inside `apply_movement`. `cast_shape`, `shape_cast`, contact events, and contact-pair queries are not used for ground detection anywhere in the codebase.

### Evidence

Complete inventory of avian query APIs in use:

| API | File:line | Purpose |
|---|---|---|
| `SpatialQuery::cast_ray_predicate` | `crates/protocol/src/character/movement.rs:31-42` | Jump ground check (4.0 units down from body center) |
| `CollidingEntities` (read in system) | `crates/protocol/src/ability/hit_detection/systems.rs:42-46`, `119-122` | Hitbox / projectile hit resolution (NOT ground-related) |
| `MapCollisionHooks::filter_pairs` | `crates/protocol/src/physics.rs:15-23` | Per-pair collision filter by map instance (NOT a query) |

No other ground-detection path exists. No `Grounded` component, no ground-sensor entity, no air-time counter found.

---

## Q8: `PlayerActions` input enum and routing

**Direct answer:** `PlayerActions` is an `Actionlike` enum with 9 variants: `Move` (DualAxis), `CameraYaw` (Axis), `Jump`, `PlaceVoxel`, `RemoveVoxel`, `Ability1`–`Ability4` (all Button). Bindings are inserted client-side only in `handle_new_character`. Reads happen in shared `protocol` code (`movement.rs` for movement/jump/facing, `ability/activation.rs` for ability slot triggers, `ability/effects.rs` for active-phase inputs). `PlaceVoxel`/`RemoveVoxel` have key bindings but no reader.

### Evidence

```rust
// crates/protocol/src/lib.rs:59-80
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy, Hash, Reflect)]
pub enum PlayerActions {
    Move,
    CameraYaw,
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
            Self::CameraYaw => InputControlKind::Axis,
            _ => InputControlKind::Button,
        }
    }
}
```

Input bindings (client-side only, inserted on `Controlled` character):

```rust
// crates/client/src/gameplay.rs:51-62
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

Variant → reader table:

| Variant | Reader (system / function) | File:line | Side |
|---|---|---|---|
| `Move` | `apply_movement` (`axis_pair`) | `protocol/src/character/movement.rs:49-53` | shared |
| `Move` | `update_facing` (`axis_pair`) | `protocol/src/character/movement.rs:74-83` | shared |
| `CameraYaw` | `sync_camera_yaw_to_input` (writer) | `client/src/gameplay.rs:176` | client |
| `CameraYaw` | `apply_movement` (`value`) | `protocol/src/character/movement.rs:52` | shared |
| `CameraYaw` | `update_facing` | `protocol/src/character/movement.rs:77` | shared |
| `Jump` | `apply_movement` (`just_pressed`) | `protocol/src/character/movement.rs:25` | shared |
| `PlaceVoxel` | (no reader) | — | — |
| `RemoveVoxel` | (no reader) | — | — |
| `Ability1`..`Ability4` | `ability_activation` | `protocol/src/ability/activation.rs:54-55` | shared |
| any variant | `apply_on_input_effects` (configurable via `OnInputEffects` RON) | `protocol/src/ability/effects.rs:268` | shared |

Client vs. server `handle_character_movement` (both in `FixedUpdate`, both call `apply_movement`):

| | Client | Server |
|---|---|---|
| File:line | `client/src/gameplay.rs:83-116` | `server/src/gameplay.rs:106-135` |
| Query filter | `(With<Predicted>, With<CharacterMarker>, Without<RespawnTimer>)` | `(With<CharacterMarker>, Without<RespawnTimer>)` |
| Body | `apply_movement(...)` identical call | `apply_movement(...)` identical call |

Ability activation trigger (shared, in `AbilityPlugin`, runs on both sides):

```rust
// crates/protocol/src/ability/activation.rs:54-55 (within ability_activation)
for (slot_idx, action) in ABILITY_ACTIONS.iter().enumerate() {
    if !action_state.just_pressed(action) { continue; }
    ...
}
```

Ability1–Ability4 are the only slot-bound activation triggers; `apply_on_input_effects` can additionally watch an arbitrary `PlayerActions` variant during an ability's `Active` phase (RON-configured per-ability).

---

## Q9: How abilities modify velocity vs. `movement.rs`

**Direct answer:** `movement.rs` writes through avian's `Forces` QueryData (`apply_linear_impulse` for jump, `apply_force` for horizontal). Ability effects instead write `LinearVelocity` **directly**: `SetVelocity` assigns components; `ApplyForce` does `velocity.0 += world_force`. Both paths target the same replicated `LinearVelocity` component, and within `FixedUpdate` there is **no declared ordering** between `handle_character_movement` and any ability effect system — an uncontended write race exists today, only avoided by the fact that ability effects on character velocity run on character entities that ability systems can touch independent of inputs the movement system reads.

### Evidence

`movement.rs` via `Forces` QueryData:

```rust
// crates/protocol/src/character/movement.rs:44
forces.apply_linear_impulse(Vec3::new(0.0, 2000.0, 0.0));
// crates/protocol/src/character/movement.rs:64
forces.apply_force(required_acceleration * mass.value());
```

`Forces` requires (`git/avian/src/dynamics/rigid_body/forces/query_data.rs`): `Position`, `Rotation`, `LinearVelocity` (mut), `AngularVelocity` (mut), `ComputedMass`, `ComputedAngularInertia`, `ComputedCenterOfMass`, `Option<LockedAxes>`, `VelocityIntegrationData` (mut), `AccumulatedLocalAcceleration` (mut), `Option<SleepTimer>` (mut), `Has<Sleeping>`.

Ability `SetVelocity` (in `apply_while_active_effects`):

```rust
// crates/protocol/src/ability/effects.rs:141 (within while-active loop)
// Query<(&Rotation, &mut LinearVelocity)>, assigns:
velocity.x = direction.x * speed;
velocity.z = direction.z * speed;
```

Ability `SetVelocity` in `apply_on_end_effects`:

```rust
// crates/protocol/src/ability/effects.rs:174
// Query<(&mut Position, &Rotation, &mut LinearVelocity)>, same assignment pattern
```

Ability `ApplyForce` in `apply_on_hit_effects`:

```rust
// crates/protocol/src/ability/hit_detection/effects.rs:108-129
// Query<(&Position, Option<&mut LinearVelocity>, &mut Health, Option<&Invulnerable>)>
if let Some(mut velocity) = velocity {
    velocity.0 += world_force;
}
```

Both paths converge on `LinearVelocity`. Within `FixedUpdate`:

- `handle_character_movement` — no explicit ordering
- `ability_activation` → … → `apply_while_active_effects` → `apply_on_end_effects` → … (chained with each other, but unordered vs. movement)
- `process_hitbox_hits` / `process_projectile_hits` — `.after(apply_on_tick_effects)`, unordered vs. movement

The `Forces` QueryData also mutates `VelocityIntegrationData`. Any future move of `ApplyForce` to `apply_linear_impulse` would create a `VelocityIntegrationData` mutable-access conflict with `handle_character_movement`, forcing an explicit `before`/`after`.

---

## Q10: Networking integration for the ability system

**Direct answer:** `AbilityPlugin` is shared and runs every system on both client and server. `ActiveAbility`, `AbilityCooldowns`, `ActiveShield`, and `ActiveBuffs` are predicted; `AbilitySlots` and `AbilityProjectileSpawn` are replicated without prediction. Ability RON-archetype components (`OnTickEffects`, etc.) are reflection-registered only, not lightyear-registered, and are inserted independently on both sides from the asset. Entity ID parity between client prediction and server uses `PreSpawned::default_with_salt(salt)` where `salt = (player_id.bits() << 32) | (slot_idx << 16)`. Hitboxes and bullets are `DisableRollback`'d and spawned locally from replicated anchors.

### Evidence

All registrations live in `ProtocolPlugin::build` (`crates/protocol/src/lib.rs`):

| Component | Call | Line | Predicted | Rollback fn | `add_map_entities` |
|---|---|---|---|---|---|
| `AbilitySlots` | `.register_component::<AbilitySlots>()` | 186 | no | — | no |
| `ActiveAbility` | `.register_component::<ActiveAbility>().add_prediction().add_map_entities()` | 187-189 | yes | default | yes |
| `AbilityCooldowns` | `.register_component::<AbilityCooldowns>().add_prediction()` | 190-191 | yes | default | no |
| `ActiveShield` | `.register_component::<ActiveShield>().add_prediction()` | 192-193 | yes | default | no |
| `ActiveBuffs` | `.register_component::<ActiveBuffs>().add_prediction()` | 194-195 | yes | default | no |
| `AbilityProjectileSpawn` | `.register_component::<AbilityProjectileSpawn>()` | 196 | no | — | no |

Contrast — movement/physics components (same file):

| Component | Call | Lines | Rollback fn | Interpolation |
|---|---|---|---|---|
| `LinearVelocity` | `.add_prediction().add_should_rollback(linear_velocity_should_rollback)` | 177-179 | ≥0.01 length delta | no |
| `AngularVelocity` | `.add_prediction().add_should_rollback(angular_velocity_should_rollback)` | 181-183 | ≥0.01 length delta | no |
| `Position` | `.add_prediction().add_should_rollback(...).add_linear_correction_fn().add_linear_interpolation()` | 197-201 | ≥0.01 length delta | yes (visual + interp) |
| `Rotation` | `.add_prediction().add_should_rollback(...).add_linear_correction_fn().add_linear_interpolation()` | 203-207 | ≥0.01 angle delta | yes (visual + interp) |

Activation uses PreSpawned + conditional server-side Replicate:

```rust
// crates/protocol/src/ability/activation.rs:86-111
let entity_id = commands
    .spawn((
        ActiveAbility { ... },
        PreSpawned::default_with_salt(salt),
        Name::new("ActiveAbility"),
    ))
    .id();
apply_ability_archetype(&mut commands, entity_id, ability_id, asset, &registry);

if let Ok(controlled_by) = server_query.get(entity) {
    commands.entity(entity_id).insert((
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
        *controlled_by,
    ));
}
```

`server_query: Query<&ControlledBy>` is present only on server-side entities → the `Replicate`/`PredictionTarget` insert branch runs only on the server.

Hitbox / bullet physics entities carry `DisableRollback` and are never replicated:

```rust
// crates/protocol/src/ability/spawn.rs:133
DisableRollback,
MeleeHitbox,
```

Bullets are created locally from the replicated `AbilityProjectileSpawn` anchor in `handle_ability_projectile_spawn` (`spawn.rs:245-278`, filters `Without<Replicated>`).

System execution context:

| System | File:line | Schedule | Runs on |
|---|---|---|---|
| `ability_activation` | `ability/plugin.rs:82` | FixedUpdate | both |
| `update_active_abilities` | `ability/plugin.rs:82` | FixedUpdate | both |
| `apply_on_tick_effects` | `ability/plugin.rs:82` | FixedUpdate | both |
| `apply_while_active_effects` | `ability/plugin.rs:82` | FixedUpdate | both |
| `apply_on_end_effects` | `ability/plugin.rs:82` | FixedUpdate | both |
| `apply_on_input_effects` | `ability/plugin.rs:82` | FixedUpdate | both |
| `ability_projectile_spawn` | `ability/plugin.rs:82` | FixedUpdate | both |
| `handle_ability_projectile_spawn` | `ability/plugin.rs` | PreUpdate | both |
| `handle_character_movement` (client) | `client/src/gameplay.rs:83` | FixedUpdate | client-only (`With<Predicted>`) |
| `handle_character_movement` (server) | `server/src/gameplay.rs:106` | FixedUpdate | server-only (no Predicted filter; all CharacterMarker) |

Ability entity despawn uses lightyear's rollback-aware path:

```rust
// crates/protocol/src/ability/activation.rs:116-143 (in advance_ability_phase, Recovery end)
commands.entity(entity).prediction_despawn()
```

No rollback / prediction conventions documented in `CLAUDE.md`. Only `README.md:108` mentions "Data-driven abilities loaded from RON assets with networked replication" (no detail).

Summary buckets:

- **Replicated + predicted**: `ActiveAbility`, `AbilityCooldowns`, `ActiveShield`, `ActiveBuffs`
- **Replicated, not predicted**: `AbilitySlots`, `AbilityProjectileSpawn`
- **Local-only, rollback-disabled**: hitboxes (`MeleeHitbox`, AoE hitboxes), bullet physics entities (`AbilityBulletOf`)
- **Reflection-only (never networked)**: `AbilityPhases`, `OnTickEffects`, `WhileActiveEffects`, `OnHitEffectDefs`, `OnEndEffects`, `OnInputEffects` — registered in `ability/plugin.rs:34-45` via `register_type`, inserted on both sides from the `.ability.ron` asset by `apply_ability_archetype`.

---

## Q11: What it would take to switch `ApplyForce` to `forces.apply_linear_impulse`

**Direct answer:** Four layers change — query shape, entity targeting, RON numeric scale (units become impulse = N·s = kg·m/s), and schedule ordering. Networking behavior is unchanged (both end up mutating the same predicted `LinearVelocity`). The structural hurdle is that `apply_on_hit_effects` currently receives the victim via a non-`Forces` query (`Option<&mut LinearVelocity>`, `Health`, `Invulnerable`); switching to `apply_linear_impulse` requires adding a `Forces`-typed query or restructuring the caller to obtain a `ForcesItem` for the victim entity without aliasing.

### Evidence

Current `ApplyForce` path (victim-targeting) vs. movement (self-targeting):

| Dimension | Current `ApplyForce` | `movement.rs` |
|---|---|---|
| Caller | `process_hitbox_hits`, `process_projectile_hits` (hit-detection chain, `.after(apply_on_tick_effects)`) | `handle_character_movement` (no ordering) |
| Query | `Query<(&Position, Option<&mut LinearVelocity>, &mut Health, Option<&Invulnerable>)>` accessed with `.get_mut(entity)` | `Forces` QueryData iterated per character |
| Entity target | Victim (resolved via `resolve_on_hit_target`) | Self |
| Mutation | `velocity.0 += world_force` (direct, no mass scaling) | `forces.apply_linear_impulse(imp)` computes `delta_vel = imp / mass` with locked-axis masking |

Target-entity component availability: character entities carry `CharacterPhysicsBundle` with `RigidBody::Dynamic`; avian auto-inserts `ComputedMass`, `ComputedAngularInertia`, `ComputedCenterOfMass`, `LinearVelocity`, `AngularVelocity`, and `VelocityIntegrationData` (as required components of `SolverBody`). So all components required by the `Forces` QueryData are present on characters. The blocker is **not** entity-shape: it is query structure, because Bevy forbids two mutable borrows of the same component from different query parameters for overlapping entity sets — adding `Forces` as a second query parameter would conflict with the existing `Option<&mut LinearVelocity>` parameter and would need that parameter removed/folded into the `Forces` path.

RON values in the repo that currently encode velocity deltas:

| File | `force` Vec3 | `frame` |
|---|---|---|
| `assets/abilities/uppercut.ability.ron` | `(0.0, 14.0, 0.0)` | `World` |
| `assets/abilities/ground_pound.ability.ron` | `(0.0, 8.0, 8.0)` | `RelativePosition` |
| `assets/abilities/shockwave.ability.ron` | `(0.0, 1.5, 8.0)` | `RelativePosition` |
| `assets/abilities/dive_kick.ability.ron` | `(0.0, 0.5, 3.0)` | `RelativePosition` |
| `assets/abilities/punch.ability.ron` | `(0.0, 0.9, 0.5)` | `RelativePosition` |
| `assets/abilities/punch2.ability.ron` | `(0.0, 1.05, 0.5)` | `RelativePosition` |
| `assets/abilities/punch3.ability.ron` | `(0.0, 2.4, 7.65)` | `RelativePosition` |
| `assets/abilities/fireball.ability.ron` | `(0.0, 2.4, 7.65)` | `RelativePosition` |
| `assets/abilities/blink_strike.ability.ron` | `(0.0, 1.2, 4.0)` | `RelativePosition` |
| `assets/abilities/teleport_burst.ability.ron` | `(0.0, 2.0, 5.0)` | `RelativePosition` |
| `assets/abilities/shield_bash.ability.ron` | `(0.0, 1.0, 5.5)` | `RelativePosition` |

Because `apply_linear_impulse` divides by `ComputedMass`, every value must be re-scaled by victim mass to preserve the same velocity delta. Character mass is derived from `Collider::capsule(radius: 2.0, height: 2.0)` at default `ColliderDensity`. `movement.rs:44` uses `2000.0` as the jump impulse, producing a velocity delta of `2000 / mass` — a reference point for what mass-scaled character impulses look like in this codebase.

Networking behavior:

```rust
// crates/protocol/src/lib.rs:177-179
app.register_component::<LinearVelocity>()
    .add_prediction()
    .add_should_rollback(linear_velocity_should_rollback);
```

Both write paths mutate the same predicted `LinearVelocity`. `forces.apply_linear_impulse` calls through `ForcesItem::linear_velocity_mut()` which dereferences to the same `LinearVelocity` component. Prediction/rollback behavior is identical: both show up in the prediction history, both trigger rollback on ≥0.01 length divergence.

Schedule ordering: `handle_character_movement` (in `Forces` QueryData, mutating `VelocityIntegrationData`) and `process_hitbox_hits`/`process_projectile_hits` currently have no declared ordering relative to each other. `apply_linear_impulse` mutates `VelocityIntegrationData` through `ForcesItem`. Switching would introduce a `VelocityIntegrationData` write conflict with `handle_character_movement`, forcing an explicit `.before(handle_character_movement)` or `.after(handle_character_movement)` on the hit-detection systems (or equivalent via system sets).

---

## Cross-Cutting Observations

- **No storage attributes used.** Every component is default (table) storage; `#[component(storage = "SparseSet")]` never appears. Markers are plain unit structs.
- **Shared protocol runs on both sides.** `AbilityPlugin` and its systems execute on both client (for prediction) and server (authoritative). Client/server divergence is expressed through query filters (`With<Predicted>` on client, absence of filter on server) and conditional component insertion (`server_query.get(entity)` gates `Replicate` insertion).
- **PreSpawned + deterministic salt** is the established pattern for client/server entity parity for ability-spawned entities (`ActiveAbility`, sub-abilities, projectile anchors).
- **DisableRollback for transient physics entities**: hitboxes and bullets are local and rollback-disabled; they are spawned from replicated "anchor" components (`AbilityProjectileSpawn`) on both sides.
- **`LinearVelocity` is a shared write target** without declared ordering between movement code and ability effects. `movement.rs` uses the avian `Forces` abstraction (mass-aware); ability effects write raw `LinearVelocity` (mass-unaware).
- **One gate, one signal**: activation has exactly one gameplay gate (cooldown); failure produces no event, only a silent `continue`. The structural location for future gates is `activation.rs:54-75`.
- **Ground detection is exactly one ray.** Origin is body center, not feet; max distance `4.0`; filtered by `MapInstanceId`.
- **Reflection-only components drive the ability archetype.** `OnTickEffects`, `WhileActiveEffects`, `OnHitEffectDefs`, `OnEndEffects`, `OnInputEffects`, `AbilityPhases` are registered with `register_type` only, not lightyear-registered. They are inserted per-ability via `apply_ability_archetype` from the asset, on both sides independently.
- **Input bindings are purely client-side** (`handle_new_character`), but every reader of `ActionState<PlayerActions>` is in shared `protocol` code — inputs flow to the server via lightyear's standard input replication.

## Open Areas

- **`PlaceVoxel` / `RemoveVoxel` readers**: these variants are bound to mouse buttons but no `action_state.just_pressed(PlaceVoxel)` or analogous call was located in the searched paths. Voxel edits may be handled via a separate `VoxelEditRequest` message path; the full routing for these variants was not traced in this research.
- **Exact signature of `Forces` QueryData** in the local avian submodule: the field list above is derived from `git/avian/src/dynamics/rigid_body/forces/query_data.rs` but the precise struct definition line numbers were not pinned.
- **Rollback recovery for ability write races**: whether current ability writes to `LinearVelocity` (from `SetVelocity` / `ApplyForce`) interact cleanly with `linear_velocity_should_rollback`'s 0.01 threshold under network jitter was not empirically tested; the protocol registrations are documented but runtime behavior under divergence was not observed.
