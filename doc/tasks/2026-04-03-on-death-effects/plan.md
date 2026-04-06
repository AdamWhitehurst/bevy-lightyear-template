# Implementation Plan

## Overview
All death behavior flows through `DeathEvent` → `OnDeathEffects` dispatch. Character respawn is `StartRespawnPointTimer`, tree→stump is `TransformInto`, one-time props use `DespawnEntity`. `RespawnTimerConfig` is deleted. `RespawnTimer` changes from absolute `expires_at: Tick` to relative `ticks_remaining: u16` for persistence compatibility. Transformation state persists across chunk eviction. `PlacementOffset` double-apply bug fixed.

---

## Phase 1: Unified Death Handling

Replace polling-based `start_respawn_timer` with event-driven `DeathEvent` → `on_death_effects` dispatch. Character respawn becomes a `DeathEffect` variant. No new visual behavior — existing respawn flow preserved through the new path.

### Changes

#### 1. DeathEvent type and apply_damage transition detection
**File**: `crates/protocol/src/character/types.rs`
**Action**: modify

Add after the `Health` impl block (lines 48–64):

```rust
/// Emitted when an entity's health transitions from alive to dead.
#[derive(Event)]
pub struct DeathEvent {
    pub entity: Entity,
}
```

Modify `apply_damage` (line 53–55) to return alive→dead transition:

```rust
/// Applies damage, clamping to zero. Returns `true` if this caused the alive→dead transition.
pub fn apply_damage(&mut self, damage: f32) -> bool {
    let was_alive = self.current > 0.0;
    self.current = (self.current - damage).max(0.0);
    was_alive && self.current <= 0.0
}
```

#### 2. Change RespawnTimer and Invulnerable to relative ticks
**File**: `crates/protocol/src/character/types.rs`
**Action**: modify

Replace `Invulnerable` (lines 66–70):

```rust
/// Post-respawn invulnerability. Prevents damage while present. Decremented each tick.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
#[reflect(Component)]
pub struct Invulnerable {
    pub ticks_remaining: u16,
}
```

Replace `RespawnTimer` (lines 91–96):

```rust
/// Marks an entity as dead and awaiting respawn. Decremented each FixedUpdate tick.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Reflect)]
#[reflect(Component)]
pub struct RespawnTimer {
    pub ticks_remaining: u16,
}
```

#### 3. Delete RespawnTimerConfig
**File**: `crates/protocol/src/character/types.rs`
**Action**: modify

Delete `DEFAULT_RESPAWN_TICKS` constant (line 73), `RespawnTimerConfig` struct (lines 75–89).

#### 4. Update re-exports
**File**: `crates/protocol/src/character/mod.rs`
**Action**: modify

Remove `RespawnTimerConfig` and `DEFAULT_RESPAWN_TICKS` from `pub use types::` line (line 7).
Add `DeathEvent` to the re-export.

**File**: `crates/protocol/src/lib.rs`
**Action**: modify

Remove `RespawnTimerConfig` and `DEFAULT_RESPAWN_TICKS` from `pub use character::` (line 34).
Add `DeathEvent`.

#### 5. OnDeathEffects and DeathEffect types
**File**: `crates/protocol/src/world_object/types.rs`
**Action**: modify

Add after existing types (after `VisualKind` enum):

```rust
/// Describes effects triggered when this object dies. Defined in `.object.ron`
/// or inserted programmatically (e.g. on characters).
#[derive(Component, Reflect, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[reflect(Component)]
pub struct OnDeathEffects(pub Vec<DeathEffect>);
```

```rust
/// A single effect applied on death.
#[derive(Reflect, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum DeathEffect {
    /// Lock entity, teleport to nearest RespawnPoint after timer expires.
    StartRespawnPointTimer { duration_ticks: u16 },
    /// Replace this entity's components with those from another object def.
    TransformInto {
        source: String,
        revert_after_ticks: Option<u16>,
    },
    /// Despawn the entity immediately.
    DespawnEntity,
}
```

Add `serde::{Deserialize, Serialize}` to imports if not already present.

#### 6. Re-export new types
**File**: `crates/protocol/src/world_object/mod.rs`
**Action**: modify

Add to `pub use types::`: `DeathEffect, OnDeathEffects`

#### 7. Register types in WorldObjectPlugin
**File**: `crates/protocol/src/world_object/plugin.rs`
**Action**: modify

Remove `app.register_type::<crate::RespawnTimerConfig>();` (line 51).

Add:
```rust
app.register_type::<super::types::OnDeathEffects>();
app.register_type::<super::types::DeathEffect>();
```

#### 8. Remove RespawnTimerConfig lightyear registration
**File**: `crates/protocol/src/lib.rs`
**Action**: modify

Delete line 168: `app.register_component::<RespawnTimerConfig>();`

