# Design Discussion

## Current State

- `DevPlugin` is 30 lines (`crates/dev/src/lib.rs:1-30`) and does one thing: installs `avian3d::PhysicsDebugPlugin`, hides it at `Startup` (`lib.rs:19-22`), and toggles it with F3 in `Update` (`lib.rs:25-30`).
- `DevPlugin` is added on client (`client/main.rs:55`) and web (`web/main.rs:41`). **Not** added on server (`server/main.rs` has no `dev` dep).
- No Cargo features, no `cfg` gates anywhere in `dev`. `avian3d` pulled unconditionally with `debug-plugin` (`crates/dev/Cargo.toml:7`).
- `bevy-inspector-egui` / `bevy_egui` are absent from the workspace; compatible versions for Bevy 0.18 are `bevy-inspector-egui 0.36` + `bevy_egui 0.39` (vendored inside `git/lightyear/Cargo.toml:232,238`, officially WASM-capable).
- Voxel chunk loading is driven by `ChunkTicket { map_entity, ticket_type, radius }` entities via `collect_tickets` (`crates/voxel_map_engine/src/lifecycle.rs:414`). `TicketType` has `Player`/`Npc`/`MapTransition` (`ticket.rs:5-12`). No public force-load API.
- Ability + world-object pipelines share `deserialize_component_map` (`crates/protocol/src/reflect_loader.rs:56-64`) and apply via `ReflectComponent::insert` (`ability/loader.rs:40-55`, `world_object/spawn.rs:14-37`). Hot-reload detects `AssetEvent::Modified` and refreshes registries (`ability/loading.rs:151-219`, `world_object/loading.rs:164-185`) but **does not re-patch already-spawned entities**.
- Interest management is purely room-based. `RoomRegistry.0`, `Room.clients`, `Room.entities` are all `pub` (`server/src/map.rs:45-56`, `git/lightyear/lightyear_replication/src/visibility/room.rs:62-67`). Lightyear's per-sender state (`ReplicationState.per_sender_state`) is `pub(crate)` — not client-accessible.
- Client-visible replication markers: `Replicated`, `Predicted`, `Interpolated`, `Controlled` (`client/src/gameplay.rs:5`).
- `bevy/file_watcher` is enabled on client + server; **absent on web** (`crates/web/Cargo.toml:15-38`). WASM assets are HTTP-fetched via Trunk; no filesystem write path exists.

## Desired End State

`DevPlugin` grows six debug panels powered by `bevy-inspector-egui`, each gated both at compile time (Cargo feature) and at runtime (toggle resource + hotkey):

1. **World inspector** — standard `WorldInspectorPlugin`.
2. **Spawn panel** — dual mode: (a) pick a registered `AbilityId`/`WorldObjectId` and spawn via existing `apply_*` pipeline; (b) free-form picker over `AppTypeRegistry` entries with `#[reflect(Component)]`.
3. **Network entity viewer** — on the client, list replicated entities with their markers (`Predicted`/`Interpolated`/`Controlled`) and `Replicated::from` sender.
4. **Chunk debugger** — list columns from `VoxelMapInstance.chunk_levels`, show `ChunkStatus` per chunk, buttons to force-load / force-evict a column.
5. **Ability editor** — live RON edit for each `Handle<AbilityAsset>`; native writes back to disk and lets the existing watcher fire; WASM edits `Assets<AbilityAsset>` directly.
6. **World-object editor** — same pattern against `Assets<WorldObjectDef>`.

Verification:
- `cargo build -p dev` with no features compiles to ~today's size (no egui dependency pulled).
- `cargo build -p dev --features inspector,chunk-debug` compiles, F3 still toggles gizmos, F4 toggles the egui root, panel sub-toggles work.
- Editing `assets/abilities/dash.ability.ron` via the editor on a native client updates live entities without despawn/respawn.
- Force-evicting a column removes its mesh entity within one tick.

## Patterns to Follow

