---
date: 2026-03-14T10:06:58-07:00
researcher: Claude
git_commit: 05da6f6f6c0e6d3f8d447e0715acdecd59c44337
branch: master
repository: bevy-lightyear-template
topic: "Refactoring integration tests into a dedicated tests crate using real app initialization"
tags: [research, testing, integration-tests, crossbeam, lightyear, test-harness]
status: complete
last_updated: 2026-03-14
last_updated_by: Claude
---

# Research: Integration Test Crate Refactor

**Date**: 2026-03-14T10:06:58-07:00
**Researcher**: Claude
**Git Commit**: 05da6f6f6c0e6d3f8d447e0715acdecd59c44337
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to refactor the project so that server, client, and web crates export app initialization for both live game and testing, move integration tests into a separate `tests` crate that uses real app initialization (no mocks/minimal apps), support crossbeam transport, and provide test harnesses with convenience methods.

## Summary

The project currently has 21 integration test files across 7 crates and 15 inline `#[cfg(test)]` modules. All three app crates (server, client, web) build and run their Bevy `App` in a single `fn main()` chain with no separation between building and running. The most sophisticated test infrastructure is `CrossbeamTestStepper` in `crates/server/tests/integration.rs`, which already implements crossbeam-based in-process client-server testing with manual time control. Lightyear's own test suite (`lightyear_tests` crate) uses this exact pattern and provides a reference architecture.

## Detailed Findings

### Current App Initialization (No Separation)

All three crates build and run in a single `fn main()` expression ending in `.run()`. No `fn build_app() -> App` exists anywhere.

#### Server (`crates/server/src/main.rs:13-40`)
- `MinimalPlugins` (headless), `StatesPlugin`, `LogPlugin`, `AssetPlugin`, `TransformPlugin`, `ScenePlugin`
- Manual asset type registration (`Mesh`, `StandardMaterial`, `Shader`, `Image`)
- `ServerPlugins { tick_duration }`, `SharedGameplayPlugin`, `ServerNetworkPlugin::default()`, `ServerGameplayPlugin`, `ServerMapPlugin`
- Has both `[lib]` and `[[bin]]` in Cargo.toml -- already exports modules publicly

#### Client (`crates/client/src/main.rs:16-51`)
- Parses client ID from CLI args, builds `ClientNetworkConfig` and `UiClientConfig`
- `DefaultPlugins` (full rendering), `ClientPlugins`, `SharedGameplayPlugin`, `ClientNetworkPlugin`, `ClientGameplayPlugin`, `ClientMapPlugin`, `RenderPlugin`, `UiPlugin`, `PhysicsDebugPlugin`
- `lib.rs` exports `gameplay`, `map`, `network` modules + lightyear type re-exports
- Has `file_watcher` feature enabled by default

#### Web (`crates/web/src/main.rs:14-43`)
- `DefaultPlugins` with custom `WindowPlugin`, WASM panic hook
- `ClientPlugins`, `SharedGameplayPlugin`, `WebClientPlugin::default()`, `ClientGameplayPlugin`, `ClientMapPlugin`, `RenderPlugin`, `UiPlugin`, `PhysicsDebugPlugin`
- `crate-type = ["cdylib", "rlib"]` -- already supports library consumption
- Reuses `ClientGameplayPlugin` and `ClientMapPlugin` from client crate, but uses own `WebClientPlugin` for networking

### Current Test Infrastructure

#### `CrossbeamTestStepper` (`crates/server/tests/integration.rs:28-178`)

The existing in-house harness. Creates paired client+server apps connected via `CrossbeamIo::new_pair()`.

**Setup**:
- Server: `MinimalPlugins` + `ServerPlugins` + `ProtocolPlugin` + `RoomPlugin`. Spawns entity with `Server`, `RawServer`, `DeltaManager`, `CrossbeamIo`.
- Client: `MinimalPlugins` + `ClientPlugins` + `ProtocolPlugin`. Spawns entity with `Client`, `RawClient`, `PingManager`, `ReplicationSender/Receiver`, `PredictionManager`, `CrossbeamIo`, `Linked`.
- Server-side `ClientOf` entity with `LinkOf`, `PingManager`, `ReplicationSender/Receiver`, `Link`, `PeerAddr`, `Linked`, `CrossbeamIo`.
- `TimeUpdateStrategy::ManualInstant` for deterministic time.

