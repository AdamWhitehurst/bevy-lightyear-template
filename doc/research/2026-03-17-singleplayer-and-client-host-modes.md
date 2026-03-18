---
date: 2026-03-17T15:39:15-07:00
researcher: Claude Sonnet 4.6
git_commit: be2ce4fc97f404915d93b5350df28e819eca6eb2
branch: master
repository: bevy-lightyear-template
topic: "Singleplayer and Client-Host Networking Modes"
tags: [research, codebase, networking, lightyear, singleplayer, client-host, host-server, LinkOf, crossbeam]
status: complete
last_updated: 2026-03-17
last_updated_by: Claude Sonnet 4.6
---

# Research: Singleplayer and Client-Host Networking Modes

**Date**: 2026-03-17T15:39:15-07:00
**Researcher**: Claude Sonnet 4.6
**Git Commit**: be2ce4fc97f404915d93b5350df28e819eca6eb2
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to support two new modes:
- **Singleplayer** — both web and native clients run their own server but only for themselves
- **Client-host** — the native client can host its own public server to which other clients can connect

Can we run the server as a separate thread and expose Crossbeam channels for singleplayer and/or UDP ports for client-host? Are there other approaches?

## Summary

Lightyear's current API provides a purpose-built pattern for both use cases called **HostClient** (`LinkOf`). It requires no Crossbeam channels and no separate threads — instead both `ClientPlugins` and `ServerPlugins` are added to a single `App`, and the local client entity is linked directly to the server entity via the `LinkOf` relationship component, bypassing all transport I/O. For client-host, the server additionally spawns with a real transport (UDP or WebTransport) so remote clients can connect normally.

`CrossbeamIo` is a distinct, lower-level mechanism designed for two _separate_ Bevy `App` instances running in separate threads. It is already used in this project for integration testing (`CrossbeamTestStepper`), but is not the right tool for gameplay singleplayer or client-host — it adds thread management complexity for no benefit over the single-App `LinkOf` approach.

## Detailed Findings

### Approach 1: `LinkOf` (HostClient — Single App)

This is the canonical Lightyear approach for both singleplayer and client-host.

**How it works:**

Both `ClientPlugins` and `ServerPlugins` are added to a single Bevy `App`. A server entity is spawned as normal. Instead of spawning a client entity with `NetcodeClient` + a transport component, the local client entity is spawned with:

```rust
commands.spawn((
    Client::default(),
    Name::new("HostClient"),
    LinkOf { server },
))
```

`LinkOf { server }` is a Lightyear relationship component (`lightyear::prelude`) that marks the client as a direct in-process child of the server entity. No socket, no crossbeam channel, no netcode handshake — the entities are connected immediately at zero cost.

**Reference in local fork:**
[`git/lightyear/examples/common/src/cli.rs:165-200`](../../git/lightyear/examples/common/src/cli.rs) — the `Mode::HostClient` branch of `spawn_connections()`:

```rust
// Spawn server entity with transport for remote peers (omit for pure singleplayer)
let server = app.world_mut().spawn(ExampleServer { transport: ServerTransports::WebTransport { .. }, .. }).id();

// Spawn local client using LinkOf instead of NetcodeClient
let client = app.world_mut().spawn((
    Client::default(),
    Name::new("HostClient"),
    LinkOf { server },
)).id();

// Server must be started before client connects
app.add_systems(Startup, (start, connect).chain());
```

**Singleplayer variant**: Spawn the server entity with _no transport at all_ (no UDP, no WebTransport). Only the `LinkOf` client connects. No ports are opened.

**Client-host variant**: Spawn the server entity with UDP or WebTransport so remote clients can connect. The local client still uses `LinkOf`.

### Approach 2: `CrossbeamIo` (Separate Apps, Two Threads)

`CrossbeamIo` bridges two separate Bevy `App` instances — one for server, one for client — running in separate threads via in-memory `crossbeam_channel` pairs.

**How it works:**

```rust
let (client_io, server_io) = CrossbeamIo::new_pair();
// client_io goes into the client App's entity
// server_io goes into the server App's client-of entity
```

`CrossbeamPlugin` (from `lightyear_crossbeam`) drains `Link.send` into the crossbeam sender in `PostUpdate`, and pulls from the crossbeam receiver into `Link.recv` in `PreUpdate`. Both entities need an explicit `Linked` component immediately (no handshake required).

