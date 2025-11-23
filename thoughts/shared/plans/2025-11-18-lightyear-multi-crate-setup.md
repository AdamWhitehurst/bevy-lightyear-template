# Lightyear Multi-Crate Setup Implementation Plan

## Overview

Implement a complete multi-crate workspace for a Bevy + Lightyear networked game with:
- Dedicated authoritative server supporting UDP, WebTransport, and WebSocket simultaneously
- Native client application connecting via UDP
- WASM client application connecting via WebTransport or WebSocket
- Shared protocol crate for messages, components, and channel definitions

This provides a production-ready foundation for building networked games that work across native desktop and web browsers.

## Current State Analysis

**Workspace Structure**: `/home/aw/Dev/bevy-lightyear-template/Cargo.toml:1-11`
- 2 members: `crates/client`, `crates/server`
- Shared dependencies: bevy 0.17.2, lightyear 0.25.5, avian3d 0.4.1
- No features configured on dependencies

**Server**: `crates/server/src/main.rs:1-11`
- Minimal Bevy app with `MinimalPlugins`
- No Lightyear integration
- No transport configuration

**Client**: `crates/client/src/main.rs:1-13`
- Minimal Bevy app with `DefaultPlugins` and 3D camera
- No Lightyear integration
- No network connection code

**Missing**:
- Protocol/shared crate for network definitions
- WASM client crate (`crates/web`)
- Certificate generation for WebTransport/WebSocket
- Lightyear plugin configuration
- Transport-specific entity spawning
- Build tooling for WASM

## Desired End State

A fully functional multi-transport networked application where:

1. **Server** listens on three ports simultaneously:
   - UDP: `0.0.0.0:5000`
   - WebTransport: `0.0.0.0:5001` (with self-signed certificate)
   - WebSocket: `0.0.0.0:5002` (with self-signed certificate)

2. **Native Client** connects to server via UDP on `127.0.0.1:5000` and exchanges messages

3. **WASM Client** can be built and served, connecting via WebTransport (`127.0.0.1:5001`) or WebSocket (`127.0.0.1:5002`)

4. **Protocol** defines:
   - `Message1` bidirectional message type
   - `Channel1` ordered reliable channel
   - Shared between all crates

### Verification:

**Automated**:
- `cargo check --workspace` - All crates compile
- `cargo build --release -p server` - Server builds
- `cargo build --release -p client` - Native client builds
- `cargo build --release -p web --target wasm32-unknown-unknown` - WASM builds

**Manual**:
- Server starts and listens on all three ports
- Native client connects via UDP and logs connection success
- WASM client loads in browser and connects via WebTransport/WebSocket
- Messages sent from client appear in server logs
- No panics or connection failures

## What We're NOT Doing

- **Gameplay implementation**: No game logic, physics, or entity replication beyond basic connection
- **Authentication**: Using manual authentication with default keys for local development
- **Production security**: Self-signed certificates only, not production TLS
- **Advanced features**: No delta compression, prediction, interpolation, or rollback
- **Multiple client instances**: Single client connection pattern only
- **Steam transport**: Focusing on UDP and web transports only
- **Custom error handling**: Using basic `Result` and `?` operator
- **Persistence**: No save/load or database integration

## Implementation Approach

Follow the patterns from `git/lightyear/examples/simple_setup/` but adapt to multi-crate architecture:

1. **Protocol-first design**: Define shared types in separate crate before implementing client/server
2. **Entity-based networking**: Spawn server/client entities with transport-specific components
3. **Multi-transport via multiple entities**: Server spawns three separate entities, one per transport
4. **Compile-time certificate embedding**: WASM client embeds certificate digest via `include_str!()`
5. **Feature-based configuration**: Enable only required transports per crate to minimize binary size
6. **Bevy CLI for WASM**: Use metadata-driven build configuration for development workflow

## Phase 1: Protocol Crate & Workspace Setup

### Overview
Create the shared protocol crate with minimal "hello world" message/channel definitions and update workspace configuration to include web crate.

### Changes Required:

#### 1. Create Protocol Crate

**Directory**: `crates/protocol/`

**File**: `crates/protocol/Cargo.toml`
```toml
[package]
name = "protocol"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true }
lightyear = { workspace = true }
serde = { version = "1.0", features = ["derive"] }
```

