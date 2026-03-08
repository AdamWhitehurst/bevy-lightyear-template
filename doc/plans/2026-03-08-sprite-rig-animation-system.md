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

Research source: `doc/research/2026-03-07-sprite-rig-animation-overgrowth.md`

---

## Current State

- Characters render as `Capsule3d` + `StandardMaterial` in `crates/render/src/lib.rs:86-90`
- `bevy_common_assets` 0.14 and `serde` already in workspace `Cargo.toml`
- `bevy_animation` and `bevy_sprite` already enabled in `crates/web/Cargo.toml`; native build uses
  `default-features = true` so these are already available in the render crate
- No sprite, image, or animation assets exist in `assets/`

## Desired End State

Characters visually appear as 2D sprite rigs billboarded in the 3D world. Bone sprites animate
through idle/walk/run locomotion states driven by velocity. Ability activations trigger corresponding
animations. Sprites can be flipped horizontally based on facing direction.

### Verification
- `cargo client` — characters appear as colored rectangle bone hierarchies (placeholders for real
  sprites), animating through idle/walk/run as characters move
- Ability activation causes the matching ability animation to play, then returns to locomotion
- Characters face the camera (billboard)
- Characters flip when moving left vs right

---

## What We Are NOT Doing

- Actual artist-authored sprite PNG files (placeholder programmatic sprites only)
- Full blend-tree locomotion (state machine with crossfades is sufficient for Phase 3)
- Spine runtime or any external animation library
- Skin swapping / appearance evolution (separate plan)
- IK or procedural hit reactions (separate plan)
- Audio/particle animation events (hooks defined, no implementation)
- Server-side animation knowledge of any kind

---

## Architecture

Animation code lives entirely in the render crate under `crates/render/src/sprite_rig/`. A
`SpriteRigPlugin` is added to `RenderPlugin`.

### Module layout

```
crates/render/src/
  sprite_rig/
    mod.rs         — SpriteRigPlugin, re-exports
    asset.rs       — SpriteRigAsset, SpriteAnimAsset, SpriteAnimSetAsset + type definitions
    spawn.rs       — rig spawning, billboard, facing systems
    animation.rs   — AnimationClip building, AnimationGraph setup, locomotion updates
    animset.rs     — SpriteAnimSetAsset loading, ability animation bridge
```

### Key design decisions (from research)

1. **Shared AnimationClips (Option B)**: clips built once at load time using deterministic
   `AnimationTargetId::from_names(iter::once(&Name::new(bone_name)))`. All instances of the same
   rig share clip and graph assets. Per-instance `AnimationPlayer` + `AnimationTransitions` hold
   independent playback state.

2. **Billboard**: system sets rig root rotation to face camera each frame (same pattern as
   `health_bar::billboard_face_camera`).

3. **Placeholder sprites**: `Sprite { custom_size: Some(Vec2::new(w, h)), color, ..default() }` —
   no image files required. The 1×1 white fallback image (`Handle::<Image>::default()`) is used.

4. **Hot-reload**: `AssetEvent::Modified { id }` on `SpriteAnimAsset` triggers in-place clip
   replacement via `clips.insert(handle.id(), new_clip)`.

5. **Capsule kept**: the 3D capsule mesh remains for now. Sprite rig is added alongside it as a
   child hierarchy. Capsule removal is a separate cleanup task.

---

## Phase 1: Asset Type Definitions and Loading Infrastructure

### Overview

Define Rust types for `.rig.ron`, `.anim.ron`, and `.animset.ron` assets. Register
`RonAssetPlugin` for each. Load the brawler rig and animation assets at startup. No visuals yet.

### 1.1 Add dependencies to render/Cargo.toml

**File**: `crates/render/Cargo.toml`

```toml
bevy_common_assets = { workspace = true }
serde = { workspace = true, features = ["derive"] }
```

### 1.2 Asset type definitions

**File**: `crates/render/src/sprite_rig/asset.rs`