#### 9. Emit DeathEvent at the damage site
**File**: `crates/protocol/src/hit_detection/effects.rs`
**Action**: modify
Add `death_events: &mut EventWriter<DeathEvent>` parameter to `apply_on_hit_effects` (after `rotation_query`).

In the `Damage` arm (~line 98–101):

```rust
if let Ok((_, _, mut health, invulnerable)) = target_query.get_mut(entity) {
    if invulnerable.is_none() && health.apply_damage(remaining_damage) {
        death_events.write(DeathEvent { entity });
    }
} else {
    warn!("Damage target {:?} not found", entity);
}
```

Add `use crate::DeathEvent;` to imports.

#### 10. Thread EventWriter through call sites
**File**: `crates/protocol/src/hit_detection/systems.rs`
**Action**: modify
Add `mut death_events: EventWriter<DeathEvent>` system parameter to both `process_hitbox_hits` (~line 33) and `process_projectile_hits` (~line 107). Pass `&mut death_events` to each `apply_on_hit_effects` call.

Add `use crate::DeathEvent;` to imports.

#### 11. on_death_effects system (Phase 1: only StartRespawnPointTimer and DespawnEntity)
**File**: `crates/server/src/gameplay.rs`
**Action**: modify
Add new system:

```rust
/// Dispatches death effects for entities with OnDeathEffects.
fn on_death_effects(
    mut commands: Commands,
    mut events: EventReader<DeathEvent>,
    query: Query<&OnDeathEffects>,
) {
    for event in events.read() {
        let Ok(effects) = query.get(event.entity) else {
            trace!("Entity {:?} died without OnDeathEffects, no dispatch", event.entity);
            continue;
        };
        for effect in &effects.0 {
            match effect {
                DeathEffect::StartRespawnPointTimer { duration_ticks } => {
                    commands.entity(event.entity).insert((
                        RespawnTimer {
                            ticks_remaining: *duration_ticks,
                        },
                        RigidBodyDisabled,
                        ColliderDisabled,
                    ));
                }
                DeathEffect::DespawnEntity => {
                    commands.entity(event.entity).try_despawn();
                }
                DeathEffect::TransformInto { .. } => {
                    trace!("TransformInto not yet implemented until phase 2");
                }
            }
        }
    }
}
```

#### 12. Rewrite process_respawn_timers for relative ticks, always teleport
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

Replace `process_respawn_timers` (lines 160–203):

```rust
/// Decrements respawn timers. On expiry: teleport to nearest RespawnPoint,
/// restore health, grant invulnerability, re-enable physics.
fn process_respawn_timers(
    mut commands: Commands,
    mut query: Query<
        (
            Entity,
            &mut RespawnTimer,
            &mut Health,
            &mut Position,
            Option<&mut LinearVelocity>,
        ),
        Without<RespawnPoint>,
    >,
    respawn_query: Query<&Position, With<RespawnPoint>>,
) {
    for (entity, mut timer, mut health, mut position, velocity) in &mut query {
        timer.ticks_remaining = timer.ticks_remaining.saturating_sub(1);
        if timer.ticks_remaining > 0 {
            continue;
        }
        let respawn_pos = nearest_respawn_pos(&position, &respawn_query);
        trace!("Entity {entity:?} respawn timer expired, respawning at {respawn_pos:?}");
        position.0 = respawn_pos;
        if let Some(mut velocity) = velocity {
            velocity.0 = Vec3::ZERO;
        }
        health.restore_full();
        commands
            .entity(entity)
            .remove::<(RespawnTimer, RigidBodyDisabled, ColliderDisabled)>();
        commands.entity(entity).insert(Invulnerable {
            ticks_remaining: 128,
        });
    }
}
```

Update `nearest_respawn_pos` — remove `Without<CharacterMarker>` filter:

```rust
fn nearest_respawn_pos(
    current_pos: &Position,
    respawn_query: &Query<&Position, With<RespawnPoint>>,
) -> Vec3 {
    // ... body unchanged ...
}
```

#### 13. Rewrite expire_invulnerability for relative ticks
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

Replace `expire_invulnerability` (lines 220–231):

```rust
fn expire_invulnerability(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Invulnerable)>,
) {
    for (entity, mut invuln) in &mut query {
        invuln.ticks_remaining = invuln.ticks_remaining.saturating_sub(1);
        if invuln.ticks_remaining == 0 {
            commands.entity(entity).remove::<Invulnerable>();
        }
    }
}
```

No longer needs `Res<LocalTimeline>`.

#### 14. Delete start_respawn_timer, update system schedule
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

Delete `start_respawn_timer` function entirely (lines 133–157).

Update system schedule inside `ServerGameplayPlugin::build` (starts at line 19, schedule at lines 26–33):

