# Design Discussion

## Current State

`PlayerActions` is currently both a physical/network input vocabulary and a gameplay/editor vocabulary: it contains movement, camera yaw, jump, voxel edit actions, and ability slots (`crates/protocol/src/lib.rs:70-89`). Client setup binds those actions directly to WASD/gamepad movement, mouse voxel buttons, and `Digit1`-`Digit4` ability hotkeys (`crates/client/src/gameplay.rs:63-74`).

Ability activation is centralized enough to preserve, but it still polls `ActionState<PlayerActions>` directly. The current path maps ability actions to slots, checks `just_pressed`, applies slot lookup, asset lookup, cooldown, duplicate-active, and grounded/airborne validation, then spawns `ActiveAbility` (`crates/protocol/src/ability/activation.rs:18-33`, `crates/protocol/src/ability/activation.rs:61-155`). Active-phase input effects also poll `ActionState<PlayerActions>` and ability assets store concrete input actions in `EffectTrigger::OnInput` / `InputEffect` (`crates/protocol/src/ability/effects.rs:305-342`, `crates/protocol/src/ability/types.rs:101-116`, `crates/protocol/src/ability/types.rs:309-321`).

Editor/world interactions are similarly coupled to raw inputs. Terrain brush input reads `PlayerActions::PlaceVoxel`, has local egui pointer suppression, and emits the existing `VoxelBrushEditRequest` (`crates/client/src/map.rs:614-696`, `crates/client/src/map.rs:710-715`). Legacy voxel edit reads `PlaceVoxel` / `RemoveVoxel` directly (`crates/client/src/map.rs:824-885`). Dev tools already expose useful ownership seams through `EditingMode::{Terrain, PlaceDefinition, PlaceFreeForm, SelectEdit}` and inspector panel state, but dev hotkeys and panel requests still read/mutate raw UI state directly (`crates/dev/src/state.rs:5-25`, `crates/dev/src/lib.rs:54-65`, `crates/dev/src/panels/spawn.rs:170-206`, `crates/dev/src/panels/spawn.rs:278-350`).

Server authority boundaries already exist and should remain unchanged: terrain/world-object edits are request/ack/reject messages (`crates/protocol/src/map/voxel.rs:25-65`, `crates/protocol/src/world_object/types.rs:28-170`), with server-side validation and handlers for terrain/world-object placement, delete, move, and rotate (`crates/server/src/map.rs:777-862`, `crates/server/src/map.rs:900-1027`, `crates/server/src/map.rs:1160-1193`). This supports the vision's Stage Editing split: players edit home-base layout, admins edit the overworld, and tools create predefined instances (`VISION.md:119-123`).

## Desired End State

Input routing has one explicit ownership decision before non-movement gameplay/editor commands are emitted. UI/text/pointer capture, dev inspector panels, terrain editing, world-object editing, gameplay ability activation, and future UI/controller command producers no longer compete by independently polling raw keys or mouse buttons.

Correctness means:

- Focused text or UI-owned keyboard input does not emit ability/editor commands.
- UI-owned pointer input does not emit terrain/object commands.
- Terrain mode primary action produces terrain command intent only, not world-object placement/selection/editing.
- Keyboard and synthetic/UI-originated ability activation enter the same ability validation path.
- Ability assets express semantic ability/domain triggers, not `PlayerActions` variants.
- Terrain/world-object commands still translate into existing authoritative request/ack/reject flows.

## Patterns to Follow

- Preserve ability validation as gameplay-owned shared simulation logic; change the input source, not cooldown/slot/asset/grounded/duplicate-active semantics (`crates/protocol/src/ability/activation.rs:61-155`).
- Keep protocol for shared fixed-tick ability intent and existing network contracts; do not move client/dev editor command details into protocol unless they are actual network messages (`crates/protocol/src/map/voxel.rs:25-65`, `crates/protocol/src/world_object/types.rs:28-170`).
- Keep editor mode as the domain ownership seam, but require explicit tool/panel activation rather than letting default `EditingMode::Terrain` globally capture input (`crates/dev/src/state.rs:5-13`, `crates/dev/src/panels/spawn.rs:197-206`).
- Preserve existing `MessageSender<...Request>` request boundaries for terrain and world-object mutation (`crates/client/src/map.rs:695-696`, `crates/client/src/map.rs:883-885`).
- Continue using explicit `trace!` for expected no-op/suppression paths; `map.rs` already follows this for missing context, UI pointer suppression, and no-action cases (`crates/client/src/map.rs:619-643`, `crates/client/src/map.rs:710-715`).
- Update README surfaces when hotkey behavior, dev inspector behavior, terrain brush behavior, or ability `OnInput` schema changes (`README.md:135-144`, `README.md:166-188`).

Patterns not to follow:

- Do not keep expanding `PlayerActions` as the universal gameplay/editor command enum; it already mixes movement, camera, jump, voxel edits, and ability slots (`crates/protocol/src/lib.rs:70-89`).
- Do not let ability assets depend on concrete input enum values (`crates/protocol/src/ability/types.rs:101-116`, `crates/protocol/src/ability/types.rs:309-321`).
- Do not add another per-system egui/raw-input gate; terrain brush's local pointer suppression is useful evidence, but the target is centralized ownership (`crates/client/src/map.rs:635-643`, `crates/client/src/map.rs:710-715`).
- Do not treat client command gating as authorization. Existing server validation remains the security/authority boundary (`crates/server/src/map.rs:812-862`, `crates/server/src/map.rs:971-1027`).

## Design Decisions

1. **Input ownership model**: per-frame ownership snapshot — chosen for the first pass because it gives one testable decision point without prematurely building a full context stack.
2. **Ability intent location**: shared protocol ability intents only — ability fixed-tick intent belongs in shared simulation/protocol code; terrain/world-object editor commands stay client/dev-local until translated into existing network requests.
3. **Ability intent observation**: read-only per-caster/per-tick snapshot — activation and active-phase input effects can observe the same tick without destructive event ordering bugs.
4. **Editor pointer ownership**: gesture-latched ownership — pointer owner is chosen on press/start and retained through drag/release/cancel, preventing UI/world split-gesture bugs.
5. **Vision boundary**: this is infrastructure for Combat, Stage Editing, Home-Base editing, and Overworld admin tooling, not a new player-facing editing feature; it preserves the Home-Base/Overworld editing distinction in `VISION.md:119-123`.

## What We're NOT Doing

- Adding production ability UI, hotbars, radial menus, drag/drop, cooldown UI, tooltips, macros, or input rebinding.
- Redesigning movement/camera input beyond documenting whether capture suppresses jump/movement/camera yaw.
- Changing terrain/world-object request/ack/reject protocol contracts unless implementation reveals a narrow compatibility need.
- Adding new server permission/admin infrastructure in this refactor.
- Promoting dev/admin world-editing into normal overworld player tools.
- Replacing Lightyear prediction/authority with client-local command execution.
- Creating a broad global command enum that mixes unrelated domains.

## Open Risks

- Fixed-tick ability intent transport may be more subtle than the design summary: it must reach predicted client and server simulation without being missed or double-observed.
- Bevy scheduling must ensure ownership snapshots run before command producers, and producers before consumers.
- Keyboard/text capture policy for movement, jump, and camera yaw must be explicit; otherwise low-level input can leak through while semantic commands are suppressed.
- Feature-gated inspector/spawn-panel tests could silently miss key editor cases unless verification names those feature sets.
- Existing server-side world-edit validation may be permissive; this design preserves it but does not solve admin/player permission policy.
