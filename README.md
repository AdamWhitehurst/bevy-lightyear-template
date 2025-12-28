# Bevy Lightyear Template

Multi-transport networked game template using Bevy and Lightyear.

**Game Vision**: See [VISION.md](VISION.md) for the game design document.

## Features

- **Server**: Authoritative server supporting UDP, WebTransport, and WebSocket
- **Native Client**: Desktop client connecting via UDP
- **WASM Client**: Browser client connecting via WebTransport/WebSocket
- **Voxel Map System**: Networked voxel terrain with bevy_voxel_world integration

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
bevy run --bin web
```

Opens browser to HTTPS dev server. Client connects via WebTransport on `127.0.0.1:5001`.

**Note**: Accept the self-signed certificate warning in your browser.

## Project Structure

```
bevy-lightyear-template/
├── crates/
│   ├── protocol/       # Shared network protocol and voxel map types
│   ├── server/         # Authoritative server with voxel world
│   ├── client/         # Native client with voxel rendering
│   ├── web/            # WASM client
│   ├── render/         # 3D rendering systems
│   └── ui/             # UI components
├── certificates/       # TLS certificates (generated)
├── scripts/            # Build and run scripts
├── doc/                # Documentation and plans
└── git/                # Git submodules (bevy_voxel_world, lightyear)
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
# From project root:
bevy run --bin web

# Or with auto-open in browser:
bevy run --bin web --open
```