**File**: `crates/protocol/src/lib.rs`
```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

// Message definitions
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Message1(pub usize);

// Channel marker
pub struct Channel1;

// Protocol registration plugin
pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        // Register message
        app.register_message::<Message1>()
            .add_direction(NetworkDirection::Bidirectional);

        // Register channel
        app.add_channel::<Channel1>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..default()
        })
        .add_direction(NetworkDirection::Bidirectional);
    }
}

// Shared constants
pub const PROTOCOL_ID: u64 = 0;
pub const PRIVATE_KEY: [u8; 32] = [0; 32];
pub const FIXED_TIMESTEP_HZ: f64 = 64.0;
```

#### 2. Update Workspace Configuration

**File**: `Cargo.toml`
```toml
[workspace]
members = [
    "crates/protocol",
    "crates/client",
    "crates/server",
    "crates/web",
]
resolver = "2"

[workspace.dependencies]
avian3d = "0.4.1"
bevy = "0.17.2"
lightyear = "0.25.5"
protocol = { path = "crates/protocol" }
serde = "1.0"

# WASM-specific dependencies
wasm-bindgen = "0.2"
console_error_panic_hook = "0.1"
getrandom = { version = "0.3", features = ["wasm_js"] }
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check -p protocol` - Protocol crate compiles
- [x] `cargo build -p protocol` - Protocol crate builds successfully

#### Manual Verification:
- [x] `crates/protocol/` directory exists with Cargo.toml and src/lib.rs
- [x] Workspace resolver accepts all 4 members without errors

---

## Phase 2: Multi-Transport Server

### Overview
Configure the server to spawn three separate server entities (UDP, WebTransport, WebSocket) and set up certificate generation for TLS-based transports.

### Changes Required:

#### 1. Server Dependencies

**File**: `crates/server/Cargo.toml`
```toml
[package]
name = "server"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true }
lightyear = { workspace = true, features = ["server", "netcode", "udp", "webtransport", "websocket"] }
protocol = { workspace = true }
anyhow = "1.0"
```

#### 2. Server Implementation

**File**: `crates/server/src/main.rs`
```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::server::*;
use lightyear_netcode::server::*;
use lightyear_udp::server::*;
use lightyear_webtransport::server::*;
use lightyear_websocket::server::*;
use protocol::*;
use std::net::SocketAddr;
use std::time::Duration;

fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .add_systems(Startup, start_server)
        .add_observer(handle_new_client)
        .run();
}

fn start_server(mut commands: Commands) {
    info!("Starting multi-transport server...");

    let netcode_config = NetcodeConfig {
        protocol_id: PROTOCOL_ID,
        private_key: Key::from(PRIVATE_KEY),
        ..default()
    };

    // UDP Server (port 5000)
    commands.spawn((
        Name::new("UDP Server"),
        NetcodeServer::new(netcode_config.clone()),
        LocalAddr(SocketAddr::from(([0, 0, 0, 0], 5000))),
        ServerUdpIo::default(),
    ));
    info!("UDP server listening on 0.0.0.0:5000");

    // WebTransport Server (port 5001)
    let wt_certificate = WebTransportCertificateSettings::AutoSelfSigned(vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ]);
    commands.spawn((
        Name::new("WebTransport Server"),
        NetcodeServer::new(netcode_config.clone()),
        LocalAddr(SocketAddr::from(([0, 0, 0, 0], 5001))),
        WebTransportServerIo {
            certificate: (&wt_certificate).into(),
        },
    ));
    info!("WebTransport server listening on 0.0.0.0:5001");

    // WebSocket Server (port 5002)
    let ws_config = lightyear_websocket::ServerConfig::builder()
        .with_bind_address(SocketAddr::from(([0, 0, 0, 0], 5002)))
        .with_identity(lightyear_websocket::Identity::self_signed(vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
        ]).expect("Failed to generate WebSocket certificate"));
    commands.spawn((
        Name::new("WebSocket Server"),
        NetcodeServer::new(netcode_config),
        LocalAddr(SocketAddr::from(([0, 0, 0, 0], 5002))),
        WebSocketServerIo { config: ws_config },
    ));
    info!("WebSocket server listening on 0.0.0.0:5002");

    info!("Server started successfully");
}

fn handle_new_client(
    trigger: On<Add, Connected>,
    mut commands: Commands,
) {
    info!("New client connected: {:?}", trigger.entity);
    commands.entity(trigger.entity).insert(ReplicationSender::new(
        Duration::from_millis(100),
        SendUpdatesMode::SinceLastAck,
        false,
    ));
}
```

#### 3. Certificate Generation Script