```rust
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Rig definition ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, Asset, TypePath)]
pub struct SpriteRigAsset {
    pub bones: Vec<BoneDef>,
    pub slots: Vec<SlotDef>,
    pub skins: HashMap<String, HashMap<String, AttachmentDef>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoneDef {
    pub name: String,
    pub parent: Option<String>,
    pub default_transform: BoneTransform2d,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BoneTransform2d {
    pub translation: Vec2,
    pub rotation: f32, // degrees
    pub scale: Vec2,
}

impl Default for BoneTransform2d {
    fn default() -> Self {
        Self { translation: Vec2::ZERO, rotation: 0.0, scale: Vec2::ONE }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SlotDef {
    pub name: String,
    pub bone: String,
    pub z_order: f32,
    pub default_attachment: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttachmentDef {
    pub image: String,   // asset path (unused until real sprites are added)
    pub anchor: SpriteAnchorDef,
    pub size: Vec2,      // display size in world units
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub enum SpriteAnchorDef {
    #[default]
    Center,
    TopCenter,
    BottomCenter,
}

// ── Animation definition ─────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, Asset, TypePath)]
pub struct SpriteAnimAsset {
    pub name: String,
    pub duration: f32,
    pub looping: bool,
    pub bone_timelines: HashMap<String, BoneTimeline>,
    pub events: Vec<AnimEventKeyframe>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct BoneTimeline {
    pub rotation: Vec<RotationKeyframe>,
    pub translation: Vec<TranslationKeyframe>,
    pub scale: Vec<ScaleKeyframe>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RotationKeyframe {
    pub time: f32,
    pub value: f32, // degrees
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

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub enum CurveType {
    #[default]
    Linear,
    Step,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnimEventKeyframe {
    pub time: f32,
    pub name: String,
}

// ── Animation set ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, Asset, TypePath)]
pub struct SpriteAnimSetAsset {
    pub rig: String,
    pub locomotion: LocomotionConfig,
    pub ability_animations: HashMap<String, String>, // ability_id → anim asset path
    pub hit_react: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocomotionConfig {
    pub entries: Vec<LocomotionEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocomotionEntry {
    pub clip: String, // anim asset path
    pub speed_threshold: f32,
}
```

### 1.3 Example RON asset files

**File**: `assets/rigs/brawler.rig.ron`

```ron
#![enable(implicit_some)]
(
    bones: [
        (name: "root",  parent: None,          default_transform: (translation: (0.0, 0.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "torso", parent: Some("root"),  default_transform: (translation: (0.0, 0.6), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "head",  parent: Some("torso"), default_transform: (translation: (0.0, 0.8), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "arm_l", parent: Some("torso"), default_transform: (translation: (-0.35, 0.3), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "arm_r", parent: Some("torso"), default_transform: (translation: (0.35, 0.3), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "leg_l", parent: Some("root"),  default_transform: (translation: (-0.2, 0.0), rotation: 0.0, scale: (1.0, 1.0))),
        (name: "leg_r", parent: Some("root"),  default_transform: (translation: (0.2, 0.0),  rotation: 0.0, scale: (1.0, 1.0))),
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
            "torso_default": (image: "sprites/brawler/torso.png", anchor: Center,       size: (0.5, 0.7)),
            "head_default":  (image: "sprites/brawler/head.png",  anchor: BottomCenter, size: (0.45, 0.45)),
            "arm_default":   (image: "sprites/brawler/arm.png",   anchor: TopCenter,    size: (0.22, 0.5)),
            "leg_default":   (image: "sprites/brawler/leg.png",   anchor: TopCenter,    size: (0.22, 0.6)),
        },
    },
)
```

**File**: `assets/anims/brawler/idle.anim.ron`

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

**File**: `assets/anims/brawler/walk.anim.ron`

```ron
#![enable(implicit_some)]
(
    name: "walk",
    duration: 0.6,
    looping: true,
    bone_timelines: {
        "torso": (
            rotation: [(time: 0.0, value: 0.0, curve: Linear), (time: 0.3, value: 3.0, curve: Linear), (time: 0.6, value: 0.0, curve: Linear)],
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
    events: [
        (time: 0.15, name: "footstep_left"),
        (time: 0.45, name: "footstep_right"),
    ],
)
```

