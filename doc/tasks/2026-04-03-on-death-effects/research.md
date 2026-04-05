# Research Findings

## Q1: Damage Pipeline — Full Health to Dead

### Findings
- Two hit detection paths: `process_hitbox_hits` (`crates/protocol/src/hit_detection/systems.rs:33`) and `process_projectile_hits` (`:107`). Both call `apply_on_hit_effects`.
- `apply_on_hit_effects` (`crates/protocol/src/hit_detection/effects.rs:61`) iterates `on_hit.effects` vec. For `AbilityEffect::Damage`: resolves target, applies damage buffs from `ActiveBuffs`, checks `ActiveShield` absorption, then calls `health.apply_damage(amount)`.
- `Health::apply_damage` (`crates/protocol/src/character/types.rs:53`): `self.current = (self.current - damage).max(0.0)`.
- Death detection is **polling**: `start_respawn_timer` (`crates/server/src/gameplay.rs:133`) runs every `FixedUpdate`, queries entities with `Health` but no `RespawnTimer`, checks `health.is_dead()` (`types.rs:57`: `self.current <= 0.0`). No observers or events for death.

## Q2: Post-Death Behavior and RespawnTimerConfig

### Findings
- On death (`gameplay.rs:149-156`): inserts `RespawnTimer { expires_at }`, `RigidBodyDisabled`, `ColliderDisabled`. Entity **persists** — no despawn, no separate `Dead` marker.
- `RespawnTimerConfig` is an optional per-entity component. If absent, `DEFAULT_RESPAWN_TICKS = 256` (`types.rs:73`).
- `RespawnTimerConfig` is registered in `WorldObjectPlugin` (`plugin.rs:51`) — world objects can set it via `.object.ron`.
- Respawn execution (`gameplay.rs:160-203`): when timer expires, characters teleport to nearest `RespawnPoint`; world objects stay at current position. `health.restore_full()`, removes timer + physics disablers, inserts `Invulnerable { expires_at: tick + 128 }`.

## Q3: World Object Entity Spawning

### Findings
- `spawn_world_object` (`crates/server/src/world_object.rs:21`): spawns `(WorldObjectId, Rotation::default(), MapInstanceId, Replicate::to_clients(NetworkTarget::All))`.
- Then `clone_def_components` + `apply_object_components` inserts all reflected components from `WorldObjectDef`.
- Caller `spawn_chunk_entities` (`crates/server/src/chunk_entities.rs:52-69`) adds `Position` and `ChunkEntityRef { chunk_pos, map_entity }`.
- `MapInstanceId` triggers observer `on_map_instance_id_added` (`crates/server/src/map.rs:414`) which handles room assignment + `NetworkVisibility`.
- **Replicated**: `WorldObjectId`, `Position`, `MapInstanceId`, any registered def components. **Server-only**: `ChunkEntityRef`, `Replicate`, `NetworkVisibility`.

## Q4: WorldObjectDef RON Deserialization Pipeline

### Findings
- `.object.ron` format: flat RON map `{ "full::TypePath": (data), ... }`.
- Loader: `WorldObjectLoader` (`crates/protocol/src/world_object/loader.rs:18`) captures `AppTypeRegistry` via `FromWorld`.
- Load path: bytes -> `reflect_loader::deserialize_component_map` (`reflect_loader.rs:56`) -> `ComponentMapDeserializer` -> per-entry: key via `TypeRegistrationDeserializer` (lookup by type path), value via `TypedReflectDeserializer`, optional upgrade via `ReflectFromReflect`.
- Result: `WorldObjectDef { components: Vec<Box<dyn PartialReflect>> }`.
- Insertion: `apply_object_components` (`crates/protocol/src/world_object/spawn.rs:8`) -> `insert_reflected_component` (`:23`) -> `ReflectComponent::insert`.
- **Requirements for new component**: `#[derive(Reflect)]`, `#[reflect(Component)]`, `Deserialize` (or reflect-based deser), `Clone`, registered via `app.register_type::<T>()` in `WorldObjectPlugin` (`plugin.rs:50-55`).

