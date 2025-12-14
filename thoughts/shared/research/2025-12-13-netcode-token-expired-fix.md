---
date: 2025-12-13T11:23:14-08:00
researcher: claude
git_commit: 014ae7ca9fea21de89d77c54b87a8aaa4ee2c6ed
branch: master
repository: bevy-lightyear-template
topic: "Fixing TokenExpired error when clients connect after 30 seconds"
tags: [research, netcode, lightyear, connection, token]
status: complete
last_updated: 2025-12-13
last_updated_by: claude
last_updated_note: "Added follow-up research for on-connect authentication pattern"
---

# Research: Fixing TokenExpired Error

**Date**: 2025-12-13T11:23:14-08:00
**Researcher**: claude
**Git Commit**: 014ae7ca9fea21de89d77c54b87a8aaa4ee2c6ed
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question
How to fix this error when clients try to connect after waiting 30 seconds:
```
2025-12-13T19:17:46.116598Z  WARN lightyear_netcode::error: Netcode error: Packet(TokenExpired)
```

## Summary

The error occurs because the client's `ConnectToken` expires 30 seconds after creation (the default `token_expire_secs` value). When using `Authentication::Manual`, the token is generated when `NetcodeClient::new()` is called. If connection to the server doesn't complete within 30 seconds, the token expires.

**Fix**: Set `token_expire_secs` in client's `NetcodeConfig`:

```rust
NetcodeClient::new(auth, NetcodeConfig {
    token_expire_secs: -1, // never expires (or use a larger positive value)
    ..default()
})
```

## Detailed Findings

### Current Client Implementation

