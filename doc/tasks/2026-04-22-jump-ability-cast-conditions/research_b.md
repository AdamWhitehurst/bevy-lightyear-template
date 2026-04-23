# Research — Jump Ability / Cast Conditions

Source: answers to `questions.md`. All `file:line` references are relative to repo root `/home/aw/ws/bevy-lightyear-template`.

---

## Q1 — Current Jump Implementation in `movement.rs`

### Entry point

`apply_movement` in `crates/protocol/src/character/movement.rs:9`. Full signature:

```rust
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

Import (same file):

```rust
use avian3d::prelude::{forces::ForcesItem, *};
```

### Jump input detection

`movement.rs:25`:

```rust
if action_state.just_pressed(&PlayerActions::Jump) {
```

### Inline ground contact verification

Immediately inside the `just_pressed` guard (`movement.rs:27-42`):

```rust
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

- Origin: character `Position` (capsule center).
- Direction: `Dir3::NEG_Y` (straight down).
- Max distance: `4.0` world units.
- `solid: false`.
- Predicate: hits accepted only when the hit entity shares the caster's `MapInstanceId` (or the caster has no map id).

### Impulse application

If the ray hit (`movement.rs:44`):

```rust
forces.apply_linear_impulse(Vec3::new(0.0, 2000.0, 0.0));
```

`forces: &mut ForcesItem` comes from the avian `Forces` QueryData in the caller's query tuple. `apply_linear_impulse` divides the impulse by the body's `ComputedMass` to get the velocity delta (see Q11 for the mass arithmetic).

### Horizontal movement (same function, `movement.rs:49-64`)

Reads `action_state.axis_pair(&PlayerActions::Move)` and `action_state.value(&PlayerActions::CameraYaw)`, rotates the 2D move vector by yaw, and applies `forces.apply_force(required_acceleration * mass.value())` toward `MAX_SPEED = 15.0` capped at `MAX_ACCELERATION = 500.0`.

### Callers (two)

**Client** (`crates/client/src/gameplay.rs:83-116`):

```rust
fn handle_character_movement(
    time: Res<Time>,
    spatial_query: SpatialQuery,
    map_ids: Query<&MapInstanceId>,
    mut query: Query<
        (Entity, &ActionState<PlayerActions>, &ComputedMass, &Position, Forces, Option<&MapInstanceId>),
        (With<Predicted>, With<CharacterMarker>, Without<RespawnTimer>),
    >,
) {
    for (entity, action_state, mass, position, mut forces, player_map_id) in &mut query {
        apply_movement(entity, mass, time.delta_secs(), &spatial_query,
            action_state, position, &mut forces, player_map_id, &map_ids);
    }
}
```

**Server** (`crates/server/src/gameplay.rs:106-135`): identical body, query filter omits `With<Predicted>`.

Both are registered at `FixedUpdate` with no explicit ordering (`.add_systems(FixedUpdate, handle_character_movement)`).

---

## Q2 — End-to-End Ability Flow

### Input binding

`PlayerActions` enum defined in `crates/protocol/src/lib.rs:59-70` (see Q8). `InputPlugin::<PlayerActions>` registered in `ProtocolPlugin::build` (`lib.rs:92-101`).

Bindings on the client only (`client/src/gameplay.rs:51-62`):

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

### Activation — `activation.rs`

`ability_activation` (`crates/protocol/src/ability/activation.rs:34-113`):

```rust
pub fn ability_activation(
    mut commands: Commands,
    ability_defs: Res<AbilityDefs>,
    ability_assets: Res<Assets<AbilityAsset>>,
    registry: Res<AppTypeRegistry>,
    default_slots: Res<DefaultAbilitySlots>,
    timeline: Res<LocalTimeline>,
    mut query: Query<(
        Entity,
        &ActionState<PlayerActions>,
        Option<&AbilitySlots>,
        &mut AbilityCooldowns,
        &PlayerId,
    )>,
    server_query: Query<&ControlledBy>,
)
```

Per-frame logic:
1. Current tick from `LocalTimeline::tick()`.
2. Iterate entities with `ActionState<PlayerActions>` + `AbilityCooldowns` + `PlayerId`.
3. For each of `ABILITY_ACTIONS = [Ability1, Ability2, Ability3, Ability4]` (`activation.rs:17-22`): `action_state.just_pressed(action)`.
4. Resolve slot → `AbilityId` via `AbilitySlots` (or `DefaultAbilitySlots`).
5. Look up `Handle<AbilityAsset>` in `AbilityDefs`, fetch asset, call `extract_phases(asset)`.
6. Cooldown check: `cooldowns.is_on_cooldown(slot_idx, tick, phases.cooldown)` — silent `continue` if on cooldown.
7. Stamp `cooldowns.last_used[slot_idx] = Some(tick)`.
8. Salt:
   ```rust
   let salt = (player_id.0.to_bits()) << 32 | (slot_idx as u64) << 16 | 0u64;
   ```
9. Spawn `ActiveAbility` entity with `PreSpawned::default_with_salt(salt)`, `phase: AbilityPhase::Startup`, `phase_start_tick: tick`.
10. `apply_ability_archetype` — reflect-inserts every deserialized component from the RON asset.
11. If caster has `ControlledBy` (server-side), inserts `Replicate::to_clients(NetworkTarget::All)`, `PredictionTarget::to_clients(NetworkTarget::All)`, and `*controlled_by`.

### Phase progression — `lifecycle.rs` + `activation.rs`

`update_active_abilities` (`activation.rs:145-178`):

```rust
pub fn update_active_abilities(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    mut query: Query<(
        Entity,
        &mut ActiveAbility,
        &AbilityPhases,
        Option<&OnHitEffectDefs>,
    )>,
)
```

Calls `advance_ability_phase` (`activation.rs:116-143`):

```rust
fn advance_ability_phase(
    commands: &mut Commands,
    entity: Entity,
    active: &mut ActiveAbility,
    phases: &AbilityPhases,
    tick: Tick,
)
```

- `elapsed = tick - active.phase_start_tick`.
- Compared against `phases.phase_duration(&active.phase)` (one of `startup`/`active`/`recovery`).
- Transitions `Startup → Active → Recovery`, then `commands.entity(entity).prediction_despawn()`.
- On `Startup → Active` transition and if `OnHitEffectDefs` is present and non-empty, `update_active_abilities` inserts `OnHitEffects { effects, caster, original_caster, depth }`.
- On `Active → Recovery`, removes `OnHitEffects`.

Other `lifecycle.rs` systems:
- `expire_buffs` (`lifecycle.rs:9-24`) — retains only `ActiveBuffs` entries where `b.expires_tick - tick > 0`; removes component when the vec is empty.
- `aoe_hitbox_lifetime` (`lifecycle.rs:42-54`) — despawns AoE hitboxes after `aoe.duration_ticks`.
- `ability_bullet_lifetime` (`lifecycle.rs:56-71`) — despawns bullets after `AbilityProjectileSpawn.lifetime_ticks`.
- `cleanup_effect_markers_on_removal` (`lifecycle.rs:26-40`) — observer on `Remove, ActiveAbility` that strips `OnTickEffects`, `WhileActiveEffects`, `OnHitEffects`, `OnHitEffectDefs`, `OnEndEffects`, `OnInputEffects`, `ProjectileSpawnEffect`, `AbilityPhases`.

### Side-effect dispatch — `effects.rs`

Four systems, all in `FixedUpdate`, chained after `update_active_abilities`:

**`apply_on_tick_effects`** (`effects.rs:28-137`): queries `(OnTickEffects, ActiveAbility)`. Skips unless phase is `Active`. Checks `tick_effect.tick != active_offset` where `active_offset = (tick - active.phase_start_tick) as u16`. Dispatches `Melee` → `spawn_melee_hitbox`; `AreaOfEffect` → `spawn_aoe_hitbox`; `Projectile` → inserts `ProjectileSpawnEffect`; `Ability` → `spawn_sub_ability`; `Teleport` → mutates caster `Position`; `Shield` → inserts `ActiveShield`; `Buff` → inserts `ActiveBuffs`. `ApplyForce`, `SetVelocity`, `Damage` hit the `_ => warn!("Unhandled …")` catch-all here.

**`apply_while_active_effects`** (`effects.rs:139-163`):

```rust
pub fn apply_while_active_effects(
    query: Query<(&WhileActiveEffects, &ActiveAbility)>,
    mut caster_query: Query<(&Rotation, &mut LinearVelocity)>,
) {
    ...
    AbilityEffect::SetVelocity { speed, target } => {
        let target_entity = resolve_caster_target(&target, active);
        if let Ok((rotation, mut velocity)) = caster_query.get_mut(target_entity) {
            let direction = super::types::facing_direction(rotation);
            velocity.x = direction.x * speed;
            velocity.z = direction.z * speed;
        }
    }
```

Only `SetVelocity` is handled here. X/Z overwrite, Y preserved.

**`apply_on_end_effects`** (`effects.rs:165-246`): runs when `phase == Recovery && phase_start_tick == tick` (exact transition tick). Handles `SetVelocity`, `Ability`, `Teleport`, `Shield`, `Buff`.

**`apply_on_input_effects`** (`effects.rs:248-295`): skips unless phase is `Active`. Reads `ActionState<PlayerActions>` from the caster, checks `just_pressed`. Only handles `Ability` (combo sub-ability spawn).

### Hit-time dispatch — `apply_on_hit_effects` (in `hit_detection/effects.rs:62-153`)

Called by `process_hitbox_hits` (systems.rs:34-90) and `process_projectile_hits` (systems.rs:110-163). This is where `ApplyForce`, `Damage`, `Shield`, `Buff`, `Ability` are dispatched — see Q11 for the full `ApplyForce` body.

### FixedUpdate ordering (`ability/plugin.rs:81-118`)

```rust
app.add_systems(
    FixedUpdate,
    (
        ability_activation,
        update_active_abilities,
        apply_on_tick_effects,
        apply_while_active_effects,
        apply_on_end_effects,
        apply_on_input_effects,
        ability_projectile_spawn,
    )
        .chain()
        .run_if(ready.clone()),
);

app.add_systems(
    FixedUpdate,
    (
        crate::hit_detection::update_hitbox_positions,
        crate::hit_detection::process_hitbox_hits,
        crate::hit_detection::process_projectile_hits,
        crate::hit_detection::cleanup_hitbox_entities,
    )
        .chain()
        .after(apply_on_tick_effects)
        .run_if(ready.clone()),
);

app.add_systems(
    FixedUpdate,
    (expire_buffs, aoe_hitbox_lifetime, ability_bullet_lifetime)
        .after(crate::hit_detection::process_hitbox_hits)
        .after(crate::hit_detection::process_projectile_hits)
        .run_if(ready.clone()),
);
```

---

## Q3 — Ability Data Schema

### File format

RON with extension `.ability.ron`. All concrete files start with `#![enable(implicit_some)]` so `Option<T>` fields drop `Some(…)` wrapping. Flat map keyed by Bevy type-path string, value is the component in RON struct syntax.

Example — `assets/abilities/punch.ability.ron`:

```ron
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

### Component types (all in `crates/protocol/src/ability/types.rs`)

```rust
// types.rs:18-20
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[type_path = "protocol::ability"]
pub struct AbilityId(pub String);

// types.rs:23-30
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect, Default)]
#[type_path = "protocol::ability"]
pub enum EffectTarget {
    #[default]
    Caster,
    Victim,
    OriginalCaster,
}

