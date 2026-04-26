## Approach

Drop non-WebTransport transports and their tests first (two phases, smallest blast radius), then carve the now-trimmed `network.rs` files into three side-oriented crates (`client_lightyear`, `server_lightyear`, `client_web_lightyear`). Each phase ends in a known-good state that the next builds on; failure of a later phase still leaves earlier ones independently valuable.

---

## Phase 1: Drop Crossbeam transport + harness

Removes the `Crossbeam` variant from both transport enums, the `CrossbeamTestStepper`, and all crossbeam-based integration tests. After this phase only WebTransport, UDP, and WebSocket variants remain; `cargo test-all` runs the single UDP test plus persistence tests.

**Files**: `crates/client/src/network.rs`, `crates/server/src/network.rs`, `crates/server/tests/integration.rs`, root `Cargo.toml`, `crates/client/Cargo.toml`, `crates/server/Cargo.toml`.

**Key changes**:
- `ClientTransport::Crossbeam(CrossbeamIo)` — removed
- `ServerTransport::Crossbeam { io }` — removed
- `use lightyear::crossbeam::CrossbeamIo;` — removed
- `lightyear_crossbeam` workspace dep — removed
- `crossbeam` lightyear feature — removed from `client` and `server`
- `CrossbeamTestStepper` and all `test_crossbeam_*` / map-switch / homebase / voxel tests — deleted

**Verify**: `cargo check-all` passes; `cargo test-all` passes (only `test_client_server_udp_connection` + persistence tests remain); `cargo client` + `cargo server` still connect via WebTransport (manual).

---

## Phase 2: Drop UDP + WebSocket; collapse to single-transport configs

Removes the `Udp` and `WebSocket` variants and collapses `ClientTransport`/`ServerTransport` enums into flat single-transport config structs. Deletes the now-orphaned UDP test. After this phase the codebase is WebTransport-only, but the wiring still lives in the game crates.

**Files**: `crates/client/src/network.rs`, `crates/server/src/network.rs`, `crates/server/tests/integration.rs`, root `Cargo.toml`, `crates/client/Cargo.toml`, `crates/server/Cargo.toml`, `crates/web/Cargo.toml`.

**Key changes**:
- `ClientTransport` enum — removed; `ClientNetworkConfig.transport` field — removed; `ClientNetworkConfig.certificate_digest: String` — added
- `ServerTransport` enum — removed; `ServerNetworkConfig.transports: Vec<_>` — removed
- `ServerNetworkConfig { bind_addr: IpAddr, port: u16, protocol_id, private_key, cert_pem_path, key_pem_path, ... }` — flat shape
- `start_server` — collapsed to single-entity spawn (no `match`, no `Vec` iteration)
- `setup_client` — single insert of `WebTransportClientIo { certificate_digest }`, no `match`
- WASM `cfg` gate on `Udp` arm — gone with the arm
- `load_webtransport_identity` — unchanged
- `register_required_components_with::<ClientOf, ReplicationSender>` — unchanged
- `udp`, `websocket` lightyear features — removed (`udp` from client/server, `websocket` from server/web)
- `raw_connection` lightyear umbrella feature — removed from root `Cargo.toml`
- `test_client_server_udp_connection` — deleted; `crates/server/tests/integration.rs` reduced to whatever remains compiling (likely empty file or removed)

**Verify**: `cargo check-all` passes; `cargo test-all` passes; `cargo client` + `cargo server` connect over WebTransport (manual); `cargo web` still builds (still routed through old `crates/web/src/network.rs`).

---

## Phase 3: Extract `client_lightyear` crate

Creates the new `crates/client_lightyear` workspace crate, moves the trimmed client networking code into it, and rewires `crates/client` to depend on it. The `certificate_digest` becomes a required config field with no compile-time const inside the new crate; the binary's `main` (or `crates/client`) supplies it via `include_str!`.

**Files**: new `crates/client_lightyear/{Cargo.toml,src/lib.rs,src/webtransport.rs,src/netcode.rs,src/connection.rs}`; `crates/client/src/network.rs` (deleted); `crates/client/src/lib.rs`, `crates/client/src/main.rs` or wherever the plugin is added; `crates/client/Cargo.toml`; root `Cargo.toml` workspace members.

**Key changes**:
- `pub struct ClientNetworkConfig { client_addr: SocketAddr, server_addr: SocketAddr, client_id: u64, protocol_id: u64, private_key: [u8; 32], certificate_digest: String, token_expire_secs: u32 }` — moved, `certificate_digest` becomes required (no const default)
- `pub struct ClientNetworkPlugin { pub config: ClientNetworkConfig }` — moved
- `pub fn setup_client(commands: Commands, config: ClientNetworkConfig)` — moved; inserts the eight-component bundle + `WebTransportClientIo { certificate_digest }`
- `on_connected` / `on_disconnected` observers — moved
- Internal module split: `webtransport` (IO insert), `netcode` (auth + `NetcodeClient` construction), `connection` (entity bundle + observers)
- `crates/client/Cargo.toml` — drops `lightyear` features that move into `client_lightyear` (keeps only what game-side code uses); adds `client_lightyear = { workspace = true }`; retains `file_watcher` default feature
- `crates/client` `main`/lib — supplies `certificate_digest` via its own `include_str!("../../certificates/digest.txt")` (or empty for native dev)

