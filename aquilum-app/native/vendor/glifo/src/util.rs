// Copyright 2025 the Vello Authors and the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Utility helper functions.

use core::ops::Sub;
use peniko::kurbo::Affine;

// From <https://github.com/linebender/tiny-skia/blob/68b198a7210a6bbf752b43d6bc4db62445730313/path/src/scalar.rs#L12>
const SCALAR_NEARLY_ZERO_F32: f32 = 1.0 / (1 << 12) as f32;
const SCALAR_NEARLY_ZERO_F64: f64 = 1.0 / (1 << 12) as f64;

/// A number of useful methods for f32 numbers.
pub(crate) trait FloatExt: Sized + Sub<f32, Output = f32> {
    /// Whether the number is approximately 0.
    fn is_nearly_zero(&self) -> bool {
        self.is_nearly_zero_within_tolerance(SCALAR_NEARLY_ZERO_F32)
    }

    /// Whether the number is approximately 0, with a given tolerance.
    fn is_nearly_zero_within_tolerance(&self, tolerance: f32) -> bool;
}

impl FloatExt for f32 {
    #[inline(always)]
    fn is_nearly_zero_within_tolerance(&self, tolerance: f32) -> bool {
        debug_assert!(tolerance >= 0.0, "tolerance must be positive");

        self.abs() <= tolerance
    }
}

pub(crate) trait AffineExt {
    /// Whether the transform has any skewing coefficient.
    fn has_skew(&self) -> bool;

    /// Whether the transform has a vertical skew.
    fn has_vertical_skew(&self) -> bool;

    /// Whether the transform has positive, uniform scaling factors and no skew.
    fn is_positive_uniform_scale_without_skew(&self) -> bool;

    /// Whether the transform has positive, uniform scaling factors and no vertical skew.
    fn is_positive_uniform_scale_without_vertical_skew(&self) -> bool;

    /// Whether the transform has non-unit scale or skew.
    ///
    /// Note that negative scales (i.e. -1.0) are explicitly allowed.
    fn has_non_unit_skew_or_scale(&self) -> bool;
}

impl AffineExt for Affine {
    #[inline]
    fn has_skew(&self) -> bool {
        let [_, b, c, _, _, _] = self.as_coeffs();
        b.abs() > SCALAR_NEARLY_ZERO_F64 || c.abs() > SCALAR_NEARLY_ZERO_F64
    }

    #[inline]
    fn is_positive_uniform_scale_without_skew(&self) -> bool {
        let [a, _, _, d, _, _] = self.as_coeffs();
        (a - d).abs() <= SCALAR_NEARLY_ZERO_F64 && a > 0.0 && d > 0.0 && !self.has_skew()
    }

    #[inline]
    fn has_non_unit_skew_or_scale(&self) -> bool {
        let [a, _, _, d, _, _] = self.as_coeffs();
        self.has_skew()
            || (1.0 - a.abs()).abs() > SCALAR_NEARLY_ZERO_F64
            || (1.0 - d.abs()).abs() > SCALAR_NEARLY_ZERO_F64
    }

    /// Whether the transform has a vertical skew.
    #[inline]
    fn has_vertical_skew(&self) -> bool {
        let [_, b, _, _, _, _] = self.as_coeffs();
        b.abs() > SCALAR_NEARLY_ZERO_F64
    }

    #[inline]
    fn is_positive_uniform_scale_without_vertical_skew(&self) -> bool {
        let [a, _, _, d, _, _] = self.as_coeffs();
        (a - d).abs() <= SCALAR_NEARLY_ZERO_F64 && a > 0.0 && d > 0.0 && !self.has_vertical_skew()
    }
}


/// Tiny bias for discrete glyph-pixel decisions. Keeps full-frame and
/// integer-cropped glyph placements stable under last-ulp translation noise.
/// Must stay far below a horizontal subpixel cache bucket (currently 0.25px).
const PIXEL_DECISION_BIAS: f64 = 1e-6;

/// Integer pixel for a cached glyph. Pair with `biased_fract` so the atlas
/// lookup and final image quad use the *same* coordinate split.
#[inline]
pub(crate) fn biased_floor(v: f64) -> f64 {
    (v + PIXEL_DECISION_BIAS).floor()
}