**Methods**: `new()`, `init()` (triggers Start/Connect), `tick_step(n)`, `wait_for_connection()`.

**Generic helpers**: `MessageBuffer<M>`, `collect_messages<M>`, `EventBuffer<E>`, `collect_events<E>`.

This harness uses `ProtocolPlugin` only -- NOT the full `ServerGameplayPlugin`/`ClientGameplayPlugin`. Tests that need gameplay systems add them individually.

#### Test File Inventory

| File | App Setup | Focus |
|------|-----------|-------|
| `server/tests/integration.rs` (1507 lines) | `CrossbeamTestStepper` + individual systems | Full client-server: messages, events, map transitions, chunk requests, voxel edits |
| `server/tests/connection_flow.rs` | `MinimalPlugins` + `ServerPlugins` | Server start/connection flow |
| `server/tests/observers.rs` | `MinimalPlugins` + `ServerPlugins` | Observer registration |
| `server/tests/rooms.rs` | `MinimalPlugins` + `RoomPlugin` | Room registry, membership, transfers |
| `server/tests/map_transition.rs` | `MinimalPlugins` + `RoomPlugin` | Room transfer, pending transitions |
| `server/tests/multi_transport.rs` | `MinimalPlugins` + `ServerPlugins` | Transport entity creation |
| `server/tests/plugin.rs` | `MinimalPlugins` + `ServerPlugins` + `ServerNetworkPlugin` | Plugin configuration |
| `server/tests/voxel_persistence.rs` | No App (pure functions) | Chunk save/load |
| `server/tests/world_persistence.rs` | No App (pure functions) | World persistence functions |
| `client/tests/connection.rs` | `MinimalPlugins` + `ClientPlugins` | Client entity creation |
| `client/tests/plugin.rs` | `MinimalPlugins` + `ClientPlugins` + `ClientNetworkPlugin` | Plugin configuration |
| `client/tests/map_transition.rs` | `MinimalPlugins` + `StatesPlugin` + custom | Map transition state machine |
| `web/tests/plugin.rs` | WASM-only: `MinimalPlugins` + `ClientPlugins` + `WebClientPlugin` | Web plugin |
| `web/tests/wasm_integration.rs` | WASM-only: minimal | WASM basics |
| `ui/tests/ui_plugin.rs` | `MinimalPlugins` + `StatesPlugin` + `UiPlugin` | UI states, buttons |
| `ui/tests/map_transition_state.rs` | `MinimalPlugins` + `StatesPlugin` + `UiPlugin` | Transition UI |
| `render/tests/render_plugin.rs` | `DefaultPlugins` (headless) + `RenderPlugin` | Camera setup |
| `protocol/tests/ability_systems.rs` | `MinimalPlugins` + custom ability setup | Ability activation, combos, projectiles |
| `protocol/tests/physics_isolation.rs` | `MinimalPlugins` + `PhysicsPlugins` + `AssetPlugin` | Physics collision isolation |
| `voxel_map_engine/tests/api.rs` | `MinimalPlugins` + `VoxelPlugin` | Voxel API |
| `voxel_map_engine/tests/lifecycle.rs` | `MinimalPlugins` + `VoxelPlugin` | Chunk lifecycle |

#### Shared Test Utilities (`crates/protocol/src/test_utils.rs`)

Behind `test_utils` feature flag:
- `test_protocol_plugin()` -- returns `ProtocolPlugin`
- `assert_message_registered<M>(app)` -- verifies message registration
- `TestTrigger` event type (in `lib.rs:144`, gated by `test_utils` feature)

### Crossbeam Transport Architecture

