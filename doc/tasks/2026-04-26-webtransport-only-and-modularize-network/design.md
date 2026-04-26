# Design Discussion

## Current State

Three transport-IO surface areas live in user code today:

- **`crates/client/src/network.rs`**: `ClientTransport` enum (`Udp`/`WebTransport`/`Crossbeam`) selected via `match` in `setup_client`; eight-component base bundle + one transport-IO component per variant. Two `cfg(target_family = "wasm")` gates, both on the `Udp` arm (`network.rs:112,122`). Default is `WebTransport` using `CERTIFICATE_DIGEST = include_str!("../../../certificates/digest.txt")` (`network.rs:11,24-30`).
- **`crates/server/src/network.rs`**: `ServerTransport` enum (`Udp`/`WebTransport`/`WebSocket`/`Crossbeam`); `start_server` iterates `Vec<ServerTransport>` and spawns one entity per variant (`network.rs:17-28`, `98-208`). WebTransport identity loads via `IoTaskPool::scope` + `async_compat::Compat` (`network.rs:80-91`). `ClientOf → ReplicationSender` wired via `register_required_components_with` (`network.rs:70-72`).
- **`crates/web/src/network.rs`**: `WebClientPlugin` is a thin config-builder; loads digest under `cfg(target_family = "wasm")`, hardcodes `client_addr = 0.0.0.0:5001` and `server_addr = 127.0.0.1:5001`, delegates to `ClientNetworkPlugin` (`network.rs:6,20-38`).

**Cross-crate references to dropped types: none in production `.rs` files.** Outside the three `network.rs` files, no `.rs` file references `ClientTransport`, `ServerTransport`, `UdpIo`, `ServerUdpIo`, `WebSocketServerIo`. `CrossbeamIo` and `lightyear_crossbeam` are referenced only in `crates/server/tests/integration.rs` (lines 45, 345, 469, 1142).

**Lightyear feature flags currently enabled** (per-crate, root `Cargo.toml:46-51` + per-crate `Cargo.toml`):
- `client`: `udp`, `crossbeam`, `webtransport` + others
- `server`: `udp`, `webtransport`, `websocket`, `crossbeam` + others
- `web`: `webtransport`, `websocket` + others
- Workspace umbrella: `leafwing`, `raw_connection`
- `lightyear_crossbeam` workspace dep used directly by `crates/server` (production) and `crates/server/tests/integration.rs`.

**Test inventory** (`crates/server/tests/integration.rs`): 13+ integration tests covering connection, replication, map switching, voxel chunk push/ack, homebase spawn. All use `lightyear_crossbeam::CrossbeamIo::new_pair()` via a custom `CrossbeamTestStepper` (`integration.rs:32-182`) that bypasses `ServerNetworkPlugin`/`ClientNetworkPlugin` entirely. One test (`test_client_server_udp_connection`, line 238) uses real UDP via the production plugin path.

**Lightyear's own decomposition** (`git/lightyear/`): `lightyear_link`, `lightyear_connection`, `lightyear_transport`, `lightyear_netcode`, `lightyear_webtransport` are standalone crates. `lightyear_client`/`lightyear_server` are **not** crates — they are `ClientPlugins`/`ServerPlugins` plugin groups inside the umbrella `lightyear` crate (`lightyear/src/{client,server}.rs`).

## Desired End State

A WebTransport-only, single-transport networking surface, with connection/handshake plumbing extracted from the game crates into three new dedicated crates.

**Three new workspace crates:**

1. **`client_lightyear`** — generic native+WASM WebTransport client setup. Owns `ClientNetworkConfig`, `ClientNetworkPlugin`, `setup_client`, connect/disconnect observers. Internal modules organize by concern (`webtransport`, `netcode`, `connection`).
2. **`server_lightyear`** — WebTransport server setup. Owns `ServerNetworkConfig`, `ServerNetworkPlugin`, `setup_server`, async cert loader, `ClientOf → ReplicationSender` registration.
3. **`client_web_lightyear`** — WASM-specific preset. Owns `WebClientPlugin` (renamed if helpful), does the `include_str!` for the digest under `cfg(target_family = "wasm")`, applies WASM-specific address defaults, delegates to `client_lightyear::ClientNetworkPlugin`.

