# Structure Outline

## Approach

Migrate `ApplyForce` to `forces.apply_linear_impulse` first (orthogonal correctness fix), then build the grounded-marker foundation, expand the slot array, introduce `ConditionalEffects` as a reflected gate evaluated during `ability_activation`, and finally author `jump.ability.ron` to replace inline jump in `apply_movement`. Each phase is independently verifiable; the jump removal is the last step so the game stays playable until then.

---

## Phase 1: Migrate `AbilityEffect::ApplyForce` to `apply_linear_impulse`

Switches the only `ApplyForce` consumer (melee on-hit) onto avian's `Forces` API so impulse is mass-scaled, wake-aware, and `LockedAxes`-respecting. Rescale the melee RON to preserve current knockback feel.

**Files**: `crates/protocol/src/hit_detection/effects.rs`, `assets/abilities/punch1.ability.ron` (and any other ability RON using `ApplyForce`).

**Key changes**:
- `target_query: Query<(&Position, Forces, &mut Health, Option<&Invulnerable>)>` — replaces `Option<&mut LinearVelocity>` with avian's `Forces` `QueryData`.
- `ApplyForce` arm: `forces.apply_linear_impulse(world_force)` instead of `velocity.0 += world_force`.
- `warn!("ApplyForce target {:?} not a rigid body", entity)` on `target_query.get_mut` miss.
- Melee RON `force: (0.0, 0.9, 2.85)` rescaled by character `ComputedMass` (read at runtime once, hardcode the scaled value).

**Verify**: `cargo check-all` and `cargo test-all` pass. Manual: `cargo client` + `cargo server`, melee a target, knockback feels equivalent to pre-change (record/compare distance traveled).

---

## Phase 2: `IsGrounded` sparse-set marker + `detect_grounded` system

Lifts the existing ground ray cast out of `apply_movement` into a dedicated detector that maintains an `IsGrounded` marker on character entities. No consumer yet — pure foundation.

**Files**: `crates/protocol/src/character/types.rs` (new component), `crates/protocol/src/character/movement.rs` (new system), `crates/protocol/src/character/plugin.rs` (or wherever character systems are added — schedule registration), `crates/protocol/src/lib.rs` (no `register_component` — local-only).

**Key changes**:
- `#[derive(Component)] #[component(storage = "SparseSet")] pub struct IsGrounded;` — local-only marker, not registered for replication or prediction.
- `fn detect_grounded(commands: Commands, spatial_query: SpatialQuery, characters: Query<(Entity, &Position, Option<&MapInstanceId>, Has<IsGrounded>), With<CharacterMarker>>, map_ids: Query<&MapInstanceId>)` — same ray params as current jump check (`Dir3::NEG_Y`, max 4.0, `solid=false`, map predicate, excluded-self filter); inserts/removes `IsGrounded` to match hit state.
- Scheduling: `.before(handle_character_movement).before(ability_activation)` in `FixedUpdate` on both client and server.
- Comment at the `.before(...)` site explaining ordering invariant.

**Verify**: `cargo check-all` passes. Manual: add a temporary `trace!` for grounded transitions, run client+server, observe the marker toggling on jump arc and landing.

---

## Phase 3: Expand `AbilitySlots` to 5 entries + bind `Jump` to slot 4

Structural prep. Jump still works via the inline path; this phase only widens the array, the cooldown table, and `ABILITY_ACTIONS` so a future ability can occupy slot 4.

**Files**: `crates/protocol/src/ability/types.rs`, `crates/protocol/src/ability/activation.rs`, any default-slot/loadout RON or constructor (`DefaultAbilitySlots`), `crates/protocol/src/lib.rs` (re-check `AbilitySlots` registration — array length change must not break `Serialize`).

**Key changes**:
- `pub struct AbilitySlots(pub [Option<AbilityId>; 5])` (was 4).
- `pub struct AbilityCooldowns { pub last_used: [Option<Tick>; 5] }`.
- `const ABILITY_ACTIONS: [PlayerActions; 5] = [..Ability1..=Ability4, PlayerActions::Jump]` — `Jump` at index 4.
- `ability_action_to_slot` unchanged in body (uses `iter().position`).

**Verify**: `cargo check-all` and `cargo test-all` pass. Manual: existing Ability1–4 behavior unchanged (slot 4 is `None`, jump still inlined).

---

## Phase 4: `ConditionalEffects` component + activation-time evaluation

Adds the data-driven cast gate. Evaluated after the cooldown check; matched effects are appended to a cloned `OnTickEffects` at tick 0 before the archetype is applied.

**Files**: `crates/protocol/src/ability/types.rs` (new types), `crates/protocol/src/ability/loader.rs` (extractor + archetype hook), `crates/protocol/src/ability/activation.rs` (evaluation), `crates/protocol/src/ability/plugin.rs` (`register_type` calls).