```rust
app.add_event::<DeathEvent>();
app.add_systems(
    FixedUpdate,
    (
        on_death_effects
            .after(hit_detection::process_projectile_hits)
            .after(hit_detection::process_hitbox_hits),
        process_respawn_timers.after(on_death_effects),
        expire_invulnerability,
    ),
);
```

Add imports: `OnDeathEffects`, `DeathEffect`, `WorldObjectId` from `protocol::world_object`.

#### 15. Insert OnDeathEffects on characters at spawn
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

In `handle_connected` (~line 285–289), replace `RespawnTimerConfig::default()` with:

```rust
OnDeathEffects(vec![DeathEffect::StartRespawnPointTimer {
    duration_ticks: 256,
}]),
```

In `spawn_dummy_target` (~line 65), replace `RespawnTimerConfig::default()` with:

```rust
OnDeathEffects(vec![DeathEffect::StartRespawnPointTimer {
    duration_ticks: 256,
}]),
```

#### 16. Remove RespawnTimerConfig from tree_circle.object.ron
**File**: `assets/objects/tree_circle.object.ron`
**Action**: modify

Delete the line:
```ron
    "protocol::RespawnTimerConfig": (duration_ticks: 384),
```

(Tree will get `OnDeathEffects([TransformInto { ... }])` in Phase 2.)

### Verification
#### Automated
- [x] `cargo check-all` passes

#### Manual
- [ ] `cargo server` + `cargo client` — damage a character to death
- [ ] Confirm: entity gets `RespawnTimer`, physics disabled, teleports to `RespawnPoint` after countdown, health restored, invulnerability granted
- [ ] Confirm: multiple hits on dead entity don't produce duplicate events (only one `RespawnTimer` inserted)
- [ ] Confirm: trees with `Health` but no `OnDeathEffects` sit at 0 HP with no death behavior (intentional — they get `OnDeathEffects` in Phase 2)
- [ ] Confirm: `DeathEvent` fires only once per alive→dead transition (hitting a 0 HP entity again does not re-emit)

---

## Phase 2: Transform on Death (End-to-End)

Tree dies → stump appears in-place. Implements `TransformInto` handler, transformation diff logic, lightyear registration, and client visual reconstruction.

### Changes

#### 1. ActiveTransformation type
**File**: `crates/protocol/src/world_object/types.rs`
**Action**: modify

Add after `DeathEffect`:

```rust
/// Tracks an active transformation on a world object. Persisted across chunk eviction.
#[derive(Component, Reflect, Clone, Debug)]
#[reflect(Component)]
pub struct ActiveTransformation {
    pub source: String,
    pub ticks_remaining: Option<u16>,
}
```

#### 2. Re-export and register
**File**: `crates/protocol/src/world_object/mod.rs`
**Action**: modify

Add `ActiveTransformation` to `pub use types::`.

**File**: `crates/protocol/src/world_object/plugin.rs`
**Action**: modify

Add:
```rust
app.register_type::<super::types::ActiveTransformation>();
```

#### 3. Register VisualKind and ActiveTransformation with lightyear
**File**: `crates/protocol/src/lib.rs`
**Action**: modify

Add after `WorldObjectId` registration (~line 158):

```rust
app.register_component::<world_object::VisualKind>();
app.register_component::<world_object::ActiveTransformation>();
```

No `.add_prediction()` — replicated-only. Note: `VisualKind` was not previously lightyear-registered. This registration is a **prerequisite** for `on_visual_kind_changed` (Phase 2 step 6) — without it, the client never receives `VisualKind` changes on existing entities.

#### 4. apply_transformation helper and TransformationContext SystemParam
**File**: `crates/server/src/world_object.rs`
**Action**: modify

`apply_transformation` and its callers (`on_death_effects`, `tick_active_transformations`) share the same 5 read-only resources. Bundle them:

```rust
#[derive(SystemParam)]
pub struct TransformationContext<'w> {
    pub defs: Res<'w, WorldObjectDefRegistry>,
    pub type_registry: Res<'w, AppTypeRegistry>,
    pub vox_registry: Res<'w, VoxModelRegistry>,
    pub vox_assets: Res<'w, Assets<VoxModelAsset>>,
    pub meshes: Res<'w, Assets<Mesh>>,
}
```

`apply_transformation` then takes `&TransformationContext` instead of 5 separate params. Callers use `ctx: TransformationContext` as a single system param.

Note: this is the first resource-only `SystemParam` in the codebase (existing ones like `VoxelWorld` and `MapCollisionHooks` bundle queries). Justified here by the 5-resource signature shared across 3 call sites. The Phase 4 exclusive systems (`evict_chunk_entities`, `save_all_chunk_entities_on_exit`) do NOT call `apply_transformation` — they only serialize — so no `SystemState` extraction is needed. `spawn_chunk_entities` (Phase 4 step 6) is a regular system and uses `TransformationContext` directly.

