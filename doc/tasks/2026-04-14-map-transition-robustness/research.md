# Research Findings

## Q1: MapTransitionState — Sub-states, Systems, and Gates

### Findings
- `MapTransitionState` is a `SubStates` of `ClientState::InGame`, defined at `ui/src/state.rs:16-22`. Two variants: `Playing` (default), `Transitioning`.
- **Transitioning systems:**
  - `OnEnter(Transitioning)` → `setup_transition_loading_screen` (`ui/src/lib.rs:51-54`): spawns full-screen black overlay with `DespawnOnExit(MapTransitionState::Transitioning)`.
  - `Update` chain: `(check_transition_chunks_loaded, handle_map_transition_end).run_if(in_state(Transitioning))` (`client/src/map.rs:87-90`).
- **Playing systems:** No systems conditioned specifically on `Playing`. General `InGame` systems (chunk sync, input, HUD) run regardless of sub-state.
- **Entry gate:** Sole trigger is `map_switch_button_interaction` (`ui/src/lib.rs:510`) which sets `MapTransitionState::Transitioning` on button press, after guard `*transition_state.get() != Transitioning` at line 484. `handle_map_transition_start` does NOT set the state.
- **Exit gate:** `handle_map_transition_end` (`client/src/map.rs:649`) sets `MapTransitionState::Playing` upon receiving `MapTransitionEnd` from server. Server sends this after receiving `MapTransitionReady` from client.

## Q2: Server-Side Room Management During Transition

### Findings
- All room changes occur in `execute_server_transition` (`server/src/map.rs:1145-1221`), four `commands.trigger(RoomEvent{...})` calls at lines 1171-1186:
  1. `RemoveEntity(player_entity)` from old room (1172-1174)
  2. `RemoveSender(client_entity)` from old room (1175-1177)
  3. `AddEntity(player_entity)` to new room (1179-1181)
  4. `AddSender(client_entity)` to new room (1183-1185)
- **Deferred execution:** `commands.trigger()` queues a `Command`. The observer `handle_room_event` runs synchronously at command-flush time, not at the call site. All four mutations apply atomically when Bevy flushes between `Update` and `PostUpdate`.
- **Ordering guarantees:** `RoomPlugin::apply_room_events` is in `PostUpdate` set `ReplicationBufferSystems::BeforeBuffer`. Order:
  - `BeforeBuffer` → room membership → `VisibilityState::Lost/Gained` on `ReplicationState`
  - `Buffer` → `replicate()` sees Lost, prepares despawn actions
  - `AfterBuffer` → `update_network_visibility` resets Lost→Default
  - `Flush` → `send_replication_messages` serializes and sends
- Despawn for old room + spawn for new room are processed and sent to transport in the same `PostUpdate` frame.

## Q3: Lightyear Replication Pipeline and Room Membership

### Findings
- **Yes, old-room entities can still arrive on the client after `RemoveSender`.** In-flight `UpdatesMessage` packets serialized in previous frames (before the room change) are already in transit on the wire.
- `ActionsChannel` is reliable (retried until ack), so despawn actions will eventually arrive. But `UpdatesChannel` is unreliable — stale update packets may arrive before the despawn.
- **No flush query API exists.** The public API (`Room`, `RoomEvent`, `RoomPlugin`, `RoomTarget`, `ReplicationState`, `NetworkVisibility`, `ReplicationSender`, `ReplicationBufferSystems`) exposes nothing for "pending-flush complete" or per-room/per-client quiescence.
- `VisibilityState::Lost` is consumed (reset to `Default`) in `update_network_visibility` in the same frame, and `state.spawned` is set to false. After that, the server has no memory the entity was ever visible to that client.

## Q4: Client Chunk Pipeline End-to-End

