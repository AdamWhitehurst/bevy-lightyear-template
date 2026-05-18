# Structure Outline

## Approach

Introduce a client-local input command module that builds a `ClientInputOwnershipSnapshot` before gameplay/editor input is emitted. For fixed-tick networked input, the ownership snapshot must be computed or copied into fixed-tick-accessible state before transport writers run in `FixedPreUpdate`. Separate two paths explicitly:

- **Networked gameplay input transport**: movement, camera yaw, jump, ability activation, and active ability input effects must enter the Lightyear/Leafwing `ActionState` before `lightyear_inputs::client::InputSystems::BufferClientInputs`, so predicted client and server simulation read the same buffered tick input.
- **Client-local command ownership**: terrain brush gestures, world-object tool intents, and dev UI/panel hotkeys are gated locally, then translated into existing authoritative request messages where applicable.

Keep client ownership, gesture, schedule, control-input, and editor command routing in `crates/client/src/input/`. Rename `PlayerActions` to `NetworkedPlayerActions` to make the transport role explicit. Add a client-only `RawClientActions` Leafwing input vocabulary for physical bindings, then map semantic ability/control inputs from filtered `ActionState<RawClientActions>` into `ActionState<NetworkedPlayerActions>` unless a later phase explicitly migrates to a new Lightyear input transport type.

Local source facts checked: `lightyear_inputs::client::InputSystems::{WriteClientInputs, BufferClientInputs}` are chained in `FixedPreUpdate`; Lightyear buffers `ActionState` into `InputBuffer` in `BufferClientInputs`; the Leafwing adapter adds `InputManagerPlugin` and restores inputs before Leafwing's fixed tick; Leafwing updates mapped `ActionState` from `InputMap` before the fixed loop. Therefore ownership-sensitive physical bindings should live on `InputMap<RawClientActions>`, not directly on `InputMap<NetworkedPlayerActions>`. Post-hoc overwriting of networked actions is only acceptable as a temporary migration step because it depends on subtle ordering against Leafwing internals.

## Phase 1: Ownership Snapshot Filters Networked Ability Slots

Add the client input ownership snapshot and route keyboard ability-slot presses into the Lightyear/Leafwing input transport. Ability activation keeps the existing validation path, but reads filtered `ActionState<NetworkedPlayerActions>` on the shared predicted/server simulation path instead of a client-local ability intent component.

**Files**: `crates/client/src/input/mod.rs`, `crates/client/src/input/ownership.rs`, `crates/client/src/input/schedule.rs`, `crates/client/src/input/ability.rs`, `crates/client/src/gameplay.rs`, `crates/client/tests/input_commands.rs`, `crates/protocol/src/lib.rs`, `crates/protocol/src/ability/input.rs`, `crates/protocol/src/ability/activation.rs`, `crates/protocol/tests/ability_systems.rs`, `README.md`

**Key changes**:

- `PlayerActions` renamed to `NetworkedPlayerActions` — explicit Lightyear/Leafwing network input vocabulary
- `ClientInputCommandPlugin` — new client plugin that owns capture and transport-write scheduling
- `ClientInputOwnershipSnapshot { keyboard: KeyboardInputOwner, pointer: PointerInputOwner }` — new client-local resource computed or copied before fixed transport writers run
- `KeyboardInputOwner::{Gameplay, Ui, Text}` — new owner enum
- move direct ability hotkey bindings from `InputMap<NetworkedPlayerActions>` to client-only `InputMap<RawClientActions>`
- `write_filtered_ability_actions(ownership: Res<ClientInputOwnershipSnapshot>, query: Query<(&ActionState<RawClientActions>, &mut ActionState<NetworkedPlayerActions>), With<Controlled>>)` — copies ability button state into networked input only when gameplay owns keyboard
- schedule ownership snapshot availability and filtered action writers in `FixedPreUpdate` before `lightyear_inputs::client::InputSystems::BufferClientInputs`; place transport writers in/around `lightyear_inputs::client::InputSystems::WriteClientInputs`
- `activate_abilities(..., action_state: &ActionState<NetworkedPlayerActions>, ...)` — validation semantics unchanged; shared systems read buffered/predicted network input
- `crates/client/tests/input_commands.rs` — new integration-style regression test harness for plugin/system behavior, avoiding brittle helper-unit tests

**Regression tests**:

