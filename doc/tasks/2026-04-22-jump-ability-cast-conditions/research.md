# Research Findings

## Q1: Jump implementation in `crates/protocol/src/character/movement.rs`

**Direct answer:** Jump is inline in `apply_movement` — on `just_pressed(Jump)` it fires a synchronous downward ray cast from the capsule center, and on a hit applies a `2000` Y impulse via avian's `ForcesItem::apply_linear_impulse`. There is no stored grounded state, no cooldown, no event, and no SystemSet gating.

### Evidence

- `apply_movement` signature (`crates/protocol/src/character/movement.rs:9-22`):
```rust
// crates/protocol/src/character/movement.rs:9-22
pub fn apply_movement(
    entity: Entity,
    mass: &ComputedMass,
    delta_secs: f32,
    spatial_query: &SpatialQuery,
    action_state: &ActionState<PlayerActions>,
    position: &Position,
    forces: &mut ForcesItem,
    player_map_id: Option<&MapInstanceId>,
    map_ids: &Query<&MapInstanceId>,
)
```

- Jump detection + ground check + impulse (`movement.rs:25-46`):
```rust
// crates/protocol/src/character/movement.rs:25-46
if action_state.just_pressed(&PlayerActions::Jump) {
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
    {
        forces.apply_linear_impulse(Vec3::new(0.0, 2000.0, 0.0));
    }
}
```

- Ray origin is `position.0` (capsule center). Max distance `4.0`, `solid=false`. Capsule has `radius=2.0`, `height=2.0` (`crates/protocol/src/character/types.rs:9-10`), so the ray reaches 2 units below the capsule bottom.
- The map-filter predicate restricts hits to entities in the same `MapInstanceId` as the character; entities without a map id are permitted.
- `ForcesItem::apply_linear_impulse` (avian 0.5 `forces/query_data.rs:276-284`): `Δv = (1/m) * impulse` applied directly to `LinearVelocity` after waking the body. `LockedAxes::ROTATION_LOCKED` is set on the character so translational axes pass through unmodified.
- Horizontal movement uses `forces.apply_force(required_acceleration * mass.value())` (`movement.rs:64`), which writes into `VelocityIntegrationData::linear_increment` — accumulated continuously across substeps rather than applied instantaneously.

## Q2: End-to-end ability flow (input → activation → lifecycle → effects)

**Direct answer:** Four FixedUpdate chains drive abilities. `ability_activation` detects `just_pressed` on `Ability1–4`, looks up the slot→`AbilityId`→`AbilityAsset`, gates on cooldown, spawns a predicted `ActiveAbility` entity, and inserts the asset's reflected components. `update_active_abilities` advances the `AbilityPhase` enum (Startup→Active→Recovery→despawn) based on tick-elapsed vs. `AbilityPhases`. Four effect systems (`apply_on_tick_effects`, `apply_while_active_effects`, `apply_on_end_effects`, `apply_on_input_effects`) dispatch phase-specific `AbilityEffect` variants.

### Evidence

**Input binding:** four button actions (`activation.rs:17-22`):
```rust
// crates/protocol/src/ability/activation.rs:17-22
const ABILITY_ACTIONS: [PlayerActions; 4] = [
    PlayerActions::Ability1,
    PlayerActions::Ability2,
    PlayerActions::Ability3,
    PlayerActions::Ability4,
];
```

**Activation core (`activation.rs:34-114`):**
```rust
// crates/protocol/src/ability/activation.rs:55-111 (abridged)
if !action_state.just_pressed(action) { continue; }
let slot_idx = ability_action_to_slot(action)?;
let ability_id = slots.0[slot_idx].as_ref()?;
let handle = ability_defs.0.get(ability_id)?;
let asset = assets.get(handle)?;
let phases = extract_phases(asset)?;                // AbilityPhases
if cooldowns.is_on_cooldown(slot_idx, tick, phases.cooldown) { continue; }
cooldowns.last_used[slot_idx] = Some(tick);
let salt = (player_id.to_bits() as u64) << 32 | (slot_idx as u64) << 16;
let entity_id = commands.spawn((
    ActiveAbility { def_id, caster: entity, original_caster: entity,
                    target: entity, phase: AbilityPhase::Startup,
                    phase_start_tick: tick, ability_slot: slot_idx as u8, depth: 0 },
    PreSpawned::default_with_salt(salt),
    Name::new("ActiveAbility"),
)).id();
apply_ability_archetype(&mut commands, entity_id, asset, registry.0.clone());
if let Ok(controlled_by) = server_query.get(entity) {
    commands.entity(entity_id).insert((
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
        *controlled_by,
    ));
}
```