### Findings
- **Stage 1: `handle_chunk_data_sync`** (`client/src/map.rs:148-187`): Drains `MessageReceiver<ChunkDataSync>`, inserts `ChunkData` into `VoxelMapInstance.tree` (octree), adds column to `instance.chunk_levels: HashMap<IVec2, u32>`, adds position to `instance.chunks_needing_remesh: HashSet<IVec3>`.
- **Stage 2: `spawn_remesh_tasks`** (`voxel_map_engine/src/lifecycle.rs:1019-1108`): Drains `chunks_needing_remesh` into a `BinaryHeap<ChunkWork>` ordered by `propagator.min_distance_to_source()`. Spawns async `mesh_chunk_greedy` tasks on `AsyncComputeTaskPool`. Tracks in `PendingRemeshes.tasks: Vec<RemeshTask>` and `ChunkWorkTracker.remeshing: HashSet<IVec3>`. Cap: `MAX_PENDING_REMESH_TASKS = 512`.
- **Stage 3: `poll_remesh_tasks`** (`voxel_map_engine/src/lifecycle.rs:1111-1193`): Polls completed tasks, spawns `VoxelChunk` entity with `Mesh3d` + `MeshMaterial3d` + `Transform` as child of map entity (lines 1165-1176). Removes from `tracker.remeshing`.
- **Stage 4: `attach_chunk_colliders`** (`protocol/src/map/colliders.rs:11-40`): Reacts to `Changed<Mesh3d>` / `Added<Mesh3d>` on `VoxelChunk`, generates `Collider::trimesh_from_mesh`, inserts `(Collider, RigidBody::Static, terrain_collision_layers())`.
- **Pipeline completion check:** No single flag exists. Full completion = position absent from `chunks_needing_remesh` AND absent from `ChunkWorkTracker.remeshing` AND a `VoxelChunk` child entity exists with `Mesh3d` AND that entity has `Collider`.

## Q5: Ticket/Propagator System

### Findings
- `TicketLevelPropagator` (`voxel_map_engine/src/propagator.rs:33-43`): Component on each map entity. Holds `sources: HashMap<Entity, TicketSource>`, `levels: HashMap<IVec2, u32>`, `pending_by_level: BTreeMap<u32, HashSet<IVec2>>`.
- `set_source` is called exclusively inside `collect_tickets` within `update_chunks` (`lifecycle.rs:488-496`). `update_chunks` is gated by `.run_if(generation_enabled)` (`lib.rs:50`). `ChunkGenerationEnabled` is only inserted by `ServerMapPlugin` (`server/src/map.rs:633`).
- **Client consequence:** `update_chunks` never runs on client → `collect_tickets` never runs → `set_source` never called → propagator has zero sources. `min_distance_to_source()` returns `u32::MAX` for all columns → all chunks have identical priority in `spawn_remesh_tasks` → heap ordering degenerates to arbitrary.
- `ChunkTicket` IS present on the client player (`client/src/map.rs:143`), but nothing reads it for propagator purposes.

## Q6: `check_transition_chunks_loaded` Readiness Criterion

### Findings
- Function at `client/src/map.rs:584-623`. Sole criterion: `instance.chunk_levels.is_empty()` (line 608). As soon as ONE column of raw chunk data arrives, the function sends `MapTransitionReady` and inserts `TransitionReadySent`.
- **Available but unconsulted data:**
  - `instance.chunk_levels.len()` — total columns received
  - `instance.chunks_needing_remesh.len()` — chunks queued for meshing but not yet meshed
  - `ChunkWorkTracker.remeshing` — chunks with in-flight mesh tasks
  - `VoxelChunk` entity count (via query) — chunks fully meshed and spawned
  - Player `Position` (on the entity, set at `client/src/map.rs:456`) — could scope readiness to chunks near spawn
- The function does not query `Position`, `VoxelChunk` entities, `ChunkWorkTracker`, or `chunks_needing_remesh.len()`.

## Q7: `push_chunks_to_clients` — Selection, Ordering, Rate Limit

### Findings
- `ClientChunkVisibility` (`server/src/map.rs:877-886`): `sent_chunks: HashSet<IVec3>`, `sent_columns: HashSet<IVec2>`, `tracked_map: Option<Entity>`. Both sets cleared when `tracked_map` changes (line 906-910).
- **Rate limit:** `MAX_CHUNK_SENDS_PER_TICK = 16` (`server/src/map.rs:889`). Loop breaks at `sent >= 16` (line 1005-1007).
- **Column selection** (`compute_loaded_columns`, `map.rs:951-972`): Square of `[-radius, +radius]` around player column. Chebyshev distance: `dx.abs().max(dz.abs())`. Skips if `base_level + distance > LOAD_LEVEL_THRESHOLD` or if column not yet generated.
- **Chunk ordering** (`send_unsent_chunks`, `map.rs:976-1023`): Candidates sorted ascending by column Chebyshev distance (line 1001). Closest-first.
- **No manifest protocol.** `ChunkDataSync` carries one chunk at a time. `UnloadColumn` is the only other chunk message. The client has no way to know total expected chunk count.