- `client_input_plugin_initializes_command_resources` — adding `ClientInputCommandPlugin` to a minimal `App` initializes ownership and schedule-facing resources without requiring gameplay/map plugins.
- `raw_ability_hotkey_populates_raw_client_action` — pressing `Digit1` sets `RawClientActions::Ability1` in client-local input state.
- `ability_hotkey_writes_networked_action_when_keyboard_owned_by_gameplay` — pressing `Digit1` with `KeyboardInputOwner::Gameplay` sets `NetworkedPlayerActions::Ability1` in `ActionState<NetworkedPlayerActions>` before buffering.
- `ability_does_not_fire_when_ui_owns_keyboard` — pressing `Digit1` with `KeyboardInputOwner::Ui` leaves the networked action unpressed before buffering and no ability activation occurs.
- `ability_does_not_fire_when_text_owns_keyboard` — pressing `Digit1` with `KeyboardInputOwner::Text` leaves the networked action unpressed before buffering and no ability activation occurs.
- `ability_input_filter_runs_before_lightyear_buffers_inputs` — schedule-order test proves ownership filtering happens before `lightyear_inputs::client::InputSystems::BufferClientInputs`.
- Existing protocol ability activation tests continue to prove slot lookup, cooldown, duplicate-active, asset lookup, and grounded/airborne validation still work from `ActionState<NetworkedPlayerActions>`.

**Verify**: Run `cargo test -p protocol ability_systems` and `cargo test -p client input_commands`; check manually that `1`-`4` still activate abilities when gameplay owns keyboard, do not activate while typing/focused UI owns keyboard, and still replicate/predict through Lightyear input buffering.

---

## Phase 2: Shared Ability Intent Drives Active-Phase Input Effects

Replace ability asset references to concrete network input actions with semantic ability inputs, but keep runtime activation/effect observation on the Lightyear-buffered network input path. Active-phase `OnInput` effects map `AbilityInput` to the filtered `ActionState<NetworkedPlayerActions>` for the current tick instead of reading a client-local ECS resource.

**Files**: `crates/protocol/src/ability/input.rs`, `crates/protocol/src/ability/types.rs`, `crates/protocol/src/ability/effects.rs`, `crates/protocol/src/ability/plugin.rs`, `crates/protocol/src/reflect_loader.rs`, `crates/protocol/tests/ability_systems.rs`, `crates/client/tests/input_commands.rs`, `assets/abilities.ron`, `README.md`

**Key changes**:

- `AbilityInput { Slot(usize), Jump }` — semantic trigger type for ability assets, if jump remains ability-bound
- `EffectTrigger::OnInput { input: AbilityInput, effect: AbilityEffect }` — modified schema
- `InputEffect { input: AbilityInput, effect: AbilityEffect }` — modified schema
- `AbilityInput::to_networked_action(self) -> Option<NetworkedPlayerActions>` — mapping from semantic ability input to transport action while `NetworkedPlayerActions` remains the packet vocabulary
- `apply_input_effects(..., action_query: Query<&ActionState<NetworkedPlayerActions>>, ...)` — modified source; active effects observe the same buffered tick input as activation
- Explicit non-goal: do not introduce a separate networked ability input component/type in this phase unless the plan is expanded into a broader Lightyear transport migration.

**Regression tests**:

- `active_input_effect_fires_from_semantic_ability_input` — an active ability with `OnInput { input: AbilityInput::Slot(0), ... }` applies the effect when `NetworkedPlayerActions::Ability1` is pressed in the buffered/current tick action state.
- `active_input_effect_ignores_unmatched_ability_input` — the same active effect does not fire for a different networked ability action.
- `abilities_asset_loads_after_on_input_schema_migration` — load the real ability asset and prove migrated semantic `OnInput` data is accepted by the runtime asset path.
- Existing protocol ability tests cover that activation and active-phase effects observe the same tick from `ActionState<NetworkedPlayerActions>` without relying on client-local resources.

**Verify**: Run `cargo test -p protocol ability_systems` and `cargo test -p client input_commands`; check that combo/active `OnInput` ability effects fire from semantic asset input mapped onto `NetworkedPlayerActions`.

---

## Phase 3: Terrain Brush Uses Latched Pointer Ownership

Move terrain brush and legacy voxel edit input behind gesture-latched pointer ownership. Terrain mode primary action emits only terrain/voxel command intent, preserving existing `VoxelBrushEditRequest` and `VoxelEditRequest` network boundaries.