**File**: `certificates/generate.sh`
```bash
#!/bin/bash

# Generate self-signed certificate for WebTransport/WebSocket
# Valid for 14 days, using EC prime256v1 curve

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cd "$SCRIPT_DIR" || exit 1

echo "Generating self-signed certificate..."

openssl req -x509 \
    -newkey ec \
    -pkeyopt ec_paramgen_curve:prime256v1 \
    -keyout key.pem \
    -out cert.pem \
    -days 14 \
    -nodes \
    -subj "/CN=localhost"

echo "Extracting certificate digest..."

FINGERPRINT=$(openssl x509 -in cert.pem -noout -sha256 -fingerprint | \
    sed 's/^.*=//' | sed 's/://g')

echo -n "$FINGERPRINT" > digest.txt

echo "Certificate generated successfully!"
echo "Digest: $FINGERPRINT"
echo ""
echo "Files created:"
echo "  - cert.pem (certificate)"
echo "  - key.pem (private key)"
echo "  - digest.txt (SHA-256 fingerprint)"
```

**File**: `certificates/.gitignore`
```
*.pem
digest.txt
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check -p server` - Server compiles
- [x] `cargo build --release -p server` - Server builds in release mode

#### Manual Verification:
- [x] Run `sh certificates/generate.sh` - Creates cert.pem, key.pem, digest.txt
- [x] Run `cargo run -p server` - Server starts without panics
- [x] Server logs show all three transports listening
- [x] No errors about missing certificates or bind failures

---

## Phase 3: Native Client (UDP)

### Overview
Implement the native client that connects to the server via UDP on port 5000.

### Changes Required:

#### 1. Client Dependencies

**File**: `crates/client/Cargo.toml`
```toml
[package]
name = "client"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true }
lightyear = { workspace = true, features = ["client", "netcode", "udp"] }
protocol = { workspace = true }
anyhow = "1.0"
```

#### 2. Client Implementation

**File**: `crates/client/src/main.rs`
```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::client::*;
use lightyear_netcode::client::*;
use lightyear_udp::*;
use protocol::*;
use std::net::SocketAddr;
use std::time::Duration;

const CLIENT_ADDR: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0)),
    0, // Random port
);
const SERVER_ADDR: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
    5000, // UDP server port
);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .add_systems(Startup, setup)
        .add_observer(on_connected)
        .add_observer(on_disconnected)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera3d::default());

    info!("Connecting to server at {}...", SERVER_ADDR);

    let auth = Authentication::Manual {
        server_addr: SERVER_ADDR,
        client_id: 0,
        private_key: Key::from(PRIVATE_KEY),
        protocol_id: PROTOCOL_ID,
    };

    let client = commands
        .spawn((
            Name::new("Client"),
            Client::default(),
            LocalAddr(CLIENT_ADDR),
            PeerAddr(SERVER_ADDR),
            Link::new(None),
            ReplicationReceiver::default(),
            NetcodeClient::new(auth, NetcodeConfig::default())
                .expect("Failed to create NetcodeClient"),
            UdpIo::default(),
        ))
        .id();

    commands.trigger(Connect { entity: client });
}

fn on_connected(trigger: On<Add, Connected>) {
    info!("Successfully connected to server! Entity: {:?}", trigger.entity);
}

fn on_disconnected(trigger: On<Add, Disconnected>) {
    info!("Disconnected from server. Entity: {:?}", trigger.entity);
}
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check -p client` - Client compiles
- [x] `cargo build --release -p client` - Client builds in release mode

#### Manual Verification:
- [x] Start server: `cargo run -p server`
- [x] Start client: `cargo run -p client`
- [x] Client logs show "Successfully connected to server!"
- [x] Server logs show "New client connected"
- [x] No connection timeouts or panics

---

## Phase 4: WASM Client (WebTransport/WebSocket)

### Overview
Create the WASM client crate that can connect via WebTransport or WebSocket, with compile-time certificate digest embedding and Bevy CLI support.

### Changes Required:

#### 1. Create WASM Client Crate

**File**: `crates/web/Cargo.toml`
```toml
[package]
name = "web"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
bevy = { workspace = true, features = ["bevy_winit", "webgl2"] }
lightyear = { workspace = true, features = [
    "client",
    "netcode",
    "webtransport",
    "websocket",
    "web",
] }
protocol = { workspace = true }

[target.'cfg(target_family = "wasm")'.dependencies]
wasm-bindgen = { workspace = true }
console_error_panic_hook = { workspace = true }
getrandom = { workspace = true }

[package.metadata.bevy_cli.web]
rustflags = ["--cfg", "getrandom_backend=\"wasm_js\""]
default-features = false
features = ["client", "netcode", "webtransport"]
```