## Q8: World Object Spawning and Replication

### Findings
- **Server:** `spawn_chunk_entities` (`server/src/chunk_entities.rs:24-95`) drains `PendingEntitySpawns`, calls `spawn_world_object` which inserts `Replicate::to_clients(NetworkTarget::All)`. Then inserts `ChunkEntityRef { chunk_pos, map_entity }` (line 71-77) — this is a **server-only** component, not registered for replication.
- **Client:** `on_world_object_replicated` (`client/src/world_object.rs:30-65`) triggers on `Added<Replicated>` + `WorldObjectId`. Looks up visual def, generates collider, attaches vox mesh. No check against chunk state.
- **No chunk association on client.** `ChunkEntityRef` is never replicated. The client handler has no knowledge of which chunk a world object belongs to. No mechanism to defer visual setup until terrain is meshed.

## Q9: Client Cleanup During Transition

### Findings
- `handle_map_transition_start` (`client/src/map.rs:418-485`) calls:
  1. `despawn_all_maps_except` (line 523-539): removes all non-target entries from `MapRegistry`, despawns map entities recursively (all chunk children go with them).
  2. `despawn_foreign_world_objects` (line 500-515): despawns all `WorldObjectId` + `MapInstanceId` entities not matching the target.
- **Survivors:** `MapRegistry` resource (mutated, not destroyed), `VoxelPredictionState` (untouched), `OverworldMap` resource (still holds handle to despawned entity), the player entity (frozen, not despawned).
- **No guard against late arrivals.** `on_world_object_replicated` fires on `Added<Replicated>` with no check against `MapTransitionState` or `MapInstanceId` of the current active map. Late-arriving replicated entities from the old room will be fully set up visually.

## Q10: Loading Screen

### Findings
- `setup_transition_loading_screen` (`ui/src/lib.rs:542-566`): Full-screen black `BackgroundColor` with `GlobalZIndex(100)`, single `Text::new("Loading...")` at font size 48. `DespawnOnExit(MapTransitionState::Transitioning)`. System signature is only `mut commands: Commands` — queries nothing from ECS.
- **Available progress data (not used):**
  - `VoxelMapInstance.chunk_levels.len()` — columns received
  - `VoxelMapInstance.chunks_needing_remesh.len()` — pending mesh queue
  - `ChunkWorkTracker.remeshing` count — in-flight mesh tasks
  - `VoxelChunk` entity count — fully meshed chunks
  - `TransitionReadySent` marker — whether ready signal was sent
  - `PendingTransition(MapInstanceId)` — target map identity
  - `MapTransitionState` — current phase

## Q11: Client-Side State Bound to Server Connection

### Findings
- **Lightyear-managed:** `ReplicationReceiver`, `PredictionManager` on client entity (`client/src/network.rs:99-108`). `NetcodeClient` (session token).
- **Replicated player state:** `Position`, `Rotation`, `LinearVelocity`, `AngularVelocity` (predicted+rollback), `MapInstanceId`, `Health`, `CharacterType`, ability slots/cooldowns/buffs.
- **Transition-specific on player:** `PendingTransition`, `TransitionReadySent`, `RigidBodyDisabled`, `ColliderDisabled`, `DisableRollback`.
- **Connection-scoped resources:** `MapRegistry` (mutated during transition), `VoxelPredictionState` (pending edits keyed to connection sequence numbers).
- **Maximum detachment point:** After `handle_map_transition_start` runs but before first `ChunkDataSync` arrives. At this point: old maps despawned, old world objects despawned, player frozen, new `VoxelMapInstance` spawned but `chunk_levels` empty, `MapTransitionReady` not yet sent, loading screen visible. Only live connection-state: `PredictionManager`, frozen player entity, new empty `VoxelMapInstance`.

## Q12: Transition Logic Distribution Across Crates

