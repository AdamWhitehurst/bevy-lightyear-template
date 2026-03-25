# Camera Rotation (Q/E with Lerp) Implementation Plan

## Overview

Add 90-degree orbital camera rotation around the player on Q/E press with smooth visual lerping. Movement and facing become camera-relative by adding a `CameraYaw` single-axis input to `PlayerActions` — `apply_movement` and `update_facing` read it and rotate internally. Server receives the yaw via lightyear input replication and computes identically.

## Current State Analysis

- Camera: fixed offset `(0, 18, -36)`, snaps to player each frame (`crates/render/src/camera.rs:22-36`)
- Movement: world-space, `Vec3::new(-move_dir.x, 0.0, move_dir.y)` (`crates/protocol/src/character/movement.rs:54`)
- Facing: world-space, `atan2(move_dir.x, -move_dir.y)` (`crates/protocol/src/character/movement.rs:78`)
- Q/E: unbound
- Lighting: fixed position `(8, 16, 8)` (`crates/render/src/camera.rs:18`)
- Billboard systems read camera transform live — adapt automatically
- `Rotation` is predicted with rollback (`crates/protocol/src/lib.rs:190-194`)

### Key Discoveries:
- `ActionState::set_value()`/`value()` exist for `InputControlKind::Axis` — perfect for a single float
- Leafwing updates `ActionState` in `PreUpdate`
- Lightyear buffers inputs in `FixedPreUpdate` at `InputSystems::BufferClientInputs`
- Existing exponential lerp pattern: `(SPEED * dt).min(1.0)` (`crates/sprite_rig/src/animation.rs:70-88`)

## Desired End State

- Q rotates camera 90° counter-clockwise around player, E rotates clockwise
- Four discrete positions (0°, 90°, 180°, 270°), smooth visual lerp between them
- WASD movement is camera-relative (W = toward camera "forward")
- Character faces camera-relative direction
- Lighting follows camera rotation
- Movement snaps to new orientation immediately on Q/E press (uses `target_angle`), camera visually catches up
- Server receives `CameraYaw` via input replication — prediction identical on both sides

### Verification:
- Press Q/E: camera smoothly orbits to next 90° position
- During/after rotation, WASD moves character relative to camera facing
- Character faces movement direction relative to camera
- Lighting rotates with camera
- Multiplayer: no rollback fighting, server movement matches client

## What We're NOT Doing

- Free-rotation (mouse orbit) — discrete 90° only
- Camera zoom
- Camera tilt adjustment

## Implementation Approach

**CameraYaw as replicated input**: Add `CameraYaw` variant to `PlayerActions` with `InputControlKind::Axis`. Client writes `CameraOrbitState.target_angle` into it each frame via `set_value()`. Both `apply_movement` and `update_facing` read it via `value()` and rotate the movement/facing internally. No `ActionState` mutation of the `Move` axis — rotation is applied inside the shared protocol systems.

Using `target_angle` (discrete 0/π/2/π/3π/2) rather than `current_angle` (lerped visual value) ensures deterministic, stable values with no floating-point divergence between client and server.

## Phase 1: Camera Orbit State & Lerped Follow

### Overview
Add `CameraOrbitState` component to the camera entity. Modify `follow_player` to rotate the offset vector by the current (lerped) angle.

### Changes Required:

#### 1. `CameraOrbitState` component
**File**: `crates/render/src/camera.rs`

```rust
/// Orbital camera state for discrete 90° rotation around the player.
#[derive(Component)]
pub struct CameraOrbitState {
    /// Target angle in radians (one of 0, π/2, π, 3π/2)
    pub target_angle: f32,
    /// Current angle in radians (lerps toward target)
    pub current_angle: f32,
}

impl Default for CameraOrbitState {
    fn default() -> Self {
        Self {
            target_angle: 0.0,
            current_angle: 0.0,
        }
    }
}
```

#### 2. Spawn camera with `CameraOrbitState`
**File**: `crates/render/src/camera.rs`
**Change**: Add `CameraOrbitState::default()` to the camera spawn bundle.

```rust
pub(crate) fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 18.0, -36.0).looking_at(Vec3::ZERO, Dir3::Y),
        CameraOrbitState::default(),
    ));
}
```

#### 3. Modify `follow_player` to use rotated offset
**File**: `crates/render/src/camera.rs`

```rust
const BASE_OFFSET: Vec3 = Vec3::new(0.0, 18.0, -36.0);

pub(crate) fn follow_player(
    player_query: Query<&Position, With<Controlled>>,
    mut camera_query: Query<(&mut Transform, &CameraOrbitState), With<Camera3d>>,
) {
    let Ok(player_pos) = player_query.single() else {
        return;
    };
    let Ok((mut camera_transform, orbit)) = camera_query.single_mut() else {
        return;
    };

    let rotated_offset = Quat::from_rotation_y(orbit.current_angle) * BASE_OFFSET;
    camera_transform.translation = **player_pos + rotated_offset;
    camera_transform.look_at(**player_pos, Dir3::Y);
}
```

#### 4. Lerp system for orbit angle
**File**: `crates/render/src/camera.rs`

