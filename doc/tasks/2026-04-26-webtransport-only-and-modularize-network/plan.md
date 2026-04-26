# Implementation Plan

## Overview

Drop UDP/WebSocket/Crossbeam transports leaving WebTransport-only, then extract the trimmed networking code into three new workspace crates (`client_lightyear`, `server_lightyear`, `client_web_lightyear`). End state: `crates/{client,server,web}/src/network.rs` deleted; transport-IO symbols live only inside the three new crates; workspace dependency graph stays acyclic.

Commands referenced throughout: `cargo check-all` (`check --workspace`), `cargo test-native` (`make test-native`), `cargo client`, `cargo server`, `cargo web`. Per CLAUDE.md, **never run two `check`/`build`/`test` invocations concurrently**.

---

## Phase 1: Drop Crossbeam transport + harness

### Changes

#### 1. `crates/client/src/network.rs`
**Action**: modify

- Remove `use lightyear::crossbeam::CrossbeamIo;` (line 2).
- Remove `ClientTransport::Crossbeam(CrossbeamIo)` variant (line 21).
- Remove the `ClientTransport::Crossbeam(crossbeam_io) => { entity_builder.insert(crossbeam_io); }` arm (lines 119-121).

#### 2. `crates/server/src/network.rs`
**Action**: modify

- Remove `ServerTransport::Crossbeam { io: lightyear_crossbeam::CrossbeamIo }` variant (lines 24-27).
- Remove the entire `ServerTransport::Crossbeam { io } => { ... }` arm (lines 192-207).

#### 3. `crates/server/tests/integration.rs`
**Action**: delete crossbeam tests + harness

Delete every block whose body uses `lightyear_crossbeam`, `CrossbeamTestStepper`, `RawServer`, or `RawClient`. Specifically delete:
- `CrossbeamTestStepper` struct + impl (lines ~32-182).
- `MessageBuffer<M>` / `EventBuffer<E>` / `collect_messages` / `collect_events` helpers if they become unused after deletions (verify with grep — keep what `test_client_server_udp_connection` still uses).
- Helpers `add_server_map_systems`, `insert_test_terrain_defs`, `register_overworld_on_server`, `spawn_server_character`, `add_voxel_server_systems` — all crossbeam-dependent.
- All `#[test]` fns named `test_crossbeam_*` (lines 450, 597, 642, 711, 779).
- `test_client_server_plugin_initialization` (342) and `test_plugin_transport_configuration` (415) — both reference removed transports.
- `map_switch_request_triggers_transition_start` (937), `duplicate_switch_request_ignored` (1051), `server_and_client_spawn_matching_homebase_configs` (1138), `test_voxel_edit_ack_received` (1382), `test_server_pushes_chunks_without_request` (1518), `test_server_sends_unload_column_when_out_of_range` (1639).

Keep:
- `test_client_server_udp_connection` (238) — UDP test, removed in Phase 2.
- `test_voxel_messages_registered` (581) if it does not depend on crossbeam (verify; if it does, remove and adjust as needed).

After deletions, drop now-unused `use` lines.

#### 4. `Cargo.toml` (workspace root)
**Action**: modify

- Remove `lightyear_crossbeam = { path = "git/lightyear/lightyear_crossbeam" }` (line 50).

#### 5. `crates/client/Cargo.toml`
**Action**: modify

- Remove `"crossbeam"` from `lightyear` features list (line 16).

#### 6. `crates/server/Cargo.toml`
**Action**: modify

- Remove `"crossbeam"` from `lightyear` features list (line 20).
- Remove `lightyear_crossbeam = { workspace = true }` line (line 21).

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test-native` passes (only `test_client_server_udp_connection` + persistence tests run)
- [x] `grep -rn 'CrossbeamIo\|lightyear_crossbeam\|CrossbeamTestStepper\|RawClient\|RawServer' crates/` returns no hits

#### Manual
- [ ] `cargo client` + `cargo server` connect over WebTransport; basic gameplay works

---

## Phase 2: Drop UDP + WebSocket; collapse to single-transport configs

### Changes

#### 1. `crates/client/src/network.rs`
**Action**: modify — remove enum, flatten config, single insert

Replace the file with this shape (keep the existing observers and authentication construction; just remove `ClientTransport` and the `match`):

```rust
use bevy::prelude::*;
use lightyear::netcode::Key;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use lightyear::webtransport::client::WebTransportClientIo;
use protocol::*;
use std::net::SocketAddr;