**Workspace dependency**: `lightyear_crossbeam` at `Cargo.toml:38`, consumed by server crate directly and client crate via `lightyear::crossbeam::CrossbeamIo` re-export.

**How it works**: `CrossbeamIo::new_pair()` creates two components with cross-wired `crossbeam_channel::Sender<Bytes>` / `Receiver<Bytes>`. The `CrossbeamPlugin` drains receiver in `PreUpdate` and sends in `PostUpdate`. Entities are immediately `Linked` -- no handshake.

**Already supported in transport enums**:
- `ServerTransport::Crossbeam { io }` (`crates/server/src/network.rs:25-26`)
- `ClientTransport::Crossbeam(CrossbeamIo)` (`crates/client/src/network.rs:21`)

### Lightyear's Reference Architecture (`lightyear_tests` crate)

Lightyear's own test suite uses the exact same pattern as a workspace-level test crate:

```
lightyear_tests/
├── src/
│   ├── lib.rs           # mod declarations
│   ├── protocol.rs      # test protocol definitions
│   ├── stepper.rs       # ClientServerStepper harness
│   ├── client_server/   # client-server integration tests
│   ├── host_server/
│   ├── multi_server/
│   └── timeline/
└── Cargo.toml
```

**`ClientServerStepper`** is lightyear's equivalent of our `CrossbeamTestStepper`:
- Holds `client_apps: Vec<App>`, `server_app: App`, entity IDs, timing state
- `StepperConfig::single()` / `::with_netcode_clients(n)` for preset configurations
- `frame_step(n)` / `tick_step(n)` advance time and update all apps
- Convenience accessors: `stepper.client(0)`, `stepper.server()`, `stepper.client_of(0)`, `stepper.client_app()`
- Crossbeam connection with `TimeUpdateStrategy::ManualInstant`

Key difference: lightyear's stepper is a library export (`pub struct`), not test-internal code. Tests import it from `crate::stepper`.

### What Each Crate Exports Today

| Crate | Public API | Plugins Available |
|-------|-----------|-------------------|
| `protocol` | Full gameplay types, `ProtocolPlugin`, `SharedGameplayPlugin`, `test_utils` (feature-gated) | `ProtocolPlugin`, `SharedGameplayPlugin`, `AbilityPlugin`, `AppStatePlugin` |
| `server` | `gameplay`, `map`, `network`, `persistence` modules | `ServerNetworkPlugin`, `ServerGameplayPlugin`, `ServerMapPlugin` |
| `client` | `gameplay`, `map`, `network` modules + lightyear re-exports | `ClientNetworkPlugin`, `ClientGameplayPlugin`, `ClientMapPlugin` |
| `web` | `network` module + `WebClientPlugin` + protocol re-export | `WebClientPlugin` |
| `render` | `RenderPlugin` | `RenderPlugin` |
| `ui` | `UiPlugin` | `UiPlugin` |
| `voxel_map_engine` | `VoxelPlugin` + full API | `VoxelPlugin` |

### Dev-Dependencies Across Crates

| Crate | Test Dev-Dependencies |
|-------|----------------------|
| `server` | `tempfile`, `ndshape`, `approx`, `mock_instant`, `test-log`, `protocol[test_utils]`, `client`, `ui` |
| `client` | `approx`, `mock_instant`, `test-log`, `protocol[test_utils]` |
| `ui` | `protocol[test_utils]` |
| `web` | `protocol[test_utils]`, `wasm-bindgen-test`, `console_error_panic_hook` |
| `protocol` | `bevy` (extended), `lightyear_replication`, `serde_json` |
| `voxel_map_engine` | `approx`, `bevy` (default), `tempfile` |

## Code References