Add `use std::collections::HashSet;` and `use bevy::ecs::reflect::ReflectComponent;`.
Make `clone_def_components` and `vox_trimesh_collider` `pub(crate)` (both are private fns in this file).
Note: `apply_object_components` is imported from `protocol::world_object`, not defined here.

Add:

```rust
/// Transforms an entity by diffing current def against source def.
///
/// Removes components in `current_def` but absent from `source_def`.
/// Inserts/overwrites components from `source_def`.
pub fn apply_transformation(
    commands: &mut Commands,
    entity: Entity,
    current_def: &WorldObjectDef,
    source_def: &WorldObjectDef,
    ctx: &TransformationContext,
) {
    let source_type_paths: HashSet<&str> = source_def
        .components
        .iter()
        .map(|c| c.reflect_type_path())
        .collect();

    remove_absent_components(commands, entity, current_def, &source_type_paths, &ctx.type_registry);

    let vox_collider = vox_trimesh_collider(source_def, &ctx.vox_registry, &ctx.vox_assets, &ctx.meshes);
    let use_vox_collider = vox_collider.is_some();
    let components = clone_def_components(source_def, use_vox_collider);
    apply_object_components(commands, entity, components, ctx.type_registry.0.clone());

    if let Some(collider) = vox_collider {
        commands.entity(entity).insert(collider);
    }
}

fn remove_absent_components(
    commands: &mut Commands,
    entity: Entity,
    current_def: &WorldObjectDef,
    keep_paths: &HashSet<&str>,
    type_registry: &AppTypeRegistry,
) {
    let registry = type_registry.read();
    for component in &current_def.components {
        let path = component.reflect_type_path();
        if keep_paths.contains(path) {
            continue;
        }
        let Some(registration) = registry.get_with_type_path(path) else {
            continue;
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>().cloned() else {
            continue;
        };
        commands.queue(move |world: &mut World| {
            if let Some(mut entity_mut) = world.get_entity_mut(entity) {
                reflect_component.remove(&mut entity_mut);
            }
        });
    }
}
```

#### 5. Implement TransformInto in on_death_effects
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

Expand `on_death_effects` signature and `TransformInto` arm:

```rust
fn on_death_effects(
    mut commands: Commands,
    mut events: EventReader<DeathEvent>,
    query: Query<(&OnDeathEffects, Option<&WorldObjectId>)>,
    ctx: TransformationContext,
) {
    for event in events.read() {
        let Ok((effects, obj_id)) = query.get(event.entity) else {
            trace!("Entity {:?} died without OnDeathEffects, no dispatch", event.entity);
            continue;
        };
        for effect in &effects.0 {
            match effect {
                DeathEffect::StartRespawnPointTimer { duration_ticks } => {
                    commands.entity(event.entity).insert((
                        RespawnTimer {
                            ticks_remaining: *duration_ticks,
                        },
                        RigidBodyDisabled,
                        ColliderDisabled,
                    ));
                }
                DeathEffect::DespawnEntity => {
                    commands.entity(event.entity).try_despawn();
                }
                DeathEffect::TransformInto { source, revert_after_ticks } => {
                    let Some(obj_id) = obj_id else {
                        debug_assert!(false, "TransformInto on entity without WorldObjectId");
                        continue;
                    };
                    let source_id = WorldObjectId(source.clone());
                    let Some(source_def) = ctx.defs.get(&source_id) else {
                        warn!("Unknown transformation source '{source}'");
                        continue;
                    };
                    let Some(current_def) = ctx.defs.get(obj_id) else {
                        warn!("Unknown current def '{}'", obj_id.0);
                        continue;
                    };
                    crate::world_object::apply_transformation(
                        &mut commands, event.entity,
                        current_def, source_def, &ctx,
                    );
                    commands.entity(event.entity).insert(ActiveTransformation {
                        source: source.clone(),
                        ticks_remaining: *revert_after_ticks,
                    });
                }
            }
        }
    }
}
```

`debug_assert!` for missing `WorldObjectId` — this is a configuration error (TransformInto should only exist on world objects which always have an ID). `warn!` for missing defs — could be a typo in RON data, non-fatal.

Add imports: `ActiveTransformation`, `TransformationContext`.

#### 6. Refactor attach_visual and add on_visual_kind_changed
**File**: `crates/client/src/world_object.rs`
**Action**: modify

Now that `VisualKind` is replicated via lightyear (step 3), the client receives it as a component. Visual setup moves entirely to `on_visual_kind_changed` — which fires on both initial replication (`Added` implies `Changed`) and subsequent transformations. Remove all visual/mesh handling from `on_world_object_replicated` (the `attach_visual` / `attach_vox_mesh` calls and the child entity spawn). `on_world_object_replicated` continues to handle collider setup and non-visual def components only.

Refactor `attach_visual` to take `&VisualKind` directly (instead of `&WorldObjectDef`):