/// Fractional part matching `biased_floor`, always in [0, 1).
/// The f64-to-f32 conversion may round the result to 1; the atlas key
/// quantizer must clamp such values rather than wrap to bucket zero.
#[inline]
pub(crate) fn biased_fract(v: f64) -> f64 {
    let biased = v + PIXEL_DECISION_BIAS;
    biased - biased.floor()
}

#[cfg(test)]
mod tests {
    use super::{AffineExt, biased_floor, biased_fract};
    use peniko::kurbo::Affine;

    #[test]
    fn glyph_pixel_split_stays_consistent_under_integer_cropping() {
        for position in [-1234.0, -12.0, -0.5, 0.0, 7.0, 9.25, 341.5, 4096.0] {
            for noise in [-1e-9, 0.0, 1e-9] {
                let x = position + noise;
                for origin in [0.0, 1.0, 29.0, 142.0, 1020.0] {
                    assert_eq!(
                        biased_floor(x - origin) + origin,
                        biased_floor(x),
                        "integer crop changed pixel: x={x}, origin={origin}"
                    );
                    assert!(
                        (biased_fract(x - origin) - biased_fract(x)).abs() < 1e-8,
                        "integer crop changed fraction: x={x}, origin={origin}"
                    );
                }
            }
        }
    }

    #[test]
    fn glyph_pixel_split_resists_noise_around_integers() {
        for pixel in [-1234.0, -1.0, 0.0, 7.0, 341.0, 4096.0] {
            for noise in [-1e-9, 0.0, 1e-9] {
                assert_eq!(biased_floor(pixel + noise), pixel);
                assert!(biased_fract(pixel + noise) < 1e-5);
            }
        }
    }

    #[test]
    fn detects_positive_uniform_scale_without_skew() {
        let transform = Affine::scale(2.0);

        assert!(!transform.has_skew());
        assert!(!transform.has_vertical_skew());
        assert!(transform.is_positive_uniform_scale_without_skew());
        assert!(transform.is_positive_uniform_scale_without_vertical_skew());
    }

    #[test]
    fn rejects_positive_uniform_scale_without_skew_when_horizontally_skewed() {
        let transform = Affine::new([2.0, 0.0, 0.25, 2.0, 0.0, 0.0]);

        assert!(transform.has_skew());
        assert!(!transform.has_vertical_skew());
        assert!(!transform.is_positive_uniform_scale_without_skew());
        assert!(transform.is_positive_uniform_scale_without_vertical_skew());
    }

    #[test]
    fn rejects_positive_uniform_scale_without_vertical_skew_when_vertically_skewed() {
        let transform = Affine::new([2.0, 0.25, 0.0, 2.0, 0.0, 0.0]);

        assert!(transform.has_skew());
        assert!(transform.has_vertical_skew());
        assert!(!transform.is_positive_uniform_scale_without_skew());
        assert!(!transform.is_positive_uniform_scale_without_vertical_skew());
    }

    #[test]
    fn rejects_non_uniform_or_non_positive_scale() {
        let non_uniform = Affine::new([2.0, 0.0, 0.0, 3.0, 0.0, 0.0]);
        let flipped = Affine::new([-2.0, 0.0, 0.0, -2.0, 0.0, 0.0]);

        assert!(non_uniform.has_non_unit_skew_or_scale());
        assert!(!non_uniform.is_positive_uniform_scale_without_skew());
        assert!(!non_uniform.is_positive_uniform_scale_without_vertical_skew());
        assert!(flipped.has_non_unit_skew_or_scale());
        assert!(!flipped.is_positive_uniform_scale_without_skew());
        assert!(!flipped.is_positive_uniform_scale_without_vertical_skew());
    }

    #[test]
    fn allows_unit_axis_flips() {
        let flip_x = Affine::new([-1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let flip_y = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);
        let flip_xy = Affine::new([-1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);

        assert!(!flip_x.has_non_unit_skew_or_scale());
        assert!(!flip_y.has_non_unit_skew_or_scale());
        assert!(!flip_xy.has_non_unit_skew_or_scale());
    }
}