// types.rs:33-42
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Reflect)]
#[type_path = "protocol::ability"]
pub enum ForceFrame {
    #[default]
    World,
    Caster,
    Victim,
    RelativePosition,
    RelativeRotation,
}

// types.rs:45-99
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Reflect)]
#[type_path = "protocol::ability"]
pub enum AbilityEffect {
    Melee      { #[serde(default)] id: Option<String>,
                 #[serde(default)] target: EffectTarget },
    Projectile { #[serde(default)] id: Option<String>, speed: f32, lifetime_ticks: u16 },
    SetVelocity{ speed: f32, target: EffectTarget },
    Damage     { amount: f32, target: EffectTarget },
    ApplyForce { force: Vec3,
                 #[serde(default)] frame: ForceFrame,
                 target: EffectTarget },
    AreaOfEffect { #[serde(default)] id: Option<String>,
                   #[serde(default)] target: EffectTarget,
                   radius: f32,
                   #[serde(default)] duration_ticks: Option<u16> },
    Ability   { id: String, target: EffectTarget },
    Teleport  { distance: f32 },
    Shield    { absorb: f32 },
    Buff      { stat: String, multiplier: f32, duration_ticks: u16, target: EffectTarget },
}

// types.rs:141-149
#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[type_path = "protocol::ability"]
#[reflect(Component, Serialize, Deserialize)]
pub struct AbilityPhases {
    pub startup: u16,
    pub active: u16,
    pub recovery: u16,
    pub cooldown: u16,
}

// types.rs:282-288
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
#[type_path = "protocol::ability"]
pub struct TickEffect {
    #[serde(default)]
    pub tick: u16,
    pub effect: AbilityEffect,
}

// types.rs:291-326 — the five component wrappers used as assets:
#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct OnTickEffects(pub Vec<TickEffect>);

#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct WhileActiveEffects(pub Vec<AbilityEffect>);

#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct OnEndEffects(pub Vec<AbilityEffect>);

#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct InputEffect {
    pub action: PlayerActions,
    pub effect: AbilityEffect,
}

#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct OnInputEffects(pub Vec<InputEffect>);

#[derive(Component, Clone, Debug, PartialEq, Reflect, Serialize, Deserialize, Default)]
#[reflect(Component, Serialize, Deserialize)]
pub struct OnHitEffectDefs(pub Vec<AbilityEffect>);

// types.rs:178-180
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, Asset, TypePath)]
#[type_path = "protocol::ability"]
pub struct AbilitySlots(pub [Option<AbilityId>; 4]);