**Lifecycle state machine (`activation.rs:116-143`):**
```rust
// crates/protocol/src/ability/activation.rs:116-143 (abridged)
let elapsed = tick - active.phase_start_tick;
if elapsed >= phases.phase_duration(&active.phase) as i16 {
    match active.phase {
        Startup  => { active.phase = Active;   active.phase_start_tick = tick; }
        Active   => { active.phase = Recovery; active.phase_start_tick = tick; }
        Recovery => { commands.entity(entity).prediction_despawn(); }
    }
}
```
After transition to `Active`, `OnHitEffects { effects, caster, original_caster, depth }` is inserted if the ability defines `OnHitEffectDefs`; it is removed when leaving `Active` (`activation.rs:161-176`).

**Effect dispatch systems (all in `effects.rs`):**

| System | Lines | Fires when |
|---|---|---|
| `apply_on_tick_effects` | 28–137 | `phase == Active` AND `(tick - phase_start_tick) == tick_effect.tick` |
| `apply_while_active_effects` | 139–163 | every tick while `phase == Active` |
| `apply_on_end_effects` | 165–246 | exact tick `phase == Recovery && phase_start_tick == tick` (first Recovery tick) |
| `apply_on_input_effects` | 248–295 | `phase == Active` AND `action_state.just_pressed(input_effect.action)` on caster |

## Q3: Ability definition schema and loading

**Direct answer:** Files are `*.ability.ron` loaded by a custom `AbilityAssetLoader` that deserializes them as flat maps of `{ "type_path": <component> }` using the reflection registry. The loaded `AbilityAsset` stores `Vec<Box<dyn PartialReflect>>`. At activation time `apply_ability_archetype` inserts each reflected component onto the spawned `ActiveAbility` entity.

### Evidence

**File format** (`crates/protocol/src/reflect_loader.rs:162-172` example):
```ron
{
    "protocol::ability::AbilityPhases": (startup: 4, active: 20, recovery: 0, cooldown: 16),
    "protocol::ability::OnTickEffects": ([(tick: 0, effect: Melee())]),
    "protocol::ability::OnHitEffectDefs": ([
        Damage(amount: 5.0, target: Victim),
        ApplyForce(force: (0.0, 0.9, 2.85), frame: RelativePosition, target: Victim),
    ]),
    "protocol::ability::OnInputEffects": ([(action: Ability1, effect: Ability(id: "punch2", target: Caster))]),
}
```

**Core timing component** (`crates/protocol/src/ability/types.rs:144-149`):
```rust
pub struct AbilityPhases {
    pub startup: u16,
    pub active: u16,
    pub recovery: u16,
    pub cooldown: u16,
}
```

**`AbilityEffect` — enum body** (`crates/protocol/src/ability/types.rs:47-99`):
```rust
pub enum AbilityEffect {
    Melee { id: Option<String>, target: EffectTarget },
    Projectile { id: Option<String>, speed: f32, lifetime_ticks: u16 },
    SetVelocity { speed: f32, target: EffectTarget },
    Damage { amount: f32, target: EffectTarget },
    ApplyForce { force: Vec3, frame: ForceFrame, target: EffectTarget },
    AreaOfEffect { id: Option<String>, target: EffectTarget, radius: f32, duration_ticks: Option<u16> },
    Ability { id: String, target: EffectTarget },
    Teleport { distance: f32 },
    Shield { absorb: f32 },
    Buff { stat: String, multiplier: f32, duration_ticks: u16, target: EffectTarget },
}
```

**`EffectTrigger`** (`types.rs:104-117`):
```rust
pub enum EffectTrigger {
    OnTick { tick: u16, effect: AbilityEffect },
    WhileActive(AbilityEffect),
    OnHit(AbilityEffect),
    OnEnd(AbilityEffect),
    OnInput { action: PlayerActions, effect: AbilityEffect },
}
```

**`AbilitySlots`** (per-character loadout; also loadable as `.ability_slots.ron`) — `types.rs:180`:
```rust
pub struct AbilitySlots(pub [Option<AbilityId>; 4]);
```

**Loader pipeline** (`crates/protocol/src/ability/loading.rs`):
- Non-WASM: `asset_server.load_folder("abilities")` at `Startup` (line 37); `insert_ability_defs` waits for `LoadedFolder` then registers handles keyed by stem with `.ability.ron` stripped (lines 86–109, 221–245).
- WASM: loads `abilities.manifest.ron` (a `Vec<String>`) then individually loads each `abilities/{id}.ability.ron` (lines 44–83).
- Hot reload: `reload_ability_defs` watches `AssetEvent::<AbilityAsset>::Modified` (lines 151–177).

