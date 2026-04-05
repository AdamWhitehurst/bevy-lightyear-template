# Structure Outline

## Approach
In-place entity mutation on death: swap components to match a source def (e.g. tree→stump), revert after a tick countdown. All death handling — including character respawn — flows through `DeathEvent` → `OnDeathEffects` dispatch. Persist transformation state across chunk eviction via `ReflectPersist`. Fix `PlacementOffset` double-apply via `ReflectSpawnOnly`.

## Phase 1: Unified Death Handling

Replace polling-based `start_respawn_timer` with event-driven `DeathEvent` → `on_death_effects` dispatch. Character respawn becomes `StartRespawnPointTimer` variant of `DeathEffect`, applied programmatically at character spawn. No new visual behavior — existing respawn flow preserved through the new path.

**Files**: `crates/protocol/src/character/types.rs`, `crates/protocol/src/hit_detection/effects.rs`, `crates/protocol/src/world_object/types.rs`, `crates/protocol/src/world_object/plugin.rs`, `crates/server/src/gameplay.rs`
**Key changes**:
- `DeathEvent { pub entity: Entity }` — new `Event`
- `Health::apply_damage(&mut self, damage: f32) -> bool` — returns `true` when transitioning alive→dead (was `> 0.0`, now `<= 0.0`)
- `apply_on_hit_effects` (`hit_detection/effects.rs`) — when `apply_damage` returns `true`, send `DeathEvent` via `EventWriter<DeathEvent>`. Both damage paths (`process_hitbox_hits`, `process_projectile_hits`) funnel through this fn, so emission is in one place
- `OnDeathEffects(pub Vec<DeathEffect>)` — new `Component`, `Reflect`, `Deserialize`
- `DeathEffect` enum — `StartRespawnPointTimer { duration_ticks: Option<u16> }` variant (+ `TransformInto` variant stubbed but not handled yet)
- `on_death_effects(events: EventReader<DeathEvent>, query: Query<(&OnDeathEffects, ...)>, ...)` — new system, dispatches `DeathEffect` variants. `StartRespawnPointTimer`: inserts `RespawnTimer` + `RigidBodyDisabled` + `ColliderDisabled` (same logic as old `start_respawn_timer`)
- Character spawn site — insert `OnDeathEffects(vec![StartRespawnPointTimer { duration_ticks: None }])` on characters programmatically
- Delete `start_respawn_timer` system entirely
- Register `OnDeathEffects` in `WorldObjectPlugin` type registry
- System ordering: `on_death_effects.after(process_projectile_hits)`, `process_respawn_timers.after(on_death_effects)`