**Files**: `crates/client/src/input/ownership.rs`, `crates/client/src/input/gestures.rs`, `crates/client/src/input/editor.rs`, `crates/client/src/map.rs`, `crates/dev/src/state.rs`, `crates/client/tests/input_commands.rs`, `crates/client/tests/plugin.rs`, `README.md`

**Key changes**:

- `PointerInputOwner::{World, Ui, TerrainBrush, WorldObject}` — new/expanded pointer owner enum
- `ClientPointerGestureState { owner: Option<PointerInputOwner>, active_button: Option<MouseButton> }` — new latched gesture resource; `None` means no active press/drag and the current snapshot owner applies
- `update_pointer_ownership(...)` — new system scheduled before editor command producers
- `TerrainCommandIntent::{BrushStroke, LegacyVoxelEdit}` — new client-local command intent enum/event, if needed to decouple raw input from request sending
- `handle_terrain_brush_input(ownership: Res<ClientInputOwnershipSnapshot>, gesture: Res<ClientPointerGestureState>, ...)` — modified gate

**Regression tests**:

- `pointer_press_over_ui_latches_ui_owner_until_release` — press begins with UI pointer ownership, drag/release over world, and no terrain/world-object intent is emitted.
- `terrain_does_not_edit_when_ui_owns_pointer` — left click with `PointerInputOwner::Ui` emits no terrain brush or legacy voxel intent.
- `terrain_does_not_edit_when_world_object_owns_pointer` — left click with `PointerInputOwner::WorldObject` emits no terrain intent.
- `pointer_press_over_terrain_latches_terrain_owner_until_release` — press begins in Terrain mode over world, later UI hover does not steal the active brush gesture before release.
- `terrain_mode_primary_action_emits_only_terrain_intent` — left click in Terrain mode emits terrain/voxel intent and no world-object intent.
- Existing client plugin tests continue to prove terrain requests are sent through `VoxelBrushEditRequest` / `VoxelEditRequest` boundaries.

**Verify**: Run `cargo test -p client --features spawn-panel input_commands` and `cargo test -p client --features spawn-panel plugin`; manually confirm UI clicks/drags do not sculpt terrain, world clicks in Terrain mode still send brush requests, and Terrain mode does not place/select world objects.

---

## Phase 4: World-Object Tools Use Explicit Command Ownership

Route world-object placement, cursor pick, move, rotate, and delete through explicit world-object command ownership. Panel buttons and keyboard shortcuts produce the same local command intents before existing `WorldObject*Request` sends.

**Files**: `crates/client/src/input/editor.rs`, `crates/client/src/map.rs`, `crates/dev/src/panels/spawn.rs`, `crates/dev/src/state.rs`, `crates/client/tests/input_commands.rs`, `crates/client/tests/plugin.rs`, `crates/server/tests/world_object_placement.rs`, `crates/server/tests/world_object_edit.rs`, `README.md`

**Key changes**:

- `WorldObjectCommandIntent::{Place, Pick, Move, Rotate, Delete}` — new client-local command intent enum/event
- `SpawnPanelUi` request flags converted or bridged to `WorldObjectCommandIntent`
- `handle_world_object_*_input(..., intents: EventReader<WorldObjectCommandIntent>, ...)` — modified consumers
- `editing_mode_accepts_world_object_commands(mode: EditingMode) -> bool` — new helper

**Regression tests**:

- `place_definition_mode_primary_action_emits_only_world_object_place_intent` — left click in Place mode emits placement intent and no terrain intent.
- `world_object_place_does_not_fire_when_ui_owns_pointer` — left click with `PointerInputOwner::Ui` emits no world-object placement intent.
- `world_object_place_does_not_fire_when_terrain_owns_pointer` — left click with `PointerInputOwner::TerrainBrush` emits no world-object placement intent.
- `select_edit_mode_primary_action_emits_pick_or_edit_intent_only` — armed cursor pick/move in Select/Edit mode emits the matching world-object intent and no terrain intent.
- `world_object_keyboard_delete_does_not_fire_when_ui_or_text_owns_keyboard` — `Delete` with `KeyboardInputOwner::Ui` or `Text` emits no delete intent.
- `panel_delete_and_keyboard_delete_share_delete_intent` — panel delete request and `Delete` key both produce the same `WorldObjectCommandIntent::Delete` when ownership permits it.
- Existing server tests continue to prove placement/edit requests remain authoritative and validated server-side.