**Instantiation** (`crates/protocol/src/ability/loader.rs:24-56`): `apply_ability_archetype` clones each reflected component via `reflect_clone()` and queues a world closure that looks up `ReflectComponent` in the type registry and calls `reflect_component.insert(&mut entity_mut, component.as_ref(), &registry)`.

## Q4: Sparse-set storage and marker conventions

**Direct answer:** No component anywhere in the workspace uses `#[component(storage = "SparseSet")]`. Every `#[derive(Component)]` uses the implicit default (`Table`). Markers are plain unit structs stored in Table storage; transient states are expressed as small components added/removed by systems (e.g. `RespawnTimer`, `Invulnerable`, `OnHitEffects`). There is no `Grounded`/`InAir`/`Casting` marker — ground is a fresh ray cast per jump; "casting" is the existence of an `ActiveAbility` entity keyed to the caster.

### Evidence

**Full marker inventory:**

| Component | Kind | Add site | Remove site |
|---|---|---|---|
| `CharacterMarker` | unit | `handle_connected` observer (`server/gameplay.rs:407`) | never |
| `DummyTarget` | unit | (commented) `server/gameplay.rs:88-104` | never |
| `RespawnPoint` | unit | map load / validate (`server/map.rs:548-555`, `server/gameplay.rs:148`) | never |
| `MapSaveTarget` | unit (via `#[require]`) | automatic with `RespawnPoint` | never |
| `MeleeHitbox` | unit | `spawn_melee_hitbox` (`ability/spawn.rs:134`) | entity despawn |
| `JointRoot` | unit | `spawn_sprite_rigs` (`sprite_rig/spawn.rs:177`) | entity despawn |
| `HealthBarRoot`/`HealthBarForeground` | unit | `spawn_health_bar` (`render/health_bar.rs:77,87`) | entity despawn |
| `TransitionReadySent` | unit | client transition state machine | client transition state machine |

**Transient-state (non-unit) components — closest pattern to "Grounded":**

| Component | Body | Add / Remove |
|---|---|---|
| `RespawnTimer { expires_at: Tick }` | `character/types.rs:103` | `start_respawn_timer` / `process_respawn_timers` (`server/gameplay.rs:175, 317`); used as `Without<RespawnTimer>` in movement query |
| `Invulnerable { expires_at: Tick }` | `character/types.rs:77` | `process_respawn_timers` / `expire_invulnerability` (`server/gameplay.rs:318, 346`) |
| `PendingTransition(MapInstanceId)` | `map/transition.rs:18` | `start_map_transition` / `complete_map_transition` (`server/transition.rs:68, 141`) |
| `OnHitEffects { effects, caster, … }` | `ability/types.rs:274` | Startup→Active transition / Active→Recovery transition (`ability/activation.rs:163-176`) |
| `ProjectileSpawnEffect { speed, lifetime_ticks }` | `ability/types.rs:241` | `apply_on_tick_effects` (`effects.rs:86`) / `ability_projectile_spawn` (`spawn.rs:241`) one-shot |
| `ActiveShield { remaining }` | `ability/types.rs:367` | `apply_on_tick_effects`/`apply_on_end_effects` / drains to zero |
| `ActiveBuffs(Vec<ActiveBuff>)` | `ability/types.rs:373` | `apply_buff` / `expire_buffs` (`lifecycle.rs:21`) |

**External concept — Bevy sparse-set storage (not used here):**
Bevy's `Component` derive supports `#[component(storage = "SparseSet")]` which stores a component in sparse arrays rather than in archetype tables. Bevy's docs describe sparse-set storage as optimised for components that are frequently added and removed (avoids archetype moves). If introduced here for a transient marker like a grounded flag, the declaration would look like:
```rust
#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct Grounded;
```
— no codebase evidence of this pattern currently.

## Q5: Activation-time gates on abilities

**Direct answer:** Only one gate exists — the cooldown check in `ability_activation`. There are no resource/mana costs, no state checks (grounded, stunned, dead, in-air, casting), and no targeting prerequisites at activation time. Failure is a bare `continue` with no event, message, UI signal, or log beyond the `warn!` logs for missing asset data.

### Evidence

**Cooldown structure** (`crates/protocol/src/ability/types.rs:219-238`):
```rust
pub struct AbilityCooldowns { pub last_used: [Option<Tick>; 4] }

impl AbilityCooldowns {
    pub fn is_on_cooldown(&self, slot: usize, tick: Tick, cooldown_ticks: u16) -> bool {
        match self.last_used[slot] {
            Some(last) => (tick - last).unsigned_abs() <= cooldown_ticks,
            None => false,
        }
    }
}
```