const CERTIFICATE_DIGEST: &str = include_str!("../../../certificates/digest.txt");

/// Configuration for the client network plugin (WebTransport-only).
#[derive(Clone, Resource)]
pub struct ClientNetworkConfig {
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub client_id: u64,
    pub protocol_id: u64,
    pub private_key: [u8; 32],
    pub certificate_digest: String,
    pub token_expire_secs: i32,
}

impl Default for ClientNetworkConfig {
    fn default() -> Self {
        Self {
            client_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            server_addr: SocketAddr::from(([127, 0, 0, 1], 5001)),
            client_id: 0,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            certificate_digest: CERTIFICATE_DIGEST.trim().to_string(),
            token_expire_secs: 30,
        }
    }
}

pub struct ClientNetworkPlugin {
    pub config: ClientNetworkConfig,
}

impl Default for ClientNetworkPlugin {
    fn default() -> Self {
        Self { config: ClientNetworkConfig::default() }
    }
}

impl Plugin for ClientNetworkPlugin {
    fn build(&self, app: &mut App) {
        let config = self.config.clone();
        app.insert_resource(config.clone());
        app.add_systems(Startup, move |commands: Commands| {
            setup_client(commands, config.clone());
        });
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);
    }
}

fn setup_client(mut commands: Commands, config: ClientNetworkConfig) {
    let auth = Authentication::Manual {
        server_addr: config.server_addr,
        client_id: config.client_id,
        private_key: Key::from(config.private_key),
        protocol_id: config.protocol_id,
    };
    let netcode_config = NetcodeConfig {
        token_expire_secs: config.token_expire_secs,
        ..Default::default()
    };

    commands.spawn((
        Name::new("Client"),
        Client::default(),
        LocalAddr(config.client_addr),
        PeerAddr(config.server_addr),
        Link::new(None),
        ReplicationReceiver::default(),
        PredictionManager::default(),
        NetcodeClient::new(auth, netcode_config).unwrap(),
        WebTransportClientIo { certificate_digest: config.certificate_digest },
    ));
}

fn on_connected(trigger: On<Add, Connected>) {
    info!("Client {:?} connected!", trigger.entity);
}
fn on_disconnected(trigger: On<Add, Disconnected>) {
    info!("Client {:?} disconnected!", trigger.entity);
}
```

#### 2. `crates/client/src/lib.rs`
**Action**: modify

- Remove `ClientTransport` from the re-export at line 10:
  ```rust
  pub use network::ClientNetworkConfig;
  ```

#### 3. `crates/server/src/network.rs`
**Action**: modify — remove enum, flatten config, single spawn

Replace the file with this shape:

```rust
use async_compat::Compat;
use bevy::prelude::*;
use bevy::tasks::IoTaskPool;
use lightyear::netcode::{Key, NetcodeServer};
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use protocol::*;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

const CERT_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../certificates/cert.pem");
const KEY_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../certificates/key.pem");
const REPLICATION_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Resource)]
pub struct ServerNetworkConfig {
    pub bind_addr: IpAddr,
    pub port: u16,
    pub protocol_id: u64,
    pub private_key: [u8; 32],
    pub replication_interval: Duration,
}

impl Default for ServerNetworkConfig {
    fn default() -> Self {
        Self {
            bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: 5001,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            replication_interval: REPLICATION_INTERVAL,
        }
    }
}

pub struct ServerNetworkPlugin {
    pub config: ServerNetworkConfig,
}

impl Default for ServerNetworkPlugin {
    fn default() -> Self {
        Self { config: ServerNetworkConfig::default() }
    }
}

impl Plugin for ServerNetworkPlugin {
    fn build(&self, app: &mut App) {
        let config = self.config.clone();
        app.insert_resource(config.clone());
        app.register_required_components_with::<ClientOf, ReplicationSender>(|| {
            ReplicationSender::new(REPLICATION_INTERVAL, SendUpdatesMode::SinceLastAck, false)
        });
        app.add_systems(Startup, move |commands: Commands| {
            start_server(commands, config.clone());
        });
    }
}

