# Implementation Plan

## Overview

Refactor input routing into two explicit paths:

1. **Networked gameplay transport**: movement, camera yaw, jump, ability activation, and active ability input effects are first read through a client-local Leafwing `ActionState<RawClientActions>`, ownership-filtered, then copied into `ActionState<NetworkedPlayerActions>` before `lightyear_inputs::client::InputSystems::BufferClientInputs` in `FixedPreUpdate`. Predicted clients and the server continue to simulate from the same buffered Lightyear/Leafwing network input.
2. **Client-local command ownership**: terrain gestures, world-object tools, and dev/editor hotkeys are read through client-local Leafwing `ActionState<RawClientActions>`, gated by ownership locally, then translated into the existing authoritative request messages or UI state changes.

`PlayerActions` is renamed to `NetworkedPlayerActions` to make its transport role explicit. Keep ownership-sensitive physical bindings in a separate client-only `RawClientActions` Leafwing input map, then translate permitted actions into the Lightyear-buffered `NetworkedPlayerActions` or local command intents. Do not add new production `ButtonInput::just_pressed` command paths, and do not bind ownership-sensitive physical inputs directly to `InputMap<NetworkedPlayerActions>` except as a temporary migration fallback being removed in the same phase.

## Phase 1: Ownership Snapshot Filters Networked Ability Slots

### Goals

- Rename `PlayerActions` to `NetworkedPlayerActions` across protocol/client/server/tests.
- Add the client input module, ownership snapshot, and client-only `RawClientActions` input vocabulary.
- Move ability hotkey physical bindings from the networked `InputMap<NetworkedPlayerActions>` to a local `InputMap<RawClientActions>`.
- Translate filtered ability button state from `ActionState<RawClientActions>` into `ActionState<NetworkedPlayerActions>` before Lightyear buffers input.
- Keep ability activation validation unchanged, but feed it filtered networked input.

### Changes

#### 1. Rename networked action enum

**File**: `crates/protocol/src/lib.rs`  
**Action**: modify

Rename:

```rust
pub enum PlayerActions { ... }
```

to:

```rust
/// Lightyear/Leafwing network input vocabulary for predicted player simulation.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy, Hash, Reflect)]
pub enum NetworkedPlayerActions {
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
```

Update the `Actionlike` impl and `ProtocolPlugin`:

```rust
impl Actionlike for NetworkedPlayerActions { ... }

app.add_plugins(InputPlugin::<NetworkedPlayerActions> {
    config: InputConfig::<NetworkedPlayerActions> { ... },
});
```

Update all imports/usages of `PlayerActions` in protocol, client, server, assets/tests to `NetworkedPlayerActions`. Do not use `NetworkedPlayerActions` as the physical binding vocabulary for ownership-sensitive client input.

#### 2. Client input module export

**File**: `crates/client/src/lib.rs`  
**Action**: modify

```rust
pub mod input;
```

#### 3. Schedule labels

**File**: `crates/client/src/input/schedule.rs`  
**Action**: create

```rust
use bevy::prelude::*;

/// Ordered client input routing stages.
#[derive(SystemSet, Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ClientInputSet {
    Capture,
    WriteTransport,
    ProduceLocalCommands,
    Consume,
}
```

`WriteTransport` must run in `FixedPreUpdate` before `lightyear_inputs::client::InputSystems::BufferClientInputs`.

#### 4. Ownership snapshot

**File**: `crates/client/src/input/ownership.rs`  
**Action**: create

```rust
use bevy::prelude::*;

/// Client-local input ownership visible to fixed-tick transport writers.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientInputOwnershipSnapshot {
    pub keyboard: KeyboardInputOwner,
    pub pointer: PointerInputOwner,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyboardInputOwner {
    #[default]
    Gameplay,
    Ui,
    Text,
}

impl KeyboardInputOwner {
    pub fn allows_ability_commands(self) -> bool {
        matches!(self, Self::Gameplay)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerInputOwner {
    #[default]
    World,
    Ui,
}
```

If ownership is first computed in an `Update` UI/dev pass, add a copied fixed-tick resource or guarantee the same resource is updated before fixed transport writers. Do not let fixed writers depend on stale ownership.

Implementation note: egui text focus must be captured into this snapshot before transport writers run. Use `bevy_egui::input::EguiWantsInput` in `ClientInputSet::Capture` to set `KeyboardInputOwner::Text` while egui wants keyboard input; otherwise the default `Gameplay` owner lets `1`-`4` leak into `NetworkedPlayerActions` and Lightyear buffering while typing in text fields.