// types.rs:329-332
#[derive(Asset, TypePath)]
pub struct AbilityAsset {
    pub components: Vec<Box<dyn PartialReflect>>,
}

// types.rs:166-175
#[derive(Resource, Clone, Debug, Default)]
pub struct AbilityDefs {
    pub abilities: HashMap<AbilityId, Handle<AbilityAsset>>,
}
```

Note: `AbilityDef` at `types.rs:120-128` also exists with a flat single-struct schema but is **not used** by the asset loader — the real schema is the component-map format above.

### Loader — `loader.rs:59-93`

```rust
#[derive(TypePath)]
pub(super) struct AbilityAssetLoader {
    type_registry: TypeRegistryArc,
}

impl FromWorld for AbilityAssetLoader {
    fn from_world(world: &mut World) -> Self {
        Self { type_registry: world.resource::<AppTypeRegistry>().0.clone() }
    }
}

impl AssetLoader for AbilityAssetLoader {
    type Asset = AbilityAsset;
    type Settings = ();
    type Error = crate::reflect_loader::ReflectLoadError;

    fn extensions(&self) -> &[&str] { &["ability.ron"] }

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let registry = self.type_registry.read();
        let components = crate::reflect_loader::deserialize_component_map(&bytes, &registry)?;
        Ok(AbilityAsset { components })
    }
}
```

`deserialize_component_map` (`reflect_loader.rs:56-64`) iterates the RON map: each key is resolved via `TypeRegistrationDeserializer`, each value via `TypedReflectDeserializer`, upcasting via `ReflectFromReflect` when available. Output is `Vec<Box<dyn PartialReflect>>`.

### Discovery / indexing — `loading.rs`

Native (`loading.rs:32-39`): `asset_server.load_folder("abilities")`, handle stored in `AbilityFolderHandle`.

Once loaded (`loading.rs:87-109`, `insert_ability_defs`): filter folder handles by `.ability.ron` suffix, strip extension to form `AbilityId(file_stem)`, build `HashMap<AbilityId, Handle<AbilityAsset>>` into `AbilityDefs` resource. Hot reload via `reload_ability_defs` on `AssetEvent::Modified`.

WASM (`loading.rs:43-84`): loads `abilities.manifest.ron` (`AbilityManifest(Vec<String>)`) then issues individual `load("abilities/{id}.ability.ron")` calls.

### Spawning — `loader.rs:24-56`

```rust
pub(crate) fn apply_ability_archetype(
    commands: &mut Commands,
    entity: Entity,
    asset: &AbilityAsset,
    registry: TypeRegistryArc,
) {
    let components: Vec<Box<dyn PartialReflect>> = asset.components.iter()
        .map(|c| c.reflect_clone().expect("...").into_partial_reflect())
        .collect();

    commands.queue(move |world: &mut World| {
        let registry = registry.read();
        let mut entity_mut = world.entity_mut(entity);
        for component in &components {
            let type_path = component.reflect_type_path();
            let Some(registration) = registry.get_with_type_path(type_path) else {
                warn!("Ability component type not registered: {type_path}");
                continue;
            };
            let Some(reflect_component) = registration.data::<ReflectComponent>() else {
                warn!("Type missing #[reflect(Component)]: {type_path}");
                continue;
            };
            reflect_component.insert(&mut entity_mut, component.as_ref(), &registry);
        }
    });
}
```

`extract_phases` (`loader.rs:9-21`) iterates `asset.components`, matches `TypeId::of::<AbilityPhases>()`, downcasts via `try_downcast_ref`.

### Invariants / defaults

- `AbilityPhases` fields all required (no `#[serde(default)]`).
- `TickEffect::tick`, `Melee::id`/`target`, `Projectile::id`, `ApplyForce::frame`, `AreaOfEffect::id`/`target`/`duration_ticks` — all have `#[serde(default)]`.
- Unknown type-paths → `warn!` + skip (does not fail load).
- Types without `#[reflect(Component)]` → `warn!` + skip.
- RON parse error → `ReflectLoadError::Ron` (fails the asset load).
- `AbilitySlots` loaded via `bevy_common_assets::ron::RonAssetPlugin` (standard serde), not the reflect loader (`plugin.rs:50-51`).

### Concrete RON files in `assets/abilities/`

**speed_burst.ability.ron** — pure `WhileActiveEffects::SetVelocity`:
```ron
#![enable(implicit_some)]
{
    "protocol::ability::AbilityPhases": (startup: 5, active: 25, recovery: 6, cooldown: 60),
    "protocol::ability::OnTickEffects": ([(tick: 0, effect: Buff(stat: "speed", multiplier: 2.5, duration_ticks: 60, target: Caster))]),
    "protocol::ability::WhileActiveEffects": ([SetVelocity(speed: 30.0, target: Caster)]),
}
```

**ground_pound.ability.ron**:
```ron
#![enable(implicit_some)]
{
    "protocol::ability::AbilityPhases": (startup: 48, active: 56, recovery: 16, cooldown: 150),
    "protocol::ability::OnTickEffects": ([
        (tick: 8,  effect: AreaOfEffect(target: Victim, radius: 5.0,  duration_ticks: 20)),
        (tick: 30, effect: AreaOfEffect(target: Victim, radius: 10.0, duration_ticks: 20)),
        (tick: 55, effect: AreaOfEffect(target: Victim, radius: 15.0, duration_ticks: 20)),
    ]),
    "protocol::ability::OnHitEffectDefs": ([
        Damage(amount: 25.0, target: Victim),
        ApplyForce(force: (0.0, 8.0, 8.0), frame: RelativePosition, target: Victim),
    ]),
}
```

