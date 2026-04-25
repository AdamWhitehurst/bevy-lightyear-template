# Research Findings

## Q1: Server / Link / Transport architecture

### Findings

**`Server` is a component, not an entity type** (`git/lightyear/lightyear_link/src/server.rs:18-23`):

```rust
#[derive(Component, Default, Debug, PartialEq, Eq, Reflect)]
#[component(on_add = Server::on_add)]
#[relationship_target(relationship = LinkOf, linked_spawn)]
pub struct Server { links: Vec<Entity> }
```

- `on_add` hook inserts `Unlinked` if no link-state present (`server.rs:26-37`).
- Server lifecycle components `Starting`/`Started`/`Stopping`/`Stopped` live in `git/lightyear/lightyear_connection/src/server.rs:37-99`. `Started::on_add` registers the entity in `PeerMetadata` under `PeerId::Server` (`server.rs:56`).
- `Start`/`Stop` triggers drive lifecycle (`connection/src/server.rs:27-35`); `Start` re-triggers `LinkStart` (`server.rs:106-113`).

**`Link` is transport-agnostic** (`git/lightyear/lightyear_link/src/lib.rs:65-75`): holds `LinkReceiver`/`LinkSender` `VecDeque<Bytes>` buffers plus `LinkStats`/`LinkState`. Physical IO components on the same entity (`UdpIo`, `CrossbeamIo`, etc.) drain/fill those buffers. Link state is three marker components via hooks: `Linked`, `Linking`, `Unlinked` (`lib.rs:236-288`).

**Server→Links ownership uses Bevy ECS `Relationship`**: `LinkOf` (`lightyear_link/src/server.rs:62-83`) has `RelationshipTarget = Server`. `LinkOf::on_insert_hook` (`server.rs:86-131`) appends to `server.links`. No resource-side mapping — data lives in `Server.links`. `Server::unlinked` observer despawns all children when server goes `Unlinked` (`server.rs:39-57`).

**`Transport` is the reliability/channel layer, not IO** (`git/lightyear/lightyear_transport/src/channel/builder.rs:57-80`): `#[require(Link)]`, sits same-entity as `Link`, adds channels + `PacketBuilder` + `PriorityManager`. The word "Transport" in lightyear refers to this layer; raw socket layer is "Link"/"IO".

**Topology**: one server entity carries one IO component (`ServerUdpIo`, `SteamServerIo`, `WebTransportServerIo`) + optional `NetcodeServer`. Per-client child entities (`LinkOf { server }`) carry their own `Link`, optional `Transport`, no IO. Server's IO system iterates `server.collection()` to drive per-client Link buffers. Example for UDP: `lightyear_udp/src/server.rs:95-124` (send) and `:126-250` (receive — spawns new `LinkOf` on unknown src addr at `:211-229`).

**Bundle spawned per-client-connect per transport**:
- UDP: `(LinkOf, Link::new(None), Linked, PeerAddr, UdpLinkOfIO)` (`lightyear_udp/src/server.rs:211-229`)
- Netcode post-handshake insert: `(Connected, LocalId, RemoteId(Netcode), ClientOf, TokenUserData)` (`lightyear_netcode/src/server_plugin.rs:261-267`)
- WebTransport: `(LinkOf, Link::new(None), PeerAddr)` + `AeronetLinkOf` bridge (`lightyear_webtransport/src/server.rs:97-111`)
- Steam: `(LinkOf, Link::new(None), ClientOf, Connected, RemoteId(Steam), SteamClientOf)` (`lightyear_steam/src/server.rs:182-195`)

---

## Q2: Per-transport server-side API

### Findings

**`lightyear_udp::ServerUdpPlugin`** (`lightyear_udp/src/server.rs:65`):
- Component `ServerUdpIo` `#[require(Server)]` (`server.rs:32-37`). Marks entity as UDP server. `LocalAddr` also required on entity.
- `On<LinkStart>` observer binds `std::net::UdpSocket` non-blocking, inserts `Linked` (`server.rs:69-86`).
- `PreUpdate::receive` (`server.rs:126-250`) reads datagrams, dispatches into per-child `Link.recv`, spawns new `LinkOf` for unknown srcs.
- `PostUpdate::send` (`server.rs:95-124`) drains each child's `Link.send` via `send_to`.
- Per-client marker: `UdpLinkOfIO` zero-sized (`server.rs:41`).
- No config struct — uses `LocalAddr` component for bind.
- Layer: transport *component* on server entity. Handles child spawning internally.