## Q5: Lightyear Component Registration

### Findings
- `register_component::<T>()` (`lightyear_replication/src/registry/registry.rs:426`): assigns `ComponentNetId`, registers serde fns, sets `predicted: false`.
- `.add_prediction()` (`lightyear_prediction/src/registry.rs:350`): registers `PredictionHistory<C>`, `PredictionMetadata`, rollback systems, sets `predicted: true`.
- Replicated-only: component inserted directly on client entity. Prediction-enabled: wrapped in `Confirmed<C>`, rollback via `PredictionHistory<C>`.

**All registrations** (`crates/protocol/src/lib.rs:154-202`):

| Component | Prediction | Extra |
|---|---|---|
| `MapInstanceId` | No | |
| `WorldObjectId` | No | |
| `PlayerId` | No | |
| `ColorComponent` | Yes | |
| `Name` | No | |
| `CharacterMarker` | Yes | |
| `DummyTarget` | Yes | |
| `CharacterType` | Yes | |
| `Health` | Yes | |
| `Invulnerable` | Yes | |
| `RespawnTimerConfig` | No | |
| `RespawnTimer` | Yes | |
| `LinearVelocity` | Yes | custom `should_rollback` |
| `AngularVelocity` | Yes | custom `should_rollback` |
| `AbilitySlots` | No | |
| `ActiveAbility` | Yes | `.add_map_entities()` |
| `AbilityCooldowns` | Yes | |
| `ActiveShield` | Yes | |
| `ActiveBuffs` | Yes | |
| `AbilityProjectileSpawn` | No | |
| `Position` | Yes | custom rollback, linear correction, interpolation |
| `Rotation` | Yes | custom rollback, linear correction, interpolation |

## Q6: Tick-Based Timers

### Findings
- `RespawnTimer { expires_at: Tick }` (`crates/protocol/src/character/types.rs:79-96`) stores an absolute tick.
- Current tick via `timeline: Res<LocalTimeline>` -> `timeline.tick()` (used at `gameplay.rs:141,176,225`).
- Timer start (`gameplay.rs:149`): `expires_at: tick + duration as i16`.
- Expiry check (`gameplay.rs:178`): `tick < timer.expires_at` -> continue; else respawn.
- Ordering chain (`gameplay.rs:26-33`): `process_projectile_hits -> start_respawn_timer -> process_respawn_timers`. All in `FixedUpdate`.
- **Saving/reloading**: `RespawnTimer` is not persisted. Eviction saves only `object_id` + `position`. Dead entities reload as alive with full health from their `.object.ron` definition. 
- Timers that would be Persisted would need to be relative e.g. `RespawnTimer { ticks_remaining: i16 }`

## Q7: Chunk-Entity Ownership

### Findings
- `ChunkEntityRef { chunk_pos: IVec3, map_entity: Entity }` (`crates/protocol/src/map/mod.rs:28-31`) — sole runtime link between entity and chunk. No ECS parent-child hierarchy.
- Inserted at spawn (`chunk_entities.rs:63-69`).
- On eviction (`chunk_entities.rs:96-148`): entity despawned. On reload: new entity gets fresh `ChunkEntityRef` with same `chunk_pos`. No entity continuity.

## Q8: Enum-Based Effect Dispatch

### Findings
- `AbilityEffect` enum (`crates/protocol/src/ability/types.rs:47`): `Melee`, `Projectile`, `SetVelocity`, `Damage`, `ApplyForce`, `AreaOfEffect`, `Ability`, `Teleport`, `Shield`, `Buff`.
- `EffectTrigger` enum (`types.rs:102`): wraps `AbilityEffect` with timing — `OnTick`, `WhileActive`, `OnHit`, `OnEnd`, `OnInput`.
- Triggers partition into per-trigger component vecs: `OnTickEffects`, `WhileActiveEffects`, `OnHitEffectDefs`, `OnEndEffects`, `OnInputEffects`.
- Each trigger has its own dispatch system with a `match` on `AbilityEffect` variants.
- Multiple effects: iterated in vec order within a single system invocation. No explicit sequencing primitive beyond vec ordering.
- Recursive composition: `AbilityEffect::Ability { id, target }` spawns sub-abilities via `spawn_sub_ability` (`spawn.rs:36`), capped at depth 4.