**blink_strike.ability.ron**:
```ron
#![enable(implicit_some)]
{
    "protocol::ability::AbilityPhases": (startup: 3, active: 4, recovery: 10, cooldown: 28),
    "protocol::ability::OnTickEffects": ([
        (tick: 0, effect: Teleport(distance: 6.0)),
        (tick: 1, effect: Melee()),
    ]),
    "protocol::ability::OnHitEffectDefs": ([
        Damage(amount: 18.0, target: Victim),
        ApplyForce(force: (0.0, 1.2, 4.0), frame: RelativePosition, target: Victim),
    ]),
}
```

Other files (`punch2`, `punch3`, `uppercut`, `shockwave`, `shield_bash`, `dive_kick`) follow the same pattern; `ApplyForce` values enumerated in Q11.

---

## Q4 — Marker Component Conventions

### Sparse-set storage

**Zero** `#[component(storage = "SparseSet")]` attributes exist anywhere in the codebase. Every component uses default table storage.

### True unit-struct markers

```rust
// crates/protocol/src/character/types.rs:19-24
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CharacterMarker;

#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DummyTarget;

// crates/protocol/src/character/types.rs:36-39
#[derive(Component, Clone, Debug)]
#[require(MapSaveTarget)]
pub struct RespawnPoint;

// crates/protocol/src/ability/types.rs:257-259
#[derive(Component, Clone, Debug)]
pub struct MeleeHitbox;

// crates/protocol/src/map/persistence.rs:4-6
#[derive(Component, Clone, Debug, Default)]
pub struct MapSaveTarget;

// crates/protocol/src/map/transition.rs:46-47
#[derive(Component)]
pub struct TransitionReadySent;

// crates/render/src/health_bar.rs:8-12
#[derive(Component)] pub(crate) struct HealthBarRoot;
#[derive(Component)] pub(crate) struct HealthBarForeground;

// crates/sprite_rig/src/spawn.rs
#[derive(Component)] pub struct JointRoot;
```

These are **never removed individually** — the entity is despawned. They are used purely as `With<_>` / `Without<_>` query filters.

### Timed-presence markers (data-bearing)

```rust
// crates/protocol/src/character/types.rs:101-106
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RespawnTimer {
    pub expires_at: Tick,
}

// crates/protocol/src/character/types.rs:77-80
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Invulnerable {
    pub expires_at: Tick,
}
```

**Canonical add pattern** (`server/src/gameplay.rs:155-183`):

```rust
fn start_respawn_timer(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    mut events: MessageReader<DeathEvent>,
    query: Query<
        (Option<&RespawnTimerConfig>, Has<OnDeathEffects>),
        (Without<RespawnTimer>, Without<RespawnPoint>),
    >,
) {
    let tick = timeline.tick();
    for event in events.read() {
        let Ok((config, has_death_effects)) = query.get(event.entity) else { continue; };
        if has_death_effects { continue; }
        let duration = config.map(|c| c.duration_ticks).unwrap_or(DEFAULT_RESPAWN_TICKS);
        commands.entity(event.entity).insert((
            RespawnTimer { expires_at: tick + duration as i16 },
            RigidBodyDisabled,
            ColliderDisabled,
        ));
    }
}
```

**Canonical remove pattern** (`server/src/gameplay.rs:279-322`):

```rust
commands
    .entity(entity)
    .remove::<(RespawnTimer, RigidBodyDisabled, ColliderDisabled)>();
commands.entity(entity).insert(Invulnerable { expires_at: tick + 128i16 });
```

`Invulnerable` is removed by a polling system `expire_invulnerability` (`server/src/gameplay.rs:339-350`) when `tick >= invuln.expires_at`.

### Client-side observer pattern (`client/src/gameplay.rs:119-148`)

```rust
fn on_respawn_timer_added(trigger: On<Add, RespawnTimer>, ...) {
    commands.entity(entity).insert((Visibility::Hidden, RigidBodyDisabled, ColliderDisabled));
}
fn on_respawn_timer_removed(trigger: On<Remove, RespawnTimer>, ...) {
    commands.entity(entity)
        .remove::<(RigidBodyDisabled, ColliderDisabled)>()
        .insert(Visibility::Inherited);
}
```

Client uses `On<Add, T>` / `On<Remove, T>` observers (not polling systems) to react to marker presence changes for visual effects.

### Lightyear registration (`crates/protocol/src/lib.rs:163-174`)

```rust
app.register_component::<PlayerId>();
app.register_component::<ColorComponent>().add_prediction();
app.register_component::<Name>();
app.register_component::<CharacterMarker>().add_prediction();
app.register_component::<DummyTarget>().add_prediction();
app.register_component::<CharacterType>().add_prediction();
app.register_component::<Health>().add_prediction();
app.register_component::<Invulnerable>().add_prediction();
app.register_component::<RespawnTimerConfig>();
app.register_component::<RespawnTimer>().add_prediction();
```

**Convention summary:**
1. No sparse-set storage anywhere.
2. Unit-struct markers: add once, never remove (entity despawn removes them).
3. Timed markers: carry `expires_at: Tick`; inserted bundled with physics-disabling companions (`RigidBodyDisabled`, `ColliderDisabled`); removed via polling system that compares `tick` to `expires_at`.
4. Replicated markers derive `Serialize + Deserialize` and are registered `.add_prediction()`.
5. Client visual reactions use `On<Add, _>`/`On<Remove, _>` observers.

---

## Q5 — Activation-Time Gates

Existing gates in `ability_activation` and downstream pipeline — **no cooldown or resource-cost check apart from the tick-based `AbilityCooldowns`**:

| # | Gate | Location | Failure signal |
|---|------|----------|----------------|
| 1 | `action_state.just_pressed(action)` | `activation.rs:55` | silent `continue` |
| 2 | slot empty (`slots.0[slot_idx].is_none()`) | `activation.rs:58-60` | silent `continue` |
| 3 | ability not in `AbilityDefs` | `activation.rs:61-64` | `warn!` + `continue` |
| 4 | asset not yet loaded | `activation.rs:65-68` | `warn!` + `continue` |
| 5 | missing `AbilityPhases` | `activation.rs:69-72` | `warn!` + `continue` |
| 6 | on cooldown | `activation.rs:73-75` | silent `continue` |
| 7 | sub-ability recursion depth ≥ 4 | `spawn.rs:50-53` | `warn!` + `return` |
| 8 | self-hit exclusion | `hit_detection/systems.rs:62-64` | silent `continue` |
| 9 | duplicate hit (`HitTargets`) | `hit_detection/systems.rs:65-67` | silent `continue` |
| 10 | `Invulnerable` on victim | `hit_detection/effects.rs:101` | silent damage skip |