fn load_webtransport_identity() -> lightyear::webtransport::prelude::Identity {
    IoTaskPool::get()
        .scope(|s| {
            s.spawn(Compat::new(async {
                lightyear::webtransport::prelude::Identity::load_pemfiles(CERT_PEM, KEY_PEM)
                    .await
                    .expect("Failed to load WebTransport certificates")
            }));
        })
        .pop()
        .unwrap()
}

fn start_server(mut commands: Commands, config: ServerNetworkConfig) {
    let wt_certificate = load_webtransport_identity();
    let digest = wt_certificate.certificate_chain().as_slice()[0].hash();
    info!("WebTransport certificate digest: {}", digest);

    let server = commands
        .spawn((
            Name::new("WebTransport Server"),
            Server::default(),
            NetcodeServer::new(server::NetcodeConfig {
                protocol_id: config.protocol_id,
                private_key: Key::from(config.private_key),
                ..default()
            }),
            LocalAddr(SocketAddr::from((config.bind_addr, config.port))),
            WebTransportServerIo { certificate: wt_certificate },
        ))
        .id();
    commands.trigger(Start { entity: server });
    info!("WebTransport server listening on {}:{}", config.bind_addr, config.port);
}
```

#### 4. `crates/web/src/network.rs`
**Action**: modify — match the new flat config shape

Replace the `ClientNetworkConfig { ..., transport: ClientTransport::WebTransport { certificate_digest }, .. }` construction with:

```rust
let config = ClientNetworkConfig {
    client_addr: SocketAddr::from(([0, 0, 0, 0], 5001)),
    server_addr: SocketAddr::from(([127, 0, 0, 1], 5001)),
    client_id: 0,
    protocol_id: PROTOCOL_ID,
    private_key: PRIVATE_KEY,
    certificate_digest,
    ..default()
};
```

Remove `ClientTransport` from the re-export line:
```rust
pub use client::network::{ClientNetworkConfig, ClientNetworkPlugin};
```

#### 5. `crates/server/tests/integration.rs`
**Action**: delete `test_client_server_udp_connection` (the only test surviving Phase 1 that uses UDP). After this, the file may be empty — if so, delete it. If `test_voxel_messages_registered` (or any other non-network test) survived Phase 1, leave it.

#### 6. `crates/client/Cargo.toml`
**Action**: modify

- Remove `"udp"` from `lightyear` features (line 16). Final list: `["client", "netcode", "webtransport", "leafwing", "prediction", "replication", "interpolation"]`.

#### 7. `crates/server/Cargo.toml`
**Action**: modify

- Remove `"udp"` and `"websocket"` from `lightyear` features (line 20). Final list: `["server", "netcode", "webtransport", "leafwing", "replication"]`.

#### 8. `crates/web/Cargo.toml`
**Action**: modify

- Remove `"websocket"` from `lightyear` features (line 39-48 block).

#### 9. `Cargo.toml` (workspace root)
**Action**: modify

- Remove `"raw_connection"` from the `lightyear` umbrella features list. After Phase 1 deletions, no production code references `RawClient`/`RawServer`. Final block:
  ```toml
  lightyear = { path = "git/lightyear/lightyear", features = ["leafwing"] }
  ```

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test-native` passes
- [x] `grep -rn 'ClientTransport\|ServerTransport\|UdpIo\|ServerUdpIo\|WebSocketServerIo' crates/` returns no hits

#### Manual
- [ ] `cargo client` + `cargo server` connect over WebTransport; gameplay works
- [ ] `cargo web` builds (still routed through `crates/web/src/network.rs` for now)

---

## Phase 3: Extract `client_lightyear` crate

### Changes

#### 1. `crates/client_lightyear/Cargo.toml`
**Action**: create

```toml
[package]
name = "client_lightyear"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true, default-features = true }
lightyear = { workspace = true, features = [
    "client",
    "netcode",
    "webtransport",
    "leafwing",
    "prediction",
    "replication",
    "interpolation",
] }
protocol = { workspace = true }
```

#### 2. `crates/client_lightyear/src/lib.rs`
**Action**: create

```rust
//! Generic native+WASM WebTransport client setup.
mod connection;
mod netcode;
mod webtransport;

pub use connection::{ClientNetworkConfig, ClientNetworkPlugin};
```

#### 3. `crates/client_lightyear/src/connection.rs`
**Action**: create

