# Implementation Plan

## Overview

Replace the scattered map transition code with a unified transition module implementing a five-phase client state machine (Idle→Cleanup→Loading→Ready→Complete→Idle), split-phase server orchestration, spatial-radius readiness gating, and client-side propagator activation. Both initial connection and mid-game transitions flow through the same path.

## Module Placement (Crate Dependency Constraint)

`protocol` depends on `voxel_map_engine`. `client`, `server`, and `ui` all depend on `protocol`. **`protocol` cannot depend on `client`, `server`, or `ui`.**

This dictates where code lives:

| Location | Contents |
|----------|----------|
| `protocol/src/transition/types.rs` | `TransitionPhase`, `ClientTransitionState`, `MapTransitionEntity`, `TransitionPending` component, `TRANSITION_READINESS_RADIUS` constant |
| `protocol/src/transition/relocation.rs` | `relocate_remove`, `relocate_add` helpers (entity-type agnostic) |
| `protocol/src/transition/plugin.rs` | `TransitionPlugin` — registers types and messages only |
| `client/src/transition.rs` **(new file)** | Client state machine, message handlers, spatial readiness check, entity guards, cleanup system, map spawn helpers. Has access to `MapTransitionState`, `VoxelPredictionState`, `TerrainDefRegistry`, etc. |
| `server/src/transition.rs` **(new file)** | `start_map_transition` (Phase 1 helper), `complete_map_transition` (Phase 2 system). Has access to `RoomRegistry`, `WorldSavePath`, etc. |
| `ui/src/lib.rs` | Loading screen text update system. `MapTransitionState` stays in `ui/src/state.rs`. |

Systems that reference crate-local types (e.g., `MapTransitionState` in `ui`, `RoomRegistry` in `server`) live in that crate and are registered in that crate's plugin. The `protocol/src/transition/` module holds only types, messages, and entity-agnostic helpers.

---

## Phase 1: Foundation Types and Module Scaffold

Stand up `protocol/src/transition/` with all shared types, messages, and plugin shell. No behavior changes.

### Changes

#### 1. Create directory `crates/protocol/src/transition/`

#### 2. `crates/protocol/src/transition/types.rs` (create)

```rust
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use crate::map::{MapInstanceId, MapTransitionStart};
use voxel_map_engine::lifecycle::world_to_column_pos;

/// Client-side transition phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Reflect)]
pub enum TransitionPhase {
    #[default]
    Idle,
    Cleanup,
    Loading,
    Ready,
    Complete,
}

/// Client-side transition state resource.
#[derive(Resource, Debug)]
pub struct ClientTransitionState {
    pub phase: TransitionPhase,
    pub target_map: Option<MapInstanceId>,
    pub readiness_radius: u32,
    pub spawn_position: Vec3,
    /// Chunk-space column derived from spawn_position via world_to_column_pos.
    pub spawn_column: IVec2,
    pub chunk_size: u32,
    pub column_y_range: (i32, i32),
    /// Raw server Entity IDs from MapTransitionEntity messages.
    /// Lightyear does NOT auto-remap these — remapping only happens if
    /// the message type implements MapEntities AND .add_map_entities()
    /// is chained on registration. We skip both deliberately.
    pub pending_entities: Vec<Entity>,
    pub end_received: bool,
}

impl Default for ClientTransitionState {
    fn default() -> Self {
        Self {
            phase: TransitionPhase::Idle,
            target_map: None,
            readiness_radius: 0,
            spawn_position: Vec3::ZERO,
            spawn_column: IVec2::ZERO,
            chunk_size: 1,
            column_y_range: (0, 0),
            pending_entities: Vec::new(),
            end_received: false,
        }
    }
}

impl ClientTransitionState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Initialize from a MapTransitionStart message.
    pub fn begin(&mut self, start: &MapTransitionStart) {
        self.phase = TransitionPhase::Cleanup;
        self.target_map = Some(start.target.clone());
        self.readiness_radius = start.readiness_radius;
        self.spawn_position = start.spawn_position;
        self.chunk_size = start.chunk_size;
        self.column_y_range = start.column_y_range;
        self.pending_entities.clear();
        self.end_received = false;
        // Use the engine's canonical world→chunk conversion
        self.spawn_column = world_to_column_pos(start.spawn_position, start.chunk_size);
    }
}

/// Server→Client message carrying an unmapped server-side Entity ID for a
/// relocated entity. We deliberately skip MapEntities + add_map_entities()
/// so the server Entity arrives unchanged on the client. Client polls
/// RemoteEntityMap::get_local until the mapping resolves.
#[derive(Serialize, Deserialize, Clone, Debug, Reflect, Message)]
#[type_path = "protocol::transition"]
pub struct MapTransitionEntity {
    pub entity: Entity,
}

/// Inserted on the player entity after server Phase 1 completes.
/// Carries data needed for Phase 2 (complete_map_transition).
#[derive(Component)]
pub struct TransitionPending {
    pub client_entity: Entity,
    pub target_map_id: MapInstanceId,
    pub new_room: Entity,
    /// Entities removed from old room in Phase 1 that need AddEntity in Phase 2.
    pub relocated_entities: Vec<Entity>,
}

/// Default readiness radius (Chebyshev column distance from spawn).
pub const TRANSITION_READINESS_RADIUS: u32 = 2;
```