**Current usage in this project**: Already used in `CrossbeamTestStepper` (`crates/server/tests/integration.rs:28-178`) for integration tests. Both `ServerTransport::Crossbeam { io }` ([`crates/server/src/network.rs`](../../crates/server/src/network.rs)) and `ClientTransport::Crossbeam(CrossbeamIo)` ([`crates/client/src/network.rs`](../../crates/client/src/network.rs)) variants exist.

**Why this is not the right tool for gameplay modes**: Running two `App`s in two threads requires `App: Send` (unsafe impl exists in lightyear examples as `SendApp`), manual thread management, and two separate ECS worlds. For singleplayer, this adds complexity with no advantage. `CrossbeamIo` is explicitly documented as "primarily intended for local testing or scenarios where in-process message passing is desired."

### Approach 3: Separate Server Process (Current Default)

The existing architecture: `cargo server` runs a dedicated `server` binary, and `cargo client` / web connects to it over the network. This is the only currently supported mode and is not suitable for singleplayer.

## Architecture Constraints Per Platform

| Constraint | Native Client | Web (WASM) |
|---|---|---|
| UDP server | ✓ | ✗ (no socket binding) |
| WebTransport server | ✓ | ✗ |
| Crossbeam (two Apps, two threads) | ✓ | ✗ (single-threaded WASM) |
| `LinkOf` (single App, zero transport) | ✓ | ✓ (if `server` feature compiles for WASM) |
| `server` lightyear feature on WASM | Unknown — needs validation | Unknown — needs validation |

**Key open question**: Whether Lightyear's `server` feature compiles and functions correctly for WASM. The `ServerPlugins` group is purely ECS (no I/O required if no transport is configured), but whether it has any `#[cfg(not(target_family = "wasm"))]` guards needs to be verified by attempting a WASM build.

## Current Crate Feature Gaps

For the `HostClient` pattern, both `ClientPlugins` and `ServerPlugins` must be available in the same crate.

**Native client (`crates/client`)**: Currently has lightyear `client` feature but NOT `server`. Would need `server` added to its lightyear dependency features to use `ServerPlugins`.

