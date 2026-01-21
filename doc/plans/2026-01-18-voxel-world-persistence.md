# Voxel World Persistence Implementation Plan

## Overview

Implement persistence for the server's voxel world modifications using binary serialization (bincode) with debounced saves. The system will automatically load saved modifications on server startup and save changes with a 1-second debounce delay (max 5 seconds dirty duration) plus graceful shutdown saves.

## Current State Analysis

The server tracks all voxel modifications in a `VoxelModifications` resource containing `Vec<(IVec3, VoxelType)>` at `crates/server/src/map.rs:31-35`. This data exists only in memory and is lost on server restart. The procedural terrain base (flat terrain below y=0) is deterministic and doesn't require saving.

### Key Discoveries:
- VoxelModifications resource initialized via `init_resource::<VoxelModifications>()` in ServerMapPlugin at `crates/server/src/map.rs:18`
- Modifications appended in `handle_voxel_edit_requests` system at `crates/server/src/map.rs:86-88`
- VoxelType already has `Serialize` and `Deserialize` derives at `crates/protocol/src/map.rs:36-59`
- No existing file I/O patterns in codebase - this is the first persistence implementation
- VoxelWorld<MapWorld> resource available after PreStartup schedule completes
- Startup schedule is ideal for loading: resources exist, chunks haven't spawned yet
- Last schedule with AppExit detection is the safe place for shutdown saves

## Desired End State

Server voxel world modifications persist across restarts. On startup, the server loads saved modifications from `world_save/voxel_world.bin`. During gameplay, modifications are saved 1 second after the last edit (or after 5 seconds of continuous edits). On graceful shutdown, any pending changes are saved immediately.

### Verification:
1. **Automated**: Server starts, loads file, builds successfully
2. **Manual**:
   - Place voxels, restart server, verify voxels persist
   - Edit continuously, verify save happens within 5 seconds
   - Corrupt save file, verify server starts with empty world and backs up corrupt file

## What We're NOT Doing

- Compression (gzip/zstd) - not needed yet
- Backup rotation - single save file only
- Multiple worlds - single world hardcoded
- Async I/O - blocking writes acceptable for current scale
- Spatial indexing - full vector only
- Automatic migration between generation versions - incompatible saves rejected

## Implementation Approach

Add bincode dependency for binary serialization. Add seed and generation_version fields to MapWorld resource (in protocol crate) to track procedural generation parameters. Create serializable wrapper struct with version and generation metadata fields. Implement atomic file writes (write to temp, rename). On load, verify generation metadata matches current MapWorld configuration - reject incompatible saves to prevent modifications from applying to wrong terrain. Load in Startup schedule before chunks spawn. Track dirty state and implement debounced save logic in Update schedule. Save on AppExit in Last schedule.

## Phase 1: Core Serialization Infrastructure

### Overview
Add binary serialization capability with atomic file writes and error recovery.

### Changes Required:

#### 1. Add bincode dependency
**File**: `crates/server/Cargo.toml`
**Changes**: Add bincode to dependencies section

```toml
[dependencies]
# ... existing dependencies ...
bincode = "1.3"
```

#### 2. Add generation metadata to MapWorld
**File**: `crates/protocol/src/map.rs`
**Changes**: Modify MapWorld struct (around line 10-11)

**Find this struct**:
```rust
#[derive(Resource, Clone, Default, Serialize, Deserialize, Eq, PartialEq)]
pub struct MapWorld;
```

**Replace with**:
```rust
#[derive(Resource, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct MapWorld {
    pub seed: u64,
    pub generation_version: u32,
}

impl Default for MapWorld {
    fn default() -> Self {
        Self {
            seed: 0,  // Flat terrain has no seed
            generation_version: 1,  // Generation algorithm version
        }
    }
}
```

#### 3. Create serialization data structure
**File**: `crates/server/src/map.rs`
**Changes**: Add save file struct after VoxelModifications definition (after line 35)

```rust
#[derive(Serialize, Deserialize)]
struct VoxelWorldSave {
    version: u32,
    generation_seed: u64,
    generation_version: u32,
    modifications: Vec<(IVec3, VoxelType)>,
}

const SAVE_VERSION: u32 = 1;
const SAVE_PATH: &str = "world_save/voxel_world.bin";
```

