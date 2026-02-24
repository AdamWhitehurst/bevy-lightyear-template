# Project Memory

## Key Architecture
- **Ability system**: `ActiveAbility` is a prespawned/predicted entity (not a component on characters). Spawned with `PreSpawned` salt, `prediction_despawn()` on completion.
- **Effect triggers**: `EffectTrigger::OnCast(effect)` / `EffectTrigger::WhileActive(effect)` in `AbilityDef.effects: Vec<EffectTrigger>`
- **Movement**: Characters can move during abilities. Dash overrides via `SetVelocity` WhileActive effect.
- **Hit detection**: `process_melee_hits` queries `ActiveAbility` entities (not characters) with `MeleeHitboxActive`

## Lightyear Patterns
- `NetworkTimeline` is a **trait** (not just a type) — must stay in imports for `.tick()` method
- `PredictionDespawnCommandsExt` from `lightyear::prelude` provides `.prediction_despawn()` on `EntityCommands`
- `MapEntities` trait from `bevy::ecs::entity` — chain `.add_map_entities()` on component registration
- Server detection pattern: `Query<&ControlledBy>` — only exists on server entities
- `PlayerId(PeerId)` on character entities for prespawn salt computation

## Test Notes
- Tests using `ability_activation` need `#[ignore]` because `PreSpawned::on_add` hook requires lightyear Server/Client
- Phase transition tests work by directly spawning `ActiveAbility` entities

## Cargo Aliases
- `cargo check-all` = `cargo check --workspace`
- `cargo test-all` = `cargo make test-all` (but `cargo test --workspace --exclude web` works directly)
