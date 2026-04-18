# Research Questions

## Context
Focus on the `dev` crate, the `voxel_map_engine` chunk lifecycle, the ability asset pipeline in `protocol/src/ability/`, Lightyear replication/interest-management in `server/src/map.rs` and `protocol/src/lib.rs`, the reflect registrations across plugins, and workspace-wide Cargo structure. The client/server/web binaries show how `DevPlugin` is composed into the running app.

## Questions

1. What does `crates/dev/src/lib.rs` currently contain, how is `DevPlugin` wired into the client/server/web binaries, and what Bevy version + vendored dependency patches (`git/bevy`, `git/lightyear`, `git/avian`) constrain which versions of `bevy-inspector-egui` or `bevy_egui` could be added?

2. How does `voxel_map_engine` represent a chunk's lifecycle state (loaded, unloaded, streaming, generating, meshing)? What components/resources/events carry this state, and what public APIs in `api.rs`, `ticket.rs`, or `lifecycle.rs` would an external caller use to force-load or evict a specific chunk?

3. Trace the full path of an ability RON file from disk to runtime: where the files live, how `AbilityAssetLoader` reads them via `AppTypeRegistry`, what shared infrastructure (e.g. `reflect_loader.rs`, `ComponentMapDeserializer`) it relies on, how `AssetServer` hot-reload/file-watch is configured for abilities, and what happens to in-flight entities when an `AbilityAsset` is modified and reloaded.

4. Trace the full path of a world-object RON file from disk to runtime: where the files live, how the world-object loader in `protocol/src/world_object/` reads them via `AppTypeRegistry`, how it uses the shared reflect infrastructure, how hot-reload behaves, and what happens to already-spawned world-object entities when their definition asset changes. Note any meaningful differences from the ability pipeline.

5. How is Lightyear interest management implemented on the server — what does `RoomRegistry` track, how are clients assigned to rooms, where is `NetworkVisibility` computed, and is there any notion of per-entity radius or range (vs. pure room membership) in the codebase today?

6. Across `ability/plugin.rs`, `world_object/plugin.rs`, `voxel_map_engine/src/lib.rs`, and `protocol/src/lib.rs`, what is the complete set of types registered with `AppTypeRegistry` and which of those are `Component`s vs. data-only reflected types? What existing patterns (e.g. `ComponentMapDeserializer`) convert a reflected type registry entry into a live entity?

7. What Cargo features or conditional compilation patterns already exist in this workspace (per-crate `[features]`, `cfg` gates, dev-only dependencies), and how does `DevPlugin`'s existing `hide_physics_debug` demonstrate the project's current approach to toggling debug behavior at runtime?

8. How does a client currently learn which entities it owns or can see — what Lightyear components/markers (`Replicated`, `ReplicationTarget`, connection IDs, predicted/interpolated markers) are visible on the client side, and where on the server is the mapping "this entity is replicated to these client IDs" authoritative?

9. What is the startup and plugin-registration order in `client/main.rs`, `server/main.rs`, and `web/main.rs`, at what point does `AppState` transition to `Ready`, and where in that order does `DevPlugin` sit relative to plugins that own the systems a debug UI would want to read from (voxel map, abilities, networking)?

10. What wasm-specific constraints apply to the `web` crate — specifically: does the current build enable `AssetServer` file-watching on wasm (and can it), does the project's wasm target have any filesystem write access for RON hot-editing, how does the web client load assets (HTTP fetch vs. bundled), and does any existing code already gate functionality on `target_arch = "wasm32"` or a `web` Cargo feature?

11. What are the known wasm constraints and limitations of `bevy-inspector-egui` (and its underlying `bevy_egui`) at the Bevy version used here — officially supported or not, which features or widgets are known to break or degrade on wasm (file dialogs, clipboard, pointer/keyboard capture, performance), and are there any open upstream issues or required feature flags for wasm builds? Make sure the constraints are relevant to the current versions