**`lightyear_webtransport::WebTransportServerPlugin`** (`lightyear_webtransport/src/server.rs:19`):
- Registers `AeronetPlugin`, `ServerAeronetPlugin`, upstream `aeronet_webtransport::WebTransportServerPlugin` (`server.rs:29`).
- Component `WebTransportServerIo { certificate: Identity }` `#[require(Server)]` (`server.rs:50-52`).
- `On<LinkStart>::link` (`server.rs:55-80`) builds `wtransport::ServerConfig`, spawns child aeronet entity, calls `WebTransportServer::open(config).apply(child)`.
- `On<SessionRequest>` auto-accepts (`server.rs:82-84`); `On<Add, Session>::on_connection` spawns `LinkOf` and wires `AeronetLinkOf` (`server.rs:88-111`).
- Layer: transport component backed by aeronet child entity; `aeronet_webtransport` manages wtransport socket lifecycle via tokio.

**`lightyear_steam::SteamServerPlugin`** (`lightyear_steam/src/server.rs:25`):
- Registers `AeronetPlugin`, `ServerAeronetPlugin`, `aeronet_steam::server::SteamNetServerPlugin`.
- Component `SteamServerIo { target: ListenTarget, config: SessionConfig }` `#[require(Server)]` (`server.rs:55-58`).
- Per-client marker `SteamClientOf` `#[require(SkipNetcode)]` (`server.rs:63-64`) — Steam bypasses netcode.
- Six observers: `LinkStart`, `Add,Linked`→`Started`, `Start`→`LinkStart`, `SessionRequest` auto-accept, `Add,Session`→spawn `LinkOf`, `Disconnected`, `Stop`→`Close` (`server.rs:67-236`).
- Prerequisite: `SteamworksClient` Bevy resource; inserted via `SteamAppExt::add_steam_resources(app_id)` (`lightyear_steam/src/lib.rs:59-69`).

**`lightyear_crossbeam::CrossbeamPlugin`** (`lightyear_crossbeam/src/lib.rs:61`):
- Component `CrossbeamIo { sender, receiver }` with `#[require(Link::new(None))]`, `#[require(LocalAddr(LOCALHOST))]`, `#[require(PeerAddr(LOCALHOST))]` (`lib.rs:38-41`). **No `#[require(Server)]`**.
- Symmetric peer-to-peer, no server/link-of relationship. `CrossbeamIo::new_pair()` (`lib.rs:49-54`) returns both ends.
- `On<LinkStart>` immediately inserts `Linked` (`lib.rs:73-85`); `LinkReceiveSystems::BufferToLink` / `LinkSystems::Send` systems (`lib.rs:126,129`).
- Layer: peer link component; no child spawning, no server concept at this layer.

---

## Q3: Global-state conflicts

### Findings

**UDP**: No `init_resource`/`insert_resource`. No async runtime. No statics/lazy_static. Only guard is `is_plugin_added::<LinkPlugin>()` at `lightyear_udp/src/server.rs:255`; `ServerUdpPlugin` itself is *not* guarded against double-registration. Multi-instance: safe; distinct `ServerUdpIo` entities with different `LocalAddr` work independently.

**WebTransport**:
- `WebTransportRuntime` resource from `aeronet_webtransport/src/session.rs:60` via `init_resource`; `Default` impl builds `tokio::runtime::Builder::new_multi_thread()` and **`Box::leak`s** it (`aeronet_webtransport/src/runtime.rs:69-74`). Process-global in practice.
- rustls `CryptoProvider::install_default()` process-global (`aeronet_webtransport/src/session.rs:50-58`) — handles already-installed case with log+continue.
- `WebTransportSessionPlugin` guarded by `is_plugin_added` at `aeronet_webtransport/src/server/mod.rs:35`; `WebTransportServerPlugin` itself is not guarded.

**Steam**:
- `SteamworksClient` resource: inserted by `add_steam_resources` (`lightyear_steam/src/lib.rs:64`). `steamworks::Client::init_app(app_id)` at `lib.rs:60` — underlying `SteamAPI_Init` is process-global.
- `PollGroup` resource inserted lazily first time a `SteamNetIo` is added (`aeronet_steam/src/session.rs:125`) — single poll group across all Steam sessions.
- `run_callbacks` `PreUpdate` system added once by `add_steam_resources` (`lib.rs:65-67`); calling `add_steam_resources` twice would re-init + duplicate the system.
- `SteamNetServerPlugin`/`SteamServerPlugin` guard sub-plugins but not themselves.

