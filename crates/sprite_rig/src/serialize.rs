//! Canonical, deterministic RON serialization for animation assets.
//!
//! `ron` serializes `HashMap`s in nondeterministic iteration order, so the outer
//! `bone_timelines` / `ability_animations` maps are emitted manually in a canonical
//! order, delegating every map-free sub-value (keyframe vecs, leaf scalars) to `ron`.
//! Pure string-returning functions — the disk write path lives in the editor.

use crate::asset::{BoneBlendMode, SpriteAnimAsset, SpriteAnimSetAsset};
use ron::ser::PrettyConfig;
use serde::Serialize;

/// Header enabling RON's `implicit_some` extension; every authored asset file starts
/// with this, and `Some` values are emitted bare to match.
const RON_HEADER: &str = "#![enable(implicit_some)]\n";

/// Serializes a clip to canonical RON: `bone_timelines` emitted in `bone_order` (the
/// rig's bone-index order), default `blend_mode` (`Override`) omitted, keyframes one per
/// line. Deterministic and stable — serializing the round-tripped result again is
/// byte-identical.
pub fn serialize_anim(asset: &SpriteAnimAsset, bone_order: &[String]) -> String {
    let mut out = String::from(RON_HEADER);
    out.push_str("(\n");
    out.push_str(&format!("    name: {},\n", leaf(&asset.name)));
    out.push_str(&format!("    duration: {},\n", leaf(&asset.duration)));
    out.push_str(&format!("    looping: {},\n", leaf(&asset.looping)));
    out.push_str("    bone_timelines: {\n");
    for bone in ordered_bones(asset, bone_order) {
        let timeline = &asset.bone_timelines[&bone];
        out.push_str(&format!("        {}: (\n", leaf(&bone)));
        if timeline.blend_mode != BoneBlendMode::default() {
            out.push_str(&format!(
                "            blend_mode: {},\n",
                leaf(&timeline.blend_mode)
            ));
        }
        out.push_str(&format!(
            "            rotation: {},\n",
            nested(&timeline.rotation, "            ")
        ));
        out.push_str(&format!(
            "            translation: {},\n",
            nested(&timeline.translation, "            ")
        ));
        out.push_str(&format!(
            "            scale: {},\n",
            nested(&timeline.scale, "            ")
        ));
        out.push_str("        ),\n");
    }
    out.push_str("    },\n");
    out.push_str(&format!("    events: {},\n", nested(&asset.events, "    ")));
    out.push_str(")\n");

    debug_assert!(
        ron::de::from_str::<SpriteAnimAsset>(&out).is_ok(),
        "serialize_anim produced unparseable RON:\n{out}"
    );
    out
}

/// Serializes an animset to canonical RON: `ability_animations` emitted in sorted-key
/// order, `hit_react` written bare when `Some` (the `implicit_some` header makes it
/// round-trip) and `None` otherwise.
pub fn serialize_animset(asset: &SpriteAnimSetAsset) -> String {
    let mut out = String::from(RON_HEADER);
    out.push_str("(\n");
    out.push_str(&format!("    rig: {},\n", leaf(&asset.rig)));
    out.push_str("    locomotion: (\n");
    out.push_str(&format!(
        "        entries: {},\n",
        nested(&asset.locomotion.entries, "        ")
    ));
    out.push_str("    ),\n");
    out.push_str("    ability_animations: {\n");
    let mut ability_ids: Vec<&String> = asset.ability_animations.keys().collect();
    ability_ids.sort();
    for id in ability_ids {
        out.push_str(&format!(
            "        {}: {},\n",
            leaf(id),
            leaf(&asset.ability_animations[id])
        ));
    }
    out.push_str("    },\n");
    match &asset.hit_react {
        Some(path) => out.push_str(&format!("    hit_react: {},\n", leaf(path))),
        None => out.push_str("    hit_react: None,\n"),
    }
    out.push_str(")\n");

    debug_assert!(
        ron::de::from_str::<SpriteAnimSetAsset>(&out).is_ok(),
        "serialize_animset produced unparseable RON:\n{out}"
    );
    out
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

/// A single-line RON leaf value (string/float/bool/enum) — `ron`'s compact form.
fn leaf<T: Serialize>(value: &T) -> String {
    ron::ser::to_string(value).expect("RON leaf serialization cannot fail")
}

/// A `ron`-pretty-rendered block (keyframe vec, locomotion entries), re-indented so
/// every line after the first nests at `indent` inside the assembled document. Structs
/// render compact (one keyframe per line), matching the authored style.
fn nested<T: Serialize>(value: &T, indent: &str) -> String {
    let config = PrettyConfig::default()
        .struct_names(false)
        .compact_structs(true);
    let block =
        ron::ser::to_string_pretty(value, config).expect("RON block serialization cannot fail");
    block.replace('\n', &format!("\n{indent}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{BoneTimeline, CurveType, RotationKeyframe};
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
