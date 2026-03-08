---
date: 2026-03-07 09:35:21 PST
researcher: adam
git_commit: db7639b980a2eb485f2cac017cab7ea6644871b9
branch: master
repository: bevy-lightyear-template
topic: "Data-Driven Sprite Rig Animations Inspired by Overgrowth"
tags: [research, animation, sprite-rig, overgrowth, procedural-animation, bevy, 2d-animation]
status: complete
last_updated: 2026-03-07
last_updated_by: adam
last_updated_note: "Resolved open questions, added animation-ability bridge analysis"
---

# Research: Data-Driven Sprite Rig Animations Inspired by Overgrowth

**Date**: 2026-03-07 09:35:21 PST
**Researcher**: adam
**Git Commit**: db7639b980a2eb485f2cac017cab7ea6644871b9
**Branch**: master
**Repository**: bevy-lightyear-template

## Research Question

How does Overgrowth's animation system work, and how can we implement a 2D data-driven "sprite rig" animation system in Bevy where characters are compositions of sprites (head, torso, arms, legs) loaded from assets, with animations also loaded from assets?

## Summary

Overgrowth uses a **hybrid animation system** with ~13 keyframe poses, procedural interpolation, IK, and physics-based ragdoll — all scripted via data files. The core insight is: **a tiny number of authored poses + code-driven interpolation produces complex, responsive animation** without hundreds of hand-animated frames.

For our 2.5D brawler, we translate this to **2D sprite rigs**: characters composed of separate sprite images (head, torso, arms, legs) arranged on a bone hierarchy, with animations defined as keyframed transforms on those bones. Everything loads from RON asset files. Bevy has no built-in 2D skeletal animation, but its entity hierarchy + `Transform` propagation + `AnimationClip`/`AnimationGraph` provide all the primitives needed.

The codebase currently has **no animation or sprite code** — characters render as capsule placeholders in `crates/render/src/lib.rs`.

---

## Part 1: Overgrowth's Animation System

### Architecture: Three Pillars

David Rosen's system (detailed in his [GDC 2014 "Animation Bootcamp" talk](https://www.gdcvault.com/play/1020583/Animation-Bootcamp-An-Indie-Approach)) combines:

1. **Keyframe poses** — ~13 total hand-authored static poses (idle, crouch, walk-pass, walk-reach, run-pass, run-reach, jump, etc.) stored as `.anm` files
2. **Procedural interpolation** — Code blends between poses using Catmull-Rom cubic splines, driven by game state (velocity, combat state, input)
3. **Physics-based animation** — Active ragdoll for hit reactions, falls, and death; smooth blend between scripted and physics-driven animation

### Key Technique: Minimal Poses + Procedural Blending

Locomotion uses only **2 keyframes per gait** (pass pose + reach pose). Mirroring gives 4 effective frames per cycle. The system:

- Advances cycle phase by **distance traveled** (not time) → eliminates foot sliding
- Interpolates between walk and run poses based on **velocity** → seamless speed transitions
- Stacks **4+ interpolation passes** per bone from different pose sources → complex output from simple inputs

### Data-Driven Design

- **Animations** stored as `.anm` (PHXANM) binary files, authored in Blender with a custom addon
- **Animation selection/blending** driven by AngelScript files (`Data/Scripts/aschar.as`) — the engine calls 32 hook functions that scripts implement
- **Event keyframes** embedded in `.anm` files on "DEF bones" — fire script callbacks at authored moments (attack impact, footstep, ragdoll activation)
- **No explicit state machine graph** — state is managed procedurally in script based on velocity, combat state, input, and AI goals
- **Retargeting** supported: BVH motion capture data can be remapped to character skeletons

### What We Take From Overgrowth

| Overgrowth Concept | Our 2D Sprite Rig Equivalent |
|---|---|
| ~13 keyframe poses | Small set of authored keyframe animations per character rig |
| Procedural pose blending | Bevy `AnimationGraph` blend nodes with runtime weight adjustment |
| Distance-driven locomotion | Animation phase driven by movement speed |
| Script-driven animation selection | Bevy systems selecting animation states based on game state |
| Event keyframes | Animation events triggering cosmetic effects (sound, particles) |
| Active ragdoll | Procedural hit reactions (recoil/knockback transforms applied additively) |
| Per-character scripting | Per-rig animation configuration loaded from asset files |

### Overgrowth Sources

