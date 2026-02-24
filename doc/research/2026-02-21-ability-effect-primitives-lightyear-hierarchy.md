---
date: 2026-02-21T17:12:43-0800
researcher: Claude Opus 4.6
git_commit: 09cb455d5b5392488798e8764aee96e9c1c9ccdc
branch: master
repository: bevy-lightyear-template
topic: "How to implement ability-effect-primitives with lightyear replication and bevy hierarchy"
tags: [research, codebase, ability, effects, lightyear, replication, prediction, rollback, prespawning, hierarchy, parent-child]
status: complete
last_updated: 2026-02-21
last_updated_by: Claude Opus 4.6
last_updated_note: "Resolved open questions 1-6; added prespawn salt strategy, component filtering findings, ReplicateLike behavior, ChildOf/FixedUpdate limitation"
supersedes: doc/research/2026-02-20-ability-effect-primitives-implementation-analysis.md
---

# Research: Ability Effect Primitives — Lightyear Replication & Bevy Hierarchy

**Date**: 2026-02-21T17:12:43-0800
**Git Commit**: 09cb455d5b5392488798e8764aee96e9c1c9ccdc
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to implement the `doc/design/2026-02-13-ability-effect-primitives.md` design while correctly accounting for: (1) lightyear replication, prediction, confirmation, prespawning, and rollback; and (2) bevy parent-child entity hierarchies (`ChildOf`, `with_children()`).

The prior research (`doc/research/2026-02-20-ability-effect-primitives-implementation-analysis.md`) omitted these concerns, which would result in double-spawning, rollback failures, and no entity-relationship tracking.

## Summary

The design doc's central change — `ActiveAbility` moving from a component on the character to a spawned entity — requires careful integration with lightyear's prediction model. Lightyear v0.25.5 provides three entity spawning strategies (normal replication, prespawning, deterministic prediction) and automatic parent-child replication via `HierarchySendPlugin`. The existing projectile spawn pattern (`PreSpawned` + server-only `Replicate`/`PredictionTarget`/`ControlledBy`) is the correct model for spawning `ActiveAbility` entities and their child hitbox/AoE entities. Bevy's `ChildOf` relationship, replicated by lightyear, enables hierarchical entity management for ability → hitbox → bullet chains.

---

## Detailed Findings

### 1. Lightyear's Prediction Model

Lightyear maintains **two representations** of predicted entities on the client:

- `C` — the predicted component value (written by local simulation)
- `Confirmed<C>` — the server-authoritative value (written by replication)

Components registered with `.add_prediction()` participate in rollback. On each server state update, lightyear compares `Confirmed<C>` against the `PredictionHistory<C>` at the confirmed tick. If the `should_rollback` function returns `true` (or no custom function is registered, in which case it uses `PartialEq`), lightyear restores `Confirmed<C>` values and re-simulates all `FixedUpdate` systems from the confirmed tick to the current predicted tick.

