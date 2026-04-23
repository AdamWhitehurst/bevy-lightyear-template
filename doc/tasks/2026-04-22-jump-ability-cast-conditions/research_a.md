# Research Findings

## Q1: Jump implementation in `movement.rs`

### Findings
- **Input detection**: `action_state.just_pressed(&PlayerActions::Jump)` gates the whole block — fires only on the tick the button is first pressed (`crates/protocol/src/character/movement.rs:25`).
- **Ground detection**: inline `SpatialQuery::cast_ray_predicate` call at `movement.rs:30-42`.
  - Origin: `position.0` (character `Position`); direction: `Dir3::NEG_Y`; max distance: `4.0`; `exclude_dynamic: false`.
  - Filter: `SpatialQueryFilter::from_excluded_entities([entity])` excludes the character itself (`movement.rs:28`).
  - Predicate (`movement.rs:37-39`): accepts a hit only if the hit entity's `MapInstanceId` matches the character's, or either has no `MapInstanceId` — map instance isolation.
  - No shape-cast, no `RayCaster` component, no `CollidingEntities`, no `Collisions`/`ContactPairs` — just a single ad-hoc ray query.
- **Impulse**: `forces.apply_linear_impulse(Vec3::new(0.0, 2000.0, 0.0))` (`movement.rs:44`) — raw +Y impulse via avian's `Forces` SystemParam.
- **Components read**: `Entity`, `ComputedMass`, `Position`, `ActionState<PlayerActions>`, `Option<&MapInstanceId>`, `Query<&MapInstanceId>` (for predicate).
- **Components written**: `ForcesItem` (impulse at `:44`, horizontal force at `:64`); horizontal path reads `LinearVelocity` via `forces.linear_velocity()` (`movement.rs:55`).

## Q2: End-to-end ability flow

### Findings
- **Trigger**: `ability_activation` (`crates/protocol/src/ability/activation.rs:34`) reads `ActionState<PlayerActions>`; iterates `ABILITY_ACTIONS = [Ability1..Ability4]` (`activation.rs:17-22`) and calls `just_pressed` per slot (`activation.rs:55`).
- **Activation system**: `activation.rs:34-114`
  - Looks up slot → `AbilityId` via `AbilitySlots`; loads `AbilityAsset` via `AbilityDefs`.
  - Cooldown check: `AbilityCooldowns.is_on_cooldown(slot, tick, cooldown_ticks)` at `:73` — failure is a silent `continue` (`:75`).
  - Records `cooldowns.last_used[slot_idx] = Some(tick)` at `:77`.
  - Spawns `ActiveAbility` entity with `PreSpawned::default_with_salt(salt)` encoding `player_id << 32 | slot_idx << 16` (`:86-101`).
  - `apply_ability_archetype` stamps reflected components from asset (`:103`; impl `loader.rs:24-55`).
  - Server path conditionally inserts `Replicate::to_clients(All)`, `PredictionTarget::to_clients(All)`, `ControlledBy` (`:105-111`).
- **Lifecycle progression**: `update_active_abilities` (`activation.rs:145`) calls `advance_ability_phase` (`:116-143`).
  - Phase machine: `Startup → Active → Recovery`; transitions by comparing `tick - phase_start_tick` to `phases.phase_duration(&phase)` (`:124`).
  - On Startup→Active: inserts `OnHitEffects` (`:164-169`).
  - On Active→Recovery: removes `OnHitEffects` (`:174-176`).
  - End of Recovery: `commands.entity(entity).prediction_despawn()` (`:140`).
  - Observer `cleanup_effect_markers_on_removal` strips effect-marker components on `Remove<ActiveAbility>` (`lifecycle.rs:26`).
  - Other lifecycle timers: `expire_buffs` (`lifecycle.rs:9`), `aoe_hitbox_lifetime` (`lifecycle.rs:42`), `ability_bullet_lifetime` (`lifecycle.rs:56`).
