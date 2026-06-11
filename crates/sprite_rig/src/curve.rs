use bevy::math::{
    curve::{Curve, Interval},
    StableInterpolate,
};
use bevy::reflect::Reflect;

use crate::asset::CurveType;

/// A curve over unevenly-spaced keyframes where each segment interpolates according to the
/// **left** key's `CurveType`: `Linear` blends left→right via `StableInterpolate`, `Step`
/// holds the left value until the right key's time. Keys must be sorted ascending by time
/// and contain at least two entries.
#[derive(Clone, Debug, Reflect)]
pub struct SegmentedKeyframeCurve<T> {
    keys: Vec<(f32, T, CurveType)>,
    domain: Interval,
}

impl<T: StableInterpolate + Clone> SegmentedKeyframeCurve<T> {
    /// Builds the curve, returning `None` if fewer than two keys are supplied.
    pub fn new(keys: Vec<(f32, T, CurveType)>) -> Option<Self> {
        if keys.len() < 2 {
            return None;
        }
        let domain = Interval::new(keys[0].0, keys[keys.len() - 1].0).ok()?;
        Some(Self { keys, domain })
    }
}

impl<T: StableInterpolate + Clone> Curve<T> for SegmentedKeyframeCurve<T> {
    fn domain(&self) -> Interval {
        self.domain
    }

    fn sample_unchecked(&self, t: f32) -> T {
        // Clamp below first / above last.
        if t <= self.keys[0].0 {
            return self.keys[0].1.clone();
        }
        let last = self.keys.len() - 1;
        if t >= self.keys[last].0 {
            return self.keys[last].1.clone();
        }
        // Find the segment [i, i+1] containing t.
        let i = self
            .keys
            .partition_point(|(time, _, _)| *time <= t)
            .saturating_sub(1);
        let (t0, ref v0, curve) = self.keys[i];
        let (t1, ref v1, _) = self.keys[i + 1];
        match curve {
            CurveType::Step => v0.clone(),
            CurveType::Linear => {
                let s = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
                v0.interpolate_stable(v1, s)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn step_holds_left_linear_lerps_ends_clamp() {
        let curve = SegmentedKeyframeCurve::new(vec![
            (0.0, 0.0_f32, CurveType::Step),
            (1.0, 10.0, CurveType::Linear),
            (2.0, 20.0, CurveType::Linear),
        ])
        .expect("three keys build a curve");

        assert_relative_eq!(curve.sample_unchecked(0.5), 0.0);
        assert_relative_eq!(curve.sample_unchecked(1.5), 15.0);
        assert_relative_eq!(curve.sample_unchecked(-1.0), 0.0);
        assert_relative_eq!(curve.sample_unchecked(3.0), 20.0);
    }

    #[test]
    fn fewer_than_two_keys_returns_none() {
        assert!(SegmentedKeyframeCurve::new(vec![(0.0, 1.0_f32, CurveType::Linear)]).is_none());
        assert!(SegmentedKeyframeCurve::<f32>::new(vec![]).is_none());
    }
}