- **Runtime toggle**: `keys.just_pressed(KeyCode::F3)` → flip a `bool` in `Update` (`crates/dev/src/lib.rs:25-30`). New panel toggles mirror this with F4–F9.
- **Reflect → live entity**: `registry.get_with_type_path(path).and_then(|r| r.data::<ReflectComponent>())` then `reflect_component.insert(&mut entity_mut, component, &registry)` (`protocol/src/ability/loader.rs:40-55`, `world_object/spawn.rs:14-37`). Spawn panel (free-form) and re-patch path must reuse this.
- **WASM gating**: `#[cfg(target_arch = "wasm32")]` blocks in protocol crates (`ability/loading.rs`, `world_object/loading.rs`). New WASM-vs-native divergence (disk write vs. in-memory edit) uses the same gate.
- **Cargo features**: `[features]` section with `default = [...]` and additive flags, matching `client/Cargo.toml:7-9`. Optional deps declared with `dep:name` under the feature.
- **Room read pattern**: `Res<RoomRegistry>` → `Query<&Room>` is already used by `flush_voxel_broadcasts` (`server/src/map.rs:838-842`). Server-side panels, if ever added, would reuse this — but this task does not add them.
- **Ticket-based chunk control**: spawn an entity with `ChunkTicket` + `GlobalTransform` to load; despawn it to evict (`ticket.rs:61-85`, `lifecycle.rs:414-445`).

Patterns to **avoid**:
- Do not depend on `ReplicationState.per_sender_state` — it's `pub(crate)` and would require a Lightyear patch.
- Do not add another copy of `clone_def_components` (already duplicated at `server/src/world_object.rs:134` and `client/src/world_object.rs:90`). If the re-patch path needs it, extract it into `protocol/src/world_object/` once.
- Do not introduce bare `return`/`continue` without a `trace!` per CLAUDE.md.

## Design Decisions

1. **Two-layer gating (Q1 = C).** Umbrella Cargo feature `inspector` on `crates/dev` pulls `bevy-inspector-egui` + `bevy_egui`. Per-panel features `world-inspector`, `spawn-panel`, `netviz`, `chunk-debug`, `ability-editor`, `world-object-editor` each depend on `inspector`. Each enabled panel also honors a runtime `bool` in a single `DevInspectorState` resource, flipped by an F-key and surfaced in a top egui menu. Why: matches existing runtime-toggle pattern, lets devs build a stripped binary that pays zero code + zero deps for panels they disable, and still allows hiding a panel without rebuilding when it's compiled in.

2. **Client-only DevPlugin (Q2 = A).** No new server integration, no Lightyear dev-message protocol. Netviz shows what the client can observe: `Replicated`, `Predicted`, `Interpolated`, `Controlled`, and `Replicated::from`. Why: preserves headless-server deployment model, keeps scope bounded, and the task's "Lightyear replication and interest-management state" is legibly representable from client-side markers for dev purposes. Explicit limitation recorded under Open Risks.

3. **Disk write-through on native + in-memory on WASM + extended re-patch (Q3 = C).** RON editor serializes the edited `AbilityAsset`/`WorldObjectDef` through `bevy_reflect::serde::ReflectSerializer`, then:
   - Native: writes bytes to the asset's source path; the existing `bevy/file_watcher` triggers the existing `reload_*_defs` systems.
   - WASM: mutates `Assets<…>` directly (no disk access available) and manually sends `AssetEvent::Modified` so reload systems fire.
   Additionally, `reload_ability_defs` and `reload_world_object_defs` are extended: after refreshing the registry, they query all entities that were spawned from the changed def/ability and re-apply components via the existing `ReflectComponent::insert` path. Why: user explicitly asked for both; re-patch removes the despawn/respawn dev friction observed in research.

