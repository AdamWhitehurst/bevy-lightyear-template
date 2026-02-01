---
date: 2026-01-31T10:47:29-08:00
researcher: Claude
git_commit: fedd0a992c43c474428b025a7a4f3d87740532e9
branch: master
repository: bevy-lightyear-template
topic: "WebTransport Configuration for Server, Native, and Web Clients"
tags: [research, codebase, networking, webtransport, certificates, lightyear]
status: complete
last_updated: 2026-01-31
last_updated_by: Claude
last_updated_note: "Clarified that multi-transport is not supported"
---

# Research: WebTransport Configuration for Server, Native, and Web Clients

**Date**: 2026-01-31T10:47:29-08:00
**Researcher**: Claude
**Git Commit**: fedd0a992c43c474428b025a7a4f3d87740532e9
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to switch transport method to WebTransport for server, native, and web. Including properly generating certificates for server and using them in clients.

## Summary

The project already has WebTransport support implemented. Switching to WebTransport requires:
1. **Server**: Change to `ServerTransport::WebTransport { port: 5001 }` (replaces UDP)
2. **Native Client**: Use `ClientTransport::WebTransport { certificate_digest }` with the server's certificate hash
3. **Web Client**: Already configured via `WebClientPlugin` which includes the digest at compile time

**Important constraint**: Multi-transport is not supported. The server can only run one transport at a time. To support both native and web clients, use WebTransport for all (native clients support WebTransport).

The key challenge is certificate management: certificates must use ECDSA P-256, are limited to 14 days validity, and clients must receive the SHA-256 hash to verify the server.

## Detailed Findings

### Server Configuration

**File**: [crates/server/src/network.rs](crates/server/src/network.rs)

The server supports multiple transports via `ServerTransport` enum:

```rust
pub enum ServerTransport {
    Udp { port: u16 },
    WebTransport { port: u16 },
    WebSocket { port: u16 },
    Crossbeam { io: lightyear_crossbeam::CrossbeamIo },
}
```

**Current default** (lines 37-39): Only UDP on port 5000:
```rust
transports: vec![ServerTransport::Udp { port: 5000 }],
```

**To enable WebTransport**, modify to:
```rust
transports: vec![ServerTransport::WebTransport { port: 5001 }],
```

**Note**: Multi-transport does not work. The server can only use one transport at a time. Choose either UDP (native clients) or WebTransport (native + web clients).

**WebTransport server setup** (lines 104-139):
- Generates self-signed certificate with SANs: `localhost`, `127.0.0.1`, `::1`
- Uses `lightyear::webtransport::prelude::Identity::self_signed(sans)`
- Spawns `WebTransportServerIo` with the certificate

**Issue**: The current implementation generates a new certificate on each startup. Clients cannot know the digest in advance. For production, certificates should be loaded from files.

### Certificate Generation

**File**: [certificates/generate.sh](certificates/generate.sh)

Existing script generates proper WebTransport certificates:

```bash
openssl req -x509 \
    -newkey ec \
    -pkeyopt ec_paramgen_curve:prime256v1 \
    -keyout key.pem \
    -out cert.pem \
    -days 14 \
    -nodes \
    -subj "/CN=localhost"
```

**Requirements**:
- ECDSA with P-256 curve (mandatory for WebTransport)
- Max 14 days validity (browser limitation for self-signed)
- SHA-256 fingerprint stored without colons

**Output files**:
- `certificates/cert.pem` - Certificate
- `certificates/key.pem` - Private key
- `certificates/digest.txt` - SHA-256 fingerprint (hex, no colons)

### Native Client Configuration

**File**: [crates/client/src/network.rs](crates/client/src/network.rs)

`ClientTransport` enum (lines 12-19):
```rust
pub enum ClientTransport {
    Udp,
    WebTransport { certificate_digest: String },
    Crossbeam(CrossbeamIo),
}
```

**To use WebTransport in native client**:
```rust
let config = ClientNetworkConfig {
    server_addr: SocketAddr::from(([127, 0, 0, 1], 5001)),  // WebTransport port
    transport: ClientTransport::WebTransport {
        certificate_digest: "ABC123...".to_string(),  // From digest.txt
    },
    ..default()
};
```

**Native client internals** (from Lightyear):
- Uses `wtransport` crate with `IpBindConfig::InAddrAnyV4`
- Certificate digest parsed as hex, converted to `Sha256Digest`
- Empty digest + `dangerous-configuration` feature skips validation (dev only)

### Web Client Configuration

