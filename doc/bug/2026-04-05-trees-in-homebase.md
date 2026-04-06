---
date: 2026-04-05T21:00:00-07:00
researcher: Claude
git_commit: 638eb180
branch: qrspi-commands
repository: bevy-lightyear-template
topic: "Trees spawning in homebases"
tags: [bug, lightyear, replication, rooms, world-objects, eviction]
status: in-progress
last_updated: 2026-04-05
last_updated_by: Claude
---

# Bug: Overworld trees persist on client after map transition to homebase

**Date**: 2026-04-05T21:00:00-07:00
**Researcher**: Claude
**Git Commit**: 638eb180
**Branch**: qrspi-commands
**Repository**: bevy-lightyear-template

## User's Prompt

Trees are being spawned in homebases when there is no PlacementRules dictating so. It might be that the world objects are not properly being assigned to rooms.

## Summary

Overworld tree entities replicated via lightyear persist on the client after the player transitions from overworld to homebase. The server correctly generates trees only for the overworld (homebase terrain def is empty). The root cause is a timing mismatch between entity eviction (Update) and replication visibility processing (PostUpdate).

## Investigation

### Server-side generation is correct

- `homebase.terrain.ron` is empty `{}` — no `PlacementRules`, no `HeightMap`
- Server uses `FlatGenerator` for homebases → `place_features()` returns empty vec
- No stale entity files in `worlds/homebase-0/`
- Server debug logs confirm: trees only spawned for `map=Overworld`

### Client does not generate entities locally

- Client sets `generates_chunks = false` for all maps
- World objects only appear via lightyear replication (`Added<Replicated>`)

### Room system is correctly configured

- `RoomPlugin` is registered (`map.rs:434`)
- `on_map_instance_id_added` observer adds entities to rooms via `RoomEvent::AddEntity`
- Map transitions properly call `RemoveSender` / `AddSender` (`map.rs:957-972`)
- Server debug confirmed: after transition, surviving tree entities show `visibility: Default, spawned: false` — the visibility-lost cycle completed for them

### Two despawn paths in lightyear

Lightyear has two ways to send entity despawns to clients:

1. **Visibility-lost path** (buffer system, `buffer.rs:362-364`): During per-frame buffer iteration, checks `lost_visibility = (visibility == Lost) && spawned`. Requires entity to be alive. Runs in PostUpdate/Buffer.

2. **Entity-despawn observer** (`buffer.rs:557-598`): Observer on `On<Remove, (Replicate, ReplicationState, ReplicateLike)>`. Fires during command flush when entity is despawned. Queries `With<Replicating>` and filters by `is_visible()`.

### The timing mismatch

During map transition, two things happen in the same Update frame:

1. `handle_map_switch_requests` → `commands.trigger(RoomEvent::RemoveSender(client))` (deferred)
2. `evict_chunk_entities` → `commands.entity(entity).despawn()` for out-of-range entities (deferred)

During command flush:
- `handle_room_event` observer fires → writes `lose_visibility` into `RoomEvents` resource for ALL entities in the room
- Entity despawns execute → `buffer_entity_despawn_replicate_remove` observer fires

In PostUpdate:
- `apply_room_events` (BeforeBuffer) → tries to set `VisibilityState::Lost` on each entity's `ReplicationState` — but the 72 evicted entities are already despawned, so the query misses them
- Buffer (Buffer) → processes the 12 surviving entities with `Lost` + `spawned=true` → sends despawn to client ✓
- The 72 evicted entities were supposed to be handled by the despawn observer, but it fails

### Why the despawn observer fails

The observer at `buffer.rs:562-564` requires `With<Replicating>`:
```rust
entity_query: Query<
    (&ReplicationGroup, &ReplicationState, Has<NetworkVisibility>),
    With<Replicating>,
>,
```

During `commands.entity(e).despawn()`, Bevy removes components in archetype-internal order. If `Replicating` is removed before `Replicate` or `ReplicationState`, the observer fires but the query fails → returns early → no despawn sent.

**Evidence**: Server log shows 84 → 12 entities after transition. Client log shows only 12 `Replicated REMOVED` events. The remaining 78 client entities persist indefinitely with `map=Some(Overworld)`.

## Code References

- `crates/server/src/world_object.rs:31-38` — world object spawn with `Replicate::to_clients(NetworkTarget::All)`
- `crates/server/src/chunk_entities.rs:97-149` — `evict_chunk_entities` despawns entities in Update
- `crates/server/src/map.rs:414-430` — `on_map_instance_id_added` observer (room assignment)
- `crates/server/src/map.rs:936-972` — map transition room events
- `git/lightyear/lightyear_replication/src/send/buffer.rs:557-598` — despawn observer
- `git/lightyear/lightyear_replication/src/visibility/room.rs:92-192` — `handle_room_event`
- `git/lightyear/lightyear_replication/src/visibility/room.rs:194-251` — `apply_room_events` (PostUpdate)

## Hypotheses

### H1: Trees replicate to homebase clients due to missing NetworkVisibility at spawn time

**Hypothesis:** `Replicate::to_clients(NetworkTarget::All)` registers all clients immediately, but `NetworkVisibility` (added by deferred observer) isn't present until the next command flush. During that window, entities replicate to all clients.
**Prediction:** Server debug would show trees spawned only for Overworld. Client debug would show overworld trees arriving.
**Test:** Added `warn!` in spawn and replication paths.
**Decision:** Invalidated — server only spawns for Overworld, client only receives Overworld trees. No new replication occurs after transitioning.

### H2: Replicated world object entities persist on client after map transition