### Cooldown component & check

```rust
// crates/protocol/src/ability/types.rs
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AbilityCooldowns {
    pub last_used: [Option<Tick>; 4],
}

impl AbilityCooldowns {
    pub fn is_on_cooldown(&self, slot: usize, current_tick: Tick, cooldown_ticks: u16) -> bool {
        self.last_used[slot]
            .map(|last| (current_tick - last).unsigned_abs() <= cooldown_ticks)
            .unwrap_or(false)
    }
}
```

`activation.rs:73-75`:
```rust
if cooldowns.is_on_cooldown(slot_idx, tick, phases.cooldown) {
    continue;
}
```

### Absence of

- No resource/mana/stamina checks.
- No targeting prerequisites (line-of-sight, range, target-required gates).
- No `Grounded` / `Airborne` / state-machine gates.
- No event emission on gate failure. Failures are always silent `continue` or a `warn!` log.

---

## Q6 — Physics-Adjacent System Ordering

### `handle_character_movement`

Client (`client/src/gameplay.rs:21`):
```rust
app.add_systems(FixedUpdate, handle_character_movement);
```

Server (`server/src/gameplay.rs:32`):
```rust
app.add_systems(FixedUpdate, handle_character_movement);
```

Neither has `.in_set(...)`, `.before(...)`, `.after(...)`. No relation to avian's `PhysicsSet` expressed anywhere in the codebase.

### `update_facing`

```rust
// crates/protocol/src/lib.rs:256 (in SharedGameplayPlugin)
app.add_systems(FixedUpdate, update_facing.run_if(ready));
```

No ordering.

### Ability chain (`ability/plugin.rs:81-118`)

See Q2 for the full `.chain()` listing. Relative to each other:

```
ability_activation
  → update_active_abilities
  → apply_on_tick_effects
  → apply_while_active_effects
  → apply_on_end_effects
  → apply_on_input_effects
  → ability_projectile_spawn
[.after(apply_on_tick_effects)]
  update_hitbox_positions → process_hitbox_hits → process_projectile_hits → cleanup_hitbox_entities
[.after(process_hitbox_hits/projectile_hits)]
  expire_buffs, aoe_hitbox_lifetime, ability_bullet_lifetime
```

### Server-only systems (`server/src/gameplay.rs:34-53`)

```rust
on_death_effects
    .after(hit_detection::process_projectile_hits)
    .after(hit_detection::process_hitbox_hits),
start_respawn_timer
    .after(hit_detection::process_projectile_hits)
    .after(hit_detection::process_hitbox_hits),
process_respawn_timers.after(start_respawn_timer),
```

### Client-only `sync_camera_yaw_to_input`

```rust
app.add_systems(
    FixedPreUpdate,
    sync_camera_yaw_to_input.before(InputSystems::BufferClientInputs),
);
```

### Avian integration

`PhysicsPlugins::default()` is registered in `SharedGameplayPlugin` (`lib.rs:244-251`). Avian's `PhysicsSchedulePlugin` runs physics in `FixedPostUpdate`. Since Bevy runs `FixedUpdate` before `FixedPostUpdate`, all the movement/ability/hit-detection systems above execute **before** avian integrates `LinearVelocity` → `Position` that same frame.

**No file** uses `.in_set(PhysicsSet::...)`, `.before(PhysicsSet::...)`, or `.after(PhysicsSet::...)`. **No file** defines a custom `SystemSet` enum for physics-adjacent ordering.

---

## Q7 — Ground Contact Queries

The **only** ground-contact mechanism in the codebase is `SpatialQuery::cast_ray_predicate`, called inline in `apply_movement`:

```rust
// crates/protocol/src/character/movement.rs:30-41
spatial_query.cast_ray_predicate(
    ray_cast_origin,     // Vec3 = position.0
    Dir3::NEG_Y,
    4.0,                 // max_distance
    false,               // solid
    &filter,             // SpatialQueryFilter::from_excluded_entities([entity])
    &|hit_entity| match (player_map_id, map_ids.get(hit_entity).ok()) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    },
)
```

`spatial_query: &SpatialQuery` is passed in as a system parameter to both `handle_character_movement` callers (client/server).

### APIs not in use for ground

- `ShapeCaster` — absent.
- `RayCaster` (component-based) — absent.
- `CollidingEntities` — used for hitbox/AoE/projectile hit detection only, not ground.
- `ContactPairs` — absent.
- `Collisions` — absent.

There is **no** `Grounded` component, no cached ground state, no ground-tracking system. The only ground check is the inline per-jump raycast. If ground state is desired outside `just_pressed(Jump)`, it does not yet exist.

---

## Q8 — `PlayerActions` Enum Definition and Routing

### Definition (`crates/protocol/src/lib.rs:59-80`)

