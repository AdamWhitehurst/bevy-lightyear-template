---
date: 2025-11-16T21:42:59-08:00
researcher: Claude Code
git_commit: 1e807041c1ef77b7eac4f51d8c31909437791682
branch: master
repository: bevy-lightyear-template
topic: "Multi-crate setup for Lightyear server/client with UDP, WebTransport, and WebSocket"
tags: [research, codebase, lightyear, bevy, networking, multi-transport, server, client, wasm]
status: complete
last_updated: 2025-11-17
last_updated_by: Claude Code
last_updated_note: "Added answers to open questions and WASM testing options"
---

# Research: Multi-crate Setup for Lightyear Server/Client

**Date**: 2025-11-16T21:42:59-08:00
**Researcher**: Claude Code
**Git Commit**: 1e807041c1ef77b7eac4f51d8c31909437791682
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to properly set up:
- `crates/server` - dedicated authoritative server that supports UDP, WebTransport, and Websocket simultaneously
- `crates/client` - a native client app that connects to server via UDP
- `crates/web` - a WASM client app that connects to server via WebTransport or Websocket
- Using `bevy` and `lightyear` dependencies

## Summary

The project currently has a workspace with two basic crates (`crates/server` and `crates/client`) that have minimal Bevy setup but no Lightyear integration. Based on the Lightyear source code patterns found in `git/lightyear/`, a proper multi-transport setup requires:

1. **Server**: Simultaneously supports multiple transports by adding feature-gated transport plugins and spawning server entities with transport-specific IO components
2. **Native Client**: Uses UDP transport with `UdpIo` component and `NetcodeClient` for authentication
3. **WASM Client**: Uses WebTransport or WebSocket with platform-specific certificate handling via conditional compilation
4. **Workspace Features**: Configure Cargo.toml to enable specific transport features per crate
5. **Shared Protocol**: Use a shared crate for message definitions and channel configuration

## Detailed Findings

### Current Project Structure

**Workspace Configuration**: `/home/aw/Dev/bevy-lightyear-template/Cargo.toml`
- 2 members: `crates/client`, `crates/server`
- Shared dependencies: bevy 0.17.2, lightyear 0.25.5, avian3d 0.4.1
- Resolver version 2

**Client Crate**: `crates/client/src/main.rs:1-13`
- Minimal Bevy app with DefaultPlugins and Camera3d
- No Lightyear integration present

**Server Crate**: `crates/server/src/main.rs:1-11`
- Minimal Bevy app with MinimalPlugins
- No Lightyear integration present

**Missing**: No `crates/web` directory exists for WASM client

### Lightyear Transport Architecture

**Three-Layer System**:
1. **IO Layer**: Raw byte transmission (UDP, WebSocket, WebTransport)
2. **Connection Layer**: Authentication and state management (Netcode)
3. **Transport Layer**: Channels with reliability/ordering guarantees

**Component Hierarchy**:
- All IO transports require `Link` component for buffering
- Connection layers (NetcodeClient/Server) require `Link` and `Client`/`Server` components
- Transport layer manages channels over the connection

Source: `git/lightyear/lightyear_transport/src/channel/builder.rs:61-88`

### Server: Multi-Transport Support

**Pattern: Feature-Gated Plugin Architecture**

Location: `git/lightyear/lightyear/src/server.rs:47-84`

The server automatically supports all enabled transports by adding feature-gated plugins:

```rust
impl PluginGroup for ServerPlugins {
    fn build(self) -> PluginGroupBuilder {
        // ...
        #[cfg(all(feature = "udp", not(target_family = "wasm")))]
        let builder = builder.add(lightyear_udp::server::ServerUdpPlugin);

        #[cfg(all(feature = "webtransport", not(target_family = "wasm")))]
        let builder = builder.add(lightyear_webtransport::server::WebTransportServerPlugin);

        #[cfg(all(feature = "websocket", not(target_family = "wasm")))]
        let builder = builder.add(lightyear_websocket::server::WebSocketServerPlugin);
        // ...
    }
}
```

**Pattern: Multi-Transport Server Entity**

Location: `git/lightyear/examples/common/src/server.rs:65-160`

For runtime transport selection, servers use an enum-based pattern with component hooks:

