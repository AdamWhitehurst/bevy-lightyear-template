---
date: 2026-05-16
topic: input-command-architecture
---

# Input Command Architecture Requirements

## Problem Frame

UI, dev tools, world editing, and abilities currently consume overlapping raw or low-level input. This makes conflicts possible by construction: a key can both type into egui and activate gameplay, mouse clicks can mean UI interaction or world edit, and ability data can depend on concrete input enum values.

The goal is a systemic refactor: make input conflicts impossible by routing all non-movement intent through explicit capture, semantic commands, and gameplay-owned validation. This is not a new UI/gameplay feature pass.

Vision fit: this supports Combat, Stage Editing, Home-Base editing, and Overworld admin tooling by making interaction systems maintainable across modes without changing the game vision.

---

## Actors

- A1. Player: uses keyboard, mouse, controller, and future UI surfaces to control characters and activate abilities.
- A2. Dev/editor user: uses egui panels and hotkeys to inspect, sculpt terrain, and place/edit world objects.
- A3. Gameplay systems: validate and execute ability, movement, and world-editing intent.
- A4. UI systems: render current state and emit requests, but do not own gameplay mutation.
- A5. Networking/prediction systems: replicate or validate gameplay-changing intent and reconcile outcomes.

---

## Key Flows

- F1. Ability activation through gameplay focus
  - **Trigger:** Player presses an ability binding or clicks a future ability UI control while gameplay owns input.
  - **Actors:** A1, A3, A5
  - **Steps:** input capture confirms gameplay ownership; input/UI emits `ActivateAbilitySlot`; ability systems validate slot, cooldown, grounded state, and duplicate active ability; prediction/server authority handles execution.
  - **Outcome:** ability activation uses one validation path regardless of physical input source.
  - **Covered by:** R1, R3, R4, R6

- F2. Text or panel focus suppresses gameplay intent
  - **Trigger:** User types or clicks while egui text/panel interaction owns input.
  - **Actors:** A2, A4
  - **Steps:** input capture marks keyboard and/or pointer as UI-owned; gameplay command emission is suppressed for captured inputs; UI receives the interaction normally.
  - **Outcome:** gameplay abilities and world edits cannot fire from captured UI input.
  - **Covered by:** R1, R2, R7

- F3. World editing through semantic commands
  - **Trigger:** Dev/editor user applies terrain brush or world-object tool.
  - **Actors:** A2, A3, A5
  - **Steps:** selected interaction mode owns the pointer/action; UI or hotkey emits a world-edit command; client prediction may apply local feedback; server validates and accepts/rejects the request.
  - **Outcome:** terrain/object editing no longer competes with ability or generic place/remove input.
  - **Covered by:** R1, R3, R5, R6

---

## Requirements

**Input ownership and capture**

- R1. There must be one explicit input ownership decision before gameplay commands are emitted for keyboard, pointer, and controller-style actions.
- R2. UI/text/panel capture must suppress gameplay and editor command emission for the captured input class while preserving normal UI interaction.
- R3. Interaction modes must make ownership explicit for gameplay, terrain brush, world-object placement, world-object selection/editing, and dev inspector interactions.

**Semantic commands**

- R4. UI widgets, hotkeys, and controller bindings must emit the same semantic command for the same player intent.
- R5. World editing must consume semantic editor commands instead of independently reading generic placement/removal actions.
- R6. Gameplay-changing commands must remain compatible with existing client prediction and server authority boundaries.

**Ability activation**

- R7. Ability activation must consume ability-slot or ability-intent commands rather than polling physical/input-action state directly inside activation logic.
- R8. Existing ability validation responsibilities must remain gameplay-owned: cooldowns, grounded requirements, duplicate active ability prevention, and slot/ability lookup.
- R9. Ability data must not store concrete physical or low-level input enum values for runtime triggers; it should refer to semantic ability/domain triggers.

**UI and dev tools**

- R10. Immediate-mode UI systems should render state and emit commands/requests; they should not directly perform gameplay or world mutation.
- R11. Dev inspector toggles and panels should use the same capture/routing principles as gameplay-facing UI.

**Debuggability**

- R12. Expected command rejection or suppression must be traceable with explicit reasons, consistent with the project rule against silent early-outs.

---

## Acceptance Examples

- AE1. **Covers R1, R2, R7.** Given an egui text field is focused, when the user types `1`, the text field receives `1` and no ability activation command is emitted.
- AE2. **Covers R4, R7, R8.** Given gameplay owns input, when the user presses ability binding `1` or clicks the equivalent ability UI control, both paths emit the same activation intent and use the same validation path.
- AE3. **Covers R3, R5.** Given terrain brush mode owns pointer input, when the user applies the primary action, terrain editing receives an editor command and ability/world-object placement systems do not also consume that action.
- AE4. **Covers R9.** Given an ability asset defines a combo or active-phase trigger, it refers to a semantic ability/domain trigger rather than `PlayerActions::Ability1` or another concrete input enum variant.

---

## Success Criteria

- Input conflicts are prevented by architecture rather than patched per-system.
- UI, hotkeys, and future control surfaces can trigger the same gameplay intent without duplicate validation code.
- Ability validation remains centralized and authoritative.
- World/editor interactions are mode-owned and do not compete with ability input.
- A planner can turn this document into phased implementation work without inventing scope or product behavior.

---

## Scope Boundaries

- Do not add new gameplay features.
- Do not add save/load input settings as part of this refactor.
- Do not implement item hotbars, macros, radial menus, drag/drop hotbar editing, tooltips, cooldown UI, or controller-specific UX unless separately requested.
- Do not replace all movement/camera input architecture unless needed for command routing boundaries.
- Do not weaken server authority for world edits or ability execution.
- Do not make UI the owner of gameplay state; UI is a state projection and command source.

---

## Key Decisions

- Use whole-architecture scope rather than a bug-first patch: the priority is making the class of bugs impossible.
- Preserve existing ability validation as a leverage point instead of rewriting ability execution from scratch.
- Treat `PlayerActions` as too broad for long-term domain vocabulary; use staged translation boundaries rather than an all-at-once enum rewrite.
- Treat dev/world editing as first-class interaction modes because they share input surfaces with gameplay.

---

## Dependencies / Assumptions

- Current code uses `PlayerActions` across movement, camera yaw, jump, voxel editing, and ability slots.
- Current ability activation and active input effects poll `ActionState<PlayerActions>`.
- Current terrain/world-object tools read generic placement/removal/delete input directly in client map systems.
- Current egui/dev panels already provide useful state and mode concepts that can be adapted into command production.

---

## Outstanding Questions

### Resolve Before Planning

- None.

### Deferred to Planning

- [Affects R1-R3][Technical] Where should the capture/context state live so it works with Bevy scheduling, egui context timing, and Lightyear input buffering?
- [Affects R4-R8][Technical] Which command types should be client-local ECS events versus replicated/predicted protocol inputs or messages?
- [Affects R9][Needs research] What is the smallest compatible migration path for existing ability assets and `EffectTrigger::OnInput` tests?
- [Affects R10-R11][Technical] Which existing dev resources should remain resources and which should become per-controller/editor components?

---

## Next Steps

-> `/ce-plan` for structured implementation planning.