**File**: `assets/anims/brawler/run.anim.ron`

```ron
#![enable(implicit_some)]
(
    name: "run",
    duration: 0.4,
    looping: true,
    bone_timelines: {
        "torso": (
            rotation: [(time: 0.0, value: -5.0, curve: Linear), (time: 0.2, value: -8.0, curve: Linear), (time: 0.4, value: -5.0, curve: Linear)],
            translation: [],
            scale: [],
        ),
        "arm_l": (
            rotation: [(time: 0.0, value: 30.0, curve: Linear), (time: 0.2, value: -30.0, curve: Linear), (time: 0.4, value: 30.0, curve: Linear)],
            translation: [],
            scale: [],
        ),
        "arm_r": (
            rotation: [(time: 0.0, value: -30.0, curve: Linear), (time: 0.2, value: 30.0, curve: Linear), (time: 0.4, value: -30.0, curve: Linear)],
            translation: [],
            scale: [],
        ),
        "leg_l": (
            rotation: [(time: 0.0, value: -35.0, curve: Linear), (time: 0.2, value: 35.0, curve: Linear), (time: 0.4, value: -35.0, curve: Linear)],
            translation: [],
            scale: [],
        ),
        "leg_r": (
            rotation: [(time: 0.0, value: 35.0, curve: Linear), (time: 0.2, value: -35.0, curve: Linear), (time: 0.4, value: 35.0, curve: Linear)],
            translation: [],
            scale: [],
        ),
    },
    events: [
        (time: 0.1, name: "footstep_left"),
        (time: 0.3, name: "footstep_right"),
    ],
)
```

**File**: `assets/anims/brawler/punch.anim.ron`

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

**File**: `assets/anims/brawler/brawler.animset.ron`

```ron
#![enable(implicit_some)]
(
    rig: "rigs/brawler.rig.ron",
    locomotion: (
        entries: [
            (clip: "anims/brawler/idle.anim.ron", speed_threshold: 0.0),
            (clip: "anims/brawler/walk.anim.ron", speed_threshold: 2.0),
            (clip: "anims/brawler/run.anim.ron",  speed_threshold: 6.0),
        ],
    ),
    ability_animations: {
        "punch": "anims/brawler/punch.anim.ron",
    },
    hit_react: None,
)
```

### 1.4 Plugin + loading infrastructure

**File**: `crates/render/src/sprite_rig/mod.rs`

Registers `RonAssetPlugin` for each asset type, loads handles at startup, registers with
`TrackedAssets`.

```rust
pub mod asset;

use asset::*;
use bevy::prelude::*;
use bevy_common_assets::ron::RonAssetPlugin;
use protocol::app_state::TrackedAssets;

pub struct SpriteRigPlugin;

impl Plugin for SpriteRigPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            RonAssetPlugin::<SpriteRigAsset>::new(&["rig.ron"]),
            RonAssetPlugin::<SpriteAnimAsset>::new(&["anim.ron"]),
            RonAssetPlugin::<SpriteAnimSetAsset>::new(&["animset.ron"]),
        ));
        app.add_systems(Startup, load_default_rig_assets);
    }
}

#[derive(Resource)]
pub struct DefaultAnimSetHandle(pub Handle<SpriteAnimSetAsset>);

fn load_default_rig_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut tracked: ResMut<TrackedAssets>,
) {
    let handle = asset_server.load::<SpriteAnimSetAsset>("anims/brawler/brawler.animset.ron");
    tracked.add(handle.clone());
    commands.insert_resource(DefaultAnimSetHandle(handle));
}
```

Add `SpriteRigPlugin` to `RenderPlugin::build`.

### Success Criteria — Phase 1

#### Automated Verification
- [ ] `cargo check-all` passes with no errors
- [ ] `cargo client` starts, logs show rig/anim assets loaded (no asset errors in console)

---

## Phase 2: Rig Spawning, Billboard, and Facing

### Overview

When a character receives a `SpriteRig` component, spawn a bone entity hierarchy as children.
Billboarding and facing direction are also handled here.