### Findings
- **`protocol/src/map/transition.rs`** (pure types): `MapChannel`, `PlayerMapSwitchRequest`, `PendingTransition`, `MapTransitionStart`, `MapTransitionReady`, `MapTransitionEnd`, `TransitionReadySent`. All exclusively transition.
- **`client/src/map.rs`** — transition-exclusive: `handle_map_transition_start` (418-485), `check_transition_chunks_loaded` (584-623), `handle_map_transition_end` (625-652), `despawn_all_maps_except` (523-539), `despawn_foreign_world_objects` (500-515). Shared: `handle_chunk_data_sync`, `attach_chunk_ticket_to_player`, all voxel edit systems, `MapRegistry`, `VoxelPredictionState`.
- **`server/src/map.rs`** — transition-exclusive: `handle_map_switch_requests` (1059-1123), `execute_server_transition` (1145-1221), `handle_map_transition_ready` (1379-1413), `ensure_map_exists`/`spawn_homebase`/`resolve_switch_target`/`load_homebase_seed`. Shared: `push_chunks_to_clients`, `ClientChunkVisibility`, `RoomRegistry`, all voxel edit/save systems.
- **`ui/src/lib.rs`** — transition-exclusive: `MapTransitionState` sub-state, `setup_transition_loading_screen` (542-566), transition logic inside `map_switch_button_interaction` (468-511). Shared: `ClientState`, `DespawnOnExit` pattern, general HUD.
- **Encapsulation patterns elsewhere:** `protocol/src/ability/` is the clearest model — dedicated subdirectory, own plugin (`AbilityPlugin`), re-exports through `mod.rs`. `protocol/src/hit_detection/` and `protocol/src/terrain/` follow similar patterns. The transition flow does NOT follow this — it is distributed across four files with no dedicated plugin or subdirectory.

## Q13: Initial Connection vs. Mid-Game Transition

### Findings
- **Initial connection:** User presses Connect → `ClientState::Connecting` → `on_entering_connecting_state` (`ui/src/lib.rs:94-118`) creates `NetcodeClient` and triggers `Connect` → lightyear connects → `Connected` added → `on_client_connected` (`ui/src/lib.rs:132-138`) sets `ClientState::InGame`.
- **Server on connect:** `handle_connected` (`server/src/gameplay.rs:352-418`) spawns character with `Replicate`, `PredictionTarget`, places on `MapInstanceId::Overworld`, adds client to overworld room.
- **First map load:** `spawn_overworld` (`client/src/map.rs:98-128`) runs on `OnEnter(AppState::Ready)`, spawns overworld `VoxelMapInstance` independently of connection. `attach_chunk_ticket_to_player` inserts `ChunkTicket` once predicted player exists.
- **Shared between both paths:** `handle_chunk_data_sync`, `attach_chunk_colliders`, `CharacterPhysicsBundle` (inserted via `handle_new_character`), `push_chunks_to_clients` (server).
- **Divergences:**
  - Initial has **no handshake** (`MapTransitionStart`/`Ready`/`End` only run mid-game).
  - Initial spawns overworld map client-side in `OnEnter(AppState::Ready)`, not from a server message.
  - Initial has **no player freeze** (`RigidBodyDisabled`/`DisableRollback`).
  - Initial loading screen is `DespawnOnExit(ClientState::Connecting)`, not `DespawnOnExit(MapTransitionState::Transitioning)`.
  - Server initial join: just `AddSender` to overworld room. Mid-game: `RemoveEntity`+`RemoveSender` from old, `AddEntity`+`AddSender` to new.

## Q14: Player Entity Components and Remote Player Replication