- **Effect systems** (`effects.rs`, all chained in `FixedUpdate`):
  - `apply_on_tick_effects` (`:28`): during `Active` phase, matches `tick_effect.tick == active_offset`; dispatches `Melee`/`AreaOfEffect`/`Projectile`/`Ability`/`Teleport`/`Shield`/`Buff`.
  - `apply_while_active_effects` (`:139`): every Active tick; handles `SetVelocity` — directly writes `LinearVelocity.x/.z` (`:153-155`).
  - `apply_on_end_effects` (`:165`): fires only when `phase == Recovery && phase_start_tick == current_tick`.
  - `apply_on_input_effects` (`:248`): during Active, checks `just_pressed` on caster; dispatches `Ability` sub-abilities.
- **Complete chain**: input `just_pressed` → `ability_activation` → spawn `ActiveAbility` → `update_active_abilities` advances phases → tick/while/end/input effect systems → hit detection (`process_hitbox_hits`, `process_projectile_hits`) → `apply_on_hit_effects` (`hit_detection/effects.rs:62`) → Recovery elapses → `prediction_despawn` → `cleanup_effect_markers_on_removal`.

## Q3: Ability data schema

### Findings
- **Format**: RON, extension `ability.ron`. Loader registered at `crates/protocol/src/ability/loader.rs:78`.
- **Example files**: `assets/abilities/punch.ability.ron`, `speed_burst.ability.ron`, `blink_strike.ability.ron`; slots default at `assets/default.ability_slots.ron`.
- **File structure**: map of fully-qualified type path → reflected component value. Example keys:
  - `"protocol::ability::AbilityPhases"` — `(startup, active, recovery, cooldown)` tick counts.
  - `"protocol::ability::OnTickEffects"` — `Vec<(tick: u16, effect: AbilityEffect)>`.
  - `"protocol::ability::OnHitEffectDefs"`, `WhileActiveEffects`, `OnEndEffects`, `OnInputEffects` — lists of `AbilityEffect`.
- **`AbilityEffect` variants** (`crates/protocol/src/ability/types.rs:47-99`): `Melee`, `Projectile { speed, lifetime_ticks }`, `SetVelocity { speed, target }`, `Damage { amount, target }`, `ApplyForce { force, frame, target }`, `AreaOfEffect { radius, duration_ticks }`, `Ability { id, target }`, `Teleport { distance }`, `Shield { absorb }`, `Buff { stat, multiplier, duration_ticks, target }`.
- **`AbilityAsset`**: `Vec<Box<dyn PartialReflect>>` (`types.rs:330`) — not typed fields, raw reflected components.
- **Load flow** (`loader.rs:78-93`): `AbilityAssetLoader` reads bytes → `reflect_loader::deserialize_component_map(&bytes, &registry)` → `AbilityAsset { components }`.
- **Orchestration** (`loading.rs`): native `load_ability_defs` loads `"abilities"` folder (`:37`); `insert_ability_defs` builds `AbilityDefs` HashMap keyed by filename stem (`:87-109`); WASM uses `abilities.manifest.ron` (`:42-84`); hot-reload via `AssetEvent::Modified` (`:151`); default slots at `:247-284`.
- **Instantiation**: `apply_ability_archetype` (`loader.rs:24`) iterates asset components, looks up `ReflectComponent`, `insert`s each onto the entity (`:40-55`). Sub-abilities via `spawn_sub_ability` with depth guard of 4 (`spawn.rs:36, :50`).

## Q4: Sparse-set component conventions

### Findings
- **No sparse-set components exist.** No occurrences of `StorageType::SparseSet` or `#[component(storage = "SparseSet")]` anywhere in the codebase.
- All components currently use default table storage.
- Marker-style components (e.g., `ActiveAbility`, `ActiveShield`, `ActiveBuffs`, `OnHitEffects`) are inserted/removed via `commands.entity(e).insert(...)` / `.remove::<T>()` or reflected insertion in `apply_ability_archetype` (`loader.rs:40-55`).