**Hypothesis:** Overworld tree entities are not children of the map entity. Lightyear should despawn them when room visibility is lost, but they persist.
**Prediction:** Client-side timer logging `WorldObjectId` entities would show stale overworld trees alive after transition.
**Test:** Added 2-second timer system logging all world object entities with map IDs.
**Decision:** Validated — 78+ overworld trees persist on client with `map=Some(Overworld)` after transition to homebase.

### H3: Lightyear despawn observer fails due to Replicating component removal order

**Hypothesis:** During entity despawn, Bevy may remove `Replicating` before `Replicate`/`ReplicationState`. The despawn observer requires `With<Replicating>`, so the query fails and no despawn is sent.
**Prediction:** Entities going through visibility-lost path (alive on server) would be despawned on client. Entities going through despawn observer (evicted on server) would persist.
**Test:** Added client-side observer on `Remove<Replicated>` to detect despawns. Added server-side `ReplicationState` debug logging.
**Decision:** Consistent with evidence — 12 visibility-lost despawns work, 78 observer despawns fail. But not conclusively proven — could also be the timing issue (entities despawned before `apply_room_events` runs).

## Fixes

### F1: Add NetworkVisibility to all room-scoped Replicate entity spawns

**Root Cause:** Without `NetworkVisibility`, `is_visible(false)` always returns `true`, bypassing room filtering.
**Fix:** Add `NetworkVisibility` to spawn bundles of `spawn_world_object`, `spawn_dummy_target`, `handle_connected`.
**Risk:** Low.
**Decision:** Necessary but insufficient — entities still persist because the despawn path fails.

### F2: Deferred eviction via PendingEviction marker

**Root Cause:** `evict_chunk_entities` despawns entities in Update before PostUpdate replication systems can process visibility loss.
**Fix:** Remove entities from room + mark `PendingEviction` in Update. Despawn in PostUpdate after buffer flush.
**Risk:** Low — one frame delay before entity cleanup.
**Decision:** Tested, user reported it did not work. Needs re-investigation — may have been a build/test issue, or the timing is still wrong.

### H4: Most entities not in ReplicableRootEntities — Bevy flush is non-recursive

**Hypothesis:** `Replicate::on_insert` hook adds entities to `ReplicableRootEntities` via `world.commands().queue(...)` (deferred). Bevy's `flush_commands` does a single `apply_or_drop_queued` — it does NOT loop. Commands generated during the flush are left for the next flush cycle. Most entities are spawned in `spawn_chunk_entities` and their hook commands aren't processed until a later flush, so they're never in `ReplicableRootEntities` when the buffer runs on a send tick.

**Prediction:** `ReplicableRootEntities.len()` at buffer send time would be much smaller than the room entity count.

**Test:** Added `warn!` in buffer system logging `ReplicableRootEntities` size at each send tick.

**Decision:** Validated — room has 2199 entities but `ReplicableRootEntities` has only 24-97 at send time. `apply_room_events` sets `Lost` on all 2199 entities, but the buffer only iterates the 24-97 in `ReplicableRootEntities`. Entities not in the set never get `prepare_entity_despawn` called.

**Root Cause Chain:**
1. `spawn_chunk_entities` queues `commands.spawn(...)` for world objects
2. First flush: entities created → `Replicate::on_insert` hook fires → `world.commands().queue(add to ReplicableRootEntities + populate per_sender_state)` — queues another command
3. Bevy `flush_commands` does NOT loop — the hook's command stays in the queue
4. The command is processed on the NEXT flush (next frame or next `apply_deferred`)
5. Meanwhile, room events fire immediately via observers (adding entities to `room.entities`)
6. Room has 2199 entities, but `ReplicableRootEntities` only has the entities whose hook commands were flushed
7. On map transition, `RemoveSender` → `lose_visibility` for all 2199 → `apply_room_events` sets `Lost` on all
8. Buffer only iterates `ReplicableRootEntities` (24-97 entities) → only those get despawns
9. The other ~2100 entities have `Lost` set but never get processed by the buffer

## Fixes

### F1: Add NetworkVisibility to all room-scoped Replicate entity spawns

**Root Cause:** Without `NetworkVisibility`, `is_visible(false)` always returns `true`, bypassing room filtering.
**Fix:** Add `NetworkVisibility` to spawn bundles of `spawn_world_object`, `spawn_dummy_target`, `handle_connected`.
**Risk:** Low.
**Decision:** Necessary but insufficient — entities still persist because of H4.

### F2: Move evict_chunk_entities to PostUpdate after buffer flush

**Root Cause:** `evict_chunk_entities` despawns entities in Update before PostUpdate replication systems can process visibility loss.
**Fix:** Move `evict_chunk_entities` to PostUpdate after `ReplicationBufferSystems::Flush`.
**Risk:** Low.
**Decision:** Necessary but insufficient — entities survive to PostUpdate, `Lost` is set, but the buffer skips most entities because they're not in `ReplicableRootEntities` (H4).

### F3: Pending — needs to address H4

The core issue is that `ReplicableRootEntities` is populated via a deferred command from `Replicate::on_insert`, and Bevy's `flush_commands` is non-recursive. Solutions must either:
- (a) Ensure all entities are in `ReplicableRootEntities` before the buffer processes them (requires upstream lightyear fix to use non-deferred insertion)
- (b) Use a different mechanism to send despawns that doesn't depend on `ReplicableRootEntities` (e.g., iterate entities with `Lost` visibility directly via a query)
- (c) Client-side cleanup: despawn replicated entities whose `MapInstanceId` doesn't match the current map during transition

## Solutions

*Pending — F3 approach needs to be chosen*