#### 3. `crates/protocol/src/transition/plugin.rs` (create)

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use super::types::*;

pub struct TransitionPlugin;

impl Plugin for TransitionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClientTransitionState>();
        app.register_type::<TransitionPhase>();

        // Register new message. Channel is specified at send time
        // (sender.send::<MapChannel>(...)), not at registration.
        // Existing MapTransitionStart/Ready/End registration stays
        // in ProtocolPlugin::build (protocol/src/lib.rs:133-147).
        app.register_message::<MapTransitionEntity>()
            .add_direction(NetworkDirection::ServerToClient);
    }
}
```

#### 4. `crates/protocol/src/transition/mod.rs` (create)

```rust
mod types;
pub mod relocation;
mod plugin;

pub use types::*;
pub use plugin::TransitionPlugin;
```

#### 5. Modify `crates/protocol/src/map/transition.rs`
**Action**: Add `readiness_radius` field to `MapTransitionStart`.

Add field after `column_y_range`:
```rust
pub readiness_radius: u32,
```

#### 6. Modify `crates/protocol/src/lib.rs`

- Add `pub mod transition;` to module declarations.
- Add `TransitionPlugin` to the plugin group (near line 229).
- Add `MapTransitionEntity` to the `pub use map::{...}` re-export block.

#### 7. Modify `crates/server/src/map.rs` — `execute_server_transition`

At line 1212, the `MapTransitionStart` construction — add:
```rust
readiness_radius: protocol::transition::TRANSITION_READINESS_RADIUS,
```

### Verification
#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test-all` passes

#### Manual
- [ ] No runtime behavior change. Existing transition still works.

---

## Phase 2: Client Propagator Activation

Extract `collect_tickets` from `update_chunks` so it runs unconditionally. This gives the client propagator sources for distance-based remesh priority.

### Changes

#### 1. `crates/voxel_map_engine/src/lifecycle.rs` — Extract `collect_tickets`

Currently `collect_tickets` (lines 414-509) is a private function called from `update_chunks` at line 335, borrowing `update_chunks`'s query params and `Local<HashMap<Entity, CachedTicket>>`.

**Step A**: Convert to a standalone public system. It needs the same `map_query` type as `update_chunks` (the 12-tuple at lines 310-332) because it accesses `TicketLevelPropagator`, `VoxelMapInstance` (for `chunk_size`), and `GlobalTransform` through that query. Keep the full 12-tuple to avoid Bevy query conflicts with `update_chunks` running in the same chain:

```rust
pub fn collect_tickets(
    mut map_query: Query<(
        Entity, &mut VoxelMapInstance, &VoxelMapConfig, &MapDimensions,
        &VoxelGenerator, &mut PendingChunks, &mut TicketLevelPropagator,
        &GlobalTransform, &mut ChunkWorkBudget, &mut GenQueue,
        &mut PendingSaves, &mut ChunkWorkTracker,
    )>,
    ticket_query: Query<(Entity, &ChunkTicket, &GlobalTransform)>,
    mut ticket_cache: Local<HashMap<Entity, CachedTicket>>,
) {
    // Body identical to current lines 432-508.
    // Parameters come from system injection instead of update_chunks.
}
```

**Step B**: Remove the `collect_tickets(...)` call from `update_chunks` (line 335). Remove `ticket_query` (line 329) and `ticket_cache` (line 331) from `update_chunks`'s signature. `TicketLevelPropagator` stays in the 12-tuple — `propagator.propagate()` is called at line 356 inside `update_chunks`'s main loop.

#### 2. `crates/voxel_map_engine/src/lib.rs` — Register unconditionally

Add `collect_tickets` before the generation-gated systems:

```rust
app.add_systems(
    Update,
    (
        lifecycle::ensure_pending_chunks,
        lifecycle::collect_tickets,  // NEW — unconditional, before update_chunks
        (lifecycle::update_chunks, lifecycle::poll_chunk_tasks).run_if(generation_enabled),
        lifecycle::reset_chunk_budgets.run_if(not(generation_enabled)),
        lifecycle::despawn_out_of_range_chunks,
        lifecycle::drain_pending_saves,
        lifecycle::spawn_remesh_tasks,
        lifecycle::poll_remesh_tasks,
    )
        .chain(),
);
```

`collect_tickets` calls `propagator.set_source()` which populates `sources`. `spawn_remesh_tasks` uses `propagator.min_distance_to_source()` (propagator.rs:161-167) which reads `sources` directly — it does NOT depend on `propagator.propagate()` (the level map). So sources alone are sufficient for distance-based ordering on the client.

### Verification
#### Automated
- [ ] `cargo check-all` passes
- [ ] `cargo test-all` passes

#### Manual
- [ ] `cargo server` + `cargo client` — chunks mesh closest-first around player. Transition between maps: chunks near spawn mesh first.

---

