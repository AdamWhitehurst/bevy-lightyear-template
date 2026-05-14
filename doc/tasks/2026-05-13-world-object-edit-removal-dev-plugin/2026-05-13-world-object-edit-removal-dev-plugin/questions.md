# Research Questions

## Context

Focus on the development tooling around inspector panels, the world-object definition and placement lifecycle, and how replicated object entities are
represented across client and server. Also inspect existing input, picking/raycast, persistence, and component-reflection patterns that affect runtime
object mutation.

## Questions

1. How is the development plugin assembled and feature-gated, and how do its existing panels manage UI state, keyboard toggles, `egui` rendering, and
   access to ECS resources?
2. How does the current world-object definition pipeline load, register, clone, and apply reflected components from `.object.ron` assets on both
   server and client?
3. What is the full flow for def-driven world-object placement, from client UI state through terrain interaction and network messages to authoritative
   server spawning and client replication?
4. How are world-object entities identified after spawning, including `WorldObjectId`, map/chunk ownership, replication markers, visual children,
   physics components, and any stable or persisted entity references?
5. What existing patterns remove, despawn, transform, or replace world-object-related entities or components, and how do those paths handle children,
   colliders, replicated state, and chunk/map bookkeeping?
6. What runtime editing mechanisms already exist through reflection, the world inspector, or component insertion/removal helpers, and which
   world-object components are registered or marked specially for reflection, persistence, or spawn-only behavior?
7. How do input focus, terrain raycast/picking, camera context, and UI interaction currently determine which world position or entity is being
   targeted in client gameplay and dev tooling?
8. What tests cover world-object placement, rejection, replication, persistence, and lifecycle changes, and what test utilities or app setup patterns
   do they use?