```rust
#[derive(Clone, Debug)]
pub enum ServerTransports {
    Udp { local_port: u16 },
    WebTransport { local_port: u16, certificate: WebTransportCertificateSettings },
    WebSocket { local_port: u16 },
}

#[derive(Component)]
#[component(on_add = ExampleServer::on_add)]
pub struct ExampleServer {
    pub transport: ServerTransports,
    // ...
}
```

The `on_add` hook inserts transport-specific IO components based on the transport variant.

**Transport-Specific IO Components**:
- UDP: `ServerUdpIo::default()` + `LocalAddr(SocketAddr)` (server.rs:103-108)
- WebTransport: `WebTransportServerIo { certificate }` + `LocalAddr` (server.rs:109-121)
- WebSocket: `WebSocketServerIo { config }` + `LocalAddr` (server.rs:122-136)

All non-Steam transports also add `NetcodeServer` for authentication (server.rs:95-102).

**Simultaneous Transport Support**:

To support all transports simultaneously, a server spawns **multiple server entities**, one per transport. Each entity has:
- Different `LocalAddr` (can be same port, different IPs, or different ports)
- Transport-specific IO component
- Shared `NetcodeServer` configuration (same protocol_id, private_key)

Example pattern from simple_setup:
```rust
// Spawn UDP server entity
commands.spawn((
    NetcodeServer::new(NetcodeConfig::default()),
    LocalAddr(SocketAddr::from(([0, 0, 0, 0], 5000))),
    ServerUdpIo::default(),
));

// Spawn WebTransport server entity
commands.spawn((
    NetcodeServer::new(NetcodeConfig::default()),
    LocalAddr(SocketAddr::from(([0, 0, 0, 0], 5000))),
    WebTransportServerIo { certificate },
));
```

Source: `git/lightyear/lightyear_udp/src/server.rs:28-263`, `git/lightyear/lightyear_webtransport/src/server.rs:48-112`

### Native Client: UDP Transport

**Pattern: UDP Client Entity**

Location: `git/lightyear/examples/simple_setup/src/client.rs:22-43`

Native clients spawn a single client entity with UDP-specific components:

```rust
fn startup(mut commands: Commands) -> Result {
    let auth = Authentication::Manual {
        server_addr: SERVER_ADDR,
        client_id: 0,
        private_key: Key::default(),
        protocol_id: 0,
    };
    let client = commands
        .spawn((
            Client::default(),
            LocalAddr(CLIENT_ADDR),
            PeerAddr(SERVER_ADDR),
            Link::new(None),
            ReplicationReceiver::default(),
            NetcodeClient::new(auth, NetcodeConfig::default())?,
            UdpIo::default(),
        ))
        .id();
    commands.trigger(Connect { entity: client });
    Ok(())
}
```

**Required Components**:
- `Client::default()` - core client marker
- `LocalAddr(SocketAddr)` - local UDP bind address
- `PeerAddr(SocketAddr)` - server address
- `Link::new(Option<RecvLinkConditioner>)` - transport abstraction
- `NetcodeClient` - authentication layer
- `UdpIo::default()` - UDP transport

**UDP Implementation Details**:

Location: `git/lightyear/lightyear_udp/src/lib.rs:51-193`

- Non-blocking socket binding on `LinkStart` trigger
- MTU: 1472 bytes (`lightyear_udp/src/lib.rs:43`)
- Parallel send/receive systems using `par_iter_mut()`
- `BytesMut` buffer for datagram assembly

**Plugin Setup**:

```rust
app.add_plugins(ClientPlugins {
    tick_duration: Duration::from_secs_f64(1.0 / 60.0),
});
```

Source: `git/lightyear/examples/simple_setup/src/main.rs:39-49`

### WASM Client: WebTransport/WebSocket

**Pattern: Platform-Conditional Transport**

Location: `git/lightyear/examples/common/src/client.rs:17-124`

WASM clients use conditional compilation to select WebTransport or WebSocket:

```rust
#[derive(Clone, Debug)]
pub enum ClientTransports {
    #[cfg(not(target_family = "wasm"))]
    Udp,
    WebTransport,
    WebSocket,
}

#[derive(Component)]
#[component(on_add = ExampleClient::on_add)]
pub struct ExampleClient {
    pub transport: ClientTransports,
    // ...
}
```

**WebTransport Setup**:

```rust
ClientTransports::WebTransport => {
    add_netcode(&mut entity_mut)?;
    let certificate_digest = {
        #[cfg(target_family = "wasm")]
        {
            include_str!("../../../certificates/digest.txt").to_string()
        }
        #[cfg(not(target_family = "wasm"))]
        {
            "".to_string()
        }
    };
    entity_mut.insert(WebTransportClientIo { certificate_digest });
}
```

**WebSocket Setup**:

```rust
ClientTransports::WebSocket => {
    add_netcode(&mut entity_mut)?;
    let config = {
        #[cfg(target_family = "wasm")]
        {
            ClientConfig::default()
        }
        #[cfg(not(target_family = "wasm"))]
        {
            ClientConfig::builder().with_no_cert_validation()
        }
    };
    entity_mut.insert(WebSocketClientIo { config });
}
```

Source: `git/lightyear/examples/common/src/client.rs:77-97`

**Certificate Handling for WebTransport**:

WASM clients require the server certificate digest for validation. Two approaches:

1. **Embedded digest** (examples pattern): `include_str!()` at compile time
2. **Runtime fetch**: Fetch digest from server or use env variable

Native clients can use empty digest with `dangerous-configuration` feature for testing.

Location: `git/lightyear/lightyear_webtransport/src/client.rs:73-128`

**WASM-Specific Plugin Setup**:

Location: `git/lightyear/lightyear/src/client.rs:61`

```rust
#[cfg(target_family = "wasm")]
let builder = builder.add(lightyear_web::ClientPlugin);
```

### Certificate Configuration (WebTransport)

**Pattern: Self-Signed Certificates**

Location: `git/lightyear/examples/common/src/server.rs:172-248`

Servers use `WebTransportCertificateSettings` enum:

```rust
pub enum WebTransportCertificateSettings {
    AutoSelfSigned(Vec<String>),  // SANs list
    FromFile { cert: String, key: String },
}
```

Default SANs: `["localhost", "127.0.0.1", "::1"]`

Environment variable support:
- `ARBITRIUM_PUBLIC_IP` - auto-added to SAN for Edgegap deployment
- `SELF_SIGNED_SANS` - comma-separated additional SANs

The certificate digest is computed and can be:
- Written to file for WASM inclusion
- Passed via environment variable to WASM app
- Ignored on native with `dangerous-configuration` feature

Source: `git/lightyear/examples/common/src/server.rs:196-245`

### Feature Configuration

**Lightyear Features**:

Location: `git/lightyear/lightyear/Cargo.toml:159-177`

```toml
udp = ["dep:lightyear_udp", "std"]
webtransport = ["std", "dep:lightyear_webtransport"]
webtransport_self_signed = ["lightyear_webtransport/self-signed"]
websocket = ["std", "dep:lightyear_websocket"]
websocket_self_signed = ["lightyear_websocket/self-signed"]
```

**Example Workspace Features**:

Location: `git/lightyear/examples/common/Cargo.toml:14-26`

```toml
[features]
default = ["client", "server", "netcode", "udp"]
client = ["lightyear/client"]
server = ["lightyear/server"]
netcode = ["lightyear/netcode"]
udp = ["lightyear/udp"]
```

**Compile-Time Validation**:

Location: `git/lightyear/examples/common/src/lib.rs:72-95`

```rust
#[cfg(all(feature = "steam", target_family = "wasm"))]
compile_error!("steam feature is not supported in wasm");

#[cfg(all(feature = "server", target_family = "wasm"))]
compile_error!("server feature is not supported in wasm");
```

### Channel Configuration

**Transport Layer Channels**:

Location: `git/lightyear/lightyear_transport/src/channel/builder.rs:32-55`

Channels define reliability and ordering guarantees:

```rust
pub struct ChannelSettings {
    pub mode: ChannelMode,
    pub send_frequency: Duration,
    pub priority: f32,
}
```

**ChannelMode Variants** (builder.rs:341-385):
- `UnorderedUnreliableWithAcks` - No guarantees, but ACKs tracked
- `UnorderedUnreliable` - No guarantees
- `SequencedUnreliable` - Drop out-of-order, keep newest
- `UnorderedReliable(ReliableSettings)` - Guaranteed delivery, any order
- `SequencedReliable(ReliableSettings)` - Guaranteed, sequenced
- `OrderedReliable(ReliableSettings)` - Guaranteed, ordered