**Game crates after extraction:**
- `crates/client` — `network.rs` removed; depends on `client_lightyear`. Native binaries supply their own `certificate_digest` via config.
- `crates/server` — `network.rs` removed; depends on `server_lightyear`.
- `crates/web` — `network.rs` removed; depends on `client_web_lightyear` (which transitively pulls `client_lightyear`).

**Removed code:**
- `ClientTransport`, `ServerTransport` enums.
- All Udp/WebSocket/Crossbeam IO insert paths.
- Lightyear features: `udp`, `crossbeam` (from `client`); `udp`, `websocket`, `crossbeam` (from `server`); `websocket` (from `web`); `raw_connection` (from workspace umbrella).
- Workspace dep: `lightyear_crossbeam`.
- All crossbeam-based tests in `crates/server/tests/integration.rs` and the `CrossbeamTestStepper` harness. The single UDP test goes too (UDP is dropped). The integration file is reduced to whatever, if anything, can be expressed without `lightyear_crossbeam` or `udp`.

**Verification:**
- `cargo check-all` passes with no transport symbols outside the three new crates.
- `cargo test-all` passes (after the integration test rewrite/removal).
- `cargo client` + `cargo server` connect over WebTransport and gameplay works end-to-end (manual; prompt user to verify).
- `cargo web` builds and connects from a browser.
- Workspace dependency graph remains acyclic; `server` still has no production dep on `client`.

## Patterns to Follow

