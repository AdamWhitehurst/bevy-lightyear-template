# Research Questions

## Context

Focus on the project's networking layer: `crates/client/src/network.rs`, `crates/server/src/network.rs`, `crates/web/src/network.rs`, the workspace `Cargo.toml`, the per-crate `Cargo.toml` feature flags for `lightyear`, and the `certificates/` directory. Also focus on the `git/lightyear/` submodule layout — its sub-crates (`lightyear_link`, `lightyear_connection`, `lightyear_netcode`, `lightyear_transport`, `lightyear_webtransport`, `lightyear_client`, `lightyear_server`, `lightyear_crossbeam`, etc.) and how they compose at the use site. Touch test infrastructure (`crates/protocol/src/test_utils.rs`, any integration tests in `crates/server/` or `crates/client/`) only insofar as it touches the connection abstractions.

## Questions

1. In `crates/client/src/network.rs`, how is the lightyear client entity assembled — what components are spawned (`Client`, `Link`, `NetcodeClient`, `LocalAddr`, `PeerAddr`, `ReplicationReceiver`, `PredictionManager`, transport IO), how does the `ClientTransport` enum select between `Udp` / `WebTransport` / `Crossbeam`, and which `cfg(target_family = "wasm")` gates apply?

2. In `crates/server/src/network.rs`, how does `start_server` spawn one server entity per `ServerTransport` variant — what is needed for each (UDP socket, WebTransport identity loaded asynchronously via `IoTaskPool`/`async-compat`, WebSocket self-signed identity, Crossbeam `io`), and how is `ClientOf` → `ReplicationSender` registered?

3. How does `crates/web/src/network.rs` and its `WebClientPlugin` relate to `crates/client/src/network.rs::ClientNetworkPlugin` — what does the web layer add or override (cert digest loading, server addr, `target_family = "wasm"` gates), and what would change if the underlying client plugin only supported WebTransport?

4. Outside the three `network.rs` files, where are `ClientTransport`, `ServerTransport`, `UdpIo`, `ServerUdpIo`, `WebSocketServerIo`, `CrossbeamIo`, `lightyear_crossbeam`, or the `udp` / `websocket` / `crossbeam` lightyear features referenced — including each crate's `Cargo.toml`, dev/diagnostics code, examples, scripts, `Makefile.toml`, and CI config?

5. How is the `git/lightyear/` workspace decomposed — for each of `lightyear_link`, `lightyear_connection`, `lightyear_transport`, `lightyear_netcode`, `lightyear_webtransport`, `lightyear_client`, `lightyear_server`, what types/components/plugins does it export, and how do the lightyear examples (under `git/lightyear/examples/`) compose these crates to set up a WebTransport client and server?

6. How does the WebTransport handshake work end-to-end in this project — how are `certificates/cert.pem`, `certificates/key.pem`, and `certificates/digest.txt` produced (which script/build step), how is the digest baked into the client at compile time via `include_str!`, how is the server `Identity` loaded asynchronously, and how does the netcode token / `Authentication::Manual` flow layer on top?

7. Which workspace tests currently depend on the Crossbeam in-memory transport or otherwise touch client/server connection setup, what fixtures/helpers does `protocol::test_utils` expose, and which crates enable `protocol`'s `test_utils` feature?

8. What is the current inter-crate dependency graph among `client`, `server`, `web`, `protocol`, `dev`, `render`, `ui`, and `persistence` (especially: which crates depend on `lightyear`, `lightyear_crossbeam`, `lightyear_replication`, with which feature sets), and what cycles or constraints would have to be respected when carving out new `client_lightyear` and `server_lightyear` crates?
