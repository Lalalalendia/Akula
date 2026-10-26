//! Fail-safe conversion for invalid premultiplied RGBA8 input (Vello #2005).
//!
//! Valid premultiplied channels obey R,G,B <= A; an untrusted pixmap may violate
//! that invariant. Multiplying one such channel by the fixed-point reciprocal
//! can exceed u16 (and +128 can overflow). Normalize *only malformed chunks*
//! before the vectorized conversion, preserving the fast SIMD path and every
//! byte of valid premultiplied data.

pub(super) fn clamp(data: &mut [u8]) {
    if data.chunks_exact(4).all(|px| px[..3].iter().all(|&channel| channel <= px[3])) {
        return;
    }
    for px in data.chunks_exact_mut(4) {
        let alpha = px[3];
        for channel in &mut px[..3] {
            *channel = (*channel).min(alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::unpremultiply_rgba8;
    use super::clamp;
    use alloc::vec::Vec;

    fn expected(component: u8, alpha: u8) -> u8 {
        if alpha == 0 { return 0; }
        let component = u32::from(component.min(alpha));
        ((component * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255) as u8
    }

    #[test]
    fn unpremultiply_malformed_scalar_tail_clamps_without_overflow() {
        // Exact upstream #2005 reproducer that used to overflow at +128.
        let mut pixels = [102, 179, 217, 108];
        assert!(unpremultiply_rgba8(&mut pixels));
        assert_eq!(pixels, [241, 255, 255, 108]);

        let mut transparent = [255, 42, 17, 0];
        assert!(unpremultiply_rgba8(&mut transparent));
        assert_eq!(transparent, [0, 0, 0, 0]);

        let mut opaque = [255, 42, 17, 255];
        assert!(!unpremultiply_rgba8(&mut opaque));
        assert_eq!(opaque, [255, 42, 17, 255]);
    }

    #[test]
    fn unpremultiply_malformed_simd_body_and_scalar_tail_match_reference() {
        let mut pixels = Vec::new();
        // 17 pixels exercise both the 64-byte SIMD body and the scalar tail.
        for i in 0..17_u8 {
            let alpha = [0_u8, 1, 2, 7, 64, 108, 128, 254, 255][usize::from(i) % 9];
            pixels.extend_from_slice(&[255, alpha.saturating_add(1), alpha / 2, alpha]);
        }
        let input = pixels.clone();
        assert!(unpremultiply_rgba8(&mut pixels));
        for (before, actual) in input.chunks_exact(4).zip(pixels.chunks_exact(4)) {
            for channel in 0..3 {
                let want = expected(before[channel], before[3]);
                assert!(actual[channel].abs_diff(want) <= 1,
                    "SIMD/scalar mismatch channel={channel}, before={before:?}, got={actual:?}, expected={want}");
            }
            assert_eq!(actual[3], before[3]);
        }
    }

    #[test]
    fn unpremultiply_malformed_valid_input_is_byte_preserving_before_conversion() {
        let mut valid = Vec::new();
        for i in 0..19_u8 {
            let a = i.saturating_mul(13);
            valid.extend_from_slice(&[a, a / 2, 0, a]);
        }
        let baseline = valid.clone();
        clamp(&mut valid);
        assert_eq!(valid, baseline, "valid RGBA must not be rewritten");
        let was_transparent = unpremultiply_rgba8(&mut valid);
        assert!(was_transparent);
        for (before, actual) in baseline.chunks_exact(4).zip(valid.chunks_exact(4)) {
            for channel in 0..3 {
                assert!(actual[channel].abs_diff(expected(before[channel], before[3])) <= 1);
            }
        }
    }
}