### 2.1 New components

**File**: `crates/render/src/sprite_rig/spawn.rs`

```rust
/// Reference to the rig asset for this character. Triggers rig spawning when added.
#[derive(Component)]
pub struct SpriteRig(pub Handle<SpriteRigAsset>);

/// Maps bone names to their spawned child entities.
#[derive(Component, Default)]
pub struct BoneEntities(pub HashMap<String, Entity>);

/// Which horizontal direction this character is facing.
#[derive(Component, PartialEq, Clone, Copy)]
pub enum Facing { Left, Right }
```

### 2.2 Rig spawning system

**File**: `crates/render/src/sprite_rig/spawn.rs`

Trigger: `Added<SpriteRig>`

Steps:
1. Resolve the `SpriteRigAsset`. If not yet loaded, log a warning and skip (the asset should be
   loaded before characters spawn because TrackedAssets gates `AppState::Ready`).
2. Build a parent map: `bone_name → parent_bone_name`. Find root bones (no parent).
3. Use topological sort (parents before children). Because bone count is small (≤ 15), a simple
   repeated-pass sort is fine.
4. Resolve z_order per bone from `SpriteRigAsset.slots` (bone → z_order lookup, default 0.0).
5. Spawn bone entities as children. Each bone entity gets:
   - `Name::new(bone_name.clone())`
   - `Transform` built from `BoneDef.default_transform`: `Transform { translation: Vec3::new(x, y, z_order), rotation: Quat::from_rotation_z(degrees.to_radians()), scale: Vec3::new(sx, sy, 1.0) }`
   - `Sprite { custom_size: Some(bone_display_size(bone_name)), color: placeholder_color(bone_name), ..default() }` — colored rectangles as placeholders
6. Insert `BoneEntities` on the character root.

```rust
fn spawn_sprite_rigs(
    mut commands: Commands,
    query: Query<(Entity, &SpriteRig), Added<SpriteRig>>,
    rig_assets: Res<Assets<SpriteRigAsset>>,
    animset_handle: Res<DefaultAnimSetHandle>,
    animset_assets: Res<Assets<SpriteAnimSetAsset>>,
) {
    for (entity, rig_component) in &query {
        let Some(rig) = rig_assets.get(&rig_component.0) else {
            warn!(?entity, "SpriteRig asset not loaded at spawn time");
            continue;
        };
        let bone_map = spawn_bone_hierarchy(&mut commands, entity, rig);
        commands.entity(entity).insert(BoneEntities(bone_map));
    }
}
```

`spawn_bone_hierarchy` returns `HashMap<String, Entity>`. Bone entities are spawned as children of
`entity` using `commands.entity(entity).with_children(...)`.

### 2.3 Billboard system

Each frame, rotate the character root's Transform to face the camera (project onto the XZ plane,
rotate around Y). Follow the existing `billboard_face_camera` pattern in `health_bar.rs`.

```rust
fn billboard_rigs_face_camera(
    mut rigs: Query<&mut Transform, With<SpriteRig>>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
) {
    let Ok(cam_transform) = camera.single() else { return };
    let cam_pos = cam_transform.translation();
    for mut transform in &mut rigs {
        let to_cam = (cam_pos - transform.translation).with_y(0.0);
        if to_cam.length_squared() > 0.0001 {
            transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, to_cam.normalize());
        }
    }
}
```

### 2.4 Facing direction

```rust
fn apply_facing_to_rig_sprites(
    characters: Query<(&Facing, &BoneEntities), Changed<Facing>>,
    mut sprites: Query<&mut Sprite>,
) {
    for (facing, bones) in &characters {
        let flip = *facing == Facing::Left;
        for &bone_entity in bones.0.values() {
            if let Ok(mut sprite) = sprites.get_mut(bone_entity) {
                sprite.flip_x = flip;
            }
        }
    }
}
```

A separate system reads `LinearVelocity.x` and updates `Facing` accordingly:

```rust
fn update_facing_from_velocity(
    mut characters: Query<(&mut Facing, &avian3d::prelude::LinearVelocity), With<CharacterMarker>>,
) {
    for (mut facing, velocity) in &mut characters {
        if velocity.x > 0.1 {
            *facing = Facing::Right;
        } else if velocity.x < -0.1 {
            *facing = Facing::Left;
        }
    }
}
```

### 2.5 Wire into add_character_meshes

**File**: `crates/render/src/lib.rs`

In `add_character_meshes`, after the existing mesh insertion, also insert:

```rust
commands.entity(entity).insert((
    SpriteRig(default_animset.rig_handle.clone()), // resolved from DefaultAnimSetHandle
    Facing::Right,
));
```

For Phase 2, `SpriteRig` carries the rig handle directly. Since `DefaultAnimSetHandle` holds the
animset, we need the rig handle separately. Add a `DefaultRigHandle(Handle<SpriteRigAsset>)` resource
loaded in `load_default_rig_assets` by resolving the rig path from the animset asset, or simply
load `rigs/brawler.rig.ron` directly at startup as a second handle.

Simpler: load `rigs/brawler.rig.ron` directly in `load_default_rig_assets`:

```rust
#[derive(Resource)]
pub struct DefaultRigHandle(pub Handle<SpriteRigAsset>);

// in load_default_rig_assets:
let rig_handle = asset_server.load::<SpriteRigAsset>("rigs/brawler.rig.ron");
tracked.add(rig_handle.clone());
commands.insert_resource(DefaultRigHandle(rig_handle));
```

Then `add_character_meshes` takes `Res<DefaultRigHandle>` and inserts `SpriteRig(default_rig.0.clone())`.

### Success Criteria — Phase 2

#### Automated Verification
- [ ] `cargo check-all` passes

#### Manual Verification
- [ ] `cargo client` — colored rectangle bone hierarchies appear on character entities, facing the camera
- [ ] Characters flip horizontally when moving left
- [ ] Bone hierarchy is correct (head above torso, arms at sides, legs at bottom)

---

## Phase 3: Animation Clip Building and Playback

### Overview

Build `AnimationClip` assets from loaded `SpriteAnimAsset` data. Build an `AnimationGraph` per
rig type with locomotion clip nodes. Attach `AnimationPlayer` + `AnimationTransitions` to
characters. A locomotion system transitions between idle/walk/run based on speed.

### 3.1 Resources

**File**: `crates/render/src/sprite_rig/animation.rs`

```rust
/// Maps SpriteAnimAsset ID → derived Handle<AnimationClip>.
#[derive(Resource, Default)]
pub struct BuiltAnimations(pub HashMap<AssetId<SpriteAnimAsset>, Handle<AnimationClip>>);

/// Pre-built locomotion graph shared across all brawler instances.
/// Maps animation name (from animset clip path) → NodeIndex in the graph.
#[derive(Resource)]
pub struct BrawlerAnimGraph {
    pub graph_handle: Handle<AnimationGraph>,
    pub node_map: HashMap<String, AnimationNodeIndex>,
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
                if let Some(source) = source_assets.get(*id) {
                    let clip = build_clip_from(source);
                    let handle = clips.add(clip);
                    built.0.insert(*id, handle);
                }
            }
            AssetEvent::Modified { id } => {
                if let Some(source) = source_assets.get(*id) {
                    let clip = build_clip_from(source);
                    if let Some(handle) = built.0.get(id) {
                        clips.insert(handle.id(), clip);
                    } else {
                        warn!(?id, "Modified SpriteAnimAsset has no built clip entry");
                    }
                }
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
            let times: Vec<f32> = timeline.rotation.iter().map(|k| k.time).collect();
            let values: Vec<Quat> = timeline.rotation.iter()
                .map(|k| Quat::from_rotation_z(k.value.to_radians()))
                .collect();
            if let Ok(curve) = UnevenSampleAutoCurve::new(times.into_iter().zip(values)) {
                clip.add_curve_to_target(
                    target_id,
                    AnimatableCurve::new(animated_field!(Transform::rotation), curve),
                );
            }
        }

        if !timeline.translation.is_empty() {
            let times: Vec<f32> = timeline.translation.iter().map(|k| k.time).collect();
            let values: Vec<Vec3> = timeline.translation.iter()
                .map(|k| Vec3::new(k.value.x, k.value.y, 0.0))
                .collect();
            if let Ok(curve) = UnevenSampleAutoCurve::new(times.into_iter().zip(values)) {
                clip.add_curve_to_target(
                    target_id,
                    AnimatableCurve::new(animated_field!(Transform::translation), curve),
                );
            }
        }
    }

    clip
}
```