**ReliableSettings** (builder.rs:387-409):
- `rtt_resend_factor: f32` - RTT multiplier for resend (default 1.5)
- `rtt_resend_min_delay: Duration` - minimum resend delay

### System Ordering

**Phase Organization**:

Location: `git/lightyear/lightyear_netcode/src/client_plugin.rs:225-242`

```
PreUpdate:
  LinkSystems::Receive → ConnectionSystems::Receive → TransportSystems::Receive

PostUpdate:
  TransportSystems::Send → ConnectionSystems::Send → LinkSystems::Send
```

Data flows through the layers bidirectionally:
- **Outgoing**: Transport → Connection (encrypt) → Link (IO)
- **Incoming**: Link (IO) → Connection (decrypt) → Transport

## Code References

### Current Project
- `Cargo.toml:1-11` - Workspace configuration
- `crates/server/Cargo.toml:1-10` - Server crate dependencies
- `crates/client/Cargo.toml:1-10` - Client crate dependencies
- `crates/server/src/main.rs:1-11` - Minimal server setup
- `crates/client/src/main.rs:1-13` - Minimal client setup

### Lightyear Source (git/lightyear/)
- `lightyear/src/server.rs:47-84` - Server plugin group with multi-transport
- `lightyear/src/client.rs:13-65` - Client plugin group
- `examples/common/src/server.rs:65-160` - Multi-transport server pattern
- `examples/common/src/client.rs:17-124` - Multi-transport client pattern
- `examples/simple_setup/src/server.rs:41-51` - Minimal UDP server
- `examples/simple_setup/src/client.rs:22-43` - Minimal UDP client
- `lightyear_udp/src/lib.rs:51-193` - UDP transport implementation
- `lightyear_udp/src/server.rs:28-263` - UDP server implementation
- `lightyear_webtransport/src/client.rs:30-162` - WebTransport client
- `lightyear_webtransport/src/server.rs:48-112` - WebTransport server
- `lightyear_websocket/src/client.rs:28-56` - WebSocket client
- `lightyear_websocket/src/server.rs:40-93` - WebSocket server
- `lightyear_transport/src/channel/builder.rs:32-409` - Channel configuration

## Architecture Documentation

### Multi-Crate Setup Pattern

**Recommended Structure**:
```
bevy-lightyear-template/
├── Cargo.toml                 # Workspace definition
├── crates/
│   ├── protocol/              # Shared messages, components, channels
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs         # Public API
│   │       ├── messages.rs    # Message definitions
│   │       ├── components.rs  # Replicated components
│   │       └── channels.rs    # Channel configuration
│   ├── server/                # Authoritative server (native only)
│   │   ├── Cargo.toml         # Features: server, netcode, udp, webtransport, websocket
│   │   └── src/
│   │       ├── main.rs        # Multi-transport server setup
│   │       └── systems.rs     # Server-side gameplay
│   ├── client/                # Native client (desktop)
│   │   ├── Cargo.toml         # Features: client, netcode, udp
│   │   └── src/
│   │       ├── main.rs        # UDP client setup
│   │       └── systems.rs     # Client-side gameplay
│   └── web/                   # WASM client (browser)
│       ├── Cargo.toml         # Features: client, netcode, webtransport, websocket
│       ├── src/
│       │   ├── lib.rs         # Wasm entry point
│       │   └── systems.rs     # Same as client/
│       └── index.html         # HTML wrapper
└── git/                       # Git submodules (bevy, lightyear, avian)
```

**Workspace Cargo.toml**:
```toml
[workspace]
members = ["crates/protocol", "crates/server", "crates/client", "crates/web"]
resolver = "2"

[workspace.dependencies]
bevy = { version = "0.17.2", default-features = false }
lightyear = { version = "0.25.5", default-features = false }
protocol = { path = "crates/protocol" }
```

**Server Cargo.toml**:
```toml
[dependencies]
bevy = { workspace = true, features = ["bevy_winit", "multi_threaded"] }
lightyear = { workspace = true, features = ["server", "netcode", "udp", "webtransport", "websocket"] }
protocol = { workspace = true }
```

**Native Client Cargo.toml**:
```toml
[dependencies]
bevy = { workspace = true, features = ["default"] }
lightyear = { workspace = true, features = ["client", "netcode", "udp"] }
protocol = { workspace = true }
```