```rust
/// Attaches visual mesh for the given VisualKind.
fn attach_visual(
    commands: &mut Commands,
    entity: Entity,
    visual: &VisualKind,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    default_material: &DefaultVoxModelMaterial,
) {
    match visual {
        VisualKind::Vox(path) => {
            attach_vox_mesh(commands, entity, path, vox_registry, vox_assets, default_material);
        }
        _ => {
            trace!("World object entity {entity:?} has no Vox visual, skipping mesh attachment");
        }
    }
}
```

Add `on_visual_kind_changed` — the single system for all world object visual setup (initial and subsequent):

```rust
/// Rebuilds visuals when VisualKind changes via replication (e.g. tree→stump).
pub fn on_visual_kind_changed(
    mut commands: Commands,
    query: Query<(Entity, &VisualKind), Changed<VisualKind>>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    default_material: Res<DefaultVoxModelMaterial>,
    children_query: Query<&Children>,
) {
    for (entity, visual) in &query {
        despawn_visual_children(&mut commands, entity, &children_query);
        attach_visual(
            &mut commands, entity, visual,
            &vox_registry, &vox_assets, &default_material,
        );
    }
}

fn despawn_visual_children(
    commands: &mut Commands,
    entity: Entity,
    children_query: &Query<&Children>,
) {
    if let Ok(children) = children_query.get(entity) {
        for &child in children.iter() {
            commands.entity(child).despawn();
        }
    }
}
```

**File**: `crates/client/src/gameplay.rs`
**Action**: modify

Register alongside `on_world_object_replicated` (line 23):

```rust
app.add_systems(Update, (
    on_world_object_replicated,
    on_visual_kind_changed,
).run_if(ready));
```

Add `on_visual_kind_changed` to import from `crate::world_object`.

#### 7. Stump object def
**File**: `assets/objects/stump_circle.object.ron`
**Action**: create

```ron
{
    "protocol::world_object::types::ObjectCategory": Scenery,
    // Placeholder: reuses tree model until a stump vox model is created
    "protocol::world_object::types::VisualKind": Vox("models/trees/tree_circle.vox"),
    "avian3d::collision::collider::constructor::ColliderConstructor": Cylinder(radius: 0.5, height: 1.0),
    "avian3d::dynamics::rigid_body::RigidBody": Static,
    "avian3d::collision::collider::layers::CollisionLayers": (memberships: (32), filters: (14)),
}
```

No `Health` — stumps are intentionally undamageable. No `PlacementOffset` — position inherited from original entity.

#### 8. Add OnDeathEffects to tree_circle.object.ron
**File**: `assets/objects/tree_circle.object.ron`
**Action**: modify

Add:
```ron
    "protocol::world_object::types::OnDeathEffects": ([
        TransformInto(
            source: "stump_circle.object.ron",
            revert_after_ticks: Some(1000),
        ),
    ]),
```

### Verification
#### Automated
- [x] `cargo check-all` passes

#### Manual
- [ ] `cargo server` + `cargo client` — damage tree to death
- [ ] Observe: stump appears in same position (no pop, no gap)
- [ ] Observe: stump has no health (not damageable)
- [ ] Observe: characters still respawn via `StartRespawnPointTimer` path
- [ ] Observe: `ActiveTransformation` present on stump entity

---

## Phase 3: Revert After Delay

Stump reverts to tree after configured ticks.

### Changes

#### 1. tick_active_transformations system
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

Add:

```rust
/// Decrements active transformation timers. Reverts entity when countdown reaches zero.
fn tick_active_transformations(
    mut commands: Commands,
    mut query: Query<(Entity, &mut ActiveTransformation, &WorldObjectId)>,
    ctx: TransformationContext,
) {
    for (entity, mut transform, obj_id) in &mut query {
        let Some(ref mut remaining) = transform.ticks_remaining else {
            continue; // Permanent transformation — trace emitted at insertion time (on_death_effects)
        };
        *remaining = remaining.saturating_sub(1);
        if *remaining > 0 {
            continue;
        }
        let source_id = WorldObjectId(transform.source.clone());
        let Some(source_def) = ctx.defs.get(&source_id) else {
            warn!("Cannot revert: unknown source def '{}'", transform.source);
            continue;
        };
        let Some(original_def) = ctx.defs.get(obj_id) else {
            warn!("Cannot revert: unknown original def '{}'", obj_id.0);
            continue;
        };
        crate::world_object::apply_transformation(
            &mut commands, entity,
            source_def, original_def, &ctx,
        );
        commands.entity(entity).remove::<ActiveTransformation>();
    }
}
```

#### 2. Register system
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

Add to `FixedUpdate` schedule:

```rust
tick_active_transformations,
```

### Verification
#### Automated
- [ ] `cargo check-all` passes

