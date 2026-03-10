---
date: 2026-03-08
author: claude
git_commit: 6f2f87c
branch: master
repository: bevy-lightyear-template
topic: "Sprite Rig Animation System"
status: draft
---

# Sprite Rig Animation System — Implementation Plan

## Overview

Replace the 3D capsule character visuals with data-driven 2D sprite rigs: characters composed of
separate sprite images per body part (head, torso, arms, legs), animated via Bevy's native
`AnimationClip`/`AnimationGraph`/`AnimationTransitions` primitives. All animation data loads from
RON asset files. Animations are client-local cosmetic only — the server never sees them.

The system supports **multiple rig types** via a replicated `CharacterType` component. Each
`CharacterType` maps to an animset asset on the client. For now, only one type exists (`Humanoid`),
but the architecture handles N types without structural changes.

Research source: `doc/research/2026-03-07-sprite-rig-animation-overgrowth.md`

---

## Current State

- Characters render as `Capsule3d` + `StandardMaterial` in `crates/render/src/lib.rs:86-90`
- `bevy_common_assets` 0.14 and `serde` already in workspace `Cargo.toml`
- `bevy_animation` and `bevy_sprite` already enabled in `crates/web/Cargo.toml`; native build uses
  `default-features = true` so these are already available in the render crate
- No sprite, image, or animation assets exist in `assets/`
- No `CharacterType` concept exists — all characters are structurally identical (`CharacterMarker`)

## Desired End State

Characters visually appear as 2D sprite rigs billboarded in the 3D world. Bone sprites animate
through idle/walk/run locomotion states driven by velocity. Ability activations trigger corresponding
animations. The rig billboard flips horizontally based on facing direction.

Each character has a `CharacterType` (replicated from server) that determines which rig and
animation set is used on the client. Adding a new character type requires: a new `CharacterType`
variant, a new animset RON file, and the corresponding rig/animation RON files.

### Verification
- `cargo client` — characters appear as colored rectangle bone hierarchies (placeholders for real
  sprites), animating through idle/walk/run as characters move
- Ability activation causes the matching ability animation to play, then returns to locomotion
- Characters face the camera (billboard)
- Characters flip when moving left vs right

---

## What We Are NOT Doing

- Actual artist-authored sprite PNG files (placeholder programmatic sprites only)

- Full distance-driven locomotion phase advancement (Overgrowth-style, where animation phase
  advances by distance traveled rather than time — out of scope, standard time-based playback is
  sufficient for now)

- Spine runtime or any external animation library
- Skin swapping / appearance evolution (separate plan)
- IK or procedural hit reactions (separate plan)
- Audio/particle animation events (hooks defined, no implementation)
- Server-side animation knowledge of any kind
- Multiple character types beyond `Humanoid` (architecture supports it, content does not ship yet)

---

## Architecture

Animation code lives in a dedicated `crates/sprite_rig/` workspace crate. `RenderPlugin` depends on
it and adds `SpriteRigPlugin` during its build.

### Multi-rig design

The system uses a **registry pattern** rather than global singleton resources:

| Concept | Type | Description |
|---|---|---|
| `CharacterType` | Replicated component | Server-assigned enum (`Humanoid`, future variants). Lives in `protocol`. |
| `RigRegistry` | Resource | Maps `CharacterType` → animset asset path. Defined in `sprite_rig` crate. |
| `AnimSetRef` | Component | Per-character `Handle<SpriteAnimSetAsset>`, resolved from `CharacterType` on the client. |
| `BuiltAnimGraphs` | Resource | `HashMap<AssetId<SpriteAnimSetAsset>, BuiltAnimGraph>`. One graph per animset type, shared across all instances of that type. |

Data flow:
```
Server spawns character with CharacterType::Humanoid
  → Replicated to client
  → Client render system sees CharacterType, looks up RigRegistry
  → Inserts AnimSetRef(handle) + SpriteRig(rig_handle)
  → Graph built per-animset (shared), AnimationPlayer per-instance
```

### Crate setup

**New workspace member**: `crates/sprite_rig/`

**`Cargo.toml`** (workspace root): Add `"crates/sprite_rig"` to `members` list.

**`crates/sprite_rig/Cargo.toml`**:
```toml
[package]
name = "sprite_rig"
version = "0.1.0"
edition = "2021"

[dependencies]
avian3d = { workspace = true }
bevy = { workspace = true, features = ["bevy_animation"] }
bevy_common_assets = { workspace = true }
protocol = { workspace = true }
serde = { workspace = true, features = ["derive"] }

[dev-dependencies]
approx = { workspace = true }
test-log = { workspace = true }
```

**`crates/render/Cargo.toml`**: Add `sprite_rig = { path = "../sprite_rig" }`.

### Module layout

```
crates/sprite_rig/
  src/
    lib.rs         — SpriteRigPlugin, re-exports
    asset.rs       — SpriteRigAsset, SpriteAnimAsset, SpriteAnimSetAsset + type definitions
    spawn.rs       — rig spawning, billboard, facing systems
    animation.rs   — AnimationClip building, AnimationGraph setup, blend-tree locomotion
    animset.rs     — SpriteAnimSetAsset loading, ability animation bridge
  tests/
    sprite_rig.rs  — Integration test: headless app, rig spawn, animation playback
```

### Key design decisions (from research)

1. **Shared AnimationClips (Option B)**: clips built once at load time using deterministic
   `AnimationTargetId::from_names(iter::once(&Name::new(bone_name)))`. All instances of the same
   rig share clip and graph assets. Per-instance `AnimationPlayer` + `AnimationTransitions` hold
   independent playback state.

2. **Billboard**: a `RigBillboard` child entity is spawned under the character; all bones are
   children of the billboard. A system rotates the billboard to face the camera each frame
   (same pattern as `health_bar::billboard_face_camera`).

3. **Placeholder quads**: `Mesh3d(Plane3d)` + `MeshMaterial3d(StandardMaterial { unlit: true, cull_mode: None })` —
   colored 3D quads visible in the 3D scene. No image files required for placeholders.

4. **Hot-reload**: `AssetEvent::Modified { id }` on `SpriteAnimAsset` triggers in-place clip
   replacement via `clips.insert(handle.id(), new_clip)`.

5. **Capsule removed**: the sprite rig replaces the capsule mesh. The physics collider (capsule)
   remains but the visual capsule is removed.

---

## Phase 1: CharacterType + Asset Type Definitions and Loading Infrastructure

### Overview

Add `CharacterType` to protocol. Define Rust types for `.rig.ron`, `.anim.ron`, and `.animset.ron`
assets. Register `RonAssetPlugin` for each. Set up `RigRegistry` to map character types to animset
paths. Load assets at startup. No visuals yet.

### 1.1 CharacterType in protocol

**File**: `crates/protocol/src/lib.rs`

Add a replicated+predicted component that identifies what kind of character this is:

```rust
/// Determines which sprite rig and animation set a character uses on the client.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect, Default)]
pub enum CharacterType {
    #[default]
    Humanoid,
}
```

Register for replication with prediction in `ProtocolPlugin::build`:

```rust
app.register_component::<CharacterType>().add_prediction();
```

Add `CharacterType` to the character spawn in `crates/server/src/gameplay.rs`:

In `handle_connected` (player spawn), add `CharacterType::Humanoid` to the spawn bundle.

In `spawn_dummy_target`, add `CharacterType::Humanoid` to the spawn bundle.

Export `CharacterType` from `crates/protocol/src/lib.rs` pub use block.

### 1.2 Asset type definitions

**File**: `crates/sprite_rig/src/asset.rs`

```rust
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Defines a character's bone hierarchy, sprite slots, and skin variants.
#[derive(Clone, Debug, Serialize, Deserialize, Asset, TypePath)]
pub struct SpriteRigAsset {
    pub bones: Vec<BoneDef>,
    pub slots: Vec<SlotDef>,
    pub skins: HashMap<String, HashMap<String, AttachmentDef>>,
}

/// A single bone in the rig hierarchy.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoneDef {
    pub name: String,
    pub parent: Option<String>,
    pub default_transform: BoneTransform2d,
}

/// 2D transform for a bone: translation (x, y), rotation (degrees), scale (x, y).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoneTransform2d {
    pub translation: Vec2,
    pub rotation: f32,
    pub scale: Vec2,
}

impl Default for BoneTransform2d {
    fn default() -> Self {
        Self { translation: Vec2::ZERO, rotation: 0.0, scale: Vec2::ONE }
    }
}

/// A draw-order slot attached to a bone.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SlotDef {
    pub name: String,
    pub bone: String,
    pub z_order: f32,
    pub default_attachment: String,
}

/// A sprite image attachment for a slot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttachmentDef {
    pub image: String,
    pub anchor: SpriteAnchorDef,
    pub size: Vec2,
}

/// Sprite anchor point.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub enum SpriteAnchorDef {
    #[default]
    Center,
    TopCenter,
    BottomCenter,
}

/// Keyframed animation for a set of bones.
#[derive(Clone, Debug, Serialize, Deserialize, Asset, TypePath)]
pub struct SpriteAnimAsset {
    pub name: String,
    pub duration: f32,
    pub looping: bool,
    pub bone_timelines: HashMap<String, BoneTimeline>,
    pub events: Vec<AnimEventKeyframe>,
}

/// Keyframe timelines for a single bone's transform channels.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BoneTimeline {
    pub rotation: Vec<RotationKeyframe>,
    pub translation: Vec<TranslationKeyframe>,
    pub scale: Vec<ScaleKeyframe>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RotationKeyframe {
    pub time: f32,
    pub value: f32,
    pub curve: CurveType,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranslationKeyframe {
    pub time: f32,
    pub value: Vec2,
    pub curve: CurveType,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScaleKeyframe {
    pub time: f32,
    pub value: Vec2,
    pub curve: CurveType,
}

/// Interpolation curve type between keyframes.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub enum CurveType {
    #[default]
    Linear,
    Step,
}

/// A named event fired at a specific time during animation playback.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnimEventKeyframe {
    pub time: f32,
    pub name: String,
}

/// Maps locomotion states and ability IDs to animation clips for a rig.
#[derive(Clone, Debug, Serialize, Deserialize, Asset, TypePath)]
pub struct SpriteAnimSetAsset {
    pub rig: String,
    pub locomotion: LocomotionConfig,
    pub ability_animations: HashMap<String, String>,
    pub hit_react: Option<String>,
}

/// Locomotion blend tree configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocomotionConfig {
    pub entries: Vec<LocomotionEntry>,
}

/// A single entry in the locomotion blend tree.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocomotionEntry {
    pub clip: String,
    pub speed_threshold: f32,
}
```

### 1.3 Example RON asset files

**File**: `assets/rigs/humanoid.rig.ron`

```ron
#![enable(implicit_some)]
(
    bones: [
        (name: "root",  parent: None,          default_transform: (translation: (0.0, 0.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "torso", parent: Some("root"),  default_transform: (translation: (0.0, 1.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "head",  parent: Some("torso"), default_transform: (translation: (0.0, 1.8), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "arm_l", parent: Some("torso"), default_transform: (translation: (-1.2, 0.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "arm_r", parent: Some("torso"), default_transform: (translation: (1.2, 0.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "leg_l", parent: Some("root"),  default_transform: (translation: (-0.5, -1.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "leg_r", parent: Some("root"),  default_transform: (translation: (0.5, -1.0),  rotation: 0.0, scale: (1.0, 1.0))),
    ],
    slots: [
        (name: "torso", bone: "torso", z_order: 0.0,  default_attachment: "torso_default"),
        (name: "head",  bone: "head",  z_order: 0.3,  default_attachment: "head_default"),
        (name: "arm_l", bone: "arm_l", z_order: -0.1, default_attachment: "arm_default"),
        (name: "arm_r", bone: "arm_r", z_order: 0.1,  default_attachment: "arm_default"),
        (name: "leg_l", bone: "leg_l", z_order: -0.2, default_attachment: "leg_default"),
        (name: "leg_r", bone: "leg_r", z_order: 0.2,  default_attachment: "leg_default"),
    ],
    skins: {
        "default": {
            "torso_default": (image: "sprites/humanoid/torso.png", anchor: Center,       size: (2.0, 2.5)),
            "head_default":  (image: "sprites/humanoid/head.png",  anchor: BottomCenter, size: (1.5, 1.5)),
            "arm_default":   (image: "sprites/humanoid/arm.png",   anchor: TopCenter,    size: (0.8, 2.0)),
            "leg_default":   (image: "sprites/humanoid/leg.png",   anchor: TopCenter,    size: (1.0, 2.5)),
        },
    },
)
```

**File**: `assets/anims/humanoid/idle.anim.ron`

```ron
#![enable(implicit_some)]
(
    name: "idle",
    duration: 2.0,
    looping: true,
    bone_timelines: {
        "torso": (
            rotation: [
                (time: 0.0, value: 0.0,  curve: Linear),
                (time: 1.0, value: 1.5,  curve: Linear),
                (time: 2.0, value: 0.0,  curve: Linear),
            ],
            translation: [],
            scale: [],
        ),
        "head": (
            rotation: [
                (time: 0.0, value: 0.0,  curve: Linear),
                (time: 1.0, value: -1.0, curve: Linear),
                (time: 2.0, value: 0.0,  curve: Linear),
            ],
            translation: [],
            scale: [],
        ),
    },
    events: [],
)
```

**File**: `assets/anims/humanoid/walk.anim.ron`

