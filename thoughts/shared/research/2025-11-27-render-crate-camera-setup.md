---
date: 2025-11-27 07:29:01 PST
researcher: Adam Whitehurst
git_commit: dc4781109b36594c4eee284c42e265d0662fa2ec
branch: master
repository: bevy-lightyear-template
topic: "How to set up a render crate with RenderPlugin for 3D camera setup"
tags: [research, codebase, render, camera, bevy, plugin, 3d]
status: complete
last_updated: 2025-11-27
last_updated_by: Adam Whitehurst
---

# Research: How to set up a `render` crate with `RenderPlugin` for 3D camera setup

**Date**: 2025-11-27 07:29:01 PST
**Researcher**: Adam Whitehurst
**Git Commit**: dc4781109b36594c4eee284c42e265d0662fa2ec
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to set up a `render` crate with `RenderPlugin` used by web and native clients that sets up a 3D camera, spawned offset from origin but looking at origin. It will not be used by the headless server.

## Summary

The current project structure separates client (with rendering), web (WASM with rendering), and server (headless) into distinct crates. Camera setup currently exists in [crates/client/src/network.rs:77](crates/client/src/network.rs#L77) using a minimal `Camera3d::default()` pattern. To create a separate render crate with proper 3D camera positioning:

1. **Create a new `crates/render/` crate** with a `RenderPlugin`
2. **Move camera setup logic** from `ClientNetworkPlugin` into `RenderPlugin`
3. **Use positioned camera pattern**: `(Camera3d::default(), Transform::from_xyz(x, y, z).looking_at(Vec3::ZERO, Vec3::Y))`
4. **Add render crate dependency** to client and web crates only
5. **Exclude from server** by not adding the dependency or plugin

The project already has clear separation between rendering (client/web with `DefaultPlugins`) and headless (server with `MinimalPlugins`), so adding a render crate follows the existing architectural pattern.

## Detailed Findings

### Current Project Structure

The project uses a workspace with four main crates:

**Protocol Crate** ([crates/protocol/](crates/protocol/))
- Shared message and channel definitions
- [crates/protocol/src/lib.rs:13-28](crates/protocol/src/lib.rs#L13-L28) - `ProtocolPlugin` implementation
- Used by all client and server binaries

**Client Crate** ([crates/client/](crates/client/))
- Native desktop client with full rendering
- [crates/client/src/main.rs:11](crates/client/src/main.rs#L11) - Uses `DefaultPlugins` for rendering
- [crates/client/src/network.rs:64-72](crates/client/src/network.rs#L64-L72) - `ClientNetworkPlugin` implementation
- [crates/client/src/network.rs:77](crates/client/src/network.rs#L77) - **Current camera spawn location**: `commands.spawn(Camera3d::default());`

**Server Crate** ([crates/server/](crates/server/))
- Headless server (no rendering)
- [crates/server/src/main.rs:10](crates/server/src/main.rs#L10) - Uses `MinimalPlugins` (no graphics)
- [crates/server/src/network.rs:63-72](crates/server/src/network.rs#L63-L72) - `ServerNetworkPlugin` implementation
- No camera or rendering code

**Web Crate** ([crates/web/](crates/web/))
- WASM browser client
- [crates/web/src/main.rs:14-20](crates/web/src/main.rs#L14-L20) - Uses `DefaultPlugins` with WindowPlugin customization
- [crates/web/src/network.rs:17-38](crates/web/src/network.rs#L17-L38) - `WebClientPlugin` wraps `ClientNetworkPlugin`
- Inherits camera spawning from client crate via dependency

### Current Camera Setup Pattern

**Minimal Camera** (currently used):
```rust
// crates/client/src/network.rs:77
commands.spawn(Camera3d::default());
```

**Positioned Camera with Lookat** (recommended pattern from Bevy examples):
```rust
// From git/bevy/examples/window/screenshot.rs:75-78
commands.spawn((
    Camera3d::default(),
    Transform::from_xyz(-2.0, 2.5, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
));
```

**Key Components:**
- `Camera3d::default()` - 3D camera component with default settings
- `Transform::from_xyz(x, y, z)` - Position the camera offset from origin
- `.looking_at(Vec3::ZERO, Vec3::Y)` - Point camera at origin (Vec3::ZERO) with Y-up orientation

### Bevy Camera Implementation Details

**Auto-Required Components** (from `git/bevy/crates/bevy_camera/src/components.rs`):
- `Camera3d` automatically includes `Camera` and `Projection` components
- `PerspectiveProjection` defaults: FOV 50°, aspect ratio 1.0, near 0.1, far 1000.0
- `Transform` must be added explicitly for custom positioning

**Transform Methods:**
- `Transform::from_xyz(x, y, z)` - Set position
- `.looking_at(target: Vec3, up: Vec3)` - Orient camera toward target
- `.looking_to(direction: Vec3, up: Vec3)` - Orient camera in direction

**Bevy Coordinate System:**
- X-axis: Right
- Y-axis: Up
- Z-axis: Back (forward is -Z)

### Conditional Compilation Strategy

The project uses clear separation without complex conditional compilation:

**Client/Web (Rendering Enabled):**
- [crates/client/Cargo.toml](crates/client/Cargo.toml) - Includes full Bevy features
- Uses `DefaultPlugins` which includes rendering, windowing, input

**Server (Headless):**
- [crates/server/Cargo.toml](crates/server/Cargo.toml) - Minimal dependencies
- Uses `MinimalPlugins` - no rendering subsystems
- Never includes rendering-related plugins

**Web-Specific Compilation:**
- [crates/web/src/network.rs:20](crates/web/src/network.rs#L20) - `#[cfg(target_family = "wasm")]` for WASM-specific code
- [crates/web/Cargo.toml:45-50](crates/web/Cargo.toml#L45-L50) - WASM target-specific dependencies

## Architecture Documentation

### Proposed Render Crate Structure

**Directory Structure:**
```
crates/
├── render/
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs (RenderPlugin implementation)
├── client/
│   └── src/
│       └── main.rs (adds RenderPlugin)
├── web/
│   └── src/
│       └── main.rs (adds RenderPlugin)
└── server/
    └── src/
        └── main.rs (does NOT add RenderPlugin)
```

**Cargo.toml for render crate:**
```toml
[package]
name = "render"
version = "0.1.0"
edition = "2021"

[dependencies]
bevy = { workspace = true }
```

**RenderPlugin implementation** (`crates/render/src/lib.rs`):
```rust
use bevy::prelude::*;

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

**Client usage** (`crates/client/src/main.rs`):
```rust
use render::RenderPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(ClientPlugins)
        .add_plugins(ProtocolPlugin)
        .add_plugins(ClientNetworkPlugin::default())
        .add_plugins(RenderPlugin)  // Add render plugin
        .run();
}
```

**Web usage** (`crates/web/src/main.rs`):
```rust
use render::RenderPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin { ... }))
        .add_plugins(ClientPlugins)
        .add_plugins(ProtocolPlugin)
        .add_plugins(WebClientPlugin)
        .add_plugins(RenderPlugin)  // Add render plugin
        .run();
}
```

**Server (NO render plugin)** (`crates/server/src/main.rs`):
```rust
// No render dependency or plugin added
fn main() {
    App::new()
        .add_plugins(MinimalPlugins)
        .add_plugins(LogPlugin::default())
        .add_plugins(ServerPlugins)
        .add_plugins(ProtocolPlugin)
        .add_plugins(ServerNetworkPlugin::default())
        .run();  // No RenderPlugin
}
```

### Plugin Dependency Pattern

**Add render to workspace** (`Cargo.toml` root):
```toml
[workspace]
members = [
    "crates/protocol",
    "crates/client",
    "crates/server",
    "crates/web",
    "crates/render",  # Add new member
]
```

**Client crate dependency** (`crates/client/Cargo.toml`):
```toml
[dependencies]
protocol = { path = "../protocol" }
render = { path = "../render" }  # Add render dependency
bevy = { workspace = true }
lightyear = { workspace = true, features = ["client", "udp", "webtransport", "crossbeam"] }
```

**Web crate dependency** (`crates/web/Cargo.toml`):
```toml
[dependencies]
protocol = { path = "../protocol" }
render = { path = "../render" }  # Add render dependency
client = { path = "../client" }
bevy = { workspace = true }
```

**Server crate** (NO render dependency in `crates/server/Cargo.toml`):
```toml
[dependencies]
protocol = { path = "../protocol" }
# NO render dependency
bevy = { workspace = true }
lightyear = { workspace = true, features = ["server", "udp", "webtransport", "websocket"] }
```

### Camera Positioning Examples

**Close camera (good for debugging):**
```rust
Transform::from_xyz(0.0, 2.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y)
// Position: 5 units back, 2 units up
```

**Angled camera (cinematic view):**
```rust
Transform::from_xyz(-5.0, 3.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y)
// Position: offset to left, elevated, and back
```

**Top-down camera:**
```rust
Transform::from_xyz(0.0, 10.0, 0.1).looking_at(Vec3::ZERO, Vec3::Y)
// Position: directly above, slightly forward to avoid gimbal lock
```

**Side view camera:**
```rust
Transform::from_xyz(10.0, 2.0, 0.0).looking_at(Vec3::ZERO, Vec3::Y)
// Position: to the right side, elevated
```

## Code References

Current camera implementation:
- [crates/client/src/network.rs:77](crates/client/src/network.rs#L77) - Current minimal camera spawn
- [crates/client/src/network.rs:75-115](crates/client/src/network.rs#L75-L115) - `setup_client` system

Plugin structure examples:
- [crates/protocol/src/lib.rs:13-28](crates/protocol/src/lib.rs#L13-L28) - `ProtocolPlugin` implementation
- [crates/client/src/network.rs:64-72](crates/client/src/network.rs#L64-L72) - `ClientNetworkPlugin` implementation
- [crates/server/src/network.rs:63-72](crates/server/src/network.rs#L63-L72) - `ServerNetworkPlugin` implementation
- [crates/web/src/network.rs:17-38](crates/web/src/network.rs#L17-L38) - `WebClientPlugin` wrapper implementation

Bevy camera source code:
- `git/bevy/crates/bevy_camera/src/components.rs` - Camera component definitions
- `git/bevy/crates/bevy_transform/src/components/transform.rs` - Transform implementation
- `git/bevy/examples/window/screenshot.rs:75-78` - Positioned camera example
- `git/bevy/examples/3d/3d_scene.rs` - Basic 3D scene with camera
- `git/bevy/examples/camera/camera_orbit.rs` - Orbiting camera controller

## Related Research

None yet - this is the first research document on render architecture.

## Open Questions

1. Should the render crate also handle lighting setup, or should that be separate?
2. Should camera position be configurable via resource/config, or hardcoded in plugin?
3. Do we need multiple camera views (debug camera, game camera, etc.)?
4. Should the render crate handle post-processing effects or just basic camera setup?
5. Do we need different camera positions for web vs native, or same for both?