**Gate check** (`crates/protocol/src/ability/activation.rs:73-77`):
```rust
if cooldowns.is_on_cooldown(slot_idx, tick, phases.cooldown) {
    continue;
}
cooldowns.last_used[slot_idx] = Some(tick);
```

**Closest adjacent gate concept — per-hit invulnerability:**
`apply_on_hit_effects` (`crates/protocol/src/hit_detection/effects.rs:101`) checks `if invulnerable.is_none()` before applying `Damage`. This is a victim-side gate after hit detection, not a caster-side gate before cast.

**Nearest "movement-is-blocked-when-state-X" pattern:**
`handle_character_movement` uses `Without<RespawnTimer>` to prevent dead characters from moving (client `gameplay.rs:99`, server `gameplay.rs:119`). There is no analogous filter on ability activation — dead characters can still attempt to cast (though `process_respawn_timers` inserts `RigidBodyDisabled`/`ColliderDisabled`, which blocks physics side-effects).

## Q6: Physics-adjacent system ordering

**Direct answer:** Movement input runs in `FixedUpdate`, unordered relative to ability systems. Avian's physics solver runs later in `FixedPostUpdate` inside `PhysicsSystems::StepSimulation` (scheduled by `LightyearAvianPlugin`). No game-authored plugin references `PhysicsSchedule`, `PhysicsSet`, or `SubstepSchedule`; scheduling primitives used are `.chain()`, `.before(InputSystems::BufferClientInputs)`, `.after(other_system)`, and `.run_if(ready)`.

### Evidence

**Schedule topology:**
```
FixedPreUpdate:
  sync_camera_yaw_to_input  [.before(InputSystems::BufferClientInputs)]   (client/gameplay.rs:22-23)

FixedUpdate:  (no SystemSet ordering between these groups)
  handle_character_movement                                               (client/gameplay.rs:20; server/gameplay.rs:32)
  (ability_activation → update_active_abilities
    → apply_on_tick_effects → apply_while_active_effects
    → apply_on_end_effects → apply_on_input_effects
    → ability_projectile_spawn).chain()                                   (ability/plugin.rs:81-93)
  (update_hitbox_positions → process_hitbox_hits
    → process_projectile_hits → cleanup_hitbox_entities).chain()
    .after(apply_on_tick_effects)                                         (ability/plugin.rs:96-107)
  expire_buffs, aoe_hitbox_lifetime, ability_bullet_lifetime
    .after(process_hitbox_hits).after(process_projectile_hits)            (ability/plugin.rs:109-114)
  on_death_effects, start_respawn_timer, tick_active_transformations,
    process_respawn_timers, expire_invulnerability                        (server/gameplay.rs:34-54)
  update_facing.run_if(ready)                                             (protocol/lib.rs:256)

FixedPostUpdate:
  PhysicsSystems::First → Prepare → StepSimulation → Writeback → Last     (Avian + LightyearAvianPlugin)

PostUpdate:
  FrameInterpolationSystems::Interpolate
  RollbackSystems::VisualCorrection
  PhysicsSystems::Writeback
  TransformSystems::Propagate
```

- Lightyear/Avian wire-up: `configure_sets` on `FixedPostUpdate`/`PostUpdate` at `git/lightyear/lightyear_avian/src/plugin.rs:155-178`.
- `AvianReplicationMode::Position` configured at `crates/protocol/src/lib.rs:239-241`.
- `FIXED_TIMESTEP_HZ = 64.0` (`crates/protocol/src/lib.rs:57`).
- Gravity: `Gravity(Vector::Y * -9.82 * 6.0)` (`crates/protocol/src/lib.rs:252`).

**No `SystemSet` is defined anywhere in game crates; no `configure_sets` is called by game code.**

## Q7: Ground contact queries

**Direct answer:** The codebase queries ground contact exclusively via `SpatialQuery::cast_ray_predicate` in `apply_movement`. No `ShapeCaster`, `RayCaster` component, `CollisionEvent`, `Collisions` resource, `ContactPair`, or `CollidingEntities`-for-ground lookup exists.

### Evidence

- Single call site for ground: `crates/protocol/src/character/movement.rs:31-42` (quoted in Q1).
- `SpatialQuery` system param is passed in from both movement systems (`client/gameplay.rs:85`, `server/gameplay.rs:109`).
- `CollidingEntities` is referenced only in hit detection for projectile/hitbox hits (`hit_detection/systems.rs:43,119`), not for ground.
- `SpatialQueryFilter::from_excluded_entities([entity])` excludes the character itself; per-hit map-instance filtering is done in the predicate closure.