#### 5. Client-only raw action vocabulary

**File**: `crates/client/src/input/raw.rs`  
**Action**: create

```rust
use bevy::prelude::*;
use leafwing_input_manager::prelude::*;

/// Client-local physical input vocabulary populated by Leafwing before ownership filtering.
#[derive(Actionlike, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum RawClientActions {
    Move,
    CameraYaw,
    Jump,
    Ability1,
    Ability2,
    Ability3,
    Ability4,
    PlaceVoxel,
    RemoveVoxel,
    Delete,
}

pub fn raw_client_input_map() -> InputMap<RawClientActions> {
    InputMap::default()
        .with(RawClientActions::Ability1, KeyCode::Digit1)
        .with(RawClientActions::Ability2, KeyCode::Digit2)
        .with(RawClientActions::Ability3, KeyCode::Digit3)
        .with(RawClientActions::Ability4, KeyCode::Digit4)
        // Move existing WASD/gamepad/mouse/delete bindings here as phases migrate them.
}
```

Register this with a normal client-side Leafwing input manager, not Lightyear's network input plugin, and attach `ActionState<RawClientActions>` / `InputMap<RawClientActions>` to the locally controlled player input entity. `RawClientActions` is never buffered, replicated, or read by shared protocol simulation.

#### 6. Client input plugin

**File**: `crates/client/src/input/mod.rs`  
**Action**: create

```rust
//! Client-local ownership and command routing.

pub mod ability;
pub mod ownership;
pub mod raw;
pub mod schedule;

use bevy::prelude::*;
use leafwing_input_manager::prelude::InputManagerPlugin;
use lightyear_inputs::client::InputSystems;

use self::ability::write_filtered_ability_actions;
use self::ownership::ClientInputOwnershipSnapshot;
use self::raw::RawClientActions;
use self::schedule::ClientInputSet;

/// Routes physical client input into ownership-filtered network/local commands.
pub struct ClientInputCommandPlugin;

impl Plugin for ClientInputCommandPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(InputManagerPlugin::<RawClientActions>::default())
            .init_resource::<ClientInputOwnershipSnapshot>()
            .configure_sets(
                FixedPreUpdate,
                (ClientInputSet::Capture, ClientInputSet::WriteTransport)
                    .chain()
                    .before(InputSystems::BufferClientInputs),
            )
            .add_systems(
                FixedPreUpdate,
                write_filtered_ability_actions
                    .in_set(ClientInputSet::WriteTransport)
                    .before(InputSystems::BufferClientInputs),
            );
    }
}
```

Wire `ClientInputCommandPlugin` into the client app near `ClientGameplayPlugin`.

#### 7. Filtered ability transport writer

**File**: `crates/client/src/input/ability.rs`  
**Action**: create

```rust
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::Controlled;
use protocol::NetworkedPlayerActions;

use super::ownership::ClientInputOwnershipSnapshot;
use super::raw::RawClientActions;

const ABILITY_ACTION_MAP: [(RawClientActions, NetworkedPlayerActions); 4] = [
    (RawClientActions::Ability1, NetworkedPlayerActions::Ability1),
    (RawClientActions::Ability2, NetworkedPlayerActions::Ability2),
    (RawClientActions::Ability3, NetworkedPlayerActions::Ability3),
    (RawClientActions::Ability4, NetworkedPlayerActions::Ability4),
];

/// Copies ownership-filtered raw ability slot buttons into the networked input state.
pub fn write_filtered_ability_actions(
    ownership: Res<ClientInputOwnershipSnapshot>,
    mut query: Query<(
        &ActionState<RawClientActions>,
        &mut ActionState<NetworkedPlayerActions>,
    ), With<Controlled>>,
) {
    for (raw_actions, mut networked_actions) in &mut query {
        for (_, networked_action) in ABILITY_ACTION_MAP {
            networked_actions.release(&networked_action);
        }

        if !ownership.keyboard.allows_ability_commands() {
            trace!(owner = ?ownership.keyboard, "ability keyboard input suppressed");
            continue;
        }

        for (raw_action, networked_action) in ABILITY_ACTION_MAP {
            if raw_actions.just_pressed(&raw_action) {
                networked_actions.press(&networked_action);
            }
        }
    }
}
```