**Crossbeam**: No singletons, no runtime, no statics. `is_plugin_added::<LinkPlugin>()` at `lib.rs:122`. Safe.

**Multi-instance conclusion for a single Bevy app**: UDP + WebTransport + Crossbeam can coexist without resource conflicts. Steam adds one process-global constraint (`SteamAPI_Init` called once) but is compatible with the others. Each transport plugin should only be `add_plugins`'d once per app.

---

## Q4: Netcode layering

### Findings

**Netcode is a connection-layer plugin operating on `Link` buffers, not a socket**. Systems `receive` (`lightyear_netcode/src/server_plugin.rs:192-293`) and `send` (`:102-190`) read/write `link.recv`/`link.send`. Requires an IO component to move bytes.

**System-schedule ordering** (`server_plugin.rs:352-369`):
- `PreUpdate`: `LinkSystems::Receive` → `ConnectionSystems::Receive` → `TransportSystems::Receive`
- `PostUpdate`: `TransportSystems::Send` → `ConnectionSystems::Send` → `LinkSystems::Send`

**Which transports use netcode**:
- UDP + netcode: typical pair. `NetcodeServer` `#[require(Server)]` (`server_plugin.rs:34`); UDP server also requires `Server`. Both components on same server entity.
- WebTransport / WebSocket: use `RawConnectionPlugin` (`lightyear_raw_connection/src/server.rs:19-119`). `on_link_of_linked` (`:44-60`) inserts `Connected` + `RemoteId(PeerId::Raw(...))` when `Linked` appears. System ordering omits `ConnectionSystems` (`:107-113`) — netcode never runs.
- Steam: bypasses via `SteamClientOf` `#[require(SkipNetcode)]` (`lightyear_steam/src/server.rs:63-64`). Netcode queries filter `Without<SkipNetcode>` (`server_plugin.rs:104-115, 204-206`). Identity from `PeerId::Steam(u64)` set at session accept (`steam/src/server.rs:186-194`).
- Crossbeam: no opinion; user layers whatever connection plugin they need.

**Cross-transport markers**: All connected link entities get `ClientOf` (`lightyear_connection/src/client_of.rs:8-10`). `SkipNetcode` flag (`:20-21`) explicitly supports mixing transport types on one server — its doc comment describes this scenario (`:13-19`). `PeerMetadata` resource holds all `PeerId` variants (`Netcode(u64)`, `Steam(u64)`, `Raw(SocketAddr)`) in one map.

---

## Q5: Project's `network.rs` behavior

### Findings

**Function is named `start_server`, not `spawn_server_transports`** (`crates/server/src/network.rs:93`). Called from `Startup` closure in `ServerNetworkPlugin::build` (`:74-76`).

**Config** (`network.rs:17-38`):
```rust
pub enum ServerTransport {
    Udp { port: u16 },
    WebTransport { port: u16 },
    WebSocket { port: u16 },
    Crossbeam { io: lightyear_crossbeam::CrossbeamIo },
}
pub struct ServerNetworkConfig {
    pub transports: Vec<ServerTransport>,
    pub bind_addr: [u8; 4], pub protocol_id: u64,
    pub private_key: [u8; 32], pub replication_interval: Duration,
}
```
Default: single `WebTransport { port: 5001 }` (`network.rs:43`).

**Per-variant bundle** (all include `Server::default()` + `NetcodeServer::new(...)`):

| Variant | Name | `LocalAddr` | IO component |
|---|---|---|---|
| UDP (`:99-124`) | "UDP Server" | yes | `ServerUdpIo::default()` |
| WebTransport (`:125-156`) | "WebTransport Server" | yes | `WebTransportServerIo { certificate }` |
| WebSocket (`:157-191`) | "WebSocket Server" | yes | `WebSocketServerIo { config }` |
| Crossbeam (`:192-207`) | "Crossbeam Server" | **no** | moved-in `CrossbeamIo` |

Each arm triggers `Start { entity: server }` after spawn. WebTransport loads `certificates/cert.pem`+`key.pem` synchronously via `load_webtransport_identity` blocking on `IoTaskPool` (`:80-91`; panics via `.expect()` on failure `:86`). WebSocket uses `Identity::self_signed(["localhost","127.0.0.1"])` (`:165`, panics on failure).

**Plugin features** (`crates/server/Cargo.toml:20`): `"server", "netcode", "udp", "webtransport", "websocket", "leafwing", "replication", "crossbeam"` — all compiled unconditionally. No `#[cfg]` gates in `network.rs`. Note: `"steam"` is **not** in the feature list.