#### Manual
- [ ] `cargo server` + `cargo client` — damage tree, observe stump
- [ ] Wait ~16s (1000 ticks at 60Hz) — tree reappears with full health
- [ ] Damage again — full cycle repeats
- [ ] Characters unaffected

---

## Phase 4: Persistence Across Eviction

Transformation and respawn timer state survive chunk unload/reload. Fixes `PlacementOffset` double-apply.

### Changes

#### 1. ReflectPersist and ReflectSpawnOnly type data markers
**File**: `crates/protocol/src/world_object/types.rs`
**Action**: modify
These are the first custom reflect type data markers in the codebase. The `FromType` pattern follows Bevy's own convention (`ReflectComponent`, `ReflectDefault`, etc.) — novel for this project but well-established in the ecosystem.

Add:

```rust
/// Reflect type data: component is serialized during chunk eviction.
#[derive(Clone)]
pub struct ReflectPersist;

impl<T: Reflect> bevy::reflect::FromType<T> for ReflectPersist {
    fn from_type() -> Self {
        ReflectPersist
    }
}

/// Reflect type data: component is only applied on first spawn, skipped on reload.
#[derive(Clone)]
pub struct ReflectSpawnOnly;

impl<T: Reflect> bevy::reflect::FromType<T> for ReflectSpawnOnly {
    fn from_type() -> Self {
        ReflectSpawnOnly
    }
}
```

Re-export `ReflectPersist`, `ReflectSpawnOnly` from `mod.rs`.

#### 2. Apply markers
**File**: `crates/protocol/src/world_object/types.rs` — on `ActiveTransformation`:
```rust
#[reflect(Component, Persist)]
```

**File**: `crates/protocol/src/world_object/types.rs` — on `PlacementOffset`:
```rust
#[reflect(Component, SpawnOnly)]
```

**File**: `crates/protocol/src/character/types.rs` — on `Health`:
```rust
#[reflect(Component, Default, Persist)]
```
Add `use crate::world_object::ReflectPersist;` to imports.

**File**: `crates/protocol/src/character/types.rs` — on `RespawnTimer`:
```rust
#[reflect(Component, Persist)]
```

Note: both `Health` and `RespawnTimer` are in `character/types.rs`. The single `use crate::world_object::ReflectPersist;` import covers both.

#### 3. Register markers in WorldObjectPlugin
**File**: `crates/protocol/src/world_object/plugin.rs`
**Action**: modify

The `#[reflect(Persist)]` / `#[reflect(SpawnOnly)]` attributes should auto-register via `FromType` during `register_type`. If not, add explicit:

```rust
app.register_type_data::<super::types::ActiveTransformation, super::types::ReflectPersist>();
app.register_type_data::<super::types::PlacementOffset, super::types::ReflectSpawnOnly>();
app.register_type_data::<crate::Health, super::types::ReflectPersist>();
app.register_type_data::<crate::character::RespawnTimer, super::types::ReflectPersist>();
```

