---
date: 2026-03-12T20:46:58-07:00
researcher: Claude
git_commit: 3b57143799b8caf0e8a39b49b0d3ed42dabc42c2
branch: master
repository: bevy-lightyear-template
topic: "Streaming RON asset files to web clients"
tags: [research, assets, wasm, web, ron, bevy, lightyear]
status: complete
last_updated: 2026-03-12
last_updated_by: Claude
---

# Research: Streaming RON Asset Files to Web Clients

**Date**: 2026-03-12T20:46:58-07:00
**Researcher**: Claude
**Git Commit**: 3b57143799b8caf0e8a39b49b0d3ed42dabc42c2
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

Is there a way to stream RON asset files to web clients and have clients load them?

## Summary

**Yes.** There are three viable approaches, two of which require minimal changes:

1. **Bevy already fetches assets over HTTP in WASM builds** — `HttpWasmAssetReader` uses browser `fetch()` internally. If RON files are served alongside the WASM bundle, they load with zero extra configuration.
2. **`WebAssetPlugin` (built into Bevy 0.17+)** allows loading assets from arbitrary HTTP URLs via `asset_server.load("https://...")`. Works on both native and WASM.
3. **Lightyear message channels** could theoretically send RON data as serialized bytes, but this is awkward and unnecessary given the HTTP approaches.

The project already has a WASM workaround for `load_folder` (manifest file + individual loads) that works with HTTP fetching. Existing `RonAssetPlugin` loaders work unchanged — only the transport layer differs.

## Detailed Findings

### Current Asset Loading Architecture

Assets live in workspace-root `assets/` and are loaded via `bevy_common_assets` `RonAssetPlugin` (v0.15, `ron` 0.12).