## Phase 3: Spatial Readiness + Client State Machine

Replace `check_transition_chunks_loaded` with spatial-radius readiness. Wire the client-side state machine. Update loading screen.

### Changes

#### 1. `crates/client/src/transition.rs` (create)

This file lives in the `client` crate where it has access to `MapTransitionState`, `VoxelPredictionState`, `TerrainDefRegistry`, etc.

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear_connection::prelude::Client;
use lightyear_messages::MessageManager;
use voxel_map_engine::prelude::*;
use protocol::map::*;
use protocol::transition::*;
use ui::state::MapTransitionState;
use std::collections::HashSet;

/// Receives MapTransitionEnd, sets end_received flag.
pub fn receive_transition_end(
    mut receivers: Query<&mut MessageReceiver<MapTransitionEnd>>,
    mut state: ResMut<ClientTransitionState>,
) {
    for mut receiver in &mut receivers {
        for _end in receiver.receive() {
            trace!("Received MapTransitionEnd");
            state.end_received = true;
        }
    }
}

/// Receives MapTransitionEntity, accumulates server entity IDs.
pub fn receive_transition_entities(
    mut receivers: Query<&mut MessageReceiver<MapTransitionEntity>>,
    mut state: ResMut<ClientTransitionState>,
) {
    for mut receiver in &mut receivers {
        for msg in receiver.receive() {
            trace!("Received MapTransitionEntity entity={:?}", msg.entity);
            state.pending_entities.push(msg.entity);
        }
    }
}