**Web client (`crates/web`)**: Currently has lightyear `client` features but NOT `server`. Same gap. Additionally does not depend on `lightyear_crossbeam` (correct — WASM can't use it).

Current feature sets from `Cargo.toml`:

| Crate | Has `client` | Has `server` | Has `crossbeam` |
|---|---|---|---|
| `server` | ✗ | ✓ | ✓ |
| `client` | ✓ | ✗ | ✓ |
| `web` | ✓ | ✗ | ✗ |

## What Changes Are Required

### Native Singleplayer (HostClient, no remote transport)

1. Add `"server"` to the `client` crate's lightyear features in `crates/client/Cargo.toml`
2. Add `ServerPlugins` to the client `App` when in singleplayer mode
3. Spawn a server entity with no transport components (no UDP, no WebTransport)
4. Spawn local client entity with `LinkOf { server }` instead of `NetcodeClient` + transport
5. Trigger `Start` on server, then `Connect` on client (or the server-start equivalent in this project)

### Native Client-Host (HostClient, with remote transport)

Same as singleplayer, but step 3 additionally spawns the server with `ServerTransport::Udp` or `ServerTransport::WebTransport` so remote clients can connect.

### Web Singleplayer (HostClient)

1. Add `"server"` to the `web` crate's lightyear features (WASM viability must be verified first)
2. Spawn a server entity with no transport
3. Spawn local client entity with `LinkOf { server }`

Web client-host is not possible (no ability to bind ports for incoming connections).

## Existing Infrastructure Relevant to This Feature

- [`crates/server/src/network.rs`](../../crates/server/src/network.rs) — `ServerNetworkPlugin`, `ServerTransport` enum, `start_server` system
- [`crates/client/src/network.rs`](../../crates/client/src/network.rs) — `ClientNetworkPlugin`, `ClientTransport` enum with `Crossbeam` variant already present
- [`crates/server/tests/integration.rs:28-178`](../../crates/server/tests/integration.rs) — `CrossbeamTestStepper` with full client+server in one process (test context, but proves the pattern works)
- [`git/lightyear/examples/common/src/cli.rs:92-204`](../../git/lightyear/examples/common/src/cli.rs) — Reference `HostClient` and `Separate` mode implementations
- [`git/lightyear/examples/lobby/src/client.rs:60-97`](../../git/lightyear/examples/lobby/src/client.rs) — Runtime transition from `NetcodeClient` to `LinkOf` when a connected client becomes host

## Code References

- `crates/client/src/network.rs:15-22` — `ClientTransport` enum (Crossbeam variant exists)
- `crates/server/src/network.rs:17-28` — `ServerTransport` enum (Crossbeam, UDP, WebTransport, WebSocket)
- `crates/server/src/network.rs:65-78` — `ServerNetworkPlugin::build`
- `crates/client/src/network.rs:72-82` — `ClientNetworkPlugin::build`
- `crates/server/tests/integration.rs:28-178` — `CrossbeamTestStepper` (in-process server+client, test-only)
- `git/lightyear/examples/common/src/cli.rs:92-104` — `Mode::HostClient` plugin setup
- `git/lightyear/examples/common/src/cli.rs:165-200` — `Mode::HostClient` entity spawning with `LinkOf`
- `git/lightyear/lightyear_crossbeam/src/lib.rs:38-54` — `CrossbeamIo::new_pair()`

## Architecture Documentation

### `LinkOf` vs `CrossbeamIo` Decision Matrix

| | `LinkOf` (HostClient) | `CrossbeamIo` (Separate) |
|---|---|---|
| App structure | Single `App` | Two `App`s, two threads |
| Transport overhead | Zero (direct ECS relationship) | In-memory channel copy |
| Thread requirements | None | Requires `Send` + thread spawn |
| WASM compatible | Likely yes (no I/O) | No (single-threaded) |
| Remote clients can join | Yes (server can also have real transport) | Yes (server can also have real transport) |
| Current project use | Not yet used in gameplay | Integration tests only |
| Lightyear's recommended use | Singleplayer, listen server, client-host | Local testing / dev tools |

### Lobby Example's Runtime Host Transition Pattern

The lobby example shows a client that _starts_ as a connected multiplayer client and then _transitions_ to host at runtime:

```rust
// Disconnect from lobby server
commands.trigger(Unlink { entity: local_client, reason: "Client becoming Host".to_string() });
commands.entity(local_client).remove::<NetcodeClient>();
// Re-link as host
commands.entity(local_client).insert(LinkOf { server });
```

This runtime transition pattern could support a "become host" flow, not just startup-time modes.

## Historical Context

- `doc/research/2025-12-31-crossbeam-clientserverstepper-integration.md` — Deep dive on `CrossbeamIo` and `ClientServerStepper` for integration testing; confirms `CrossbeamIo` is test/dev-tool oriented
- `doc/research/2026-03-14-integration-test-crate-refactor.md` — Confirms `CrossbeamTestStepper` already works in this project; documents that no singleplayer or client-host mode currently exists
- `doc/research/2025-11-16-lightyear-multi-crate-setup.md` — Established original multi-crate transport architecture

## Open Questions

1. **WASM `server` feature**: Does adding the `server` lightyear feature to `web`'s Cargo.toml compile for WASM? `ServerPlugins` may contain `#[cfg(not(target_family = "wasm"))]` guards or platform-specific dependencies. Verify with `cargo build -p web --target wasm32-unknown-unknown` after adding the feature.

2. **App state / plugin ordering**: The client app's `AppState` machine gates gameplay behind `AppState::Ready`. A singleplayer mode needs both the server and client to reach ready state from a single startup path. How does `SharedGameplayPlugin` (added to both client and server today) behave when added twice to the same `App`?

3. **Asset loading collision**: Server and client both add `AssetPlugin` with different `file_path` values today. A single HostClient app needs one unified `AssetPlugin` configuration.

4. **Physics**: Both server and client run Avian physics. In HostClient mode both are in one `App`. Does adding Avian physics twice cause conflicts, or does lightyear/Avian handle this?

5. **`ReplicationSender` on `ClientOf`**: In the current `ServerNetworkPlugin`, `ReplicationSender` is registered as a required component on `ClientOf` entities. In HostClient mode, the local `ClientOf` entity is created via `LinkOf` without going through `ServerNetworkPlugin`. Does this required component still get inserted automatically?