#### 2. WASM Client Implementation

**File**: `crates/web/src/lib.rs`
```rust
use bevy::prelude::*;
use lightyear::prelude::*;
use lightyear::client::*;
use lightyear_netcode::client::*;
use protocol::*;
use std::net::SocketAddr;
use std::time::Duration;

#[cfg(target_family = "wasm")]
use lightyear_webtransport::client::*;

#[cfg(target_family = "wasm")]
use wasm_bindgen::prelude::*;

const SERVER_ADDR: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
    5001, // WebTransport server port
);

const CLIENT_ADDR: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::new(0, 0, 0, 0)),
    0,
);

#[cfg(target_family = "wasm")]
#[wasm_bindgen(start)]
pub fn start() {
    // Set panic hook for better error messages in browser console
    console_error_panic_hook::set_once();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Lightyear WASM Client".to_string(),
                canvas: Some("#bevy".to_string()),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .add_systems(Startup, setup)
        .add_observer(on_connected)
        .add_observer(on_disconnected)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera3d::default());

    info!("WASM Client: Connecting to server at {}...", SERVER_ADDR);

    let auth = Authentication::Manual {
        server_addr: SERVER_ADDR,
        client_id: 0,
        private_key: Key::from(PRIVATE_KEY),
        protocol_id: PROTOCOL_ID,
    };

    // Load certificate digest at compile time
    let certificate_digest = {
        #[cfg(target_family = "wasm")]
        {
            include_str!("../../../certificates/digest.txt").to_string()
        }
        #[cfg(not(target_family = "wasm"))]
        {
            String::new()
        }
    };

    info!("Using certificate digest: {}", certificate_digest);

    let client = commands
        .spawn((
            Name::new("WASM Client"),
            Client::default(),
            LocalAddr(CLIENT_ADDR),
            PeerAddr(SERVER_ADDR),
            Link::new(None),
            ReplicationReceiver::default(),
            NetcodeClient::new(auth, NetcodeConfig::default())
                .expect("Failed to create NetcodeClient"),
            #[cfg(target_family = "wasm")]
            WebTransportClientIo { certificate_digest },
        ))
        .id();

    commands.trigger(Connect { entity: client });
}

fn on_connected(trigger: On<Add, Connected>) {
    info!("WASM Client: Successfully connected to server! Entity: {:?}", trigger.entity);
}

fn on_disconnected(trigger: On<Add, Disconnected>) {
    info!("WASM Client: Disconnected from server. Entity: {:?}", trigger.entity);
}
```

#### 3. HTML Wrapper

**File**: `crates/web/index.html`
```html
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8"/>
    <meta name="viewport" content="width=device-width, initial-scale=1, user-scalable=no">
    <title>Lightyear WASM Client</title>
    <style>
        body, html {
            margin: 0;
            padding: 0;
            width: 100vw;
            height: 100vh;
            background-color: #1a1a1a;
        }
        canvas {
            display: block;
            touch-action: none;
        }
        canvas:focus {
            outline: none;
        }
    </style>
    <link data-trunk rel="rust"/>
</head>
<body>
    <canvas id="bevy"></canvas>
</body>
</html>
```

#### 4. Cargo Config for WASM

**File**: `.cargo/config.toml`
```toml
[target.wasm32-unknown-unknown]
rustflags = ["--cfg", "web_sys_unstable_apis"]
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check -p web --target wasm32-unknown-unknown` - WASM client compiles
- [x] `cargo build --release -p web --target wasm32-unknown-unknown` - WASM builds

#### Manual Verification:
- [ ] Install Bevy CLI: `cargo install bevy_cli`
- [ ] Generate certificates: `sh certificates/generate.sh`
- [ ] Start server: `cargo run -p server`
- [ ] From `crates/web/`, run: `bevy run web`
- [ ] Browser opens to local HTTPS server
- [ ] Accept self-signed certificate warning
- [ ] Browser console shows "WASM Client: Successfully connected to server!"
- [ ] Server logs show WebTransport client connection
- [ ] No WASM panics or connection errors

---

## Phase 5: Build Tooling & Cargo Aliases

### Overview
Add convenience aliases for running each component and document the full development workflow.

### Changes Required:

#### 1. Root Cargo Config

