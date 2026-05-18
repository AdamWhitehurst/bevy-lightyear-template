## Context & Research

### Relevant Code and Patterns

- `crates/protocol/src/lib.rs` defines `PlayerActions::{Move, CameraYaw, Jump, PlaceVoxel, RemoveVoxel, Ability1..Ability4}`.
- `crates/client/src/gameplay.rs` binds WASD/gamepad movement, mouse voxel actions, and `Digit1`-`Digit4` ability inputs into `InputMap<PlayerActions>`.
- `crates/protocol/src/ability/activation.rs` maps `PlayerActions` to ability slots and performs the validation to preserve: slot lookup, ability asset lookup, cooldown checks, duplicate active ability prevention, and grounded/airborne conditional effects.
- `crates/protocol/src/ability/effects.rs` polls caster `ActionState<PlayerActions>` for active-phase `OnInputEffects`.
- `crates/protocol/src/ability/types.rs` stores concrete `PlayerActions` in `EffectTrigger::OnInput` and `InputEffect`.
- `crates/client/src/map.rs` reads `PlaceVoxel`, `RemoveVoxel`, and raw `Delete` for terrain brush, voxel edit, world-object pick/place/move/delete/rotate flows. It already uses many `trace!` early-outs and has terrain-only egui pointer suppression.
- `crates/dev/src/state.rs` already has `EditingMode::{Terrain, PlaceDefinition, PlaceFreeForm, SelectEdit}` as a useful ownership seam.
- `crates/dev/src/lib.rs` and `crates/dev/src/panels/spawn.rs` read raw function keys / mutate panel request flags directly.
- Server-authoritative boundaries already exist for terrain and world objects in `crates/protocol/src/map/voxel.rs`, `crates/protocol/src/world_object/types.rs`, and handlers in `crates/server/src/map.rs`.
- Ability regression coverage lives primarily in `crates/protocol/tests/ability_systems.rs`; world-object UI/server coverage lives in `crates/client/tests/plugin.rs`, `crates/server/tests/world_object_placement.rs`, and `crates/server/tests/world_object_edit.rs`.

### Institutional Learnings

- `docs/solutions/` does not exist in this repo.
- Project memory `core/project/input-command-architecture.md`: central capture/context stack, semantic intents, ability validation preservation, and dev/world editing as explicit interaction modes are the preferred direction.
- Project memory `core/project/terrain-sculpting.md`: server remains authoritative; client prediction is visual responsiveness only; rejects roll back whole predictions; egui pointer suppression already matters for brush input.
- `docs/tasks/2026-05-15-terrain-sculpting-brushes/design.md`: terrain tools should route through explicit editing mode and not compete with object placement/selection modes.

### External References

- Not used. The repo already has direct local patterns for Bevy ECS events/resources, Lightyear request/ack boundaries, ability validation, and terrain/world-object authority. External research would add little practical value for this architecture plan.

---