**WASM Client Cargo.toml**:
```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
bevy = { workspace = true, features = ["bevy_winit", "webgl2"] }
lightyear = { workspace = true, features = ["client", "netcode", "webtransport", "websocket", "web"] }
protocol = { workspace = true }

[target.'cfg(target_family = "wasm")'.dependencies]
wasm-bindgen = "0.2"
```

### Implementation Patterns

**Server: Spawn Multiple Transports**

```rust
fn startup(mut commands: Commands) -> Result {
    let private_key = Key::default();
    let protocol_id = 0;

    // UDP server
    commands.spawn((
        NetcodeServer::new(NetcodeConfig {
            protocol_id,
            private_key,
            ..Default::default()
        }),
        LocalAddr(SocketAddr::from(([0, 0, 0, 0], 5000))),
        ServerUdpIo::default(),
        Name::from("UDP Server"),
    ));

    // WebTransport server
    let certificate = WebTransportCertificateSettings::AutoSelfSigned(vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
    ]);
    commands.spawn((
        NetcodeServer::new(NetcodeConfig {
            protocol_id,
            private_key,
            ..Default::default()
        }),
        LocalAddr(SocketAddr::from(([0, 0, 0, 0], 5001))),
        WebTransportServerIo {
            certificate: (&certificate).into(),
        },
        Name::from("WebTransport Server"),
    ));

    // WebSocket server
    let sans = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let config = ServerConfig::builder()
        .with_bind_address(SocketAddr::from(([0, 0, 0, 0], 5002)))
        .with_identity(Identity::self_signed(sans).unwrap());
    commands.spawn((
        NetcodeServer::new(NetcodeConfig {
            protocol_id,
            private_key,
            ..Default::default()
        }),
        LocalAddr(SocketAddr::from(([0, 0, 0, 0], 5002))),
        WebSocketServerIo { config },
        Name::from("WebSocket Server"),
    ));

    Ok(())
}
```

**Native Client: UDP**

```rust
fn startup(mut commands: Commands) -> Result {
    let auth = Authentication::Manual {
        server_addr: SocketAddr::from(([127, 0, 0, 1], 5000)),
        client_id: 0,
        private_key: Key::default(),
        protocol_id: 0,
    };

    let client = commands.spawn((
        Client::default(),
        LocalAddr(SocketAddr::from(([0, 0, 0, 0], 0))),  // Random port
        PeerAddr(SocketAddr::from(([127, 0, 0, 1], 5000))),
        Link::new(None),
        ReplicationReceiver::default(),
        NetcodeClient::new(auth, NetcodeConfig::default())?,
        UdpIo::default(),
        Name::from("Client"),
    )).id();

    commands.trigger(Connect { entity: client });
    Ok(())
}
```

**WASM Client: WebTransport with Conditional Compilation**

```rust
fn startup(mut commands: Commands) -> Result {
    let server_port = {
        #[cfg(target_family = "wasm")]
        { 5001 }  // WebTransport port
        #[cfg(not(target_family = "wasm"))]
        { 5001 }  // Can test WebTransport on native too
    };

    let auth = Authentication::Manual {
        server_addr: SocketAddr::from(([127, 0, 0, 1], server_port)),
        client_id: 0,
        private_key: Key::default(),
        protocol_id: 0,
    };

    let certificate_digest = {
        #[cfg(target_family = "wasm")]
        {
            // Embedded at compile time
            include_str!("../../../certificates/digest.txt").to_string()
        }
        #[cfg(not(target_family = "wasm"))]
        {
            // Native can skip validation for testing
            "".to_string()
        }
    };

    let client = commands.spawn((
        Client::default(),
        LocalAddr(SocketAddr::from(([0, 0, 0, 0], 0))),
        PeerAddr(SocketAddr::from(([127, 0, 0, 1], server_port))),
        Link::new(None),
        ReplicationReceiver::default(),
        NetcodeClient::new(auth, NetcodeConfig::default())?,
        WebTransportClientIo { certificate_digest },
        Name::from("WASM Client"),
    )).id();

    commands.trigger(Connect { entity: client });
    Ok(())
}
```

### Key Principles

1. **Shared Protocol**: All message types, replicated components, and channel configs in `crates/protocol`
2. **Multi-Transport Server**: Spawn separate server entities per transport, each with unique port
3. **Platform-Specific Clients**: Use `#[cfg(target_family = "wasm")]` for WASM vs native differences
4. **Feature Gating**: Enable only needed transports per crate to minimize binary size
5. **Certificate Management**: Store digest in file, embed in WASM via `include_str!()`
6. **Authentication**: Share protocol_id and private_key across all transports for same game session