**File**: `.cargo/config.toml`
```toml
[alias]
server = "run -p server"
client = "run -p client"
web-build = "build --release -p web --target wasm32-unknown-unknown"

# Check all crates
check-all = "check --workspace"

# Build all native targets
build-all = "build --workspace --exclude web"
```

#### 2. Development Scripts

**File**: `scripts/run_server.sh`
```bash
#!/bin/bash
set -e

echo "Starting Lightyear server..."
cargo run -p server "$@"
```

**File**: `scripts/run_client.sh`
```bash
#!/bin/bash
set -e

echo "Starting native client..."
cargo run -p client "$@"
```

**File**: `scripts/run_web.sh`
```bash
#!/bin/bash
set -e

echo "Generating certificates if needed..."
if [ ! -f "certificates/digest.txt" ]; then
    sh certificates/generate.sh
fi

echo "Starting WASM client with Bevy CLI..."
cd crates/web
bevy run web "$@"
```

**File**: `scripts/setup.sh`
```bash
#!/bin/bash
set -e

echo "Setting up Lightyear development environment..."

# Install WASM target
echo "Installing wasm32-unknown-unknown target..."
rustup target add wasm32-unknown-unknown

# Install Bevy CLI
echo "Installing Bevy CLI..."
cargo install bevy_cli

# Generate certificates
echo "Generating certificates..."
sh certificates/generate.sh

echo "Setup complete!"
echo ""
echo "To run:"
echo "  Server:  cargo server  or  sh scripts/run_server.sh"
echo "  Client:  cargo client  or  sh scripts/run_client.sh"
echo "  Web:     sh scripts/run_web.sh"
```

#### 3. README Documentation

**File**: `README.md`
```markdown
# Bevy Lightyear Template

Multi-transport networked game template using Bevy and Lightyear.

## Features

- **Server**: Authoritative server supporting UDP, WebTransport, and WebSocket
- **Native Client**: Desktop client connecting via UDP
- **WASM Client**: Browser client connecting via WebTransport/WebSocket

## Quick Start

### 1. Setup

```bash
sh scripts/setup.sh
```

This installs dependencies and generates certificates.

### 2. Run Server

```bash
cargo server
```

Server listens on:
- UDP: `0.0.0.0:5000`
- WebTransport: `0.0.0.0:5001`
- WebSocket: `0.0.0.0:5002`

### 3. Run Native Client

```bash
cargo client
```

Connects to server via UDP on `127.0.0.1:5000`.

### 4. Run WASM Client

```bash
sh scripts/run_web.sh
```

Opens browser to HTTPS dev server. Client connects via WebTransport on `127.0.0.1:5001`.

**Note**: Accept the self-signed certificate warning in your browser.

## Project Structure

```
bevy-lightyear-template/
├── crates/
│   ├── protocol/       # Shared network protocol
│   ├── server/         # Authoritative server
│   ├── client/         # Native client
│   └── web/            # WASM client
├── certificates/       # TLS certificates (generated)
├── scripts/            # Build and run scripts
└── git/                # Git submodules (dependencies)
```

## Development

### Cargo Aliases

- `cargo server` - Run server
- `cargo client` - Run native client
- `cargo check-all` - Check all crates
- `cargo build-all` - Build all native targets
- `cargo web-build` - Build WASM client

### Certificate Regeneration

Certificates expire after 14 days. Regenerate with:

```bash
sh certificates/generate.sh
```

### WASM Development

Bevy CLI provides hot reload for WASM development:

```bash
cd crates/web
bevy run web
```

## Protocol

The shared protocol crate defines:
- `Message1` - Bidirectional message type
- `Channel1` - Ordered reliable channel
- Shared constants (protocol ID, keys, tick rate)

Extend `crates/protocol/src/lib.rs` to add game-specific messages and components.

## License

MIT OR Apache-2.0
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all` - All crates compile
- [x] `cargo build-all` - All native targets build
- [x] `cargo web-build` - WASM target builds

#### Manual Verification:
- [ ] Run `sh scripts/setup.sh` - Completes without errors
- [ ] Run `sh scripts/run_server.sh` - Server starts
- [ ] Run `sh scripts/run_client.sh` - Client connects to server
- [ ] Run `sh scripts/run_web.sh` - Browser opens and WASM client connects
- [x] README instructions are accurate and complete
- [x] All cargo aliases work as documented