Use the actual Leafwing API available in this repo if `release`/`press` signatures differ. Clearing networked actions first is intentional because ability keys are bound only to `RawClientActions`, then copied into `NetworkedPlayerActions` after ownership filtering.

#### 8. Move ability hotkey bindings to raw input

**File**: `crates/client/src/gameplay.rs`  
**Action**: modify

Remove these physical bindings from the controlled entity `InputMap<NetworkedPlayerActions>`:

```rust
.with(NetworkedPlayerActions::Ability1, KeyCode::Digit1)
.with(NetworkedPlayerActions::Ability2, KeyCode::Digit2)
.with(NetworkedPlayerActions::Ability3, KeyCode::Digit3)
.with(NetworkedPlayerActions::Ability4, KeyCode::Digit4)
```

Bind those keys in the client-only `InputMap<RawClientActions>` instead. Keep the `ActionState<NetworkedPlayerActions>` / `InputMap<NetworkedPlayerActions>` marker required by Lightyear, but do not bind ownership-sensitive physical inputs to it.

#### 9. Ability activation imports/types

**Files**: `crates/protocol/src/ability/activation.rs`, `crates/protocol/src/ability/mod.rs`, `crates/protocol/tests/ability_systems.rs`  
**Action**: modify

Update `ActionState<PlayerActions>` to `ActionState<NetworkedPlayerActions>`. Keep existing slot helpers, renamed:

```rust
const ABILITY_ACTIONS: [NetworkedPlayerActions; 5] = [
    NetworkedPlayerActions::Ability1,
    NetworkedPlayerActions::Ability2,
    NetworkedPlayerActions::Ability3,
    NetworkedPlayerActions::Ability4,
    NetworkedPlayerActions::Jump,
];
```

Do not introduce a non-networked `AbilityInputState` component for activation.

#### 10. README

**File**: `README.md`  
**Action**: modify

Document that `1`-`4` first populate client-local `RawClientActions`, then are ownership-filtered before entering Lightyear input buffering; UI/text keyboard ownership suppresses ability activation.

### Regression tests

**File**: `crates/client/tests/input_commands.rs`  
**Action**: create

Add integration-style tests:

- `client_input_plugin_initializes_command_resources`
- `raw_ability_hotkey_populates_raw_client_action`
- `ability_hotkey_writes_networked_action_when_keyboard_owned_by_gameplay`
- `ability_does_not_fire_when_ui_owns_keyboard`
- `ability_does_not_fire_when_text_owns_keyboard`
- `ability_input_filter_runs_before_lightyear_buffers_inputs`
- `egui_keyboard_focus_captures_text_ownership`
- `egui_pointer_focus_captures_ui_pointer_ownership`

Protocol ability tests must continue proving slot lookup, asset lookup, cooldown refusal, duplicate active refusal, and grounded/airborne validation using `ActionState<NetworkedPlayerActions>`.

### Verification

- [x] Before each cargo command: `pgrep -af 'cargo (build|check|test|make)'`; wait or kill existing build/check/test.
- [x] `cargo test -p protocol ability_systems`
- [x] `cargo test -p client input_commands`
- [x] Manual: `1`-`4` activate only when gameplay owns keyboard and still travel through Lightyear input buffering.
  - Initial manual test found `1`-`4` still fired while typing in egui text input because the ownership snapshot was not populated from egui focus.
  - Fixed by capturing `EguiWantsInput` in `ClientInputSet::Capture` before ability filtering and Lightyear input buffering.
  - User retested and confirmed `1`-`4` do not fire abilities when typing into an input.

---

## Phase 2: Semantic Ability Asset Input Maps to Networked Transport

### Goals

- Remove concrete network action names from ability asset `OnInput` schema.
- Keep runtime observation on `ActionState<NetworkedPlayerActions>`.
- Map semantic `AbilityInput` to networked actions inside shared ability code.

### Changes

#### 1. Semantic ability input type

**File**: `crates/protocol/src/ability/input.rs`  
**Action**: create

```rust
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::NetworkedPlayerActions;

/// Semantic ability-domain input used by ability assets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[type_path = "protocol::ability"]
pub enum AbilityInput {
    Slot(usize),
    Jump,
}

impl AbilityInput {
    pub fn to_networked_action(self) -> Option<NetworkedPlayerActions> {
        match self {
            Self::Slot(0) => Some(NetworkedPlayerActions::Ability1),
            Self::Slot(1) => Some(NetworkedPlayerActions::Ability2),
            Self::Slot(2) => Some(NetworkedPlayerActions::Ability3),
            Self::Slot(3) => Some(NetworkedPlayerActions::Ability4),
            Self::Slot(_) => None,
            Self::Jump => Some(NetworkedPlayerActions::Jump),
        }
    }
}
```

