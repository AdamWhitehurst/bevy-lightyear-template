//! Canonical, deterministic RON serialization for animation assets.
//!
//! `ron` owns all syntax and layout; determinism comes from the types: `serialize_anim`
//! wraps the asset in [`CanonicalAnim`], whose `Serialize` impl emits the
//! `bone_timelines` `HashMap` in rig bone order (an external parameter no derive can
//! thread through), while the animset's `ability_animations` is a `BTreeMap` and needs
//! no wrapper at all. Pure string-returning functions — the disk write path lives in
//! the editor.

use crate::asset::{SpriteAnimAsset, SpriteAnimSetAsset};
use ron::extensions::Extensions;
use ron::ser::PrettyConfig;
use serde::ser::{SerializeMap, SerializeStruct};
use serde::{Serialize, Serializer};

/// Serializes a clip to canonical RON: `bone_timelines` emitted in `bone_order` (the
/// rig's bone-index order), default `blend_mode` (`Override`) omitted via its serde
/// attr, keyframes one per line. Deterministic and stable — serializing the
/// round-tripped result again is byte-identical.
pub fn serialize_anim(asset: &SpriteAnimAsset, bone_order: &[String]) -> String {
    let doc = ron::ser::to_string_pretty(&CanonicalAnim { asset, bone_order }, pretty_config(4))
        .expect("RON serialization of a clip cannot fail");
    finish_document(doc, |out| ron::de::from_str::<SpriteAnimAsset>(out).is_ok())
}

/// Serializes an animset to canonical RON. The derived `Serialize` is already canonical:
/// `ability_animations` is a sorted `BTreeMap`, and the `implicit_some` extension writes
/// `hit_react` bare when `Some` and `None` otherwise.
pub fn serialize_animset(asset: &SpriteAnimSetAsset) -> String {
    let doc = ron::ser::to_string_pretty(asset, pretty_config(3))
        .expect("RON serialization of an animset cannot fail");
    finish_document(doc, |out| {
        ron::de::from_str::<SpriteAnimSetAsset>(out).is_ok()
    })
}

/// Serialize-only view of a clip that emits `bone_timelines` in rig bone order instead
/// of `HashMap` iteration order. Field list mirrors `SpriteAnimAsset` — the round-trip
/// tests guard against drift.
struct CanonicalAnim<'a> {
    asset: &'a SpriteAnimAsset,
    bone_order: &'a [String],
}

impl Serialize for CanonicalAnim<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("SpriteAnimAsset", 5)?;
        s.serialize_field("name", &self.asset.name)?;
        s.serialize_field("duration", &self.asset.duration)?;
        s.serialize_field("looping", &self.asset.looping)?;
        s.serialize_field(
            "bone_timelines",
            &OrderedTimelines {
                asset: self.asset,
                bone_order: self.bone_order,
            },
        )?;
        s.serialize_field("events", &self.asset.events)?;
        s.end()
    }
}

/// The `bone_timelines` map re-keyed into rig bone order for serialization.
struct OrderedTimelines<'a> {
    asset: &'a SpriteAnimAsset,
    bone_order: &'a [String],
}

impl Serialize for OrderedTimelines<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let bones = ordered_bones(self.asset, self.bone_order);
        let mut map = serializer.serialize_map(Some(bones.len()))?;
        for bone in &bones {
            map.serialize_entry(bone, &self.asset.bone_timelines[bone])?;
        }
        map.end()
    }
}

/// `bone_timelines` keys in rig `bone_order`. Bones the rig order doesn't know are
/// appended in sorted-name order — a clip animating an unknown bone is a bug, so this
/// also `debug_assert!`s.
fn ordered_bones(asset: &SpriteAnimAsset, bone_order: &[String]) -> Vec<String> {
    let mut ordered: Vec<String> = bone_order
        .iter()
        .filter(|bone| asset.bone_timelines.contains_key(*bone))
        .cloned()
        .collect();
    let mut unknown: Vec<String> = asset
        .bone_timelines
        .keys()
        .filter(|bone| !bone_order.contains(bone))
        .cloned()
        .collect();
    debug_assert!(
        unknown.is_empty(),
        "clip '{}' animates bones missing from the rig bone order: {unknown:?}",
        asset.name
    );
    unknown.sort();
    ordered.extend(unknown);
    ordered
}

/// Shared output shape: anonymous structs, the `#![enable(implicit_some)]` header with
/// bare `Some` values, and containers deeper than `depth_limit` rendered inline — which
/// puts keyframes (clips) and locomotion entries (animsets) one per line like the
/// authored files.
fn pretty_config(depth_limit: usize) -> PrettyConfig {
    PrettyConfig::default()
        .struct_names(false)
        .depth_limit(depth_limit)
        .extensions(Extensions::IMPLICIT_SOME)
}

