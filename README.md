# Bevy Lightyear Template

Multi-transport networked game template using Bevy and Lightyear.

**Game Vision**: See [VISION.md](VISION.md) for the game design document.

## Features

- **Server**: Authoritative server supporting UDP, WebTransport, and WebSocket
- **Native Client**: Desktop client connecting via UDP
- **WASM Client**: Browser client connecting via WebTransport/WebSocket
- **Voxel Map System**: Networked voxel terrain (voxel_map_engine, in progress)
- **Ability System**: Data-driven abilities loaded from RON assets with networked replication

## Quick Start

### 1. Setup

```bash
sh scripts/setup.sh
```

This installs dependencies and generates certificates.

### 2. Configure Nostr identities

Nostr `nsec1...` values are plaintext private keys. Do not commit them, paste them into logs, or share them. The server
and client use them differently:

#### Server identity

The server signs its relay announcement with a durable Nostr key. Provide it with either:

- `SERVER_NSEC`, which takes precedence, or
- `keys/server.nsec`, used as the local development fallback.

Both sources may contain either a raw `nsec1...` secret or an encrypted NIP-49 `ncryptsec1...` value. If you use
`ncryptsec1...`, also set `SERVER_NSEC_PASSPHRASE`.

```bash
# One-shot env var
SERVER_NSEC='nsec1...' cargo server

# Or local file fallback
mkdir -p keys
printf '%s\n' 'nsec1...' > keys/server.nsec
chmod 600 keys/server.nsec
cargo server

# Encrypted server key
SERVER_NSEC='ncryptsec1...' SERVER_NSEC_PASSPHRASE='...' cargo server
```

#### Client identity

The native client starts on the Nostr Login screen:

- **Generate** creates a new Nostr key, encrypts it with the passphrase you enter, and stores only encrypted identity
  data in `worlds/identity.bin`.
- **Import** accepts an existing `nsec1...`, encrypts it with the passphrase you enter, and stores only encrypted
  identity data in `worlds/identity.bin`.
- On later launches, enter the same passphrase on **Unlock** to reuse the same public key and durable identity.
- To reset the native client identity, stop the client and delete `worlds/identity.bin`.

The web client can Generate or Import for the current browser session, but it does not write `worlds/identity.bin`.

Relay discovery uses the default public relay list unless `NOSTR_RELAYS` is set to a comma-separated list of `wss://...`
relay URLs:

```bash
NOSTR_RELAYS='wss://relay.damus.io,wss://nos.lol' cargo client
```

### 3. Run Server

```bash
cargo server
```

Server listens on:

- UDP: `0.0.0.0:5000`
- WebTransport: `0.0.0.0:5001`
- WebSocket: `0.0.0.0:5002`

### 4. Run Native Client

```bash
cargo client
```

Connects to server via UDP on `127.0.0.1:5000`.

### 5. Run WASM Client

```bash
bevy run --bin web
```

Opens browser to HTTPS dev server. Client connects via WebTransport on `127.0.0.1:5001`.

**Note**: Accept the self-signed certificate warning in your browser.

## Project Structure

```
bevy-lightyear-template/
├── crates/
│   ├── protocol/       # Shared network protocol, voxel map, and ability types
│   ├── server/         # Authoritative server with voxel world
│   ├── client/         # Native client with voxel rendering
│   ├── web/            # WASM client
│   ├── render/         # 3D rendering systems
│   ├── sprite_rig/     # 2D sprite rig animation system
│   ├── ui/             # UI components
│   └── dev/            # Development tooling (physics debug, runtime toggles)
├── assets/             # Game assets (ability definitions, etc.)
├── certificates/       # TLS certificates (generated)
├── scripts/            # Build and run scripts
├── doc/                # Documentation and plans
├── crates/voxel_map_engine/ # Custom voxel engine (replacing bevy_voxel_world)
└── git/                # Git submodules (lightyear, etc.)
```

## Development

### Cargo Aliases

- `cargo server` - Run server
- `cargo client` - Run native client
- `cargo check-all` - Check all crates
- `cargo build-all` - Build all native targets
- `cargo web-build` - Build WASM client

### Dev Inspector

Press `F4` to toggle the dev inspector root menu. With the spawn panel enabled, press `F6` or use the root menu to open
it. Def-driven world-object placement is server-authoritative: select an object, arm placement, preview the terrain
target, then click terrain in-game. The same panel can select existing replicated world objects by arming cursor pick
and clicking in-game or by nearby list, then request authoritative delete, move, or yaw rotation edits that persist
across chunk reloads. Free-form spawning remains client-local. The Terrain tab provides activatable brush sculpting
controls, rectangular width/height settings, mode-applied left-click brush strokes, discrete/continuous stroke modes
with continuous frame-rate throttling, initial-hit-face stroke locking, UI-click suppression, a voxel footprint preview,
and server-authoritative Fill Air/Remove/Paint Existing/Replace All brush strokes while in Terrain editing mode.

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

## Ability System

Abilities are defined in `assets/abilities.ron` and loaded at startup. Each character has 4 ability slots mapped to keys
1-4.

### Hotkeys

- `1` - Ability slot 1
- `2` - Ability slot 2
- `3` - Ability slot 3
- `4` - Ability slot 4
- `F3` - Toggle physics debug wireframes

### Defining Abilities

Edit `assets/abilities.ron` to add or modify abilities. Each ability has:

- Phase durations (startup, active, recovery) in ticks (64 ticks = 1 second)
- Cooldown in ticks
- Effects list with triggers: `OnTick` (fires once on a specified Active-phase tick offset, defaults to tick 0),
  `WhileActive` (fires every tick), `OnHit` (fires when a hitbox/projectile hits a target), `OnEnd` (fires on Active
  exit), or `OnInput` (fires on input during Active for combo chaining)
- Effect types: `Melee`, `Projectile`, `AreaOfEffect`, `SetVelocity`, `Damage`, `ApplyForce`, `Ability` (spawns
  sub-ability), `Teleport`, `Shield`, or `Buff`