#### 4. Implement atomic save function
**File**: `crates/server/src/map.rs`
**Changes**: Add save function before ServerMapPlugin impl (after VoxelWorldSave struct)

```rust
fn save_voxel_world_to_disk(
    modifications: &[(IVec3, VoxelType)],
    map_world: &MapWorld,
) -> std::io::Result<()> {
    use std::fs;
    use std::path::Path;

    let save_data = VoxelWorldSave {
        version: SAVE_VERSION,
        generation_seed: map_world.seed,
        generation_version: map_world.generation_version,
        modifications: modifications.to_vec(),
    };

    // Create directory if it doesn't exist
    if let Some(parent) = Path::new(SAVE_PATH).parent() {
        fs::create_dir_all(parent)?;
    }

    // Serialize to bytes
    let bytes = bincode::serialize(&save_data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    // Atomic write: temp file + rename
    let temp_path = format!("{}.tmp", SAVE_PATH);
    fs::write(&temp_path, bytes)?;
    fs::rename(temp_path, SAVE_PATH)?;

    eprintln!("Saved {} voxel modifications to {}", modifications.len(), SAVE_PATH);
    Ok(())
}
```

#### 5. Implement load function with error recovery
**File**: `crates/server/src/map.rs`
**Changes**: Add load function after save function

```rust
fn load_voxel_world_from_disk(
    map_world: &MapWorld,
) -> Vec<(IVec3, VoxelType)> {
    use std::fs;
    use std::path::Path;

    let path = Path::new(SAVE_PATH);

    // File doesn't exist - normal for first run
    if !path.exists() {
        eprintln!("No save file found at {}, starting with empty world", SAVE_PATH);
        return Vec::new();
    }

    // Read file
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error reading save file: {}, starting with empty world", e);
            return Vec::new();
        }
    };

    // Deserialize
    let save_data: VoxelWorldSave = match bincode::deserialize(&bytes) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Error deserializing save file: {}", e);
            // Backup corrupt file
            let backup_path = format!("{}.corrupt", SAVE_PATH);
            if let Err(e) = fs::rename(path, &backup_path) {
                eprintln!("Failed to backup corrupt file: {}", e);
            } else {
                eprintln!("Backed up corrupt file to {}", backup_path);
            }
            eprintln!("Starting with empty world");
            return Vec::new();
        }
    };

    // Check save file version
    if save_data.version != SAVE_VERSION {
        eprintln!(
            "Save file version mismatch (expected {}, got {}), starting with empty world",
            SAVE_VERSION, save_data.version
        );
        return Vec::new();
    }

    // Check generation compatibility
    if save_data.generation_seed != map_world.seed {
        eprintln!(
            "WARNING: Save file generation seed mismatch (saved: {}, current: {})",
            save_data.generation_seed, map_world.seed
        );
        eprintln!("Modifications may not align with current procedural terrain!");
        eprintln!("Starting with empty world to avoid inconsistencies");
        return Vec::new();
    }

    if save_data.generation_version != map_world.generation_version {
        eprintln!(
            "WARNING: Generation algorithm version mismatch (saved: {}, current: {})",
            save_data.generation_version, map_world.generation_version
        );
        eprintln!("Modifications may not align with current procedural terrain!");
        eprintln!("Starting with empty world to avoid inconsistencies");
        return Vec::new();
    }

    eprintln!("Loaded {} voxel modifications from {}", save_data.modifications.len(), SAVE_PATH);
    save_data.modifications
}
```

### Success Criteria:

#### Automated Verification:
- [x] Server builds: `cargo check --package server`
- [x] All tests pass: `cargo test-all`

#### Manual Verification:
- [x] Functions compile and link correctly
- [x] No warnings from new code

---

## Phase 2: Startup Loading

### Overview
Load saved voxel modifications on server startup before chunks spawn or clients connect.

### Changes Required:

#### 1. Add load system to plugin
**File**: `crates/server/src/map.rs`
**Changes**: Add system registration in ServerMapPlugin::build (after line 18)