**File**: [crates/web/src/network.rs](crates/web/src/network.rs)

`WebClientPlugin` (lines 17-40):
- Loads certificate digest at compile time: `include_str!("../../../certificates/digest.txt")`
- Hardcoded to connect to `127.0.0.1:5001`
- Uses `ClientTransport::WebTransport { certificate_digest }`

**WASM internals** (from Lightyear):
- Uses `xwt_web` crate for browser WebTransport API
- Certificate hash passed as `CertificateHash { algorithm: Sha256, value: Vec<u8> }`

**Alternative digest sources** (from Lightyear examples):
- URL hash: `example.com/#ABCD1234...`
- JavaScript global: `window.CERT_DIGEST`

### Certificate Loading from Files

Lightyear supports loading certificates from PEM files. From [git/lightyear/examples/common/src/server.rs:172-248](git/lightyear/examples/common/src/server.rs):

```rust
pub enum WebTransportCertificateSettings {
    AutoSelfSigned(Vec<String>),  // Current approach
    FromFile { cert: String, key: String },  // File-based
}

// File loading
Identity::load_pemfiles(cert_pem_path, private_key_pem_path).await
```

**To use pre-generated certificates**, the server would need modification to:
1. Load `certificates/cert.pem` and `certificates/key.pem`
2. Print/store the digest for client distribution

### Feature Flags

**Server** ([crates/server/Cargo.toml](crates/server/Cargo.toml)):
```toml
lightyear = { features = ["server", "netcode", "udp", "webtransport", ...] }
```

**Client** ([crates/client/Cargo.toml](crates/client/Cargo.toml)):
```toml
lightyear = { features = ["client", "netcode", "udp", "webtransport", ...] }
```

**Web** ([crates/web/Cargo.toml](crates/web/Cargo.toml)):
```toml
lightyear = { features = ["client", "netcode", "webtransport", ...] }
```

All required features are already enabled.

## Code References

| File | Line | Description |
|------|------|-------------|
| [crates/server/src/network.rs](crates/server/src/network.rs) | 11-22 | `ServerTransport` enum definition |
| [crates/server/src/network.rs](crates/server/src/network.rs) | 37-39 | Default transport configuration |
| [crates/server/src/network.rs](crates/server/src/network.rs) | 104-139 | WebTransport server setup |
| [crates/client/src/network.rs](crates/client/src/network.rs) | 12-19 | `ClientTransport` enum definition |
| [crates/client/src/network.rs](crates/client/src/network.rs) | 111-113 | WebTransport client setup |
| [crates/web/src/network.rs](crates/web/src/network.rs) | 20-21 | Compile-time digest loading |
| [certificates/generate.sh](certificates/generate.sh) | 12-19 | OpenSSL certificate generation |

## Architecture Documentation

### Transport Layer Architecture

**Constraint**: Only one transport can be active at a time (multi-transport not supported).

```
┌─────────────────────────────────────────────┐
│              SERVER (choose one)            │
│  ┌──────────────┐   OR   ┌───────────────┐ │
│  │ UDP Server   │        │ WebTransport  │ │
│  │ (port 5000)  │        │ Server (5001) │ │
│  └──────────────┘        └───────────────┘ │
│         │                       │          │
│         │          ┌────────────┴────────┐ │
│         │          │ TLS Certificate     │ │
│         │          │ (self-signed/file)  │ │
│         │          └────────────────────-┘ │
└─────────┼───────────────────┼──────────────┘
          │                   │
          ▼                   ▼
    Native Client       Native + Web Client
    (UDP only)          (WebTransport)
                              │
                     ┌────────┴────────┐
                     │ Certificate     │
                     │ Digest (SHA-256)│
                     └─────────────────┘
```

**Implication**: To support both native and web clients, use WebTransport (native clients can also use WebTransport).

### Certificate Flow

1. **Generate** (one-time or every 14 days):
   ```bash
   ./certificates/generate.sh
   ```

2. **Server startup**: Load certificate from files or generate

3. **Client build** (web): `include_str!` embeds digest at compile time

4. **Client runtime** (native): Pass digest to `ClientTransport::WebTransport`

## Web Sources

- [MDN WebTransport Constructor](https://developer.mozilla.org/en-US/docs/Web/API/WebTransport/WebTransport)
- [WTransport GitHub Repository](https://github.com/BiagioFesta/wtransport)
- [WebTransport Certificate Hash (Feisty Duck)](https://www.feistyduck.com/newsletter/issue_85_webtransport_allows_tls_connections_with_certificate_hash)