#### Files Created:
- [x] `.cargo/config.toml` with cargo aliases
- [x] `scripts/run_server.sh`
- [x] `scripts/run_client.sh`
- [x] `scripts/run_web.sh`
- [x] `scripts/setup.sh`
- [x] `README.md`
- [ ] All .sh scripts made executable (chmod +x)

---

## Testing Strategy

### Automated Testing

Currently minimal automated testing. Future improvements:
- Unit tests for protocol serialization
- Integration tests for client-server message exchange
- CI pipeline for cross-platform builds

### Manual Testing Workflow

1. **Server Multi-Transport Test**:
   - Start server: `cargo server`
   - Verify logs show all three transports listening
   - No bind errors or panics

2. **UDP Connection Test**:
   - Server running
   - Start native client: `cargo client`
   - Verify client logs show connection success
   - Verify server logs show new client
   - No timeout errors

3. **WebTransport Connection Test**:
   - Server running
   - Generate fresh certificates: `sh certificates/generate.sh`
   - Start WASM client: `sh scripts/run_web.sh`
   - Accept certificate warning in browser
   - Verify browser console shows connection success
   - Verify server logs show WebTransport client
   - No WASM panics

4. **WebSocket Fallback Test**:
   - Modify `crates/web/src/lib.rs` to use WebSocketClientIo instead of WebTransportClientIo
   - Change SERVER_ADDR port to 5002
   - Run WASM client
   - Verify WebSocket connection succeeds

5. **Multi-Client Test**:
   - Server running
   - Start native client
   - Start WASM client in browser
   - Verify both clients connected simultaneously
   - Server should show two separate client connections

### Edge Cases to Test

- Certificate expiration (after 14 days)
- Connection loss and reconnection
- Server restart with clients connected
- Invalid certificate digest in WASM client
- Port already in use
- Firewall blocking ports

## Performance Considerations

### Binary Size

**WASM Optimization**:
- Release profile uses `lto = true` and `opt-level = 3`
- Consider enabling `wasm-opt` post-processing for further reduction
- Current baseline: ~10-15 MB WASM binary (unoptimized)

**Native Optimization**:
- Server can use `opt-level = "z"` for smaller binary if size matters
- Client benefits from default release settings for best runtime performance

### Network Performance

**Tick Rate**: 64 Hz (15.625ms intervals)
- Suitable for real-time games
- Adjust `FIXED_TIMESTEP_HZ` in protocol crate if needed

**Replication Interval**: 100ms
- Balance between responsiveness and bandwidth
- Adjust `SERVER_REPLICATION_INTERVAL` for different use cases

**Transport Selection**:
- UDP: Lowest latency, best for native clients
- WebTransport: QUIC-based, good for web clients, lower latency than WebSocket
- WebSocket: Highest compatibility, fallback for browsers without WebTransport

### Scalability

Current implementation supports:
- Multiple simultaneous transports
- Multiple clients per transport
- Single-server architecture (no horizontal scaling)

For production scaling, consider:
- Load balancing across multiple server instances
- Database for persistent state
- Matchmaking service

## Migration Notes

### From Existing Projects

If migrating from a single-crate Lightyear example:

1. **Extract protocol code**:
   - Move message/component definitions to `crates/protocol/src/lib.rs`
   - Move channel configuration to protocol plugin
   - Export constants like protocol ID

2. **Split client/server**:
   - Move server startup to `crates/server/src/main.rs`
   - Move client startup to `crates/client/src/main.rs`
   - Remove feature gates - use separate crates instead

3. **Add WASM support**:
   - Create `crates/web` with lib target
   - Configure Bevy CLI metadata
   - Add certificate digest embedding

4. **Update dependencies**:
   - Remove old Lightyear features from workspace dependencies
   - Add features to individual crate dependencies
   - Add protocol crate dependency

### Breaking Changes

This implementation uses:
- Lightyear 0.25.5 (latest API)
- Bevy 0.17.2 (latest ECS patterns)
- Entity-based observers instead of systems for connection events

If using older Lightyear/Bevy versions, adapt:
- Observer syntax → system-based event handling
- Component registration API changes
- Transport configuration differences

## References

- Original research: `thoughts/shared/research/2025-11-16-lightyear-multi-crate-setup.md`
- Lightyear simple_setup example: `git/lightyear/examples/simple_setup/`
- Lightyear protocol patterns: `git/lightyear/examples/*/src/protocol.rs`
- Certificate generation: `git/lightyear/certificates/generate.sh`
- WASM build patterns: `git/lightyear/examples/*/Cargo.toml` (bevy_cli.web metadata)