## Q5: Activation-time gates

### Findings
- **Only cooldowns**: `AbilityCooldowns.is_on_cooldown(slot, tick, cooldown_ticks)` (`types.rs:233`) checked at `activation.rs:73-75`.
- **No other gates**: no mana/resource checks, no character-state checks (grounded, stunned, airborne), no targeting prerequisites, no interrupt conditions.
- **Failure signaling**: silent `continue` in loop — no event, no log, no component insertion on failure (`activation.rs:75`).

## Q6: System ordering for physics-adjacent systems

### Findings
- **Movement entry points** in `FixedUpdate`:
  - Server: `ServerGameplayPlugin::build` (`crates/server/src/gameplay.rs:32`) registers `handle_character_movement` with no ordering.
  - Client: `ClientGameplayPlugin::build` (`crates/client/src/gameplay.rs:20`) — same, no ordering.
  - `update_facing`: `SharedGameplayPlugin::build` (`crates/protocol/src/lib.rs:256`) with only `.run_if(in_state(AppState::Ready))`.
  - `sync_camera_yaw_to_input`: `FixedPreUpdate`, `.before(InputSystems::BufferClientInputs)` (`client/src/gameplay.rs:23-24`).
- **Avian physics**: `PhysicsPlugins::default()` added in `SharedGameplayPlugin::build` at `lib.rs:244-251` with `MapCollisionHooks`; `PhysicsTransformPlugin`, `PhysicsInterpolationPlugin`, `IslandSleepingPlugin` are disabled. Avian's internal `PhysicsSet` runs in `FixedUpdate` — `handle_character_movement` has no explicit ordering relative to avian sets.
- **Ability systems** (`crates/protocol/src/ability/plugin.rs:83-93`) run in `FixedUpdate` as a single `.chain()`: `ability_activation → update_active_abilities → apply_on_tick_effects → apply_while_active_effects → apply_on_end_effects → apply_on_input_effects → ability_projectile_spawn`. Hit detection runs `.after(apply_on_tick_effects)` (`plugin.rs:99-107`).
- **No cross-ordering** declared between movement systems and ability-effect systems despite both writing to `LinearVelocity` / `Forces`.
- **Primitives used**: `.run_if`, `.before(InputSystems::BufferClientInputs)`, `.after(hit_detection::*)` (for death/respawn, not movement), `.chain()` in ability plugin. No named `SystemSet` or `.in_set` for movement systems.

## Q7: Ground-contact queries

### Findings
- **API in use**: `SpatialQuery::cast_ray_predicate` exclusively. No `ShapeCaster`, no `RayCaster` component, no `CollidingEntities`, no `Collisions`, no `ContactPairs`.
- **Only call site**: `crates/protocol/src/character/movement.rs:30-42`, fired every tick the Jump button is just-pressed.
- **SystemParam acquisition**: `SpatialQuery` obtained in `handle_character_movement` at `server/src/gameplay.rs:108` and `client/src/gameplay.rs:85`; passed to `apply_movement` (`server:129`, `client:109`).
- **`MapCollisionHooks`** in `physics.rs` implements `CollisionHooks::filter_pairs` for map-instance isolation; does no ground detection.

## Q8: `PlayerActions` input routing

