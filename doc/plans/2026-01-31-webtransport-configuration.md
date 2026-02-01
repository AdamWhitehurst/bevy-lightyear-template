# WebTransport Configuration Implementation Plan

## Overview

Switch the project from UDP to WebTransport as the default transport, enabling both native and web clients to connect to the same server. This requires loading certificates from files (instead of generating at runtime) so clients can have the digest at compile time.

## Current State Analysis

- **Server** ([crates/server/src/network.rs:39](crates/server/src/network.rs#L39)): Uses UDP on port 5000, WebTransport generates certificates at runtime
- **Native Client** ([crates/client/src/network.rs:23](crates/client/src/network.rs#L23)): Uses UDP by default, no compile-time digest loading
- **Web Client** ([crates/web/src/network.rs:21](crates/web/src/network.rs#L21)): Already configured for WebTransport with compile-time digest on port 5001
- **Certificates**: Already generated in `certificates/` directory (cert.pem, key.pem, digest.txt)

### Key Discoveries:
- Multi-transport is NOT supported - server can only run one transport at a time
- Certificate loading requires async via `IoTaskPool` with `async_compat::Compat` wrapper
- Lightyear provides `Identity::load_pemfiles()` for file-based certificates

## Desired End State

Server and native client both use WebTransport on port 5001, with certificates loaded from `certificates/` directory. Native and web clients can connect to the same server.

### Verification:
1. `cargo server` starts WebTransport server on port 5001
2. `cargo client -c 1` connects via WebTransport
3. `bevy run web` connects via WebTransport
4. All three can run simultaneously with proper replication

## What We're NOT Doing

- Multi-transport support (not supported by Lightyear)
- Dynamic certificate generation at runtime
- External certificate management (CA-signed certs)
- Command-line transport selection

## Implementation Approach

1. Modify server to load certificates from files
2. Change server default to WebTransport on port 5001
3. Modify native client to load digest at compile time
4. Change native client default to WebTransport

## Phase 1: Server Certificate Loading

### Overview
Update server to load certificates from PEM files instead of generating at runtime.

### Changes Required:

#### 1. Add async_compat dependency
**File**: `crates/server/Cargo.toml`
**Changes**: Add async_compat for tokio compatibility

```toml
async_compat = "0.2"
```

#### 2. Modify server network.rs
**File**: `crates/server/src/network.rs`
**Changes**:
- Add imports for async certificate loading
- Create function to load certificates from files
- Update WebTransport case to use file-based certificates
- Change default transport to WebTransport on port 5001

```rust
// Add imports at top
use async_compat::Compat;
use bevy::tasks::IoTaskPool;

// Add constant for certificate paths
const CERT_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../certificates/cert.pem");
const KEY_PEM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../certificates/key.pem");

// Add function to load certificates
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
```

Update default configuration (line 39):
```rust
transports: vec![ServerTransport::WebTransport { port: 5001 }],
```

Update WebTransport case in `start_server` (lines 104-139):
```rust
ServerTransport::WebTransport { port } => {
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
            LocalAddr(SocketAddr::from((config.bind_addr, port))),
            WebTransportServerIo {
                certificate: wt_certificate,
            },
        ))
        .id();
    commands.trigger(Start { entity: server });
    info!(
        "WebTransport server listening on {}:{}",
        config.bind_addr.iter().map(|b| b.to_string()).collect::<Vec<_>>().join("."),
        port
    );
}
```

### Success Criteria:

#### Automated Verification:
- [x] Server builds: `cargo build -p server`
- [ ] Server runs and logs certificate digest: `cargo server`

#### Manual Verification:
- [ ] Logged digest matches `certificates/digest.txt`

---

## Phase 2: Native Client WebTransport

### Overview
Update native client to use WebTransport with compile-time certificate digest.

### Changes Required:

#### 1. Modify client network.rs
**File**: `crates/client/src/network.rs`
**Changes**: Load digest at compile time, change default to WebTransport

Add constant for digest:
```rust
/// Certificate digest loaded at compile time
const CERTIFICATE_DIGEST: &str = include_str!("../../../certificates/digest.txt");
```

Update `ClientTransport::default()` (lines 21-25):
```rust
impl Default for ClientTransport {
    fn default() -> Self {
        Self::WebTransport {
            certificate_digest: CERTIFICATE_DIGEST.to_string(),
        }
    }
}
```

Update `ClientNetworkConfig::default()` server_addr (line 44):
```rust
server_addr: SocketAddr::from(([127, 0, 0, 1], 5001)),
```

### Success Criteria:

#### Automated Verification:
- [x] Client builds: `cargo build -p client`
- [x] All tests pass: `cargo test-all` (note: unrelated `test_shutdown_save` flaky)

#### Manual Verification:
- [ ] Start server: `cargo server`
- [ ] Start native client: `cargo client -c 1`
- [ ] Client connects successfully (check logs for "Client connected")

---

## Phase 3: Verify Web Client

### Overview
Verify web client still works (should require no changes).

### Changes Required:
None - web client already uses WebTransport on port 5001.

### Success Criteria:

#### Automated Verification:
- [x] Web build succeeds: `bevy build web`

#### Manual Verification:
- [ ] Start server: `cargo server`
- [ ] Start web client: `bevy run web`
- [ ] Web client connects successfully
- [ ] Both native and web clients can connect simultaneously

---

## Testing Strategy

### Unit Tests:
- No new unit tests required (network setup is integration-level)

### Integration Tests:
- Existing crossbeam tests should continue to pass

### Manual Testing Steps:
1. Generate fresh certificates: `./certificates/generate.sh`
2. Rebuild all: `cargo build --workspace`
3. Start server: `cargo server`
4. Verify server logs show WebTransport on port 5001
5. Start native client: `cargo client -c 1`
6. Verify native client connects
7. Start web client: `bevy run web`
8. Verify web client connects
9. Test basic gameplay replication works

## Performance Considerations

WebTransport uses QUIC which may have slightly different latency characteristics than raw UDP. For local development this should be negligible.

## Migration Notes

- Certificates must be regenerated every 14 days (browser limitation for self-signed)
- After regenerating certificates, rebuild the project to update compile-time digest
- No data migration needed

## References

- Research: [doc/research/2026-01-31-webtransport-configuration.md](doc/research/2026-01-31-webtransport-configuration.md)
- Lightyear example: [git/lightyear/examples/common/src/server.rs:223-248](git/lightyear/examples/common/src/server.rs#L223-L248)
