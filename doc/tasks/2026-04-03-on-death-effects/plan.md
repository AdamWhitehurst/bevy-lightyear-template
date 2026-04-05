# Implementation Plan

## Overview
When a world object with `OnDeathEffects` dies, it transforms in-place into another object def (e.g. tree→stump), optionally reverting after a tick countdown. Transformation state persists across chunk eviction/reload. Also fixes the `PlacementOffset` double-apply bug on reload.

---

## Phase 1: DeathEvent Infrastructure

### Changes

#### 1. DeathEvent type
**File**: `crates/protocol/src/character/types.rs`
**Action**: modify

Add `DeathEvent` after the `Health` impl block (~line 64):

```rust
/// Emitted when an entity's health transitions from alive to dead.
#[derive(Event)]
pub struct DeathEvent {
    pub entity: Entity,
}
```

Modify `Health::apply_damage` (line 53–55) to return whether the entity just died:

```rust
/// Applies damage, clamping to zero. Returns `true` if this caused the alive→dead transition.
pub fn apply_damage(&mut self, damage: f32) -> bool {
    let was_alive = self.current > 0.0;
    self.current = (self.current - damage).max(0.0);
    was_alive && self.current <= 0.0
}
```

#### 2. Emit DeathEvent at the damage site
**File**: `crates/protocol/src/hit_detection/effects.rs`
**Action**: modify

Thread `&mut EventWriter<DeathEvent>` through `apply_on_hit_effects`. Add parameter after `rotation_query`:

```rust
pub(crate) fn apply_on_hit_effects(
    // ... existing params ...
    rotation_query: &Query<&Rotation>,
    death_events: &mut EventWriter<DeathEvent>,
) {
```

In the `Damage` arm (~line 98–101), use the `bool` return to emit:

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

#### 3. Thread EventWriter through call sites
**File**: `crates/protocol/src/hit_detection/systems.rs`
**Action**: modify

Both `process_hitbox_hits` and `process_projectile_hits` need `mut death_events: EventWriter<DeathEvent>` as a system parameter, passed to `apply_on_hit_effects`.

In `process_hitbox_hits` (~line 33): add `mut death_events: EventWriter<DeathEvent>` param, pass `&mut death_events` to the call at line 69.

In `process_projectile_hits` (~line 107): add `mut death_events: EventWriter<DeathEvent>` param, pass `&mut death_events` to the call at line 138.

Add `use crate::DeathEvent;` to imports (or `use super::effects::DeathEvent` depending on module structure — `DeathEvent` is defined in `character/types.rs` and re-exported from `crate`).

#### 4. Refactor start_respawn_timer to consume DeathEvent
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

Replace `start_respawn_timer` (lines 133–157):

```rust
fn start_respawn_timer(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    mut events: EventReader<DeathEvent>,
    query: Query<
        Option<&RespawnTimerConfig>,
        (Without<RespawnTimer>, Without<RespawnPoint>),
    >,
) {
    let tick = timeline.tick();
    for event in events.read() {
        let Ok(config) = query.get(event.entity) else {
            continue;
        };
        let duration = config
            .map(|c| c.duration_ticks)
            .unwrap_or(DEFAULT_RESPAWN_TICKS);
        commands.entity(event.entity).insert((
            RespawnTimer {
                expires_at: tick + duration as i16,
            },
            RigidBodyDisabled,
            ColliderDisabled,
        ));
    }
}
```

#### 5. Register event and update system ordering
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

In `GameplayPlugin::build` (around lines 19–35):

```rust
app.add_event::<DeathEvent>();
app.add_systems(FixedUpdate, (
    start_respawn_timer
        .after(hit_detection::process_projectile_hits)
        .after(hit_detection::process_hitbox_hits),
    process_respawn_timers.after(start_respawn_timer),
    expire_invulnerability,
));
```

No `emit_death_events` system needed — events are emitted directly at the damage site.

Add `DeathEvent` to imports from `protocol`.

#### 6. Re-export DeathEvent
**File**: `crates/protocol/src/lib.rs`
**Action**: modify

Add `DeathEvent` to the public re-exports (wherever `Health` is re-exported).

### Verification
#### Automated
- [ ] `cargo check-all` passes