## Q9: Client-Side World Object Resolution

### Findings
- Trigger: `on_world_object_replicated` (`crates/client/src/world_object.rs:31`) — `Query<(Entity, &WorldObjectId), Added<Replicated>>`.
- Client receives only: `WorldObjectId`, `Rotation`, `MapInstanceId` via replication.
- Client reconstructs: all def components via `clone_def_components` + `apply_object_components`, optionally a trimesh collider, and a child mesh entity (`Mesh3d` + `MeshMaterial3d`).
- **Asymmetric**: server has no visual child; client has no `ChunkEntityRef`, `Replicate`, or `NetworkVisibility`.
- On despawn+respawn at same position: visual pops out and pops back in. No transition or continuity.

## Q10: Lightyear Child Entity Replication and Ordering

### Findings
- `HierarchySendPlugin` (`lightyear_replication/src/hierarchy.rs:68`): propagates `ReplicateLike` to descendants of `Replicate` entities. Parent+children buffered in same send pass.
- `ActionsMessage` groups all spawns/despawns/inserts/removes for a `ReplicationGroupId` into one blob. Within a group, despawns and spawns in the same tick land in the same message — no gap on client.
- **Across groups** (e.g. `DEFAULT_GROUP`): no ordering guarantee. Despawn of entity A and spawn of entity B could arrive in different frames — one-frame gap possible.
- Updates (`UpdatesMessage`, sequenced unreliable) are held until corresponding `ActionsMessage` tick is processed (`receive.rs:637-650`).

## Q11: Component Addition/Removal on Live Entities

### Findings
- Send side (`buffer.rs`): detects added/changed/removed components per tick via Bevy `ComponentTicks`. All packed into one `EntityActions`.
- Receive side (`receive.rs:979-1026`): within one `ActionsMessage`, all inserts and removes for an entity are applied via `batch_insert` + `remove_by_ids` atomically. No intermediate frame with partial state.
- Value mutations (not structural) go via `UpdatesMessage` (sequenced unreliable) — can be dropped, but next update overwrites. Ordered relative to structural changes via `last_action_tick`.

## Q12: Chunk Eviction and Persistence

### Findings
- `evict_chunk_entities` (`chunk_entities.rs:96-148`): saves only `WorldObjectSpawn { object_id: String, position: Vec3 }`. All other component state (Health, RespawnTimer, physics) discarded.
- No `ReflectPersist` or custom persistence trait exists. Only standard Bevy reflect machinery (`ReflectComponent`).
- Type registrations for reflect pipeline (`plugin.rs:50-55`): `Health`, `RespawnTimerConfig`, `ObjectCategory`, `VisualKind`, `ColliderConstructor`, `PlacementOffset`.
- Requirements for reflect pipeline: `#[derive(Reflect)]`, `#[reflect(Component)]`, `Deserialize`, registered in `AppTypeRegistry`.

## Q13: Entity Reload Flow

### Findings
- No separate reload code path. Same `spawn_chunk_entities` handles both fresh and reloaded spawns.
- Disk check in `spawn_features_task` (`generation.rs:121-131`): `load_chunk_entities` returns saved spawns if file exists, else `place_features` generates new ones.
- Loaded spawns flow through `PendingEntitySpawns` -> `spawn_chunk_entities` -> `spawn_world_object` + `apply_object_components`.
- All components reconstructed from `.object.ron` definition. No runtime state restored.
- `from_disk` flag (`generation.rs`) only gates `dirty_chunks` insertion — does not affect entity spawning behavior.

## Q14: PlacementOffset Application

