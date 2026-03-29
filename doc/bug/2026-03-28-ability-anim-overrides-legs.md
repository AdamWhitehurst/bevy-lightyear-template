---
date: 2026-03-28T12:00:00-05:00
researcher: Claude
git_commit: e1d8c160
branch: master
repository: bevy-lightyear-template
topic: "Ability animations override leg locomotion"
tags: [bug, animation, sprite-rig, masking]
status: investigating
last_updated: 2026-03-28
last_updated_by: Claude
---

# Bug: Ability Animations Override Leg Locomotion

**Date**: 2026-03-28
**Git Commit**: e1d8c160
**Branch**: master

## User's Prompt

The animations aren't blending completely correctly. Despite not specifying anything for the legs, the punch animation is overriding leg animation while it plays. If player is standing and triggers punch, then start moving while the punch animation plays, the legs do not start walking until punch animations finishes. Similarly, if player is running and punches then stops while punch animation is playing, the legs will continue running until punch finishes.

We expect that the legs will walk/run/stay-still independent of the punch animation because the punch does not specify anything for legs.

## Summary

Two interacting mechanisms freeze leg animation during abilities:

1. **`build_clip_from`** (`animation.rs:227`) fills ALL bones with hold-at-default curves — including bones the animation doesn't specify (legs in punch). The punch clip actively drives legs to their default position.

2. **`trigger_ability_animations`** (`animset.rs:61`) sets `loco_state.active = false`, which causes `update_locomotion_blend_weights` to skip weight updates. Locomotion clips keep playing at their pre-punch weights but can't adapt to velocity changes.

Result: standing→punch→move = legs stay idle (idle weight frozen at 1.0). Running→punch→stop = legs keep running (run weight frozen at 1.0).

## Investigation

### Clip Building: Hold-at-Default for All Bones

`build_clip_from` (`animation.rs:227-260`) iterates ALL `bone_defaults` from the rig. For bones NOT in the animation's `bone_timelines`, it calls `add_hold_at_default_curves` which adds rotation + translation curves holding the bone at its default position.

The punch animation (`punch.anim.ron`) only specifies `torso` and `arm_r`. But the built clip has curves for ALL 7 bones (root, torso, head, arm_l, arm_r, leg_l, leg_r).

This was intentional for locomotion blending (comment on line 225: "ensuring every clip contributes to every bone for correct blend weighting"), but it's harmful for ability clips that should only affect specific bones.

### Locomotion State Freeze

`trigger_ability_animations` (`animset.rs:60-62`):
```rust
transitions.play(&mut player, node_idx, ABILITY_CROSSFADE);
loco_state.active = false;
```

`update_locomotion_blend_weights` (`animation.rs:566-567`):
```rust
if !loco_state.active {
    continue; // locomotion disabled during ability animations
}
```

This freezes locomotion weights for the entire ability duration.

### Graph Structure

`build_graph_for_animset` (`animation.rs:431-469`):
- Locomotion clips → children of a blend node → child of root
- Ability clips → direct children of root

Both subtrees contribute to root's blend for ALL bones.

## Code References

- `crates/sprite_rig/src/animation.rs:227-260` — `build_clip_from` with hold-at-default
- `crates/sprite_rig/src/animation.rs:263-284` — `add_hold_at_default_curves`
- `crates/sprite_rig/src/animation.rs:431-469` — `build_graph_for_animset`
- `crates/sprite_rig/src/animation.rs:548-596` — `update_locomotion_blend_weights` with active check
- `crates/sprite_rig/src/animset.rs:26-63` — `trigger_ability_animations`
- `crates/sprite_rig/src/animset.rs:69-101` — `return_to_locomotion`
- `assets/anims/humanoid/punch.anim.ron` — only torso + arm_r

## Hypotheses

### H1: Locomotion freeze + hold-at-default curves lock legs during abilities

**Hypothesis:** `loco_state.active = false` freezes locomotion blend weights, and the ability clip's hold-at-default curves for legs prevent locomotion from driving legs independently.

**Prediction:** If we keep locomotion weights updating and prevent ability clips from affecting bones they don't specify, legs will respond to velocity independently of ability playback.

**Test:** Implement per-bone masking, verify legs animate independently during punch.

**Decision:** Validated — analysis confirms the two mechanisms. Revised fix approach after discussion (F1 rejected, F2 accepted).

## Fixes

### F1: Hardcoded upper/lower body mask groups (addresses H1)

**Root Cause:** All clips drive all bones (hold-at-default fill). Locomotion disabled wholesale during abilities.

**Fix:** Define UPPER_BODY/LOWER_BODY mask groups, mask ability clips to disable lower body, mask locomotion to disable upper body during abilities.

**Decision:** Rejected — hardcoded body regions don't generalize to non-humanoid rigs.

### F2: Per-bone masking derived from ability clip's `bone_timelines` (addresses H1)

**Root Cause:** Same as F1.

**Fix:**
1. Assign each bone its own mask group (bone index = group index, max 64 bones per rig via `AnimationMask = u64`)
2. Register bone → mask group in `AnimationGraph` via `add_target_to_mask_group`
3. When building ability clip graph nodes: compute mask from bones NOT in `SpriteAnimAsset::bone_timelines` — those groups get masked out on the ability node
4. When ability triggers: compute the inverse mask (bones the ability DOES specify) and apply it to the locomotion blend node, so locomotion stops driving those bones
5. When ability ends: remove the mask from the locomotion blend node
6. Keep locomotion weights updating during abilities (remove `loco_state.active` freeze)
7. Use `AnimationPlayer::play()` directly for abilities instead of `AnimationTransitions::play()`
8. If an ability animation explicitly defines leg keyframes, those legs ARE masked from locomotion — the ability controls them

**Risk:**
- Max 64 bones per rig (u64 mask) — sufficient for sprite rigs
- `root` bone: if not specified in ability, locomotion keeps driving it (correct default)
- Need to store per-ability mask info so trigger/return systems can apply/remove locomotion masks

**Decision:** Approved

### F3: Unmask on clip finish, not just ability removal (addresses H1)

**Root Cause:** F2's `return_to_locomotion` only triggered on `ActiveAbility` removal. Ability entities persist through Recovery phase after the animation clip finishes. During that gap, the locomotion blend is still masked and the finished clip holds its final frame values — bones freeze.

**Fix:** `return_to_locomotion` now triggers on `clip_finished || ability_removed` (whichever comes first) using `ActiveAnimation::is_finished()`.

**Risk:** None — the mask and clip cleanup is the same, just triggered earlier.

**Decision:** Approved

## Solutions

F2 + F3 implemented. Changes:
- `animation.rs`: `BuiltAnimGraph` gains `locomotion_blend_node` and `ability_bone_masks`
- `animation.rs`: `build_graph_for_animset` — per-bone mask groups, ability clips get `add_clip_with_mask`, computes specified/unspecified masks
- `animation.rs`: removed `LocomotionState`, removed `loco_state.active` gate from `update_locomotion_blend_weights`
- `animset.rs`: `trigger_ability_animations` — masks ability bones out of locomotion blend, plays via `AnimationPlayer::play()` directly
- `animset.rs`: `return_to_locomotion` — unmasks on clip finish or ability removal, stops ability clip
- `animset.rs`: new `ActiveAbilityAnim` component tracks which ability anim is playing