**Follow:**
- Lightyear example composition in `git/lightyear/examples/common/src/{client,server}.rs:51-97 / 96-163` — direct insert of `WebTransportClientIo { certificate_digest }` and `WebTransportServerIo { certificate }` next to the standard `Link`/`Client`/`Server`/`Netcode*` bundle.
- `IoTaskPool::scope(|s| s.spawn(Compat::new(async { Identity::load_pemfiles(...).await })))` for server cert loading (`crates/server/src/network.rs:80-91`). Bevy required-component registration for `ClientOf → ReplicationSender` (`crates/server/src/network.rs:70-72`).
- "Thin configurator" plugin shape: 1 resource + 1 startup system + a small number of observers. The umbrella `ClientPlugins`/`ServerPlugins` plugin groups are added by **binaries**, not by these wrapper plugins (matches `crates/web/src/main.rs:26-28`).
- Side-oriented external naming, concern-oriented internal modules: `client_lightyear::{webtransport, netcode, connection}` mirrors lightyear's per-concern split without paying the multi-crate boilerplate cost.
- Shared netcode constants `PROTOCOL_ID` / `PRIVATE_KEY` stay in `crates/protocol/src/lib.rs:55-56` (they're already the single source of truth for both sides).
- Per-binary `include_str!("../../../certificates/digest.txt")` for the digest, supplied through `ClientNetworkConfig.certificate_digest`.

**Do not follow:**
- The `Vec<ServerTransport>` + per-variant `match` shape (`crates/server/src/network.rs:98-208`) — collapses to a flat `ServerNetworkConfig { bind_addr, port, ... }` once only one variant remains.
- Duplicated `include_str!` for the digest in both `crates/client/src/network.rs:11` and `crates/web/src/network.rs:21`. Each binary's `main` (or `client_web_lightyear` for WASM) does the `include_str!` exactly once and feeds it via config.
- The `client_addr` port mismatch between `crates/client/src/network.rs:48` (port `0`) and `crates/web/src/network.rs:28` (port `5001`). Pick the right value per side intentionally; don't carry the inconsistency forward without justification.
- `CrossbeamTestStepper` (`integration.rs:32-182`) — being removed wholesale per Q1=C.

## Design Decisions

1. **Crate granularity**: three side-oriented crates (`client_lightyear`, `server_lightyear`, `client_web_lightyear`), internal modules by concern. *Why*: matches the task's literal naming, current scope has no cross-side types worth a shared crate, per-concern external splitting would be premature given the small size of each concern.

2. **Server config shape**: flat `ServerNetworkConfig { bind_addr: IpAddr, port: u16, protocol_id, private_key, cert_pem_path, key_pem_path, ... }`. Single WT server per process. *Why*: only one variant remains; the enum and `Vec` wrapping become dead weight. Trivial to re-add if multi-port becomes a real need.

3. **Certificate digest sourcing**: `client_lightyear` exposes `certificate_digest: String` as a required `ClientNetworkConfig` field; no const. Each consuming binary owns its own `include_str!` (or empty string for native, matching lightyear example convention). *Why*: keeps `client_lightyear` free of binary-specific compile-time inputs and makes test/dev override trivial.

4. **WASM concerns isolation**: a third crate `client_web_lightyear` owns the WASM-specific config preset (digest `include_str!` under WASM cfg, hardcoded WASM addresses). `client_web_lightyear` depends on `client_lightyear`; `crates/web` depends on `client_web_lightyear`. *Why*: keeps `client_lightyear` target-agnostic; collects all WASM-only behaviour in one place; mirrors the existing `web` → `client` wrapper layering.

5. **Test strategy**: delete crossbeam-based tests outright; remove the `CrossbeamTestStepper` harness and the `lightyear_crossbeam` workspace dep. *Why*: per Q1=C. Ramifications flagged in **Open Risks**. The single UDP test goes with UDP.

6. **Lightyear feature flags**: trim per-crate features to exactly what each new crate needs. `client_lightyear`: `client`, `netcode`, `webtransport`, `leafwing`, `prediction`, `replication`, `interpolation`. `server_lightyear`: `server`, `netcode`, `webtransport`, `leafwing`, `replication`. Workspace umbrella drops `raw_connection`. *Why*: smallest surface area; `raw_connection` was only consumed by the deleted `RawClient`/`RawServer` test markers.

7. **Replication wiring**: keep `register_required_components_with::<ClientOf, ReplicationSender>` in `server_lightyear`. *Why*: existing pattern, no reason to change.

## What We're NOT Doing

- Not changing the netcode auth scheme (`Authentication::Manual` stays).
- Not changing the certificate generation pipeline (`certificates/generate.sh`, `Makefile.toml` `[tasks.generate-certs]`, `[tasks.ensure-certs]`).
- Not relocating `PROTOCOL_ID` / `PRIVATE_KEY` from `crates/protocol`.
- Not migrating `crates/server/tests/integration.rs` to a new transport — it's deleted (per Q1=C). No replacement integration test suite is built in this task.
- Not splitting per-concern crates (`network_netcode`, `network_link`, etc.) — single crate per side, modules within.
- Not changing `crates/protocol`'s `test_utils` feature or its consumers.
- Not changing `crates/render`'s `frame_interpolation` lightyear feature or `crates/ui`'s `client`+`netcode` features (they reference lightyear types, not transport setup).
- Not introducing env-var or config-file driven address selection.
- Not changing `client_addr` defaults beyond fixing the existing inconsistency where it falls naturally out of the split.

## Open Risks

- **Substantial test coverage loss.** Deleting `crates/server/tests/integration.rs` removes coverage for: connection establishment, reconnection, client→server and server→client messages, event triggers, map switch transition, duplicate map switch idempotency, homebase config matching, voxel edit ack, server-pushed chunk replication, out-of-range chunk unload (`integration.rs:238, 342, 415, 450, 597, 642, 711, 779, 937, 1051, 1138, 1382, 1518, 1639`). Some of this functionality has no other validation path. Consider scheduling a follow-up to rebuild a minimal integration suite over a different harness.
- **`UdpIo` vs `ServerUdpIo` typo flagged in research.** Moot — UDP is being removed. No action.
- **Self-signed WebSocket identity removal.** `crates/server/src/network.rs:160-170`'s synchronous `Identity::self_signed(...)` path goes away with WebSocket. No fallback if a deployment environment can't terminate WT (e.g. some corporate proxies block QUIC/UDP). Acceptable per task framing ("WebTransport-only") but worth noting.
- **`lightyear_replication` workspace dep.** Currently a dev-dep of `crates/protocol` only. Confirm during implementation whether it remains required after the test deletions.
- **`web` crate's `default-features = false` on `client`.** `client`'s `default = ["file_watcher"]` is disabled by `web`. After the split, ensure the `file_watcher` feature lives in whichever crate still controls it (likely `crates/client` proper, not `client_lightyear`).
- **`raw_connection` umbrella feature removal.** Confirm at implementation time that nothing in the production path observes `RawClient`/`RawServer` markers (research said no, but worth a final sweep).
- **Crate naming convention divergence from lightyear.** Lightyear uses `lightyear_<concern>`; we're using `<role>_lightyear` (per task wording). Cosmetic; documented here so future readers know it was an intentional choice.