## Related Research

None yet - this is the first research document.

## Decisions

1. **Server transports**: Use different ports per transport
   - UDP: 5000
   - WebTransport: 5001
   - WebSocket: 5002
   - Rationale: WebTransport uses QUIC/UDP, conflicts with UDP on same port

2. **Certificate digest distribution**: Option A - Embed at compile time via `include_str!()`
   - Primary: Embed digest from `certificates/digest.txt` into WASM binary
   - Fallback: Support URL hash (`#<digest>`) and window variable (`window.CERT_DIGEST`)
   - Source: `git/lightyear/examples/common/src/client.rs:85-97`

3. **Code sharing**: Option B - Shared `crates/client_shared` library
   - Shared gameplay, rendering, and input systems
   - Reduces duplication between native and WASM clients
   - Platform-specific code only in main.rs/lib.rs

4. **WASM testing**: Use Bevy CLI for development
   - Tool: `bevy run web` from `crates/web/`
   - Automatic HTTPS server with hot reload
   - Certificate generation: `cargo make ensure-certs`
   - See expanded section below for details

5. **Protocol crate features**: No features to disable components
   - Enable all replicated components in protocol crate
   - Binary size optimization via cargo profile settings instead
   - Simpler build configuration

## Follow-up Research: WASM Client Testing Options

**Updated**: 2025-11-17T07:16:42-08:00

### Local Development Workflows

Based on patterns in `git/lightyear/`, there are three primary approaches:

#### Option A: Bevy CLI (Recommended for Development)