### Findings
- **Fully-loaded predicted player components:** `PlayerId`, `CharacterMarker`, `CharacterType`, `Health`, `Invulnerable`, `RespawnTimer`, `ColorComponent`, `Position`, `Rotation`, `LinearVelocity`, `AngularVelocity` (all replicated+predicted), `MapInstanceId` (replicated), ability components, `Predicted`, `Replicated`, `Controlled`, `CharacterPhysicsBundle` (Collider, RigidBody::Dynamic, LockedAxes, Friction, CollisionLayers), `InputMap<PlayerActions>` (controlled only), `ChunkTicket`, `SpriteRig`+`AnimSetRef`+`Facing` (visual rig), `FrameInterpolate<Position/Rotation>`, health bar child.
- **Physics inserted by** `handle_new_character` (`client/src/gameplay.rs:66-71`) on `Added<Predicted>` / `Added<Interpolated>`.
- **Visual rig by** `resolve_character_rig` (`sprite_rig/src/spawn.rs:64-88`) on `Added<CharacterType>`.
- **Remote player replication:** When client's sender is added to a room, lightyear replicates all entities in that room. Remote players arrive as `Interpolated` entities. Setup cascades through component observers (`Added<CharacterType>`, `Added<Position>`, `Added<Health>`).
- **No mechanism to determine all remote players have arrived.** No expected-player count is tracked. No gameplay gate on remote player arrival. Players appear entity-by-entity as lightyear replicates them.

## Cross-Cutting Observations

- **Readiness criterion is minimal.** `check_transition_chunks_loaded` fires after a single column arrives. Rich progress data exists on `VoxelMapInstance` (`chunk_levels`, `chunks_needing_remesh`) and `ChunkWorkTracker` but is unconsulted.
- **No manifest/completion signal.** Neither chunks nor world objects have a server→client count or completion message. The client cannot know how many chunks or world objects to expect.
- **Late-arrival vulnerability.** After `RemoveSender`, in-flight packets can still deliver old-room entities. `on_world_object_replicated` and all visual setup systems have no guard against `MapTransitionState` or active map check.
- **Client propagator is dead.** The `TicketLevelPropagator` exists on client map entities but has zero sources, making remesh prioritization arbitrary during transitions (and at all other times).
- **Initial connect and mid-game transition are separate code paths** with significant shared infrastructure but no shared readiness/handshake logic. The initial path has no freeze, no loading gate, no readiness check.
- **Transition logic is scattered.** Types in `protocol/`, behavior in `client/` and `server/` (mixed with general map systems), UI in `ui/`. The `ability/` subdirectory pattern exists as a model for consolidation but is not applied here.

## Resolved Open Areas

### Lightyear Action Ordering Across Rooms — NO cross-group ordering guarantee

Lightyear uses a single `ActionsChannel` (registered as `UnorderedReliable` at `lightyear_replication/src/plugin.rs:54`), but layers per-group sequence IDs on top (`sender.rs:832`, `message.rs:254`). Each replication group has its own independent `GroupChannel` with its own send/receive sequence counters (`receive.rs:524`). `send_actions_messages` (`sender.rs:575-694`) iterates groups via `EntityHashSet` (unspecified order), producing separate `ActionsMessage`s per group. On the client, `apply_world` (`receive.rs:395-511`) drains each group's buffer independently with no cross-group synchronization.

**Conclusion:** Old-room `EntityDespawn` and new-room `EntitySpawn` are in different replication groups. They can be applied in either order on the client. There is no guarantee despawns arrive before spawns.

### `OverworldMap` Resource — Dead code, no risk

`OverworldMap` is defined on both client (`client/src/map.rs:95-96`) and server (`server/src/map.rs:62-63`) as a newtype `Resource` wrapping `Entity`. It is inserted once on each side and **never subsequently read by any system**. All map entity lookups go through `MapRegistry`. The stale handle after despawn is inert.

### `VoxelPredictionState` Across Transitions — Real interference risk

`VoxelPredictionState` (`client/src/map.rs:27-47`) holds `pending: Vec<VoxelPrediction>` where each entry has only `(sequence, position: IVec3, old_voxel, new_voxel)` — no `MapInstanceId` or map entity reference. It is never cleared during transition. Two interference paths:

1. **Broadcast/section-update suppression** (`map.rs:227-236`, `267-275`): Incoming server voxel updates are silently skipped if their `IVec3` position matches any pending prediction. A stale prediction from the old map can suppress a legitimate server update for the same world-space coordinate on the new map.
2. **Reject path** (`map.rs:406-410`): `handle_voxel_edit_reject` writes `reject.correct_voxel` to `chunk_ticket.map_entity` — the player's *current* `ChunkTicket` target, which post-transition points to the new map. A reject for an old-map edit would write to the new map at the same `IVec3`.

The ack path (`map.rs:370-386`) only prunes `pending` in-memory — no voxel write — so it is safe.