#### Manual
- [ ] `cargo server` + `cargo client` — damage a character to death, confirm respawn behavior unchanged (teleport to respawn point, full health, invulnerability)
- [ ] Damage an already-dead entity (multiple hits same tick) — confirm only one `DeathEvent` fires (add temporary `info!` in `start_respawn_timer` to verify, then remove)

---

## Phase 2: Transform on Death (End-to-End)

### Changes

#### 1. New types: OnDeathEffects, DeathEffect, ActiveTransformation
**File**: `crates/protocol/src/world_object/types.rs`
**Action**: modify

Add after existing types (after `VisualKind` enum):

```rust
/// Describes effects triggered when this object dies. Defined in `.object.ron`.
#[derive(Component, Reflect, Deserialize, Clone, Debug)]
#[reflect(Component)]
pub struct OnDeathEffects(pub Vec<DeathEffect>);

/// A single effect applied on death.
#[derive(Reflect, Deserialize, Clone, Debug)]
pub enum DeathEffect {
    /// Replace this entity's components with those from another object def.
    TransformInto {
        source: String,
        revert_after_ticks: Option<u16>,
    },
}

/// Tracks an active transformation on a world object. Persisted across chunk eviction.
#[derive(Component, Reflect, Clone, Debug)]
#[reflect(Component)]
pub struct ActiveTransformation {
    pub source: String,
    pub ticks_remaining: Option<u16>,
}
```

Add `serde::Deserialize` to imports if not already present.

#### 2. Re-export new types
**File**: `crates/protocol/src/world_object/mod.rs`
**Action**: modify

Add to the `pub use types::` line:
`ActiveTransformation, DeathEffect, OnDeathEffects`

#### 3. Register types in WorldObjectPlugin
**File**: `crates/protocol/src/world_object/plugin.rs`
**Action**: modify

Add after existing `register_type` calls (line 56):

```rust
app.register_type::<super::types::OnDeathEffects>();
app.register_type::<super::types::DeathEffect>();
app.register_type::<super::types::ActiveTransformation>();
```

#### 4. Register VisualKind and ActiveTransformation with lightyear
**File**: `crates/protocol/src/lib.rs`
**Action**: modify

Add after `WorldObjectId` registration (~line 158):

```rust
app.register_component::<world_object::VisualKind>();
app.register_component::<world_object::ActiveTransformation>();
```

No `.add_prediction()` — these are replicated-only, server-authoritative.

#### 5. apply_transformation helper
**File**: `crates/server/src/world_object.rs`
**Action**: modify

Add new public function. This diffs the source def against the entity's current def: removes components present on entity's original def but absent from source, inserts/overwrites components from source.