**Registered predicted components** ([lib.rs:160-196](crates/protocol/src/lib.rs#L160)):

| Component | Rollback threshold | Correction |
|---|---|---|
| `ColorComponent`, `CharacterMarker`, `DummyTarget`, `Health`, `Invulnerable` | PartialEq (default) | None |
| `ActiveAbility`, `AbilityCooldowns` | PartialEq (default) | None |
| `LinearVelocity`, `AngularVelocity` | 0.01 distance | None |
| `Position`, `Rotation` | 0.01 distance/angle | Linear correction + interpolation |

**Non-predicted (replicate-only)**: `ChunkRenderTarget<MapWorld>`, `Name`, `AbilitySlots`, `AbilityProjectileSpawn`.

### 2. Current Prespawning Pattern (Projectiles)

The existing projectile system is a two-phase prespawn pattern in shared `FixedUpdate`:

**Phase 1** — `ability_projectile_spawn` ([ability.rs:473-521](crates/protocol/src/ability.rs#L473)):
```rust
// Both client and server execute this (shared FixedUpdate)
let mut cmd = commands.spawn((
    spawn_info,                                          // AbilityProjectileSpawn data
    PreSpawned::default_with_salt(active.step as u64),   // deterministic hash
    Name::new("AbilityProjectileSpawn"),
));

// Server-only: ControlledBy only exists on server entities
if let Ok(controlled_by) = server_query.get(entity) {
    cmd.insert((
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
        *controlled_by,
    ));
}
```

**Phase 2** — `handle_ability_projectile_spawn` ([ability.rs:525-548](crates/protocol/src/ability.rs#L525)):
Spawns child bullet entity with `DisableRollback`, `AbilityBulletOf(spawn_entity)` relationship, physics, and collision components.

**How prespawning works internally** ([git/lightyear/lightyear_replication/src/prespawn.rs](git/lightyear/lightyear_replication/src/prespawn.rs)):

1. `PreSpawned::on_add` computes a hash from: current tick + sorted component `NetId`s in the archetype + user salt. Uses `seahash::SeaHasher` for cross-process determinism.
2. An observer registers the hash in `PreSpawnedReceiver.prespawn_hash_to_entities`.
3. When the server's replication arrives, the client's `apply_actions_message` calls `prespawned_receiver.matches(hash, remote_entity)`. If matched: the remote entity maps to the existing local entity, `PreSpawned` is removed, `Predicted` is inserted.
4. Unmatched prespawned entities are cleaned up after 50 ticks.
5. **On rollback**: all prespawned entities spawned after the rollback tick are despawned, then re-created during rollback re-simulation.

**Salt requirement**: The salt must disambiguate entities spawned on the same tick with the same archetype. Currently `active.step as u64` is used for projectiles (sufficient because only one projectile per combo step). For `ActiveAbility` entities, a new deterministic salt is needed.

### 3. Server-Only Code Branching Pattern

The codebase uses `Query<&ControlledBy>` as a **server detection mechanism** ([ability.rs:486,510](crates/protocol/src/ability.rs#L486)). `ControlledBy` is a lightyear component only present on server-side entities. On the client, this query returns `Err`, so the `Replicate`/`PredictionTarget`/`ControlledBy` insertion block is skipped.

This is the established pattern for shared systems that need server-only behavior. The new `ActiveAbility` entity spawn must follow this same pattern.

### 4. Lightyear Parent-Child Replication

Lightyear v0.25.5 replicates entity hierarchies via `HierarchySendPlugin` ([git/lightyear/lightyear_replication/src/hierarchy.rs](git/lightyear/lightyear_replication/src/hierarchy.rs)):

**`ReplicateLike`**: A relationship component auto-inserted on child entities. When a parent has `Replicate`, the `propagate_through_hierarchy` system walks descendants and inserts `ReplicateLike { root: parent }` on each child that lacks its own `Replicate` or `DisableReplicateHierarchy`.

**`ChildOf` is replicated**: `ChildOf` is registered as a replicated component by `HierarchySendPlugin`. Entity references in `ChildOf` are mapped via `RemoteEntityMap` on the receiving side, so the hierarchy is preserved on the client.

**`DisableReplicateHierarchy`**: Marker component that stops replication propagation through a subtree.

**Implications**: If an `ActiveAbility` entity has `Replicate` and its hitbox/AoE entities are spawned as children (via `ChildOf`), they automatically inherit replication settings without needing their own `Replicate`. The hierarchy is maintained on the client. However, see section 4.1 — `ChildOf` is not suitable for `FixedUpdate` game logic due to stale `GlobalTransform`.

**Test confirmation**: The lightyear test `test_spawn_predicted_with_hierarchy` ([git/lightyear/lightyear_replication/tests/spawn.rs:28-70](git/lightyear/lightyear_replication/tests/spawn.rs#L28)) spawns a parent+child with `Replicate`+`PredictionTarget`, and verifies the predicted child has `ChildOf` pointing to the predicted parent.

### 4.1. ChildOf Transform Propagation Does Not Run in FixedUpdate

Bevy's `TransformPlugin` registers `propagate_parent_transforms` and `sync_simple_transforms` only in `PostStartup` and `PostUpdate` ([git/bevy/crates/bevy_transform/src/plugins.rs:17-42](git/bevy/crates/bevy_transform/src/plugins.rs#L17)). These systems do NOT run in `FixedUpdate`.

Any `GlobalTransform` values read during `FixedUpdate` are stale from the previous frame's `PostUpdate`. For entities using `ChildOf`, local `Transform` changes made in `FixedUpdate` won't propagate to `GlobalTransform` until after all `FixedUpdate` ticks complete.

Since all ability and hit detection systems run in `FixedUpdate` using avian3d `Position`/`Rotation` (not `Transform`/`GlobalTransform`), `ChildOf` transform propagation provides no benefit for game-logic entities. This is why custom relationships are preferred for hitbox/AoE entities.

### 5. Current Bevy Hierarchy Usage in Codebase

Parent-child (`ChildOf`/`with_children()`) is used only in render and UI crates:

- **Health bars** ([render/src/health_bar.rs:44-64](crates/render/src/health_bar.rs#L44)): Character → HealthBarRoot(Billboard) → Background/Foreground meshes. Traverses hierarchy via `ChildOf` for billboard rotation and `Children` for health updates.
- **UI screens** ([ui/src/lib.rs](crates/ui/src/lib.rs)): Standard Node → Button → Text hierarchies with `DespawnOnExit` for state-scoped cleanup.

**Game entities are flat**: Characters, projectiles, and spawn-info entities are root-level with no `ChildOf`. Cross-entity references use plain `Entity` fields (`ProjectileOwner(Entity)`, `AbilityProjectileSpawn.shooter: Entity`) or Bevy relationships (`AbilityBulletOf`/`AbilityBullets` with `linked_spawn`).

### 6. Bevy Relationships vs ChildOf

The codebase uses two entity relationship mechanisms:

**Bevy `ChildOf`/`Children`** (built-in hierarchy): Propagates `Transform` → `GlobalTransform`. Used for entities that need spatial parenting (health bars above characters). `DespawnOnExit` and default Bevy behavior cascade despawn to children.

**Bevy relationships** (`#[relationship]`/`#[relationship_target]`): Custom typed relationships without transform propagation. Current usage: `AbilityBulletOf`/`AbilityBullets` with `linked_spawn` — bullet lifecycle is tied to spawn entity, but no transform inheritance.

**Key difference for ability entities**: Hitbox/AoE entities need spatial positioning relative to the caster (e.g., melee hitbox in front of caster). Using `ChildOf` would give automatic transform propagation. Using a custom relationship (`ActiveAbilityOf`/`ActiveAbilities`) would require manual position management but avoid unintended transform coupling.

### 7. DisableRollback and Predicted Entity Despawning

**`DisableRollback`** ([git/lightyear/lightyear_prediction/src/rollback.rs:236](git/lightyear/lightyear_prediction/src/rollback.rs#L236)): Marker component that excludes an entity from rollback entirely. During rollback, `DisabledDuringRollback` is temporarily inserted, hiding the entity from all queries. Current usage: bullet entities ([ability.rs:545](crates/protocol/src/ability.rs#L545)).

**`DeterministicPredicted`** ([git/lightyear/lightyear_prediction/src/rollback.rs:174-203](git/lightyear/lightyear_prediction/src/rollback.rs#L174)): For entities predicted without server confirmation. Two modes:
- `skip_despawn: false` (default): Entity is despawned during rollback and re-created by re-simulation.
- `skip_despawn: true`: Entity persists through rollback; rollback is disabled for first `enable_rollback_after` ticks (default 20).

**`prediction_despawn()`** ([git/lightyear/lightyear_prediction/src/despawn.rs:69](git/lightyear/lightyear_prediction/src/despawn.rs#L69)): Instead of `try_despawn()`, inserts `PredictionDisable` marker. Entity stays alive so rollback can restore it. If the confirmed entity is also despawned, the predicted entity is truly removed.

**Current code uses `try_despawn()`**: Both `despawn_ability_projectile_spawn` ([ability.rs:559](crates/protocol/src/ability.rs#L559)) and `ability_bullet_lifetime` ([ability.rs:576](crates/protocol/src/ability.rs#L576)) use `try_despawn()`. Since bullets have `DisableRollback`, this works but is not the general pattern for predicted entities.

### 8. Entity Mapping (MapEntities)

Replicated components containing `Entity` references must derive `MapEntities` so lightyear can remap entity IDs between server and client. The existing `AbilityBulletOf` uses `#[entities]` attribute on the `Entity` field, which handles mapping.

**For the new `ActiveAbility` struct**:
```rust
pub struct ActiveAbility {
    pub def_id: AbilityId,
    pub caster: Entity,           // needs MapEntities
    pub original_caster: Entity,  // needs MapEntities
    pub target: Entity,           // needs MapEntities
    pub phase: AbilityPhase,
    pub phase_start_tick: Tick,
    pub depth: u8,
}
```

The `caster`, `original_caster`, and `target` fields reference other entities and must be correctly mapped. If `ActiveAbility` is registered with lightyear (`.add_prediction()`), it needs `#[derive(MapEntities)]` or manual `MapEntities` implementation.

### 9. ReplicationGroup

**Definition** ([git/lightyear/lightyear_replication/src/send/components.rs:165](git/lightyear/lightyear_replication/src/send/components.rs#L165)): Entities in the same `ReplicationGroup` are sent together atomically — guaranteed to be updated at the same time on the remote.

**Current usage**: No explicit `ReplicationGroup` in the codebase. Each entity uses the default (entity-based group ID).

**PREDICTION_GROUP**: Lightyear defines `PREDICTION_GROUP = ReplicationGroup::new_id(1)` as a constant shared group, auto-inserted when `PredictionTarget` is added.

**For ability entities**: If an `ActiveAbility` entity and its child hitbox entities need atomic replication, they should share a `ReplicationGroup`. Lightyear's hierarchy replication (`ReplicateLike`) may handle this automatically when children inherit from the parent.

---

## Architecture: Entity Spawn Strategies for New Design

### ActiveAbility Entity Spawning

The design requires `ActiveAbility` to be a **spawned entity** instead of a component on the character. This entity must:

1. Exist on both client (predicted) and server (authoritative)
2. Be matched via prespawning
3. Participate in rollback (its phase progression affects downstream systems)
4. Reference the caster/target entities (requires `MapEntities`)

**Recommended pattern** (follows existing projectile pattern):

```rust
// In shared FixedUpdate (ability_activation or a new system)
let mut cmd = commands.spawn((
    ActiveAbility {
        def_id, caster: entity, original_caster: entity,
        target: entity, phase: AbilityPhase::Startup,
        phase_start_tick: tick, depth: 0,
    },
    PreSpawned::default_with_salt(salt),  // deterministic salt
    Name::new("ActiveAbility"),
));

// Server-only
if let Ok(controlled_by) = server_query.get(entity) {
    cmd.insert((
        Replicate::to_clients(NetworkTarget::All),
        PredictionTarget::to_clients(NetworkTarget::All),
        *controlled_by,
    ));
}
```

**Salt computation**: Must be deterministic and unique per ability activation on the same tick. Options:
- `ability_id` hash + slot index: unique per slot per tick
- Combine `ability_slot as u64` with a counter: handles multiple activations per tick

### Hitbox/AoE Child Entity Spawning

Melee hitboxes and AoE entities are ephemeral (exist during Active phase). Use a **custom relationship** (not `ChildOf`), following the existing `AbilityBulletOf`/`AbilityBullets` pattern:

```rust
#[derive(Component)]
#[relationship(relationship_target = ActiveAbilityHitboxes)]
struct HitboxOf(#[entities] Entity);

#[derive(Component, Default)]
#[relationship_target(relationship = HitboxOf, linked_spawn)]
struct ActiveAbilityHitboxes(Vec<Entity>);
```

**Rationale**: Bevy's `propagate_parent_transforms` runs only in `PostUpdate` ([plugins.rs:30-40](git/bevy/crates/bevy_transform/src/plugins.rs#L30)), not `FixedUpdate`. All ability/hitbox systems run in `FixedUpdate` using `Position`/`Rotation` (avian3d), so `ChildOf` transform propagation would be stale and unusable. Custom relationship with `linked_spawn` ties lifecycle to the parent `ActiveAbility` entity without transform coupling.

### Marker Components (OnCastEffects, WhileActiveEffects, etc.)

These are **ephemeral dispatch markers** inserted and consumed within single ticks/phases. They should NOT be registered with lightyear:

- They are derived from `ActiveAbility` state (which IS replicated/predicted)
- `dispatch_effect_markers` recomputes them each tick from the ability's phase and trigger list
- Registering them would add unnecessary replication overhead and rollback complexity
- They contain `Vec<AbilityEffect>` which may reference entities — but these are consumed locally

### Buff/Shield/Grab Components

These ARE persistent state that affects gameplay:

| Component | Registration | Rollback | Notes |
|---|---|---|---|
| `ActiveBuff { stat, multiplier, expires_tick }` | `.add_prediction()` | Yes | Affects stat queries during resimulation |
| `ActiveShield { remaining }` | `.add_prediction()` | Yes | Intercepts damage during resimulation |
| `GrabbedBy(Entity)` | `.add_prediction()` + `MapEntities` | Yes | Entity ref needs remapping |
| `Grabbing(Entity)` | `.add_prediction()` + `MapEntities` | Yes | Entity ref needs remapping |

### ActiveAbility Despawning

When an `ActiveAbility` finishes (exits Recovery phase):

- **ActiveAbility entity**: Use `prediction_despawn()` ([git/lightyear/lightyear_prediction/src/despawn.rs:69](git/lightyear/lightyear_prediction/src/despawn.rs#L69)) instead of `try_despawn()`. This inserts `PredictionDisable` so rollback can restore the entity if needed.
- **Child hitbox/AoE entities**: `linked_spawn` on the custom relationship automatically despawns children when the parent is despawned.
- **Bullet entities** (with `DisableRollback`): Current `try_despawn()` remains acceptable.

---

## Implementation Considerations

### Rollback Safety

1. **ActiveAbility must be predicted**: It drives all ability logic. Without prediction, ability phases would lag behind by RTT. Already registered with `.add_prediction()` — this registration moves from the character component to the spawned entity component.

2. **Prespawned entities are despawned on rollback**: During rollback, all prespawned entities spawned after the rollback tick are despawned and re-created by re-simulation. This means `ability_activation` (or its replacement) must be deterministic — same inputs at same tick must produce same spawns.

3. **Marker components re-derived on rollback**: Since `dispatch_effect_markers` runs in `FixedUpdate` and recomputes markers from `ActiveAbility` state, rollback re-simulation naturally recomputes correct markers. No special rollback handling needed for markers.

4. **Entity references survive rollback**: `caster`, `original_caster`, `target` fields on `ActiveAbility` reference entities that persist through rollback (characters are never despawned). However, if an `ActiveAbility` entity is despawned and re-created during rollback, these references must be re-established — the prespawn hash ensures the same entity is matched.

### Entity Mapping

Components on replicated entities that contain `Entity` fields must implement `MapEntities`:
- `ActiveAbility.caster`, `.original_caster`, `.target`
- `GrabbedBy(Entity)`, `Grabbing(Entity)`
- `OnHitEffects.caster`, `.original_caster`

Without `MapEntities`, entity IDs from the server won't map to the correct client entities.

### Double-Spawn Prevention

The prespawning model prevents double-spawning:
1. Client spawns `ActiveAbility` with `PreSpawned::default_with_salt(salt)` in shared `FixedUpdate`
2. Server spawns the same entity with the same `PreSpawned` salt, plus `Replicate`/`PredictionTarget`
3. When server replication arrives, lightyear matches by hash and maps the server entity to the existing client entity
4. If a rollback occurs before matching, the client entity is despawned and re-created during resimulation — still with the same hash, so it matches when the server replication arrives

**Without prespawning**: The server would replicate an `ActiveAbility` entity, the client would create a `Predicted` copy, and the existing client-predicted `ActiveAbility` would be a duplicate. This is the "double-spawning" problem.

### Schedule Ordering

The new dispatch systems must maintain ordering guarantees:

```
FixedUpdate:
  ability_activation (spawn ActiveAbility entities)
  → update_active_abilities (advance phases)
  → dispatch_effect_markers (partition triggers into marker components)
  → apply_on_cast_effects (process OnCast, spawn hitboxes/projectiles/AoE)
  → apply_while_active_effects (process SetVelocity)
  → apply_on_input_effects (process OnInput, requires caster ActionState)
  → apply_on_end_effects (process OnEnd)

  // After dispatch:
  → ensure_melee_hit_targets (spatial query for melee hitboxes)
  → process_melee_hits (fire OnHitEffects)
  → process_projectile_hits (fire OnHitEffects)

PreUpdate:
  handle_ability_projectile_spawn (Phase 2 bullet spawn)
```

All systems in `FixedUpdate` participate in rollback re-simulation.

---

## Code References

- [crates/protocol/src/ability.rs:504-516](crates/protocol/src/ability.rs#L504) — current prespawn pattern for projectiles
- [crates/protocol/src/ability.rs:525-548](crates/protocol/src/ability.rs#L525) — bullet spawn with `DisableRollback`
- [crates/protocol/src/ability.rs:184-192](crates/protocol/src/ability.rs#L184) — `AbilityBulletOf`/`AbilityBullets` relationship
- [crates/protocol/src/lib.rs:160-196](crates/protocol/src/lib.rs#L160) — all lightyear component registration
- [crates/protocol/src/lib.rs:239-268](crates/protocol/src/lib.rs#L239) — system schedule
- [crates/server/src/gameplay.rs:151-174](crates/server/src/gameplay.rs#L151) — server character spawn (Replicate + PredictionTarget + ControlledBy)
- [crates/client/src/gameplay.rs:18-42](crates/client/src/gameplay.rs#L18) — client handling of Predicted/Replicated entities
- [crates/render/src/health_bar.rs:44-64](crates/render/src/health_bar.rs#L44) — only ChildOf/with_children usage in game code
- [git/lightyear/lightyear_replication/src/prespawn.rs](git/lightyear/lightyear_replication/src/prespawn.rs) — prespawn hash and matching
- [git/lightyear/lightyear_replication/src/hierarchy.rs](git/lightyear/lightyear_replication/src/hierarchy.rs) — ReplicateLike hierarchy propagation
- [git/lightyear/lightyear_prediction/src/rollback.rs](git/lightyear/lightyear_prediction/src/rollback.rs) — rollback, DisableRollback, DeterministicPredicted
- [git/lightyear/lightyear_prediction/src/despawn.rs](git/lightyear/lightyear_prediction/src/despawn.rs) — prediction_despawn()
- [git/lightyear/lightyear_replication/src/send/archetypes.rs:77-112](git/lightyear/lightyear_replication/src/send/archetypes.rs#L77) — component filtering (only registered components replicated)
- [git/lightyear/lightyear_replication/src/send/buffer.rs:131-147](git/lightyear/lightyear_replication/src/send/buffer.rs#L131) — ReplicateLike child iteration in send pipeline
- [git/lightyear/lightyear_replication/src/send/plugin.rs:448-453](git/lightyear/lightyear_replication/src/send/plugin.rs#L448) — component-remove observer scoped to registered IDs only
- [git/lightyear/demos/spaceships/src/shared.rs:222-224](git/lightyear/demos/spaceships/src/shared.rs#L222) — canonical example of client_id salt for prespawning
- [git/bevy/crates/bevy_transform/src/plugins.rs:17-42](git/bevy/crates/bevy_transform/src/plugins.rs#L17) — transform propagation runs only in PostUpdate (not FixedUpdate)

## Related Research

- [doc/research/2026-02-20-ability-effect-primitives-implementation-analysis.md](doc/research/2026-02-20-ability-effect-primitives-implementation-analysis.md) — superseded by this document; covers data model and dispatch changes but omits replication and hierarchy

## Resolved Questions

### 1. Prespawn Salt Strategy (RESOLVED)

**Decision**: Compound salt including client ID.

The `PreSpawned` hash is computed from exactly three inputs ([prespawn.rs:380-440](git/lightyear/lightyear_replication/src/prespawn.rs#L380)): current tick (`u16`), sorted `NetId`s of registered components in the archetype, and optional user salt. **No per-client or per-entity information is included by default.** Two players activating the same ability on the same tick produce identical hashes without a salt.

The lightyear spaceships demo confirms this pattern ([git/lightyear/demos/spaceships/src/shared.rs:222-224](git/lightyear/demos/spaceships/src/shared.rs#L222)):
```rust
// the default hashing algorithm uses the tick and component list. in order to disambiguate
// between two players spawning a bullet on the same tick, we add client_id to the mix.
let prespawned = PreSpawned::default_with_salt(player.client_id.to_bits());
```

For `ActiveAbility` entities where multiple abilities can be active on the same tick per player, the salt must encode:
- **Client identity** (prevents cross-player hash collision)
- **Ability slot or def_id** (disambiguates concurrent abilities from the same player)
- **Depth** (disambiguates recursive sub-ability spawns)

Example compound salt:
```rust
let salt = (client_id as u64) << 32
         | (ability_slot as u64) << 16
         | (depth as u64);
PreSpawned::default_with_salt(salt)
```

**Hash collision behavior** ([prespawn.rs:104-120](git/lightyear/lightyear_replication/src/prespawn.rs#L104)): When multiple entities share a hash, they are stored in a `Vec` and matched via `pop()` (last-inserted wins). A warning is logged: `"Multiple pre-spawned entities share the same hash, this might cause extra rollbacks"`.

**Client ID availability**: The salt needs the client ID on both client and server. The server has it via `ControlledBy`. The client needs access to its own client ID — check how the existing codebase or lightyear API exposes this.

### 2. Custom Relationship for Hitboxes (RESOLVED)

**Decision**: Use custom `#[relationship]` (not `ChildOf`).

Bevy's `propagate_parent_transforms` runs only in `PostUpdate` ([git/bevy/crates/bevy_transform/src/plugins.rs:30-40](git/bevy/crates/bevy_transform/src/plugins.rs#L30)), not in `FixedUpdate`. `GlobalTransform` values read during `FixedUpdate` are stale from the previous frame. Since all ability/hitbox systems run in `FixedUpdate` using `Position`/`Rotation` (avian3d), `ChildOf` transform propagation provides no benefit and could cause confusion.

Custom relationship with `linked_spawn` ties lifecycle without transform coupling, matching the existing `AbilityBulletOf`/`AbilityBullets` pattern.

### 3. ActiveAbility Participates in Rollback (RESOLVED)

**Decision**: `ActiveAbility` entities use `.add_prediction()`, NOT `DisableRollback`.

Ability phase progression determines which effects fire. Incorrect phase on rollback would produce wrong game state. `ActiveAbility` must be predicted and rolled back so that re-simulation recomputes phases correctly.

### 4. Use `prediction_despawn()` (RESOLVED)

**Decision**: `ActiveAbility` entities use `prediction_despawn()` when their phase sequence completes.

This inserts `PredictionDisable` instead of immediately despawning, allowing rollback to restore the entity if needed. Safer than `try_despawn()` under network jitter.

### 5. Unregistered Marker Components on Replicated Entities (RESOLVED)

**Finding: Lightyear only replicates explicitly registered components. Unregistered components are silently ignored.**

The filtering occurs in `ReplicatedArchetypes::update()` ([send/archetypes.rs:77-112](git/lightyear/lightyear_replication/src/send/archetypes.rs#L77)). For each component in an archetype, lightyear:

1. Gets the component's `TypeId`
2. Looks it up in `ComponentRegistry.component_metadata_map`
3. Checks for `replication` metadata
4. **If not found**: emits a `trace!` log (`"not including {:?} because it is not registered for replication"`) and skips the component

This means `OnCastEffects`, `WhileActiveEffects`, `OnInputEffects`, `OnEndEffects` can safely live on `ActiveAbility` entities without registration. They will never be serialized or sent over the network. No `DisableReplicateHierarchy` needed.

The component-remove observer is also scoped to only registered `ComponentId`s ([send/plugin.rs:448-453](git/lightyear/lightyear_replication/src/send/plugin.rs#L448)), so insertion/removal of unregistered marker components won't trigger replication events.

### 6. `OnHitEffects` on Hitbox Entities with `ReplicateLike` (RESOLVED — but moot given Q2 decision)

**Finding: `ReplicateLike` entities use the same component registry filtering as `Replicate` entities. Unregistered components are ignored.**

`ReplicateLike` child entities go through the identical archetype-filtering path as `Replicate` entities ([send/archetypes.rs:69-73](git/lightyear/lightyear_replication/src/send/archetypes.rs#L69) — archetypes with `ReplicateLike` are tracked; [send/buffer.rs:131-147](git/lightyear/lightyear_replication/src/send/buffer.rs#L131) — children are iterated via `ReplicateLikeChildren` and each calls `replicate_entity` with the same component filtering).

However, **this is moot for the current design**: since we're using custom relationships (not `ChildOf`) per question 2, hitbox entities won't have `ReplicateLike` propagated to them. They'll be independent entities.

If hitbox entities need replication at all (for multiplayer hit detection consistency), they would need their own `Replicate` + `PreSpawned` + `PredictionTarget`. If they are local-only (spawned in shared `FixedUpdate`, never replicated), they need none of these — just `DisableRollback` like current bullets.

**Key detail on `ReplicateLike` behavior**: Child entities with `ReplicateLike` inherit `Replicate`, `PredictionTarget`, `ControlledBy`, and `NetworkVisibility` settings from the root entity (with per-child overrides possible). `PreSpawned` is always read from the child entity itself. Children are NOT tracked in `sender.replicated_entities` — they're discovered via `ReplicateLikeChildren` during root iteration ([send/buffer.rs:131](git/lightyear/lightyear_replication/src/send/buffer.rs#L131)).

### 7. Client ID Access in Shared Code (RESOLVED)

**Finding: Store `PeerId` in a replicated component on the character entity.**

Lightyear provides `LocalId(pub PeerId)` as a component on the `Client` entity ([lightyear_core/src/id.rs:17](git/lightyear/lightyear_core/src/id.rs#L17)), accessible via `Single<&LocalId, With<Client>>`. However, this only exists on the client side — the server has no `Client` entity. In a shared `FixedUpdate` system that queries *character* entities (not connection entities), the system needs the `PeerId` *on the character entity itself*.

`ControlledBy.owner` is an `Entity` pointing to a server-side connection entity — not a `PeerId`. On the client, only the fieldless `Controlled` marker exists ([control.rs:11](git/lightyear/lightyear_replication/src/control.rs#L11)). There is no path from `ControlledBy` → `PeerId` without a secondary query against the connection entity (server-only).

The established lightyear pattern across all demos and examples is:

1. **Define** a replicated component: `PlayerId(pub PeerId)` (used by fps, projectiles, avian_physics, lobby examples) or `Player { client_id: PeerId, ... }` (spaceships demo)
2. **Server inserts** it on the character entity at spawn, reading `RemoteId` from the `ClientOf` connection entity ([git/lightyear/demos/spaceships/src/server.rs:90-122](git/lightyear/demos/spaceships/src/server.rs#L90))
3. **Client receives** it via replication — available on the predicted entity
4. **Shared system queries** this component on the character entity, calls `.to_bits()` for salt

The spaceships demo's `shared_player_firing` system reads `Player.client_id` at [shared.rs:167,224](git/lightyear/demos/spaceships/src/shared.rs#L167):
```rust
let prespawned = PreSpawned::default_with_salt(player.client_id.to_bits());
```

**This codebase does not currently have a `PlayerId` component.** The server's `handle_connected` ([server/src/gameplay.rs:128-175](crates/server/src/gameplay.rs#L128)) spawns characters with `ControlledBy` but no `PeerId`-carrying component. Adding one is a prerequisite for the compound prespawn salt.

### 8. Hitbox Entity Replication Strategy (RESOLVED)

**Decision: Local-only with `DisableRollback`.**

Hitbox/AoE entities are deterministically derived from `ActiveAbility` state in shared `FixedUpdate`. Both client and server run the same simulation and produce the same hitbox entities. This matches the existing bullet pattern — bullets are spawned in shared `FixedUpdate` with `DisableRollback` and no replication on the bullet entity itself (only the `AbilityProjectileSpawn` parent is replicated).

## Open Questions

None remaining. All questions resolved.