Owns the `ClientNetworkConfig` resource, the `ClientNetworkPlugin`, the `setup_client` startup system, and the `on_connected`/`on_disconnected` observers. Note: `certificate_digest` is required (no default const — binaries supply it).

```rust
use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use protocol::{PROTOCOL_ID, PRIVATE_KEY};
use std::net::SocketAddr;

#[derive(Clone, Resource)]
pub struct ClientNetworkConfig {
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub client_id: u64,
    pub protocol_id: u64,
    pub private_key: [u8; 32],
    pub certificate_digest: String,
    pub token_expire_secs: i32,
}

impl Default for ClientNetworkConfig {
    fn default() -> Self {
        Self {
            client_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            server_addr: SocketAddr::from(([127, 0, 0, 1], 5001)),
            client_id: 0,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            certificate_digest: String::new(),
            token_expire_secs: 30,
        }
    }
}

pub struct ClientNetworkPlugin {
    pub config: ClientNetworkConfig,
}

impl Default for ClientNetworkPlugin {
    fn default() -> Self {
        Self { config: ClientNetworkConfig::default() }
    }
}

impl Plugin for ClientNetworkPlugin {
    fn build(&self, app: &mut App) {
        let config = self.config.clone();
        app.insert_resource(config.clone());
        app.add_systems(Startup, move |commands: Commands| {
            spawn_client_entity(commands, config.clone());
        });
        app.add_observer(on_connected);
        app.add_observer(on_disconnected);
    }
}

fn spawn_client_entity(mut commands: Commands, config: ClientNetworkConfig) {
    let netcode_client = crate::netcode::build_netcode_client(&config);
    let webtransport_io = crate::webtransport::build_io(&config);

    commands.spawn((
        Name::new("Client"),
        Client::default(),
        LocalAddr(config.client_addr),
        PeerAddr(config.server_addr),
        Link::new(None),
        ReplicationReceiver::default(),
        PredictionManager::default(),
        netcode_client,
        webtransport_io,
    ));
}

fn on_connected(trigger: On<Add, Connected>) {
    info!("Client {:?} connected!", trigger.entity);
}
fn on_disconnected(trigger: On<Add, Disconnected>) {
    info!("Client {:?} disconnected!", trigger.entity);
}
```

#### 4. `crates/client_lightyear/src/netcode.rs`
**Action**: create

```rust
use lightyear::netcode::Key;
use lightyear::prelude::client::*;
use lightyear::prelude::*;

use crate::connection::ClientNetworkConfig;

pub(crate) fn build_netcode_client(config: &ClientNetworkConfig) -> NetcodeClient {
    let auth = Authentication::Manual {
        server_addr: config.server_addr,
        client_id: config.client_id,
        private_key: Key::from(config.private_key),
        protocol_id: config.protocol_id,
    };
    let netcode_config = NetcodeConfig {
        token_expire_secs: config.token_expire_secs,
        ..Default::default()
    };
    NetcodeClient::new(auth, netcode_config).unwrap()
}
```

#### 5. `crates/client_lightyear/src/webtransport.rs`
**Action**: create

```rust
use lightyear::webtransport::client::WebTransportClientIo;

use crate::connection::ClientNetworkConfig;

pub(crate) fn build_io(config: &ClientNetworkConfig) -> WebTransportClientIo {
    WebTransportClientIo { certificate_digest: config.certificate_digest.clone() }
}
```

#### 6. `crates/client/src/network.rs`
**Action**: delete

#### 7. `crates/client/src/lib.rs`
**Action**: modify

Remove `pub mod network;` and the `network::ClientNetworkConfig` re-export. Replace with a re-export from the new crate so existing call sites (`use client::network::ClientNetworkConfig` etc.) keep compiling — actually since the only re-export is `pub use network::ClientNetworkConfig;` and only `crates/web/src/network.rs` uses it (`pub use client::network::{...}`), we can either:
- (a) keep a slim `pub mod network { pub use client_lightyear::*; }` shim, or
- (b) update the web crate to import directly from `client_lightyear`.

Choose (b) — Phase 5 deletes `crates/web/src/network.rs` anyway. Update `crates/web/src/network.rs` to `use client_lightyear::{ClientNetworkConfig, ClientNetworkPlugin};` instead of `use client::network::...`.