**Note**: Verify `AnimationTargetId::from_names` signature in Bevy 0.17. It may be
`from_names(impl Iterator<Item = &Name>)` or similar. Adjust accordingly. Also verify
`UnevenSampleAutoCurve::new` accepts `impl Iterator<Item = (f32, T)>` in 0.17.

### 3.3 AnimationGraph building

After all locomotion clips for a rig are loaded, build the `AnimationGraph`:

```rust
fn build_brawler_anim_graph(
    // Triggered when all required clip handles exist in BuiltAnimations
    mut commands: Commands,
    animset_handle: Res<DefaultAnimSetHandle>,
    animset_assets: Res<Assets<SpriteAnimSetAsset>>,
    anim_handle_map: Res<LoadedAnimHandles>, // AssetId<SpriteAnimAsset> by path
    built: Res<BuiltAnimations>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    // Only run once, after all locomotion anim assets are built
    if commands.get_resource::<BrawlerAnimGraph>().is_some() { return; }

    let Some(animset) = animset_assets.get(&animset_handle.0) else { return };

    let mut graph = AnimationGraph::new();
    let mut node_map = HashMap::new();

    for entry in &animset.locomotion.entries {
        // Look up the clip handle by anim path
        if let Some(clip_handle) = lookup_clip_handle(&entry.clip, &built, &anim_handle_map) {
            let node_idx = graph.add_clip(clip_handle.clone(), 1.0, AnimationMask::default());
            node_map.insert(entry.clip.clone(), node_idx);
        } else {
            return; // Not all clips built yet, retry next frame
        }
    }

    // Also add ability animation clips
    for (_ability_id, clip_path) in &animset.ability_animations {
        if let Some(clip_handle) = lookup_clip_handle(clip_path, &built, &anim_handle_map) {
            let node_idx = graph.add_clip(clip_handle.clone(), 1.0, AnimationMask::default());
            node_map.insert(clip_path.clone(), node_idx);
        } else {
            return;
        }
    }

    let graph_handle = graphs.add(graph);
    commands.insert_resource(BrawlerAnimGraph { graph_handle, node_map });
}
```

`LoadedAnimHandles` is a resource mapping anim asset path string → `Handle<SpriteAnimAsset>`, built
by the startup loader so we can look up handles by path.

### 3.4 Attach AnimationPlayer to characters + add AnimationTarget to bones

When `BrawlerAnimGraph` is available and a character has `BoneEntities`:

```rust
fn attach_animation_players(
    mut commands: Commands,
    characters: Query<(Entity, &BoneEntities), (With<CharacterMarker>, Without<AnimationPlayer>)>,
    anim_graph: Option<Res<BrawlerAnimGraph>>,
    rig_assets: Res<Assets<SpriteRigAsset>>,
    rig_handles: Query<&SpriteRig>,
) {
    let Some(anim_graph) = anim_graph else { return };

    for (entity, bone_entities) in &characters {
        // Add AnimationPlayer + AnimationTransitions + AnimationGraphHandle to root
        commands.entity(entity).insert((
            AnimationPlayer::default(),
            AnimationTransitions::default(),
            AnimationGraphHandle(anim_graph.graph_handle.clone()),
        ));

        // Add AnimationTarget to each bone entity
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

### 3.5 Start idle animation once AnimationPlayer is attached

```rust
fn start_idle_animation(
    mut query: Query<(&mut AnimationPlayer, &mut AnimationTransitions), Added<AnimationPlayer>>,
    anim_graph: Option<Res<BrawlerAnimGraph>>,
    animset_handle: Res<DefaultAnimSetHandle>,
    animset_assets: Res<Assets<SpriteAnimSetAsset>>,
) {
    let Some(anim_graph) = anim_graph else { return };
    let Some(animset) = animset_assets.get(&animset_handle.0) else { return };

    let idle_path = &animset.locomotion.entries[0].clip;
    let Some(&idle_node) = anim_graph.node_map.get(idle_path) else { return };

    for (mut player, mut transitions) in &mut query {
        transitions.play(&mut player, idle_node, Duration::ZERO);
    }
}
```

### 3.6 Locomotion state machine

```rust
/// Tracks which locomotion clip is currently playing.
#[derive(Component)]
pub struct LocomotionState {
    pub current_clip_path: String,
}

