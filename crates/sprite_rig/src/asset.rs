use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Defines a character's bone hierarchy, sprite slots, and skin variants.
#[derive(Clone, Debug, Serialize, Deserialize, Asset, TypePath)]
pub struct SpriteRigAsset {
    /// Conversion factor from sprite image pixels to world units. A bone quad's
    /// world size is `(image_pixels / pixels_per_unit) * attachment.scale`.
    pub pixels_per_unit: f32,
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
        Self {
            translation: Vec2::ZERO,
            rotation: 0.0,
            scale: Vec2::ONE,
        }
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
///
/// The rendered quad size is `(image_pixels / rig.pixels_per_unit) * scale`.
/// A `scale` of `(1.0, 1.0)` renders the sprite at its natural pixel size.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttachmentDef {
    pub image: String,
    pub anchor: SpriteAnchorDef,
    pub scale: Vec2,
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