/// Main state machine driver. Evaluates phase gates, advances phase.
pub fn update_transition_state(
    mut state: ResMut<ClientTransitionState>,
    registry: Res<MapRegistry>,
    map_query: Query<(&VoxelMapInstance, &ChunkWorkTracker, &Children)>,
    chunk_query: Query<(&VoxelChunk, Has<Collider>), With<Mesh3d>>,
    mut ready_senders: Query<&mut MessageSender<MapTransitionReady>>,
    manager_query: Query<&MessageManager, With<Client>>,
    entity_exists: Query<Entity>,
    mut next_transition: ResMut<NextState<MapTransitionState>>,
) {
    match state.phase {
        TransitionPhase::Idle => return,

        TransitionPhase::Cleanup => {
            // [SERVER-SWITCH SEAM] — At this point, old maps are despawned,
            // prediction state is cleared, loading screen is visible. A
            // server-switch would: disconnect, connect to new server, re-auth,
            // then resume at Loading when MapTransitionStart arrives.

            // Gate: VoxelMapInstance exists in MapRegistry for target map
            let Some(target) = &state.target_map else {
                debug_assert!(false, "Cleanup phase with no target_map");
                return;
            };
            let Some(&map_entity) = registry.0.get(target) else {
                return; // Map not yet spawned
            };
            if map_query.get(map_entity).is_err() {
                return; // VoxelMapInstance not yet on entity
            }
            trace!("Transition: Cleanup → Loading");
            state.phase = TransitionPhase::Loading;
        }

        TransitionPhase::Loading => {
            // Gate: spatial readiness
            if !check_spatial_readiness(&state, &registry, &map_query, &chunk_query) {
                return;
            }
            for mut sender in &mut ready_senders {
                sender.send::<MapChannel>(MapTransitionReady);
            }
            trace!("Transition: Loading → Ready (sent MapTransitionReady)");
            state.phase = TransitionPhase::Ready;
        }

        TransitionPhase::Ready => {
            // Gate: end_received AND all pending entities resolved
            if !state.end_received {
                return;
            }
            if !state.pending_entities.is_empty()
                && !check_entities_resolved(&state, &manager_query, &entity_exists)
            {
                return;
            }
            trace!("Transition: Ready → Complete");
            state.phase = TransitionPhase::Complete;
        }

        TransitionPhase::Complete => {
            next_transition.set(MapTransitionState::Playing);
            state.reset();
            trace!("Transition: Complete → Idle");
        }
    }
}
```

**`check_spatial_readiness`** — uses `VoxelChunk.position` directly (avoids the `chunk_world_offset` reverse-mapping issue):

```rust
fn check_spatial_readiness(
    state: &ClientTransitionState,
    registry: &MapRegistry,
    map_query: &Query<(&VoxelMapInstance, &ChunkWorkTracker, &Children)>,
    chunk_query: &Query<(&VoxelChunk, Has<Collider>), With<Mesh3d>>,
) -> bool {
    let target = state.target_map.as_ref()?; // returns false via early return
    let &map_entity = registry.0.get(target)?;
    let (instance, tracker, children) = map_query.get(map_entity).ok()?;

    let radius = state.readiness_radius as i32;
    let (y_min, y_max) = state.column_y_range;

    // Check every column within Chebyshev distance of spawn column
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            let col = IVec2::new(state.spawn_column.x + dx, state.spawn_column.y + dz);

            // Column data must have arrived from server
            if !instance.chunk_levels.contains_key(&col) {
                return false;
            }

            // No chunks in this column still pending remesh or in-flight
            for y in y_min..=y_max {
                let pos = IVec3::new(col.x, y, col.y);
                if instance.chunks_needing_remesh.contains(&pos)
                    || tracker.remeshing.contains(&pos)
                {
                    return false;
                }
            }
        }
    }

    // Verify VoxelChunk children of this map within radius have Collider.
    // Uses VoxelChunk.position (chunk-space IVec3) directly — avoids
    // reverse-mapping from Transform which requires accounting for
    // chunk_world_offset's -Vec3::ONE rendering adjustment.
    for &child in children.iter() {
        if let Ok((chunk, has_collider)) = chunk_query.get(child) {
            let chunk_col = IVec2::new(chunk.position.x, chunk.position.z);
            let dx = (chunk_col.x - state.spawn_column.x).abs();
            let dz = (chunk_col.y - state.spawn_column.y).abs();
            if dx <= radius && dz <= radius && !has_collider {
                return false;
            }
        }
    }

    true
}
```

Note: The function signature uses `?` for early returns. Since it returns `bool`, not `Option`, use explicit `let/else` or nested `if let` instead. The pseudocode above shows intent — adjust to actual Rust patterns during implementation.

**`check_entities_resolved`**:

```rust
fn check_entities_resolved(
    state: &ClientTransitionState,
    manager_query: &Query<&MessageManager, With<Client>>,
    entity_exists: &Query<Entity>,
) -> bool {
    let Ok(manager) = manager_query.get_single() else { return false; };
    state.pending_entities.iter().all(|&remote| {
        manager
            .entity_mapper
            .get_local(remote)
            .is_some_and(|local| entity_exists.get(local).is_ok())
    })
}
```

#### 2. `crates/client/src/lib.rs` — Add module

Add `pub mod transition;` and register systems in `ClientPlugin` (or a new `ClientTransitionPlugin`):

```rust
// Run during InGame — receives messages even before Transitioning is set
let in_game = in_state(ClientState::InGame);
app.add_systems(
    Update,
    (
        transition::receive_transition_end,
        transition::receive_transition_entities,
        transition::update_transition_state,
    )
        .chain()
        .run_if(in_game),
);
```

Systems run when `InGame` (not gated on `Transitioning`) because the initial `MapTransitionStart` arrives before `Transitioning` is set. The state machine only advances when `phase != Idle`.

#### 3. `crates/client/src/map.rs` — Remove old systems

- **Remove** `check_transition_chunks_loaded` (lines 584-623).
- **Remove** `handle_map_transition_end` (lines 625-652).
- **Remove** their system registrations from `ClientMapPlugin::build` (lines 87-90).
- **Modify** `handle_map_transition_start`: add `ResMut<ClientTransitionState>` and `ResMut<VoxelPredictionState>` params. After existing cleanup/freeze/spawn logic, add:
  ```rust
  transition_state.begin(&transition);
  prediction_state.pending.clear();
  ```

#### 4. `crates/ui/src/lib.rs` — Loading screen update

Add `LoadingScreenText` marker component to the text entity in `setup_transition_loading_screen`.

Add update system:

```rust
fn update_loading_screen_text(
    state: Res<ClientTransitionState>,
    mut text_query: Query<&mut Text, With<LoadingScreenText>>,
    registry: Res<MapRegistry>,
    map_query: Query<&VoxelMapInstance>,
) {
    let Ok(mut text) = text_query.get_single_mut() else { return; };
    let phase_name = match state.phase {
        TransitionPhase::Cleanup => "Cleanup",
        TransitionPhase::Loading => "Loading",
        TransitionPhase::Ready => "Syncing",
        TransitionPhase::Complete | TransitionPhase::Idle => "Loading",
    };
    let mut detail = String::new();
    if let Some(ref target) = state.target_map {
        if let Some(&map_entity) = registry.0.get(target) {
            if let Ok(instance) = map_query.get(map_entity) {
                let columns = instance.chunk_levels.len();
                let pending = instance.chunks_needing_remesh.len();
                detail = format!("\nChunks: {columns} cols, {pending} pending mesh");
            }
        }
    }
    *text = Text::new(format!("{phase_name}...{detail}"));
}
```

Register in `UiPlugin` with `.run_if(in_state(MapTransitionState::Transitioning))`.

### Verification
#### Automated
- [ ] `cargo check-all` passes
- [ ] `cargo test-all` passes

#### Manual
- [ ] `cargo server` + `cargo client` — mid-game transition: loading screen stays up until nearby terrain is meshed+collidered. Shows phase name and chunk counts.

---

## Phase 4: Split-Phase Server Orchestration + Entity Relocation

Replace `execute_server_transition` with Phase 1/Phase 2 split. Extract relocation helpers.

### Changes

#### 1. `crates/protocol/src/transition/relocation.rs` (create)

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use crate::map::MapInstanceId;

/// Remove an entity from its current room, update MapInstanceId.
/// Optionally update Position. Does NOT call AddEntity — the caller
/// decides when to add to the new room (e.g. deferred to Phase 2).
pub fn relocate_remove(
    commands: &mut Commands,
    entity: Entity,
    old_room: Entity,
    target_map_id: &MapInstanceId,
    spawn_position: Option<Vec3>,
) {
    commands.trigger(RoomEvent {
        room: old_room,
        target: RoomTarget::RemoveEntity(entity),
    });
    commands.entity(entity).insert(target_map_id.clone());
    if let Some(pos) = spawn_position {
        // Only insert Position. Caller handles velocity zeroing if needed
        // (not all entities with Position have LinearVelocity — e.g.
        // RespawnPoints, static world objects).
        commands.entity(entity).insert(avian3d::prelude::Position(pos));
    }
}

/// Add an entity to a room. Counterpart to relocate_remove.
pub fn relocate_add(commands: &mut Commands, entity: Entity, new_room: Entity) {
    commands.trigger(RoomEvent {
        room: new_room,
        target: RoomTarget::AddEntity(entity),
    });
}
```

