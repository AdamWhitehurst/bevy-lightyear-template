---
date: 2025-12-24T17:24:11-08:00
researcher: Claude
git_commit: 0249846c7cc5ddb7dbf44030b128f3c477ca4166
branch: master
repository: bevy-lightyear-template
topic: "bevy_voxel_world MapPlugins for Server/Client"
tags: [research, voxel, map, networking, physics, bevy_voxel_world]
status: complete
last_updated: 2025-12-24
last_updated_by: Claude
last_updated_note: "Added follow-up research for initial sync options"
---

# Research: bevy_voxel_world MapPlugins for Server/Client

**Date**: 2025-12-24T17:24:11-08:00
**Researcher**: Claude
**Git Commit**: 0249846c7cc5ddb7dbf44030b128f3c477ca4166
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How to use `bevy_voxel_world` to write MapPlugins for server and client that can load the default flat level we have now as a map. The client receives the map level data from server. The map is editable, so server supports editing commands to place and remove voxels from the map. The client can left-click on the map to place a voxel and right-click to remove one. The map has a rigidbody and collider to interact with physics.

## Summary

`bevy_voxel_world` (v0.13+ for Bevy 0.17) provides a chunk-based voxel terrain system with async meshing. It has no built-in networking or physics - these must be integrated manually. The current codebase has a simple 100x1x100 static floor; replacing it with voxel terrain requires:

1. **Server**: Run `VoxelWorldPlugin`, handle voxel edit messages, replicate changes
2. **Client**: Run `VoxelWorldPlugin`, send edit requests, apply server updates
3. **Physics**: Use Avian3D's `Collider::voxels()` or trimesh from chunk meshes
4. **Networking**: Define custom lightyear messages for voxel edits

## Detailed Findings

### Current Ground Implementation

The existing flat level is a single static cuboid:

- **Server spawns** at `crates/server/src/gameplay.rs:46-54`
- **Constants**: `FLOOR_WIDTH=100.0`, `FLOOR_HEIGHT=1.0` at `crates/protocol/src/lib.rs:13-14`
- **Physics**: `FloorPhysicsBundle` with `Collider::cuboid()` and `RigidBody::Static`
- **Replication**: Uses `Replicate::to_clients(NetworkTarget::All)`

### bevy_voxel_world Crate

**Version**: 0.13.0+ for Bevy 0.17

**Core API**:
```rust
// Config trait
impl VoxelWorldConfig for MyWorld {
    type MaterialIndex = u8;
    type ChunkUserBundle = ();

    fn spawning_distance(&self) -> u32 { 25 }

    fn voxel_lookup_delegate(&self) -> VoxelLookupDelegate {
        Box::new(|pos: IVec3| {
            if pos.y < 0 { WorldVoxel::Solid(0) } else { WorldVoxel::Air }
        })
    }
}

// Runtime access via system param
fn edit_voxels(mut voxel_world: VoxelWorld<MyWorld>) {
    voxel_world.set_voxel(IVec3::new(5, 0, 5), WorldVoxel::Solid(1));
    voxel_world.set_voxel(IVec3::new(3, 0, 3), WorldVoxel::Air);
}
```

**Key features**:
- 32x32x32 chunks with async mesh generation
- Two-layer system: procedural base + persistent modifications
- Only modified voxels stored in memory
- Built-in raycasting: `voxel_world.raycast(ray, &|_| true)`

### Networking Integration

The codebase uses lightyear with:
- `Replicate::to_clients()` for entity replication
- `InputPlugin<PlayerActions>` for input networking
- `SendUpdatesMode::SinceLastAck` for efficient deltas

**Required additions for voxel networking**:

```rust
// In protocol crate
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VoxelEditRequest {
    pub position: IVec3,
    pub voxel: VoxelType, // Solid(u8) or Air
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VoxelEditBroadcast {
    pub position: IVec3,
    pub voxel: VoxelType,
}
```

Register as lightyear messages with appropriate channels.

### Physics Integration

Avian3D 0.4+ supports native voxel colliders:

```rust
use avian3d::prelude::*;

// Option 1: Voxel collider from occupied positions
let collider = Collider::voxels(voxel_size, occupied_positions);

// Option 2: Trimesh from chunk mesh
let collider = Collider::trimesh_from_mesh(&mesh).unwrap();
```

**Integration approach**: Use `ChunkUserBundle` to attach colliders to chunks:

```rust
impl VoxelWorldConfig for MapWorld {
    type ChunkUserBundle = (Collider, RigidBody);

    fn chunk_meshing_delegate(&self) -> Option<ChunkMeshingDelegate<Self>> {
        Some(Box::new(|voxel_data, _| {
            let mesh = generate_mesh(voxel_data);
            let collider = Collider::trimesh_from_mesh(&mesh);
            (mesh, Some((collider, RigidBody::Static)))
        }))
    }
}
```

### Input Handling for Voxel Editing

Current input uses `leafwing-input-manager`. For voxel editing:

```rust
// Add to PlayerActions
pub enum PlayerActions {
    Move,
    Jump,
    PlaceVoxel,   // Left-click
    RemoveVoxel,  // Right-click
}

// Input mapping
InputMap::new([...])
    .with(PlayerActions::PlaceVoxel, MouseButton::Left)
    .with(PlayerActions::RemoveVoxel, MouseButton::Right)
```

**Raycast for click target**:
```rust
fn handle_voxel_click(
    voxel_world: VoxelWorld<MapWorld>,
    camera: Query<(&Camera, &GlobalTransform)>,
    windows: Query<&Window>,
    action_state: Query<&ActionState<PlayerActions>, With<Controlled>>,
) {
    let (camera, transform) = camera.single().unwrap();
    let window = windows.single().unwrap();

    if let Some(cursor) = window.cursor_position() {
        if let Ok(ray) = camera.viewport_to_world(transform, cursor) {
            if let Some(hit) = voxel_world.raycast(ray, &|_| true) {
                // hit.position = voxel position
                // hit.normal = surface normal for adjacent placement
            }
        }
    }
}
```

## Architecture for MapPlugins

### Server MapPlugin

```rust
pub struct ServerMapPlugin;

impl Plugin for ServerMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelWorldPlugin::with_config(MapWorld::default()))
           .add_systems(Startup, setup_map)
           .add_systems(FixedUpdate, (
               handle_voxel_edit_requests,
               broadcast_voxel_changes,
           ));
    }
}

fn setup_map(mut voxel_world: VoxelWorld<MapWorld>) {
    // Generate flat terrain at y < 0
    // Procedural via voxel_lookup_delegate
}

fn handle_voxel_edit_requests(
    mut events: EventReader<MessageReceive<VoxelEditRequest>>,
    mut voxel_world: VoxelWorld<MapWorld>,
    mut broadcast: EventWriter<MessageSend<VoxelEditBroadcast>>,
) {
    for msg in events.read() {
        voxel_world.set_voxel(msg.position, msg.voxel.into());
        broadcast.send(VoxelEditBroadcast {
            position: msg.position,
            voxel: msg.voxel
        });
    }
}
```

### Client MapPlugin

```rust
pub struct ClientMapPlugin;

impl Plugin for ClientMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelWorldPlugin::with_config(MapWorld::default()))
           .add_systems(Update, (
               handle_voxel_input,
               apply_voxel_broadcasts,
           ));
    }
}

fn handle_voxel_input(
    voxel_world: VoxelWorld<MapWorld>,
    action_state: Query<&ActionState<PlayerActions>, With<Controlled>>,
    camera: Query<(&Camera, &GlobalTransform), With<VoxelWorldCamera<MapWorld>>>,
    windows: Query<&Window>,
    mut send: EventWriter<MessageSend<VoxelEditRequest>>,
) {
    // Raycast, determine position, send request to server
}

fn apply_voxel_broadcasts(
    mut events: EventReader<MessageReceive<VoxelEditBroadcast>>,
    mut voxel_world: VoxelWorld<MapWorld>,
) {
    for msg in events.read() {
        voxel_world.set_voxel(msg.position, msg.voxel.into());
    }
}
```

### Shared MapWorld Config

```rust
#[derive(Resource, Clone, Default)]
pub struct MapWorld;

impl VoxelWorldConfig for MapWorld {
    type MaterialIndex = u8;
    type ChunkUserBundle = (Collider, RigidBody);

    fn spawning_distance(&self) -> u32 { 10 }

    fn voxel_lookup_delegate(&self) -> VoxelLookupDelegate {
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
```

## Code References

- `crates/protocol/src/lib.rs:13-14` - Current floor constants
- `crates/protocol/src/lib.rs:61-74` - FloorPhysicsBundle
- `crates/server/src/gameplay.rs:46-54` - Server floor spawn
- `crates/client/src/gameplay.rs:16-26` - Client floor physics handling
- `crates/protocol/src/lib.rs:18-31` - PlayerActions enum
- `crates/protocol/src/lib.rs:80-85` - InputPlugin registration
- `crates/server/src/network.rs:203-209` - Replication sender config

## External References

