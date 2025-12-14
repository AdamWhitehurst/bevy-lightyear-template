# Fresh NetcodeClient on Connect Implementation Plan

## Overview

Fix the TokenExpired error by creating a fresh `NetcodeClient` with a new token each time the user initiates a connection, rather than reusing the token created at startup.

## Current State Analysis

- [network.rs:93](crates/client/src/network.rs#L93) - `NetcodeClient` created once at `Startup` with `NetcodeConfig::default()` (30-second token expiry)
- [lib.rs:39-48](crates/ui/src/lib.rs#L39-L48) - `on_entering_connecting_state` only triggers `Connect`, doesn't refresh token
- `ClientNetworkConfig` is passed to plugin but not available as a resource

### Key Discoveries:
- Token is generated when `NetcodeClient::new()` is called, not when `Connect` is triggered
- Lightyear's lobby example shows the pattern: insert fresh `NetcodeClient` before triggering `Connect`
- `ClientNetworkConfig` must be a resource for UI crate to access it

## Desired End State

When user clicks "Connect", a fresh `NetcodeClient` with a new token is inserted before triggering connection. This ensures the token is always valid regardless of how long the user waits on the main menu.

### Verification:
1. Start client, wait 60+ seconds on main menu
2. Click Connect
3. Connection succeeds (no TokenExpired error)

## What We're NOT Doing

- Not implementing auth server token fetching (future enhancement)
- Not changing server-side token validation
- Not modifying `token_expire_secs` (keeping default 30s is fine with fresh tokens)

## Implementation Approach

Make `ClientNetworkConfig` a resource, then modify `on_entering_connecting_state` to insert a fresh `NetcodeClient` before triggering `Connect`.

---

## Phase 1: Make ClientNetworkConfig a Resource

### Overview
Insert `ClientNetworkConfig` as a Bevy resource so other systems can access it.

### Changes Required:

#### 1. Add Resource derive and insert as resource
**File**: `crates/client/src/network.rs`

Add `Resource` derive to `ClientNetworkConfig`:
```rust
#[derive(Clone, Resource)]
pub struct ClientNetworkConfig {
```

Insert as resource in plugin build:
```rust
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
```

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Client builds: `cargo build -p client`

---

## Phase 2: Add Re-exports from Client Crate

### Overview
Re-export lightyear types needed by UI crate for creating `NetcodeClient`.

### Changes Required:

#### 1. Add public re-exports
**File**: `crates/client/src/lib.rs`

Add re-exports:
```rust
pub use lightyear::netcode::{Authentication, Key, NetcodeClient, NetcodeConfig};
```

### Success Criteria:

#### Automated Verification:
- [x] Client builds: `cargo build -p client`

---

## Phase 3: Update on_entering_connecting_state

### Overview
Modify the system to create and insert a fresh `NetcodeClient` before triggering `Connect`.

### Changes Required:

#### 1. Update imports and system signature
**File**: `crates/ui/src/lib.rs`

Add imports:
```rust
use client::{Authentication, ClientNetworkConfig, Key, NetcodeClient, NetcodeConfig};
```

Replace `on_entering_connecting_state`:
```rust
fn on_entering_connecting_state(
    mut commands: Commands,
    client_query: Query<Entity, With<Client>>,
    config: Res<ClientNetworkConfig>,
) {
    info!("Entering Connecting state, triggering connection...");
    let client_entity = client_query.single().expect("Client entity should exist");

    // Create fresh authentication with new token
    let auth = Authentication::Manual {
        server_addr: config.server_addr,
        client_id: config.client_id,
        private_key: Key::from(config.private_key),
        protocol_id: config.protocol_id,
    };

    // Insert fresh NetcodeClient (replaces old one, generates new token)
    commands.entity(client_entity).insert(
        NetcodeClient::new(auth, NetcodeConfig::default())
            .expect("Failed to create NetcodeClient"),
    );

    commands.trigger(Connect {
        entity: client_entity,
    });
}
```

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Server builds and runs: `cargo server`
- [x] Client builds and runs: `cargo client -c 1`

#### Manual Verification:
- [ ] Start server, start client, wait 60+ seconds on main menu, click Connect - connection succeeds
- [ ] Disconnect and reconnect multiple times - all connections succeed
- [ ] Cancel during connection, wait, reconnect - works correctly

---

## Testing Strategy

### Unit Tests:
- Existing UI tests should continue to pass (they mock the connection)

### Integration Tests:
- Existing integration tests verify connection flow

### Manual Testing Steps:
1. `cargo server` in one terminal
2. `cargo client -c 1` in another terminal
3. Wait 60 seconds on main menu
4. Click Connect
5. Verify connection succeeds (no TokenExpired warning in logs)
6. Click Main Menu to disconnect
7. Wait 30 seconds
8. Click Connect again
9. Verify second connection succeeds

## References

- Research: `thoughts/shared/research/2025-12-13-netcode-token-expired-fix.md`
- Lightyear lobby example: `git/lightyear/examples/lobby/src/client.rs:62-96`