If jump is not an ability-bound `OnInput` trigger in current assets, remove `Jump` instead of adding unused schema.

#### 2. Exports and reflection

**Files**: `crates/protocol/src/ability/mod.rs`, `crates/protocol/src/ability/plugin.rs`, `crates/protocol/src/reflect_loader.rs`  
**Action**: modify

Export/register `AbilityInput`. Replace reflected schema extraction from `action` to `input`.

#### 3. Ability asset schema types

**File**: `crates/protocol/src/ability/types.rs`  
**Action**: modify

```rust
use super::input::AbilityInput;

pub enum EffectTrigger {
    // ...
    OnInput {
        input: AbilityInput,
        effect: AbilityEffect,
    },
}

pub struct InputEffect {
    pub input: AbilityInput,
    pub effect: AbilityEffect,
}
```

Remove `NetworkedPlayerActions` from ability asset schema types unless used for non-asset transport helpers.

#### 4. Active input effects

**File**: `crates/protocol/src/ability/effects.rs`  
**Action**: modify

`apply_on_input_effects` continues to query `ActionState<NetworkedPlayerActions>` on the caster. For each `InputEffect`, map semantic input to network action:

```rust
let Some(action) = input_effect.input.to_networked_action() else {
    trace!(input = ?input_effect.input, "OnInput skipped: semantic input has no network action");
    continue;
};
if !action_state.just_pressed(&action) {
    continue;
}
```

Preserve existing phase checks, target resolution, tick use, and effect application.

#### 5. Asset migration

**Files**: `assets/abilities/*.ability.ron` **Action**: modify

Replace concrete action fields:

```ron
OnInput(action: Ability1, effect: ...)
```

with semantic input fields:

```ron
OnInput(input: Slot(0), effect: ...)
```

Mapping: `Ability1..4 -> Slot(0)..Slot(3)`, `Jump -> Jump` if retained.

#### 6. README

**File**: `README.md`  
**Action**: modify

Document semantic `OnInput` schema and note that runtime maps semantic ability inputs onto filtered networked input.

### Regression tests

**File**: `crates/protocol/tests/ability_systems.rs`

- `active_input_effect_fires_from_semantic_ability_input`
- `active_input_effect_ignores_unmatched_ability_input`
- `abilities_asset_loads_after_on_input_schema_migration`
- Existing activation/effect tests prove activation and active input effects observe the same tick from `ActionState<NetworkedPlayerActions>`.

### Verification

- [x] Before each cargo command: `pgrep -af 'cargo (build|check|test|make)'`; wait or kill existing build/check/test.
- [x] `cargo test -p protocol ability_systems`
- [x] `cargo test -p protocol --test ability_systems`
- [x] `cargo test -p client input_commands`
- [x] `cargo test -p client --test input_commands`
- [x] `rg 'OnInput\([^\n]*action|action: Ability|\bPlayerActions\b' assets crates/protocol/src/ability crates/protocol/tests/ability_systems.rs` shows no stale concrete ability asset schema references.

---

## Phase 3: Terrain Brush Uses Latched Pointer Ownership

### Goals

- Add pointer owners and a latched pointer gesture resource with an explicit inactive state.
- Gate terrain brush and legacy voxel edit input through pointer ownership.
- Preserve existing terrain request/ack/reject protocols.

### Changes

#### 1. Pointer owners

**File**: `crates/client/src/input/ownership.rs`  
**Action**: modify

```rust
pub enum PointerInputOwner {
    #[default]
    World,
    Ui,
    TerrainBrush,
    WorldObject,
}

impl PointerInputOwner {
    pub fn allows_terrain(self) -> bool {
        matches!(self, Self::TerrainBrush)
    }

    pub fn allows_world_object(self) -> bool {
        matches!(self, Self::WorldObject)
    }
}
```

#### 2. Latched gesture state

**File**: `crates/client/src/input/gestures.rs`  
**Action**: create