fn update_locomotion_animation(
    mut characters: Query<(
        &mut AnimationPlayer,
        &mut AnimationTransitions,
        &mut LocomotionState,
        &avian3d::prelude::LinearVelocity,
    ), With<CharacterMarker>>,
    anim_graph: Option<Res<BrawlerAnimGraph>>,
    animset_handle: Res<DefaultAnimSetHandle>,
    animset_assets: Res<Assets<SpriteAnimSetAsset>>,
) {
    let Some(anim_graph) = anim_graph else { return };
    let Some(animset) = animset_assets.get(&animset_handle.0) else { return };

    for (mut player, mut transitions, mut loco_state, velocity) in &mut characters {
        let speed = velocity.xz().length();
        let target_clip = select_locomotion_clip(speed, &animset.locomotion);

        if target_clip != loco_state.current_clip_path {
            if let Some(&node_idx) = anim_graph.node_map.get(target_clip) {
                transitions.play(&mut player, node_idx, Duration::from_millis(150));
                loco_state.current_clip_path = target_clip.to_string();
            }
        }
    }
}

fn select_locomotion_clip<'a>(speed: f32, config: &'a LocomotionConfig) -> &'a str {
    let mut selected = &config.entries[0].clip;
    for entry in &config.entries {
        if speed >= entry.speed_threshold {
            selected = &entry.clip;
        }
    }
    selected.as_str()
}
```

### Success Criteria — Phase 3

#### Automated Verification
- [ ] `cargo check-all` passes

#### Manual Verification
- [ ] `cargo client` — character bone rectangles animate (rotate/oscillate) through idle, walk, run
- [ ] Transitions are smooth (no snapping)
- [ ] Editing an `.anim.ron` file on disk updates the animation without restarting (hot-reload)

---

## Phase 4: Ability Animation Bridge

### Overview

When an `ActiveAbility` entity appears for a character, play the corresponding animation clip.
When the ability ends, return to locomotion. Cosmetic animation events are dispatched as Bevy events
(no implementation required — just the dispatch hook).

### 4.1 Ability animation trigger system

**File**: `crates/render/src/sprite_rig/animset.rs`

```rust
fn trigger_ability_animations(
    added_abilities: Query<&ActiveAbility, Added<ActiveAbility>>,
    mut characters: Query<(
        &mut AnimationPlayer,
        &mut AnimationTransitions,
        &mut LocomotionState,
    ), With<CharacterMarker>>,
    anim_graph: Option<Res<BrawlerAnimGraph>>,
    animset_handle: Res<DefaultAnimSetHandle>,
    animset_assets: Res<Assets<SpriteAnimSetAsset>>,
) {
    let Some(anim_graph) = anim_graph else { return };
    let Some(animset) = animset_assets.get(&animset_handle.0) else { return };

    for ability in &added_abilities {
        let Ok((mut player, mut transitions, mut loco_state)) =
            characters.get_mut(ability.caster)
        else {
            // caster entity not found on client (may be on wrong tick) — not a bug
            continue;
        };

        let ability_id = ability.def_id.0.as_str();
        if let Some(clip_path) = animset.ability_animations.get(ability_id) {
            if let Some(&node_idx) = anim_graph.node_map.get(clip_path.as_str()) {
                transitions.play(&mut player, node_idx, Duration::from_millis(80));
                loco_state.current_clip_path = String::new(); // clear so locomotion re-asserts after
            } else {
                warn!(ability_id, "Ability animation node not found in anim graph");
            }
        }
    }
}
```

### 4.2 Return to locomotion after ability ends

`RemovedComponents<ActiveAbility>` does not carry the ability data (entity is despawned). We need
to return to locomotion on any tick where the character has no `ActiveAbility` as the caster.

Simplest approach: `update_locomotion_animation` already runs every frame and transitions to the
correct locomotion state based on speed. When `LocomotionState.current_clip_path` is empty (cleared
by the ability trigger), the locomotion system will detect a mismatch and transition back.

This works because the locomotion system checks `target_clip != loco_state.current_clip_path`.

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

Register an observer to receive events. Events are triggered on the `AnimationPlayer`'s entity:

```rust
app.add_observer(on_animation_event_fired);