Final `crates/client/src/lib.rs`:
```rust
pub mod gameplay;
pub mod map;
pub mod transition;
pub mod world_object;

pub use lightyear::netcode::{Key, NetcodeClient};
pub use lightyear::prelude::client::NetcodeConfig;
pub use lightyear::prelude::Authentication;
pub use client_lightyear::ClientNetworkConfig;
```

#### 8. `crates/client/src/main.rs`
**Action**: modify

Replace `use network::{ClientNetworkConfig, ClientNetworkPlugin};` (line 14) with:
```rust
use client_lightyear::{ClientNetworkConfig, ClientNetworkPlugin};
```
Remove the `pub mod network;` line at top of file (line was via lib.rs; main.rs imports from there). Inspect imports in `main.rs` — if `network` was referenced, switch to `client_lightyear`.

The `ClientNetworkConfig::default()` here must produce a valid digest. Add:
```rust
let network_config = ClientNetworkConfig {
    client_id,
    certificate_digest: include_str!("../../../certificates/digest.txt").trim().to_string(),
    ..Default::default()
};
```

#### 9. `crates/client/Cargo.toml`
**Action**: modify

- Add `client_lightyear = { workspace = true }` to `[dependencies]`.
- Drop the lightyear features that have moved into `client_lightyear`. Final lightyear features for `client`: keep `"client", "netcode", "leafwing", "prediction", "replication", "interpolation"` only if still consumed by remaining `crates/client/src/**` (likely yes — `gameplay/map/transition` use these). Drop `"webtransport"` since transport-IO insert is now elsewhere; verify with `cargo check-all`. If a missing feature breaks the build, restore it.
- Keep `default = ["file_watcher"]` and `tracy` features.

#### 10. `crates/web/src/network.rs`
**Action**: modify

Update imports:
```rust
use client_lightyear::{ClientNetworkConfig, ClientNetworkPlugin};
```
(Remove the `pub use client::network::{...}` line.)

#### 11. `crates/web/Cargo.toml`
**Action**: modify

Add `client_lightyear = { workspace = true }`.

#### 12. `Cargo.toml` (workspace root)
**Action**: modify

Add to `[workspace.members]`: `"crates/client_lightyear"`.
Add to `[workspace.dependencies]`: `client_lightyear = { path = "crates/client_lightyear" }`.

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test-native` passes
- [x] `grep -rn 'mod network' crates/client/src/` returns nothing
- [x] `grep -rn 'WebTransportClientIo\|NetcodeClient::new' crates/client/src/ crates/web/src/` returns nothing (these now live only in `client_lightyear`)

#### Manual
- [ ] `cargo client` connects to `cargo server`; gameplay works

---

## Phase 4: Extract `server_lightyear` crate

### Changes

#### 1. `crates/server_lightyear/Cargo.toml`
**Action**: create

```toml
[package]
name = "server_lightyear"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true, default-features = true }
lightyear = { workspace = true, features = [
    "server",
    "netcode",
    "webtransport",
    "leafwing",
    "replication",
] }
protocol = { workspace = true }
async-compat = "0.2"
```

#### 2. `crates/server_lightyear/src/lib.rs`
**Action**: create

```rust
//! WebTransport server setup.
mod connection;
mod netcode;
mod webtransport;

pub use connection::{ServerNetworkConfig, ServerNetworkPlugin};
```

#### 3. `crates/server_lightyear/src/connection.rs`
**Action**: create

```rust
use bevy::prelude::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use protocol::{PROTOCOL_ID, PRIVATE_KEY};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

const REPLICATION_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Resource)]
pub struct ServerNetworkConfig {
    pub bind_addr: IpAddr,
    pub port: u16,
    pub protocol_id: u64,
    pub private_key: [u8; 32],
    pub cert_pem_path: PathBuf,
    pub key_pem_path: PathBuf,
    pub replication_interval: Duration,
}

impl Default for ServerNetworkConfig {
    fn default() -> Self {
        Self {
            bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: 5001,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            cert_pem_path: PathBuf::new(),
            key_pem_path: PathBuf::new(),
            replication_interval: REPLICATION_INTERVAL,
        }
    }
}

pub struct ServerNetworkPlugin {
    pub config: ServerNetworkConfig,
}

