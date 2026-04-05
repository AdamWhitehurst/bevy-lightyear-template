# Design Discussion

## Current State

- Death detection is polling-based: `start_respawn_timer` (`server/src/gameplay.rs:133`) checks `health.is_dead()` each `FixedUpdate`. No events or observers.
- On death: `RespawnTimer`, `RigidBodyDisabled`, `ColliderDisabled` inserted. Entity persists. On timer expiry: health restored, physics re-enabled, `Invulnerable` granted (`gameplay.rs:160-203`).
- World objects reuse the character respawn system. No world-object-specific death behavior.
- Eviction saves only `WorldObjectSpawn { object_id, position }` (`chunk_entities.rs:96-148`). All runtime state discarded.
- Reload uses the same `spawn_chunk_entities` path as fresh spawn — no distinction (`chunk_entities.rs:20-73`).
- `PlacementOffset` double-applies on reload (bug): eviction saves the already-offset position, reload re-applies the offset (`chunk_entities.rs:62,119`).
- No `Changed<WorldObjectId>` or `Changed<VisualKind>` systems exist. Client reconstructs visuals only on `Added<Replicated>` (`client/src/world_object.rs:30-65`).
- Component filtering during spawn exists only for `ColliderConstructor` via a boolean flag in `clone_def_components` (`server/src/world_object.rs:57-72`).

## Desired End State

A tree defined in `tree_circle.object.ron` with `OnDeathEffects` can:
1. Die (health reaches zero)
2. Transform in-place into a stump (visual, collider, remove health) while keeping `WorldObjectId("tree_circle.object.ron")`
3. After a configured delay, revert to a tree with full health
4. Survive chunk eviction and reload at any point in this lifecycle
5. All state changes replicate correctly to clients with no gap frames

Verification: spawn a tree, damage it to death, observe stump appears (no pop), wait for revert, observe tree reappears. Evict chunk mid-stump, reload, observe stump with correct countdown. Run `cargo server` + `cargo client`.

## Patterns to Follow

**Effect enum dispatch** (`protocol/src/ability/types.rs:47`): `AbilityEffect` enum with match-based dispatch. `OnDeathEffects` should follow the same enum-of-effects pattern with a `DeathEffect` enum.

**RON component definitions** (`protocol/src/world_object/loader.rs:18`): components in `.object.ron` are deserialized via `TypeRegistry` + `TypedReflectDeserializer`. New components need `#[derive(Reflect)]`, `#[reflect(Component)]`, `Deserialize`, registered in `WorldObjectPlugin` (`plugin.rs:50-55`).

**Reflect type data markers**: no custom type data markers exist yet, but `ReflectComponent` and `ReflectFromReflect` are used throughout. `ReflectPersist` and `ReflectSpawnOnly` follow the same `FromType<T>` pattern.

**Observer for auto-wiring** (`server/src/map.rs:414`): `on_map_instance_id_added` fires on `Add<MapInstanceId>` and handles room assignment. In-place mutation preserves `MapInstanceId` so no room re-assignment needed.

**Lightyear atomicity** (research Q11): component adds/removes on a live entity are batched into one `EntityActions` and applied atomically on the client. In-place mutation avoids the cross-group gap frame risk of despawn+respawn.

### Patterns to NOT Follow

**`RespawnTimer` for delayed effects**: the current respawn system (`gameplay.rs:160-203`) has character-specific logic (teleport to respawn point) interleaved with generic logic. Using `RespawnTimer` for transformation revert would further couple these concerns. Use a dedicated `ActiveTransformation` component instead.

## Design Decisions

1. **In-place mutation, not despawn+respawn**: swap visual/collider/health components on the existing entity. Lightyear delivers all changes atomically within one `EntityActions`. No room re-assignment needed. Client reconstructs visuals on `Changed<VisualKind>` (new system) — this covers both transform and revert, and keeps client visuals decoupled from the transformation concept. `WorldObjectId` stays unchanged — the entity's identity is always the original object.

2. **`DeathEvent` emitted from `Health::apply_damage`**: when `apply_damage` transitions health to `<= 0.0` (alive→dead), it emits `DeathEvent`. This centralizes death detection at the causal point rather than discovering it later via polling. A single `on_death_effects` system consumes `DeathEvent` and dispatches all `DeathEffect` variants. Eliminates the old `start_respawn_timer` polling system entirely.

3. **`StartRespawnPointTimer` replaces `start_respawn_timer`**: the existing character respawn behavior (disable physics, start timer, teleport to `RespawnPoint` on expiry) becomes a `DeathEffect` variant. Characters get `OnDeathEffects([StartRespawnPointTimer { duration_ticks: None }])` applied programmatically at spawn (not from `.object.ron`). `process_respawn_timers` still handles timer expiry and teleport — only the death detection + timer insertion is unified through `DeathEvent`.

4. **`ReflectSpawnOnly` type data marker**: components marked `#[reflect(SpawnOnly)]` are only applied on first spawn, skipped on reload. `PlacementOffset` is the first consumer. Checked in `extract_placement_offset` and available for future spawn-time-only components. Requires threading a `from_disk` flag through `WorldObjectSpawn` → `spawn_chunk_entities`.

5. **`ReflectPersist` type data marker**: components marked `#[reflect(Persist)]` are serialized during eviction and restored on reload. Stored alongside `WorldObjectSpawn` in the chunk save file. `ActiveTransformation.ticks_remaining` is already relative — no tick conversion needed at save/load boundaries.

6. **`ActiveTransformation` over `RespawnTimer` for revert**: a persisted component that tracks what def is currently skinned over the entity and how many ticks remain until revert. Decoupled from the character respawn system. Ticked down by its own system.