#### 4. Expand WorldObjectSpawn with persisted components
**File**: `crates/voxel_map_engine/src/config.rs`
**Action**: modify

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldObjectSpawn {
    pub object_id: String,
    pub position: Vec3,
    #[serde(default)]
    pub persisted_components: Vec<PersistedComponent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedComponent {
    pub type_path: String,
    pub ron_data: String,
}
```

`#[serde(default)]` ensures backward compatibility with old save files.

**File**: `crates/voxel_map_engine/src/persistence.rs`
**Action**: modify

Bump `ENTITY_SAVE_VERSION` from `1` to `2`.

#### 5. Serialize ReflectPersist components during eviction
**File**: `crates/server/src/chunk_entities.rs`
**Action**: modify
Refactor `evict_chunk_entities` to an exclusive system (`&mut World`) to access both query results and arbitrary reflected components. The two-pass approach (collect entity IDs, then serialize) is required because `query.iter(world)` borrows `world` immutably, preventing `world.entity()` during iteration.

```rust
/// Collects entities whose chunk column is no longer loaded.
fn collect_evictable_entities(
    world: &mut World,
) -> Vec<(Entity, Entity, IVec3, String, Vec3)> {
    let mut entity_query = world.query::<(Entity, &ChunkEntityRef, &WorldObjectId, &Position)>();
    let mut map_query = world.query::<(&VoxelMapInstance, &VoxelMapConfig)>();
    let mut result = Vec::new();
    for (entity, chunk_ref, obj_id, pos) in entity_query.iter(world) {
        let Ok((instance, _)) = map_query.get(world, chunk_ref.map_entity) else {
            continue;
        };
        let col = chunk_to_column(chunk_ref.chunk_pos);
        if instance.chunk_levels.contains_key(&col) {
            continue;
        }
        result.push((
            entity,
            chunk_ref.map_entity,
            chunk_ref.chunk_pos,
            obj_id.0.clone(),
            Vec3::from(pos.0),
        ));
    }
    result
}

/// Saves chunk entity data to disk and despawns entities.
fn save_and_despawn_evicted(
    world: &mut World,
    by_chunk: HashMap<(Entity, IVec3), Vec<(Entity, WorldObjectSpawn)>>,
) {
    let mut map_query = world.query::<&VoxelMapConfig>();
    for ((map_entity, chunk_pos), entities) in by_chunk {
        let Ok(config) = map_query.get(world, map_entity) else {
            continue;
        };
        let spawns: Vec<WorldObjectSpawn> = entities.iter().map(|(_, s)| s.clone()).collect();
        if let Some(ref dir) = config.save_dir {
            let dir = dir.clone();
            let pool = AsyncComputeTaskPool::get();
            pool.spawn(async move {
                if let Err(e) =
                    voxel_map_engine::persistence::save_chunk_entities(&dir, chunk_pos, &spawns)
                {
                    error!("Failed to save evicted chunk entities at {chunk_pos}: {e}");
                }
            })
            .detach();
        }
        for (entity, _) in entities {
            world.despawn(entity);
        }
    }
}
```

Then `evict_chunk_entities` becomes:

```rust
pub fn evict_chunk_entities(world: &mut World) {
    let type_registry = world.resource::<AppTypeRegistry>().clone();
    let registry = type_registry.read();

    let to_process = collect_evictable_entities(world);
    if to_process.is_empty() {
        return;
    }

    let mut by_chunk: HashMap<(Entity, IVec3), Vec<(Entity, WorldObjectSpawn)>> = HashMap::new();
    for (entity, map_entity, chunk_pos, object_id, position) in to_process {
        let entity_ref = world.entity(entity);
        let persisted = serialize_persisted_components(entity_ref, &registry);
        by_chunk
            .entry((map_entity, chunk_pos))
            .or_default()
            .push((entity, WorldObjectSpawn { object_id, position, persisted_components: persisted }));
    }

    drop(registry);
    save_and_despawn_evicted(world, by_chunk);
}
```

```rust
fn serialize_persisted_components(
    entity_ref: EntityRef,
    registry: &bevy::reflect::TypeRegistry,
) -> Vec<PersistedComponent> {
    let mut result = Vec::new();
    for registration in registry.iter() {
        if registration.data::<ReflectPersist>().is_none() {
            continue;
        }
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            continue;
        };
        let Some(component) = reflect_component.reflect(entity_ref) else {
            continue;
        };
        let serializer = bevy::reflect::serde::ReflectSerializer::new(component, registry);
        match ron::to_string(&serializer) {
            Ok(ron_data) => {
                result.push(PersistedComponent {
                    type_path: registration.type_info().type_path().to_string(),
                    ron_data,
                });
            }
            Err(e) => {
                warn!(
                    "Failed to serialize persisted '{}': {e}",
                    registration.type_info().type_path()
                );
            }
        }
    }
    result
}
```

#### 6. Restore persisted components on reload
**File**: `crates/server/src/chunk_entities.rs`
**Action**: modify

Add `TransformationContext` as a system param to `spawn_chunk_entities` (replacing the individual `defs`, `type_registry`, `vox_registry`, `vox_assets`, `meshes` params). This aligns with `apply_transformation`'s `&TransformationContext` signature from Phase 2 step 4.

In `spawn_chunk_entities`, after spawning entity and applying def components:

```rust
let is_reload = !spawn.persisted_components.is_empty();
let offset = extract_placement_offset(def, is_reload);
// ... existing position + ChunkEntityRef insertion ...

if is_reload {
    restore_persisted_components(&mut commands, entity, &spawn.persisted_components, &ctx.type_registry);

    if let Some(source_name) = find_persisted_source(&spawn.persisted_components) {
        let source_id = WorldObjectId(source_name);
        if let Some(source_def) = ctx.defs.get(&source_id) {
            crate::world_object::apply_transformation(
                &mut commands, entity, def, source_def, &ctx,
            );
        }
    }
}
```

Extract the transformation source from persisted RON without full deserialization:

```rust
fn find_persisted_source(persisted: &[PersistedComponent]) -> Option<String> {
    let pc = persisted.iter().find(|pc| pc.type_path.ends_with("ActiveTransformation"))?;
    #[derive(Deserialize)]
    struct Partial { source: String }
    let partial: Partial = ron::from_str(&pc.ron_data).ok()?;
    Some(partial.source)
}
```

This is safe because `ActiveTransformation`'s RON format is stable (we control the type). Avoids deferred-command complexity — the source name is available before command flush.

Deserialize persisted components, then reuse `apply_object_components` for batched insertion (single deferred command, same pattern as initial spawn):

```rust
fn restore_persisted_components(
    commands: &mut Commands,
    entity: Entity,
    persisted: &[PersistedComponent],
    type_registry: &AppTypeRegistry,
) {
    let registry = type_registry.read();
    let mut values: Vec<Box<dyn PartialReflect>> = Vec::new();
    for pc in persisted {
        let Some(registration) = registry.get_with_type_path(&pc.type_path) else {
            warn!("Unknown persisted type '{}'", pc.type_path);
            continue;
        };
        let Ok(mut de) = ron::Deserializer::from_str(&pc.ron_data) else {
            warn!("Invalid RON for persisted '{}'", pc.type_path);
            continue;
        };
        let type_de = bevy::reflect::serde::TypedReflectDeserializer::new(registration, &registry);
        match type_de.deserialize(&mut de) {
            Ok(value) => values.push(value),
            Err(e) => warn!("Failed to deserialize persisted '{}': {e}", pc.type_path),
        }
    }
    drop(registry);
    apply_object_components(commands, entity, values, type_registry.0.clone());
}
```

#### 7. Skip PlacementOffset on reload
**File**: `crates/server/src/chunk_entities.rs`
**Action**: modify

```rust
fn extract_placement_offset(def: &WorldObjectDef, is_reload: bool) -> Vec3 {
    if is_reload {
        return Vec3::ZERO;
    }
    def.components
        .iter()
        .find_map(|c| c.try_downcast_ref::<PlacementOffset>())
        .map(|o| o.0)
        .unwrap_or(Vec3::ZERO)
}
```

#### 8. Update save_all_chunk_entities_on_exit
**File**: `crates/server/src/chunk_entities.rs`
**Action**: modify

Refactor to an exclusive system to access reflected components. Current system reads `AppExit` messages, groups all entities by chunk, saves synchronously.

```rust
pub fn save_all_chunk_entities_on_exit(world: &mut World) {
    if !has_app_exit(world) {
        return;
    }

    let type_registry = world.resource::<AppTypeRegistry>().clone();
    let registry = type_registry.read();
    let by_chunk = collect_all_chunk_spawns(world, &registry);
    drop(registry);

    save_chunks_synchronously(world, by_chunk);
}

fn has_app_exit(world: &mut World) -> bool {
    world.resource_mut::<MessageReader<AppExit>>().read().next().is_some()
}

/// Collects all chunk entities grouped by (map_entity, chunk_pos) with persisted components.
fn collect_all_chunk_spawns(
    world: &mut World,
    registry: &bevy::reflect::TypeRegistry,
) -> HashMap<(Entity, IVec3), Vec<WorldObjectSpawn>> {
    let mut entity_query = world.query::<(Entity, &ChunkEntityRef, &WorldObjectId, &Position)>();
    let entities: Vec<_> = entity_query
        .iter(world)
        .map(|(entity, chunk_ref, obj_id, pos)| {
            (entity, chunk_ref.map_entity, chunk_ref.chunk_pos, obj_id.0.clone(), Vec3::from(pos.0))
        })
        .collect();

    let mut by_chunk: HashMap<(Entity, IVec3), Vec<WorldObjectSpawn>> = HashMap::new();
    for (entity, map_entity, chunk_pos, object_id, position) in entities {
        let entity_ref = world.entity(entity);
        let persisted = serialize_persisted_components(entity_ref, registry);
        by_chunk
            .entry((map_entity, chunk_pos))
            .or_default()
            .push(WorldObjectSpawn { object_id, position, persisted_components: persisted });
    }
    by_chunk
}

fn save_chunks_synchronously(
    world: &mut World,
    by_chunk: HashMap<(Entity, IVec3), Vec<WorldObjectSpawn>>,
) {
    let mut map_query = world.query::<&VoxelMapConfig>();
    for ((map_entity, chunk_pos), spawns) in by_chunk {
        let Ok(config) = map_query.get(world, map_entity) else {
            continue;
        };
        if let Some(ref dir) = config.save_dir {
            if let Err(e) =
                voxel_map_engine::persistence::save_chunk_entities(dir, chunk_pos, &spawns)
            {
                error!("Shutdown save failed for chunk {chunk_pos}: {e}");
            }
        }
    }
}
```

Key difference from `evict_chunk_entities`: saves all entities (not just evictable ones), saves synchronously (not async), and does not despawn. `collect_all_chunk_spawns` is reusable by both if needed.

### Verification
#### Automated
- [ ] `cargo check-all` passes

#### Manual
- [ ] `cargo server` + `cargo client` — damage tree, observe stump
- [ ] Walk away to trigger chunk eviction, return — stump reloads with remaining countdown
- [ ] Wait for countdown — tree reverts with full health
- [ ] Verify fresh trees spawn at correct position (PlacementOffset applied once)
- [ ] Verify reloaded trees don't double-offset
- [ ] Restart server — entities load correctly from shutdown save
- [ ] Kill character, trigger chunk eviction mid-respawn-timer, return — timer resumes correctly