impl Default for ServerNetworkPlugin {
    fn default() -> Self {
        Self { config: ServerNetworkConfig::default() }
    }
}

impl Plugin for ServerNetworkPlugin {
    fn build(&self, app: &mut App) {
        let config = self.config.clone();
        let interval = config.replication_interval;
        app.insert_resource(config.clone());
        app.register_required_components_with::<ClientOf, ReplicationSender>(move || {
            ReplicationSender::new(interval, SendUpdatesMode::SinceLastAck, false)
        });
        app.add_systems(Startup, move |commands: Commands| {
            start_server(commands, config.clone());
        });
    }
}

fn start_server(mut commands: Commands, config: ServerNetworkConfig) {
    let netcode = crate::netcode::build_netcode_server(&config);
    let webtransport_io = crate::webtransport::build_io(&config);

    let server = commands
        .spawn((
            Name::new("WebTransport Server"),
            Server::default(),
            netcode,
            LocalAddr(SocketAddr::from((config.bind_addr, config.port))),
            webtransport_io,
        ))
        .id();
    commands.trigger(Start { entity: server });
    info!("WebTransport server listening on {}:{}", config.bind_addr, config.port);
}
```

#### 4. `crates/server_lightyear/src/netcode.rs`
**Action**: create

```rust
use lightyear::netcode::{Key, NetcodeServer};
use lightyear::prelude::server;
use bevy::prelude::*;

use crate::connection::ServerNetworkConfig;

pub(crate) fn build_netcode_server(config: &ServerNetworkConfig) -> NetcodeServer {
    NetcodeServer::new(server::NetcodeConfig {
        protocol_id: config.protocol_id,
        private_key: Key::from(config.private_key),
        ..default()
    })
}
```

#### 5. `crates/server_lightyear/src/webtransport.rs`
**Action**: create

```rust
use async_compat::Compat;
use bevy::tasks::IoTaskPool;
use bevy::prelude::*;
use lightyear::prelude::server::WebTransportServerIo;
use lightyear::webtransport::prelude::Identity;
use std::path::Path;

use crate::connection::ServerNetworkConfig;

pub(crate) fn build_io(config: &ServerNetworkConfig) -> WebTransportServerIo {
    let certificate = load_identity(&config.cert_pem_path, &config.key_pem_path);
    let digest = certificate.certificate_chain().as_slice()[0].hash();
    info!("WebTransport certificate digest: {}", digest);
    WebTransportServerIo { certificate }
}