**Verify**: `cargo check-all` passes. `cargo server` + `cargo client` — damage a character to death, confirm respawn timer + teleport to RespawnPoint still works identically. World objects with `Health` but no `OnDeathEffects` should have no death behavior (previously they'd get respawn timer — confirm this is acceptable or add fallback).

---

## Phase 2: Transform on Death (End-to-End)

Tree dies → stump appears in-place. Implements `TransformInto` handler, transformation diff logic, lightyear registration for changed components, and client visual reconstruction via `Changed<VisualKind>`.

**Files**: `crates/protocol/src/world_object/types.rs`, `crates/protocol/src/world_object/spawn.rs`, `crates/protocol/src/world_object/plugin.rs`, `crates/protocol/src/lib.rs`, `crates/server/src/gameplay.rs`, `crates/server/src/world_object.rs`, `crates/client/src/world_object.rs`, `assets/objects/tree_circle.object.ron` (add `OnDeathEffects`), new `assets/objects/stump_circle.object.ron`

**Key changes**:
- `ActiveTransformation { source: String, ticks_remaining: Option<u16> }` — new `Component`, `Reflect`
- `on_death_effects` — add `TransformInto` match arm: load source def, diff components (remove absent, apply/overwrite present), insert `ActiveTransformation`
- `apply_transformation(entity, source_def, current_def, commands, type_registry)` — new fn in `server/world_object.rs`. Diffs source def components against current def: removes components present on entity but absent from source, inserts/overwrites components from source
- Register `ActiveTransformation`, `VisualKind` with lightyear (replicated, no prediction)
- Register `ActiveTransformation` in `WorldObjectPlugin` type registry
- Client: `on_visual_kind_changed(query: Query<(Entity, &VisualKind), Changed<VisualKind>>)` — new system, rebuilds mesh on the entity. Move `Mesh3d`+`MeshMaterial3d` from child to parent entity (simplifies overwrite)
- Client: `on_world_object_replicated` — updated to use parent-entity visuals (align with `on_visual_kind_changed`)

**Verify**: `cargo check-all` passes. `cargo server` + `cargo client` — spawn near trees, damage a tree to death, observe stump appears in place (no pop, no gap). Confirm characters still respawn normally via `StartRespawnPointTimer`.

---

## Phase 3: Revert After Delay

Stump reverts to tree after configured ticks. Completes the transformation lifecycle.

**Files**: `crates/server/src/gameplay.rs` (or new `crates/server/src/transformation.rs`), `crates/server/src/world_object.rs`

**Key changes**:
- `tick_active_transformations(mut query: Query<(Entity, &mut ActiveTransformation)>)` — new `FixedUpdate` system. Decrements `ticks_remaining`. When zero: triggers revert
- `revert_transformation(entity, original_def, source_def, commands, type_registry)` — reuses same diff logic as `apply_transformation` but in reverse direction (original def is now the "target")
- On revert: re-apply original def components from `WorldObjectId`, remove `ActiveTransformation`, restore `Health` to full

**Verify**: `cargo server` + `cargo client` — damage tree to death, observe stump, wait for `revert_after_ticks` duration, observe tree reappears with full health. Damage again — full cycle repeats.

---

## Phase 4: Persistence Across Eviction

Transformation state survives chunk unload/reload. Also fixes `PlacementOffset` double-apply bug.

**Files**: `crates/protocol/src/world_object/types.rs`, `crates/protocol/src/world_object/spawn.rs`, `crates/protocol/src/world_object/plugin.rs`, `crates/server/src/chunk_entities.rs`, `crates/voxel_map_engine/src/config.rs`

**Key changes**:
- `ReflectPersist` — new type data marker, `impl FromType<T> for ReflectPersist` (trivial — presence is the signal)
- `ReflectSpawnOnly` — new type data marker, same pattern
- `#[reflect(Persist)]` on `ActiveTransformation`, `Health`
- `#[reflect(SpawnOnly)]` on `PlacementOffset`
- `WorldObjectSpawn` — add `persisted_components: Vec<(String, Vec<u8>)>` field (type path + RON-serialized bytes). Default empty for fresh spawns
- `evict_chunk_entities` — scan entity's components for `ReflectPersist` type data, serialize each into `persisted_components`
- `spawn_chunk_entities` — if `persisted_components` is non-empty: restore persisted components after base spawn. If `ActiveTransformation` is among them: apply source def instead of base def (skip wasted apply-then-overwrite)
- `extract_placement_offset` — check `ReflectSpawnOnly`; skip if `persisted_components` is non-empty (i.e. this is a reload)
- `from_disk` flag: thread through `WorldObjectSpawn` or infer from `persisted_components.is_empty()`

**Verify**: `cargo server` + `cargo client` — damage tree, observe stump. Move far enough to trigger chunk eviction. Return to chunk — stump reloads with remaining countdown, eventually reverts. Also verify fresh trees spawn with correct `PlacementOffset` and reloaded trees don't double-offset.

---

## Testing Checkpoints

| After Phase | What should be true |
|---|---|
| 1 | `DeathEvent` fires on alive→dead. Characters respawn via `StartRespawnPointTimer` through `on_death_effects`. Old `start_respawn_timer` deleted. `process_respawn_timers` unchanged. |
| 2 | Tree with `OnDeathEffects([TransformInto { .. }])` transforms into stump on death. Client sees stump via `Changed<VisualKind>`. Characters unaffected. |
| 3 | Stump reverts to tree after `revert_after_ticks`. Full health restored. Cycle repeatable. |
| 4 | Stump persists across eviction/reload with remaining countdown. `PlacementOffset` no longer double-applies. |
