# Render Crate with 3D Camera Setup Implementation Plan

## Overview

Create a dedicated `render` crate containing a `RenderPlugin` that handles 3D camera setup with proper positioning for client and web binaries. The camera will be positioned offset from origin and configured to look at the origin point. This plugin will NOT be used by the headless server.

## Current State Analysis

**Camera Location:**
- Currently spawned in [crates/client/src/network.rs:77](crates/client/src/network.rs#L77) within `ClientNetworkPlugin`
- Implementation: `commands.spawn(Camera3d::default());`
- No positioning or lookat behavior configured
- Camera spawns at origin (0, 0, 0) with default orientation

**Architecture:**
- 4 workspace members: `protocol`, `client`, `server`, `web`
- Client and web use `DefaultPlugins` (includes rendering)
- Server uses `MinimalPlugins` (headless, no rendering)
- Web delegates to `ClientNetworkPlugin`, inheriting camera setup

**Plugin Patterns:**
- Standard pattern: `struct` → `impl Plugin` → `fn build`
- Systems added via `app.add_systems(Startup, system_fn)`
- Example: [crates/protocol/src/lib.rs:13-28](crates/protocol/src/lib.rs#L13-L28) - `ProtocolPlugin`
- Example: [crates/client/src/network.rs:64-72](crates/client/src/network.rs#L64-L72) - `ClientNetworkPlugin`

### Key Discoveries:
- Camera setup is currently mixed with networking logic in `ClientNetworkPlugin`
- Both client and web would benefit from shared render setup
- Server explicitly avoids rendering dependencies (no bevy rendering features)
- Workspace dependencies pattern: `{ workspace = true }` for shared deps

## Desired End State

**New Structure:**
```
crates/
├── render/           # NEW: Rendering setup
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── client/
├── server/           # Does NOT depend on render
├── web/
└── protocol/
```

**Camera Behavior:**
- Positioned at `(-5.0, 3.0, 8.0)` - offset from origin
- Looking at `Vec3::ZERO` (origin)
- Up vector: `Vec3::Y` (Y-axis up)
- Uses `Transform::from_xyz().looking_at()` pattern

**Plugin Integration:**
- Client: adds `RenderPlugin` after `ProtocolPlugin`
- Web: adds `RenderPlugin` after `ProtocolPlugin`
- Server: does NOT add `RenderPlugin`, no dependency

### Verification:
- [ ] All three binaries build successfully
- [ ] Camera renders scene from angled, elevated position
- [ ] Server builds without render crate dependency
- [ ] No camera spawn in `ClientNetworkPlugin`

## What We're NOT Doing

- Not adding lighting setup (camera only)
- Not making camera position configurable via resource/config
- Not implementing multiple camera views (debug, game, etc.)
- Not adding post-processing effects
- Not creating different camera positions for web vs native
- Not adding camera controllers or movement systems

## Implementation Approach

**Strategy:**
Follow existing plugin patterns in the codebase. Create minimal render crate with single responsibility: camera setup. Use workspace dependency pattern for Bevy. Integrate via standard plugin addition in client/web main.rs files.

**Key Decisions:**
- Camera position: `(-5.0, 3.0, 8.0)` provides good angled view for debugging
- Lookat target: `Vec3::ZERO` keeps focus on origin where entities typically spawn
- Plugin ordering: After `ProtocolPlugin`, before or after `ClientNetworkPlugin` (doesn't matter, no dependencies)

---

## Phase 1: Create Render Crate Structure

### Overview
Set up the new render crate with proper workspace configuration and basic plugin implementation.

### Changes Required:

#### 1. Workspace Configuration
**File**: `Cargo.toml` (root)
**Changes**: Add render to workspace members

```toml
[workspace]
members = [
    "crates/protocol",
    "crates/client",
    "crates/server",
    "crates/web",
    "crates/render",  # Add this line
]
```

**Line to modify**: After line 6, add `"crates/render",`

#### 2. Render Crate Manifest
**File**: `crates/render/Cargo.toml` (NEW)
**Changes**: Create complete Cargo.toml

```toml
[package]
name = "render"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true, default-features = true }

[lints]
workspace = true
```

#### 3. Render Plugin Implementation
**File**: `crates/render/src/lib.rs` (NEW)
**Changes**: Create RenderPlugin with camera setup

```rust
use bevy::prelude::*;

/// Plugin that sets up rendering components (camera, etc.)
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_camera);
    }
}

fn setup_camera(mut commands: Commands) {
    // Spawn 3D camera offset from origin, looking at origin
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(-5.0, 3.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
```

### Success Criteria:

#### Automated Verification:
- [ ] Render crate builds: `cargo build -p render`
- [ ] No compilation errors
- [ ] Workspace recognizes new member: `cargo metadata | grep render`

#### Manual Verification:
- [ ] File structure created correctly
- [ ] Cargo.toml follows workspace patterns

---

## Phase 2: Integrate RenderPlugin into Client

### Overview
Add render dependency to client crate and integrate RenderPlugin into the client's plugin chain.

### Changes Required:

#### 1. Client Dependencies
**File**: `crates/client/Cargo.toml`
**Changes**: Add render dependency

```toml
[dependencies]
bevy = { workspace = true, default-features = true }
lightyear = { workspace = true, features = ["client", "netcode", "udp", "crossbeam", "webtransport"] }
protocol = { workspace = true }
render = { path = "../render" }  # Add this line
```

**Line to add**: After line 9 (after protocol dependency)

#### 2. Client Main - Import
**File**: `crates/client/src/main.rs`
**Changes**: Add use statement for RenderPlugin

```rust
pub mod network;

use bevy::prelude::*;
use lightyear::prelude::client::*;
use network::ClientNetworkPlugin;
use protocol::*;
use render::RenderPlugin;  // Add this line
use std::time::Duration;
```

**Line to add**: After line 6 (after protocol import)

#### 3. Client Main - Plugin Addition
**File**: `crates/client/src/main.rs`
**Changes**: Add RenderPlugin to plugin chain

```rust
fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .add_plugins(ClientNetworkPlugin::default())
        .add_plugins(RenderPlugin)  // Add this line
        .run();
}
```

**Line to add**: After line 16 (after ClientNetworkPlugin)

### Success Criteria:

#### Automated Verification:
- [ ] Client builds: `cargo build -p client`
- [ ] Client runs without panicking: `cargo run -p client` (let it start, then Ctrl+C)

#### Manual Verification:
- [ ] Client window opens with camera view
- [ ] No errors in console about missing camera

---

## Phase 3: Integrate RenderPlugin into Web

### Overview
Add render dependency to web crate and integrate RenderPlugin into the web client's plugin chain.

### Changes Required:

#### 1. Web Dependencies
**File**: `crates/web/Cargo.toml`
**Changes**: Add render dependency

```toml
[dependencies]
bevy = { workspace = true, default-features = false, features = [
    # ... existing features ...
] }
lightyear = { workspace = true, features = ["client", "netcode", "webtransport", "websocket"] }
protocol = { workspace = true }
client = { path = "../client" }
render = { path = "../render" }  # Add this line
```

**Line to add**: After line 43 (after client dependency)

#### 2. Web Main - Import
**File**: `crates/web/src/main.rs`
**Changes**: Add use statement for RenderPlugin

```rust
use bevy::prelude::*;
use lightyear::prelude::client::*;
use protocol::*;
use render::RenderPlugin;  // Add this line
use std::time::Duration;

pub mod network;
use network::WebClientPlugin;
```

**Line to add**: After line 3 (after protocol import)

#### 3. Web Main - Plugin Addition
**File**: `crates/web/src/main.rs`
**Changes**: Add RenderPlugin to plugin chain

```rust
fn main() {
    #[cfg(target_family = "wasm")]
    console_error_panic_hook::set_once();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Lightyear WASM Client".to_string(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(ClientPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        })
        .add_plugins(ProtocolPlugin)
        .add_plugins(WebClientPlugin::default())
        .add_plugins(RenderPlugin)  // Add this line
        .run();
}
```

**Line to add**: After line 25 (after WebClientPlugin)

### Success Criteria:

#### Automated Verification:
- [ ] Web builds: `cargo build -p web`
- [ ] WASM builds: `cargo build -p web --target wasm32-unknown-unknown`

#### Manual Verification:
- [ ] Web client runs in browser
- [ ] Camera view is visible
- [ ] No console errors about duplicate cameras

---

## Phase 4: Remove Camera from ClientNetworkPlugin

### Overview
Clean up the old camera spawn code from ClientNetworkPlugin, which is now handled by RenderPlugin.

### Changes Required:

#### 1. Client Network Setup
**File**: `crates/client/src/network.rs`
**Changes**: Remove camera spawn line

**Old code (lines 75-78):**
```rust
fn setup_client(mut commands: Commands, config: ClientNetworkConfig) {
    // Spawn camera
    commands.spawn(Camera3d::default());

    // Create authentication
```

**New code:**
```rust
fn setup_client(mut commands: Commands, config: ClientNetworkConfig) {
    // Create authentication
```

**Action**: Delete lines 76-77 (comment and spawn command)

### Success Criteria:

#### Automated Verification:
- [ ] All tests pass: `cargo test`
- [ ] Client builds: `cargo build -p client`
- [ ] Web builds: `cargo build -p web`
- [ ] Server builds: `cargo build -p server`

#### Manual Verification:
- [ ] Client launches with single camera (from RenderPlugin)
- [ ] Web launches with single camera (from RenderPlugin)
- [ ] Server builds and runs without camera or render dependency
- [ ] No duplicate camera warnings in logs

---

## Phase 5: Final Testing & Verification

### Overview
Comprehensive testing of all binaries to ensure correct camera setup and no regressions.

### Testing Steps:

#### Automated Tests:
1. Run full test suite: `cargo test`
2. Build all crates: `cargo build --workspace`
3. Check server doesn't depend on render: `cargo tree -p server | grep render` (should be empty)

#### Manual Testing - Client:
1. Run client: `cargo run -p client`
2. Verify window opens
3. Verify camera is positioned correctly (viewing from offset angle)
4. Check console for no camera-related errors
5. Verify scene is visible and in frame

#### Manual Testing - Web:
1. Build and run web: `bevy run web` or equivalent
2. Open browser to web client
3. Verify camera is positioned correctly
4. Check browser console for no errors
5. Verify consistent behavior with native client

#### Manual Testing - Server:
1. Run server: `cargo run -p server`
2. Verify server starts without errors
3. Verify no rendering-related logs or warnings
4. Confirm server remains headless (no window)

### Success Criteria:

#### Automated Verification:
- [ ] All tests pass: `cargo test`
- [ ] All crates build: `cargo build --workspace`
- [ ] Server has no render dependency: `cargo tree -p server | grep render` returns empty
- [ ] Client runs: `cargo run -p client`
- [ ] Server runs: `cargo run -p server`

#### Manual Verification:
- [ ] Client displays 3D scene from angled camera position
- [ ] Web client displays identical camera view
- [ ] Camera is positioned at (-5.0, 3.0, 8.0) looking at origin
- [ ] Server runs headless with no rendering artifacts
- [ ] No duplicate camera warnings
- [ ] Performance is unchanged

---

## Testing Strategy

### Unit Tests:
No unit tests required for this change (camera setup is integration-level functionality).

### Integration Tests:
Existing integration tests should continue to pass. Camera setup doesn't affect networking or protocol logic.

### Manual Testing Steps:
1. **Visual Verification**: Run client and verify camera angle is elevated and offset
2. **Consistency Check**: Ensure web and native clients have identical camera views
3. **Server Isolation**: Verify server has no render dependencies in `cargo tree`
4. **Performance**: Ensure no performance degradation from camera changes
5. **Edge Case**: Test with no entities in scene to ensure camera setup is independent

## Performance Considerations

**Minimal Impact:**
- Single camera spawn during Startup schedule (same as before)
- No additional systems or queries
- No runtime overhead beyond existing camera
- Render crate has minimal dependencies (bevy only)

**Memory:**
- Negligible increase (one additional crate in workspace)
- Camera component count unchanged (still 1 camera)

## Migration Notes

**No Data Migration Required:**
- This is a code reorganization, not a data change
- No existing save data or persistent state affected
- Camera setup happens fresh on each client launch

**Backwards Compatibility:**
- Not applicable (no released version yet)
- Internal refactor only

**Deployment:**
- No special deployment steps
- Standard rebuild of all client binaries
- Server binary size should be unchanged (no new dependencies)

## References

- Original research: [thoughts/shared/research/2025-11-27-render-crate-camera-setup.md](thoughts/shared/research/2025-11-27-render-crate-camera-setup.md)
- Current camera implementation: [crates/client/src/network.rs:77](crates/client/src/network.rs#L77)
- Plugin pattern example: [crates/protocol/src/lib.rs:13-28](crates/protocol/src/lib.rs#L13-L28)
- Client plugin example: [crates/client/src/network.rs:64-72](crates/client/src/network.rs#L64-L72)
- Bevy Transform documentation: `git/bevy/crates/bevy_transform/src/components/transform.rs`
- Bevy Camera3d component: `git/bevy/crates/bevy_camera/src/components.rs`