- [bevy_voxel_world GitHub](https://github.com/splashdust/bevy_voxel_world)
- [bevy_voxel_world docs.rs](https://docs.rs/bevy_voxel_world)
- [set_voxel.rs example](https://github.com/splashdust/bevy_voxel_world/blob/main/examples/set_voxel.rs)
- [Avian 0.4 voxel colliders](https://joonaa.dev/blog/09/avian-0-4)
- [logic_voxels multiplayer reference](https://github.com/mwbryant/logic_voxels)

## Gaps and Considerations

1. **Initial sync**: Client needs full voxel state on connect (chunk streaming or initial message)
2. **Chunk collider regeneration**: When voxels change, chunk colliders must be rebuilt
3. **Authority**: Server must validate edits (range checks, rate limiting)
4. **Serialization**: `WorldVoxel` needs serde derives for networking
5. **Camera requirement**: `VoxelWorldCamera<T>` component required for chunk spawning

## Decisions

1. **Persistence**: No saving/persistence yet
2. **Initial sync**: Option B - Chunk Streaming (server-authoritative)
3. **Materials**: Solid colors for now
4. **Physics colliders**: Voxel grid (faster updates)

---

## Follow-up Research: Initial Sync Options (2025-12-24)

### Option A: Deterministic Generation (Recommended for procedural terrain)

Both server and client share the same `voxel_lookup_delegate`. Client generates terrain locally.
- Server only sends **modification layer** (the `HashMap<IVec3, WorldVoxel>` of edits)
- Leverages bevy_voxel_world's two-layer architecture
- Minimal network traffic for unedited worlds

```rust
// On client connect, server sends only modifications
#[derive(Message, Serialize, Deserialize)]
pub struct InitialVoxelState {
    pub modifications: Vec<(IVec3, VoxelType)>,
}
```

**Pros**: Minimal bandwidth, instant terrain on connect
**Cons**: Exposes generation algorithm to clients

### Option B: Chunk Streaming

Server tracks each client's position and streams visible chunks.

```rust
#[derive(Component)]
pub struct VoxelViewer {
    pub view_distance: u32,  // Chunks radius
    pub known_chunks: HashSet<IVec3>,
}

// Server system
fn stream_chunks_to_clients(
    viewers: Query<(&VoxelViewer, &Position, &ControlledBy)>,
    voxel_world: VoxelWorld<MapWorld>,
) {
    for (viewer, pos, controlled) in &viewers {
        let current_chunk = pos.0.as_ivec3() / 32;
        let visible = chunks_in_radius(current_chunk, viewer.view_distance);
        let new_chunks = visible.difference(&viewer.known_chunks);
        // Send new_chunks to controlled.owner
    }
}
```

**Pros**: Works for server-generated worlds, supports dynamic terrain
**Cons**: More bandwidth, complexity

### Option C: Hybrid (Recommended)

Combine A and B:
1. Share `voxel_lookup_delegate` for base terrain (deterministic)
2. On connect, send modification HashMap
3. Stream modification updates as they happen

```rust
// Initial sync
#[derive(Message)]
pub struct WorldSeed { pub seed: u64 }

#[derive(Message)]
pub struct ModificationSync { pub edits: Vec<(IVec3, VoxelType)> }

// Runtime edits
#[derive(Message)]
pub struct VoxelEditBroadcast { pub position: IVec3, pub voxel: VoxelType }
```

### Compression

For large modification sets, apply compression:

```rust
// Pipeline: Palette encode -> RLE -> lz4
use lz4_flex::compress_prepend_size;

fn compress_modifications(mods: &[(IVec3, VoxelType)]) -> Vec<u8> {
    let serialized = bincode::serialize(mods).unwrap();
    compress_prepend_size(&serialized)
}
```

Libraries:
- `lz4_flex` - fastest, pure Rust, `no_std` support
- `bincode` - compact binary serialization

### Lightyear Channel Setup

```rust
#[derive(Channel)]
pub struct ChunkChannel;

// In ProtocolPlugin
app.add_channel::<ChunkChannel>(ChannelSettings {
    mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
    ..default()
});

app.add_message::<InitialVoxelState>(MessageSettings {
    channel: ChannelKind::of::<ChunkChannel>(),
    ..default()
});
```

### References

- [logic_voxels](https://github.com/mwbryant/logic_voxels) - Bevy multiplayer voxel with renet
- [Minecraft chunk format](https://minecraft.wiki/w/Java_Edition_protocol/Chunk_format)
- [Voxel compression techniques](https://eisenwave.github.io/voxel-compression-docs/rle/rle.html)
- [lz4_flex](https://github.com/PSeitz/lz4_flex)
