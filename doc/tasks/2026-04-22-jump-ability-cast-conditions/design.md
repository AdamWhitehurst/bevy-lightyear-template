# Design Discussion

## Current State

- Jump is inline in `apply_movement` (`crates/protocol/src/character/movement.rs:28-47`): on `just_pressed(Jump)` a downward ray cast from `position.0` (max 4.0, map-filtered) and, on hit, `forces.apply_linear_impulse((0, 2000, 0))`. No cooldown, no event, no stored grounded state.
- Ability pipeline (chained in `FixedUpdate`, `ability/plugin.rs:81-93`): `ability_activation` → `update_active_abilities` → `apply_on_tick_effects` → `apply_while_active_effects` → `apply_on_end_effects` → `apply_on_input_effects` → `ability_projectile_spawn`.
- Activation gate is cooldown-only (`activation.rs:73-77`). No state checks, no resource costs. Failure is a bare `continue` with no event.
- `AbilitySlots(pub [Option<AbilityId>; 4])` (`types.rs:180`), bound to `ABILITY_ACTIONS = [Ability1..=Ability4]` (`activation.rs:17-22`). `AbilityCooldowns::last_used: [Option<Tick>; 4]` (`types.rs:219`).
- Ability definitions load from `*.ability.ron` into `AbilityAsset { components: Vec<Box<dyn PartialReflect>> }` via `AbilityAssetLoader`, then `apply_ability_archetype` inserts each reflected component onto the spawned `ActiveAbility` entity (`loader.rs:24-56`). `extract_phases` (`loader.rs:9-21`) is the template for typed extraction from the reflected list.
- No sparse-set components exist in the workspace (`research.md` Q4). No `SystemSet` is defined by game code.
- Ground contact query exists only at `movement.rs:31-42` (`SpatialQuery::cast_ray_predicate`).
- `AbilityEffect::ApplyForce { force, frame, target }` currently writes directly into `LinearVelocity` via `velocity.0 += world_force` (mass-independent, no wake, no locked-axes handling) at `hit_detection/effects.rs:108-129`. The only content using it today is the melee on-hit RON: `ApplyForce(force: (0.0, 0.9, 2.85), frame: RelativePosition, target: Victim)` (`reflect_loader.rs:162-172`).

## Desired End State

1. Jump is removed from `apply_movement`. The `PlayerActions::Jump` input drives activation of a data-defined jump ability identical in mechanics (same ray origin/distance/map filter, same `(0, 2000, 0)` impulse via `forces.apply_linear_impulse`).
2. Ground detection is a dedicated system `detect_grounded` that maintains an `IsGrounded` sparse-set marker on character entities, scheduled `.before(handle_character_movement)` and `.before(ability_activation)` in `FixedUpdate`.
3. A data-driven `ConditionalEffects` component on ability definitions lets abilities declare state-gated effect branches: if present and no entry matches, activation is refused; if any match, their effects are queued for dispatch at tick 0 of Active. One mechanism serves both "only cast if grounded" and "grounded→jump / airborne→flip" use cases.
4. `AbilityEffect::ApplyForce` is converted wholesale to use `forces.apply_linear_impulse` (mass-scaled, wake-aware, locked-axes-aware). Existing melee RON value is rescaled by mass to preserve hit feel.

**Verification**:
- Holding `Space` repeatedly jumps exactly as before (same arc, same cadence when landing).
- Jumping in mid-air does nothing (no impulse, no cooldown consumption, no `ActiveAbility` spawn).
- `cargo check-all` and `cargo test-all` pass.
- Melee on-hit knockback preserves its previous feel after rescaling.
- Removing `ConditionalEffects` from the jump RON (or replacing its condition with `Airborne`) changes the cast gating — proves data-driven.

## Patterns to Follow