```rust
impl Plugin for ServerMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VoxelWorldPlugin::<MapWorld>::with_config(MapWorld))
            .init_resource::<VoxelModifications>()
            .add_systems(Startup, load_voxel_world)  // <-- Add this line
            .add_systems(
                Update,
                (
                    handle_voxel_edit_requests,
                    protocol::attach_chunk_colliders,
                    debug_server_chunks.run_if(on_timer(Duration::from_secs_f32(5.0))),
                ),
            )
            .observe(send_initial_voxel_state);
    }
}
```

#### 2. Implement load system
**File**: `crates/server/src/map.rs`
**Changes**: Add system function before ServerMapPlugin impl

```rust
fn load_voxel_world(
    mut voxel_world: VoxelWorld<MapWorld>,
    mut modifications: ResMut<VoxelModifications>,
    map_world: Res<MapWorld>,
) {
    let loaded_mods = load_voxel_world_from_disk(&map_world);

    if loaded_mods.is_empty() {
        return;
    }

    // Apply to VoxelModifications resource (for network sync)
    modifications.modifications = loaded_mods.clone();

    // Apply to VoxelWorld (populates bevy_voxel_world's internal ModifiedVoxels)
    for (pos, voxel_type) in &loaded_mods {
        voxel_world.set_voxel(*pos, (*voxel_type).into());
    }

    eprintln!("Applied {} loaded modifications to voxel world", loaded_mods.len());
}
```

### Success Criteria:

#### Automated Verification:
- [x] Server builds and runs: `cargo server`
- [x] Client can connect: `cargo client -c 1`
- [x] All tests pass: `cargo test-all`

#### Manual Verification:
- [x] Server starts successfully with no save file (logs "No save file found")
- [x] Create save file manually with test data, verify it loads on startup
- [x] Server logs show loaded modifications count
- [x] Connecting client receives loaded voxel state

---

## Phase 3: Debounced Save System

### Overview
Implement debounced save logic that saves 1 second after last edit, with a maximum 5-second dirty duration, plus graceful shutdown saves.

### Changes Required:

#### 1. Add dirty tracking resource
**File**: `crates/server/src/map.rs`
**Changes**: Add resource after VoxelModifications definition (after line 35, before VoxelWorldSave)

```rust
#[derive(Resource)]
struct VoxelDirtyState {
    is_dirty: bool,
    last_edit_time: f64,
    first_dirty_time: Option<f64>,
}

impl Default for VoxelDirtyState {
    fn default() -> Self {
        Self {
            is_dirty: false,
            last_edit_time: 0.0,
            first_dirty_time: None,
        }
    }
}

const SAVE_DEBOUNCE_SECONDS: f64 = 1.0;
const MAX_DIRTY_SECONDS: f64 = 5.0;
```

#### 2. Initialize dirty state resource
**File**: `crates/server/src/map.rs`
**Changes**: Add to ServerMapPlugin::build (after VoxelModifications init at line 18)

```rust
.init_resource::<VoxelModifications>()
.init_resource::<VoxelDirtyState>()  // <-- Add this line
.add_systems(Startup, load_voxel_world)
```

#### 3. Mark dirty on voxel edits
**File**: `crates/server/src/map.rs`
**Changes**: Modify handle_voxel_edit_requests system signature (line 37) and add dirty tracking

**Find this signature (line 37-43)**:
```rust
fn handle_voxel_edit_requests(
    mut voxel_world: VoxelWorld<MapWorld>,
    mut modifications: ResMut<VoxelModifications>,
    mut message_receiver: Query<&mut MessageReceiver<VoxelEditRequest>>,
    mut message_sender: Query<&mut MessageSender<VoxelEditBroadcast>>,
    chunk_map: Res<ChunkMap<MapWorld>>,
)
```

**Replace with**:
```rust
fn handle_voxel_edit_requests(
    mut voxel_world: VoxelWorld<MapWorld>,
    mut modifications: ResMut<VoxelModifications>,
    mut dirty_state: ResMut<VoxelDirtyState>,
    time: Res<Time>,
    mut message_receiver: Query<&mut MessageReceiver<VoxelEditRequest>>,
    mut message_sender: Query<&mut MessageSender<VoxelEditBroadcast>>,
    chunk_map: Res<ChunkMap<MapWorld>>,
)
```