4. **New `TicketType::Dev` in voxel_map_engine (Q4 = B).** Add a new variant to `TicketType` (`crates/voxel_map_engine/src/ticket.rs:5-12`) with its own level/radius constants, plus a `ChunkTicket::dev(map_entity, radius)` constructor. The chunk debugger spawns a hidden `(ChunkTicket::dev, GlobalTransform)` entity per pinned column and despawns it on "evict". Why: stays within the existing ticket-propagator pipeline (no new code paths in `lifecycle.rs`), separates dev intent from gameplay tickets in logs, and is trivially smaller than adding a bypass API in `api.rs`.

5. **Dual-mode spawn panel (Q5 = C).** Two tabs. **Tab A (Def-driven)**: dropdowns sourced from `WorldObjectDefRegistry` and `AbilityDefs`, spawn via existing `apply_object_components` / `apply_ability_archetype`. **Tab B (Free-form)**: walk `AppTypeRegistry`, filter to registrations exposing `ReflectComponent`, let the user pick N of them, instantiate via `ReflectDefault` (or skip if absent), and spawn through the same `ReflectComponent::insert` loop. Why: def-driven covers the realistic 95% dev case (want a known item/ability); free-form covers ad-hoc component testing without a RON round-trip.

## What We're NOT Doing

- **No server-side `DevPlugin`.** Server remains headless. No egui on the server, no dev-only Lightyear messages, no cross-process RPC.
- **No Lightyear patch.** `ReplicationState.per_sender_state` stays untouched; netviz uses only client-visible markers.
- **No IndexedDB / localStorage** for WASM-side RON persistence. WASM edits are explicitly session-only with an on-panel "unsaved (web)" badge.
- **No replacement of existing hot-reload systems.** We extend `reload_ability_defs` and `reload_world_object_defs` with a re-patch step, not rewrite them. F3 physics-gizmo toggle stays exactly as-is.
- **No new visibility/radius semantics.** Replication remains purely room-based. Chunk-streaming radius logic (`push_chunks_to_clients`, `map.rs:886-964`) is untouched.
- **No vox_model editor.** vox_model has no WASM reload path (`vox_model/loading.rs:170` native-only); adding one is out of scope.
- **No bypass API in `voxel_map_engine/src/api.rs`.** Force-load/evict goes exclusively through the new `TicketType::Dev`.

## Open Risks

- **RON round-trip fidelity.** `bevy_reflect::serde::ReflectSerializer` output may not be byte-identical to hand-written `.ability.ron` / `.object.ron` files (ordering, optional fields). We keep the editor buffer as the edited text, parse on save, and only write when parse succeeds — the on-disk file will be re-formatted after first save. Acceptable for dev use; flag to user before wide adoption.
- **Re-patch semantics.** Re-inserting components overwrites existing ones, but does **not** remove components that were deleted from the def. First-pass: re-insert everything still in the def; surface a warning in the panel if a prior component is no longer present. Full reconciliation (strip+replace archetype) is a follow-up.
- **Free-form spawn invariants.** A user-assembled entity with no `Replicate` or wrong `ReplicationTarget` will not sync over the network. Panel only spawns on the client world (no `Replicate`). Flag clearly in UI: "dev spawn is client-local; use def-driven for replicated entities."
- **`TicketType::Dev` interaction with `LOAD_LEVEL_THRESHOLD`.** New variant must use `level = 0` (Player-equivalent) so `LevelDiff` actually includes its columns; picking a level above the threshold would silently do nothing. Constants must be documented in `ticket.rs`.
- **bevy-inspector-egui 0.36 + bevy_egui 0.39 compile.** Dep tree conflicts with existing workspace deps have not been empirically verified. First implementation phase is a scratch build of just the umbrella `inspector` feature to confirm compilation on both native and `wasm32-unknown-unknown` before touching panel code.
- **Clipboard on WASM.** `manage_clipboard` in `bevy_egui` requires `--cfg=web_sys_unstable_apis` — already set in `crates/web/.cargo/config.toml:22-28`. Enable only if free-form spawn copy-paste turns out to be needed; leave off by default.