7. **Source def is single source of truth for transformed state**: the transformation source's `.object.ron` defines exactly what the entity looks like while transformed. No special fields on `TransformInto` for health, collider, etc. — if stump def has no `Health`, `Health` is removed; if it has a different collider, it's swapped. The transformation system diffs current def components against source def components: remove absent, apply/overwrite present. Same logic for both transform and revert directions.

8. **On reload with `ActiveTransformation`: skip base def, apply transformation directly**: `spawn_chunk_entities` checks for a persisted `ActiveTransformation`. If present, applies the transformation source's def components instead of the entity's own `WorldObjectId` def components. Avoids a wasted apply-then-overwrite cycle.

## Component Design

```rust
/// Defined in .object.ron. Describes what happens when this object dies.
#[derive(Component, Reflect, Deserialize, Clone)]
#[reflect(Component)]
pub struct OnDeathEffects(pub Vec<DeathEffect>);

#[derive(Reflect, Deserialize, Clone)]
pub enum DeathEffect {
    /// Skin this entity with another object def's components.
    TransformInto {
        source: String,                  // e.g. "stump_circle.object.ron"
        revert_after_ticks: Option<u16>, // None = permanent
    },
    /// Disable physics, start respawn timer, teleport to nearest RespawnPoint on expiry.
    /// Replaces the old `start_respawn_timer` polling system. Applied programmatically
    /// to characters (not from .object.ron).
    StartRespawnPointTimer {
        duration_ticks: Option<u16>,     // None = use RespawnTimerConfig or DEFAULT_RESPAWN_TICKS
    },
    // Future: DropItems { loot_table: String },
}

/// Runtime state: tracks an active transformation. Persisted across eviction.
#[derive(Component, Reflect, Clone)]
#[reflect(Component, Persist)]
pub struct ActiveTransformation {
    pub source: String,
    pub ticks_remaining: Option<u16>,
}

/// Emitted when an entity's health reaches zero.
#[derive(Event)]
pub struct DeathEvent {
    pub entity: Entity,
}

/// Reflect type data: component is saved during chunk eviction.
#[derive(Clone)]
pub struct ReflectPersist;

/// Reflect type data: component is only applied on first spawn.
#[derive(Clone)]
pub struct ReflectSpawnOnly;
```

**Example RON** (`tree_circle.object.ron`):
```ron
{
    "game::world_object::OnDeathEffects": ([
        TransformInto(
            source: "stump_circle.object.ron",
            revert_after_ticks: Some(1000),
        ),
    ]),
    "game::Health": (max: 50.0, current: 50.0),
    "game::VisualKind": Vox("tree_circle.vox"),
    "game::PlacementOffset": ((0.0, 2.0, 0.0)),
}
```

## Data Flow

```
Health::apply_damage detects alive→dead → DeathEvent
    → on_death_effects system reads OnDeathEffects, dispatches each DeathEffect variant:
        → TransformInto: diff current def vs stump def — remove absent, apply/overwrite present, insert ActiveTransformation
        → StartRespawnPointTimer: insert RespawnTimer + RigidBodyDisabled + ColliderDisabled (same as old start_respawn_timer)
    → Lightyear replicates component changes atomically
    → Client Changed<VisualKind> rebuilds mesh (for TransformInto)

ActiveTransformation tick system decrements ticks_remaining
    → On zero: re-apply original def from WorldObjectId, restore health, remove ActiveTransformation
    → Lightyear replicates revert atomically
    → Client Changed<VisualKind> rebuilds mesh/collider

Eviction with ActiveTransformation:
    → Save WorldObjectSpawn + all ReflectPersist components (ActiveTransformation, Health)
    → On reload: spawn entity, detect persisted ActiveTransformation
    → Skip base def components, apply transformation source's def directly
    → Continue countdown from persisted ticks_remaining
```

## What We're NOT Doing

- **Loot drops / item spawning**: `DropItems` variant is stubbed but not implemented. Deferred to a future task.
- **Client prediction of death effects**: server-authoritative only. Client sees results via replication.
- **Animation/particle transitions**: stump appears instantly. Visual polish is a separate task.
- **Nested or chained transformations**: one active transformation at a time. No stump→smaller-stump chains.
- **Custom per-object respawn logic beyond what `DeathEffect` variants provide**: no per-entity callback hooks or scripting.

## Open Risks

1. **Component cleanup during transformation**: resolved — the source def is the single source of truth. Transformation diffs the entity's current def components against the source def: removes components present on entity but absent from source, applies/overwrites components from source. Same diff logic handles both transform (tree→stump) and revert (stump→tree). If stump def has no `Health`, `Health` is removed. If it has a different collider, the collider is swapped.

2. **`from_disk` flag plumbing**: `WorldObjectSpawn` lives in `voxel_map_engine` which can't depend on `protocol`. Persisted component data must be serialized generically (reflect-based) and stored alongside the spawn, possibly as a separate `Vec<u8>` or companion file.

3. **Client visual reconstruction**: the client currently places `Mesh3d` + `MeshMaterial3d` on a child entity (`client/src/world_object.rs:160-163`), but there's no technical reason for this — move them onto the parent entity directly. This simplifies transformation: `Changed<VisualKind>` just overwrites `Mesh3d` on the parent instead of managing child entity lifecycle.

4. ~~**Timer precision across eviction**~~: resolved — `ActiveTransformation.ticks_remaining` is already relative (decremented each tick by its own system). No absolute↔relative conversion needed. This risk only applied to the existing `RespawnTimer` pattern (absolute `expires_at: Tick`), which we're not using.