**Add after modifications.modifications.push() (after line 88)**:
```rust
modifications
    .modifications
    .push((request.position, request.voxel));

// Mark dirty
let now = time.elapsed_secs_f64();
if !dirty_state.is_dirty {
    dirty_state.first_dirty_time = Some(now);
}
dirty_state.is_dirty = true;
dirty_state.last_edit_time = now;
```

#### 4. Add debounced save system
**File**: `crates/server/src/map.rs`
**Changes**: Add system before ServerMapPlugin impl

```rust
fn save_voxel_world_debounced(
    modifications: Res<VoxelModifications>,
    map_world: Res<MapWorld>,
    mut dirty_state: ResMut<VoxelDirtyState>,
    time: Res<Time>,
) {
    if !dirty_state.is_dirty {
        return;
    }

    let now = time.elapsed_secs_f64();
    let time_since_edit = now - dirty_state.last_edit_time;
    let time_since_first_dirty = dirty_state.first_dirty_time
        .map(|t| now - t)
        .unwrap_or(0.0);

    let should_save = time_since_edit >= SAVE_DEBOUNCE_SECONDS
        || time_since_first_dirty >= MAX_DIRTY_SECONDS;

    if should_save {
        if let Err(e) = save_voxel_world_to_disk(&modifications.modifications, &map_world) {
            eprintln!("Failed to save voxel world: {}", e);
        }

        dirty_state.is_dirty = false;
        dirty_state.first_dirty_time = None;
    }
}
```

#### 5. Add shutdown save system
**File**: `crates/server/src/map.rs`
**Changes**: Add system before ServerMapPlugin impl

```rust
fn save_voxel_world_on_shutdown(
    mut exit_reader: MessageReader<AppExit>,
    modifications: Res<VoxelModifications>,
    map_world: Res<MapWorld>,
    dirty_state: Res<VoxelDirtyState>,
) {
    if exit_reader.is_empty() {
        return;
    }
    exit_reader.clear();

    if dirty_state.is_dirty {
        eprintln!("Saving voxel world on shutdown...");
        if let Err(e) = save_voxel_world_to_disk(&modifications.modifications, &map_world) {
            eprintln!("Failed to save voxel world on shutdown: {}", e);
        }
    }
}
```

#### 6. Register save systems
**File**: `crates/server/src/map.rs`
**Changes**: Add to ServerMapPlugin::build after Update systems block

```rust
.add_systems(
    Update,
    (
        handle_voxel_edit_requests,
        protocol::attach_chunk_colliders,
        debug_server_chunks.run_if(on_timer(Duration::from_secs_f32(5.0))),
    ),
)
.add_systems(Update, save_voxel_world_debounced)  // <-- Add this line
.add_systems(Last, save_voxel_world_on_shutdown)  // <-- Add this line
.observe(send_initial_voxel_state);
```

#### 7. Add AppExit import
**File**: `crates/server/src/map.rs`
**Changes**: Add to imports at top of file (around line 1-10)

```rust
use bevy::prelude::*;
use bevy::app::AppExit;  // <-- Add this line
use bevy::time::common_conditions::on_timer;
```

#### 8. Add MessageReader import
**File**: `crates/server/src/map.rs`
**Changes**: Add to imports

```rust
use bevy::ecs::event::MessageReader;  // <-- Add this line
```

### Success Criteria:

#### Automated Verification:
- [x] Server builds and runs: `cargo server`
- [x] Client can connect: `cargo client -c 1`
- [x] All tests pass: `cargo test-all`

#### Manual Verification:
- [x] Place voxel, wait 1 second, verify save file created
- [x] Place voxel, place another within 1 second, verify only saves after 1s of idle
- [x] Edit continuously for 6 seconds, verify save happens within 5 seconds of first edit
- [x] Place voxel, gracefully stop server (Ctrl+C), verify save happens on shutdown
- [x] Load and verify saved voxels appear in-game

---

## Phase 4: Integration Tests

### Overview
Create automated integration tests that validate core persistence functionality without requiring full client/server setup.

### Changes Required:

#### 1. Export test infrastructure
**File**: `crates/server/src/map.rs`
**Changes**: Make necessary types and functions public for testing