**External concept — alternative avian ground-contact APIs (not used here):**
- `ShapeCaster` component (avian): persistent shape-cast attached to an entity, updated automatically each physics step, results read via `ShapeHits`. Typical use: capsule controller casts a small sphere downward to detect ground with thickness tolerance.
- `Collisions` resource / `CollidingEntities` component: enumerates ongoing contact pairs; typical use for ground is filtering contacts with a normal close to world-up.
- `RayCaster` component: persistent ray attached to an entity with results in `RayHits`.

If `Grounded` were tracked via a persistent `ShapeCaster`, the shape-cast would fire every physics step (in `PhysicsStepSystems::SpatialQuery` in `FixedPostUpdate`) and its hits would be available for read in next `FixedUpdate`. The current code instead does a one-off ray per `Jump` press in `FixedUpdate`.

## Q8: `PlayerActions` enum, variants, and action routing

**Direct answer:** `PlayerActions` has 9 variants: `Move`, `CameraYaw`, `Jump`, `PlaceVoxel`, `RemoveVoxel`, `Ability1–4`. Jump and Move are consumed by `apply_movement`/`update_facing`; Ability1–4 are consumed by `ability_activation` via a fixed index lookup `PlayerAction → slot_idx (0..4) → AbilitySlots[slot_idx] → AbilityId → AbilityAsset`. Mapping is data-driven via `AbilitySlots`, not hard-coded. `PlaceVoxel`/`RemoveVoxel` have key bindings but no current consumers in the examined files. Client and server both run `handle_character_movement` in `FixedUpdate`, but the client filters `With<Predicted>` while the server filters all `With<CharacterMarker>`.

### Evidence

**Enum full body** (`crates/protocol/src/lib.rs:59-80`):
```rust
pub enum PlayerActions {
    Move, CameraYaw, Jump, PlaceVoxel, RemoveVoxel,
    Ability1, Ability2, Ability3, Ability4,
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

**Key bindings** — inserted once per controlled character (`crates/client/src/gameplay.rs:51-62`):
```rust
InputMap::new([(PlayerActions::Jump, KeyCode::Space)])
    .with(PlayerActions::Jump, GamepadButton::South)
    .with_dual_axis(PlayerActions::Move, GamepadStick::LEFT)
    .with_dual_axis(PlayerActions::Move, VirtualDPad::wasd())
    .with(PlayerActions::PlaceVoxel, MouseButton::Left)
    .with(PlayerActions::RemoveVoxel, MouseButton::Right)
    .with(PlayerActions::Ability1, KeyCode::Digit1)
    .with(PlayerActions::Ability2, KeyCode::Digit2)
    .with(PlayerActions::Ability3, KeyCode::Digit3)
    .with(PlayerActions::Ability4, KeyCode::Digit4)