**`ServerNetworkPlugin::build`** (`network.rs:66-78`): inserts `ServerNetworkConfig` resource, registers `ReplicationSender` as required-component on `ClientOf` (`:70-72`), adds `Startup` system. No transport sub-plugins added here (they presumably live elsewhere or come from `ServerPlugins`).

---

## Q6: `multi_transport.rs` test

### Findings

**Three independent `#[test]` fns, none ignored, no TODOs** (`crates/server/tests/multi_transport.rs`).

**Pattern common to all three**: build bare `App` with `MinimalPlugins` + `ServerPlugins { tick_duration }` + `protocol::ProtocolPlugin`. Spawn one server entity. Assert entity exists and has `NetcodeServer`. **Never call `App::update()` or `App::run()`**.

- `test_server_creates_udp_transport` (`:9-39`): spawns `(Name, NetcodeServer, LocalAddr(0.0.0.0:5067), ServerUdpIo::default())`.
- `test_server_creates_webtransport` (`:42-82`): uses in-memory `Identity::self_signed(["localhost","127.0.0.1","::1"])` (`:57-63`); bind `0.0.0.0:5031`. (Note: differs from `start_server` which loads PEM from disk.)
- `test_server_creates_websocket` (`:85-125`): `ws_config` bind `0.0.0.0:5052`, entity-level `LocalAddr(0.0.0.0:5042)` — **ports differ**. Self-signed identity.

Assertions on each (`:35-38, :78-81, :121-124`): `world.get_entity(server_id).is_ok()` + `world.get::<NetcodeServer>(server_id).is_some()`.

**Does the test run multiple transports in one run?** No. Each test has its own `App`, one server entity, one IO component. No Crossbeam test. No message-flow / connection-accept / child-spawn assertions. The `Start` trigger that `start_server` emits is absent.

---

## Q7: Multi-transport precedent in lightyear

### Findings

**Only one true multi-IO-same-entity site exists** (`git/lightyear/lightyear_tests/src/multi_server/steam.rs:19-77`): `test_steam_server_with_netcode_server` — `#[ignore]` because it needs a live Steam client. The server entity is spawned with `NetcodeServer` by the stepper, then `SteamServerIo` inserted via `server_mut().insert(...)`. Netcode clients connect over Crossbeam, Steam clients over Steam SDK, both resolving `ClientOf` to the same server entity. Doc-comment: "The NetcodeIO component oversees the Crossbeam clients. The SteamIO component oversees the Steam clients."

Prerequisite setup (`:19-28`):
```rust
fn add_steam_server_io(stepper: &mut ClientServerStepper) {
    stepper.server_app.add_steam_resources(STEAM_APP_ID);
    stepper.server_mut().insert(SteamServerIo {
        target: ListenTarget::Addr(server_addr),
        config: SessionConfig::default(),
    });
}
```

**All other precedent is one-of-N** (runtime/feature selected, not simultaneous):
- `git/lightyear/examples/common/src/server.rs:117-163` — `ExampleServer::on_add` matches a `ServerTransports` enum and inserts exactly one IO component per server entity.
- `git/lightyear/examples/simple_setup/src/server.rs:41-51` — single UDP server.

**Book docs are stale** (`git/lightyear/book/src/concepts/connection/multi_connection.md`): references old `NetConfig` / `ServerConfig` API that provided `Vec<NetConfig>`. Current entity-based API does not expose that.

**CHANGELOG** (`git/lightyear/lightyear/CHANGELOG.md:1214-1221`): PR #169 (`cb/multi-transport`) added the multi-transport unit test.

**Architectural enabler**: every server-side IO component observes `On<LinkStart>` independently. Multiple IO components on the same server entity each self-activate; nothing in lightyear enforces "one IO per server entity." This is the basis for the Steam+Netcode multi-io test.

---

## Q8: Runtime prerequisites

### Findings

**UDP**: No async runtime. `std::net::UdpSocket` non-blocking (`lightyear_udp/src/server.rs:80-81`). Bind in `On<LinkStart>` observer (main thread). `recv_from`/`send_to` run in Bevy `PreUpdate`/`PostUpdate` (`:95-250`) — server uses `iter_mut`, client (`lib.rs:111,131`) uses `par_iter_mut` on Bevy's task pool. One `SocketAddr` per entity; `Option<UdpSocket>` owned by component; no statics. OS rejects duplicate port bind.