```rust
// Change VoxelModifications visibility
#[derive(Resource, Default)]
pub struct VoxelModifications {  // <-- Add pub
    pub modifications: Vec<(IVec3, VoxelType)>,  // <-- Add pub
}

// Change VoxelDirtyState visibility
#[derive(Resource)]
pub struct VoxelDirtyState {  // <-- Add pub
    pub is_dirty: bool,  // <-- Add pub
    pub last_edit_time: f64,  // <-- Add pub
    pub first_dirty_time: Option<f64>,  // <-- Add pub
}

// Make save/load functions public
pub fn save_voxel_world_to_disk(  // <-- Add pub
    modifications: &[(IVec3, VoxelType)],
    map_world: &MapWorld,
) -> std::io::Result<()> {
    // ... existing implementation
}

pub fn load_voxel_world_from_disk(  // <-- Add pub
    map_world: &MapWorld,
) -> Vec<(IVec3, VoxelType)> {
    // ... existing implementation
}
```

#### 2. Create integration test file
**File**: `crates/server/tests/voxel_persistence.rs`
**Changes**: Create new integration test file

**Test structure**:
```rust
use bevy::app::AppExit;
use bevy::prelude::*;
use bevy_voxel_world::prelude::*;
use protocol::MapWorld;
use server::map::{save_voxel_world_to_disk, load_voxel_world_from_disk,
                   ServerMapPlugin, VoxelModifications, VoxelDirtyState};
use std::fs;
use std::path::Path;

// Helper: Create test app with ServerMapPlugin
fn create_test_app() -> App { /* ... */ }

// Helper: Add voxels to world and modifications (simulating edits)
fn add_test_voxels(app: &mut App, voxels: &[(IVec3, u8)]) { /* ... */ }
```

**Tests to implement**:

1. **`test_save_load_cycle`**: Validates basic persistence
   - Create app, add 3 test voxels
   - Call `save_voxel_world_to_disk()` directly
   - Verify save file exists
   - Create new app, trigger startup (loads save file)
   - Verify all 3 voxels loaded in VoxelModifications
   - Assert specific positions and materials match

2. **`test_corrupt_file_recovery`**: Validates error handling
   - Write corrupt data to save file: `fs::write(save_file, b"corrupt data")`
   - Create app (triggers load)
   - Verify backup file created: `world_save/voxel_world.bin.corrupt`
   - Verify VoxelModifications is empty (clean start)
   - Verify no panic or crash

3. **`test_generation_metadata_mismatch`**: Validates seed/version checking
   - Phase 1: Create app with default MapWorld (seed=0, version=1)
   - Add voxels and save
   - Phase 2: Create app with custom MapWorld (seed=999, version=1)
   - Trigger load (should reject due to seed mismatch)
   - Verify VoxelModifications is empty
   - Repeat with mismatched generation_version

4. **`test_shutdown_save`**: Validates AppExit save trigger
   - Create app, add voxels
   - Set VoxelDirtyState.is_dirty = true manually
   - Send `AppExit::Success` event
   - Run update (triggers Last schedule with save system)
   - Verify save file created
   - Verify file contains saved voxels

**Note**: Debounced save timing is not tested in integration tests. That requires precise time control and is validated via manual testing in Phase 5.

### Success Criteria:

#### Automated Verification:
- [x] Tests compile: `cargo test --package server --test voxel_persistence --no-run`
- [x] All tests pass: `cargo test --package server --test voxel_persistence`
- [x] Tests clean up temp files (no leftover world_save/ directories)

#### Manual Verification:
- [x] Test output shows clear pass/fail for each scenario
- [x] Corrupt file test creates and cleans up backup file
- [x] Tests run in isolation (can run individually)

---

## Phase 5: Manual Testing & Verification

### Overview
Comprehensive manual testing of all persistence scenarios in live server/client environment.

### Testing Procedures:

#### 1. Basic save/load cycle
**Steps**:
1. Start server: `cargo server`
2. Connect client: `cargo client -c 1`
3. Place several voxels at different positions
4. Wait 2 seconds for debounced save
5. Verify `world_save/voxel_world.bin` exists
6. Stop server (Ctrl+C)
7. Restart server: `cargo server`
8. Connect client: `cargo client -c 1`
9. Verify all placed voxels are present