#### 2. `crates/server/src/transition.rs` (create)

Lives in the `server` crate where it has access to `RoomRegistry`, `WorldSavePath`, `TerrainDefRegistry`, etc.

**`start_map_transition`** — Phase 1 helper, called from `handle_map_switch_requests`. Replaces `execute_server_transition`. Same parameter-passing pattern as the function it replaces (helper, not system):

```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use voxel_map_engine::prelude::ChunkTicket;
use protocol::map::*;
use protocol::transition::*;

/// Server Phase 1 — called on mid-game transition request.
///
/// Removes client sender from old room, relocates character (RemoveEntity +
/// freeze + update Position), updates ChunkTicket, sends MapTransitionStart.
/// Inserts TransitionPending marker for Phase 2.
pub fn start_map_transition(
    commands: &mut Commands,
    player_entity: Entity,
    client_entity: Entity,
    current_map_id: &MapInstanceId,
    target_map_id: &MapInstanceId,
    registry: &mut MapRegistry,
    room_registry: &mut crate::map::RoomRegistry,
    map_params_query: &Query<(&VoxelMapConfig, &MapDimensions)>,
    senders: &mut Query<&mut MessageSender<MapTransitionStart>>,
    save_path: &crate::map::WorldSavePath,
    terrain_registry: &TerrainDefRegistry,
    type_registry: &AppTypeRegistry,
    respawn_query: &Query<(&Position, &MapInstanceId), With<RespawnPoint>>,
) {
    let old_room = room_registry.get_or_create(current_map_id, commands);
    let new_room = room_registry.get_or_create(target_map_id, commands);

    // Remove client sender from old room
    commands.trigger(RoomEvent {
        room: old_room,
        target: RoomTarget::RemoveSender(client_entity),
    });

    // Resolve spawn position from RespawnPoint for the target map
    let spawn_position = respawn_query
        .iter()
        .find(|(_, mid)| *mid == target_map_id)
        .map(|(pos, _)| pos.0)
        .unwrap_or(crate::gameplay::DEFAULT_SPAWN_POS);

    // Relocate character: RemoveEntity + update MapInstanceId + Position
    relocation::relocate_remove(
        commands, player_entity, old_room, target_map_id, Some(spawn_position),
    );
    // Zero velocity (characters always have LinearVelocity via RigidBody::Dynamic)
    commands.entity(player_entity).insert(
        avian3d::prelude::LinearVelocity(Vec3::ZERO),
    );

    // Freeze character server-side
    commands.entity(player_entity).insert((
        DisableRollback,
        ColliderDisabled,
        RigidBodyDisabled,
        PendingTransition(target_map_id.clone()),
    ));

    // Ensure target map exists, get params
    let (map_entity, params) = crate::map::ensure_map_exists(
        commands, target_map_id, registry, map_params_query,
        save_path, terrain_registry, type_registry,
    );

    commands.entity(player_entity).insert(ChunkTicket::player(map_entity));

    // Send MapTransitionStart
    let mut sender = senders.get_mut(client_entity)
        .expect("Client entity must have MessageSender<MapTransitionStart>");
    sender.send::<MapChannel>(MapTransitionStart {
        target: target_map_id.clone(),
        seed: params.seed,
        generation_version: params.generation_version,
        bounds: params.bounds,
        spawn_position,
        chunk_size: params.chunk_size,
        column_y_range: params.column_y_range,
        readiness_radius: TRANSITION_READINESS_RADIUS,
    });

    // Mark for Phase 2
    commands.entity(player_entity).insert(TransitionPending {
        client_entity,
        target_map_id: target_map_id.clone(),
        new_room,
        relocated_entities: vec![player_entity],
    });
}
```

**`complete_map_transition`** — Phase 2 standalone system. Replaces `handle_map_transition_ready`:

```rust
/// Server Phase 2 — on MapTransitionReady from client.
/// AddSender to new room, unfreeze + AddEntity for relocated entities,
/// send MapTransitionEntity per entity, then MapTransitionEnd.
pub fn complete_map_transition(
    mut commands: Commands,
    mut receivers: Query<(Entity, &mut MessageReceiver<MapTransitionReady>)>,
    transition_query: Query<(Entity, &TransitionPending)>,
    mut end_senders: Query<&mut MessageSender<MapTransitionEnd>>,
    mut entity_senders: Query<&mut MessageSender<MapTransitionEntity>>,
) {
    for (client_entity, mut receiver) in &mut receivers {
        for _ready in receiver.receive() {
            let Some((player_entity, pending)) = transition_query.iter()
                .find(|(_, p)| p.client_entity == client_entity)
            else {
                warn!("MapTransitionReady from {client_entity:?} but no TransitionPending");
                continue;
            };

            trace!("Completing transition for client {client_entity:?}");

            // Add client sender to new room
            commands.trigger(RoomEvent {
                room: pending.new_room,
                target: RoomTarget::AddSender(client_entity),
            });

            // Unfreeze and add each relocated entity to new room
            for &entity in &pending.relocated_entities {
                commands.entity(entity).remove::<(
                    RigidBodyDisabled, ColliderDisabled, DisableRollback, PendingTransition,
                )>();
                relocation::relocate_add(&mut commands, entity, pending.new_room);

                // Send raw server entity ID to client
                if let Ok(mut sender) = entity_senders.get_mut(client_entity) {
                    sender.send::<MapChannel>(MapTransitionEntity { entity });
                }
            }

            // Send MapTransitionEnd (OrderedReliable guarantees order after entity messages)
            if let Ok(mut sender) = end_senders.get_mut(client_entity) {
                sender.send::<MapChannel>(MapTransitionEnd);
            }

            commands.entity(player_entity).remove::<TransitionPending>();
        }
    }
}
```

#### 3. `crates/server/src/map.rs` — Replace old functions

- **Remove** `execute_server_transition` (lines 1145-1221).
- **Remove** `handle_map_transition_ready` (lines 1379-1413).
- **Make** `MapTransitionParams` `pub` (line 1136) and `ensure_map_exists` `pub`.
- **Modify** `handle_map_switch_requests`: replace `execute_server_transition(...)` call with `transition::start_map_transition(...)` passing the same resolved data.
- **Register** `transition::complete_map_transition` in `ServerMapPlugin::build` where `handle_map_transition_ready` was.

#### 4. `crates/server/src/lib.rs` — Add module

Add `pub mod transition;`.

### Verification
#### Automated
- [ ] `cargo check-all` passes
- [ ] `cargo test-all` passes

#### Manual
- [ ] `cargo server` + `cargo client` — mid-game transition uses split phases. Character despawns/respawns correctly. Initial connect still works (unchanged path).

---

## Phase 5: Unified Initial Connection Path

Route initial connection through the same transition state machine. Remove `spawn_overworld`.

### Changes

#### 1. `crates/client/src/transition.rs` — Add `on_transition_start_received`

This unified handler replaces `handle_map_transition_start` for both paths:

```rust
/// Handles MapTransitionStart for both initial connect and mid-game.
/// Shows loading screen, despawns old maps, clears prediction state,
/// freezes player (if exists), spawns new VoxelMapInstance, starts state machine.
pub fn on_transition_start_received(
    mut commands: Commands,
    mut receivers: Query<&mut MessageReceiver<MapTransitionStart>>,
    mut registry: ResMut<MapRegistry>,
    terrain_registry: Res<TerrainDefRegistry>,
    player_query: Query<Entity, (With<Predicted>, With<CharacterMarker>, With<Controlled>)>,
    world_objects: Query<(Entity, &MapInstanceId), With<WorldObjectId>>,
    mut transition_state: ResMut<ClientTransitionState>,
    mut prediction_state: ResMut<VoxelPredictionState>,
    mut next_transition: ResMut<NextState<MapTransitionState>>,
) {
    for mut receiver in &mut receivers {
        for transition in receiver.receive() {
            trace!("MapTransitionStart target={:?}", transition.target);

            // Show loading screen
            next_transition.set(MapTransitionState::Transitioning);

            // Clear prediction state
            prediction_state.pending.clear();

            // Despawn old maps (no-op on initial connect)
            despawn_all_maps_except(&mut commands, &mut registry, &transition.target);
            despawn_foreign_world_objects(&mut commands, &world_objects, &transition.target);

            // Freeze player if one exists (mid-game only)
            if let Ok(player) = player_query.get_single() {
                commands.entity(player).insert((
                    RigidBodyDisabled, ColliderDisabled, DisableRollback,
                    PendingTransition(transition.target.clone()),
                    Position(transition.spawn_position),
                    LinearVelocity(Vec3::ZERO),
                ));
            }

            // Spawn new VoxelMapInstance if not already in registry
            let map_entity = if let Some(&existing) = registry.0.get(&transition.target) {
                existing
            } else {
                let e = spawn_map_from_transition(&mut commands, &transition, &terrain_registry);
                registry.0.insert(transition.target.clone(), e);
                e
            };

            // Update ChunkTicket on player if exists
            if let Ok(player) = player_query.get_single() {
                commands.entity(player).insert(ChunkTicket::map_transition(map_entity));
            }
            // On initial connect, no player exists yet. The server's ChunkTicket
            // on the server-side character entity drives chunk sending via
            // push_chunks_to_clients. Client doesn't need a local ChunkTicket
            // to receive chunks — handle_chunk_data_sync processes incoming
            // ChunkDataSync regardless.

            // Start state machine
            transition_state.begin(&transition);
        }
    }
}
```