- `crates/server/src/main.rs:13-40` -- Server app initialization
- `crates/client/src/main.rs:16-51` -- Client app initialization
- `crates/web/src/main.rs:14-43` -- Web app initialization
- `crates/server/src/network.rs:16-27` -- `ServerTransport` enum (includes `Crossbeam`)
- `crates/client/src/network.rs:15-22` -- `ClientTransport` enum (includes `Crossbeam`)
- `crates/server/tests/integration.rs:28-178` -- `CrossbeamTestStepper` harness
- `crates/server/tests/integration.rs:181-228` -- Generic `MessageBuffer`/`EventBuffer` helpers
- `crates/protocol/src/test_utils.rs:1-21` -- Shared test utilities
- `crates/protocol/src/lib.rs:144` -- `TestTrigger` event (test_utils feature)
- `crates/protocol/src/lib.rs:152-262` -- `ProtocolPlugin::build` (all registrations)
- `crates/protocol/src/lib.rs:280-344` -- `SharedGameplayPlugin` composition
- `crates/server/src/network.rs:88` -- `start_server` function (transport entity spawning)
- `crates/client/src/network.rs:72-82` -- `ClientNetworkPlugin` build

## Architecture Documentation

### Current Testing Tiers

1. **No-App tests**: Pure function/struct tests (persistence, data types, serialization)
2. **MinimalPlugins + domain plugin**: Lightweight headless tests for specific subsystems
3. **MinimalPlugins + lightyear plugins**: Networking tests with `ProtocolPlugin` but not full gameplay
4. **CrossbeamTestStepper**: Full client-server integration with manual time control, but using `ProtocolPlugin` only (not full app plugins)
5. **DefaultPlugins headless**: Only `render_plugin.rs`, with window/render disabled

### Gap: No Full-App Integration Tests

No test currently uses the real `ServerGameplayPlugin`/`ClientGameplayPlugin`/`ServerMapPlugin`/`ClientMapPlugin` together in a client-server configuration. The `CrossbeamTestStepper` uses `ProtocolPlugin` + `RoomPlugin` and adds individual systems as needed. This means end-to-end behavior across the full plugin stack is only verified manually.

### Timing Patterns in Tests

- Most tests: explicit `app.update()` calls
- Physics: 200-iteration loops with 1ms `thread::sleep` per tick
- Voxel lifecycle: `tick_until(condition)` polling with 200-tick max
- Crossbeam networking: `tick_step(n)` with `TimeUpdateStrategy::ManualInstant`
- UDP networking: real-time polling (300 frames, 100us sleep)

## External References

- [lightyear_tests crate](https://github.com/cBournhonesque/lightyear/tree/main/lightyear_tests) -- Reference architecture for workspace-level integration tests
- [lightyear stepper.rs](https://github.com/cBournhonesque/lightyear/blob/main/lightyear_tests/src/stepper.rs) -- `ClientServerStepper` harness
- [lightyear_crossbeam](https://github.com/cBournhonesque/lightyear/tree/main/lightyear_crossbeam) -- Crossbeam transport implementation
- [Bevy how_to_test_systems.rs](https://github.com/bevyengine/bevy/blob/main/tests/how_to_test_systems.rs) -- Official Bevy testing patterns
- [Bevy MinimalPlugins docs](https://docs.rs/bevy/latest/bevy/struct.MinimalPlugins.html)

## Related Research

No prior research documents found on this topic.

## Open Questions

1. **Server headless rendering assets**: Server registers `Mesh`, `StandardMaterial`, `Shader`, `Image` asset types for voxel mesh generation. How should the test app handle these -- include `AssetPlugin` or stub them?
2. **Web crate testability**: Web crate tests are WASM-only (`#[cfg(target_family = "wasm")]`). Should the tests crate include WASM integration tests, or focus on native crossbeam tests that exercise the same code paths?
3. **Render/UI in integration tests**: Client app includes `RenderPlugin` and `UiPlugin` which need window/GPU. Should integration tests skip these (test gameplay only) or use headless rendering?
4. **`DefaultPlugins` vs `MinimalPlugins`**: The real client/web apps use `DefaultPlugins`. Using `MinimalPlugins` in tests means some systems (like asset loading from files) won't run. How close to "real" should the test app be?
5. **Existing `CrossbeamTestStepper`**: Should the new test crate's harness evolve from the existing `CrossbeamTestStepper`, or start fresh following lightyear's `ClientServerStepper` more closely?