**WebTransport**:
- Delegates to `aeronet_webtransport` (v0.19.1) → `wtransport` (v0.6.1) → `quinn`. All tokio-based (`Cargo.lock:9836, 189, 7041`).
- `lightyear_webtransport` itself spawns no tasks; tokio runs via Bevy's `IoTaskPool`. Example cert-load uses `async_compat::Compat` wrapping the `IoTaskPool` task: "we need async_compat because wtransport expects a tokio reactor" (`examples/common/src/server.rs:231-241`).
- The leaked `tokio::Runtime` inside `WebTransportRuntime` is the actual executor (`aeronet_webtransport/src/runtime.rs:69-74`).
- **TLS cert mandatory** — `WebTransportServerIo.certificate: Identity` required (`server.rs:50-52`), passed into `ServerConfig::builder().with_identity(...)` (`:67-69`). Formats: `Identity::self_signed(sans)` (needs `self-signed`/`dangerous-configuration` features, `Cargo.toml:30,36`) or `Identity::load_pemfiles(cert,key).await` (PEM). Client pins SHA-256 via `certificate_digest: String` (`client.rs:33, 117-120`).
- Cert loaded once per `On<LinkStart>`; not lazily per-connection.

**Steam**:
- Uses `steamworks` crate (v0.12.2, `Cargo.lock:8006-8015`). User calls `SteamAppExt::add_steam_resources(app_id)` which calls `steamworks::Client::init_app(app_id)` (`lightyear_steam/src/lib.rs:59-69`). Steam SDK enforces process-global init.
- AppID is `u32` passed directly (no env/config fallback in lightyear code). Doc at `lib.rs:54-55`: "The steam resources need to be inserted before the lightyear plugins."
- `networking_utils().init_relay_network_access()` (`lib.rs:62`) enables SDR.
- `run_callbacks()` `PreUpdate` system pumps on main thread (`lib.rs:65-67`).
- `aeronet_steam` uses `blocking` crate thread-pool (`Cargo.lock:2312-2322`) — **not tokio**. No `IoTaskPool` use.
- No lightyear-level socket fd access; Steam SDK owns sockets internally.

**Cross-cutting**:
- Shared executor: WebTransport relies on `IoTaskPool` (and its own leaked tokio runtime); UDP and Steam do not. No conflict.
- Shared Bevy main thread: Steam's `run_callbacks` and UDP's send/receive systems all run there; no contention beyond normal system scheduling.
- Socket ports: any two transports attempting to bind the same UDP port fail at OS level; lightyear does not deduplicate.

---

## Cross-cutting observations

- **IO, Connection, Transport are three separate layers**. Each layer's systems run in a distinct `SystemSet` (`LinkSystems`, `ConnectionSystems`, `TransportSystems`). Netcode is a *connection* plugin; UDP/WebTransport/Steam/Crossbeam are *IO* plugins. A server can skip the connection layer (WebTransport/Steam) or include it (UDP+Netcode) independently per-link via `SkipNetcode`.
- **Relationship-driven topology**: `Server` ↔ `LinkOf` is a Bevy `Relationship`. The server entity holds `Vec<Entity>` of children; each child is one client link. The IO component on the server spawns/despawns these children automatically.
- **Observer-based start**: every IO component has an `On<LinkStart>` observer that self-activates. Adding multiple IO components to one server entity means each observer fires on the same `LinkStart`, and each opens its own endpoint. No plugin or hook enforces exclusivity.
- **All variants in `network.rs` include `NetcodeServer`** regardless of transport. For WebTransport/WebSocket this is unnecessary (they use raw connection) — whether it's harmful or merely inert is not directly answered by the code but `RawConnectionPlugin` omits `ConnectionSystems` in ordering (`raw_connection/src/server.rs:107-113`), implying netcode systems run but find no matching entities for those transports.
- **Project features**: `steam` is not enabled in `crates/server/Cargo.toml:20`; only UDP, WebTransport, WebSocket, Crossbeam are available to `start_server`.

## Open areas

- Whether `NetcodeServer` on a WebTransport/WebSocket server entity causes observable issues (double-auth attempts, spurious handshakes) is not conclusively answered from the code alone.
- The book at `git/lightyear/book/src/concepts/connection/multi_connection.md` describes multi-transport via an old API; how the author envisioned the new entity-based API achieving the same is not documented in-tree.
- No project-level test exercises two `Start`-triggered transports in one `App::update()` loop; behavior at runtime when both run is only proven by the Steam+Netcode `#[ignore]`'d test in lightyear's own repo.