#### 2. Debounce timing verification
**Steps**:
1. Start server and client
2. Place voxel, immediately check file modification time
3. Wait exactly 1 second
4. Verify file was saved (check modification time changed)
5. Place another voxel
6. Wait 0.5 seconds, place another voxel
7. Wait 0.5 seconds, place another voxel (3 edits within 1.5s)
8. Wait 1 second after last edit
9. Verify file saved only once after last edit

#### 3. Max dirty duration verification
**Steps**:
1. Start server and client
2. Note current time
3. Place voxel every 0.8 seconds (keep editing continuously)
4. After 5 seconds of editing, verify save happened
5. Verify file was saved before 6 seconds elapsed

#### 4. Corrupt file recovery
**Steps**:
1. Stop server
2. Create corrupt file: `echo "corrupt data" > world_save/voxel_world.bin`
3. Start server: `cargo server`
4. Verify server logs show "Error deserializing save file"
5. Verify server logs show "Backed up corrupt file to world_save/voxel_world.bin.corrupt"
6. Verify server starts with empty world
7. Verify backup file exists: `ls world_save/voxel_world.bin.corrupt`

#### 5. Multi-client consistency
**Steps**:
1. Start server: `cargo server`
2. Connect first client: `cargo client -c 1`
3. Place voxels
4. Connect second client: `cargo client -c 2`
5. Verify second client sees first client's voxels
6. Wait for save, restart server
7. Connect both clients again
8. Verify both clients see saved voxels

#### 6. Generation metadata validation
**Steps**:
1. Start server and place voxels, wait for save
2. Stop server
3. Edit `crates/protocol/src/map.rs` to change MapWorld default seed to 999
4. Restart server: `cargo server`
5. Verify server logs show "WARNING: Save file generation seed mismatch"
6. Verify server logs show "Starting with empty world to avoid inconsistencies"
7. Verify server starts with clean world (no loaded modifications)
8. Revert MapWorld seed back to 0

### Success Criteria:

#### Automated Verification:
- [x] All tests pass: `cargo test-all`
- [x] Server builds: `cargo server`
- [x] Client builds: `cargo client -c 1`

#### Manual Verification:
- [x] Basic save/load cycle works correctly
- [x] Debounce waits 1 second after last edit
- [x] Max dirty duration enforces 5-second save
- [x] Corrupt file recovery creates backup and starts clean
- [x] Multiple clients see consistent saved state
- [x] Generation metadata mismatch rejects save file and starts clean
- [x] No performance degradation during saves
- [x] Server logs clearly indicate save/load operations

---

## Performance Considerations

**File I/O Impact**:
- Debounced saves minimize I/O frequency
- Binary format (bincode) provides fast serialization
- Atomic writes (temp + rename) ensure consistency
- Blocking I/O acceptable for current scale (< 1MB save files expected)

**Memory Impact**:
- VoxelModifications already stores all modifications in memory
- Save operation clones Vec (temporary allocation)
- Load operation replaces Vec (single allocation)
- No additional persistent memory overhead

**Network Impact**:
- No change to existing network protocol
- Initial sync still sends full VoxelStateSync to new clients
- Saved modifications seamlessly integrate with existing flow

---

## Migration Notes

Not applicable - this is the first persistence implementation. No existing save files to migrate.

**Future Procedural Generation Changes**: When modifying the procedural generation algorithm:
1. Update `MapWorld::default()` generation_version number in `crates/protocol/src/map.rs`
2. Update seed if changing base terrain parameters
3. Incompatible saves will be automatically rejected with clear warnings
4. Consider implementing migration logic if you need to preserve old saves
5. Both server and client will automatically use the updated MapWorld configuration

---

## References

- Original research: `doc/research/2026-01-17-voxel-world-save-load.md`
- VoxelModifications resource: `crates/server/src/map.rs:31-35`
- Voxel edit handling: `crates/server/src/map.rs:36-103`
- Initial state sync: `crates/server/src/map.rs:105-118`
- VoxelType definition: `crates/protocol/src/map.rs:36-59`
- Bevy shutdown research: `doc/research/2026-01-17-bevy-shutdown-handling.md`