### Findings
- `PlacementOffset(pub Vec3)` (`crates/protocol/src/world_object/types.rs:19`).
- Read at `extract_placement_offset` (`chunk_entities.rs:190-196`): scans def components, returns `o.0` or `Vec3::ZERO`.
- Applied at `chunk_entities.rs:62`: `position = Vec3::from(spawn.position) + offset`.
- Eviction saves the **already-offset** position (`chunk_entities.rs:119`: `position: Vec3::from(pos.0)`).
- On reload, `extract_placement_offset` is called again and added again. **PlacementOffset is double-applied on reload.**
- `PlacementOffset` is also inserted onto the entity as a component via `apply_object_components`, but nothing reads the entity component — extraction operates on the `WorldObjectDef`.

## Q15: Spawn-Time-Only vs. Reload-Safe Operations

### Findings
- **No distinction exists.** `spawn_chunk_entities` runs identically for fresh and reloaded spawns.
- `WorldObjectSpawn` (`voxel_map_engine/src/config.rs:34`) carries only `object_id: String` and `position: Vec3` — no fresh/reload flag.
- `apply_object_components` inserts everything unconditionally, no branch for first spawn vs reload.
- `PlacementOffset` is the concrete example of a spawn-time-only operation that incorrectly runs on reload.

## Q16: Map Instances and Rooms

### Findings
- `MapInstanceId` (`crates/protocol/src/map/types.rs:9-15`): enum with `Overworld` and `Homebase { owner: u64 }`.
- `RoomRegistry` (`crates/server/src/map.rs:42-53`): `HashMap<MapInstanceId, Entity>` — lazily creates lightyear `Room` entities.
- Observer `on_map_instance_id_added` (`map.rs:414-429`): fires on `Add<MapInstanceId>` for any entity. Calls `room_registry.get_or_create` + `RoomEvent::AddEntity` + inserts `NetworkVisibility`. Fully automatic.
- Client-to-room: `RoomTarget::AddSender(client_entity)` dispatched manually at character spawn (`gameplay.rs:293-297`) and map transition (`map.rs:969-972`).

## Q17: Mid-Game Entity Room Assignment

### Findings
- Any entity spawned with `MapInstanceId` in its bundle automatically gets room assignment via the `on_map_instance_id_added` observer. No manual room code needed at the spawn site.
- Contract: include `MapInstanceId` + `Replicate::to_clients(NetworkTarget::All)`.
- `spawn_world_object` (`world_object.rs:31-37`) follows this pattern exactly.
- For player clients specifically, `AddSender` must be manually triggered — entity-to-room is automatic, client-sender-to-room is not.

## Q18: Deterministic Randomness

### Findings
- Custom xorshift64 RNG in `placement.rs:134-154`: seeds from `(map_seed, chunk_pos, rule_index)` via `DefaultHasher`. Used for Poisson disk object placement.
- Noise seeding in `terrain.rs:106-151`: `(seed as u32).wrapping_add(def.seed_offset)` per noise layer.
- Seed propagation: server disk -> `VoxelMapConfig` -> `MapTransitionStart` message -> client `VoxelMapConfig`. Client runs same deterministic algorithms.
- No `rand` crate in game crates. No per-tick or per-entity RNG resource for gameplay.
- No loot, drop, or chance systems exist.
- Lightyear prespawn uses `seahash::SeaHasher` for cross-process determinism (`lightyear_replication/src/prespawn.rs:364-416`).
- Gameplay systems under rollback (abilities, hit detection) contain no randomness whatsoever.

## Cross-Cutting Observations

- **Entity identity is not preserved across eviction/reload.** New ECS entity, same position and def. All runtime state lost.
- **PlacementOffset double-application is a concrete bug** in the reload path — the only spawn-time operation that produces incorrect results on reload.
- **World objects reuse the character respawn system.** The `start_respawn_timer`/`process_respawn_timers` pipeline makes no world-object-specific distinction except skipping teleport-to-respawn-point.
- **Client reconstruction is fully symmetric with server spawning** — both call `clone_def_components` + `apply_object_components` from the same `WorldObjectDef`. The client additionally attaches visuals.
- **Lightyear atomicity guarantees are per-ReplicationGroup.** World objects use `DEFAULT_GROUP` (each in its own group), so despawn+respawn at the same position has no atomicity guarantee — client may see a gap frame.
- **Death detection is polling-based with no event emission.** No `DeathEvent` or observer exists that other systems could hook into.