`spawn_map_from_transition` is extracted from the existing `handle_map_transition_start` logic (terrain def lookup, VoxelMapInstance bundle spawn). `despawn_all_maps_except` and `despawn_foreign_world_objects` are moved here from `client/src/map.rs` (made `pub` or inlined).

Gate `on_transition_start_received` with `.run_if(resource_exists::<TerrainDefRegistry>)` instead of `Option<Res<_>>`. This follows CLAUDE.md: "NEVER use `Option<Res<_>>` unless there is a legitimate reason." By the time messages arrive, the client is in `AppState::Ready` and all assets are loaded.

#### 2. `crates/client/src/map.rs` — Remove old functions

- **Remove** `spawn_overworld` (lines 98-128).
- **Remove** `handle_map_transition_start` (lines 418-485).
- **Remove** `OnEnter(AppState::Ready) => spawn_overworld` registration.
- **Remove** `handle_map_transition_start` registration.
- **Move** `despawn_all_maps_except` and `despawn_foreign_world_objects` to `client/src/transition.rs`.
- Keep: `handle_chunk_data_sync`, `attach_chunk_ticket_to_player`, voxel edit systems, `MapRegistry`, `VoxelPredictionState`.

#### 3. `crates/server/src/gameplay.rs` — Server-driven initial connect

Modify `handle_connected` (lines 352-418):

- Spawn character with all components as before, including `MapInstanceId::Overworld`. The `on_map_instance_id_added` observer fires and calls `AddEntity(player)` to overworld room — this is fine because the client's sender is NOT yet in the room, so nothing replicates.
- **Remove** the `AddSender(client_entity)` trigger at lines 413-417.
- **Insert** `TransitionPending` on the character entity.
- **Send** `MapTransitionStart` to the client.
- Add `senders: Query<&mut MessageSender<MapTransitionStart>>` and map params to signature.

```rust
// Replace lines 413-417 with:
let room = room_registry.get_or_create(&MapInstanceId::Overworld, &mut commands);

// Insert TransitionPending — Phase 2 (complete_map_transition) will AddSender
commands.entity(character_entity).insert(TransitionPending {
    client_entity,
    target_map_id: MapInstanceId::Overworld,
    new_room: room,
    relocated_entities: vec![character_entity],
});

// Send MapTransitionStart
let mut sender = start_senders.get_mut(client_entity)
    .expect("Client must have MessageSender<MapTransitionStart>");
sender.send::<MapChannel>(MapTransitionStart {
    target: MapInstanceId::Overworld,
    seed: config.seed,
    generation_version: config.generation_version,
    bounds: dimensions.bounds,
    spawn_position: spawn_pos,
    chunk_size: dimensions.chunk_size,
    column_y_range: dimensions.column_y_range,
    readiness_radius: TRANSITION_READINESS_RADIUS,
});
```

Note on `on_map_instance_id_added` (server/src/map.rs:604-620): fires on `Add<MapInstanceId>`, calls `AddEntity(entity)` to room unconditionally. The character entity is in the overworld room but invisible to the client (no `AddSender` yet). When `complete_map_transition` fires Phase 2, it calls `AddSender(client)` — at that point the client sees all room entities. It also calls `AddEntity(player)` which is a no-op duplicate (entity already in room). Lightyear handles duplicate room additions gracefully.

#### 4. `crates/ui/src/lib.rs` — Connection flow

`on_client_connected` keeps setting `ClientState::InGame` — this enables the `MapTransitionState` sub-state. The transition loading screen is managed by `MapTransitionState::Transitioning`, set by `on_transition_start_received` when the server's `MapTransitionStart` arrives.

**Modify** `map_switch_button_interaction`: replace guard `*transition_state.get() == Transitioning` with `state.phase != TransitionPhase::Idle` using `Res<ClientTransitionState>`. Remove `next_transition.set(MapTransitionState::Transitioning)` — the loading screen now appears on `MapTransitionStart` receipt, not button press.

#### 5. Update system registration in `crates/client/src/lib.rs`

```rust
app.add_systems(
    Update,
    (
        transition::on_transition_start_received,
        transition::receive_transition_end,
        transition::receive_transition_entities,
        transition::update_transition_state,
    )
        .chain()
        .run_if(in_state(ClientState::InGame))
        .run_if(resource_exists::<TerrainDefRegistry>),
);
```

### Verification
#### Automated
- [ ] `cargo check-all` passes
- [ ] `cargo test-all` passes

#### Manual
- [ ] `cargo server` + `cargo client` — fresh connect: loading screen, spatial readiness, player on solid terrain.
- [ ] Mid-game transition still works.
- [ ] `spawn_overworld` gone.

---

## Phase 6: Entity Guards and Stale Entity Cleanup

Add per-handler `MapInstanceId` checks and safety-net cleanup system.