[network.rs:77-93](crates/client/src/network.rs#L77-L93):
```rust
let auth = Authentication::Manual {
    server_addr: config.server_addr,
    client_id: config.client_id,
    private_key: Key::from(config.private_key),
    protocol_id: config.protocol_id,
};
// ...
NetcodeClient::new(auth, NetcodeConfig::default())
```

The client uses `NetcodeConfig::default()` which sets `token_expire_secs: 30`.

### Token Generation Flow

When `NetcodeClient::new()` is called with `Authentication::Manual`:
1. Lightyear internally generates a `ConnectToken` using `ConnectTokenBuilder`
2. The token's expiration is set via `.expire_seconds(token_expire_secs)`
3. Token validity window = `token_expire_secs` from creation time

### Server Configuration

Server configs at [network.rs:83-87](crates/server/src/network.rs#L83-L87) use `..default()` for remaining fields, so server-side token validation uses the 30-second default as well.

### NetcodeConfig Options

| `token_expire_secs` | Behavior |
|---------------------|----------|
| `30` (default) | Token expires 30 seconds after creation |
| Positive value | Token expires after that many seconds |
| `-1` or negative | Token never expires |

## Code References

- `crates/client/src/network.rs:93` - Client uses `NetcodeConfig::default()`
- `crates/server/src/network.rs:83-87` - Server NetcodeConfig creation
- `git/lightyear/lightyear_netcode/src/client_plugin.rs:72` - Default `token_expire_secs: 30`
- `git/lightyear/lightyear_netcode/src/auth.rs:68-72` - Token generation with expiry

## External References

- [Lightyear NetcodeConfig docs](https://docs.rs/lightyear_netcode/latest/lightyear_netcode/)
- [ConnectTokenBuilder API](https://docs.rs/lightyear_netcode/latest/lightyear_netcode/struct.ConnectTokenBuilder.html)
- [Netcode Protocol Standard](https://github.com/mas-bandwidth/netcode/blob/main/STANDARD.md)

---

## Follow-up Research: On-Connect Authentication Pattern

**Timestamp**: 2025-12-13T11:38:02-08:00

### Research Question

Instead of using `token_expire_secs: -1`, how can authentication happen upon connect so a new NetcodeConfig is created each time?

### Summary

Replace the `NetcodeClient` component with a fresh one before triggering `Connect`. This creates a new token at connection time rather than at startup.

### Current Problem

The current implementation ([network.rs:75-112](crates/client/src/network.rs#L75-L112)) creates `NetcodeClient` once at startup:

```rust
fn setup_client(/* ... */) {
    let auth = Authentication::Manual { /* ... */ };
    commands.spawn((
        // ...
        NetcodeClient::new(auth, NetcodeConfig::default()).unwrap(),
    ));
}
```

When the user clicks "Connect" in the UI, `Connect { entity }` is triggered on the existing entity—but the `NetcodeClient` still has the old token created at startup.

### Solution Pattern

From Lightyear's lobby example ([git/lightyear/examples/lobby/src/client.rs:62-96](git/lightyear/examples/lobby/src/client.rs#L62-L96)):

```rust
fn on_disconnect(
    trigger: On<Add, Disconnected>,
    local_id: Single<&LocalId>,
    mut commands: Commands,
) -> Result {
    // Create fresh authentication
    let auth = Authentication::Manual {
        server_addr: host_addr,
        client_id: local_id.0.to_bits(),
        private_key: SHARED_SETTINGS.private_key,
        protocol_id: SHARED_SETTINGS.protocol_id,
    };
    let netcode_config = NetcodeConfig {
        client_timeout_secs: 3,
        token_expire_secs: -1,  // Or use default 30 with fresh token
        ..default()
    };

    // Replace NetcodeClient component with fresh one
    commands
        .entity(trigger.entity)
        .insert(NetcodeClient::new(auth, netcode_config)?);
    Ok(())
}
```

### Implementation Approach

Modify `on_entering_connecting_state` in [lib.rs:39-47](crates/ui/src/lib.rs#L39-L47) to insert fresh `NetcodeClient` before triggering `Connect`:

```rust
fn on_entering_connecting_state(
    client: Single<Entity, With<Client>>,
    config: Res<ClientNetworkConfig>,
    mut commands: Commands,
) -> Result<(), Box<dyn std::error::Error>> {
    let entity = client.into_inner();

    // Create fresh authentication with new token
    let auth = Authentication::Manual {
        server_addr: config.server_addr,
        client_id: config.client_id,
        private_key: Key::from(config.private_key),
        protocol_id: config.protocol_id,
    };

    // Insert fresh NetcodeClient (replaces old one, generates new token)
    commands.entity(entity).insert(
        NetcodeClient::new(auth, NetcodeConfig::default())?
    );

    // Now trigger connect with fresh token
    commands.trigger(Connect { entity });
    Ok(())
}
```

### Key Insight

From Lightyear documentation: "Whenever the client or server is disconnected, you can update the Client or Server's NetConfig and the changes will take effect at the next connection attempt."

The pattern is:
1. Insert new `NetcodeClient` component (replaces existing, generates fresh token)
2. Optionally update `PeerAddr` if server address changed
3. Trigger `Connect { entity }`

### Alternative: Auth Server Pattern

For production, use `Authentication::Token` with tokens fetched from an auth backend ([git/lightyear/examples/auth/src/client.rs:60-77](git/lightyear/examples/auth/src/client.rs#L60-L77)):

```rust
fn fetch_connect_token(
    mut connect_token_request: ResMut<ConnectTokenRequestTask>,
    client: Single<Entity, With<Client>>,
    mut commands: Commands,
) -> Result {
    if let Some(task) = &mut connect_token_request.task {
        if let Some(connect_token) = block_on(future::poll_once(task)) {
            let client = client.into_inner();
            commands.entity(client).insert(NetcodeClient::new(
                Authentication::Token(connect_token),
                NetcodeConfig::default(),
            )?);
            commands.trigger(Connect { entity: client });
        }
    }
    Ok(())
}
```

### Code References

- [crates/ui/src/lib.rs:39-47](crates/ui/src/lib.rs#L39-L47) - Current `on_entering_connecting_state` (triggers Connect only)
- [crates/client/src/network.rs:75-112](crates/client/src/network.rs#L75-L112) - Startup client setup
- [git/lightyear/examples/lobby/src/client.rs:62-96](git/lightyear/examples/lobby/src/client.rs#L62-L96) - Lobby example reconnect pattern
- [git/lightyear/examples/auth/src/client.rs:60-77](git/lightyear/examples/auth/src/client.rs#L60-L77) - Auth server token pattern

### External References

- [Lightyear Lobby Example](https://github.com/cBournhonesque/lightyear/blob/main/examples/lobby/README.md)
- [Lightyear Book - Build Client Server](https://cbournhonesque.github.io/lightyear/book/tutorial/build_client_server.html)