```

**`CameraYaw`** has no physical binding. It is written programmatically: `sync_camera_yaw_to_input` (`crates/client/src/gameplay.rs:167-178`) runs in `FixedPreUpdate` `.before(InputSystems::BufferClientInputs)` and does `action_state.set_value(&PlayerActions::CameraYaw, orbit.target_angle)`.

**Routing table:**

| Variant | Consumer system | Detection | Schedule |
|---|---|---|---|
| `Move` (axis) | `apply_movement`, `update_facing` | `axis_pair` | `FixedUpdate` |
| `CameraYaw` (axis) | `apply_movement`, `update_facing` | `value` | `FixedUpdate` |
| `Jump` (button) | `apply_movement` inline | `just_pressed` | `FixedUpdate` |
| `Ability1–4` (button) | `ability_activation`, slot lookup | `just_pressed` | `FixedUpdate` (chained) |
| `PlaceVoxel`, `RemoveVoxel` | no consumer in examined files | — | — |

**Slot mapping** (`crates/protocol/src/ability/activation.rs:25-27`):
```rust
pub fn ability_action_to_slot(action: &PlayerActions) -> Option<usize> {
    ABILITY_ACTIONS.iter().position(|a| a == action)
}
```

**No `Pressed` (continuous-hold) or `Released` checks exist anywhere for any `PlayerActions` variant** — only `just_pressed`, `axis_pair`, and `value`.

**No ability-activation event bus:** `ability_activation` directly spawns the `ActiveAbility` entity via `commands.spawn`; there is no `Event` type, no `send_message`.

## Q9: Abilities modifying character velocity / impulses

**Direct answer:** Yes — abilities reach directly into `LinearVelocity` on the caster (via `SetVelocity`) and on the victim (via `ApplyForce`), bypassing the `ForcesItem` accumulator that `movement.rs` uses. Both sides run in `FixedUpdate`. No explicit ordering between ability effects and `handle_character_movement` is declared, so within a tick whichever runs last overwrites the other. Avian's solver runs later in `FixedPostUpdate` and consumes whatever `LinearVelocity` state results.

### Evidence

**Ability-side writes to `LinearVelocity`:**

| Site | Query | Mutation |
|---|---|---|
| `apply_while_active_effects` (`effects.rs:141,154-155`) | `Query<(&Rotation, &mut LinearVelocity)>` | `velocity.x = dir.x * speed; velocity.z = dir.z * speed;` |
| `apply_on_end_effects` (`effects.rs:174,185-188`) | `Query<(&mut Position, &Rotation, &mut LinearVelocity)>` | identical to above |
| `apply_on_hit_effects` (`hit_detection/effects.rs:72-77,125`) | `Query<(&Position, Option<&mut LinearVelocity>, &mut Health, Option<&Invulnerable>)>` | `velocity.0 += world_force;` (additive) |
| `process_respawn_timers` (`server/gameplay.rs:311`) | respawn reset | `velocity.0 = Vec3::ZERO;` |

**Movement-side writes via `ForcesItem` (which mutates `LinearVelocity` for impulses, and `VelocityIntegrationData` for forces):**

- `forces.apply_linear_impulse(Vec3::new(0.0, 2000.0, 0.0))` (`movement.rs:44`): immediate `LinearVelocity += J/m`.
- `forces.apply_force(required_acceleration * mass.value())` (`movement.rs:64`): accumulates into `VelocityIntegrationData::linear_increment` consumed during the physics step.

**Ordering reality:** no explicit `.before`/`.after` between ability effect systems and `handle_character_movement`. Bevy's scheduler resolves the order by conflict graph; both touch `LinearVelocity` mutably on the character, so they cannot run in parallel, but relative order is unspecified. Hit detection chains `.after(apply_on_tick_effects)` but not relative to movement. There is no declared contention guard.

## Q10: Lightyear networking integration

**Direct answer:** Components are registered with `app.register_component::<C>()` + `.add_prediction()` and optionally `.add_should_rollback(fn)`, `.add_map_entities()`, `.add_linear_correction_fn()`, `.add_linear_interpolation()`. All registration lives in `ProtocolPlugin::build` (`crates/protocol/src/lib.rs:90-208`). Ability entities are spawned with `PreSpawned::default_with_salt(salt)` for client-side predicted spawn matching; on the server side only, `Replicate::to_clients(NetworkTarget::All)` + `PredictionTarget::to_clients(NetworkTarget::All)` are inserted. There are no ability-specific network messages — ability activation flows purely through input replay of `ActionState<PlayerActions>`. Hitbox/bullet entities carry `DisableRollback`.

### Evidence

**Input plugin** (`crates/protocol/src/lib.rs:92-101`):
```rust
app.add_plugins(InputPlugin::<PlayerActions> {
    config: InputConfig::<PlayerActions> {
        rebroadcast_inputs: true,
        packet_redundancy: 20,
        ..default()
    },
});
```

**Ability component registrations** (`lib.rs:186-195`):
```rust
app.register_component::<AbilitySlots>();
app.register_component::<ActiveAbility>().add_prediction().add_map_entities();
app.register_component::<AbilityCooldowns>().add_prediction();
app.register_component::<ActiveShield>().add_prediction();
app.register_component::<ActiveBuffs>().add_prediction();
app.register_component::<AbilityProjectileSpawn>();
```

**Physics component registrations** (`lib.rs:177-207`):
```rust
app.register_component::<LinearVelocity>()
    .add_prediction()
    .add_should_rollback(linear_velocity_should_rollback);     // threshold 0.01 m/s

app.register_component::<AngularVelocity>()
    .add_prediction()
    .add_should_rollback(angular_velocity_should_rollback);

app.register_component::<Position>()
    .add_prediction()
    .add_should_rollback(position_should_rollback)             // threshold 0.01 m
    .add_linear_correction_fn()
    .add_linear_interpolation();

app.register_component::<Rotation>()
    .add_prediction()
    .add_should_rollback(rotation_should_rollback)
    .add_linear_correction_fn()
    .add_linear_interpolation();