```rust
/// Transforms an entity from its current def to a source def by diffing components.
///
/// Removes components present in `current_def` but absent from `source_def`.
/// Inserts/overwrites components from `source_def`.
/// Handles vox collider swap: if source has a Vox visual, builds trimesh; otherwise
/// removes the old collider and applies ColliderConstructor from source def if present.
pub fn apply_transformation(
    commands: &mut Commands,
    entity: Entity,
    current_def: &WorldObjectDef,
    source_def: &WorldObjectDef,
    type_registry: &AppTypeRegistry,
    vox_registry: &VoxModelRegistry,
    vox_assets: &Assets<VoxModelAsset>,
    meshes: &Assets<Mesh>,
) {
    let source_type_paths: HashSet<&str> = source_def
        .components
        .iter()
        .map(|c| c.reflect_type_path())
        .collect();

    let current_type_paths: HashSet<&str> = current_def
        .components
        .iter()
        .map(|c| c.reflect_type_path())
        .collect();

    // Remove components present on current but absent from source
    remove_absent_components(commands, entity, current_def, &source_type_paths, type_registry);

    // Apply source def components (collider-aware)
    let vox_collider = vox_trimesh_collider(source_def, vox_registry, vox_assets, meshes);
    let use_vox_collider = vox_collider.is_some();
    let components = clone_def_components(source_def, use_vox_collider);
    apply_object_components(commands, entity, components, type_registry.0.clone());

    if let Some(collider) = vox_collider {
        commands.entity(entity).insert(collider);
    }
}

/// Removes reflected components from `entity` that are in `current_def` but not in `keep_paths`.
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
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
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

Add `use std::collections::HashSet;` and `use bevy::ecs::reflect::ReflectComponent;` to imports.
Make `clone_def_components` and `vox_trimesh_collider` `pub(crate)` (currently private).

#### 6. on_death_effects system
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

Add new system:

```rust
/// Processes death effects for world objects that just died.
fn on_death_effects(
    mut commands: Commands,
    mut events: EventReader<DeathEvent>,
    effect_query: Query<(&OnDeathEffects, &WorldObjectId)>,
    defs: Res<WorldObjectDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
) {
    for event in events.read() {
        let Ok((effects, obj_id)) = effect_query.get(event.entity) else {
            continue;
        };
        for effect in &effects.0 {
            match effect {
                DeathEffect::TransformInto { source, revert_after_ticks } => {
                    let source_id = WorldObjectId(source.clone());
                    let Some(source_def) = defs.get(&source_id) else {
                        warn!("Unknown transformation source '{source}'");
                        continue;
                    };
                    let Some(current_def) = defs.get(obj_id) else {
                        warn!("Unknown current def '{}'", obj_id.0);
                        continue;
                    };
                    crate::world_object::apply_transformation(
                        &mut commands,
                        event.entity,
                        current_def,
                        source_def,
                        &type_registry,
                        &vox_registry,
                        &vox_assets,
                        &meshes,
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

Add imports: `OnDeathEffects`, `DeathEffect`, `ActiveTransformation`, `WorldObjectId`, `WorldObjectDefRegistry`, `VoxModelAsset`, `VoxModelRegistry`.

#### 7. Filter OnDeathEffects entities from respawn timer
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

In `start_respawn_timer`, add `Without<OnDeathEffects>` to the query filter so world objects with death effects skip the respawn path. Note: since we changed to `EventReader<DeathEvent>`, we need to filter inside the loop:

```rust
fn start_respawn_timer(
    mut commands: Commands,
    timeline: Res<LocalTimeline>,
    mut events: EventReader<DeathEvent>,
    query: Query<
        (Option<&RespawnTimerConfig>, Has<OnDeathEffects>),
        (Without<RespawnTimer>, Without<RespawnPoint>),
    >,
) {
    let tick = timeline.tick();
    for event in events.read() {
        let Ok((config, has_death_effects)) = query.get(event.entity) else {
            continue;
        };
        if has_death_effects {
            continue;
        }
        let duration = config
            .map(|c| c.duration_ticks)
            .unwrap_or(DEFAULT_RESPAWN_TICKS);
        commands.entity(event.entity).insert((
            RespawnTimer {
                expires_at: tick + duration as i16,
            },
            RigidBodyDisabled,
            ColliderDisabled,
        ));
    }
}
```

#### 8. System ordering for on_death_effects
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

Add to `FixedUpdate` schedule:

```rust
on_death_effects
    .after(hit_detection::process_projectile_hits)
    .after(hit_detection::process_hitbox_hits),
```

`on_death_effects` reads `EventReader<DeathEvent>`, so it must run after the systems that emit those events. `start_respawn_timer` must also run after `on_death_effects` (since Phase 2 step 7 adds the `Has<OnDeathEffects>` filter there).

#### 9. Client: on_visual_kind_changed system
**File**: `crates/client/src/world_object.rs`
**Action**: modify

Add system that rebuilds visuals when `VisualKind` changes on a replicated entity. Move mesh components to parent entity (currently on a child).

```rust
/// Rebuilds visuals when VisualKind changes via replication (e.g. tree→stump transformation).
fn on_visual_kind_changed(
    mut commands: Commands,
    query: Query<(Entity, &VisualKind), Changed<VisualKind>>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
    default_material: Res<DefaultVoxModelMaterial>,
    children_query: Query<&Children>,
) {
    for (entity, visual) in &query {
        // Despawn old visual children
        if let Ok(children) = children_query.get(entity) {
            for &child in children.iter() {
                commands.entity(child).despawn();
            }
        }
        // Rebuild visual
        attach_visual(&mut commands, entity, visual, &vox_registry, &vox_assets, &meshes, &default_material);
    }
}
```

Register this system in the client's world object plugin/setup (wherever `on_world_object_replicated` is registered). Run condition: after replication.

#### 10. Stump object def
**File**: `assets/objects/stump_circle.object.ron`
**Action**: create

```ron
{
    "protocol::world_object::types::ObjectCategory": Scenery,
    "protocol::world_object::types::VisualKind": Vox("models/trees/tree_circle.vox"),
    "avian3d::collision::collider::constructor::ColliderConstructor": Cylinder(radius: 0.5, height: 1.0),
    "avian3d::dynamics::rigid_body::RigidBody": Static,
    "avian3d::collision::collider::layers::CollisionLayers": (memberships: (32), filters: (14)),
}
```

Note: Uses the same vox model as a placeholder until a stump model exists. No `Health` — transformed stumps shouldn't be damageable. No `PlacementOffset` — position is inherited from the tree. Shorter collider to represent a stump.

#### 11. Add OnDeathEffects to tree_circle.object.ron
**File**: `assets/objects/tree_circle.object.ron`
**Action**: modify

Add entry:

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
- [ ] `cargo check-all` passes

#### Manual
- [ ] `cargo server` + `cargo client` — spawn near trees, damage a tree to death
- [ ] Observe: tree visuals change (stump appears in same position, no pop or gap)
- [ ] Observe: stump has no health component (not damageable)
- [ ] Observe: characters still respawn normally (teleport, heal, invulnerability)
- [ ] Observe: `ActiveTransformation` component is present on the stump entity (debug log or inspector)

---

## Phase 3: Revert After Delay

### Changes

#### 1. tick_active_transformations system
**File**: `crates/server/src/gameplay.rs`
**Action**: modify

Add new system:

```rust
/// Decrements active transformation timers. Triggers revert when countdown reaches zero.
fn tick_active_transformations(
    mut commands: Commands,
    mut query: Query<(Entity, &mut ActiveTransformation, &WorldObjectId)>,
    defs: Res<WorldObjectDefRegistry>,
    type_registry: Res<AppTypeRegistry>,
    vox_registry: Res<VoxModelRegistry>,
    vox_assets: Res<Assets<VoxModelAsset>>,
    meshes: Res<Assets<Mesh>>,
) {
    for (entity, mut transform, obj_id) in &mut query {
        let Some(ref mut remaining) = transform.ticks_remaining else {
            continue; // Permanent transformation, no revert
        };
        *remaining = remaining.saturating_sub(1);
        if *remaining > 0 {
            continue;
        }

        // Revert: apply original def, remove ActiveTransformation
        let source_id = WorldObjectId(transform.source.clone());
        let Some(source_def) = defs.get(&source_id) else {
            warn!("Cannot revert: unknown source def '{}'", transform.source);
            continue;
        };
        let Some(original_def) = defs.get(obj_id) else {
            warn!("Cannot revert: unknown original def '{}'", obj_id.0);
            continue;
        };

        crate::world_object::apply_transformation(
            &mut commands,
            entity,
            source_def,     // current state is the source
            original_def,   // target is the original
            &type_registry,
            &vox_registry,
            &vox_assets,
            &meshes,
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

No strict ordering needed — runs each tick independently.

### Verification
#### Automated
- [ ] `cargo check-all` passes

#### Manual
- [ ] `cargo server` + `cargo client` — damage tree to death, observe stump
- [ ] Wait for `revert_after_ticks` (1000 ticks ≈ ~16 seconds at 60Hz) — observe tree reappears
- [ ] Confirm tree has full health after revert
- [ ] Damage tree again — full cycle repeats (tree→stump→tree→stump)
- [ ] Confirm characters are unaffected by this system

---

## Phase 4: Persistence Across Eviction

### Changes

#### 1. ReflectPersist and ReflectSpawnOnly type data markers
**File**: `crates/protocol/src/world_object/types.rs`
**Action**: modify

Add after `ActiveTransformation`:

```rust
/// Reflect type data: marks a component for serialization during chunk eviction.
#[derive(Clone)]
pub struct ReflectPersist;

impl<T: Reflect> bevy::reflect::FromType<T> for ReflectPersist {
    fn from_type() -> Self {
        ReflectPersist
    }
}

/// Reflect type data: marks a component as spawn-only (skipped on reload).
#[derive(Clone)]
pub struct ReflectSpawnOnly;

impl<T: Reflect> bevy::reflect::FromType<T> for ReflectSpawnOnly {
    fn from_type() -> Self {
        ReflectSpawnOnly
    }
}
```

#### 2. Apply markers to components
**File**: `crates/protocol/src/world_object/types.rs`
**Action**: modify

On `ActiveTransformation`:
```rust
#[reflect(Component, Persist)]
```

On `PlacementOffset`:
```rust
#[reflect(Component, SpawnOnly)]
```

**File**: `crates/protocol/src/character/types.rs`
**Action**: modify

On `Health`:
```rust
#[reflect(Component, Default, Persist)]
```

This requires importing `ReflectPersist` (via `use crate::world_object::ReflectPersist;`) or registering it so reflect can find it. The `#[reflect(Persist)]` attribute looks up `ReflectPersist` in the type data registry — it must be registered.

#### 3. Register type data markers in WorldObjectPlugin
**File**: `crates/protocol/src/world_object/plugin.rs`
**Action**: modify

Add:
```rust
app.register_type_data::<super::types::ActiveTransformation, super::types::ReflectPersist>();
app.register_type_data::<super::types::PlacementOffset, super::types::ReflectSpawnOnly>();
app.register_type_data::<crate::Health, super::types::ReflectPersist>();
```

Note: `#[reflect(Persist)]` on the derive should handle this automatically if `ReflectPersist` implements `FromType<T>`. If the derive doesn't find it, fall back to explicit `register_type_data` calls.

#### 4. Expand WorldObjectSpawn with persisted components
**File**: `crates/voxel_map_engine/src/config.rs`
**Action**: modify

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldObjectSpawn {
    pub object_id: String,
    pub position: Vec3,
    /// RON-serialized persisted components. Empty for fresh spawns.
    #[serde(default)]
    pub persisted_components: Vec<PersistedComponent>,
}

/// A single persisted component: type path + RON data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersistedComponent {
    pub type_path: String,
    pub ron_data: String,
}
```

`#[serde(default)]` ensures backward compatibility — old save files without this field deserialize as empty vec.

Bump `ENTITY_SAVE_VERSION` in `crates/voxel_map_engine/src/persistence.rs` from `1` to `2`.

#### 5. Serialize ReflectPersist components during eviction
**File**: `crates/server/src/chunk_entities.rs`
**Action**: modify

Change `evict_chunk_entities` signature to access `World` (or use `&AppTypeRegistry` + component access). The system needs to iterate the entity's components, check for `ReflectPersist` type data, and serialize matching components to RON.

```rust
pub fn evict_chunk_entities(
    mut commands: Commands,
    entity_query: Query<(Entity, &ChunkEntityRef, &WorldObjectId, &Position)>,
    map_query: Query<(&VoxelMapInstance, &VoxelMapConfig)>,
    type_registry: Res<AppTypeRegistry>,
    world: &World,
) {
    let registry = type_registry.read();
    // ... existing grouping logic ...

    // For each entity being evicted, scan for ReflectPersist components
    for (entity, chunk_ref, obj_id, pos) in &entity_query {
        // ... existing eviction check ...

        let persisted = serialize_persisted_components(entity, world, &registry);

        by_chunk
            .entry((chunk_ref.map_entity, chunk_ref.chunk_pos))
            .or_default()
            .push((
                entity,
                WorldObjectSpawn {
                    object_id: obj_id.0.clone(),
                    position: Vec3::from(pos.0),
                    persisted_components: persisted,
                },
            ));
    }
    // ... rest unchanged ...
}

/// Serializes all components with ReflectPersist type data to RON.
fn serialize_persisted_components(
    entity: Entity,
    world: &World,
    registry: &bevy::reflect::TypeRegistry,
) -> Vec<PersistedComponent> {
    let mut result = Vec::new();
    let entity_ref = world.entity(entity);

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
                warn!("Failed to serialize persisted component '{}': {e}", registration.type_info().type_path());
            }
        }
    }
    result
}
```

Note: `evict_chunk_entities` currently takes `Query` params. Accessing raw `&World` alongside `Query` requires an exclusive system (`&mut World`) or a `SystemParam` that doesn't conflict. Alternative: use `entity_query` to get entity IDs, then use the type registry to reflect-read components. This may require refactoring to an exclusive system or using `Commands` with a deferred world access. Investigate the cleanest pattern — may need `world.entity(entity).get::<T>()` or `ReflectComponent::reflect(entity_ref)` which works with `EntityRef`.

Actually, `&World` conflicts with `Query` borrows. Use a system param approach:
- Keep `Query` params for the main logic
- After grouping entities, use a deferred `commands.queue(|world: &mut World| { ... })` to do the serialization before despawn
- Or refactor to an exclusive system

The simplest approach: make `evict_chunk_entities` an exclusive system that takes `&mut World` directly and does manual queries.

#### 6. Restore persisted components on reload
**File**: `crates/server/src/chunk_entities.rs`
**Action**: modify

In `spawn_chunk_entities`, after spawning and applying def components, check for persisted components:

```rust
// After apply_object_components and position insertion:
if !spawn.persisted_components.is_empty() {
    restore_persisted_components(
        &mut commands,
        entity,
        &spawn.persisted_components,
        &type_registry,
    );

    // If ActiveTransformation is persisted, apply source def instead of base def
    if let Some(active) = find_persisted::<ActiveTransformation>(&spawn.persisted_components, &type_registry) {
        let source_id = WorldObjectId(active.source.clone());
        if let Some(source_def) = defs.get(&source_id) {
            crate::world_object::apply_transformation(
                &mut commands, entity, def, source_def,
                &type_registry, &vox_registry, &vox_assets, &meshes,
            );
        }
    }
}
```

```rust
/// Deserializes and inserts persisted components onto an entity.
fn restore_persisted_components(
    commands: &mut Commands,
    entity: Entity,
    persisted: &[PersistedComponent],
    type_registry: &AppTypeRegistry,
) {
    let registry = type_registry.read();
    for pc in persisted {
        let Some(registration) = registry.get_with_type_path(&pc.type_path) else {
            warn!("Unknown persisted type '{}'", pc.type_path);
            continue;
        };
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            warn!("Persisted type '{}' missing ReflectComponent", pc.type_path);
            continue;
        };
        let deserializer = ron::Deserializer::from_str(&pc.ron_data);
        match deserializer {
            Ok(mut de) => {
                let type_deserializer = bevy::reflect::serde::TypedReflectDeserializer::new(registration, &registry);
                match type_deserializer.deserialize(&mut de) {
                    Ok(value) => {
                        let registry_arc = type_registry.0.clone();
                        let value_owned = value;
                        commands.queue(move |world: &mut World| {
                            let registry = registry_arc.read();
                            if let Some(registration) = registry.get_with_type_path(value_owned.reflect_type_path()) {
                                if let Some(reflect_component) = registration.data::<ReflectComponent>() {
                                    if let Some(mut entity_mut) = world.get_entity_mut(entity) {
                                        reflect_component.insert(&mut entity_mut, value_owned.as_ref(), &registry);
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => warn!("Failed to deserialize persisted '{}': {e}", pc.type_path),
                }
            }
            Err(e) => warn!("Invalid RON for persisted '{}': {e}", pc.type_path),
        }
    }
}
```

#### 7. Skip PlacementOffset on reload (ReflectSpawnOnly)
**File**: `crates/server/src/chunk_entities.rs`
**Action**: modify

Modify `extract_placement_offset` to skip when persisted components are non-empty (indicating reload):

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

Update call site in `spawn_chunk_entities`:
```rust
let is_reload = !spawn.persisted_components.is_empty();
let offset = extract_placement_offset(def, is_reload);
```

#### 8. Update save_all_chunk_entities_on_exit
**File**: `crates/server/src/chunk_entities.rs`
**Action**: modify

The shutdown save also needs to capture persisted components, same as eviction. Apply the same `serialize_persisted_components` logic.

### Verification
#### Automated
- [ ] `cargo check-all` passes

#### Manual
- [ ] `cargo server` + `cargo client` — damage tree to death, observe stump
- [ ] Move far enough to trigger chunk eviction (walk away until chunk unloads)
- [ ] Return to the chunk — stump reloads with remaining countdown
- [ ] Wait for countdown to expire — tree reverts with full health
- [ ] Verify fresh trees spawn at correct position (PlacementOffset applied once)
- [ ] Verify reloaded trees don't double-offset (position matches pre-eviction position)
- [ ] Restart server — verify entities load correctly from disk (shutdown save works)
