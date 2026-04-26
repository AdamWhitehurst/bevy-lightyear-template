# Task

Replace the current multi-transport networking (UDP, WebSocket, Crossbeam, WebTransport) with **WebTransport-only** on both client and server, and remove all dead code that falls out from dropping the other transports.

Then extract the connection/handshake plumbing currently inlined in `crates/client/src/network.rs` and `crates/server/src/network.rs` into two new workspace crates — `client_lightyear` and `server_lightyear` — modelled after the per-concern crate split used by `git/lightyear/` (e.g. `lightyear_client`, `lightyear_server`, `lightyear_netcode`, `lightyear_transport`, `lightyear_webtransport`).

Goal: a smaller, single-transport surface area with cleaner separation between game logic and lightyear integration.