```rust
const ORBIT_LERP_SPEED: f32 = 20.0;

/// Lerps camera orbit angle toward the target using frame-rate-independent exponential approach.
fn update_camera_orbit(time: Res<Time>, mut query: Query<&mut CameraOrbitState>) {
    let dt = time.delta_secs();
    let lerp_factor = (ORBIT_LERP_SPEED * dt).min(1.0);

    for mut orbit in &mut query {
        let diff = orbit.target_angle - orbit.current_angle;
        if diff.abs() > 0.001 {
            orbit.current_angle += diff * lerp_factor;
        } else {
            orbit.current_angle = orbit.target_angle;
        }
    }
}
```

#### 5. Register `update_camera_orbit`
**File**: `crates/render/src/lib.rs`
**Change**: Add `update_camera_orbit` to `Update`, before `follow_player`. Chain the update systems for correct ordering.

```rust
app.add_systems(
    Update,
    (
        camera::update_camera_orbit,
        camera::follow_player,
        health_bar::billboard_face_camera,
        health_bar::update_health_bars,
    )
        .chain(),
);
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all`

#### Manual Verification:
- [ ] Camera spawns at default position (unchanged from current behavior)
- [ ] No visual difference yet (orbit angle is 0)

---

## Phase 2: Q/E Input & CameraYaw PlayerAction

### Overview
Handle Q/E keypresses to cycle the camera's target angle by ±90°. Add `CameraYaw` to `PlayerActions` as a single-axis input. Client writes `target_angle` into it each frame.

### Changes Required:

#### 1. Add `CameraYaw` to `PlayerActions`
**File**: `crates/protocol/src/lib.rs`

```rust
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy, Hash, Reflect)]
pub enum PlayerActions {
    Move,
    CameraYaw,
    Jump,
    PlaceVoxel,
    RemoveVoxel,
    Ability1,
    Ability2,
    Ability3,
    Ability4,
}

impl Actionlike for PlayerActions {
    fn input_control_kind(&self) -> InputControlKind {
        match self {
            Self::Move => InputControlKind::DualAxis,
            Self::CameraYaw => InputControlKind::Axis,
            _ => InputControlKind::Button,
        }
    }
}
```

#### 2. Q/E input system
**File**: `crates/render/src/camera.rs`

```rust
/// Handles Q/E input to rotate camera orbit by 90° increments.
fn handle_camera_rotation_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut query: Query<&mut CameraOrbitState>,
) {
    let Ok(mut orbit) = query.single_mut() else {
        return;
    };

    if keys.just_pressed(KeyCode::KeyQ) {
        orbit.target_angle += std::f32::consts::FRAC_PI_2;
    }
    if keys.just_pressed(KeyCode::KeyE) {
        orbit.target_angle -= std::f32::consts::FRAC_PI_2;
    }
}
```

#### 3. Re-export `CameraOrbitState` from render crate
**File**: `crates/render/src/lib.rs`
**Change**: Add `pub use camera::CameraOrbitState;`

#### 4. Client system to write `CameraYaw` into `ActionState`
**File**: `crates/client/src/gameplay.rs`

```rust
use render::CameraOrbitState;

/// Writes the camera's target yaw angle into the player's ActionState for replication.
fn sync_camera_yaw_to_input(
    camera_query: Query<&CameraOrbitState>,
    mut player_query: Query<&mut ActionState<PlayerActions>, With<Predicted>>,
) {
    let Ok(orbit) = camera_query.single() else {
        return;
    };

    for mut action_state in &mut player_query {
        action_state.set_value(&PlayerActions::CameraYaw, orbit.target_angle);
    }
}
```

#### 5. Register systems
**File**: `crates/render/src/lib.rs`
**Change**: Add `handle_camera_rotation_input` before `update_camera_orbit` in the chain.

```rust
app.add_systems(
    Update,
    (
        camera::handle_camera_rotation_input,
        camera::update_camera_orbit,
        camera::follow_player,
        health_bar::billboard_face_camera,
        health_bar::update_health_bars,
    )
        .chain(),
);
```

**File**: `crates/client/src/gameplay.rs`
**Change**: Register `sync_camera_yaw_to_input` in `FixedPreUpdate` before lightyear buffers.

```rust
use lightyear::prelude::InputSystems;

// In ClientGameplayPlugin::build:
app.add_systems(
    FixedPreUpdate,
    sync_camera_yaw_to_input.before(InputSystems::BufferClientInputs),
);
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all`

