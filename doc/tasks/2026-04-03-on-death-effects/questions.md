# Research Questions

## Context
Focus on the world object lifecycle (spawn, health, death, despawn), the hit detection and damage pipeline, lightyear replication registration patterns, and how tick-based timers are used for delayed gameplay actions. Also examine how world object definitions are deserialized from RON and how their components are applied to entities. Investigate lightyear's entity/child-entity replication ordering and potential race conditions. Examine the `ReflectPersist` type data marker and how component persistence works during chunk eviction.

## Questions

1. How does a world object entity go from full health to "dead"? Trace the damage application pipeline from hit detection through `Health` mutation, and identify where/how death is currently detected (polling vs. events vs. observers).

2. What happens to a world object entity after death is detected? Is there a respawn flow, immediate despawn, or does the entity persist with a `Dead` marker? How does `RespawnTimerConfig` interact with world objects specifically (vs. characters)?

3. How are world object entities spawned on the server? Trace `spawn_world_object()` — what components does it insert, how does it associate with a chunk, and what lightyear replication components (e.g. `Replicate`) are attached? Which of those components are replicated to clients vs. server-only?

4. How does the `WorldObjectDef` RON deserialization work end-to-end? Trace from the `.object.ron` file through the custom asset loader, `TypeRegistry` resolution, and `apply_object_components` insertion. What must a new component type satisfy to be deserializable from RON and insertable via this pipeline?

5. How are components registered with lightyear for replication? What is the difference between replicated-only and prediction-enabled components, and what determines which category a component falls into? List all current registration calls.

6. How do tick-based timers work in this codebase (e.g. `RespawnTimer`)? How is the current tick accessed, how is expiry checked, and what system ordering constraints exist for timer-driven state transitions? How will these timers be affected by saving and reloading?

7. How does the server manage the relationship between world object entities and their parent chunks? When a world object is despawned and a new one spawned at the same position, how does chunk ownership transfer or persist?

8. What patterns exist for enum-based effect dispatch in the codebase (e.g. `OnHit` effects)? How are effect variants defined, matched, and executed? Are there any patterns for composing multiple effects in sequence?

9. How does the client resolve a replicated `WorldObjectId` component into a visual entity? What is the full set of components on a client-side world object entity vs. the server-side entity — are they symmetric, or does the client add/omit components? What compensation (if any) does the client perform after replication to reconstruct a working entity? What happens visually when a world object entity is despawned and a new one appears at the same position?

10. How does lightyear handle replication of child entities and ordering for entities with child entities or dependent components? What race conditions can occur when the server spawns or despawns an entity and spawns a replacement in the same tick — does the client see a gap, a flicker, or can the operations be batched? What guarantees does lightyear provide about replication ordering within a single tick?

11. How does lightyear handle component addition/removal on an already-replicated entity? If the server mutates components on a live entity (e.g. swapping `WorldObjectId` and `VisualKind` in-place instead of despawn+respawn), does the client receive those changes reliably and atomically?

12. How does chunk eviction currently work? When a chunk is unloaded, what happens to its world object entities — are any components saved, or is everything discarded? What reflect-based type data markers exist in the codebase (e.g. `FromType<T>` implementations), and how are they registered and queried at runtime? What would a component need to satisfy to be serializable/deserializable via the reflect system for persistence purposes?

13. How does entity reloading from saved/persisted data work? When a chunk is reloaded and its world object entities are restored, what is the full flow — what data is read, how are entities reconstructed, and which components are re-applied? Trace all code paths involved in restoring a previously-saved world object entity.

14. How is `PlacementOffset` applied during world object spawning? Is it applied additively to the entity's position, and if so, what happens when an entity is reloaded from persistence — does `PlacementOffset` get re-applied on top of an already-offset position? Trace every code path where `PlacementOffset` is read or used.

15. What distinction exists (or could exist) between "spawn-time-only" operations and "reload-safe" operations in the world object spawning pipeline? Are there any components or systems that should only run on first spawn but currently also run on reload? How does `apply_object_components` differentiate (or fail to differentiate) between a fresh spawn and a reload?

16. How do map instances and rooms work in this project? How is `MapInstanceId` assigned to entities, how does lightyear's room system control which clients see which entities, and how are world object entities associated with a specific map instance? Trace the full flow from map creation through entity room assignment.

17. When a new entity is spawned on the server mid-game (not during initial chunk loading), what steps are required to ensure it is placed in the correct lightyear room and associated with the correct `MapInstanceId`? Are there helper functions or patterns that handle this, or must each spawn site manually configure room membership?

18. How does this project and/or lightyear handle deterministic randomness (e.g. seeded RNG) for gameplay systems that must produce consistent results across server and clients? Are there existing patterns for seed propagation, per-entity or per-tick RNG, or must we rely purely on server-authority with client replication? How does lightyear's prediction/rollback interact with non-deterministic operations like random loot selection?
