# Voxel Map Plugins Implementation Plan

## Overview

Replace the static 100x1x100 floor cuboid with a voxel-based terrain system using `bevy_voxel_world`. The server owns authoritative voxel state and broadcasts changes. Admin clients can edit terrain via mouse clicks. Physics colliders are attached to voxel chunks.

## Current State Analysis

**Existing floor:**
- Server spawns static cuboid at [crates/server/src/gameplay.rs:46-54](crates/server/src/gameplay.rs#L46-L54)
- `FloorPhysicsBundle` at [crates/protocol/src/lib.rs:61-74](crates/protocol/src/lib.rs#L61-L74)
- Client adds physics via `handle_new_floor` at [crates/client/src/gameplay.rs:16-26](crates/client/src/gameplay.rs#L16-L26)
- Replicated via `FloorMarker` component

**Key discoveries:**
- Lightyear uses component registration + `Replicate::to_clients()` for entity sync
- No custom message channels exist yet; can add via `add_channel`/`add_message`
- Physics uses Avian3D 0.4.1 with `PhysicsPlugins::default()`

## Desired End State

1. Flat voxel terrain (solid below y=0) replaces static floor
2. Server maintains authoritative voxel state via `VoxelWorld<MapWorld>`
3. Clients generate identical base terrain deterministically
4. Server sends modification layer to connecting clients
5. Admin clients can left-click (place) / right-click (remove) voxels
6. Voxel chunks have physics colliders for character interaction
7. All existing character physics and replication continues working

### Verification

- Characters walk on voxel terrain with collision
- Admin can place/remove voxels, changes appear on all clients
- New clients connecting receive current modification state
- Performance: 60 FPS with 10 chunks loaded

## What We're NOT Doing

- Home-base voxel maps (separate instance type)
- Persistence to disk (server restart resets modifications)
- Multiple material types (single solid color)
- Chunk streaming based on player position (all modifications sent on connect)
- Non-admin player editing

---

## Phase 1: Add bevy_voxel_world and Core Types

### Overview
Add the voxel crate dependency and define shared types in protocol crate.

### Changes Required:

#### 1. Workspace Cargo.toml
**File**: `Cargo.toml`
**Changes**: Add bevy_voxel_world to workspace dependencies

```toml
# In [workspace.dependencies]
bevy_voxel_world = "0.13"
```

#### 2. Protocol Cargo.toml
**File**: `crates/protocol/Cargo.toml`
**Changes**: Add dependency

```toml
bevy_voxel_world.workspace = true
```

#### 3. MapWorld Config
**File**: `crates/protocol/src/map.rs` (new file)
**Changes**: Create shared voxel world configuration

```rust
use bevy::prelude::*;
use bevy_voxel_world::prelude::*;
use serde::{Deserialize, Serialize};

/// Shared voxel world configuration for server and client
#[derive(Resource, Clone, Default)]
pub struct MapWorld;

impl VoxelWorldConfig for MapWorld {
    type MaterialIndex = u8;
    type ChunkUserBundle = ();

    fn spawning_distance(&self) -> u32 {
        10
    }

    fn voxel_lookup_delegate(&self) -> VoxelLookupDelegate<Self> {
        Box::new(|pos: IVec3| {
            // Flat terrain: solid below y=0
            if pos.y < 0 {
                WorldVoxel::Solid(0)
            } else {
                WorldVoxel::Air
            }
        })
    }
}

/// Voxel type for network serialization
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum VoxelType {
    Air,
    Solid(u8),
}

impl From<VoxelType> for WorldVoxel {
    fn from(v: VoxelType) -> Self {
        match v {
            VoxelType::Air => WorldVoxel::Air,
            VoxelType::Solid(m) => WorldVoxel::Solid(m),
        }
    }
}

impl From<WorldVoxel> for VoxelType {
    fn from(v: WorldVoxel) -> Self {
        match v {
            WorldVoxel::Air => VoxelType::Air,
            WorldVoxel::Solid(m) => VoxelType::Solid(m),
            WorldVoxel::Unset => VoxelType::Air,
        }
    }
}
```

#### 4. Protocol lib.rs
**File**: `crates/protocol/src/lib.rs`
**Changes**: Export map module, remove floor constants

```rust
pub mod map;
pub use map::{MapWorld, VoxelType};

// Remove or deprecate:
// pub const FLOOR_WIDTH: f32 = 100.0;
// pub const FLOOR_HEIGHT: f32 = 1.0;
// pub struct FloorMarker;
// pub struct FloorPhysicsBundle;
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check -p protocol` passes
- [ ] `cargo test-all` passes (existing tests)

#### Manual Verification:
- [ ] N/A (no runtime behavior yet)

---

## Phase 2: Networking Messages

### Overview
Define lightyear messages for voxel editing and initial state sync.

### Changes Required:

#### 1. Message Types
**File**: `crates/protocol/src/map.rs`
**Changes**: Add network messages

```rust
use lightyear::prelude::*;

/// Client requests a voxel edit (admin only)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VoxelEditRequest {
    pub position: IVec3,
    pub voxel: VoxelType,
}

/// Server broadcasts voxel edit to all clients
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VoxelEditBroadcast {
    pub position: IVec3,
    pub voxel: VoxelType,
}

/// Server sends all modifications to connecting client
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VoxelStateSync {
    pub modifications: Vec<(IVec3, VoxelType)>,
}

/// Channel for voxel-related messages
#[derive(Channel)]
pub struct VoxelChannel;
```

#### 2. Protocol Registration
**File**: `crates/protocol/src/lib.rs`
**Changes**: Register messages and channel in ProtocolPlugin

```rust
use crate::map::{VoxelChannel, VoxelEditRequest, VoxelEditBroadcast, VoxelStateSync};

// In ProtocolPlugin::build()
app.add_channel::<VoxelChannel>(ChannelSettings {
    mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
    ..default()
});

app.add_message::<VoxelEditRequest>(MessageSettings {
    channel: ChannelKind::of::<VoxelChannel>(),
    ..default()
});

app.add_message::<VoxelEditBroadcast>(MessageSettings {
    channel: ChannelKind::of::<VoxelChannel>(),
    ..default()
});

app.add_message::<VoxelStateSync>(MessageSettings {
    channel: ChannelKind::of::<VoxelChannel>(),
    ..default()
});
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check -p protocol` passes
- [ ] `cargo test-all` passes

#### Manual Verification:
- [ ] N/A (no runtime behavior yet)

---

## Phase 3: Server MapPlugin

### Overview
Create server-side voxel world plugin that handles edit requests and broadcasts changes.

### Changes Required:

#### 1. Server Map Module
**File**: `crates/server/src/map.rs` (new file)
**Changes**: Create ServerMapPlugin

```rust
use bevy::prelude::*;
use bevy_voxel_world::prelude::*;
use lightyear::prelude::server::*;
use protocol::map::*;

pub struct ServerMapPlugin;

impl Plugin for ServerMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelWorldPlugin::<MapWorld>::headless())
            .init_resource::<VoxelModifications>()
            .add_systems(FixedUpdate, (
                handle_voxel_edit_requests,
                send_state_to_new_clients,
            ));
    }
}

/// Tracks all voxel modifications for sync
#[derive(Resource, Default)]
pub struct VoxelModifications {
    pub edits: Vec<(IVec3, VoxelType)>,
}

fn handle_voxel_edit_requests(
    mut events: EventReader<MessageReceive<VoxelEditRequest>>,
    mut voxel_world: VoxelWorld<MapWorld>,
    mut modifications: ResMut<VoxelModifications>,
    mut broadcast: EventWriter<MessageSend<VoxelEditBroadcast>>,
) {
    for MessageReceive { message, context } in events.read() {
        // TODO: Check if client is admin
        let pos = message.position;
        let voxel = message.voxel;

        voxel_world.set_voxel(pos, voxel.into());
        modifications.edits.push((pos, voxel));

        broadcast.send(MessageSend {
            message: VoxelEditBroadcast { position: pos, voxel },
            context: SendContext::new(NetworkTarget::All),
        });
    }
}

fn send_state_to_new_clients(
    mut events: EventReader<ConnectEvent>,
    modifications: Res<VoxelModifications>,
    mut send: EventWriter<MessageSend<VoxelStateSync>>,
) {
    for event in events.read() {
        send.send(MessageSend {
            message: VoxelStateSync {
                modifications: modifications.edits.clone(),
            },
            context: SendContext::new(NetworkTarget::Single(event.client)),
        });
    }
}
```

#### 2. Server lib.rs
**File**: `crates/server/src/lib.rs`
**Changes**: Add map module

```rust
pub mod map;
```

#### 3. Server Cargo.toml
**File**: `crates/server/Cargo.toml`
**Changes**: Add bevy_voxel_world dependency

```toml
bevy_voxel_world.workspace = true
```

#### 4. Server Gameplay
**File**: `crates/server/src/gameplay.rs`
**Changes**: Remove floor spawning, add ServerMapPlugin

```rust
use crate::map::ServerMapPlugin;

impl Plugin for ServerGameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ServerMapPlugin)
            .add_observer(on_client_connect);
        // Remove: .add_systems(Startup, setup) if only used for floor
    }
}

// Remove the floor spawn from setup() function
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check -p server` passes
- [ ] `cargo server` starts without crash

#### Manual Verification:
- [ ] Server logs show VoxelWorldPlugin initialized

---

## Phase 4: Client MapPlugin

### Overview
Create client-side voxel world plugin that applies server updates and renders terrain.

### Changes Required:

#### 1. Client Map Module
**File**: `crates/client/src/map.rs` (new file)
**Changes**: Create ClientMapPlugin

```rust
use bevy::prelude::*;
use bevy_voxel_world::prelude::*;
use lightyear::prelude::client::*;
use protocol::map::*;

pub struct ClientMapPlugin;

impl Plugin for ClientMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelWorldPlugin::<MapWorld>::default())
            .add_systems(Update, (
                apply_voxel_broadcasts,
                apply_initial_state,
            ));
    }
}

fn apply_voxel_broadcasts(
    mut events: EventReader<MessageReceive<VoxelEditBroadcast>>,
    mut voxel_world: VoxelWorld<MapWorld>,
) {
    for MessageReceive { message, .. } in events.read() {
        voxel_world.set_voxel(message.position, message.voxel.into());
    }
}

fn apply_initial_state(
    mut events: EventReader<MessageReceive<VoxelStateSync>>,
    mut voxel_world: VoxelWorld<MapWorld>,
) {
    for MessageReceive { message, .. } in events.read() {
        for (pos, voxel) in &message.modifications {
            voxel_world.set_voxel(*pos, (*voxel).into());
        }
    }
}
```

#### 2. Camera Component
**File**: `crates/client/src/gameplay.rs`
**Changes**: Add VoxelWorldCamera to camera entity

```rust
use bevy_voxel_world::prelude::*;
use protocol::map::MapWorld;

// In camera spawn or existing camera system:
commands.entity(camera_entity).insert(VoxelWorldCamera::<MapWorld>::default());
```

#### 3. Client lib.rs
**File**: `crates/client/src/lib.rs`
**Changes**: Add map module

```rust
pub mod map;
```

#### 4. Client Cargo.toml
**File**: `crates/client/Cargo.toml`
**Changes**: Add bevy_voxel_world dependency

```toml
bevy_voxel_world.workspace = true
```

#### 5. Client Gameplay
**File**: `crates/client/src/gameplay.rs`
**Changes**: Remove floor handling, add ClientMapPlugin

```rust
use crate::map::ClientMapPlugin;

impl Plugin for ClientGameplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ClientMapPlugin);
        // Remove: handle_new_floor system
    }
}

// Remove handle_new_floor function
```

#### 6. Remove FloorMarker Registration
**File**: `crates/protocol/src/lib.rs`
**Changes**: Remove FloorMarker component registration

```rust
// Remove: app.register_component::<FloorMarker>();
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo check -p client` passes
- [ ] `cargo client -c 1` starts without crash

#### Manual Verification:
- [ ] Voxel terrain visible in client window
- [ ] Chunks generate around camera position

---

## Phase 5: Physics Integration

### Overview
Add physics colliders to voxel chunks for character collision.

### Changes Required:

#### 1. Chunk Physics System
**File**: `crates/protocol/src/map.rs`
**Changes**: Add system to attach colliders to chunks

```rust
use avian3d::prelude::*;
use bevy_voxel_world::prelude::*;

/// Plugin for shared voxel physics (used by both server and client)
pub struct VoxelPhysicsPlugin;

impl Plugin for VoxelPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, attach_chunk_colliders);
    }
}

/// Marker for chunks that have colliders
#[derive(Component)]
pub struct ChunkHasCollider;

fn attach_chunk_colliders(
    mut commands: Commands,
    chunks: Query<(Entity, &Handle<Mesh>), (With<ChunkInfo<MapWorld>>, Without<ChunkHasCollider>)>,
    meshes: Res<Assets<Mesh>>,
) {
    for (entity, mesh_handle) in &chunks {
        if let Some(mesh) = meshes.get(mesh_handle) {
            if let Some(collider) = Collider::trimesh_from_mesh(mesh) {
                commands.entity(entity).insert((
                    collider,
                    RigidBody::Static,
                    ChunkHasCollider,
                ));
            }
        }
    }
}
```

Note: Server runs headless without mesh assets. Server-side physics requires alternative approach:

#### 2. Server Physics Alternative
**File**: `crates/server/src/map.rs`
**Changes**: Server uses voxel-based colliders instead of trimesh

```rust
fn attach_server_chunk_colliders(
    mut commands: Commands,
    chunks: Query<(Entity, &ChunkInfo<MapWorld>), Without<ChunkHasCollider>>,
    voxel_world: VoxelWorld<MapWorld>,
) {
    for (entity, chunk_info) in &chunks {
        // Get occupied voxel positions in this chunk
        let chunk_pos = chunk_info.position;
        let mut occupied = Vec::new();

        for x in 0..32 {
            for y in 0..32 {
                for z in 0..32 {
                    let local = IVec3::new(x, y, z);
                    let world = chunk_pos * 32 + local;
                    if matches!(voxel_world.get_voxel(world), WorldVoxel::Solid(_)) {
                        occupied.push(world.as_vec3());
                    }
                }
            }
        }

        if !occupied.is_empty() {
            let collider = Collider::compound(
                occupied.iter().map(|&pos| {
                    (pos, Quat::IDENTITY, Collider::cuboid(1.0, 1.0, 1.0))
                }).collect()
            );

            commands.entity(entity).insert((
                collider,
                RigidBody::Static,
                ChunkHasCollider,
            ));
        }
    }
}
```

#### 3. Add Physics Plugin
**File**: `crates/server/src/map.rs` and `crates/client/src/map.rs`
**Changes**: Add VoxelPhysicsPlugin or server-specific physics

```rust
// In ServerMapPlugin::build()
app.add_systems(Update, attach_server_chunk_colliders);

// In ClientMapPlugin::build()
app.add_plugins(VoxelPhysicsPlugin);
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo test-all` passes
- [ ] `cargo server` and `cargo client -c 1` run without crash

#### Manual Verification:
- [ ] Character stands on voxel terrain (doesn't fall through)
- [ ] Character collides with placed voxels

---

## Phase 6: Admin Voxel Editing

### Overview
Add input handling for admin clients to place/remove voxels via mouse clicks.

### Changes Required:

#### 1. PlayerActions Extension
**File**: `crates/protocol/src/lib.rs`
**Changes**: Add voxel edit actions

```rust
pub enum PlayerActions {
    Move,
    Jump,
    PlaceVoxel,
    RemoveVoxel,
}

impl Actionlike for PlayerActions {
    fn input_control_kind(&self) -> InputControlKind {
        match self {
            Self::Move => InputControlKind::DualAxis,
            Self::Jump | Self::PlaceVoxel | Self::RemoveVoxel => InputControlKind::Button,
        }
    }
}
```

#### 2. Input Mapping
**File**: `crates/client/src/gameplay.rs`
**Changes**: Add mouse button mappings

```rust
InputMap::new([(PlayerActions::Jump, KeyCode::Space)])
    .with(PlayerActions::Jump, GamepadButton::South)
    .with_dual_axis(PlayerActions::Move, GamepadStick::LEFT)
    .with_dual_axis(PlayerActions::Move, VirtualDPad::wasd())
    .with(PlayerActions::PlaceVoxel, MouseButton::Left)
    .with(PlayerActions::RemoveVoxel, MouseButton::Right)
```

#### 3. Voxel Edit Input System
**File**: `crates/client/src/map.rs`
**Changes**: Add raycast and edit request system

```rust
use leafwing_input_manager::prelude::*;

fn handle_voxel_input(
    voxel_world: VoxelWorld<MapWorld>,
    action_state: Query<&ActionState<PlayerActions>, With<Controlled>>,
    camera: Query<(&Camera, &GlobalTransform), With<VoxelWorldCamera<MapWorld>>>,
    windows: Query<&Window>,
    mut send: EventWriter<MessageSend<VoxelEditRequest>>,
) {
    let Ok(action) = action_state.get_single() else { return };
    let Ok((camera, transform)) = camera.get_single() else { return };
    let Ok(window) = windows.get_single() else { return };

    let Some(cursor) = window.cursor_position() else { return };
    let Ok(ray) = camera.viewport_to_world(transform, cursor) else { return };

    let place = action.just_pressed(&PlayerActions::PlaceVoxel);
    let remove = action.just_pressed(&PlayerActions::RemoveVoxel);

    if !place && !remove { return }

    if let Some(hit) = voxel_world.raycast(ray, &|_| true) {
        let request = if remove {
            VoxelEditRequest {
                position: hit.position,
                voxel: VoxelType::Air,
            }
        } else {
            // Place adjacent to hit surface
            let place_pos = hit.position + hit.normal.as_ivec3();
            VoxelEditRequest {
                position: place_pos,
                voxel: VoxelType::Solid(0),
            }
        };

        send.send(MessageSend {
            message: request,
            context: SendContext::default(),
        });
    }
}
```

#### 4. Register Input System
**File**: `crates/client/src/map.rs`
**Changes**: Add to ClientMapPlugin

```rust
impl Plugin for ClientMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelWorldPlugin::<MapWorld>::default())
            .add_systems(Update, (
                apply_voxel_broadcasts,
                apply_initial_state,
                handle_voxel_input,
            ));
    }
}
```

### Success Criteria:

#### Automated Verification:
- [ ] `cargo test-all` passes
- [ ] `cargo client -c 1` runs without crash

#### Manual Verification:
- [ ] Left-click on terrain places a voxel
- [ ] Right-click on voxel removes it
- [ ] Changes appear on all connected clients

---

## Phase 7: Web Client Support

### Overview
Ensure WASM build works with voxel system.

### Changes Required:

#### 1. Web Cargo.toml
**File**: `crates/web/Cargo.toml`
**Changes**: Add bevy_voxel_world dependency

```toml
bevy_voxel_world.workspace = true
```

#### 2. Web Map Integration
**File**: `crates/web/src/lib.rs` or similar
**Changes**: Include ClientMapPlugin in web build

Same as native client - `ClientMapPlugin` should work without modification.

### Success Criteria:

#### Automated Verification:
- [ ] `bevy run web` builds successfully

#### Manual Verification:
- [ ] Voxel terrain renders in browser
- [ ] Input handling works in browser

---

## Testing Strategy

### Unit Tests
- `VoxelType` serialization roundtrip
- `MapWorld` voxel_lookup_delegate returns expected values

### Integration Tests
- Server receives `VoxelEditRequest`, broadcasts `VoxelEditBroadcast`
- Client applies `VoxelStateSync` on connect
- Physics colliders attached to chunks

### Manual Testing Steps
1. Start server: `cargo server`
2. Connect client: `cargo client -c 1`
3. Verify character stands on voxel terrain
4. Left-click to place voxel, verify appears
5. Right-click to remove voxel, verify disappears
6. Connect second client, verify sees modifications
7. Disconnect and reconnect, verify receives state sync

## Performance Considerations

- Chunk spawning distance of 10 limits loaded chunks
- Server uses compound voxel colliders (not trimesh) for better performance
- Modification list grows unbounded - future optimization: spatial compression

## Migration Notes

- Remove `FloorMarker`, `FloorPhysicsBundle` from protocol
- Remove floor spawn from server gameplay
- Remove floor handling from client gameplay
- Existing character physics unchanged (uses Avian3D)

## References

- Research: [doc/research/2025-12-24-bevy-voxel-world-map-plugins.md](doc/research/2025-12-24-bevy-voxel-world-map-plugins.md)
- bevy_voxel_world: https://github.com/splashdust/bevy_voxel_world
- Avian voxel colliders: https://joonaa.dev/blog/09/avian-0-4