**Native builds** (server/client) override `AssetPlugin.file_path` to point at workspace root:
- [server/src/main.rs:18-21](crates/server/src/main.rs#L18-L21) — `concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets")`
- [client/src/main.rs:33-36](crates/client/src/main.rs#L33-L36) — same pattern

**WASM build** uses Trunk to copy assets into the served directory:
- [web/index.html:24](crates/web/index.html#L24) — `<link data-trunk rel="copy-dir" href="../../assets"/>`
- [web/src/main.rs:18-25](crates/web/src/main.rs#L18-L25) — `DefaultPlugins` with no `AssetPlugin` override (uses default `assets/` path)

**Loading flow**: Handles are created at `Startup`, tracked in `TrackedAssets`, and `AppState` transitions from `Loading` to `Ready` once all are loaded ([protocol/src/app_state.rs](crates/protocol/src/app_state.rs)).

**WASM folder workaround**: Since `load_folder` doesn't work on WASM, abilities use a manifest file:
- [ability.rs:446-485](crates/protocol/src/ability.rs#L446-L485) — loads `abilities.manifest.ron` (a `Vec<String>`), then individually loads each `{id}.ability.ron`

### Approach 1: Default WASM Behavior (Already Works)

Bevy's WASM asset reader (`HttpWasmAssetReader`) already uses browser `fetch()` to load assets over HTTP. When Trunk copies `assets/` to the dist directory, the web server serves them as static files, and the WASM client fetches them on demand.

**This is what the project already does.** No changes needed for this path — RON files are already "streamed" (fetched over HTTP) to the web client.

The key limitation: assets must be co-located with the WASM bundle on the same origin. If you want assets served from a different server (e.g., game server instead of static host), see Approach 2.

### Approach 2: `WebAssetPlugin` (Bevy 0.17+ Built-in)

The `bevy_web_asset` crate was upstreamed into Bevy 0.17. It enables loading assets from arbitrary HTTP(S) URLs:

```rust
use bevy::{asset::io::web::WebAssetPlugin, prelude::*};

App::new()
    .add_plugins(DefaultPlugins.set(WebAssetPlugin {
        silence_startup_warning: true,
    }))
    // ...

// In any system:
let handle = asset_server.load("https://game-server.com/assets/fireball.ability.ron");
```

Requirements:
- Enable `"https"` feature on `bevy` dependency
- CORS headers on the server serving assets (`Access-Control-Allow-Origin`)
- Works on both native (uses `surf` HTTP client) and WASM (uses `web-sys` fetch)

This would let the web client load RON files from the game server's HTTP endpoint rather than requiring them bundled with the WASM build.

### Approach 3: Custom `AssetReader` with Multiple Asset Sources

Bevy supports named asset sources with custom `AssetReader` implementations:

```rust
app.register_asset_source(
    "remote",
    AssetSource::build().with_reader(|| {
        Box::new(HttpAssetReader::new("https://game-server.com/assets"))
    }),
);

// Load with source prefix:
asset_server.load("remote://abilities/fireball.ability.ron");
```

This allows mixing local and remote assets. Most flexible but most work. Only needed if local and remote assets must coexist with different base URLs.

### Approach 4: Lightyear Message Channels (Not Recommended for This)

Lightyear has three channels defined ([protocol/src/lib.rs:162-208](crates/protocol/src/lib.rs#L162-L208)):
- `VoxelChannel` — OrderedReliable
- `ChunkChannel` — UnorderedReliable
- `MapChannel` — OrderedReliable

Messages are automatically fragmented at 1180 bytes and reassembled. `ChunkDataSync` already sends `PalettedChunk` payloads over `ChunkChannel`.

Theoretically, RON file contents could be serialized into a message type and sent over a channel. However, this is awkward because:
- You'd need custom message types for each asset category
- You'd need to manually deserialize into Bevy asset handles
- You'd bypass `AssetServer` benefits (handle tracking, hot-reload, dependency management)
- HTTP is simpler and already works

### `HttpWasmAssetReader` URL Mapping

For advanced control, `HttpWasmAssetReader` supports custom URL mapping:

```rust
HttpWasmAssetReader::new("assets").with_request_mapper(|path| {
    format!("https://cdn.example.com/assets/{path}")
})
```

This could redirect asset requests to a CDN or different server without requiring `WebAssetPlugin`.

## Code References

- [crates/web/index.html:24](crates/web/index.html#L24) — Trunk copy-dir directive for assets
- [crates/web/src/main.rs:18-25](crates/web/src/main.rs#L18-L25) — WASM DefaultPlugins setup
- [crates/protocol/src/ability.rs:406-414](crates/protocol/src/ability.rs#L406-L414) — RonAssetPlugin registrations
- [crates/protocol/src/ability.rs:436-485](crates/protocol/src/ability.rs#L436-L485) — Native vs WASM ability loading
- [crates/protocol/src/app_state.rs](crates/protocol/src/app_state.rs) — TrackedAssets + AppState gate
- [crates/sprite_rig/src/lib.rs:23-27](crates/sprite_rig/src/lib.rs#L23-L27) — Sprite rig RonAssetPlugin registrations
- [crates/protocol/src/lib.rs:162-208](crates/protocol/src/lib.rs#L162-L208) — Lightyear channel definitions

## External References

- [Bevy WebAssetPlugin example](https://bevy.org/examples/assets/web-asset/)
- [Upstream bevy_web_asset — Issue #16307](https://github.com/bevyengine/bevy/issues/16307)
- [Custom AssetReader example](https://github.com/bevyengine/bevy/blob/main/examples/asset/custom_asset_reader.rs)
- [Multiple Asset Sources PR #9885](https://github.com/bevyengine/bevy/pull/9885)
- [HttpWasmAssetReader URL mapping PR #21737](https://github.com/bevyengine/bevy/pull/21737)

## Architecture Documentation

| Concern | Current State |
|---------|--------------|
| WASM asset transport | HTTP fetch via `HttpWasmAssetReader` (built-in) |
| `load_folder` on WASM | Not supported; manifest workaround in place |
| RON deserialization | `bevy_common_assets` `RonAssetPlugin` — works regardless of transport |
| Asset readiness gate | `TrackedAssets` + `AppState::Loading → Ready` |
| Lightyear data streaming | Message-based with auto-fragmentation; used for voxel chunks, not assets |

## Open Questions

- Should the game server serve assets over HTTP alongside its WebSocket/WebTransport endpoint, or should a separate static file server handle it?
- Would dynamic asset loading (loading new RON files after initial startup) be needed, or only at startup?
- Are CORS headers already configured on the game server's HTTP layer?