#### Manual Verification:
- [ ] Q press: camera smoothly rotates 90° CCW around player
- [ ] E press: camera smoothly rotates 90° CW around player
- [ ] Multiple presses queue correctly (pressing Q twice → 180°)
- [ ] WASD movement still world-space (not yet camera-relative — that's Phase 3)

---

## Phase 3: Camera-Relative Movement & Facing

### Overview
Modify `apply_movement` and `update_facing` in the protocol crate to read `CameraYaw` from `ActionState` and rotate the movement direction and facing internally. Both client and server execute the same code with the same yaw value.

### Changes Required:

#### 1. Modify `apply_movement`
**File**: `crates/protocol/src/character/movement.rs`
**Change**: Read `CameraYaw` and rotate the movement direction.

```rust
// Horizontal movement
let move_dir = action_state
    .axis_pair(&PlayerActions::Move)
    .clamp_length_max(1.0);
let yaw = action_state.value(&PlayerActions::CameraYaw);
let move_dir = Quat::from_rotation_y(yaw) * Vec3::new(-move_dir.x, 0.0, move_dir.y);
```

#### 2. Modify `update_facing`
**File**: `crates/protocol/src/character/movement.rs`
**Change**: Add yaw offset to facing rotation.

```rust
pub fn update_facing(
    mut query: Query<(&ActionState<PlayerActions>, &mut Rotation), With<CharacterMarker>>,
) {
    for (action_state, mut rotation) in &mut query {
        let move_dir = action_state
            .axis_pair(&PlayerActions::Move)
            .clamp_length_max(1.0);
        if move_dir != Vec2::ZERO {
            let yaw = action_state.value(&PlayerActions::CameraYaw);
            *rotation = Rotation(Quat::from_rotation_y(
                f32::atan2(move_dir.x, -move_dir.y) + yaw,
            ));
        }
    }
}
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all`

#### Manual Verification:
- [ ] Default camera (0° rotation): WASD unchanged
- [ ] After Q press: W moves character "forward" relative to camera
- [ ] After two Q presses (180°): W moves toward what was previously "backward"
- [ ] Character faces movement direction correctly relative to camera
- [ ] Multiplayer: `cargo server` + `cargo client` — rotate camera, move — no excessive rollbacks, server matches client
- [ ] Movement snaps to new orientation on Q/E press, camera visually catches up

---

## Phase 4: Lighting Follows Camera

### Overview
Rotate the light position around the player to match camera rotation.

### Changes Required:

#### 1. Add marker component and modify lighting setup
**File**: `crates/render/src/camera.rs`

```rust
/// Marker for the main scene light that follows camera rotation.
#[derive(Component)]
pub struct MainLight;

const BASE_LIGHT_OFFSET: Vec3 = Vec3::new(8.0, 16.0, 8.0);

pub(crate) fn setup_lighting(mut commands: Commands) {
    commands.spawn((
        PointLight {
            shadows_enabled: true,
            ..default()
        },
        Transform::from_translation(BASE_LIGHT_OFFSET),
        MainLight,
    ));
}
```

#### 2. Light follow system
**File**: `crates/render/src/camera.rs`

```rust
/// Updates light position to follow camera rotation around the player.
fn update_light_position(
    player_query: Query<&Position, With<Controlled>>,
    camera_query: Query<&CameraOrbitState>,
    mut light_query: Query<&mut Transform, With<MainLight>>,
) {
    let Ok(player_pos) = player_query.single() else {
        return;
    };
    let Ok(orbit) = camera_query.single() else {
        return;
    };
    let Ok(mut light_transform) = light_query.single_mut() else {
        return;
    };

    let rotated_offset = Quat::from_rotation_y(orbit.current_angle) * BASE_LIGHT_OFFSET;
    light_transform.translation = **player_pos + rotated_offset;
}
```

#### 3. Register the system
**File**: `crates/render/src/lib.rs`
**Change**: Add `camera::update_light_position` after `camera::follow_player` in the chain.

```rust
app.add_systems(
    Update,
    (
        camera::handle_camera_rotation_input,
        camera::update_camera_orbit,
        camera::follow_player,
        camera::update_light_position,
        health_bar::billboard_face_camera,
        health_bar::update_health_bars,
    )
        .chain(),
);
```

### Success Criteria:

#### Automated Verification:
- [x] `cargo check-all`

#### Manual Verification:
- [ ] Light rotates with camera — shadows shift consistently with view angle
- [ ] No shadow popping or flickering during rotation

---

## Testing Strategy

### Manual Testing Steps:
1. `cargo server` then `cargo client` — verify default camera position unchanged
2. Press Q — camera orbits 90° CCW smoothly
3. Press E — camera orbits 90° CW smoothly
4. Press Q four times — full 360°, returns to original position
5. Hold W during Q press — movement snaps to new direction immediately, camera catches up visually
6. After rotation, verify WASD feels correct relative to camera
7. Verify character facing matches movement direction
8. Start second client — verify multiplayer prediction is clean (no jittering)
9. Check shadows rotate with camera

## Performance Considerations

- One `value()` read + `Quat::from_rotation_y` + quaternion multiply per entity per `FixedUpdate` tick — negligible
- One `sin_cos` for lerp per frame — negligible
- `CameraYaw` adds one f32 per tick to lightyear input replication — negligible bandwidth

## References

- Research: `doc/research/2026-03-21-camera-rotation-q-e-lerp.md`
- Camera system: `crates/render/src/camera.rs`
- Movement: `crates/protocol/src/character/movement.rs`
- Client gameplay: `crates/client/src/gameplay.rs`
- PlayerActions: `crates/protocol/src/lib.rs:54-73`
- Existing lerp pattern: `crates/sprite_rig/src/animation.rs:70-88`