**Key changes**:
- New reflected types:
  ```rust
  pub enum Condition { Grounded, Airborne }
  pub struct ConditionalEffect { pub condition: Condition, pub effect: AbilityEffect }
  #[derive(Component, Reflect)] #[reflect(Component)]
  pub struct ConditionalEffects(pub Vec<ConditionalEffect>);
  ```
- `extract_conditional_effects(asset: &AbilityAsset) -> Option<&ConditionalEffects>` — mirrors `extract_phases`.
- `apply_ability_archetype` signature gains a way to override one component on insert, OR a sibling helper `apply_ability_archetype_with_extra_on_tick(commands, entity_id, asset, registry, extra: Vec<(u16, AbilityEffect)>)` that clones the asset's `OnTickEffects` (or constructs an empty one), appends `extra` as `(0, effect)` entries, and substitutes during the reflect-insert queue. Choose whichever is cleaner; design.md "Open Risks" mandates path (a) — mutate the cloned reflected `OnTickEffects` value in place.
- In `ability_activation` after the cooldown check:
  - `let matched: Vec<AbilityEffect> = ConditionalEffects::evaluate(asset, caster_has_grounded)` (collect all matching entries' effects).
  - If `extract_conditional_effects(asset).is_some() && matched.is_empty()` → `continue` (no spawn, no cooldown consumption); add `trace!` explaining the gate.
  - Otherwise pass `matched` into the archetype insertion as extra tick-0 entries.
- `app.register_type::<Condition>().register_type::<ConditionalEffect>().register_type::<ConditionalEffects>()` in `ability/plugin.rs`.

**Verify**: `cargo check-all` and `cargo test-all` pass. Unit test: build a synthetic `AbilityAsset` with `ConditionalEffects([Grounded → ApplyForce])`, evaluate against an entity with and without `IsGrounded`, assert spawn vs no-spawn. Manual deferred to Phase 5.

---

## Phase 5: Jump as a data-defined ability + remove inline jump

The end-to-end slice. Authors `jump.ability.ron` with `ConditionalEffects(Grounded → ApplyForce(0,2000,0, World, Caster))`, registers it in `DefaultAbilitySlots` slot 4, and deletes the inline `just_pressed(Jump)` block from `apply_movement`. Also routes the on-tick `ApplyForce` (caster-target) through the impulse path established in Phase 1.

**Files**: `assets/abilities/jump.ability.ron` (new), `crates/protocol/src/character/movement.rs` (deletion), `crates/protocol/src/ability/effects.rs` (`apply_on_tick_effects` `ApplyForce` arm — needs same `Forces`-based dispatch as Phase 1; if Phase 1 extracted a shared helper, reuse it), `assets/ability_slots/*.ability_slots.ron` or wherever default loadouts live.

**Key changes**:
- New asset:
  ```ron
  {
      "protocol::ability::AbilityPhases": (startup: 0, active: 1, recovery: 0, cooldown: 0),
      "protocol::ability::ConditionalEffects": ([
          (condition: Grounded, effect: ApplyForce(force: (0, 2000, 0), frame: World, target: Caster)),
      ]),
  }
  ```
- `apply_movement` loses lines 28–47 (the `just_pressed(Jump)` block). Function signature loses `spatial_query`, `position`, `player_map_id`, `map_ids` if no other consumer remains in the body — verify before pruning.
- `apply_on_tick_effects` `ApplyForce { target: Caster }` arm uses `forces.apply_linear_impulse` against the caster's `Forces` — reusing the helper from Phase 1.
- `DefaultAbilitySlots` slot 4 = `Some(AbilityId("jump"))`.

**Verify**: `cargo check-all` and `cargo test-all` pass. Manual end-to-end:
- Hold/tap Space on ground → repeated jumps with same arc and cadence as before. Compare side-by-side against pre-Phase-5 build.
- Press Space mid-air → no impulse, no `ActiveAbility` spawn (inspect via `bevy-inspector` or a `trace!`), no cooldown advance.
- Edit `jump.ability.ron` to swap `Grounded` → `Airborne` (or remove `ConditionalEffects` entirely) and confirm cast gating changes accordingly — proves data-driven.
- Re-run melee playtest from Phase 1 to confirm no regression.

---

## Testing Checkpoints

- **After Phase 1**: All abilities compile and run; melee knockback visually matches previous feel after RON rescaling. Jump still works via inline path.
- **After Phase 2**: `IsGrounded` toggles on/off correctly during normal jumping; nothing else observably changes.
- **After Phase 3**: Existing 4 ability slots unchanged in behavior; slot 4 reads `None`; cooldown serialization round-trips with the wider array.
- **After Phase 4**: Unit test confirms `ConditionalEffects` evaluation logic; no production ability uses it yet so no behavior change in-game.
- **After Phase 5**: Jump is fully data-driven; airborne presses are refused; switching the RON condition flips behavior without code changes; melee unchanged.