### Changes

#### 1. `crates/client/src/world_object.rs` — `MapInstanceId` guard

Add `registry: Res<MapRegistry>` and `map_id_query: Query<&MapInstanceId>` to `on_world_object_replicated` params. At top of loop:

```rust
for (entity, id) in &query {
    if let Ok(entity_mid) = map_id_query.get(entity) {
        if !registry.0.contains_key(entity_mid) {
            trace!("Despawning stale world object {entity:?} from map {entity_mid:?}");
            commands.entity(entity).despawn_recursive();
            continue;
        }
    }
    // ... existing visual setup ...
}
```

#### 2. `crates/client/src/gameplay.rs` — Fix hardcoded Overworld + guard

In `handle_new_character` (lines 66-71):
- **Remove** the `MapInstanceId::Overworld` insertion. The entity's `MapInstanceId` comes from replication.
- **Add** guard against stale characters:

```rust
for entity in &character_query {
    if let Ok(mid) = map_ids.get(entity) {
        if !registry.0.contains_key(mid) {
            trace!("Despawning stale character {entity:?} from map {mid:?}");
            commands.entity(entity).despawn_recursive();
            continue;
        }
    }
    commands.entity(entity).insert(CharacterPhysicsBundle::default());
}
```

Add `registry: Res<MapRegistry>` and `map_ids: Query<&MapInstanceId>` to system params.

#### 3. `crates/client/src/transition.rs` — Safety-net cleanup

```rust
/// Per-frame safety net: despawn any Replicated entity whose MapInstanceId
/// doesn't match any registered map. Primary defense is per-handler guards;
/// this catches omissions. Only runs during transitions.
pub fn cleanup_stale_map_entities(
    mut commands: Commands,
    registry: Res<MapRegistry>,
    stale_query: Query<(Entity, &MapInstanceId), With<Replicated>>,
    state: Res<ClientTransitionState>,
) {
    if state.phase == TransitionPhase::Idle {
        return; // Only needed during/immediately after transitions
    }
    for (entity, mid) in &stale_query {
        if !registry.0.contains_key(mid) {
            trace!("Safety-net: despawning stale entity {entity:?} map {mid:?}");
            commands.entity(entity).despawn_recursive();
        }
    }
}
```

Register in client plugin, running after the transition state machine.

### Verification
#### Automated
- [ ] `cargo check-all` passes
- [ ] `cargo test-all` passes

#### Manual
- [ ] Transition repeatedly between maps. No stale world objects or characters visible.
- [ ] Homebase transition: character has correct MapInstanceId.

---

## Phase 7: Cleanup and Polish

Remove dead code, update README.

### Changes

#### 1. `crates/client/src/map.rs` — Remove dead code

Remove transition functions replaced in earlier phases (if not already removed):
- `check_transition_chunks_loaded`, `handle_map_transition_end`, `spawn_overworld`, `handle_map_transition_start`
- `despawn_all_maps_except`, `despawn_foreign_world_objects` (moved to transition.rs)
- Dead imports

#### 2. `crates/client/src/map.rs` + `crates/server/src/map.rs` — Remove `OverworldMap`

Remove the `OverworldMap` resource definition and its insertion on both client (`client/src/map.rs:95-96`) and server (`server/src/map.rs:62-63`). Confirmed dead code — inserted once, never read by any system. All map lookups go through `MapRegistry`.

#### 3. `crates/server/src/map.rs` — Remove dead code

Remove `execute_server_transition`, `handle_map_transition_ready`, dead imports.

#### 4. `crates/ui/src/state.rs`

`MapTransitionState` stays here. UI crate owns display state; client transition module drives it.

#### 5. `README.md`

Review and update if transition architecture is documented.

### Verification
#### Automated
- [ ] `cargo check-all` passes
- [ ] `cargo test-all` passes
- [ ] No unused function/import warnings

#### Manual
- [ ] Full regression: fresh connect, overworld→homebase, homebase→overworld, rapid transitions
- [ ] Loading screen shows phase info
- [ ] No stale entities after any transition

---

## Testing Checkpoints (Cumulative)

| Phase | Checkpoint |
|-------|-----------|
| 1 | `transition/` module exists, compiles, no behavior change |
| 2 | Client propagator has sources; chunks mesh closest-first |
| 3 | Mid-game transition waits for spatial readiness before sending Ready |
| 4 | Server splits room changes into Phase 1/2. TransitionPending + MapTransitionEntity flow works |
| 5 | Initial connect and mid-game use identical client flow. `spawn_overworld` gone. Predictions cleared |
| 6 | Stale entities caught by per-handler guards and safety-net cleanup |
| 7 | No dead transition code. README updated |

## Follow-Up Items (Not In Scope)

- **Transition timeout**: Add `started_at` timestamp to `ClientTransitionState`. If any phase exceeds a configurable threshold (e.g. 60s), log error and reset to Idle. Handles server crash mid-transition.
- **ChunkColumnReady marker**: Decouple readiness check from voxel engine internals by having the engine emit a marker when a column completes the full pipeline. Reduces coupling but adds engine complexity.