**Verify**: `cargo check-all` passes; `cargo test-all` passes; `cargo client` connects to `cargo server` over WebTransport and gameplay works (manual); no `ClientTransport`, `UdpIo`, `CrossbeamIo` symbols anywhere.

---

## Phase 4: Extract `server_lightyear` crate

Creates `crates/server_lightyear`, moves the trimmed server networking code into it, and rewires `crates/server` to depend on it.

**Files**: new `crates/server_lightyear/{Cargo.toml,src/lib.rs,src/webtransport.rs,src/netcode.rs,src/connection.rs}`; `crates/server/src/network.rs` (deleted); `crates/server/src/lib.rs` and `main.rs`; `crates/server/Cargo.toml`; root `Cargo.toml` workspace members.

**Key changes**:
- `pub struct ServerNetworkConfig { bind_addr: IpAddr, port: u16, protocol_id: u64, private_key: [u8; 32], cert_pem_path: PathBuf, key_pem_path: PathBuf }` — moved; cert paths become config (currently hardcoded via `concat!(env!("CARGO_MANIFEST_DIR"), ...)` — binary supplies)
- `pub struct ServerNetworkPlugin { pub config: ServerNetworkConfig }` — moved
- `pub fn setup_server(commands: Commands, config: ServerNetworkConfig)` — moved; single-entity spawn with `WebTransportServerIo`
- `fn load_webtransport_identity(cert_pem: &Path, key_pem: &Path) -> Identity` — moved; takes paths instead of consts
- `register_required_components_with::<ClientOf, ReplicationSender>` — moved into `ServerNetworkPlugin::build`
- Internal module split: `webtransport` (IO + identity loader), `netcode` (`NetcodeServer` construction), `connection` (replication wiring + entity bundle)
- `crates/server/Cargo.toml` — drops moved `lightyear` features; adds `server_lightyear = { workspace = true }`
- `crates/server` `main` — passes the cert paths via config

**Verify**: `cargo check-all` passes; `cargo test-all` passes; `cargo client` + `cargo server` connect, replication works (manual); no `ServerTransport`, `UdpIo`/`ServerUdpIo`, `WebSocketServerIo`, `CrossbeamIo` symbols anywhere.

---

## Phase 5: Extract `client_web_lightyear` crate

Creates `crates/client_web_lightyear` for the WASM-specific preset: it owns the `include_str!` for the digest under `cfg(target_family = "wasm")` and the WASM-side address defaults, then delegates to `client_lightyear::ClientNetworkPlugin`.

**Files**: new `crates/client_web_lightyear/{Cargo.toml,src/lib.rs}`; `crates/web/src/network.rs` (deleted); `crates/web/src/lib.rs` or `main.rs`; `crates/web/Cargo.toml`; root `Cargo.toml` workspace members.

**Key changes**:
- `pub struct WebClientPlugin` (kept name; or `ClientWebLightyearPlugin`) — moved from `crates/web/src/network.rs`
- Build body: `cfg(target_family = "wasm")` selects `include_str!("../../certificates/digest.txt")` for `certificate_digest`; non-wasm uses empty string
- `client_addr = 0.0.0.0:5001` / `server_addr = 127.0.0.1:5001` — picked intentionally (resolves the port-mismatch noted in design); document the choice
- Constructs `ClientNetworkConfig` and adds `client_lightyear::ClientNetworkPlugin { config }`
- `crates/client_web_lightyear/Cargo.toml` — depends on `client_lightyear` and `protocol` (for `PROTOCOL_ID` / `PRIVATE_KEY`)
- `crates/web/Cargo.toml` — drops direct `client` and lightyear feature deps that the new crate provides; adds `client_web_lightyear = { workspace = true }`; keeps `client = { path = "../client", default-features = false }` only if game-side code is still consumed
- `crates/web/src/lib.rs` — uses `client_web_lightyear::WebClientPlugin` instead of constructing config locally

**Verify**: `cargo check-all` passes; `cargo test-all` passes; `cargo web` builds; browser connects to `cargo server` and gameplay works (manual); workspace dependency graph remains acyclic; `server` still has no production dep on `client`/`client_lightyear`/`client_web_lightyear`.

---

## Testing Checkpoints

- **After Phase 1**: `cargo check-all` + `cargo test-all` green. Only one transport variant deleted; UDP + WebSocket + WebTransport still work. Crossbeam test harness gone; `lightyear_crossbeam` dep gone.
- **After Phase 2**: `cargo check-all` + `cargo test-all` green. WebTransport-only. `ClientTransport`/`ServerTransport` enums gone; configs are flat. `crates/server/tests/integration.rs` empty or removed. Manual: `cargo client` + `cargo server` connect.
- **After Phase 3**: `cargo check-all` + `cargo test-all` green. `crates/client_lightyear` exists; `crates/client/src/network.rs` gone. Manual: `cargo client` + `cargo server` connect.
- **After Phase 4**: `cargo check-all` + `cargo test-all` green. `crates/server_lightyear` exists; `crates/server/src/network.rs` gone. Manual: `cargo client` + `cargo server` connect, replication works.
- **After Phase 5**: `cargo check-all` + `cargo test-all` green. `crates/client_web_lightyear` exists; `crates/web/src/network.rs` gone. Manual: browser connects via `cargo web`. Final state: zero transport-IO symbols outside the three new crates; workspace acyclic.