fn load_identity(cert_pem: &Path, key_pem: &Path) -> Identity {
    let cert = cert_pem.to_path_buf();
    let key = key_pem.to_path_buf();
    IoTaskPool::get()
        .scope(|s| {
            s.spawn(Compat::new(async move {
                Identity::load_pemfiles(&cert, &key)
                    .await
                    .expect("Failed to load WebTransport certificates")
            }));
        })
        .pop()
        .unwrap()
}
```

#### 6. `crates/server/src/network.rs`
**Action**: delete

#### 7. `crates/server/src/lib.rs`
**Action**: modify

Remove `pub mod network;`.

#### 8. `crates/server/src/main.rs`
**Action**: modify

Remove `pub mod network;` and `use network::ServerNetworkPlugin;`. Replace with:
```rust
use server_lightyear::{ServerNetworkConfig, ServerNetworkPlugin};
```

Replace the plugin add line with:
```rust
.add_plugins(ServerNetworkPlugin {
    config: ServerNetworkConfig {
        cert_pem_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../certificates/cert.pem").into(),
        key_pem_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../certificates/key.pem").into(),
        ..Default::default()
    },
})
```

#### 9. `crates/server/Cargo.toml`
**Action**: modify

- Add `server_lightyear = { workspace = true }`.
- Drop lightyear features that have moved (`webtransport`). Verify with `cargo check-all`; restore any feature still consumed by remaining `crates/server/src/**` code.
- Remove `async-compat = "0.2"` (the only consumer was the deleted `network.rs`; verify with `grep -n async_compat crates/server/src`). Restore if check fails.

#### 10. `Cargo.toml` (workspace root)
**Action**: modify

Add to `[workspace.members]`: `"crates/server_lightyear"`.
Add to `[workspace.dependencies]`: `server_lightyear = { path = "crates/server_lightyear" }`.

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test-native` passes
- [x] `grep -rn 'mod network' crates/server/src/` returns nothing
- [x] `grep -rn 'WebTransportServerIo\|NetcodeServer::new\|load_pemfiles' crates/server/src/` returns nothing

#### Manual
- [ ] `cargo client` + `cargo server` connect; replication works (entity spawns visible from client)

---

## Phase 5: Extract `client_web_lightyear` crate

### Changes

#### 1. `crates/client_web_lightyear/Cargo.toml`
**Action**: create

```toml
[package]
name = "client_web_lightyear"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true, default-features = true }
client_lightyear = { workspace = true }
protocol = { workspace = true }
```

#### 2. `crates/client_web_lightyear/src/lib.rs`
**Action**: create

```rust
//! WASM-specific WebTransport client preset.
//!
//! Picks the digest under `cfg(target_family = "wasm")` and applies the
//! browser-side address defaults, then delegates to `client_lightyear::ClientNetworkPlugin`.

use bevy::prelude::*;
use client_lightyear::{ClientNetworkConfig, ClientNetworkPlugin};
use protocol::{PROTOCOL_ID, PRIVATE_KEY};
use std::net::SocketAddr;

pub struct WebClientPlugin;

impl Default for WebClientPlugin {
    fn default() -> Self { Self }
}

impl Plugin for WebClientPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(target_family = "wasm")]
        let certificate_digest =
            include_str!("../../../certificates/digest.txt").trim().to_string();
        #[cfg(not(target_family = "wasm"))]
        let certificate_digest = String::new();

        // Browser-side defaults: WT client binds locally to ephemeral; server is on 5001.
        // (Resolves the legacy port-mismatch where web bound client_addr to 5001.)
        let config = ClientNetworkConfig {
            client_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            server_addr: SocketAddr::from(([127, 0, 0, 1], 5001)),
            client_id: 0,
            protocol_id: PROTOCOL_ID,
            private_key: PRIVATE_KEY,
            certificate_digest,
            ..default()
        };
        app.add_plugins(ClientNetworkPlugin { config });
    }
}
```

#### 3. `crates/web/src/network.rs`
**Action**: delete

#### 4. `crates/web/src/lib.rs`
**Action**: modify

Replace contents with:
```rust
pub use client_web_lightyear::WebClientPlugin;
pub use protocol::*;
```

#### 5. `crates/web/src/main.rs`
**Action**: modify

Remove `pub mod network;` and `use network::WebClientPlugin;` (lines 12-13). Replace with:
```rust
use client_web_lightyear::WebClientPlugin;
```

#### 6. `crates/web/Cargo.toml`
**Action**: modify

- Replace direct `client_lightyear` dep (added Phase 3) with `client_web_lightyear = { workspace = true }`. Keep `client = { path = "../client", default-features = false }` only if remaining `crates/web/src/**` uses game-side `client::*` modules (it does — gameplay/map/transition).
- Drop `lightyear` direct features that the new crate now provides — keep only what `crates/web/src/main.rs` and remaining lib code use directly. Most likely the `lightyear` dep can be removed from `web/Cargo.toml` entirely since `ClientPlugins` is added in `main.rs` via `lightyear::prelude::client::*` re-exported through `client`. Verify with `cargo check-all`.

#### 7. `Cargo.toml` (workspace root)
**Action**: modify

Add to `[workspace.members]`: `"crates/client_web_lightyear"`.
Add to `[workspace.dependencies]`: `client_web_lightyear = { path = "crates/client_web_lightyear" }`.

### Verification

#### Automated
- [x] `cargo check-all` passes
- [x] `cargo test-native` passes
- [x] `cargo web-build` (or `cargo web` build phase) succeeds
- [x] `grep -rn 'mod network' crates/web/src/` returns nothing
- [x] `grep -rn 'ClientTransport\|ServerTransport\|UdpIo\|WebSocketServerIo\|CrossbeamIo' crates/` returns nothing
- [x] Final dependency check: `server` Cargo.toml has no `client*`/`web*` production dep; `web` does not depend on `server*`

#### Manual
- [ ] `cargo web` serves; browser connects to running `cargo server`; gameplay works
- [ ] Workspace dependency graph remains acyclic (visual confirmation: only `web → client_web_lightyear → client_lightyear`, `client → client_lightyear`, `server → server_lightyear`)