```rust
use bevy::prelude::*;

use super::ownership::{ClientInputOwnershipSnapshot, PointerInputOwner};

/// Pointer owner latched for an active press/drag. `None` means no active gesture.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientPointerGestureState {
    pub owner: Option<PointerInputOwner>,
    pub active_button: Option<MouseButton>,
}

impl ClientPointerGestureState {
    pub fn effective_owner(&self, snapshot_owner: PointerInputOwner) -> PointerInputOwner {
        self.owner.unwrap_or(snapshot_owner)
    }

    pub fn clear(&mut self) {
        self.owner = None;
        self.active_button = None;
    }
}

pub fn update_pointer_ownership(
    buttons: Res<ButtonInput<MouseButton>>,
    snapshot: Res<ClientInputOwnershipSnapshot>,
    mut gesture: ResMut<ClientPointerGestureState>,
) {
    if gesture.owner.is_none() {
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            if buttons.just_pressed(button) {
                gesture.owner = Some(snapshot.pointer);
                gesture.active_button = Some(button);
                return;
            }
        }
    }

    let Some(button) = gesture.active_button else {
        return;
    };
    if buttons.just_released(button) {
        gesture.clear();
    }
}
```

Schedule `update_pointer_ownership` before terrain/world-object command producers.

#### 3. Local terrain command intents

**File**: `crates/client/src/input/editor.rs`  
**Action**: create/modify

```rust
use bevy::prelude::*;
use protocol::map::voxel::VoxelBrushEditRequest;

#[derive(Message, Clone, Debug, PartialEq)]
pub enum TerrainCommandIntent {
    BrushStroke(VoxelBrushEditRequest),
    LegacyVoxelEdit(VoxelEditRequest),
}
```

Only add concrete payloads that already exist in current request paths.

#### 4. Input plugin registration

**File**: `crates/client/src/input/mod.rs`  
**Action**: modify

Add modules/resources/events:

```rust
pub mod editor;
pub mod gestures;

app.init_resource::<ClientPointerGestureState>()
    .add_message::<TerrainCommandIntent>();
```

#### 5. Terrain map systems

**File**: `crates/client/src/map.rs`  
**Action**: modify

Before terrain brush or legacy voxel edit sends requests, gate by:

```rust
let pointer_owner = gesture.effective_owner(ownership.pointer);
if !pointer_owner.allows_terrain() {
    trace!(?pointer_owner, "terrain input suppressed by pointer ownership");
    return;
}
```

Keep `VoxelBrushEditRequest` / `VoxelEditRequest` contracts unchanged. Expected early-outs must include `trace!`.

#### 6. Editing mode helpers

**File**: `crates/dev/src/state.rs`  
**Action**: modify

Add helpers mapping edit mode to terrain/world-object pointer ownership without creating a dev -> client dependency cycle.

### Regression tests

**File**: `crates/client/tests/input_commands.rs`

- `pointer_press_over_ui_latches_ui_owner_until_release`
- `terrain_does_not_edit_when_ui_owns_pointer`
- `terrain_does_not_edit_when_world_object_owns_pointer`
- `pointer_press_over_terrain_latches_terrain_owner_until_release`
- `terrain_mode_primary_action_emits_only_terrain_intent`

Update `crates/client/tests/plugin.rs` terrain tests to seed/latch terrain ownership before expecting terrain requests.

No existing `crates/client/tests/plugin.rs` terrain request tests were present during Phase 3 implementation, so no plugin terrain test seeding was needed.

### Verification

- [x] Before each cargo command: `pgrep -af 'cargo (build|check|test|make)'`; wait or kill existing build/check/test.
- [x] `cargo test -p client --features spawn-panel input_commands`
- [x] `cargo test -p client --features spawn-panel --test input_commands`
- [x] `cargo test -p client --features spawn-panel plugin`
- [x] `cargo test -p client --features spawn-panel --test plugin`
- [x] Manual: UI-started drags do not sculpt; terrain-started drags stay terrain-owned until release; terrain mode does not place/select world objects. Verified that egui UI buttons such as Homebase still activate while their pointer input does not leak into terrain/world commands.

---

## Phase 4: World-Object Tools Use Explicit Command Ownership

### Goals

- Route world-object place/pick/move/rotate/delete through local command intents.
- Ensure terrain and world-object tools are mutually exclusive by pointer ownership/editing mode.
- Preserve existing server-authoritative request contracts.

### Changes

#### 1. World-object intents

**File**: `crates/client/src/input/editor.rs` **Action**: modify

```rust
#[derive(Message, Clone, Debug, PartialEq)]
pub enum WorldObjectCommandIntent {
    Place,
    Pick,
    Move,
    Rotate { yaw_delta: f32 },
    Delete,
}
```