## Open Areas (Expanded)

### Lightyear ReplicationGroup and Atomic Despawn+Respawn

- `ReplicationGroup` is a component (`lightyear_replication/src/send/components.rs:759`). `Replicate` requires it; default is `DEFAULT_GROUP` (ID 0).
- `DEFAULT_GROUP` explicitly provides **no atomicity** — entities are packed up to MTU but not guaranteed together (`components.rs:37-40`).
- Custom grouping API: `ReplicationGroup::new_id(u64)` — all entities with the same ID are sent atomically. `ReplicationGroup::new_from_entity()` gives each entity its own group.
- The project never sets a custom `ReplicationGroup` anywhere in `crates/`. All world objects use the default.
- Entities in different groups have no cross-group ordering guarantee. The receiver maintains independent `GroupChannel` per group (`receive.rs:247`), each with its own sequence counter.
- Changing an entity's group post-spawn is unsupported — the old group's `GroupChannel` retains stale state, and `local_entities` is not updated.
- Child entities via `ReplicateLike` inherit the root's group ID if they don't have their own `ReplicationGroup` (`buffer.rs:219-229`).
- For atomic despawn+respawn of a world object, the replacement entity would need to be in the same group as the original. Since group IDs are set at spawn and not transferable, this would require either: (a) a stable per-position group ID, or (b) mutating the entity in-place rather than despawn+respawn.

### `from_disk` Flag and Fresh vs. Reload Distinction

- `from_disk` is a field on `ChunkGenResult` (`generation.rs:17-25`). Set `true` only when terrain loads from disk (`generation.rs:86`). Set `false` for Features stage always (`generation.rs:138`) — even when entity spawns come from `load_chunk_entities`.
- Read at two sites in `lifecycle.rs:892,903` — solely to gate `dirty_chunks` insertion.
- `PendingEntitySpawns` (`generation.rs:35`) is `Vec<(IVec3, Vec<WorldObjectSpawn>)>` — no `from_disk` field. The flag is **dropped** when results transfer to `PendingEntitySpawns` at `lifecycle.rs:913`.
- `spawn_chunk_entities` has no access to `from_disk` or any other provenance data. `WorldObjectSpawn` carries only `object_id: String` and `position: Vec3`.
- The Features-stage `from_disk` is always `false` regardless of source, so even propagating it would not distinguish entity reload from fresh generation without also fixing `spawn_features_task`.

### Server-Authoritative Random Outcomes and Rollback

- Lightyear's rollback (`lightyear_prediction/src/rollback.rs:772`) re-runs `FixedMain` in a loop from rollback tick to current tick. No RNG seed injection, no recorded state, no replay log. Any non-deterministic operation produces different results on replay.
- `DeterministicPredicted` (`rollback.rs:184`) is unrelated to RNG — it marks entities that predict without server state confirmation (e.g. client-spawned projectiles). It suppresses rollback checks but does not inject determinism.
- For replicated-but-not-predicted components (no `.add_prediction()`), the server value lands directly on the client entity with no `Confirmed<C>` wrapper, no history, no rollback comparison (`replication.rs:330-358`). The server value is taken as truth. This is the path for `MapInstanceId`, `WorldObjectId`, `PlayerId`, `Name`, `RespawnTimerConfig`, `AbilitySlots`, `AbilityProjectileSpawn`.
- For predicted components (with `.add_prediction()`), server values arrive as `Confirmed<C>`. If client prediction diverges from server (e.g. server rolled random damage), rollback triggers and snaps to the server's confirmed value, then re-simulates forward.
- All current damage in the project is deterministic given inputs — `apply_damage_buffs` multiplies fixed amounts by buff multipliers, no RNG (`hit_detection/effects.rs:21`).
- No lightyear example demonstrates server-side random outcomes replicated to clients. The pattern would be: server computes random result, sets component, replicates. If component is predicted, client rollback corrects misprediction. If not predicted, value arrives directly.