### Findings
- **Definition**: `crates/protocol/src/lib.rs:59-70`. Variants: `Move` (DualAxis), `CameraYaw` (Axis), `Jump`, `PlaceVoxel`, `RemoveVoxel`, `Ability1`, `Ability2`, `Ability3`, `Ability4`. `Actionlike` impl at `:72-80`.
- **InputMap** (client only): `crates/client/src/gameplay.rs:51-61`, inserted on confirmed+controlled entity in `handle_new_character`. Jump→Space/GamepadSouth; Move→WASD/LeftStick; PlaceVoxel→LMB; RemoveVoxel→RMB; Ability1-4→Digit1-4.
- **`CameraYaw`** is synthetic: `sync_camera_yaw_to_input` (`client/src/gameplay.rs:167-178`) writes camera orbit state into `ActionState<PlayerActions>` via `set_value` in `FixedPreUpdate.before(InputSystems::BufferClientInputs)`.
- **Movement routing**: `handle_character_movement` (both sides) calls `apply_movement` which reads `Move` and `Jump` directly from `ActionState<PlayerActions>`. Client filter: `With<Predicted> + With<CharacterMarker> + Without<RespawnTimer>` (`client/gameplay.rs:86-116`). Server filter: `With<CharacterMarker> + Without<RespawnTimer>` (`server/gameplay.rs:106-135`).
- **Ability routing**: `ability_activation` (`ability/activation.rs:34-114`) iterates `ABILITY_ACTIONS` and checks `just_pressed`; runs via `AbilityPlugin` (shared, not client/server split).
- **Leafwing**: `leafwing::InputPlugin::<PlayerActions>` registered at `crates/protocol/src/lib.rs:92-101` with `rebroadcast_inputs: true`, `packet_redundancy: 20`.

## Q9: Ability writes to physics

### Findings
- **Direct `LinearVelocity` writes** (no `ExternalImpulse`/`ExternalForce`):
  - `apply_while_active_effects` — `SetVelocity` writes `.x`/`.z` (`effects.rs:153-155`), every Active tick.
  - `apply_on_end_effects` — `SetVelocity` writes `.x`/`.z` at first Recovery tick (`effects.rs:185-189`).
  - `apply_on_hit_effects` — `ApplyForce` does `velocity.0 += world_force` (`hit_detection/effects.rs:108-129`, add at `:125`); force transformed via `ForceFrame`.
  - `Teleport` writes `Position.0` directly, not velocity (`effects.rs:297-308`).