fn on_animation_event_fired(
    trigger: On<AnimationEventFired>,
    query: Query<Entity, With<CharacterMarker>>,
) {
    let entity = trigger.entity(); // the AnimationPlayer entity = character root
    let event = trigger.event();
    // dispatch to audio/particle systems as needed
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
app.add_systems(Startup, load_default_rig_assets);
app.add_systems(Update, (
    spawn_sprite_rigs,
    billboard_rigs_face_camera,
    update_facing_from_velocity,
    apply_facing_to_rig_sprites,
    build_animation_clips,
    build_brawler_anim_graph,
    attach_animation_players,
    start_idle_animation,
    update_locomotion_animation,
    trigger_ability_animations,
).chain());
```

Use `.chain()` to ensure ordering: spawning before animation attachment, etc. Fine-tune ordering
during implementation if needed.

---

## Testing Strategy

### Unit tests

- `build_clip_from`: given a `SpriteAnimAsset` with known rotation keyframes, assert the built
  `AnimationClip` has the correct duration and curve count.
- `select_locomotion_clip`: verify speed thresholds select correct clip paths.
- Topological bone sort: given a rig with parent/child bones in random order, verify spawn order is
  parents-first.

### Integration tests

- `cargo server` + `cargo client`: spawn a character, observe locomotion animation cycling during
  movement via log events.
- Activate an ability: verify ability animation fires.

---

## Implementation Notes

1. **`AnimationTargetId::from_names` API**: Signature is
   `fn from_names<'a>(names: impl Iterator<Item = &'a Name>) -> AnimationTargetId`.
   Note: takes `Iterator`, not `IntoIterator`. `std::iter::once(&Name::new(bone_name))` is correct.

2. **`UnevenSampleAutoCurve::new`**: Signature is
   `fn new(timed_samples: impl IntoIterator<Item = (f32, T)>) -> Result<Self, UnevenCoreError>`.
   Takes `(time, value)` tuple pairs — **not** separate time and value arrays. The `times.into_iter().zip(values)` pattern in `build_clip_from` is correct.

3. **`AnimationPlayer::animation_events`**: **Does not exist.** Animation events use the Bevy
   observer system. See Phase 4.3 for the correct implementation.

4. **`AnimationNodeIndex` type**: In Bevy 0.17 this may be a type alias. Verify import path.

5. **Capsule mesh**: The `Mesh3d` capsule is kept in Phase 2-4. Remove it in a follow-up cleanup
   once sprite rigs are visually confirmed.

6. **WASM**: `bevy_animation` and `bevy_sprite` are already in `web/Cargo.toml`. No changes needed.
   Verify `cargo run web` builds after each phase.

---

## References

- Research: `doc/research/2026-03-07-sprite-rig-animation-overgrowth.md`
- Current character rendering: `crates/render/src/lib.rs:70-94`
- Health bar child hierarchy pattern: `crates/render/src/health_bar.rs`
- Billboard pattern: `crates/render/src/health_bar.rs:67-89`
- Asset loading pattern: `crates/protocol/src/ability.rs:407-668`
- TrackedAssets: `crates/protocol/src/app_state.rs`
- ActiveAbility: `crates/protocol/src/ability.rs:237-247`