**Verify**: Run `cargo test -p client --features spawn-panel input_commands`, `cargo test -p client --features spawn-panel plugin`, and `cargo test -p server world_object`; manually confirm Place/Edit tabs produce only their matching world-object requests and UI-owned pointer input does not leak into world clicks.

---

## Phase 5: Locomotion and Camera Use Central Ownership

Route movement axes and camera yaw through the same client ownership decision so text/UI focus can suppress WASD/gamepad movement and camera control before Lightyear buffers input. Preserve the existing replicated movement/camera simulation contract, but express it as filtered `NetworkedPlayerActions::Move` / `NetworkedPlayerActions::CameraYaw` transport state; change the client transport writer, not movement physics.

**Files**: `crates/client/src/input/control.rs`, `crates/client/src/input/ownership.rs`, `crates/client/src/input/schedule.rs`, `crates/client/src/gameplay.rs`, `crates/client/tests/input_commands.rs`, `crates/protocol/src/character/movement.rs`, `crates/protocol/src/diagnostics.rs`, `README.md`

**Key changes**:

- `KeyboardInputOwner::allows_locomotion(self) -> bool` — new helper; false for text/UI-owned keyboard
- `PointerInputOwner::allows_camera_control(self) -> bool` — new helper; false for UI-owned pointer/capture and editor-owned pointer gestures
- move WASD/gamepad movement and any ownership-sensitive jump binding from `InputMap<NetworkedPlayerActions>` to `InputMap<RawClientActions>`; keep direct networked bindings only for inputs explicitly safe to bypass ownership
- `write_filtered_control_actions(ownership: Res<ClientInputOwnershipSnapshot>, query: Query<(&ActionState<RawClientActions>, &mut ActionState<NetworkedPlayerActions>), With<Controlled>>)` — copies `Move`, `CameraYaw`, and jump into networked input only when ownership permits
- schedule `write_filtered_control_actions` in `FixedPreUpdate` in/around `lightyear_inputs::client::InputSystems::WriteClientInputs`, before `lightyear_inputs::client::InputSystems::BufferClientInputs`
- `sync_camera_yaw_to_input(...)` — removed or folded into the filtered transport writer so camera yaw cannot bypass ownership
- shared movement code continues to read `ActionState<NetworkedPlayerActions>` in `FixedUpdate` on predicted client and server

**Regression tests**:

- `wasd_updates_move_action_when_keyboard_owned_by_gameplay` — pressing `W`/`A` with gameplay ownership writes non-zero `NetworkedPlayerActions::Move` before buffering.
- `wasd_does_not_update_move_action_when_keyboard_owned_by_text` — same keys under text ownership write zero movement before buffering.
- `wasd_does_not_update_move_action_when_keyboard_owned_by_ui` — same keys under UI ownership write zero movement before buffering.
- `gamepad_movement_respects_keyboard_or_control_ownership_policy` — gamepad movement follows the documented ownership policy and cannot bypass text/UI suppression if gamepad is considered ownership-sensitive.
- `camera_yaw_syncs_when_pointer_allows_camera_control` — gameplay/world pointer ownership writes expected `NetworkedPlayerActions::CameraYaw` before buffering.
- `camera_yaw_does_not_change_when_pointer_owned_by_ui` — UI pointer ownership prevents unintended camera yaw changes before buffering.
- `camera_yaw_does_not_change_when_pointer_owned_by_editor_tool` — terrain/world-object pointer ownership suppresses camera yaw if editor gestures own pointer input.
- `control_input_filter_runs_before_lightyear_buffers_inputs` — schedule-order test proves movement/camera filtering happens before `lightyear_inputs::client::InputSystems::BufferClientInputs`.

**Verify**: Run `cargo test -p client input_commands` and `cargo test -p protocol character` if available, otherwise `cargo test -p protocol`; manually playtest locomotion/camera feel, confirm typing `WASD` in text/UI focus does not move, confirm normal gameplay movement/camera still feels correct, and confirm predicted/server simulation still receives movement through Lightyear input buffering.

---

## Phase 6: Dev Hotkeys and Scheduling Are Centralized

Ensure dev inspector hotkeys, ownership snapshot creation, control/semantic producers, and command consumers run in explicit order. Remove remaining raw gameplay/editor command polling except documented low-level exceptions.