- **Typed extraction from reflected ability components**: mirror `extract_phases` (`ability/loader.rs:9-21`) for a new `extract_conditional_effects(asset: &AbilityAsset) -> Option<&ConditionalEffects>`.
- **Ability effect dispatch**: the four phase-keyed systems in `ability/effects.rs`. Matched conditional effects are appended to the entity's `OnTickEffects` at tick 0 during spawn, then dispatched by the unchanged `apply_on_tick_effects` (`effects.rs:28-137`).
- **Reflected-component registration for ability fields**: every new type (`ConditionalEffects`, `ConditionalEffect`, `Condition`) gets `#[derive(Reflect)]`, container types add `#[derive(Component)]` + `#[reflect(Component)]`, and all are registered via `app.register_type::<T>()` alongside existing ability types in `ability/plugin.rs`.
- **Input → slot mapping**: the `ABILITY_ACTIONS` const array + `ability_action_to_slot` (`activation.rs:17-32`). Extend both to 5 entries, with slot 4 = `PlayerActions::Jump`.
- **Sparse-set marker convention** (new — no prior precedent in workspace): `IsGrounded` uses `#[component(storage = "SparseSet")]` because it's toggled frequently as characters jump/land; avoids per-toggle archetype moves. Unit struct, no payload, added/removed by `detect_grounded`.
- **Anti-pattern to replace**: existing `velocity.0 += world_force` application of `AbilityEffect::ApplyForce` at `hit_detection/effects.rs:108-129` (mass-independent, non-wake-aware). All call sites migrate to `forces.apply_linear_impulse`.

## Design Decisions

1. **Jump slots into a 5-entry `AbilitySlots`**: `AbilitySlots([Option<AbilityId>; 5])`, `AbilityCooldowns::last_used: [Option<Tick>; 5]`, `ABILITY_ACTIONS` gains `PlayerActions::Jump` at index 4. Keeps the fixed-size array ergonomics (simple serialization, index-based cooldowns, copy semantics) and preserves the `Jump` semantic input name.

2. **`ConditionalEffects(Vec<ConditionalEffect>)` reflected component**: optional; abilities that don't branch omit it entirely.
   ```rust
   pub enum Condition { Grounded, Airborne }
   pub struct ConditionalEffect { pub condition: Condition, pub effect: AbilityEffect }
   pub struct ConditionalEffects(pub Vec<ConditionalEffect>);
   ```
   **Activation semantics** (evaluated in `ability_activation` after cooldown check, before spawn):
   - If the asset has no `ConditionalEffects` → skip; normal trigger-based dispatch drives the ability.
   - If present: evaluate each entry's condition against the caster (`Grounded` = `Has<IsGrounded>`; `Airborne` = inverse). Collect all entries whose condition matches.
   - If the matched set is empty → `continue` (no spawn, no cooldown consumption, matches existing cooldown-miss behavior).
   - If the matched set is non-empty → proceed with normal spawn. During `apply_ability_archetype`, append the matched effects to the entity's `OnTickEffects` as `(tick: 0, effect)` entries — after any effects authored in `OnTickEffects` directly (so authored-tick-0 effects fire before conditional effects in list order). `apply_on_tick_effects` dispatches them unchanged.
   - **All matching entries fire**. Overlapping conditions fire cumulatively; non-overlapping sets (like `Grounded`/`Airborne`) behave like a switch.

3. **`IsGrounded` is a sparse-set unit component, local-only**: `#[derive(Component)] #[component(storage = "SparseSet")] pub struct IsGrounded;`. Derived each tick from deterministic state (`Position` + world colliders, both replicated), so client/server agree without network registration. Rollback re-simulation re-runs `detect_grounded` naturally. No `register_component` / `add_prediction`.

4. **`detect_grounded` mirrors the existing ray cast**: same origin (`position.0`), `Dir3::NEG_Y`, `max_toi = 4.0`, `solid = false`, `SpatialQueryFilter::from_excluded_entities([entity])`, same `MapInstanceId` predicate. Scheduled `.before(handle_character_movement).before(ability_activation)` in `FixedUpdate`. Preserves jump detection exactly. Not a `ShapeCaster` (would change detection semantics).

5. **Jump ability content** (`assets/abilities/jump.ability.ron`):
   ```ron
   {
       "protocol::ability::AbilityPhases": (startup: 0, active: 1, recovery: 0, cooldown: 0),
       "protocol::ability::ConditionalEffects": ([
           (condition: Grounded, effect: ApplyForce(force: (0, 2000, 0), frame: World, target: Caster)),
       ]),
   }
   ```
   Airborne press → matched set empty → activation refused. Grounded press → impulse queued as `OnTickEffects(tick:0)` → fires on first Active tick via `apply_linear_impulse`, preserving existing mechanics. Registered in `DefaultAbilitySlots` slot 4 so every character gets jump by default.