```rust
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

- `Actionlike` is **manually implemented** (not derived).
- `Move` = `DualAxis`, `CameraYaw` = scalar `Axis`, all others = `Button`.
- `CameraYaw` has no keyboard binding — written programmatically on the client.
- Registered via `app.register_type::<PlayerActions>()` at `ability/plugin.rs:45`.

### Lightyear input replication (`lib.rs:92-101`)

```rust
app.add_plugins(InputPlugin::<PlayerActions> {
    config: InputConfig::<PlayerActions> {
        rebroadcast_inputs: true,
        packet_redundancy: 20,
        ..default()
    },
});
```

- `rebroadcast_inputs: true` — server rebroadcasts inputs to other clients (needed for remote character interpolation).
- `packet_redundancy: 20` — 20 redundant input snapshots per packet. Comment notes that at 5, `srv_tick_past_buffer_end` caused stuck axis values and missed `JustPressed` transitions.

### `InputMap` built client-side only (`client/src/gameplay.rs:51-62`)

See Q2 listing. Inserted in `handle_new_character` at `Update` gated by `Added<Replicated> + With<CharacterMarker> + Has<Controlled>`.

### `CameraYaw` injection (`client/src/gameplay.rs:167-178`)

```rust
fn sync_camera_yaw_to_input(
    camera_query: Query<&CameraOrbitState>,
    mut player_query: Query<&mut ActionState<PlayerActions>, With<Predicted>>,
) {
    let Ok(orbit) = camera_query.single() else { return; };
    for mut action_state in &mut player_query {
        action_state.set_value(&PlayerActions::CameraYaw, orbit.target_angle);
    }
}
```

Registered at `FixedPreUpdate.before(InputSystems::BufferClientInputs)`.

### Movement consumers

Client `handle_character_movement` (`client/src/gameplay.rs:83-116`): filter `(With<Predicted>, With<CharacterMarker>, Without<RespawnTimer>)`.
Server `handle_character_movement` (`server/src/gameplay.rs:106-135`): filter `(With<CharacterMarker>, Without<RespawnTimer>)`.
Both call shared `apply_movement` in `movement.rs`.

### Ability consumers

`ability_activation` (`ability/activation.rs:34-113`) — shared via `AbilityPlugin`; runs on both sides. Reads `just_pressed(Ability1..4)`.
`apply_on_input_effects` (`ability/effects.rs:248-295`) — shared; mid-ability sub-ability trigger reading `just_pressed(input_effect.action)` from caster.

### Client-vs-server comparison

| System | Client | Server | Schedule |
|---|---|---|---|
| `handle_character_movement` | `client/gameplay.rs:83` (`With<Predicted>`) | `server/gameplay.rs:106` (no `Predicted`) | `FixedUpdate` |
| `ability_activation` | shared | shared, server branch inserts `Replicate`/`PredictionTarget` | `FixedUpdate` |
| `update_facing` | shared | shared | `FixedUpdate` |
| `sync_camera_yaw_to_input` | client-only | absent | `FixedPreUpdate` |
| `handle_new_character` (InputMap) | client-only | absent | `Update` |
| `apply_on_input_effects` | shared | shared | `FixedUpdate` |

Server never builds `InputMap`; it receives `ActionState<PlayerActions>` as a replicated component on the character entity (spawned with `ActionState::default()` at `server/gameplay.rs:396`).

### Full action → effect trace

1. Key press captured by leafwing on client.
2. `sync_camera_yaw_to_input` writes `CameraYaw` into `ActionState`.
3. `InputSystems::BufferClientInputs` serializes snapshot.
4. Network → server.
5. Lightyear writes `ActionState<PlayerActions>` onto server character (the one with `ControlledBy`).
6. `handle_character_movement` reads, calls `apply_movement`.
7. `ability_activation` reads `just_pressed(Ability_n)`, spawns `ActiveAbility` with `Replicate`/`PredictionTarget`.
8. `update_active_abilities` advances phases.
9. Effect systems (`apply_on_tick_effects` et al.) and hit detection apply gameplay effects.
10. Client runs the same systems locally on the `Predicted` entity. Lightyear rolls back on divergence using `should_rollback` thresholds (see Q10).

---

## Q9 — How Abilities Modify Velocity / Impulse Today

Two ability paths write to the character's physics state. **Neither uses avian's `Forces`/`ForcesItem` API**. Both bypass `ExternalForce`/`ExternalImpulse` (neither component is used anywhere in the codebase).

### Path 1: `SetVelocity` — direct `LinearVelocity` overwrite

Inside `apply_while_active_effects` (`effects.rs:139-163`):

```rust
pub fn apply_while_active_effects(
    query: Query<(&WhileActiveEffects, &ActiveAbility)>,
    mut caster_query: Query<(&Rotation, &mut LinearVelocity)>,
) {
    ...
    AbilityEffect::SetVelocity { speed, target } => {
        let target_entity = resolve_caster_target(&target, active);
        if let Ok((rotation, mut velocity)) = caster_query.get_mut(target_entity) {
            let direction = super::types::facing_direction(rotation);
            velocity.x = direction.x * speed;
            velocity.z = direction.z * speed;
        }
    }
```

Writes `velocity.x`, `velocity.z` directly; leaves `velocity.y` untouched (gravity preserved). Also handled in `apply_on_end_effects` (`effects.rs:165-246`) with `Query<(&mut Position, &Rotation, &mut LinearVelocity)>`.

### Path 2: `ApplyForce` — additive `LinearVelocity` mutation

Inside `apply_on_hit_effects` (`hit_detection/effects.rs:108-129`):

```rust
AbilityEffect::ApplyForce { force, frame, target } => {
    let entity = resolve_on_hit_target(target, victim, on_hit);
    if let Ok((target_pos, velocity, _, _)) = target_query.get_mut(entity) {
        let world_force = resolve_force_frame(...);
        if let Some(mut velocity) = velocity {
            velocity.0 += world_force;
        }
    }
}
```

`target_query: Query<(&Position, Option<&mut LinearVelocity>, &mut Health, Option<&Invulnerable>)>`. The `Option<&mut LinearVelocity>` silently no-ops if the entity lacks `LinearVelocity`.

### Contention / ordering with `movement.rs`

- `movement.rs::apply_movement` writes via `ForcesItem` (avian's force/impulse accumulator), which itself ends up mutating `LinearVelocity` inside avian internals.
- Ability effects write to `LinearVelocity` directly with `velocity.x/z =` or `velocity.0 +=`.
- These use different component *borrows* (`ForcesItem` vs `&mut LinearVelocity`), so there is no Bevy borrow conflict.
- Both are in `FixedUpdate` with **no explicit `.before`/`.after` ordering between `handle_character_movement` and the ability chain/hit-detection**. Bevy resolves order by topological sort with ambiguity.
- Avian's physics step runs in `FixedPostUpdate`, so both writers land before integration that frame.

---

## Q10 — Networking / Prediction Integration

### Registered ability components (`crates/protocol/src/lib.rs:185-208`)

```rust
app.register_component::<AbilitySlots>();
app.register_component::<ActiveAbility>()
    .add_prediction()
    .add_map_entities();
app.register_component::<AbilityCooldowns>()
    .add_prediction();
app.register_component::<ActiveShield>().add_prediction();
app.register_component::<ActiveBuffs>().add_prediction();
app.register_component::<AbilityProjectileSpawn>();
```

- `ActiveAbility`: predicted + entity-mapped (`MapEntities` impl at `types.rs:210-216` remaps `caster`, `original_caster`, `target`).
- `AbilityCooldowns`, `ActiveShield`, `ActiveBuffs`: predicted.
- `AbilitySlots`, `AbilityProjectileSpawn`: replicated only, no prediction.

Archetype components (`OnTickEffects`, `WhileActiveEffects`, `OnEndEffects`, `OnInputEffects`, `OnHitEffectDefs`, `OnHitEffects`) are **not registered** with lightyear — they are local-only, re-inserted deterministically on each peer by `apply_ability_archetype` after the `ActiveAbility` replicates / pre-spawns.

### Physics component registrations (`lib.rs:177-208`)

```rust
app.register_component::<LinearVelocity>()
    .add_prediction()
    .add_should_rollback(linear_velocity_should_rollback);

app.register_component::<Position>()
    .add_prediction()
    .add_should_rollback(position_should_rollback)
    .add_linear_correction_fn()
    .add_linear_interpolation();
```

Rollback thresholds (`lib.rs:211-224`):
- `Position`: `(a - b).length() >= 0.01`.
- `LinearVelocity`: `(a.0 - b.0).length() >= 0.01`.

**No `ExternalForce` or `ExternalImpulse` component is replicated — they are never registered.** The `Forces`/`ForcesItem` API writes through to `LinearVelocity` inside avian; `LinearVelocity` is the replicated ground-truth.

### `PreSpawned` + deterministic salt (`activation.rs:98`)

```rust
let salt = (player_id.0.to_bits()) << 32 | (slot_idx as u64) << 16 | 0u64;
commands.spawn((
    ActiveAbility { ... },
    PreSpawned::default_with_salt(salt),
    ...
));
```

Enables client prediction to spawn the same entity and match it with the server's spawn by salt. Sub-abilities use `compute_sub_ability_salt` (`spawn.rs:26-33`), hashing `player_id`, `slot`, `depth`, `id`.

### Conditional `Replicate` on server (`activation.rs:105-111`)

```rust
if let Ok(controlled_by) = server_query.get(entity) {
    commands.entity(entity_id).insert((
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
        *controlled_by,
    ));
}
```

The presence of `ControlledBy` on the caster signals server-side. Same pattern in `spawn_sub_ability` (`spawn.rs:92-98`) and `ability_projectile_spawn` (`spawn.rs:231-237`).

### `DisableRollback` on hitbox/bullet entities

Hitboxes and bullets are local-only:
- `spawn_melee_hitbox` (`spawn.rs:134`) — `DisableRollback`.
- `spawn_aoe_hitbox` (`spawn.rs:177`) — `DisableRollback`.
- `handle_ability_projectile_spawn` (`spawn.rs:271`) — `DisableRollback`.

### Guard against duplicate projectile spawn on client

`handle_ability_projectile_spawn` query (`spawn.rs:245-250`):
```rust
spawn_query: Query<
    (Entity, &AbilityProjectileSpawn, Option<&OnHitEffects>, &MapInstanceId),
    (Without<AbilityBullets>, Without<Replicated>),
>
```

`Without<Replicated>` prevents the client from spawning a bullet again when the server's `AbilityProjectileSpawn` component replicates in (client already spawned via prediction).

### Rollback-safe despawn

`advance_ability_phase` (`activation.rs:140`): `commands.entity(entity).prediction_despawn()`.
`despawn_ability_projectile_spawn` (`spawn.rs:280-290`): `c.prediction_despawn()`.

### `LocalTimeline` for tick reads

All tick-dependent systems use `Res<LocalTimeline>` and `timeline.tick()` — lightyear's abstraction that returns the predicted tick on client and the simulation tick on server.

### Predicted vs confirmed pattern

- **Predicted** state (runs identically on client predicted entity and server, reconciled on divergence): `ActiveAbility`, `AbilityCooldowns`, `ActiveShield`, `ActiveBuffs`, `LinearVelocity`, `Position`, `Rotation`, `AngularVelocity`.
- **Replicated-only** (server authoritative, client receives): `AbilitySlots`, `AbilityProjectileSpawn`, `CharacterMarker`, `DummyTarget`, `Invulnerable`, `RespawnTimer`.
- **Local-only** (never replicated, computed per-peer): hitbox entities, bullet entities themselves (they are derived from `AbilityProjectileSpawn`), ability archetype components.

---

## Q11 — What `ApplyForce` via `apply_linear_impulse` Would Look Like

### Current `ApplyForce` site

```rust
// crates/protocol/src/hit_detection/effects.rs:108-129
AbilityEffect::ApplyForce {
    force,
    frame,
    target,
} => {
    let entity = resolve_on_hit_target(target, victim, on_hit);
    if let Ok((target_pos, velocity, _, _)) = target_query.get_mut(entity) {
        let world_force = resolve_force_frame(
            *force,
            frame,
            source_pos,
            target_pos.0,
            on_hit.caster,
            entity,
            rotation_query,
        );
        if let Some(mut velocity) = velocity {
            velocity.0 += world_force;
        }
    } else {
        warn!("ApplyForce target {:?} not found", entity);
    }
}
```

Outer query shape:

```rust
target_query: &mut Query<(
    &Position,
    Option<&mut LinearVelocity>,
    &mut Health,
    Option<&Invulnerable>,
)>,
```

### What `apply_linear_impulse` does in avian (source: avian3d 0.5.0)

```rust
// avian3d/src/dynamics/rigid_body/forces/query_data.rs:276-284
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

Called through the `Forces` QueryData which provides `ForcesItem`. The full `Forces` tuple gives access to `LinearVelocity`, `ComputedMass`, `LockedAxes`, `SleepTimer`, `ExternalImpulse`, `ExternalForce`, etc. Impulse is divided by `ComputedMass` via `inverse_mass()`, masked by `LockedAxes`, added to `LinearVelocity`.

### Query shape change required

Current: `Option<&mut LinearVelocity>`.
Required: a full `Forces` QueryData param on the target entity. In the single-query form movement.rs uses:

```rust
target_query: Query<(&Position, Forces, &mut Health, Option<&Invulnerable>)>
```

where iterating yields `(&Position, ForcesItem, Mut<Health>, Option<&Invulnerable>)`. The `Option<_>` wrapper around `Forces` is not a supported pattern — `Forces` is a required QueryData, so the target entity **must** have a valid rigid-body archetype (`RigidBody` + `LinearVelocity` + `ComputedMass` + `LockedAxes` etc.).

### Entity requirements (current vs. switched)

Character entities already have the full rigid-body archetype via `CharacterPhysicsBundle` (`character/types.rs:111-130`):

```rust
// representative fields:
RigidBody::Dynamic,
Collider::capsule(CHARACTER_CAPSULE_RADIUS, CHARACTER_CAPSULE_HEIGHT),
LinearVelocity::ZERO,
AngularVelocity::ZERO,
LockedAxes::ROTATION_LOCKED,
...
```

- No explicit `Mass` or `ColliderDensity` — avian computes `ComputedMass` from the capsule geometry at default density `1.0`.
- `CHARACTER_CAPSULE_RADIUS = 2.0`, `CHARACTER_CAPSULE_HEIGHT = 2.0` → capsule volume ≈ 58.6. `ComputedMass ≈ 58.6` (not a small number).

All current `ApplyForce` victims are characters, so the archetype is present. Switching the query to `Forces` would still match them. Entities without `RigidBody` (e.g., AoE hitboxes, bullets) were never valid `ApplyForce` targets and still wouldn't be.

### Mass scaling of existing RON values

All current `ApplyForce` values from the asset files (all applied on hit against characters):

| Ability | RON | frame |
|---|---|---|
| uppercut | `force: (0.0, 14.0, 0.0)` | `World` |
| shockwave | `force: (0.0, 1.5, 8.0)` | `RelativePosition` |
| punch | `force: (0.0, 0.9, 0.5)` | `RelativePosition` |
| punch2 | `force: (0.0, 1.05, 0.5)` | `RelativePosition` |
| punch3 | `force: (0.0, 2.4, 7.65)` | `RelativePosition` |
| shield_bash | `force: (0.0, 1.0, 5.5)` | `RelativePosition` |
| ground_pound | `force: (0.0, 8.0, 8.0)` | `RelativePosition` |
| dive_kick | `force: (0.0, 0.5, 3.0)` | `RelativePosition` |

With the current code (`velocity.0 += world_force`), these values are **velocity deltas in m/s** (no mass division).

Under `apply_linear_impulse`, the velocity delta is `impulse / ComputedMass ≈ impulse / 58.6`. To preserve current behavior, each RON `force` value would need to be multiplied by `ComputedMass` (approximately ×58.6). For reference, the jump uses `Vec3::new(0.0, 2000.0, 0.0)` as an impulse → velocity delta of `~34 m/s` upward; `uppercut`'s `14.0 m/s` corresponds to an impulse of `~820`.

Because `ComputedMass` is derived (not an explicit constant), its exact value is what avian computes from the capsule; the scale factor would be "whatever the character's `ComputedMass` is at runtime." Designers currently hand-tune RON in m/s velocity-delta space; after the switch they would hand-tune in impulse space (`kg·m/s` — divided by a fixed character mass to get the resulting velocity delta).

### Networking / prediction behavior

Both paths write to the **same predicted component, `LinearVelocity`**. `apply_linear_impulse` uses `*self.linear_velocity_mut() += delta_vel` — identical target to `velocity.0 += world_force`. Same `.add_should_rollback(linear_velocity_should_rollback)` governs rollback. **No networking change is required**; no new component needs registration.

Two minor behavioral differences introduced by `apply_linear_impulse`:
1. `try_wake_up()` is called (resets `SleepTimer` to 0.0). Characters are generally awake anyway.
2. `LockedAxes::apply_to_vec` masks components. Characters have `LockedAxes::ROTATION_LOCKED` which affects rotation only — translational axes pass through unchanged.

### Schedule ordering

Unchanged. `process_hitbox_hits` / `process_projectile_hits` (which call `apply_on_hit_effects`) run in `FixedUpdate`, chained after `apply_on_tick_effects`. Avian integrates `LinearVelocity` in `FixedPostUpdate`. Writing through `apply_linear_impulse` in `FixedUpdate` → integrated that frame, identical to the current direct write.

### Summary: what changes

| Axis | Current | Switched |
|---|---|---|
| Write target | `LinearVelocity.0 += force` | `apply_linear_impulse(impulse)` → `LinearVelocity += impulse / mass` (after `LockedAxes` mask, wakes body) |
| Query param | `Option<&mut LinearVelocity>` | `Forces` QueryData (requires full rigid-body archetype) |
| Entity requirement | Any entity; silently no-ops without `LinearVelocity` | Must have full rigid-body archetype (all current `ApplyForce` targets already do) |
| RON value unit | velocity delta (m/s) | impulse (kg·m/s); needs `×ComputedMass` to match current motion |
| Networking/prediction | writes `LinearVelocity` (predicted) | writes `LinearVelocity` (predicted) — unchanged |
| Schedule ordering | `FixedUpdate`, before avian integration | unchanged |

---

## Cross-Cutting Observations

### Plugin composition

`SharedGameplayPlugin` (`crates/protocol/src/lib.rs:239-256`) is added by both client and server. It composes:
- `ProtocolPlugin` (component/resource/prediction registrations + `InputPlugin::<PlayerActions>`).
- `AbilityPlugin` (activation, lifecycle, effects, projectile-spawn, hit-detection system chains).
- `PhysicsPlugins::default()` (avian).
- `update_facing` at `FixedUpdate`.

Anything added here runs identically on both peers, which is how client prediction matches server simulation.

### Tick source

Every tick-aware system reads `Res<LocalTimeline>` and `timeline.tick()`. No system reads bevy `Time` or a custom tick counter for ability logic.

### Replicate-and-prediction idiom on spawn

`ability_activation`, `spawn_sub_ability`, and `ability_projectile_spawn` share the exact same conditional insert triggered by `ControlledBy` presence. This is the consistent pattern for "server spawns, client predicts" gameplay entities.

### Rollback-safe despawn

Every despawn of a tick-replicated entity goes through `commands.entity(e).prediction_despawn()` rather than `.despawn()`.

### `Forces`/`ForcesItem` usage

Only `apply_movement` uses the `Forces` QueryData and `ForcesItem` methods (`apply_linear_impulse`, `apply_force`). No other system in the codebase writes forces through the avian `Forces` API.

### No custom `SystemSet` enums

Ordering is expressed via `.chain()` and direct `.before(fn)/.after(fn)` constraints. There is no project-level `SystemSet` taxonomy.

### Inline early-outs

All ability gates use `Let/Some-else { warn!/silent; continue/return }`. Failures never emit events.

---

## Open Areas

- **Avian volume formula for `ComputedMass`**: the ~58.6 mass figure is computed from `π·r²·(4r/3 + h)` with `r=2`, `h=2` at default density `1.0`. The true value is whatever avian's capsule volume function produces at runtime; this research did not execute avian to confirm the exact numeric.
- **`TransitionReadySent` insert site**: only a `remove::<TransitionReadySent>` call was located. The insert path may live in a transition file not read; documented where it is removed only.
- **`DummyTarget` currently unused**: its only spawn site is commented out in `server/src/gameplay.rs:86-104`. It is still registered with lightyear and referenced in a `Without<DummyTarget>` query.
- **Ambiguity between `handle_character_movement` and the ability chain**: both are in `FixedUpdate` without ordering. Bevy resolves via topological sort; observed runtime order was not measured here.
