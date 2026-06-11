use bevy::prelude::*;
use sprite_rig::asset::{BoneTimeline, CurveType};

use crate::state::Channel;

/// The rotation channel as `(time, degrees, curve)` tuples for sampling/plotting.
pub fn rotation_keys(timeline: &BoneTimeline) -> Vec<(f32, f32, CurveType)> {
    timeline
        .rotation
        .iter()
        .map(|k| (k.time, k.value, k.curve))
        .collect()
}

/// A 2-component channel as `(time, value, curve)` tuples. `TranslationKeyframe` and
/// `ScaleKeyframe` are distinct types, so the unification happens here.
///
/// Panics on `Channel::Rotation` — that channel is scalar; use [`rotation_keys`].
pub fn vec2_keys(timeline: &BoneTimeline, channel: Channel) -> Vec<(f32, Vec2, CurveType)> {
    match channel {
        Channel::Translation => timeline
            .translation
            .iter()
            .map(|k| (k.time, k.value, k.curve))
            .collect(),
        Channel::Scale => timeline
            .scale
            .iter()
            .map(|k| (k.time, k.value, k.curve))
            .collect(),
        Channel::Rotation => panic!("vec2_keys called with the scalar rotation channel"),
    }
}

/// Samples a scalar channel (rotation degrees) at clip time `t`, honoring per-segment
/// `CurveType` — the same Step-holds-left / Linear-lerp rules as the runtime's
/// `SegmentedKeyframeCurve`, but over raw authored values (degrees, un-baked). Display
/// reader only: never poses the rig.
///
/// Panics on an empty key slice — callers plot/inspect only non-empty channels.
pub fn sample_scalar(keys: &[(f32, f32, CurveType)], t: f32) -> f32 {
    sample_keys(keys, t, |a, b, s| a + (b - a) * s)
}

/// Samples a 2-component channel (translation/scale) at `t`, per component.
pub fn sample_vec2(keys: &[(f32, Vec2, CurveType)], t: f32) -> Vec2 {
    sample_keys(keys, t, Vec2::lerp)
}

/// Shared segment lookup: clamp outside the key range, then interpolate the containing
/// segment by the left key's `CurveType`.
fn sample_keys<T: Copy>(keys: &[(f32, T, CurveType)], t: f32, lerp: impl Fn(T, T, f32) -> T) -> T {
    assert!(!keys.is_empty(), "sample_keys requires at least one key");
    if t <= keys[0].0 {
        return keys[0].1;
    }
    let last = keys.len() - 1;
    if t >= keys[last].0 {
        return keys[last].1;
    }
    let i = keys
        .partition_point(|(time, _, _)| *time <= t)
        .saturating_sub(1);
    let (t0, v0, curve) = keys[i];
    let (t1, v1, _) = keys[i + 1];
    match curve {
        CurveType::Step => v0,
        CurveType::Linear => {
            let s = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
            lerp(v0, v1, s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_step_holds_linear_lerps_clamps() {
        let keys = [
            (0.0, 0.0, CurveType::Step),
            (1.0, 10.0, CurveType::Linear),
            (2.0, 20.0, CurveType::Linear),
        ];
        assert_eq!(sample_scalar(&keys, 0.5), 0.0);
        assert_eq!(sample_scalar(&keys, 1.5), 15.0);
        assert_eq!(sample_scalar(&keys, -1.0), 0.0);
        assert_eq!(sample_scalar(&keys, 3.0), 20.0);
    }

    #[test]
    fn scalar_single_key_is_constant() {
        let keys = [(0.5, 7.0, CurveType::Linear)];
        assert_eq!(sample_scalar(&keys, 0.0), 7.0);
        assert_eq!(sample_scalar(&keys, 0.5), 7.0);
        assert_eq!(sample_scalar(&keys, 9.0), 7.0);
    }

    #[test]
    fn vec2_lerps_per_component() {
        let keys = [
            (0.0, Vec2::new(0.0, 4.0), CurveType::Linear),
            (1.0, Vec2::new(2.0, 0.0), CurveType::Linear),
        ];
        assert_eq!(sample_vec2(&keys, 0.5), Vec2::new(1.0, 2.0));
    }

    #[test]
    fn vec2_step_holds_left() {
        let keys = [
            (0.0, Vec2::ZERO, CurveType::Step),
            (1.0, Vec2::ONE, CurveType::Linear),
        ];
        assert_eq!(sample_vec2(&keys, 0.99), Vec2::ZERO);
        assert_eq!(sample_vec2(&keys, 1.0), Vec2::ONE);
    }
}
