//! Bounds Gaussian filter planning for pathological and nonfinite input.
//! Previously sigma² overflowed to Infinity and the variance-decimation loop
//! could never terminate. The u16 raster extent is a sufficient upper cap for
//! filter planning while keeping regular finite sigma bit-for-bit unchanged.

pub(super) fn bounded(sigma: f32) -> f32 {
    if sigma.is_nan() || sigma <= 1.0e-6 {
        0.0
    } else {
        sigma.min(65_535.0)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{compute_gaussian_kernel, plan_decimated_blur, MAX_KERNEL_SIZE};

    fn assert_normalized(weights: &[f32; MAX_KERNEL_SIZE], count: u8) {
        assert!((1..=MAX_KERNEL_SIZE as u8).contains(&count));
        let active = &weights[..usize::from(count)];
        assert!(active.iter().all(|w| w.is_finite() && *w >= 0.0),
            "invalid weights: {active:?}");
        let sum: f32 = active.iter().sum();
        assert!((sum - 1.0).abs() < 0.0001, "unnormalized weights: {active:?}");
    }

    #[test]
    fn gaussian_blur_untrusted_sigma_public_plan_is_bounded() {
        for sigma in [
            f32::NAN, f32::NEG_INFINITY, f32::INFINITY,
            f32::MAX, 1.0e30, 1.0e20, f32::MIN_POSITIVE,
            0.0, -1.0, 1.0e-30, 0.1, 1.0, 2.0, 65_535.0,
        ] {
            let (decimations, kernel, length) = plan_decimated_blur(sigma);
            assert!(decimations <= 20,
                "sigma={sigma:?}: {decimations} decimation levels");
            assert_normalized(&kernel, length);
            if sigma.is_nan() || sigma <= 1.0e-6 {
                assert_eq!((decimations, length, kernel[0]), (0, 1, 1.0));
            }
        }
    }

    #[test]
    fn gaussian_blur_untrusted_sigma_direct_kernel_is_finite() {
        for sigma in [
            f32::NAN, f32::NEG_INFINITY, f32::INFINITY,
            1.0e30, f32::MAX, 0.0, f32::MIN_POSITIVE,
        ] {
            let (kernel, count) = compute_gaussian_kernel(sigma);
            assert_normalized(&kernel, count);
        }
        let (_, baseline) = compute_gaussian_kernel(1.0);
        assert_eq!(baseline, 7, "ordinary sigma must retain its radius");
        let (decimations, _, count) = plan_decimated_blur(1.0);
        assert_eq!((decimations, count), (0, 7));
    }
}