/// Appends the trailing newline and validates the document parses back.
fn finish_document(mut doc: String, parses: impl Fn(&str) -> bool) -> String {
    doc.push('\n');
    debug_assert!(parses(&doc), "serializer produced unparseable RON:\n{doc}");
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{BoneBlendMode, BoneTimeline, CurveType, RotationKeyframe};
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Humanoid rig bone-index order (matches `topological_sort_bones` over
    /// `humanoid.rig.ron`); hard-coded for tests only.
    const HUMANOID_BONE_ORDER: [&str; 7] =
        ["root", "torso", "head", "arm_l", "arm_r", "leg_l", "leg_r"];

    fn bone_order() -> Vec<String> {
        HUMANOID_BONE_ORDER.iter().map(|s| s.to_string()).collect()
    }

    fn assets_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    #[test]
    fn authored_clips_round_trip_lossless_and_stable() {
        let clips = [
            "idle",
            "walk",
            "run",
            "jump",
            "punch",
            "punch2",
            "ground_pound",
        ];
        for clip in clips {
            let path = assets_dir().join(format!("anims/humanoid/{clip}.anim.ron"));
            let authored = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {path:?}: {e}"));
            let original: SpriteAnimAsset = ron::de::from_str(&authored)
                .unwrap_or_else(|e| panic!("failed to parse authored {clip}: {e}"));

            let serialized = serialize_anim(&original, &bone_order());
            let reparsed: SpriteAnimAsset = ron::de::from_str(&serialized)
                .unwrap_or_else(|e| panic!("failed to reparse {clip}: {e}\n{serialized}"));
            assert_eq!(original, reparsed, "{clip}: round trip changed the value");
            assert_eq!(
                serialized,
                serialize_anim(&reparsed, &bone_order()),
                "{clip}: second serialization pass not byte-identical"
            );
        }
    }

    #[test]
    fn authored_animset_round_trips_lossless_and_stable() {
        let path = assets_dir().join("anims/humanoid/humanoid.animset.ron");
        let authored = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {path:?}: {e}"));
        let original: SpriteAnimSetAsset =
            ron::de::from_str(&authored).expect("failed to parse authored animset");

        let serialized = serialize_animset(&original);
        let reparsed: SpriteAnimSetAsset = ron::de::from_str(&serialized)
            .unwrap_or_else(|e| panic!("failed to reparse animset: {e}\n{serialized}"));
        assert_eq!(original, reparsed, "round trip changed the animset");
        assert_eq!(
            serialized,
            serialize_animset(&reparsed),
            "second serialization pass not byte-identical"
        );
    }

    fn one_key_timeline(blend_mode: BoneBlendMode) -> BoneTimeline {
        BoneTimeline {
            blend_mode,
            rotation: vec![RotationKeyframe {
                time: 0.0,
                value: 0.0,
                curve: CurveType::Linear,
            }],
            translation: vec![],
            scale: vec![],
        }
    }

    #[test]
    fn bone_timelines_emit_in_bone_order_not_map_order() {
        let mut bone_timelines = HashMap::new();
        for bone in ["leg_r", "root", "head"] {
            bone_timelines.insert(bone.to_string(), one_key_timeline(BoneBlendMode::Override));
        }
        let asset = SpriteAnimAsset {
            name: "order".to_string(),
            duration: 1.0,
            looping: false,
            bone_timelines,
            events: vec![],
        };
        let serialized = serialize_anim(&asset, &bone_order());
        let positions: Vec<usize> = ["\"root\"", "\"head\"", "\"leg_r\""]
            .iter()
            .map(|needle| serialized.find(needle).expect("bone missing from output"))
            .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn default_blend_mode_omitted_additive_kept() {
        let mut bone_timelines = HashMap::new();
        bone_timelines.insert(
            "root".to_string(),
            one_key_timeline(BoneBlendMode::Override),
        );
        bone_timelines.insert(
            "torso".to_string(),
            one_key_timeline(BoneBlendMode::Additive),
        );
        let asset = SpriteAnimAsset {
            name: "modes".to_string(),
            duration: 1.0,
            looping: false,
            bone_timelines,
            events: vec![],
        };
        let serialized = serialize_anim(&asset, &bone_order());
        assert_eq!(serialized.matches("blend_mode").count(), 1);
        assert!(serialized.contains("blend_mode: Additive"));
        let reparsed: SpriteAnimAsset = ron::de::from_str(&serialized).expect("reparse");
        assert_eq!(asset, reparsed);
    }

    #[test]
    #[should_panic(expected = "missing from the rig bone order")]
    fn unknown_bone_is_a_bug() {
        let mut bone_timelines = HashMap::new();
        bone_timelines.insert(
            "tail".to_string(),
            one_key_timeline(BoneBlendMode::Override),
        );
        let asset = SpriteAnimAsset {
            name: "unknown".to_string(),
            duration: 1.0,
            looping: false,
            bone_timelines,
            events: vec![],
        };
        serialize_anim(&asset, &bone_order());
    }
}