**Files**: `crates/client/src/input/mod.rs`, `crates/client/src/input/schedule.rs`, `crates/client/src/input/control.rs`, `crates/client/src/input/ability.rs`, `crates/client/src/input/editor.rs`, `crates/client/src/gameplay.rs`, `crates/client/src/map.rs`, `crates/client/tests/input_commands.rs`, `crates/dev/src/lib.rs`, `crates/dev/src/panels/world_inspector.rs`, `crates/dev/src/panels/spawn.rs`, `crates/protocol/src/diagnostics.rs`, `README.md`

**Key changes**:

- `ClientInputSet::{Capture, WriteTransport, ProduceLocalCommands, Consume}` — client schedule labels aligned with Lightyear input buffering
- `produce_dev_hotkey_commands(keys: Res<ButtonInput<KeyCode>>, ownership: Res<ClientInputOwnershipSnapshot>, ...)` — new/modified dev hotkey gate
- `NetworkedPlayerActions` kept as the replicated low-level transport vocabulary for movement/camera/jump/ability inputs unless a later plan explicitly migrates to a new Lightyear input type
- physical bindings for ownership-sensitive actions live on `InputMap<RawClientActions>`; `InputMap<NetworkedPlayerActions>` bindings remain only for inputs proven safe to bypass ownership
- diagnostics updated from raw action checks to semantic/control input checks where appropriate

**Regression tests**:

- `client_input_sets_run_capture_before_write_transport_before_local_command_consume` — minimal app test proves schedule ordering by observing produced ownership, filtered `ActionState<NetworkedPlayerActions>`, emitted local intents, and consumed command events in one update.
- `dev_hotkeys_fire_when_gameplay_owns_keyboard` — F-key dev hotkeys trigger under gameplay/dev keyboard ownership.
- `dev_hotkeys_do_not_fire_when_ui_owns_keyboard` — F-key dev hotkeys are suppressed while UI owns keyboard.
- `dev_hotkeys_do_not_fire_when_text_owns_keyboard` — F-key dev hotkeys are suppressed while text owns keyboard.
- `networked_input_map_no_longer_triggers_ownership_sensitive_behavior_without_raw_to_networked_translators` — with translators disabled, raw key/mouse input may populate `RawClientActions` but does not activate abilities, move, rotate camera, or emit editor requests; this validates integration behavior rather than enum shape.

**Verify**: Before running, ensure no other cargo build/check/test is active, then run `cargo check-all`, `cargo test -p client --features spawn-panel input_commands`, and `cargo test-all`; manually confirm F3/F4/F5/F6 behavior, ability hotkeys, locomotion/camera, terrain brush, and world-object edit flows with focused UI/text.

## Testing Checkpoints

Use integration-style regression coverage in `crates/client/tests/input_commands.rs` for plugin/system behavior. Prefer testing public resources, events, emitted intents, and `ActionState<RawClientActions>` / `ActionState<NetworkedPlayerActions>` effects over unit-testing private helper functions. Do not add tests that only revalidate Rust type-system guarantees, such as proving a field can no longer hold a removed enum type. For every producer, include both positive ownership tests and negative ownership/exclusion tests: abilities require gameplay keyboard ownership before Lightyear buffers input; locomotion requires gameplay keyboard/control ownership before buffering; camera requires camera-eligible pointer ownership before buffering; terrain requires terrain pointer ownership; world-object tools require world-object pointer or permitted keyboard ownership; dev hotkeys require permitted keyboard ownership.

- After Phase 1: Ability activation first populates `ActionState<RawClientActions>`, then is gated by keyboard ownership before entering `ActionState<NetworkedPlayerActions>` and before Lightyear buffers input; validation behavior is unchanged; client input plugin resources initialize in a minimal `App`.
- After Phase 2: Ability assets use semantic `AbilityInput`, while shared activation/effect systems still observe Lightyear-buffered `ActionState<NetworkedPlayerActions>`.
- After Phase 3: Terrain input has latched pointer ownership and cannot compete with UI/world-object actions; regression tests cover UI-started and terrain-started gesture latching.
- After Phase 4: World-object commands share explicit local intents from both UI and pointer/keyboard sources, while server request contracts remain unchanged; regression tests cover mode-exclusive terrain vs world-object intent emission.
- After Phase 5: Locomotion and camera are written by central ownership-aware transport writers before Lightyear buffers input, while existing movement simulation remains unchanged; regression tests cover `WASD` suppression under text/UI ownership.
- After Phase 6: Scheduling and docs reflect the split between networked input transport and client-local command ownership; raw command polling remains only where explicitly documented.