- [GDC 2014 Talk (free)](https://www.gdcvault.com/play/1020583/Animation-Bootcamp-An-Indie-Approach) — [Also on Internet Archive](https://archive.org/details/GDC2014Rosen)
- [Open source engine (GitHub)](https://github.com/WolfireGames/overgrowth) — Full C++ Phoenix Engine source
- [AngelScript history (GitHub)](https://github.com/kavika13/overgrowth-scripts) — Complete scripting evolution across alphas
- [Character Scripting Wiki](https://wiki.wolfire.com/index.php/Character_Scripting) — All 32 hook functions
- [Custom Animations Wiki](https://wiki.wolfire.com/index.php/Custom_Animations) — PHXANM format, Blender pipeline
- [Why Not Euphoria (blog)](https://www.wolfire.com/blog/2009/11/why-we-are-not-using-euphoria/)
- [Procedural Animations with IK (third-party breakdown)](https://oraqia.wordpress.com/2014/07/01/procedural-animations-with-ik/)

---

## Part 2: 2D Sprite Rig Animation Concepts

### What Is a Sprite Rig?

A character composed of **separate sprite images per body part** (head, torso, upper arm, lower arm, thigh, shin, etc.) arranged on a **bone hierarchy**. Instead of frame-by-frame sprite sheets, you animate by **transforming bones** (position, rotation, scale) over time with keyframes. The runtime interpolates between keyframes.

This is also called **cutout animation** or **puppet animation**. Popularized by tools like Spine and DragonBones.

### Core Concepts

| Concept | Description |
|---|---|
| **Bones** | Tree of transforms. Each bone's world transform = parent * local |
| **Slots** | Draw-order entries attached to bones. Control which sprite is visible and at what z-depth |
| **Attachments** | Actual sprite images bound to slots. Swappable at runtime (e.g., different weapons, appearance parts) |
| **Skins** | Named collections of attachments. Swap a skin to re-costume a character while reusing animations |
| **Animations** | Named sequences of timelines. Each timeline targets a bone property (rotation, translation, scale) or slot property (attachment, color) with keyframes + interpolation curves |

### Industry-Standard Data Format (Spine JSON)

Spine's format provides a reference for our RON asset design:

```json
{
  "bones": [
    { "name": "root", "parent": null, "x": 0, "y": 0, "rotation": 0 },
    { "name": "torso", "parent": "root", "x": 0, "y": 8, "rotation": 0 }
  ],
  "slots": [
    { "name": "torso_slot", "bone": "torso", "attachment": "torso_default" }
  ],
  "skins": {
    "default": { "torso_slot": { "torso_default": { "width": 32, "height": 48 } } }
  },
  "animations": {
    "walk": {
      "bones": {
        "torso": {
          "rotate": [{ "time": 0, "angle": 0 }, { "time": 0.5, "angle": 5, "curve": "bezier" }]
        }
      }
    }
  }
}
```

### Sources

- [Spine: In Depth](http://en.esotericsoftware.com/spine-in-depth) — Definitive explanation of 2D skeletal animation concepts
- [Spine JSON Format](http://en.esotericsoftware.com/spine-json-format) — Data format reference
- [DragonBones Format Spec](https://github.com/DragonBones/Tools/blob/master/doc/dragonbones_json_format_5.5.md)
- [Marc ten Bosch: 2D Skeletal Animation](https://marctenbosch.com/skeletal2d/)
- [Bevy Issue #5280: 2D Skeletal Animation](https://github.com/bevyengine/bevy/issues/5280)

---

## Part 3: Current Codebase State

**No animation or sprite code exists.** Characters are capsule placeholders.

| File | Current State |
|---|---|
| `crates/render/src/lib.rs` | `add_character_meshes` spawns `Capsule3d` with `StandardMaterial` |
| `crates/render/src/health_bar.rs` | Billboard health bars using `Mesh3d` quads |
| `crates/web/Cargo.toml` | Enables `bevy_animation` and `bevy_sprite` features (unused) |
| `assets/` | Only RON files for abilities/slots. No images or sprite assets |

### Relevant Design Context

- `VISION.md` — Brawlers have appearance evolution: stat-driven visual changes, alignment hue-shifting, inherited phenotypes from breeding
- `doc/scratch/stats.md` — Describes per-stat visual impacts (strength → muscular, agility → sleek, etc.)
- `doc/research/2025-09-30-sonic-battle-chao-design-research.md` — Proposes `CharacterAppearance` component, discusses body part customization

The sprite rig system directly enables the vision's appearance customization: swap sprite attachments per body part based on stats, alignment, and genetics.

---

## Part 4: Bevy Primitives for Sprite Rig Animation

### Entity Hierarchy = Bone Hierarchy

Bevy's parent-child entity system is the bone tree. Child `Transform` is local (relative to parent). `GlobalTransform` is computed automatically.

```rust
commands.spawn((
    Sprite::from_image(torso_image),
    Transform::from_xyz(0.0, 0.0, 0.0),
    children![
        (
            Sprite::from_image(head_image),
            Transform::from_xyz(0.0, 16.0, 0.1), // z for draw order
        ),
        (
            Sprite::from_image(arm_l_image),
            Transform::from_xyz(-8.0, 4.0, -0.1),
        ),
    ],
))
```

### AnimationClip + AnimationGraph = Animation Playback

Bevy's built-in animation system can animate **any component field** on any entity via `AnimatableCurve` + `animated_field!`. This works for 2D sprite rigs despite being designed for 3D.

```rust
// Animate a bone's rotation over time.
// UnevenSampleAutoCurve::new takes (time, value) tuples — NOT separate time/value arrays.
let curve = AnimatableCurve::new(
    animated_field!(Transform::rotation),
    UnevenSampleAutoCurve::new([
        (0.0,  Quat::IDENTITY),
        (0.25, rot_5deg),
        (0.5,  Quat::IDENTITY),
        (0.75, rot_neg5deg),
        (1.0,  Quat::IDENTITY),
    ]).unwrap(),
);
clip.add_curve_to_target(bone_target_id, curve);
```

### AnimationGraph for Blending

`AnimationGraph` provides weighted blend trees:

```
Root (blend)
├── Idle clip (weight: varies by velocity)
└── Walk clip (weight: varies by velocity)
```

Weights adjustable at runtime. Supports **mask groups** — animate upper body with attack while lower body walks.

### AnimationTransitions for Crossfades

```rust
transitions.play(&mut player, new_node_index, Duration::from_millis(200));
```

### Custom Asset Loading

Using `bevy_common_assets` 0.14 (Bevy 0.17):

```rust
use bevy_common_assets::ron::RonAssetPlugin;

#[derive(Asset, TypePath, Deserialize)]
struct SpriteRigAsset { /* ... */ }

app.add_plugins(RonAssetPlugin::<SpriteRigAsset>::new(&["rig.ron"]));
```

### Relevant Bevy Crates

| Crate | Purpose | Notes |
|---|---|---|
| `bevy_spine` | Full Spine runtime | Requires Spine license. WASM compatible. Bevy 0.18 support. |
| `bevy_spritesheet_animation` | Frame-based spritesheet animation | Not skeletal. Bevy 0.17+. |
| `bevy_trickfilm` | RON-manifest spritesheet animation | Frame-based, not skeletal. |
| `bevy_animation_graph` | Advanced animation state machines | Visual editor. Primarily 3D-focused. |

**None of these provide a built-in 2D skeletal rig system.** A custom implementation is needed.

---

## Part 5: Proposed Asset Format Design

### Sprite Rig Definition (`*.rig.ron`)

Defines the bone hierarchy, sprite attachments, and default pose for a character type.

```ron
(
    bones: [
        (name: "root",    parent: None,           default_transform: (translation: (0.0, 0.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "torso",   parent: Some("root"),   default_transform: (translation: (0.0, 8.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "head",    parent: Some("torso"),   default_transform: (translation: (0.0, 12.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "arm_l",   parent: Some("torso"),   default_transform: (translation: (-6.0, 4.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "arm_r",   parent: Some("torso"),   default_transform: (translation: (6.0, 4.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "leg_l",   parent: Some("root"),   default_transform: (translation: (-3.0, 0.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "leg_r",   parent: Some("root"),   default_transform: (translation: (3.0, 0.0), rotation: 0.0, scale: (1.0, 1.0))),
    ],
    slots: [
        (name: "torso",  bone: "torso",  z_order: 0.0, default_attachment: "torso_default"),
        (name: "head",   bone: "head",   z_order: 0.3, default_attachment: "head_default"),
        (name: "arm_l",  bone: "arm_l",  z_order: -0.1, default_attachment: "arm_default"),
        (name: "arm_r",  bone: "arm_r",  z_order: 0.1, default_attachment: "arm_default"),
        (name: "leg_l",  bone: "leg_l",  z_order: -0.2, default_attachment: "leg_default"),
        (name: "leg_r",  bone: "leg_r",  z_order: 0.2, default_attachment: "leg_default"),
    ],
    skins: {
        "default": {
            "torso_default": (image: "sprites/brawler/torso.png", anchor: BottomCenter),
            "head_default":  (image: "sprites/brawler/head.png",  anchor: BottomCenter),
            "arm_default":   (image: "sprites/brawler/arm.png",   anchor: TopCenter),
            "leg_default":   (image: "sprites/brawler/leg.png",   anchor: TopCenter),
        },
        "muscular": {
            "torso_default": (image: "sprites/brawler/torso_muscular.png", anchor: BottomCenter),
        },
    },
)
```

### Animation Definition (`*.anim.ron`)

Defines keyframed timelines for bones, following Overgrowth's philosophy of minimal poses with interpolation.

```ron
(
    name: "walk",
    duration: 0.6,
    looping: true,
    bone_timelines: {
        "torso": (
            rotation: [(time: 0.0, value: 0.0, curve: Linear), (time: 0.3, value: 3.0, curve: CubicBezier(0.4, 0.0, 0.6, 1.0)), (time: 0.6, value: 0.0, curve: Linear)],
            translation: [],
            scale: [],
        ),
        "arm_l": (
            rotation: [(time: 0.0, value: 15.0, curve: Linear), (time: 0.3, value: -15.0, curve: Linear), (time: 0.6, value: 15.0, curve: Linear)],
            translation: [],
            scale: [],
        ),
        "arm_r": (
            rotation: [(time: 0.0, value: -15.0, curve: Linear), (time: 0.3, value: 15.0, curve: Linear), (time: 0.6, value: -15.0, curve: Linear)],
            translation: [],
            scale: [],
        ),
        "leg_l": (
            rotation: [(time: 0.0, value: -20.0, curve: Linear), (time: 0.3, value: 20.0, curve: Linear), (time: 0.6, value: -20.0, curve: Linear)],
            translation: [],
            scale: [],
        ),
        "leg_r": (
            rotation: [(time: 0.0, value: 20.0, curve: Linear), (time: 0.3, value: -20.0, curve: Linear), (time: 0.6, value: 20.0, curve: Linear)],
            translation: [],
            scale: [],
        ),
    },
    slot_timelines: {
        // Optional: swap attachments mid-animation, change color/opacity
    },
    events: [
        (time: 0.15, name: "footstep_left"),
        (time: 0.45, name: "footstep_right"),
    ],
)
```

### Animation Set (`*.animset.ron`)

Maps game states to animations for a specific rig, defining the animation graph structure.

```ron
(
    rig: "rigs/brawler.rig.ron",
    states: {
        "idle":      (clip: "anims/brawler/idle.anim.ron",   weight: 1.0),
        "walk":      (clip: "anims/brawler/walk.anim.ron",   weight: 1.0),
        "run":       (clip: "anims/brawler/run.anim.ron",    weight: 1.0),
        "attack_1":  (clip: "anims/brawler/attack_1.anim.ron", weight: 1.0),
        "hit_react": (clip: "anims/brawler/hit_react.anim.ron", weight: 1.0),
    },
    blend_trees: {
        "locomotion": (
            type: Blend1D,
            parameter: "speed",
            entries: [
                (clip: "idle", threshold: 0.0),
                (clip: "walk", threshold: 3.0),
                (clip: "run",  threshold: 8.0),
            ],
        ),
    },
    transitions: {
        ("locomotion", "attack_1"): (duration: 0.1),
        ("attack_1", "locomotion"): (duration: 0.15),
        ("*", "hit_react"):         (duration: 0.05),
    },
)
```

### Animation Visual Descriptions

What each animation state should communicate to a player:

| State | Visual Feel |
|---|---|
| **idle** | Subtle breathing cycle — torso rises and falls slightly, head tilts a few degrees. Feet planted. Conveys readiness, not stiffness. |
| **walk** | Arms swing opposite to legs. Torso bobs gently with each step. Relaxed, unhurried. |
| **run** | Exaggerated arm/leg swing, torso leans slightly forward. More vertical bounce than walk. Feels propulsive. |
| **attack_1** | Winds back briefly (startup lean), then a sharp forward drive through the strike (active). Body follows through, then settles (recovery). Should feel committed and impactful. |
| **hit_react** | Short, sharp recoil away from the hit direction. Head snaps back. Arms flinch. Recovers quickly. Should not look floppy — controlled reaction, not ragdoll. |

---

## Part 6: ECS Architecture Sketch

### Components

| Component | Purpose |
|---|---|
| `SpriteRig(Handle<SpriteRigAsset>)` | Reference to the rig definition |
| `ActiveSkin(String)` | Current skin name (swappable for appearance evolution) |
| `AnimationState` | Current blend tree parameters (speed, attack state, etc.) |
| `BoneEntity(HashMap<String, Entity>)` | Maps bone names to child entities for direct access |

### Spawning Flow

1. Load `SpriteRigAsset` from `*.rig.ron` and `SpriteAnimAsset` from `*.anim.ron` at startup
2. When spawning a character, read the rig asset → spawn root entity + child entities per bone
3. Each bone entity gets: `Sprite` (from active skin), `Transform` (from bone default), `AnimationTarget`
4. Build `AnimationClip`s from loaded `*.anim.ron` assets → populate with `AnimatableCurve`s per bone
5. Build `AnimationGraph` with blend/clip nodes per the animation set
6. Spawn `AnimationPlayer` + `AnimationGraphHandle` on root entity

### Runtime Systems

| System | Schedule | Description |
|---|---|---|
| `spawn_sprite_rig` | `Update` | Reacts to new `SpriteRig` components, spawns bone entities |
| `update_animation_parameters` | `Update` | Reads velocity/combat state → sets blend weights on `AnimationGraph` |
| `apply_skin_changes` | `Update` | Watches `ActiveSkin` changes → swaps sprite images on bone entities |
| `handle_animation_events` | `Update` | Observes typed events fired by `clip.add_event(time, MyEvent)` on the `AnimationPlayer` entity |
| `apply_hit_reactions` | `Update` | Procedural additive transforms for hit reactions (Overgrowth-inspired) |

### Hot-Reload Behavior for Rig Assets

Two distinct cases when assets change on disk:

**`*.anim.ron` changes** (animation keyframes tweaked): The derived `AnimationClip` is rebuilt in-place via `clips.insert(existing_id, new_clip)`. All existing bone entities and the `AnimationPlayer` are unaffected — they transparently pick up the new clip data. No respawning needed.

**`*.rig.ron` changes** (bone hierarchy, slot definitions, or skin mappings changed): The bone entity hierarchy itself is stale. The correct approach: watch for `AssetEvent::Modified` on `SpriteRigAsset`, despawn all existing bone children of affected character roots, then respawn from the new rig definition. The character root entity itself stays; only the bone subtree is rebuilt.

In practice, `*.rig.ron` changes only happen during development. At runtime (shipped game), asset hot-reload is disabled, so this is purely a dev workflow concern.

---

## Part 7: Connections to Game Vision

The sprite rig system directly enables VISION.md features:

| Vision Feature | Sprite Rig Mechanism |
|---|---|
| Stat-driven appearance (muscular, sleek) | Skin swapping: different sprite sets per body type |
| Alignment hue-shifting | `Sprite.color` tinting on all bone entities |
| Inherited phenotypes | Skin selection based on genetic data |
| Body part customization | Per-slot attachment swapping (independent of skin) |
| Training-based visual evolution | Gradual skin transitions as stats cross thresholds |

### Bone Transform Variations (Proportions and Shape)

Yes — bone offsets in `*.rig.ron` define default positions, but these are just starting values. A `BoneOverrides` component on the character root can carry per-bone transform adjustments applied at spawn time (or updated at runtime):

```rust
pub struct BoneOverrides {
    pub offsets: HashMap<String, Vec2>,  // additive offset per bone name
    pub scales:  HashMap<String, Vec2>,  // multiplicative scale per bone name
}
```

Examples of what this enables:

| Stat Effect | Bone Override |
|---|---|
| Tall/large body type | Root-to-torso offset scaled up |
| Wide shoulders (high strength) | `arm_l`/`arm_r` x-offset increased |
| Long legs (high agility) | `leg_l`/`leg_r` y-offset extended |
| Large head (cosmetic trait) | `head` scale increased |

Overrides compose additively onto the rig's defaults. Animations are authored against the default proportions and remain valid — stretching an arm bone longer means the arm sweeps through a larger arc, which looks natural without re-authoring the animation.

---

## Code References

- `crates/render/src/lib.rs` — Current character rendering (capsule placeholders to be replaced)
- `crates/web/Cargo.toml` — Already enables `bevy_animation` and `bevy_sprite` features
- `crates/render/src/health_bar.rs` — Billboard UI pattern, useful reference for per-character child entities
- `doc/research/2025-09-30-sonic-battle-chao-design-research.md` — `CharacterAppearance` component proposal
- `doc/scratch/stats.md` — Per-stat visual impact descriptions

## Historical Context (from doc/)

- `doc/research/2025-09-30-sonic-battle-chao-design-research.md` — Proposes `CharacterAppearance` component with body part customization and visual evolution. This research predates any animation implementation.
- `doc/scratch/stats.md` — Describes how each stat should visually affect animations (aggressive animations for high power, controlled idle for high vitality, etc.)
- `doc/scratch/vision-theorycrafting.md` — Notes on character selection, appearance inheritance from breeding
- `doc/research/2026-02-25-ability-slots-hot-reload-asset.md` — Established pattern for hot-reloadable RON assets with `bevy_common_assets`

## Related Research

- `doc/research/2025-09-30-sonic-battle-chao-design-research.md` — Character design and appearance evolution
- `doc/research/2026-02-07-ability-system-architecture.md` — Ability system (triggers animation events)
- `doc/research/2026-02-13-hit-detection-system.md` — Hit detection (triggers hit reaction animations)
- `doc/research/2026-02-25-ability-slots-hot-reload-asset.md` — Asset loading patterns to follow

## Resolved Design Decisions

1. **2.5D rendering approach** — Sprite rigs render as **billboarded sprites** in the 3D world. Bone transforms are 2D (translation x/y, rotation around z, scale x/y). The billboard system orients the entire rig toward the camera. This means the rig hierarchy uses `Transform` with only x/y translation and z-axis rotation.

2. **Sprite authoring pipeline** — Sprites are individual PNG files. Authoring tools are out of scope. **Anchors and pivots are defined in the rig file** as part of each slot's attachment definition (the `anchor` field in `*.rig.ron`), not embedded in the image files.

3. **AnimationClip build strategy** — See detailed analysis below.

4. **Lightyear replication** — Animations are **client-local only**. No animation state replicates. The server sees characters as capsule colliders; animation is purely cosmetic. Each client independently derives animation state from replicated game state (velocity, ability phase, etc.).

5. **Spine vs custom** — **Custom implementation.** Full control, no license dependencies, tailored to the project's data-driven RON asset workflow.

6. **Atlas packing** — **Individual PNG images** per body part. Simpler workflow, easier hot-reload, easier per-part swapping for appearance evolution.

7. **Animation ↔ ability bridge** — See detailed analysis below.

---

## Follow-up Research: AnimationClip Build Strategy

### The Problem

`AnimationClip` stores curves keyed by `AnimationTargetId` (a UUID). Each bone entity needs an `AnimationTarget` component with a matching UUID. The question: when and how are `AnimationClip`s built from the RON `*.anim.ron` data?

### Option A: Build at Spawn Time (Per-Instance)

Each time a character spawns:
1. Spawn bone entities, each getting a unique `AnimationTargetId` (e.g., `["character_42", "torso"].into_iter().collect::<AnimationTargetId>()`)
2. Build `AnimationClip` assets on the fly, populating curves keyed to those specific target IDs
3. Build `AnimationGraph` referencing those clips
4. Insert `AnimationPlayer` + `AnimationGraphHandle`

**Pros**: Straightforward. Each instance has its own clips and graph. No shared state complications.

**Cons**: Duplicates `AnimationClip` assets per character instance. Building clips every spawn has CPU cost. Every clip is a unique asset, so Bevy can't deduplicate them.

### Option B: Build at Load Time (Shared, Deterministic IDs)

When `*.anim.ron` assets finish loading:
1. Build `AnimationClip` assets once, using **deterministic bone-name-only** target IDs (e.g., `AnimationTargetId::from_name(&Name::new("torso"))`)
2. Store the built clips as assets, referenced by the animation set
3. At spawn time, each bone entity gets an `AnimationTarget` with the same deterministic ID and `player` pointing to its root
4. All instances of the same rig type share the same `AnimationClip` and `AnimationGraph` assets

**Pros**: Clips built once and shared across all instances. Memory efficient. No per-spawn clip creation cost. Hot-reload of `*.anim.ron` rebuilds the shared clip, all characters update.

**Cons**: Requires that `AnimationTargetId`s are derived purely from bone names (no instance-specific path). Bevy's `AnimationPlayer` resolves targets by searching its entity's descendants for matching `AnimationTarget.id` — so if two characters share the same bone names, each player only animates its own descendants (this is correct behavior).

### Recommendation: Option B (Shared, Load-Time)

Option B is the right choice. Bevy's animation system already handles this correctly — `AnimationPlayer` only affects entities that are descendants of the entity it's on AND have a matching `AnimationTarget.id`. So two characters can share the same `AnimationClip` with `AnimationTargetId::from_name(&Name::new("torso"))`, and each player will only animate its own "torso" child.

The build pipeline:
1. `*.anim.ron` loads via `bevy_common_assets` → `SpriteAnimAsset`
2. An `AssetEvent::Added` observer triggers clip building: iterate bone timelines, create `AnimatableCurve`s, add to a new `AnimationClip`, store as asset
3. The built `Handle<AnimationClip>` is stored in a resource or associated asset (e.g., a `BuiltAnimations` resource mapping animation name → clip handle)
4. At spawn time, the `AnimationGraph` is built referencing the pre-built clip handles, and the graph handle is shared across instances of the same rig type

---

## Follow-up Research: Animation ↔ Ability Bridge

### Constraint

**Abilities must not know about animations.** The ability system (`crates/protocol/src/ability.rs`) is server-authoritative, tick-based, and has zero rendering dependencies. Animation is client-local cosmetic.

### How Abilities Currently Work (Summary)

- `AbilityId(String)` identifies abilities (derived from filenames: `punch.ability.ron` → `AbilityId("punch")`)
- `ActiveAbility` component tracks phase: `Startup` → `Active` → `Recovery`, each with tick durations from `AbilityDef`
- `EffectTrigger` dispatches gameplay effects at tick offsets (`OnTick`, `WhileActive`, `OnHit`, `OnEnd`, `OnInput`)
- All timing is tick-based, deterministic, server-authoritative

### The Bridge: AnimationSet Maps AbilityId → Animation

The `*.animset.ron` file is the bridge. It lives on the client/render side and maps ability IDs to animation clips. The ability system doesn't reference it; the animation system reads it.

```ron
// brawler.animset.ron
(
    rig: "rigs/brawler.rig.ron",
    locomotion: (
        blend_parameter: "speed",
        entries: [
            (clip: "anims/brawler/idle.anim.ron", threshold: 0.0),
            (clip: "anims/brawler/walk.anim.ron", threshold: 3.0),
            (clip: "anims/brawler/run.anim.ron",  threshold: 8.0),
        ],
    ),
    ability_animations: {
        "punch":        (clip: "anims/brawler/punch.anim.ron",        phase: Startup),
        "blink_strike": (clip: "anims/brawler/blink_strike.anim.ron", phase: Startup),
        "ground_pound": (clip: "anims/brawler/ground_pound.anim.ron", phase: Startup),
    },
    hit_react: "anims/brawler/hit_react.anim.ron",
)
```

### How the Animation System Reads Ability State

A client-side system observes replicated `ActiveAbility` entities:

```rs
fn update_character_animations(
    // ActiveAbility lives on a SEPARATE standalone entity (not the character).
    // The ActiveAbility component has a `caster` field pointing to the character entity.
    abilities: Query<(&ActiveAbility, &AbilityPhase, &AbilityId)>,
    mut characters: Query<(&mut AnimationPlayer, &AnimationSet), With<CharacterMarker>>,
) {
    for (ability, phase, ability_id) in &abilities {
        // Resolve caster -> character entity via ability.caster
        if let Ok((mut player, anim_set)) = characters.get_mut(ability.caster) {
            // Look up ability_animations[ability_id] in the AnimationSet
            if let Some(clip) = anim_set.ability_animations.get(&ability_id.0) {
                // Play the clip with a short crossfade
            }
        }
    }
    // Characters with no active ability default to the locomotion blend tree
}
```

The data flow:
1. Server spawns `ActiveAbility` with `AbilityId("punch")`, replicates to clients
2. Client animation system sees character has active ability with `AbilityId("punch")`
3. Looks up `"punch"` in the character's `ability_animations` map → finds the animation clip
4. Plays that clip on the character's `AnimationPlayer`
5. When `ActiveAbility` despawns (recovery ends), transitions back to locomotion

### Two Kinds of Animation Events

There is a critical distinction between **gameplay events** and **cosmetic events**:

| Type | Source | Examples | Where it runs |
|---|---|---|---|
| **Gameplay events** | Ability system (`EffectTrigger`) | Hitbox spawn, damage, teleport, force | Server + client (predicted) |
| **Cosmetic events** | Animation timeline (`*.anim.ron` events) | Footstep sound, dust particle, screen shake, whoosh sound | Client only |

These are completely separate systems:
- **Gameplay timing** is driven by tick offsets in `AbilityDef.effect_triggers` — authoritative, deterministic
- **Cosmetic timing** is driven by keyframe times in `*.anim.ron` events — client-local, approximate

Animation events in `*.anim.ron` should only trigger cosmetic effects:

```ron
events: [
    (time: 0.1, name: "whoosh_sound"),
    (time: 0.15, name: "dust_burst"),
    (time: 0.3, name: "impact_sound"),
]
```

The animation event handler dispatches these to client-only systems (audio, particles, camera effects). It never touches gameplay state.

### Why Not Synchronize Animation Timing With Ability Timing?

The ability system runs in `FixedUpdate` at a fixed tick rate. Animations run in `Update` at variable frame rate. These are intentionally decoupled:
- Ability phases have tick durations (e.g., startup: 3 ticks, active: 5 ticks, recovery: 4 ticks)
- Animation clips have time durations (e.g., 0.4 seconds)
- The animation system maps ability phases to animation playback independently

If an ability's startup is 3 ticks at 64Hz = ~47ms, the animation clip for that ability's startup should be authored to roughly match that duration. But exact synchronization isn't needed — the animation is cosmetic, the hitbox timing is authoritative.

## Resolved Design Decisions (Follow-up)

8. **Billboard implementation** — Use **option A**: a system that sets each rig root's rotation to face the camera each frame. Simple and sufficient for now. Can be upgraded to a shader approach later if needed.

9. **Animation clip hot-reload** — See detailed analysis below.

10. **Facing direction** — Use `flip_x` on all bone sprites based on the character's facing direction. A `Facing` component on the character root drives `Sprite.flip_x` on all bone child entities.

---

## Follow-up Research: Animation Clip Hot-Reload

### The Problem

When a `*.anim.ron` file changes on disk, the `SpriteAnimAsset` is hot-reloaded by Bevy's `AssetServer`. But the derived `AnimationClip` (built programmatically from the RON data) must also be rebuilt.

### Solution: Manual AssetEvent Listener

Bevy has no built-in "derived asset" abstraction. The correct pattern is to listen for `AssetEvent::Modified` on the source asset and rebuild the derived asset in-place.

```rust
fn rebuild_animation_clips(
    mut ev: EventReader<AssetEvent<SpriteAnimAsset>>,
    source_assets: Res<Assets<SpriteAnimAsset>>,
    mut clips: ResMut<Assets<AnimationClip>>,
    mut registry: ResMut<BuiltAnimations>, // maps source AssetId -> derived Handle<AnimationClip>
) {
    for event in ev.read() {
        if let AssetEvent::Modified { id } = event {
            let source = source_assets.get(*id).unwrap();
            let new_clip = build_clip_from(source);
            let clip_handle = registry.get(*id);
            // Replace in-place — all entities holding this handle see the new data
            clips.insert(clip_handle.id(), new_clip);
        }
    }
}
```

### Key Details

- **No feedback loop**: Reading `Assets<SpriteAnimAsset>` and writing `Assets<AnimationClip>` are different type collections, so no re-triggered `Modified` events.
- **Handle stability**: `Handle<AnimationClip>` is just an `AssetId` wrapper. Calling `clips.insert(id, new_clip)` replaces the data at that ID. All entities holding that handle automatically resolve to the new clip on next access.
- **`AssetChanged<T>` filter**: Bevy provides `Query<..., AssetChanged<AnimationClip>>` to react to replaced assets. Fires the frame after modification.
- **Initial build**: On `AssetEvent::LoadedWithDependencies`, use `clips.add(clip)` to get the initial handle. Store it in the registry.
- **No dependency propagation**: [PR #20575](https://github.com/bevyengine/bevy/pull/20575) for auto-propagating Modified events is unmerged. Must listen on the source type directly.

### Build Pipeline Summary

```
[*.anim.ron on disk]
    | (AssetServer hot-reload)
    v
Assets<SpriteAnimAsset>
    | AssetEvent::LoadedWithDependencies → build_clip_from(), clips.add()
    | AssetEvent::Modified → build_clip_from(), clips.insert(existing_id)
    v
Assets<AnimationClip>  (derived, in-place replacement)
    |
    v
Entities with Handle<AnimationClip> (transparently see updated data)
```

Sources:
- [Bevy Cheat Book: Asset Events](https://bevy-cheatbook.github.io/assets/assetevent.html)
- [Bevy Derived Assets Discussion #9296](https://github.com/bevyengine/bevy/discussions/9296)
- [AssetChanged PR #16810](https://github.com/bevyengine/bevy/pull/16810)

---

## Verified Bevy 0.17 API Reference

Signatures confirmed against docs.rs/bevy/0.17.3.

### `AnimationTargetId`

```rust
// From a single &Name:
pub fn from_name(name: &Name) -> AnimationTargetId

// From a root-to-leaf path of &Name values (order matters — different paths → different IDs):
pub fn from_names<'a>(names: impl Iterator<Item = &'a Name>) -> AnimationTargetId

// FromIterator impl allows collecting string slices:
impl<T: AsRef<str>> FromIterator<T> for AnimationTargetId
// Usage: ["root", "torso", "head"].into_iter().collect::<AnimationTargetId>()
```

**Critical**: the UUID is derived from the **full path**, not just the leaf name. `from_name(&Name::new("torso"))` and `from_names([root_name, torso_name].iter())` produce **different IDs**. For the shared-clip strategy (Option B), use `from_name` with just the bone name — then all instances of the same rig type share target IDs, and `AnimationPlayer` scoping prevents cross-character interference.

### `UnevenSampleAutoCurve::new`

```rust
pub fn new(
    timed_samples: impl IntoIterator<Item = (f32, T)>,
) -> Result<UnevenSampleAutoCurve<T>, UnevenCoreError>
```

- Input is **`(time, value)` tuples** — not separate arrays of times and values.
- Returns `Err(UnevenCoreError)` if fewer than 2 samples are provided.
- Samples need not be pre-sorted; infinite/NaN times are filtered out.
- Interpolation uses `T: StableInterpolate` (lerp for `f32`/`Vec3`, nlerp for `Quat`).

```rust
// Correct usage:
UnevenSampleAutoCurve::new([
    (0.0, Vec3::ZERO),
    (0.3, Vec3::new(0.0, 5.0, 0.0)),
    (0.6, Vec3::ZERO),
]).unwrap()
```

### Animation Events — `AnimationClip::add_event` (No `AnimationPlayer::animation_events`)

`AnimationPlayer` has **no `animation_events()` method**. The mechanism is observer-based:

```rust
// 1. Define an event type:
#[derive(Event, Clone, Reflect)]
#[derive(AnimationEvent)]  // required
struct FootstepEvent;

// 2. Register on the clip at a timestamp (fires on the AnimationPlayer entity):
clip.add_event(0.15, FootstepEvent);

// Or target a specific bone entity:
clip.add_event_to_target(bone_target_id, 0.15, FootstepEvent);

// Or use a closure (no trait required):
clip.add_event_fn(0.15, |commands, entity, _time, _weight| {
    // entity is the AnimationPlayer entity
});

// 3. Observe the event on the player entity:
commands.entity(player_entity).observe(|trigger: Trigger<FootstepEvent>| {
    // play sound, spawn particle, etc.
});
```

Full `AnimationClip` event method signatures:
```rust
pub fn add_event(&mut self, time: f32, event: impl AnimationEvent)
pub fn add_event_to_target(&mut self, target: AnimationTargetId, time: f32, event: impl AnimationEvent)
pub fn add_event_fn(&mut self, time: f32, func: impl Fn(&mut Commands, Entity, f32, f32) + Send + Sync + 'static)
pub fn add_event_fn_to_target(&mut self, target: AnimationTargetId, time: f32, func: impl Fn(&mut Commands, Entity, f32, f32) + Send + Sync + 'static)
```

Sources:
- [AnimationTargetId — docs.rs/bevy/0.17](https://docs.rs/bevy/0.17.3/bevy/animation/struct.AnimationTargetId.html)
- [UnevenSampleAutoCurve — docs.rs/bevy](https://docs.rs/bevy/latest/bevy/math/curve/struct.UnevenSampleAutoCurve.html)
- [AnimationClip — docs.rs/bevy/0.17](https://docs.rs/bevy/0.17.3/bevy/animation/struct.AnimationClip.html)
- [AnimationPlayer — docs.rs/bevy](https://docs.rs/bevy/latest/bevy/animation/struct.AnimationPlayer.html)