#### 2. Message registration

**File**: `crates/client/src/input/mod.rs`  
**Action**: modify

```rust
app.add_message::<WorldObjectCommandIntent>();
```

#### 3. Editing helpers

**File**: `crates/dev/src/state.rs`  
**Action**: modify

Add `EditingMode` helpers such as:

```rust
pub fn wants_terrain_pointer(self) -> bool { ... }
pub fn wants_world_object_pointer(self) -> bool { ... }
pub fn accepts_world_object_commands(self) -> bool { ... }
```

#### 4. World-object consumers

**File**: `crates/client/src/map.rs`  
**Action**: modify

Convert raw world-object input handling to consume `WorldObjectCommandIntent`, then send existing request types:

- `WorldObjectPlacementRequest`
- `WorldObjectDeleteRequest`
- `WorldObjectMoveRequest`
- `WorldObjectRotateRequest`

Pointer-origin commands require:

```rust
if !gesture.effective_owner(ownership.pointer).allows_world_object() { trace!(...); return; }
```

Keyboard-origin delete requires permitted keyboard ownership. Do not change request payload schemas or server validation.

#### 5. Spawn panel bridge

**File**: `crates/client/src/input/editor.rs` **Action**: modify

Bridge spawn-panel flags to `WorldObjectCommandIntent` from the client input boundary so panel delete and keyboard delete share the same local command path without introducing a `dev` -> `client` dependency. UI may retain selection/arming state; execution should be intent-based.

### Regression tests

**File**: `crates/client/tests/input_commands.rs`

- `place_definition_mode_primary_action_emits_only_world_object_place_intent`
- `world_object_place_does_not_fire_when_ui_owns_pointer`
- `world_object_place_does_not_fire_when_terrain_owns_pointer`
- `select_edit_mode_primary_action_emits_pick_or_edit_intent_only`
- `world_object_keyboard_delete_does_not_fire_when_ui_or_text_owns_keyboard`
- `panel_delete_and_keyboard_delete_share_delete_intent`

Update client plugin tests to seed ownership/gesture state. Server placement/edit tests should need no behavior changes; update imports/helpers only if required.

### Verification

- [x] Before each cargo command: `pgrep -af 'cargo (build|check|test|make)'`; wait or kill existing build/check/test.
- [x] `cargo test -p client --features spawn-panel input_commands`
- [x] `cargo test -p client --features spawn-panel --test input_commands`
- [x] `cargo test -p client --features spawn-panel plugin`
- [x] `cargo test -p client --features spawn-panel --test plugin`
- [x] `cargo test -p server world_object`
- [x] Manual: place/edit tools emit only world-object requests; UI-owned pointer input does not leak into world clicks.
  - After migrating terrain/world-object pointer and Delete inputs to Leafwing `ActionState<RawClientActions>`, the user validated the phase with fixed-only `ClientInputCommandPlugin` scheduling.

---

## Phase 5: Locomotion and Camera Use Central Ownership

### Goals

- Move ownership-sensitive movement/camera/jump physical bindings into the client-only `InputMap<RawClientActions>`.
- Translate filtered movement, camera yaw, and jump from `ActionState<RawClientActions>` into `ActionState<NetworkedPlayerActions>` before Lightyear buffering.
- Preserve movement physics and network prediction/replication semantics.

### Changes

#### 1. Ownership helpers

**File**: `crates/client/src/input/ownership.rs`  
**Action**: modify

```rust
impl KeyboardInputOwner {
    pub fn allows_locomotion(self) -> bool {
        matches!(self, Self::Gameplay)
    }

    pub fn allows_jump(self) -> bool {
        matches!(self, Self::Gameplay)
    }
}

impl PointerInputOwner {
    pub fn allows_camera_control(self) -> bool {
        matches!(self, Self::World)
    }
}
```

Camera yaw is simulation-relevant in this project because movement uses it to rotate input direction.

#### 2. Control transport writer

**File**: `crates/client/src/input/control.rs`  
**Action**: create