```

**Character replication** (`crates/server/src/gameplay.rs:397-409`):
```rust
Replicate::to_clients(NetworkTarget::All),
NetworkVisibility,
PredictionTarget::to_clients(NetworkTarget::All),
ControlledBy { owner: client_entity, lifetime: Default::default() },
```

**`PreSpawned` salt scheme** (`activation.rs:84`):
```rust
let salt = (player_id.to_bits() as u64) << 32 | (slot_idx as u64) << 16 | 0u64;
```
Sub-ability salts hash `player_id`, `parent_slot`, `depth`, `ability_id_str` (`spawn.rs:26-33`).

**`DisableRollback` applied to**:
- Melee hitbox (`spawn.rs:132`), AoE hitbox (`spawn.rs:177`), bullet (`spawn.rs:266`).
- Character during transitions (`client/transition.rs:83`, `server/transition.rs:65`).

**Network-safe checklist observed:**
1. `Serialize`/`Deserialize` derives on the component.
2. `register_component::<C>().add_prediction()`.
3. Optional `add_should_rollback(fn)` with a numeric threshold to avoid spurious rollbacks.
4. `add_map_entities()` for components containing `Entity` fields (required on `ActiveAbility` because `caster`, `original_caster`, `target` are all `Entity`).
5. Transient local-only entities (hitboxes, bullets) use `DisableRollback`.
6. Deterministic spawns on client use `PreSpawned::default_with_salt(salt)`.

**No `AbilityCast` / rejection / approval message types exist.** `DeathEvent` is a Bevy `Message` (internal bus), not a lightyear network message.

## Q11: Converting `AbilityEffect::ApplyForce` to use `forces.apply_linear_impulse`

**Direct answer:** It is mechanically feasible — avian's `Forces` is `QueryData` that can be added to the target query, and `forces.apply_linear_impulse(world_force)` performs the same `Δv = J/m` computation that `movement.rs`'s jump uses. Behavioural differences arise from (a) mass scaling: the current path adds the raw vector to `LinearVelocity` (mass-independent), while impulses divide by mass so RON values would need to be rescaled by the caster's mass; (b) wake/locked-axes handling: `apply_linear_impulse` triggers `try_wake_up` and respects `LockedAxes`; (c) the target query must include the `Forces` `QueryData` and hit on entities that have a `RigidBody` — `Option<&mut LinearVelocity>` currently silently skips non-bodies, `Forces` would fail to match them. Rollback/prediction behaviour is unchanged because `LinearVelocity` is the component written either way and is already predicted + has `add_should_rollback`. No scheduling change is needed — the write still happens in `FixedUpdate` before `PhysicsSystems::StepSimulation` in `FixedPostUpdate`.

### Evidence

**Current `ApplyForce` application** (`crates/protocol/src/hit_detection/effects.rs:108-129`):
```rust
AbilityEffect::ApplyForce { force, frame, target } => {
    let entity = resolve_on_hit_target(target, victim, on_hit);
    if let Ok((target_pos, velocity, _, _)) = target_query.get_mut(entity) {
        let world_force = resolve_force_frame(
            *force, frame, source_pos, target_pos.0,
            on_hit.caster, entity, rotation_query,
        );
        if let Some(mut velocity) = velocity {
            velocity.0 += world_force;
        }
    } else {
        warn!("ApplyForce target {:?} not found", entity);
    }
}
```

**Current `target_query` shape** (`hit_detection/effects.rs:72-77`):
```rust
target_query: &mut Query<(
    &Position,
    Option<&mut LinearVelocity>,
    &mut Health,
    Option<&Invulnerable>,
)>
```

**`apply_linear_impulse` definition** (avian 0.5 `dynamics/rigid_body/forces/query_data.rs:276-284`):
```rust
fn apply_linear_impulse(&mut self, impulse: Vector) {
    if impulse != Vector::ZERO && self.try_wake_up() {
        let effective_inverse_mass = self
            .locked_axes()
            .apply_to_vec(Vector::splat(self.inverse_mass()));
        let delta_vel = effective_inverse_mass * impulse;
        *self.linear_velocity_mut() += delta_vel;
    }
}
```

**Differences by dimension:**

| Dimension | Current (`velocity.0 += force`) | Proposed (`forces.apply_linear_impulse(force)`) |
|---|---|---|
| Mass scaling | None; RON `force` interpreted as Δv (m/s) | `Δv = force / mass`; RON values need to grow by ~mass factor to match existing feel |
| `LockedAxes` | Ignored; all 3 components of velocity mutated | Components on locked axes are zeroed out by `effective_inverse_mass` |
| Wake-up | No `try_wake_up` call; a sleeping body may remain asleep until integrator touches it | `try_wake_up` is invoked; sleeping bodies wake before the impulse applies |
| Target must have | `LinearVelocity` (optional in current query; `None` is silently skipped with no warn) | full `RigidBody` + `Forces` data (`LinearVelocity`, `AngularVelocity`, `VelocityIntegrationData`, `AccumulatedLocalAcceleration`, `SleepTimer`, `ComputedMass`). Entities without a dynamic body fail the query |
| Query shape change | — | `target_query` must gain `Forces` (`QueryData`) instead of `Option<&mut LinearVelocity>`. `Forces` is not `Option`-wrappable in a straightforward sense; a secondary query or `Has<RigidBody>` guard would be required if targets might not be bodies |
| `movement.rs` call site parity | differs (direct write) | matches `forces.apply_linear_impulse(Vec3::new(0.0, 2000.0, 0.0))` at `movement.rs:44` |
| Rollback / prediction | unchanged — `LinearVelocity` is the written component in both paths; already `.add_prediction().add_should_rollback(linear_velocity_should_rollback)` at `lib.rs:177` | unchanged |
| Schedule | runs in `FixedUpdate` in `apply_on_hit_effects`; avian integrates in `FixedPostUpdate` | identical schedule; no ordering change required |

**Example RON-value rescaling evidence:**
- Jump uses `impulse = (0.0, 2000.0, 0.0)` (`movement.rs:44`); with capsule `Collider::capsule(2.0, 2.0)` and default density, the resulting jump Δv depends on avian's computed mass.
- Existing `ApplyForce` RON value in `reflect_loader.rs:162-172` is `(0.0, 0.9, 2.85)`, currently added directly to velocity — interpreted as Δv of 0.9 m/s up + 2.85 m/s forward. If switched to impulse, the scalar would scale as `new_force = old_force * mass` to preserve behaviour.

## Cross-Cutting Observations

**Shared component, no scheduling guard.** Both movement (`apply_movement` via `Forces`) and ability effects (`apply_while_active_effects`, `apply_on_end_effects`, `apply_on_hit_effects`) write to `LinearVelocity` within `FixedUpdate`. There is no `.before`/`.after` constraint between these write paths; Bevy schedules them in unspecified relative order.

**Single ground-check API.** `SpatialQuery::cast_ray_predicate` is the only ground query mechanism in the codebase. No persistent `ShapeCaster`/`RayCaster` component, no collision-event listener for ground, no `Collisions` iteration.

**Data-driven activation, code-driven gating.** Ability definition is fully data-driven through reflected RON components (`AbilityPhases`, `OnTickEffects`, etc.). The activation gate is code-driven (only cooldown); there is no data field for "required state", "resource cost", or "target prerequisite" in any ability schema struct.

**No `Pressed`/`Released` anywhere.** All button consumption in the codebase uses `just_pressed`. No hold-to-charge, no release-to-cast, no continuous-hold abilities are currently supported.

**No custom `SystemSet` in game code.** All ordering is local (`.chain()`, `.after(system)`), not set-based. The only named sets referenced are from external crates (`InputSystems`, `PhysicsSystems`, `PhysicsStepSystems`, `PredictionSystems`, `FrameInterpolationSystems`, `RollbackSystems`, `TransformSystems`).

**Prediction/rollback is `LinearVelocity`-aware.** `LinearVelocity` is both a predicted component and has a custom rollback threshold. Any system — movement or ability — writing to it participates in rollback automatically. Hitbox and bullet entities opt out with `DisableRollback`.

**Storage defaults are Table across the workspace.** No `SparseSet` storage annotations exist. Transient state is modelled as small components added/removed by systems, not as sparse-set markers.

**`ForcesItem` vs. direct `LinearVelocity` are two distinct write paths to the same final state.** `ForcesItem` is avian's helper `QueryData` that wraps `LinearVelocity`/`AngularVelocity`/`VelocityIntegrationData`/`AccumulatedLocalAcceleration`/`SleepTimer`/`ComputedMass`; `apply_linear_impulse` writes to `LinearVelocity` (mass-scaled, locked-axis-aware, wake-up); `apply_force` accumulates into `VelocityIntegrationData` consumed across substeps. Direct `velocity.0 += v` skips all of these behaviours.

## Open Areas

- **What `Pressed` semantics `just_pressed` corresponds to across tick loss**: whether lightyear's input replay on rollback re-fires a `just_pressed` on re-simulated ticks was not verified by reading lightyear source. The `InputPlugin::<PlayerActions>` `packet_redundancy: 20` setting suggests loss-tolerant replay, but exact `just_pressed` edge semantics during rollback were not confirmed.
- **Computed mass value for the character capsule.** `ComputedMass` is avian-computed from the collider and default density; the exact numeric value was not read. This matters for Q11's mass-rescaling factor.
- **Whether `Forces` `QueryData` can be `Option`-wrapped.** The current `target_query` uses `Option<&mut LinearVelocity>`; switching to `Forces` for a path that may target non-RigidBody entities was not fully explored for ergonomics.
- **No evidence located for a `Resource` that tracks global physics substep count or solver iteration.** References to `SubstepSchedule` exist in avian but the game never adds systems there.