Translation values are **(X, Y)** offsets from the bone's default position. X = horizontal
(forward/back in the character's facing direction), Y = vertical (up/down). Rotation alone
pivots limbs around their attach point; translation adds the reaching/retracting motion that
makes a pendulum swing or elliptical step path.

Arms swing forward (+X) and back (-X) like pendulums, with slight vertical drop (-Y) at the
extremes. Legs trace ellipses: forward (+X) and up (+Y) during the swing phase, then back to
origin during the stance phase. Torso bobs vertically with each step.

```ron
#![enable(implicit_some)]
(
    name: "walk",
    duration: 0.6,
    looping: true,
    bone_timelines: {
        "torso": (
            rotation: [(time: 0.0, value: 0.0, curve: Linear), (time: 0.3, value: 3.0, curve: Linear), (time: 0.6, value: 0.0, curve: Linear)],
            translation: [
                (time: 0.0,  value: (0.0, 0.0),  curve: Linear),
                (time: 0.15, value: (0.0, 0.15), curve: Linear),
                (time: 0.3,  value: (0.0, 0.0),  curve: Linear),
                (time: 0.45, value: (0.0, 0.15), curve: Linear),
                (time: 0.6,  value: (0.0, 0.0),  curve: Linear),
            ],
            scale: [],
        ),
        "arm_l": (
            // Opposite to legs: arm swings back when same-side leg swings forward
            rotation: [(time: 0.0, value: 15.0, curve: Linear), (time: 0.3, value: -15.0, curve: Linear), (time: 0.6, value: 15.0, curve: Linear)],
            translation: [
                (time: 0.0,  value: (0.3, -0.05),   curve: Linear),
                (time: 0.15, value: (0.0, 0.0),     curve: Linear),
                (time: 0.3,  value: (-0.3, -0.05),  curve: Linear),
                (time: 0.45, value: (0.0, 0.0),     curve: Linear),
                (time: 0.6,  value: (0.3, -0.05),   curve: Linear),
            ],
            scale: [],
        ),
        "arm_r": (
            rotation: [(time: 0.0, value: -15.0, curve: Linear), (time: 0.3, value: 15.0, curve: Linear), (time: 0.6, value: -15.0, curve: Linear)],
            translation: [
                (time: 0.0,  value: (-0.3, -0.05), curve: Linear),
                (time: 0.15, value: (0.0, 0.0),    curve: Linear),
                (time: 0.3,  value: (0.3, -0.05),  curve: Linear),
                (time: 0.45, value: (0.0, 0.0),    curve: Linear),
                (time: 0.6,  value: (-0.3, -0.05), curve: Linear),
            ],
            scale: [],
        ),
        "leg_l": (
            // 0.0-0.3: swing phase (foot sweeps forward and lifts, elliptical arc)
            // 0.3-0.6: stance phase (foot plants and pushes back)
            rotation: [(time: 0.0, value: -20.0, curve: Linear), (time: 0.3, value: 20.0, curve: Linear), (time: 0.6, value: -20.0, curve: Linear)],
            translation: [
                (time: 0.0,  value: (-0.2, 0.0),   curve: Linear),
                (time: 0.08, value: (-0.05, 0.15), curve: Linear),
                (time: 0.15, value: (0.1, 0.3),    curve: Linear),
                (time: 0.22, value: (0.2, 0.15),   curve: Linear),
                (time: 0.3,  value: (0.2, 0.0),    curve: Linear),
                (time: 0.6,  value: (-0.2, 0.0),   curve: Linear),
            ],
            scale: [],
        ),
        "leg_r": (
            // 0.0-0.3: stance phase (foot plants and pushes back)
            // 0.3-0.6: swing phase (foot sweeps forward and lifts, elliptical arc)
            rotation: [(time: 0.0, value: 20.0, curve: Linear), (time: 0.3, value: -20.0, curve: Linear), (time: 0.6, value: 20.0, curve: Linear)],
            translation: [
                (time: 0.0,  value: (0.2, 0.0),   curve: Linear),
                (time: 0.3,  value: (-0.2, 0.0),  curve: Linear),
                (time: 0.38, value: (-0.05, 0.15), curve: Linear),
                (time: 0.45, value: (0.1, 0.3),   curve: Linear),
                (time: 0.52, value: (0.2, 0.15),  curve: Linear),
                (time: 0.6,  value: (0.2, 0.0),   curve: Linear),
            ],
            scale: [],
        ),
    },
    events: [
        (time: 0.15, name: "footstep_left"),
        (time: 0.45, name: "footstep_right"),
    ],
)
```

**File**: `assets/anims/humanoid/run.anim.ron`

Same pendulum/ellipse pattern as walk but exaggerated. Arms pump with shorter X swing
(tucked elbows — constant inward X offset) but bigger Y bob. Legs trace wider ellipses
with higher lift and more forward reach. Torso has more pronounced vertical bounce and a
forward lean.

```ron
#![enable(implicit_some)]
(
    name: "run",
    duration: 0.4,
    looping: true,
    bone_timelines: {
        "torso": (
            rotation: [(time: 0.0, value: -5.0, curve: Linear), (time: 0.2, value: -8.0, curve: Linear), (time: 0.4, value: -5.0, curve: Linear)],
            translation: [
                (time: 0.0, value: (0.0, 0.0),  curve: Linear),
                (time: 0.1, value: (0.0, 0.25), curve: Linear),
                (time: 0.2, value: (0.0, 0.0),  curve: Linear),
                (time: 0.3, value: (0.0, 0.25), curve: Linear),
                (time: 0.4, value: (0.0, 0.0),  curve: Linear),
            ],
            scale: [],
        ),
        "arm_l": (
            // Tucked elbows: constant +X inward offset, shorter forward/back swing via Y
            rotation: [(time: 0.0, value: 30.0, curve: Linear), (time: 0.2, value: -30.0, curve: Linear), (time: 0.4, value: 30.0, curve: Linear)],
            translation: [
                (time: 0.0, value: (0.4, 0.15),  curve: Linear),
                (time: 0.1, value: (0.3, 0.0),   curve: Linear),
                (time: 0.2, value: (0.4, -0.15), curve: Linear),
                (time: 0.3, value: (0.3, 0.0),   curve: Linear),
                (time: 0.4, value: (0.4, 0.15),  curve: Linear),
            ],
            scale: [],
        ),
        "arm_r": (
            // Tucked elbows: constant -X inward offset
            rotation: [(time: 0.0, value: -30.0, curve: Linear), (time: 0.2, value: 30.0, curve: Linear), (time: 0.4, value: -30.0, curve: Linear)],
            translation: [
                (time: 0.0, value: (-0.4, -0.15), curve: Linear),
                (time: 0.1, value: (-0.3, 0.0),   curve: Linear),
                (time: 0.2, value: (-0.4, 0.15),  curve: Linear),
                (time: 0.3, value: (-0.3, 0.0),   curve: Linear),
                (time: 0.4, value: (-0.4, -0.15), curve: Linear),
            ],
            scale: [],
        ),
        "leg_l": (
            // 0.0-0.2: swing phase (wider ellipse than walk, higher knee lift)
            // 0.2-0.4: stance phase (push back)
            rotation: [(time: 0.0, value: -35.0, curve: Linear), (time: 0.2, value: 35.0, curve: Linear), (time: 0.4, value: -35.0, curve: Linear)],
            translation: [
                (time: 0.0,  value: (-0.3, 0.0),  curve: Linear),
                (time: 0.05, value: (-0.1, 0.25), curve: Linear),
                (time: 0.1,  value: (0.15, 0.5),  curve: Linear),
                (time: 0.15, value: (0.3, 0.25),  curve: Linear),
                (time: 0.2,  value: (0.3, 0.0),   curve: Linear),
                (time: 0.4,  value: (-0.3, 0.0),  curve: Linear),
            ],
            scale: [],
        ),
        "leg_r": (
            // 0.0-0.2: stance phase (push back)
            // 0.2-0.4: swing phase (wider ellipse than walk, higher knee lift)
            rotation: [(time: 0.0, value: 35.0, curve: Linear), (time: 0.2, value: -35.0, curve: Linear), (time: 0.4, value: 35.0, curve: Linear)],
            translation: [
                (time: 0.0,  value: (0.3, 0.0),   curve: Linear),
                (time: 0.2,  value: (-0.3, 0.0),  curve: Linear),
                (time: 0.25, value: (-0.1, 0.25), curve: Linear),
                (time: 0.3,  value: (0.15, 0.5),  curve: Linear),
                (time: 0.35, value: (0.3, 0.25),  curve: Linear),
                (time: 0.4,  value: (0.3, 0.0),   curve: Linear),
            ],
            scale: [],
        ),
    },
    events: [
        (time: 0.1, name: "footstep_left"),
        (time: 0.3, name: "footstep_right"),
    ],
)
```


**File**: `assets/anims/humanoid/punch.anim.ron`

```ron
#![enable(implicit_some)]
(
    name: "punch",
    duration: 0.35,
    looping: false,
    bone_timelines: {
        "torso": (
            rotation: [(time: 0.0, value: -8.0, curve: Linear), (time: 0.1, value: 8.0, curve: Linear), (time: 0.35, value: 0.0, curve: Linear)],
            translation: [],
            scale: [],
        ),
        "arm_r": (
            rotation: [(time: 0.0, value: -40.0, curve: Linear), (time: 0.1, value: 20.0, curve: Linear), (time: 0.35, value: 0.0, curve: Linear)],
            translation: [],
            scale: [],
        ),
    },
    events: [
        (time: 0.1, name: "punch_whoosh"),
    ],
)
```

**File**: `assets/anims/humanoid/humanoid.animset.ron`

```ron
#![enable(implicit_some)]
(
    rig: "rigs/humanoid.rig.ron",
    locomotion: (
        entries: [
            (clip: "anims/humanoid/idle.anim.ron", speed_threshold: 0.0),
            (clip: "anims/humanoid/walk.anim.ron", speed_threshold: 2.0),
            (clip: "anims/humanoid/run.anim.ron",  speed_threshold: 6.0),
        ],
    ),
    ability_animations: {
        "punch": "anims/humanoid/punch.anim.ron",
    },
    hit_react: None,
)
```

### 1.4 RigRegistry and plugin infrastructure

**File**: `crates/sprite_rig/src/lib.rs`

```rust
pub mod asset;

use asset::*;
use bevy::prelude::*;
use bevy_common_assets::ron::RonAssetPlugin;
use protocol::{app_state::TrackedAssets, CharacterType};
use std::collections::HashMap;

pub struct SpriteRigPlugin;

impl Plugin for SpriteRigPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            RonAssetPlugin::<SpriteRigAsset>::new(&["rig.ron"]),
            RonAssetPlugin::<SpriteAnimAsset>::new(&["anim.ron"]),
            RonAssetPlugin::<SpriteAnimSetAsset>::new(&["animset.ron"]),
        ));
        app.add_systems(Startup, load_rig_assets);
    }
}

/// Maps CharacterType to its animset asset path.
#[derive(Resource)]
pub struct RigRegistry {
    pub entries: HashMap<CharacterType, RigRegistryEntry>,
}

/// Loaded handles for one character type's rig and animset.
pub struct RigRegistryEntry {
    pub animset_handle: Handle<SpriteAnimSetAsset>,
    pub rig_handle: Handle<SpriteRigAsset>,
}

fn load_rig_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<TrackedAssets>,
) {
    let mut entries = HashMap::new();

    // Humanoid rig
    let animset_handle = asset_server.load::<SpriteAnimSetAsset>("anims/humanoid/humanoid.animset.ron");
    let rig_handle = asset_server.load::<SpriteRigAsset>("rigs/humanoid.rig.ron");
    tracked.add(animset_handle.clone());
    tracked.add(rig_handle.clone());
    entries.insert(CharacterType::Humanoid, RigRegistryEntry { animset_handle, rig_handle });

    // Future character types would add entries here.

    commands.insert_resource(RigRegistry { entries });
}
```

Add `sprite_rig::SpriteRigPlugin` to `RenderPlugin::build` in `crates/render/src/lib.rs`.

### Success Criteria — Phase 1

#### Automated Verification
- [x] `cargo check-all` passes with no errors
- [x] `cargo client` starts, logs show rig/anim assets loaded (no asset errors in console)

---

## Phase 2: Rig Spawning, Billboard, and Facing

### Overview

When a character with `CharacterType` appears on the client, resolve its rig from `RigRegistry`,
spawn the bone entity hierarchy, and set up billboarding and facing.

### 2.1 New components

**File**: `crates/sprite_rig/src/spawn.rs`

```rust
/// Reference to the rig asset for this character. Triggers rig spawning when added.
#[derive(Component)]
pub struct SpriteRig(pub Handle<SpriteRigAsset>);

/// Reference to the animset for this character. Used to resolve animation graph.
#[derive(Component)]
pub struct AnimSetRef(pub Handle<SpriteAnimSetAsset>);

/// Maps bone names to their spawned child entities.
#[derive(Component, Default)]
pub struct BoneEntities(pub HashMap<String, Entity>);

/// Which horizontal direction this character is facing.
#[derive(Component, PartialEq, Clone, Copy)]
pub enum Facing { Left, Right }

/// Marker for the billboard child entity that parents all bone entities.
#[derive(Component)]
pub struct RigBillboard;
```

### 2.2 CharacterType → rig resolution

**File**: `crates/sprite_rig/src/spawn.rs`

A system reacts to characters gaining `CharacterType` on the client (via replication) and inserts
the appropriate `SpriteRig` + `AnimSetRef` + `Facing` components:

```rust
fn resolve_character_rig(
    mut commands: Commands,
    query: Query<(Entity, &CharacterType), (Added<CharacterType>, Without<SpriteRig>)>,
    registry: Res<RigRegistry>,
) {
    for (entity, character_type) in &query {
        let entry = registry.entries.get(character_type)
            .expect("Every CharacterType variant must have a RigRegistry entry");
        commands.entity(entity).insert((
            SpriteRig(entry.rig_handle.clone()),
            AnimSetRef(entry.animset_handle.clone()),
            Facing::Right,
        ));
    }
}
```

### 2.3 Rig spawning system

**File**: `crates/sprite_rig/src/spawn.rs`

Trigger: `Added<SpriteRig>`

Steps:
1. Resolve the `SpriteRigAsset`. If not yet loaded, `debug_assert!` (should be loaded before
   `AppState::Ready`).
2. Build a parent map: `bone_name → parent_bone_name`. Find root bones (no parent).
3. Topological sort (parents before children). Bone count is small (<=15), simple repeated-pass
   sort is fine.
4. Resolve z_order per bone from `SpriteRigAsset.slots` (bone → z_order lookup, default 0.0).
5. Spawn billboard child entity with `RigBillboard` marker.
6. Spawn bone entities as children of the billboard. Each bone entity gets:
   - `Name::new(bone_name.clone())`
   - `Transform` built from `BoneDef.default_transform`:
     `Transform { translation: Vec3::new(x, y, z_order), rotation: Quat::from_rotation_z(degrees.to_radians()), scale: Vec3::new(sx, sy, 1.0) }`
   - `Mesh3d(Plane3d::new(Vec3::Z, size / 2.0))` + `MeshMaterial3d(StandardMaterial { base_color: color, unlit: true, cull_mode: None })` — unlit 3D quads as placeholders
7. Insert `BoneEntities` on the character root.

```rust
fn spawn_sprite_rigs(
    mut commands: Commands,
    query: Query<(Entity, &SpriteRig), Added<SpriteRig>>,
    rig_assets: Res<Assets<SpriteRigAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, rig_component) in &query {
        let Some(rig) = rig_assets.get(&rig_component.0) else {
            debug_assert!(false, "SpriteRig asset not loaded at spawn time");
            continue;
        };
        let slot_lookups = build_slot_lookups(rig);
        let sorted_bones = topological_sort_bones(&rig.bones);
        let billboard_id = spawn_billboard(&mut commands, entity);
        let bone_map = spawn_bone_hierarchy(
            &mut commands, billboard_id, &sorted_bones, &slot_lookups,
            &mut meshes, &mut materials,
        );
        commands.entity(entity).insert(BoneEntities(bone_map));
    }
}
```

`spawn_bone_hierarchy` returns `HashMap<String, Entity>`. Bone entities are children of the
billboard entity.

### 2.4 Billboard system

Each frame, rotate the `RigBillboard` child entity to face the camera (project onto the XZ plane).
Follows the existing `billboard_face_camera` pattern in `health_bar.rs`.

```rust
fn billboard_rigs_face_camera(
    camera_query: Query<&GlobalTransform, With<Camera3d>>,
    mut billboard_query: Query<(&GlobalTransform, &mut Transform, &ChildOf), With<RigBillboard>>,
    parent_query: Query<&GlobalTransform, Without<RigBillboard>>,
) {
    let camera_gt = camera_query.single().expect("Camera3d must exist in AppState::Ready");
    let camera_pos = camera_gt.translation();
    for (global_transform, mut transform, child_of) in &mut billboard_query {
        let billboard_pos = global_transform.translation();
        let direction = (camera_pos - billboard_pos).with_y(0.0);
        if direction.length_squared() < 0.001 { continue; }
        let world_rotation = Quat::from_rotation_arc(Vec3::Z, direction.normalize());
        let parent_rotation = parent_query
            .get(child_of.parent())
            .map(|gt| gt.to_scale_rotation_translation().1)
            .unwrap_or(Quat::IDENTITY);
        transform.rotation = parent_rotation.inverse() * world_rotation;
    }
}
```

### 2.5 Facing direction

```rust
fn apply_facing_to_rig(
    characters: Query<(Entity, &Facing), Changed<Facing>>,
    children_query: Query<&Children>,
    mut billboard_query: Query<&mut Transform, With<RigBillboard>>,
) {
    for (entity, facing) in &characters {
        let scale_x = match facing {
            Facing::Left => -1.0,
            Facing::Right => 1.0,
        };
        let children = children_query.get(entity)
            .expect("Character with Facing should have children");
        for child in children.iter() {
            if let Ok(mut transform) = billboard_query.get_mut(child) {
                transform.scale.x = scale_x;
            }
        }
    }
}
```

A separate system reads `LinearVelocity.x` and updates `Facing`:

```rust
fn update_facing_from_velocity(
    mut characters: Query<(&mut Facing, &avian3d::prelude::LinearVelocity), With<CharacterMarker>>,
) {
    for (mut facing, velocity) in &mut characters {
        if velocity.x > 0.1 {
            facing.set_if_neq(Facing::Right);
        } else if velocity.x < -0.1 {
            facing.set_if_neq(Facing::Left);
        }
    }
}
```

### 2.6 Wire into add_character_meshes

**File**: `crates/render/src/lib.rs`

Remove the capsule `Mesh3d`/`MeshMaterial3d` insertion from `add_character_meshes`. The sprite rig
replaces it. The `resolve_character_rig` system in `spawn.rs` handles rig insertion based on the
replicated `CharacterType`. No changes needed in `add_character_meshes` beyond removing the capsule
mesh — the rig spawning is driven by `CharacterType` replication, not by `add_character_meshes`.

### Success Criteria — Phase 2

#### Automated Verification
- [x] `cargo check-all` passes

#### Manual Verification
- [ ] `cargo client` — colored rectangle bone hierarchies appear on character entities, facing the camera (requires server + client connected)
- [ ] Characters flip horizontally when moving left
- [ ] Bone hierarchy is correct (head above torso, arms at sides, legs at bottom)

---

## Phase 3: Animation Clip Building and Blend-Tree Locomotion

### Overview

Build `AnimationClip` assets from loaded `SpriteAnimAsset` data. Build an `AnimationGraph` per
animset type with a **1D blend tree** for locomotion — all locomotion clips play simultaneously with
velocity-driven weights (Overgrowth-inspired). Attach `AnimationPlayer` + `AnimationTransitions`
to characters. A locomotion system updates blend weights each frame based on character speed.

### 3.1 Resources

**File**: `crates/sprite_rig/src/animation.rs`

```rust
/// Maps SpriteAnimAsset ID → derived Handle<AnimationClip>.
#[derive(Resource, Default)]
pub struct BuiltAnimations(pub HashMap<AssetId<SpriteAnimAsset>, Handle<AnimationClip>>);

/// Maps anim asset path string → strong Handle<SpriteAnimAsset>.
/// Keeps strong handles alive so Bevy doesn't garbage-collect the assets before they load.
#[derive(Resource, Default)]
pub struct LoadedAnimHandles(pub HashMap<String, Handle<SpriteAnimAsset>>);

/// Pre-built animation graphs, one per animset asset. Shared across all character
/// instances of the same type.
#[derive(Resource, Default)]
pub struct BuiltAnimGraphs(pub HashMap<AssetId<SpriteAnimSetAsset>, BuiltAnimGraph>);

/// A fully built animation graph for one animset.
pub struct BuiltAnimGraph {
    pub graph_handle: Handle<AnimationGraph>,
    /// Maps clip path → AnimationNodeIndex (for both locomotion and ability clips).
    pub node_map: HashMap<String, AnimationNodeIndex>,
    /// Locomotion entries in order of speed_threshold (for blend weight calculation).
    pub locomotion_entries: Vec<LocomotionNodeEntry>,
}

/// A locomotion clip node with its speed threshold for blend weight interpolation.
pub struct LocomotionNodeEntry {
    pub node_index: AnimationNodeIndex,
    pub speed_threshold: f32,
}
```

### 3.2 AnimationClip building

```rust
fn build_animation_clips(
    mut events: EventReader<AssetEvent<SpriteAnimAsset>>,
    source_assets: Res<Assets<SpriteAnimAsset>>,
    mut clips: ResMut<Assets<AnimationClip>>,
    mut built: ResMut<BuiltAnimations>,
) {
    for event in events.read() {
        match event {
            AssetEvent::LoadedWithDependencies { id } => {
                let source = source_assets.get(*id)
                    .expect("Asset should exist when LoadedWithDependencies fires");
                let clip = build_clip_from(source);
                let handle = clips.add(clip);
                built.0.insert(*id, handle);
            }
            AssetEvent::Modified { id } => {
                let source = source_assets.get(*id)
                    .expect("Asset should exist when Modified fires");
                let clip = build_clip_from(source);
                let handle = built.0.get(id)
                    .expect("Modified SpriteAnimAsset should have a built clip entry");
                clips.insert(handle.id(), clip);
            }
            _ => {}
        }
    }
}
```

`build_clip_from` iterates `SpriteAnimAsset.bone_timelines`, creates `AnimatableCurve`s:

```rust
fn build_clip_from(anim: &SpriteAnimAsset) -> AnimationClip {
    let mut clip = AnimationClip::default();
    clip.set_duration(anim.duration);

    for (bone_name, timeline) in &anim.bone_timelines {
        let target_id = AnimationTargetId::from_names(std::iter::once(&Name::new(bone_name)));

        if !timeline.rotation.is_empty() {
            let curve = UnevenSampleAutoCurve::new(
                timeline.rotation.iter().map(|k| (k.time, Quat::from_rotation_z(k.value.to_radians())))
            ).expect("Animation timeline should have at least 2 keyframes");
            clip.add_curve_to_target(
                target_id,
                AnimatableCurve::new(animated_field!(Transform::rotation), curve),
            );
        }

        if !timeline.translation.is_empty() {
            let curve = UnevenSampleAutoCurve::new(
                timeline.translation.iter().map(|k| (k.time, Vec3::new(k.value.x, k.value.y, 0.0)))
            ).expect("Animation timeline should have at least 2 keyframes");
            clip.add_curve_to_target(
                target_id,
                AnimatableCurve::new(animated_field!(Transform::translation), curve),
            );
        }
    }

    clip
}
```

**Note**: Verify `AnimationTargetId::from_names` signature in Bevy 0.17. It may be
`from_names(impl Iterator<Item = &Name>)` or similar. Adjust accordingly. Also verify
`UnevenSampleAutoCurve::new` accepts `impl Iterator<Item = (f32, T)>` in 0.17.

### 3.3 AnimationGraph building (per-animset)

Instead of a single global graph, graphs are built per animset and stored in `BuiltAnimGraphs`.
The system runs each frame until all registered animsets have their graphs built (all clips must
be built first).

Graph structure:
```
Root
├── Blend node (locomotion)
│   ├── idle clip  (weight: computed from speed)
│   ├── walk clip  (weight: computed from speed)
│   └── run clip   (weight: computed from speed)
├── punch clip     (weight: 0.0, activated by ability system)
└── ...other ability clips
```

```rust
fn build_anim_graphs(
    mut commands: Commands,
    registry: Res<RigRegistry>,
    animset_assets: Res<Assets<SpriteAnimSetAsset>>,
    built_anims: Res<BuiltAnimations>,
    mut built_graphs: ResMut<BuiltAnimGraphs>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    loaded_anim_handles: Res<LoadedAnimHandles>,
) {
    for (_char_type, entry) in &registry.entries {
        let animset_id = entry.animset_handle.id();
        // Skip if already built
        if built_graphs.0.contains_key(&animset_id) { continue; }

        let Some(animset) = animset_assets.get(&entry.animset_handle) else {
            // Asset not loaded yet — expected during startup before AppState::Ready
            continue;
        };

        let mut graph = AnimationGraph::new();
        let mut node_map = HashMap::new();
        let mut locomotion_entries = Vec::new();

        let blend_node = graph.add_blend(1.0, graph.root);

        let mut all_clips_ready = true;
        for loco_entry in &animset.locomotion.entries {
            let Some(clip_handle) = lookup_clip_handle(&loco_entry.clip, &built_anims, &loaded_anim_handles) else {
                all_clips_ready = false;
                break;
            };
            let node_idx = graph.add_clip(clip_handle.clone(), 0.0, blend_node);
            node_map.insert(loco_entry.clip.clone(), node_idx);
            locomotion_entries.push(LocomotionNodeEntry {
                node_index: node_idx,
                speed_threshold: loco_entry.speed_threshold,
                clip_path: loco_entry.clip.clone(),
            });
        }
        if !all_clips_ready { continue; }

        for (_ability_id, clip_path) in &animset.ability_animations {
            let Some(clip_handle) = lookup_clip_handle(clip_path, &built_anims, &loaded_anim_handles) else {
                all_clips_ready = false;
                break;
            };
            let node_idx = graph.add_clip(clip_handle.clone(), 1.0, graph.root);
            node_map.insert(clip_path.clone(), node_idx);
        }
        if !all_clips_ready { continue; }

        let graph_handle = graphs.add(graph);
        built_graphs.0.insert(animset_id, BuiltAnimGraph {
            graph_handle,
            node_map,
            locomotion_entries,
        });
    }
}
```

`LoadedAnimHandles` maps anim asset path string → strong `Handle<SpriteAnimAsset>`. It must store
strong handles (not `AssetId`) to prevent Bevy from garbage-collecting the assets before they load.

### 3.4 Attach AnimationPlayer to characters + add AnimationTarget to bones

When a character has `BoneEntities` + `AnimSetRef` and the corresponding graph is built:

```rust
fn attach_animation_players(
    mut commands: Commands,
    characters: Query<(Entity, &BoneEntities, &AnimSetRef), (With<CharacterMarker>, Without<AnimationPlayer>)>,
    built_graphs: Res<BuiltAnimGraphs>,
) {
    for (entity, bone_entities, animset_ref) in &characters {
        let animset_id = animset_ref.0.id();
        let Some(built_graph) = built_graphs.0.get(&animset_id) else {
            // Graph not yet built — expected during startup
            continue;
        };

        commands.entity(entity).insert((
            AnimationPlayer::default(),
            AnimationTransitions::default(),
            AnimationGraphHandle(built_graph.graph_handle.clone()),
        ));

        for (bone_name, &bone_entity) in &bone_entities.0 {
            let target_id = AnimationTargetId::from_names(std::iter::once(&Name::new(bone_name)));
            commands.entity(bone_entity).insert(AnimationTarget {
                id: target_id,
                player: entity,
            });
        }
    }
}
```

### 3.5 Start locomotion blend playback once AnimationPlayer is attached

All locomotion clips start playing simultaneously with looping enabled. The idle clip starts at
weight 1.0; others start at 0.0.

```rust
fn start_locomotion_blend(
    mut query: Query<(&mut AnimationPlayer, &AnimSetRef), Added<AnimationPlayer>>,
    built_graphs: Res<BuiltAnimGraphs>,
) {
    for (mut player, animset_ref) in &mut query {
        let Some(built_graph) = built_graphs.0.get(&animset_ref.0.id()) else {
            debug_assert!(false, "AnimationPlayer attached but graph not built");
            continue;
        };
        for (i, entry) in built_graph.locomotion_entries.iter().enumerate() {
            let mut anim = player.play(entry.node_index);
            anim.repeat();
            anim.set_weight(if i == 0 { 1.0 } else { 0.0 });
        }
    }
}
```

### 3.6 Blend-tree locomotion (velocity-driven weights)

Each frame, compute blend weights from character speed using 1D linear interpolation between
neighboring speed thresholds.

```rust
/// Tracks whether the character is in locomotion mode (vs ability animation).
#[derive(Component)]
pub struct LocomotionState {
    pub active: bool,
}

fn update_locomotion_blend_weights(
    mut characters: Query<(
        &mut AnimationPlayer,
        &LocomotionState,
        &AnimSetRef,
        &avian3d::prelude::LinearVelocity,
    ), With<CharacterMarker>>,
    built_graphs: Res<BuiltAnimGraphs>,
) {
    for (mut player, loco_state, animset_ref, velocity) in &mut characters {
        if !loco_state.active { continue; }

        let Some(built_graph) = built_graphs.0.get(&animset_ref.0.id()) else {
            continue; // Graph not built yet — possible during startup
        };

        let speed = velocity.xz().length();
        let weights = compute_blend_weights(speed, &built_graph.locomotion_entries);

        for (entry, weight) in built_graph.locomotion_entries.iter().zip(weights.iter()) {
            let anim = player.animation_mut(entry.node_index)
                .expect("Locomotion clip should be playing when locomotion is active");
            anim.set_weight(*weight);
        }
    }
}

/// Compute 1D blend weights from speed. Between two thresholds, linearly
/// interpolate. Below the lowest threshold, 100% lowest clip. Above the
/// highest, 100% highest clip.
fn compute_blend_weights(speed: f32, entries: &[LocomotionNodeEntry]) -> Vec<f32> {
    let mut weights = vec![0.0; entries.len()];
    if entries.is_empty() { return weights; }
    if entries.len() == 1 { weights[0] = 1.0; return weights; }

    if speed <= entries[0].speed_threshold {
        weights[0] = 1.0;
        return weights;
    }
    if speed >= entries.last().unwrap().speed_threshold {
        *weights.last_mut().unwrap() = 1.0;
        return weights;
    }

    for i in 0..entries.len() - 1 {
        let lo = entries[i].speed_threshold;
        let hi = entries[i + 1].speed_threshold;
        if speed >= lo && speed < hi {
            let t = (speed - lo) / (hi - lo);
            weights[i] = 1.0 - t;
            weights[i + 1] = t;
            return weights;
        }
    }

    weights[0] = 1.0;
    weights
}
```

### Success Criteria — Phase 3

#### Automated Verification
- [x] `cargo check-all` passes

#### Manual Verification
- [x] `cargo client` — character bone rectangles animate with smooth blended locomotion
- [ ] Speed transitions blend smoothly (idle->walk->run are interpolated, not discrete switches)
- [ ] Standing still shows pure idle; full speed shows pure run; intermediate speeds show blends
- [ ] Editing an `.anim.ron` file on disk updates the animation without restarting (hot-reload)

---

## Phase 4: Ability Animation Bridge

### Overview

When an `ActiveAbility` entity appears for a character, play the corresponding animation clip.
When the ability ends, return to locomotion. Cosmetic animation events are dispatched as Bevy events
(no implementation required — just the dispatch hook).

### 4.1 Ability animation trigger system

**File**: `crates/sprite_rig/src/animset.rs`

```rust
fn trigger_ability_animations(
    added_abilities: Query<&ActiveAbility, Added<ActiveAbility>>,
    mut characters: Query<(
        &mut AnimationPlayer,
        &mut AnimationTransitions,
        &mut LocomotionState,
        &AnimSetRef,
    ), With<CharacterMarker>>,
    built_graphs: Res<BuiltAnimGraphs>,
    animset_assets: Res<Assets<SpriteAnimSetAsset>>,
) {
    for ability in &added_abilities {
        let Ok((mut player, mut transitions, mut loco_state, animset_ref)) =
            characters.get_mut(ability.caster)
        else {
            // caster entity not found on client (may be on wrong tick) — not a bug
            continue;
        };

        let animset_id = animset_ref.0.id();
        let Some(built_graph) = built_graphs.0.get(&animset_id) else {
            continue; // Graph not built yet
        };
        let Some(animset) = animset_assets.get(&animset_ref.0) else {
            continue; // Animset not loaded yet
        };

        let ability_id = ability.def_id.0.as_str();
        if let Some(clip_path) = animset.ability_animations.get(ability_id) {
            let node_idx = built_graph.node_map.get(clip_path.as_str())
                .expect("Ability animation clip should be in the graph if it's in the animset");
            for entry in &built_graph.locomotion_entries {
                let anim = player.animation_mut(entry.node_index)
                    .expect("Locomotion clip should be playing");
                anim.set_weight(0.0);
            }
            transitions.play(&mut player, *node_idx, Duration::from_millis(80));
            loco_state.active = false;
        }
    }
}
```

### 4.2 Return to locomotion after ability ends

```rust
fn return_to_locomotion(
    abilities: Query<&ActiveAbility>,
    mut characters: Query<(
        &mut LocomotionState,
        &mut AnimationPlayer,
        &AnimSetRef,
        Entity,
    ), With<CharacterMarker>>,
    built_graphs: Res<BuiltAnimGraphs>,
) {
    for (mut loco_state, mut player, animset_ref, entity) in &mut characters {
        if loco_state.active { continue; }

        let has_active_ability = abilities.iter().any(|a| a.caster == entity);
        if has_active_ability { continue; }

        loco_state.active = true;

        let Some(built_graph) = built_graphs.0.get(&animset_ref.0.id()) else {
            continue;
        };
        for entry in &built_graph.locomotion_entries {
            if !player.is_playing_animation(entry.node_index) {
                player.play(entry.node_index).repeat();
            }
        }
    }
}
```

### 4.3 Animation event dispatch hook

Bevy's animation event API uses the **observer system**, not a method on `AnimationPlayer`.
`AnimationPlayer::animation_events()` does not exist.

Define a custom event type with `AnimationEvent`:

```rust
#[derive(AnimationEvent, Clone)]
pub struct AnimationEventFired {
    pub event_name: String,
}
```

Add events to clips at build time in `build_clip_from`:

```rust
for ev in &anim.events {
    clip.add_event(ev.time, AnimationEventFired { event_name: ev.name.clone() });
}
```

Register an observer to receive events:

```rust
app.add_observer(on_animation_event_fired);

fn on_animation_event_fired(
    trigger: On<AnimationEventFired>,
    query: Query<Entity, With<CharacterMarker>>,
) {
    let entity = trigger.entity();
    let event = trigger.event();
    info!(character = ?entity, event = %event.event_name, "animation event fired");
}
```

No consumer systems needed yet; the observer stub is defined for future use by audio/particle
systems.

### Success Criteria — Phase 4

#### Automated Verification
- [ ] `cargo check-all` passes

#### Manual Verification
- [ ] `cargo client` — activating the punch ability plays the punch animation, returns to locomotion after
- [ ] Ability animation interrupts locomotion cleanly (no visual snap)
- [ ] `warn!` logged if ability_id has no animation mapping (not an error)

---

## System Registration Order

All systems registered in `SpriteRigPlugin::build`:

```rust
app.add_systems(Startup, load_rig_assets);
app.init_resource::<BuiltAnimations>();
app.init_resource::<BuiltAnimGraphs>();
app.add_systems(Update, (
    resolve_character_rig,
    spawn_sprite_rigs,
    billboard_rigs_face_camera,
    update_facing_from_velocity,
    apply_facing_to_rig,
    build_animation_clips,
    build_anim_graphs,
    attach_animation_players,
    start_locomotion_blend,
    update_locomotion_blend_weights,
    trigger_ability_animations,
    return_to_locomotion,
).chain());
```

Use `.chain()` to ensure ordering: spawning before animation attachment, etc. Fine-tune ordering
during implementation if needed.

---

## Testing Strategy

### Unit tests (inline `#[cfg(test)]` in source files)

- `compute_blend_weights`: verify 1D interpolation at thresholds, between thresholds, below minimum,
  above maximum. Edge cases: single entry, two entries, exact threshold values.
- `build_clip_from`: given a `SpriteAnimAsset` with known rotation keyframes, assert the built
  `AnimationClip` has the correct duration.
- Topological bone sort: given a rig with parent/child bones in random order, verify spawn order is
  parents-first.

### Integration test (`crates/sprite_rig/tests/sprite_rig.rs`)

A single headless Bevy `App` test that verifies the full rig setup and animation pipeline. Runs
during `cargo test-native`. Follows the existing project pattern: `MinimalPlugins` + domain plugins,
`app.update()` loops, query-based assertions.

```rust
use bevy::prelude::*;
use sprite_rig::*;

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        TransformPlugin,
        AnimationPlugin,
        SpriteRigPlugin,
    ));
    app
}

#[test]
fn sprite_rig_spawns_bone_hierarchy() {
    let mut app = test_app();
    app.update(); // Let asset loading systems run

    // Wait for assets to load (tick until RigRegistry is available)
    // Then spawn a character entity with CharacterType::Humanoid + CharacterMarker
    // After update(), verify:
    // - BoneEntities component present with expected bone names
    // - Child entities exist with Name components matching bone names
    // - Each bone has a Transform
}

#[test]
fn animation_player_attached_and_locomotion_blends() {
    let mut app = test_app();
    // Load assets, spawn rig, wait for BuiltAnimGraphs to contain the humanoid graph
    // After update():
    // - AnimationPlayer component present on character
    // - AnimationGraphHandle present
    // - All locomotion clips are playing (player.is_playing_animation)
    // - At speed 0.0, idle clip has weight ~1.0, others ~0.0
}
```

**Note**: The exact asset loading flow in tests depends on whether `AssetPlugin::default()` can
load from the workspace `assets/` directory in the test environment. If not, construct
`SpriteRigAsset` and `SpriteAnimAsset` in-memory and insert them directly into `Assets<T>`
resources, bypassing the `AssetServer`. This is the pattern used by `voxel_map_engine` tests.

---

## Implementation Notes

1. **`AnimationTargetId::from_names` API**: Signature is
   `fn from_names<'a>(names: impl Iterator<Item = &'a Name>) -> AnimationTargetId`.
   Note: takes `Iterator`, not `IntoIterator`. `std::iter::once(&Name::new(bone_name))` is correct.

2. **`UnevenSampleAutoCurve::new`**: Signature is
   `fn new(timed_samples: impl IntoIterator<Item = (f32, T)>) -> Result<Self, UnevenCoreError>`.
   Takes `(time, value)` tuple pairs — **not** separate time and value arrays.

3. **`AnimationPlayer::animation_events`**: **Does not exist.** Animation events use the Bevy
   observer system. See Phase 4.3 for the correct implementation.

4. **`AnimationNodeIndex` type**: In Bevy 0.17 this may be a type alias. Verify import path.

5. **Capsule mesh**: The visual `Mesh3d` capsule is removed; the sprite rig replaces it. The
   physics capsule collider remains.

6. **WASM**: `bevy_animation` and `bevy_sprite` are already in `web/Cargo.toml`. No changes needed.
   Verify `cargo run web` builds after each phase.

7. **Adding a new character type**: Add a variant to `CharacterType` in protocol, add an animset
   entry in `load_rig_assets`, create the corresponding RON asset files. No code changes to the
   animation systems themselves.

---

## References

- Research: `doc/research/2026-03-07-sprite-rig-animation-overgrowth.md`
- Current character rendering: `crates/render/src/lib.rs:70-94`
- Health bar child hierarchy pattern: `crates/render/src/health_bar.rs`
- Billboard pattern: `crates/render/src/health_bar.rs:67-89`
- Asset loading pattern: `crates/protocol/src/ability.rs:407-668`
- TrackedAssets: `crates/protocol/src/app_state.rs`
- ActiveAbility: `crates/protocol/src/ability.rs:237-247`
- Character spawn (server): `crates/server/src/gameplay.rs:166-222`
- DummyTarget spawn (server): `crates/server/src/gameplay.rs:61-76`
- Replication registration: `crates/protocol/src/lib.rs:186-230`