```rust
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use lightyear::prelude::Controlled;
use protocol::NetworkedPlayerActions;

use super::ownership::ClientInputOwnershipSnapshot;
use super::raw::RawClientActions;

pub fn write_filtered_control_actions(
    ownership: Res<ClientInputOwnershipSnapshot>,
    mut query: Query<(
        &ActionState<RawClientActions>,
        &mut ActionState<NetworkedPlayerActions>,
    ), With<Controlled>>,
) {
    for (raw_actions, mut networked_actions) in &mut query {
        let movement = if ownership.keyboard.allows_locomotion() {
            raw_actions.axis_pair(&RawClientActions::Move)
        } else {
            trace!(owner = ?ownership.keyboard, "locomotion input suppressed");
            Vec2::ZERO
        };

        networked_actions.set_axis_pair(&NetworkedPlayerActions::Move, movement.clamp_length_max(1.0));

        if ownership.keyboard.allows_jump() && raw_actions.pressed(&RawClientActions::Jump) {
            networked_actions.press(&NetworkedPlayerActions::Jump);
        } else {
            networked_actions.release(&NetworkedPlayerActions::Jump);
        }

        // Copy CameraYaw from RawClientActions only when keyboard ownership permits.
    }
}
```

Use the exact Leafwing getter/setter API available in this repo. Fold `sync_camera_yaw_to_input` into this translation writer so camera yaw cannot bypass ownership.

#### 3. Schedule wiring

**File**: `crates/client/src/input/mod.rs`  
**Action**: modify

```rust
pub mod control;

app.add_systems(
    FixedPreUpdate,
    write_filtered_control_actions
        .in_set(ClientInputSet::WriteTransport)
        .before(InputSystems::BufferClientInputs),
);
```

#### 4. Gameplay input map cleanup

**File**: `crates/client/src/gameplay.rs`  
**Action**: modify

Move direct physical bindings for ownership-sensitive actions from `InputMap<NetworkedPlayerActions>` to `InputMap<RawClientActions>`:

- `VirtualDPad::wasd()` / gamepad movement if gamepad is ownership-sensitive by policy
- jump if keyboard/UI focus should suppress it
- camera yaw producer path, including render camera Q/E rotation in `crates/render/src/camera.rs`, represented through Leafwing raw actions rather than raw `ButtonInput`

Keep the `InputMap<NetworkedPlayerActions>` marker only if Lightyear requires it for local input target detection. Physical bindings may remain directly on `NetworkedPlayerActions` only for actions explicitly documented as safe to bypass ownership. Future Phase 5 implementation must not introduce new production raw Bevy `ButtonInput` command reads. Q/E typed into egui text inputs must not rotate the camera.

#### 5. Shared movement/diagnostics

**Files**: `crates/protocol/src/character/movement.rs`, `crates/protocol/src/diagnostics.rs`  
**Action**: modify imports only unless compile errors require more. Movement should continue reading `ActionState<NetworkedPlayerActions>`.

#### 6. README

**File**: `README.md`  
**Action**: modify

Document that UI/text keyboard ownership suppresses movement/jump/Q/E camera rotation/camera yaw consistently, matching WASD movement semantics.

### Regression tests

**File**: `crates/client/tests/input_commands.rs`

- `wasd_updates_raw_client_move_action`
- `wasd_updates_networked_move_action_when_keyboard_owned_by_gameplay`
- `wasd_does_not_update_networked_move_action_when_keyboard_owned_by_text`
- `wasd_does_not_update_networked_move_action_when_keyboard_owned_by_ui`
- `gamepad_movement_respects_keyboard_or_control_ownership_policy`
- `camera_rotation_updates_orbit_when_ownership_allows_camera_control`
- `camera_rotation_matches_wasd_and_ignores_pointer_ownership`
- `camera_rotation_does_not_fire_when_text_owns_keyboard`
- `camera_yaw_syncs_when_pointer_allows_camera_control`
- `camera_yaw_does_not_change_when_pointer_owned_by_ui`
- `camera_yaw_does_not_change_when_pointer_owned_by_editor_tool`
- `control_input_filter_runs_before_lightyear_buffers_inputs`

### Verification

- [x] Before each cargo command: `pgrep -af 'cargo (build|check|test|make)'`; wait or kill existing build/check/test.
- [x] `cargo test -p client --features spawn-panel --test input_commands`
- [x] `cargo test -p protocol character`
- [x] Manual: WASD/gamepad/camera still work normally with movement relative to camera yaw; typing `WASD` in UI/text does not move; typing `Q`/`E` in UI/text does not rotate camera. User verified.

---

## Phase 6: Dev Hotkeys and Scheduling Are Centralized

### Goals

- Finalize schedule ordering and remove remaining raw command polling outside approved producer boundaries.
- Gate dev hotkeys through keyboard ownership.
- Ensure docs/tests describe the split between networked transport and client-local commands.