6. **Convert `ApplyForce` to `forces.apply_linear_impulse`** (both hit-detection and the new on-tick dispatch for jump): `target_query` changes from `Option<&mut LinearVelocity>` to avian's `Forces` `QueryData`, requiring targets to be `RigidBody`. Caster and victim characters are already rigid bodies; no other current `ApplyForce` targets exist. Rescale the melee RON `(0.0, 0.9, 2.85)` by the character's computed mass (read `ComputedMass` at runtime or derive from capsule density × volume) and commit the rescaled value in the melee `.ability.ron`. Add a `warn!` on `target_query.get_mut` miss so future non-body targets don't silently disappear.

## What We're NOT Doing

- **Not introducing air control / air-movement restrictions**: `apply_movement`'s horizontal force path is untouched. `IsGrounded` is consumed only by `ConditionalEffects` evaluation.
- **Not generalizing ability input bindings to a HashMap or adding per-character rebinding**.
- **Not emitting activation-failure events**: matches existing cooldown-miss `continue` convention.
- **Not registering `IsGrounded` for networking or rollback**: derivation is deterministic.
- **Not adding effect-time conditions** (e.g. "only knockback if victim unshielded"). `ConditionalEffects` evaluates once at activation; per-effect-time gating is a separate future concern.
- **Not adding more `Condition` variants yet** (e.g. `MinHealth`, `HasBuff`). Only `Grounded` / `Airborne` ship.
- **Not adding `SystemSet` abstractions**: game code uses `.chain()` and `.before(system)` exclusively; match that.
- **Not changing the ray-cast parameters** (origin, distance, solid flag, map filter).
- **Not changing `AbilityEffect::SetVelocity`**: different semantics (absolute set, not impulse).
- **Not migrating other `AbilityEffect` variants** to `Forces`-based queries; only `ApplyForce` changes.

## Open Risks

- **Melee knockback rescaling may not exactly match previous feel**: `ComputedMass` depends on capsule geometry × density. Implementation must read the actual mass at runtime (or compute it deterministically) and rescale. `LockedAxes::ROTATION_LOCKED` zeros angular but not translational effective-inverse-mass, so the translational scaling is straightforward. A follow-up playtest may be needed.
- **`Forces` `QueryData` is not `Option`-wrappable**: any `ApplyForce` target that isn't a full rigid body will be silently skipped by the query. Current call sites target characters only, so this is fine today. The added `warn!` on miss protects future content.
- **`IsGrounded` freshness at ability activation**: `detect_grounded` must run before `ability_activation`. Both live in `FixedUpdate` without a shared set; enforce with `.before(ability_activation)`. Comment the `.before(...)` site so future reorders don't silently break the gate.
- **Appending to a reflected `OnTickEffects` component at spawn time**: `apply_ability_archetype` currently blindly inserts each reflected component. To append conditional matches into `OnTickEffects`, the activation path must either (a) mutate a cloned `OnTickEffects` reflected value before the reflect-insert queue runs, or (b) insert a second `OnTickEffects` and have the dispatch loop merge (Bevy will just overwrite, so this doesn't work — must be (a)). Implementation chooses (a): clone `OnTickEffects` from the asset if present (empty otherwise), push matched `(0, effect)` entries onto it, then apply the archetype with the modified list in place of the asset's entry. This is a small surgical change to `apply_ability_archetype` signature (or a pre-step before it).
- **Lightyear `just_pressed` on rollback re-simulation** (research Open Area 1): jump's correctness depends on the same `just_pressed` edge semantics all other abilities rely on. No jump-specific risk.
- **Character-spawn bootstrapping**: at spawn, a character has no `IsGrounded` marker until the first `detect_grounded` tick. If a client mashes Space on the same tick as character spawn, the first activation could be rejected as airborne. Acceptable; the next tick's detection will resolve it.
- **Slot 4 has no default key-rebinding UI**: there's no rebinding UI today. If added later, the 5th slot needs special treatment (its action is fixed by index). Out of scope.