**Tool**: [Bevy CLI](https://github.com/TheBevyFlock/bevy_cli)

**Setup**:
```bash
# Install Bevy CLI
cargo install bevy_cli

# Generate certificates (14-day validity)
cd "$(git rev-parse --show-toplevel)" && cargo make ensure-certs

# Run WASM dev server with HTTPS
cd crates/web
bevy run web
```

**Configuration** (`crates/web/Cargo.toml`):
```toml
[package.metadata.bevy_cli.web]
rustflags = ["--cfg", "getrandom_backend=\"wasm_js\""]
default-features = false
features = ["client", "netcode", "webtransport"]
```

**Benefits**:
- Automatic HTTPS server with self-signed certificates
- Hot reload support
- Integrated with Bevy ecosystem
- Minimal configuration

**Source**: `git/lightyear/examples/README.md:50-60`, `git/lightyear/examples/simple_box/Cargo.toml:40-43`

#### Option B: Manual wasm-bindgen + Static Server

**Tool**: wasm-bindgen CLI + Simple HTTP server

**Setup**:
```bash
# Generate certificates
cargo make ensure-certs

# Build WASM
cd crates/web
cargo build --release --target wasm32-unknown-unknown

# Run wasm-bindgen
wasm-bindgen --no-typescript --target web \
  --out-dir ../../htdocs/web \
  --out-name web \
  ../../target/wasm32-unknown-unknown/release/web.wasm

# Copy index.html
cp index.html ../../htdocs/web/

# Serve with HTTPS (requires cert.pem and key.pem)
cd ../../htdocs
python3 -m http.server 8080 --bind 127.0.0.1
```

**HTML Template** (`crates/web/index.html`):
```html
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>Game Client</title>
    <style>
        body, html { margin: 0; padding: 0; width: 100vw; height: 100%; }
        canvas { display: block; touch-action: none; }
    </style>
</head>
<body>
    <script type="module">
        import init from './web.js'
        init().catch((error) => {
            if (!error.message.startsWith("Using exceptions for control flow")) {
                throw error;
            }
        });
    </script>
</body>
</html>
```

**Benefits**:
- Full control over build process
- No additional tooling dependencies
- Suitable for CI/CD pipelines

**Drawbacks**:
- Manual rebuild required
- More complex setup
- Need separate HTTPS server

**Source**: `git/lightyear/examples/common/www/index.html`

#### Option C: Trunk

**Tool**: [Trunk](https://trunkrs.dev/)

**Setup**:
```bash
# Install trunk
cargo install trunk

# Generate certificates
cargo make ensure-certs

# Run dev server (automatically serves with HTTPS if cert files present)
cd crates/web
trunk serve --release
```

**HTML** (`crates/web/index.html`):
```html
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8"/>
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Game Client</title>
    <link data-trunk rel="rust"/>
</head>
</html>
```

**Benefits**:
- Hot reload support
- Automatic WASM rebuilding
- Integrated asset pipeline
- Popular in Bevy community

**Source**: `git/lightyear/examples/simple_box/index.html`

### Certificate Handling

All options require self-signed certificates for WebTransport:

**Generate Certificates**:
```bash
# From project root
mkdir -p certificates
cd certificates

# Generate EC-based self-signed cert (14-day validity)
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
  -keyout key.pem -out cert.pem -days 14 -nodes -subj "/CN=localhost"

# Extract SHA256 fingerprint for client
FINGERPRINT=$(openssl x509 -in cert.pem -noout -sha256 -fingerprint | \
  sed 's/^.*=//' | sed 's/://g')
echo -n "$FINGERPRINT" > digest.txt

echo "Certificate digest: $FINGERPRINT"
```

**Files Generated**:
- `certificates/cert.pem` - Server certificate
- `certificates/key.pem` - Private key
- `certificates/digest.txt` - SHA256 fingerprint for WASM clients

**Source**: `git/lightyear/certificates/generate.sh`

### Certificate Digest Distribution Methods

**Primary: Compile-time Embedding** (Chosen in Decision 2)
```rust
let certificate_digest = {
    #[cfg(target_family = "wasm")]
    {
        include_str!("../../certificates/digest.txt").to_string()
    }
    #[cfg(not(target_family = "wasm"))]
    {
        "".to_string()
    }
};
```

**Fallback: Runtime via URL Hash**
```rust
// WASM client checks window.location.hash()
// URL: https://localhost:8080#2E6DF3B559DEB469CD238CA56A903310502DE781427D758BBA0E3B2801556704
```

**Fallback: Runtime via Window Variable**
```javascript
// Set in index.html before loading WASM
window.CERT_DIGEST = "2E6DF3B559DEB469CD238CA56A903310502DE781427D758BBA0E3B2801556704";
```

**Source**: `git/lightyear/examples/common/src/server.rs:23-63`

### Required Cargo Features

**WASM Client** (`crates/web/Cargo.toml`):
```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
bevy = { workspace = true, features = ["bevy_winit", "webgl2"] }
lightyear = { workspace = true, features = [
    "client",
    "netcode",
    "webtransport",
    "webtransport_self_signed",
    "websocket",
    "websocket_self_signed",
    "web",
] }
protocol = { workspace = true }

[target.'cfg(target_family = "wasm")'.dependencies]
wasm-bindgen = "0.2"
console_error_panic_hook = "0.1"
getrandom = { version = "0.3", features = ["wasm_js"] }
```

**Build Flags**:
```bash
export RUSTFLAGS="--cfg=web_sys_unstable_apis"
export CARGO_BUILD_TARGET=wasm32-unknown-unknown
cargo build --release
```

**Source**: `git/lightyear/examples/build_wasm.sh`, `git/lightyear/examples/common/Cargo.toml:74-78`

### Testing Checklist

1. ✓ Generate certificates (`sh certificates/generate.sh`)
2. ✓ Verify digest.txt contains fingerprint
3. ✓ Start native server with WebTransport enabled (port 5001)
4. ✓ Build WASM client with embedded digest
5. ✓ Serve WASM client via HTTPS (bevy run web / trunk serve)
6. ✓ Open browser to `https://localhost:8080`
7. ✓ Accept self-signed certificate warning in browser
8. ✓ Verify WebTransport connection in browser console
9. ✓ Test WebSocket fallback if WebTransport fails

### Browser Compatibility

**WebTransport**:
- Chrome/Edge 97+
- Safari 18+ (iOS 18+)
- Firefox: Behind flag (not production-ready)

**WebSocket** (Fallback):
- All modern browsers
- Broader compatibility than WebTransport

### Recommendation

**For this project**: Use **Option A (Bevy CLI)** for development
- Fastest iteration cycle
- Built-in HTTPS support
- Bevy ecosystem integration
- Minimal configuration

**For production deployment**: Use **Option B (Manual)** in CI/CD
- Explicit build control
- No dev dependencies in production
- Standard WASM output format