### Changes

#### 1. Final schedule consolidation

**File**: `crates/client/src/input/mod.rs`  
**Action**: modify

Final ordering:

- `ClientInputSet::Capture`: ownership snapshot availability and pointer gesture latch.
- `ClientInputSet::WriteTransport`: ability/control/jump translators copy ownership-filtered `ActionState<RawClientActions>` into `ActionState<NetworkedPlayerActions>` before `InputSystems::BufferClientInputs`.
- `ClientInputSet::ProduceLocalCommands`: terrain/world-object/dev local command intents produced from Leafwing `ActionState<RawClientActions>`, not raw Bevy `ButtonInput`.
- `ClientInputSet::Consume`: map/dev consumers send existing request messages or mutate UI state.

#### 2. Dev hotkeys

**Files**: `crates/client/src/input/editor.rs`, `crates/dev/src/lib.rs`  
**Action**: modify

Add or gate dev hotkey producers with:

```rust
impl KeyboardInputOwner {
    pub fn allows_dev_hotkeys(self) -> bool {
        matches!(self, Self::Gameplay)
    }
}
```

`F3`/`F4`/`F5`/`F6` should not fire while UI/text owns keyboard input. Keep dev state in `dev`; avoid introducing a `dev` -> `client` dependency cycle.

#### 3. Raw polling cleanup

**Files**: `crates/client/src/gameplay.rs`, `crates/client/src/map.rs`, `crates/dev/src/panels/spawn.rs`, `crates/dev/src/panels/world_inspector.rs`  
**Action**: modify

Remove remaining raw gameplay/editor command polling, including inside `crates/client/src/input/` producers; production command producers should read Leafwing `ActionState<RawClientActions>` or filtered `ActionState<NetworkedPlayerActions>`, except documented low-level exceptions. Every expected early-out must use `trace!`.

#### 4. Diagnostics and README

**Files**: `crates/protocol/src/diagnostics.rs`, `README.md`  
**Action**: modify

Update names from `PlayerActions` to `NetworkedPlayerActions`. Document final behavior for ability hotkeys, semantic `OnInput`, terrain/world-object command ownership, movement/camera suppression, and dev hotkeys.

### Regression tests

**File**: `crates/client/tests/input_commands.rs`

- `client_input_sets_run_capture_before_write_transport_before_local_command_consume`
- `dev_hotkeys_fire_when_gameplay_owns_keyboard`
- `dev_hotkeys_do_not_fire_when_ui_owns_keyboard`
- `dev_hotkeys_do_not_fire_when_text_owns_keyboard`
- `networked_input_map_no_longer_triggers_ownership_sensitive_behavior_without_raw_to_networked_translators`

Use public resources/events plus both `ActionState<RawClientActions>` and `ActionState<NetworkedPlayerActions>` effects; avoid tests that only prove enum shape.

### Verification

- [x] `cargo check-all`
- [x] `cargo test -p client --features spawn-panel input_commands`
- [x] `cargo test-all` workspace/native tests passed; wasm-pack Firefox step timed out after compiling `web` without a Rust test failure.
- [x] `rg 'PlayerActions|Digit1|Digit2|Digit3|Digit4|VirtualDPad::wasd|PlaceVoxel|RemoveVoxel|MouseButton::Left|MouseButton::Right' crates/client/src crates/dev/src crates/protocol/src/ability` reviewed: remaining matches are approved raw producers, networked transport/shared ability use, diagnostics, or terrain/world-object producers.
- [x] Manual: F3/F4/F5/F6, ability hotkeys, locomotion/camera, terrain brush, and world-object edit flows work with gameplay ownership and are suppressed under focused UI/text as documented. User verified.

## Cross-Phase Constraints

- Do not change terrain/world-object request/ack/reject protocol contracts without approval.
- Do not add production ability UI, rebinding UI, hotbars, radial menus, macros, new permissions, or new gameplay features.
- Client ownership gates are UX/input correctness, not authorization; server authority remains unchanged.
- Shared predicted/server gameplay systems must read buffered `ActionState<NetworkedPlayerActions>`, not current `ClientInputOwnershipSnapshot`, raw Bevy input, `RawClientActions`, or other client-local producer state.
- Use `trace!` for expected early-outs. Use `debug_assert!`, `expect`, or `panic!` for impossible invalid state.
- After code changes in each phase, review and update `README.md` if behavior/schema/docs changed.