- **Contention/ordering vs. movement**: movement writes via `forces.apply_force` / `apply_linear_impulse` (through avian's `Forces` SystemParam), while ability effects write `LinearVelocity` directly. Both run in `FixedUpdate` with **no declared ordering relationship** between them — last-writer-wins behavior is whatever Bevy schedules, and `SetVelocity` overwrites absolute values (not additive), potentially erasing jump impulse if it runs after movement in the same tick.

## Q10: Ability ↔ lightyear networking

### Findings
- **Component registration** (`crates/protocol/src/lib.rs:185-194`):
  - `AbilitySlots`: `register_component` only (no prediction — server-authoritative loadout).
  - `ActiveAbility`: `register_component + add_prediction() + add_map_entities()`.
  - `AbilityCooldowns`, `ActiveShield`, `ActiveBuffs`: `register_component + add_prediction()`.
  - `AbilityProjectileSpawn`: `register_component` only.
- **Pre-spawning**: `ActiveAbility` spawned with `PreSpawned::default_with_salt(salt = player_id<<32 | slot_idx<<16)` (`activation.rs:86-101`) — lightyear client predicts the same entity the server will spawn.
- **Server replication**: when `ControlledBy` present, inserts `Replicate::to_clients(NetworkTarget::All)`, `PredictionTarget::to_clients(NetworkTarget::All)`, `ControlledBy` (`activation.rs:105-111`).
- **Entity remapping**: `add_map_entities()` on `ActiveAbility` (`lib.rs:189`) remaps `caster`/`original_caster`/`target` on replication.
- **Despawn**: `prediction_despawn()` (not `despawn()`) at end of Recovery (`activation.rs:140`).
- **Shared execution**: `AbilityPlugin` registered in `SharedGameplayPlugin` — same systems run on client (predicted) and server (authoritative).
- **Not network-registered** (local-only): `OnHitEffects`, `MeleeHitbox`, `AoEHitbox`, `HitTargets` — computed locally from replicated `ActiveAbility` state.
- **No custom rollback hooks**: no `should_rollback` fn registered for ability components — defaults apply.

## Q11: What would it take to apply `ApplyForce` via `Forces::apply_linear_impulse`?

### Current `ApplyForce` path
- Free function `apply_on_hit_effects` (`crates/protocol/src/hit_detection/effects.rs:62-82`), called from `process_hitbox_hits` and `process_projectile_hits` — not a Bevy system itself.
- Target query: `Query<(&Position, Option<&mut LinearVelocity>, &mut Health, Option<&Invulnerable>)>` (`effects.rs:73-78`).
- Match arm (`effects.rs:108-130`): resolves world-space force via `resolve_force_frame` (`effects.rs:35-60`) then `velocity.0 += world_force` at `effects.rs:125` — **additive**, **no mass scaling**.
- `ForceFrame` variants (`types.rs:35-42`): `World`, `Caster`, `Victim`, `RelativePosition`, `RelativeRotation`. Unchanged by the API swap.
- Silent skip when target has no `LinearVelocity` (`effects.rs:124`).

### What `Forces::apply_linear_impulse` does
- `Forces` is `#[derive(QueryData)]` (avian `query_data.rs:105-120`) — lives inside `Query<Forces>`, **not** a `SystemParam`. Fields include `&mut LinearVelocity`, `&mut AngularVelocity`, `&ComputedMass`, `&ComputedAngularInertia`, `&ComputedCenterOfMass`, `Option<&LockedAxes>`, `&mut VelocityIntegrationData`, `&mut AccumulatedLocalAcceleration`, `Option<&mut SleepTimer>`, `Has<Sleeping>`.
- `apply_linear_impulse` (avian `query_data.rs:276-284`): computes `effective_inverse_mass = locked_axes.apply_to_vec(Vec3::splat(1.0 / mass))`, then `*linear_velocity += effective_inverse_mass * impulse`. Writes `LinearVelocity` immediately (same tick, same component).
- **Mass scaling**: `Δv = impulse / mass`. Current code produces `Δv = force` (treats the RON value as a velocity delta). With `apply_linear_impulse`, `Δv = force / mass`.
- Jump impulse at `movement.rs:44` uses `Vec3::new(0.0, 2000.0, 0.0)` — scaled for character mass (≈ 70 kg → Δv ≈ 28 m/s before gravity damping).

### Required changes
- **Query shape**: `apply_on_hit_effects` receives borrowed queries. To call `Forces`, one of the call sites (`process_hitbox_hits` / `process_projectile_hits`, `plugin.rs:96-107`) must expose a `Query<Forces, ...>`. Either merge `Forces` into the existing target query alongside `Health`/`Invulnerable`, or split the effect into a separate system that takes `Query<Forces>` and runs after the hit-processing system.
- **Borrow conflicts**: the existing `rotation_query: &Query<&Rotation>` (`effects.rs:81`) reads `Rotation` for frame resolution. `Forces` does **not** include `Rotation`, so a separate read-only `Rotation` query still works — no conflict.
- **Component requirements**: `Forces` will only match entities that have `RigidBody` and all avian-derived components (`ComputedMass`, `ComputedAngularInertia`, `ComputedCenterOfMass`, `VelocityIntegrationData`, `AccumulatedLocalAcceleration`). Characters already have these via `CharacterPhysicsBundle`. Any target lacking `RigidBody` (purely kinematic entities) would **silently no-op** — stricter than today's "has LinearVelocity" check.
- `LinearVelocity` and `AngularVelocity` would be required (not `Option<>`) on targets matched by `Forces`.

### Numeric impact (asset re-tuning)
All existing `ApplyForce` uses are in `OnHitEffectDefs` with `frame: RelativePosition`, `target: Victim`:
- `assets/abilities/punch.ability.ron:7` — `force: (0.0, 0.9, 0.5)`
- `assets/abilities/punch2.ability.ron:7` — `force: (0.0, 1.05, 0.5)`
- `assets/abilities/punch3.ability.ron:7` — `force: (0.0, 2.4, 7.65)`

Today these are Δv in m/s. Under `apply_linear_impulse`, to preserve current behavior at mass `m`, each value must be **multiplied by `m`**. Using `m = 70` kg: punch3's `(0, 2.4, 7.65)` becomes `(0, 168, 535.5)`. Without re-tuning, existing knockback becomes ~70× weaker.

### Network/prediction implications
- `LinearVelocity` is registered with prediction + custom rollback: `app.register_component::<LinearVelocity>().add_prediction().add_should_rollback(linear_velocity_should_rollback)` (`crates/protocol/src/lib.rs:177-179`).
- Both the current `velocity.0 +=` path and `apply_linear_impulse` write `LinearVelocity` directly → both are equally rollback-visible. **No change in prediction behavior.**
- `VelocityIntegrationData` and `AccumulatedLocalAcceleration` are **not** registered for prediction. This matters only if switching to `apply_force` (not `apply_linear_impulse`), which accumulates into `VelocityIntegrationData`. Impulses bypass that accumulator.
- No `ExternalImpulse` / `ExternalForce` exist in this avian 0.5 setup — those legacy components aren't present anywhere (`lib.rs` grep shows none registered).

### Ordering
- `PhysicsSchedule` runs in `FixedPostUpdate` by default (avian `schedule/mod.rs:54`).
- Hit-processing systems run in `FixedUpdate` (`plugin.rs:96-107`), before `FixedPostUpdate` consumes `LinearVelocity` for integration.
- Both today's `+=` and the proposed `apply_linear_impulse` write in `FixedUpdate`, consumed identically by avian's integrator in `FixedPostUpdate`. **No ordering change needed.**

### Summary of what changes
1. Call site refactor: thread a `Query<Forces>` (or `Query<ForcesItem>`-bearing tuple) into `apply_on_hit_effects`, or split `ApplyForce` out into its own post-hit system.
2. Tighter entity requirements (must have `RigidBody` + avian derived components).
3. **Asset re-tuning mandatory** — existing RON values become mass-scaled and will produce drastically smaller knockback without adjustment.
4. Semantics change from "Δvelocity" to "impulse (N·s)"; mass-heavy targets now take proportionally less push, mass-light targets take more — physically consistent, but a behavioral change.
5. No networking/prediction changes; no schedule reordering.

## Cross-Cutting Observations

- **Two independent physics-write paths** share `FixedUpdate` without declared ordering: (a) `apply_movement` → `Forces` impulse/force, (b) ability `SetVelocity`/`ApplyForce` → direct `LinearVelocity`.
- **Ground detection is a one-shot inline ray-cast** in `apply_movement` — not a persistent `Grounded`/`Airborne` marker, not cached, not exposed outside `movement.rs`.
- **Activation gates pipeline is minimal**: cooldown only; no generic pre-check phase between input and `ActiveAbility` spawn.
- **Reflection-driven asset model**: `AbilityAsset` is a bag of boxed reflected components; any registered reflectable `Component` can be attached by placing its type-path in the RON file. This means new "gate" components could be attached to an ability via the asset rather than hard-coded in activation.
- **Shared plugin pattern**: ability activation and effects run identically on client (via prediction) and server; only movement has a client/server split (different query filters, same `apply_movement` body).

## Open Areas

- **Named `SystemSet`s for physics/ability phases** — none exist. Any future ordering requirement (e.g., "ability gates before movement writes") has no existing scaffolding to hook into.
- **No `Grounded`/`Airborne` marker component exists**. Ground state is only queried at the moment of jump input; no other system observes it.
- **No generic "activation condition" abstraction** exists; cooldown logic is inlined in `ability_activation` without a trait or pluggable predicate pipeline.

Q1: C Make Space an ability slot like the 1-4 abilities, as similar as possible to support keybinding remapping for all ability keys later 
Q2: What do you mean none of the existing effects work to replicate the jump impulse? Why can't ApplyForce work here? Why would we need to "re-interpret `frame`"?
Q3: I think we should make the conditional an Optional field on InputEffect so that we can specify different behavior e.g. Jump on ground, spin while Airborne
