//! Experimental Vello #1289 prefilter for Akula-owned Markdown image brushes.
//!
//! No glyph-atlas image interception. No global change to Vello sampling.
//! A 2x2 premultiplied RGBA box reduction costs O(source pixels) at each level.
//! Non-power-of-two dimensions use edge-clamped 2x2 blocks. Source-space
//! geometry remains unchanged; the image brush transform compensates the mip.

use vello_cpu::{PixelMetadata, Pixmap};
use vello_cpu::peniko::ImageAlphaType;

pub const MAX_LEVELS: u8 = 4;
pub const MAX_PIXELS: usize = 16_000_000;
pub const MAX_EXTRA_BYTES: usize = 32 * 1024 * 1024;

pub fn half(source: &Pixmap) -> Option<Pixmap> {
    let (width, height) = (source.width(), source.height());
    if width == 0 || height == 0 || (width == 1 && height == 1) {
        return None;
    }
    let (dst_w, dst_h) = (width.div_ceil(2), height.div_ceil(2));
    let src = source.data_as_u8_slice();
    let stride = usize::from(width) * 4;
    let (max_x, max_y) = (usize::from(width) - 1, usize::from(height) - 1);
    let mut out = Vec::with_capacity(usize::from(dst_w) * usize::from(dst_h) * 4);
    // Derive the opacity hint from the reduced pixels, not from stale source
    // metadata. Vello's opaque-image fast path depends on this exact flag.
    let mut may_have_transparency = false;
    for y in 0..usize::from(dst_h) {
        for x in 0..usize::from(dst_w) {
            // Extend::Pad semantics at odd right/bottom edges: repeat the
            // final source texel instead of rejecting real-world image sizes.
            let (x0, y0) = (x * 2, y * 2);
            let (x1, y1) = ((x0 + 1).min(max_x), (y0 + 1).min(max_y));
            let offsets = [y0 * stride + x0 * 4, y0 * stride + x1 * 4,
                           y1 * stride + x0 * 4, y1 * stride + x1 * 4];
            for channel in 0..4 {
                let sum: u16 = offsets.iter().map(|&at| u16::from(src[at + channel])).sum();
                let averaged = ((sum + 2) / 4) as u8;
                if channel == 3 && averaged != 255 {
                    may_have_transparency = true;
                }
                out.push(averaged);
            }
        }
    }
    Some(Pixmap::from_parts(
        out,
        dst_w,
        dst_h,
        PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, may_have_transparency),
    ))
}

/// Diagnostic-only row-oriented packed RGBA reducer. Both source and
/// destination are contiguous RGBA8, using the identical rounding and edge
/// clamping as half(). No allocation/cache policy change in installed builds.
#[cfg(any(test, not(feature = "installed")))]
pub fn half_row(source: &Pixmap) -> Option<Pixmap> {
    let (w, h) = (source.width(), source.height());
    if w == 0 || h == 0 || (w == 1 && h == 1) {
        return None;
    }
    let (width, height) = (usize::from(w), usize::from(h));
    let src = source.data_as_u8_slice();
    // Verify the full read envelope BEFORE using any raw pointer arithmetic.
    let src_len = width.checked_mul(height)?.checked_mul(4)?;
    if src.len() != src_len {
        return None;
    }
    let (dst_w, dst_h) = (width.div_ceil(2), height.div_ceil(2));
    let out_len = dst_w.checked_mul(dst_h)?.checked_mul(4)?;
    let mut out = vec![0_u8; out_len];
    let (src_ptr, dst_ptr) = (src.as_ptr(), out.as_mut_ptr());
    let src_stride = width * 4;
    let dst_stride = dst_w * 4;
    let mut transparent = false;
    for y in 0..dst_h {
        let src_row0 = (2 * y) * src_stride;
        let src_row1 = (2 * y + 1).min(height - 1) * src_stride;
        let dst_row = y * dst_stride;
        for x in 0..dst_w {
            let col0 = (2 * x) * 4;
            let col1 = (2 * x + 1).min(width - 1) * 4;
            // SAFETY: src is exactly width*height*4 bytes, rows/columns
            // are clamped to valid pixel coordinates, so each complete
            // 4-byte unaligned load is in-bounds. A separate owned Vec
            // with out_len initialized bytes provides the disjoint output;
            // the 4-byte write offset is in-bounds for every dst pixel.
            let reduced = unsafe {
                let read = |row: usize, col: usize| {
                    u32::from_le(src_ptr.add(row + col).cast::<u32>().read_unaligned())
                };
                packed_average4(
                    read(src_row0, col0),
                    read(src_row0, col1),
                    read(src_row1, col0),
                    read(src_row1, col1),
                )
            };
            transparent |= (reduced >> 24) != 255;
            unsafe {
                dst_ptr
                    .add(dst_row + x * 4)
                    .cast::<u32>()
                    .write_unaligned(reduced.to_le());
            }
        }
    }
    Some(Pixmap::from_parts(
        out,
        dst_w as u16,
        dst_h as u16,
        PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, transparent),
    ))
}

/// B2: direct L2 without allocating/materializing L1, preserving the
/// **two separate roundings** of half(half(src)). Test-only.
#[cfg(any(test, not(feature = "installed")))]
#[inline]
fn packed_average4(a: u32, b: u32, c: u32, d: u32) -> u32 {
    const RB: u32 = 0x00ff_00ff;
    const ROUND: u32 = 0x0002_0002;
    let even = (a & RB) + (b & RB) + (c & RB) + (d & RB);
    let odd = ((a >> 8) & RB) + ((b >> 8) & RB)
        + ((c >> 8) & RB) + ((d >> 8) & RB);
    (((even + ROUND) >> 2) & RB) | ((((odd + ROUND) >> 2) & RB) << 8)
}

#[cfg(any(test, not(feature = "installed")))]
pub fn direct_l2_packed(source: &Pixmap) -> Option<Pixmap> {
    let (w, h) = (source.width(), source.height());
    if w == 0 || h == 0 {
        return None;
    }
    let (mid_w, mid_h) = (w.div_ceil(2), h.div_ceil(2));
    // half() on the first mip returns None for an already 1x1 image.
    if mid_w == 1 && mid_h == 1 {
        return None;
    }
    let (out_w, out_h) = (mid_w.div_ceil(2), mid_h.div_ceil(2));
    let source_bytes = source.data_as_u8_slice();
    let source_stride = usize::from(w) * 4;
    let (max_x, max_y) = (usize::from(w) - 1, usize::from(h) - 1);
    let (max_mid_x, max_mid_y) = (usize::from(mid_w) - 1, usize::from(mid_h) - 1);

    let source_word = |x: usize, y: usize| {
        let at = y * source_stride + x * 4;
        u32::from_le_bytes(source_bytes[at..at + 4].try_into().expect("complete premul RGBA"))
    };
    let virtual_l1 = |x: usize, y: usize| {
        let (x0, y0) = (2 * x, 2 * y);
        let (x1, y1) = ((x0 + 1).min(max_x), (y0 + 1).min(max_y));
        packed_average4(
            source_word(x0, y0), source_word(x1, y0),
            source_word(x0, y1), source_word(x1, y1),
        )
    };
    let mut bytes = Vec::with_capacity(usize::from(out_w) * usize::from(out_h) * 4);
    let mut may_have_transparency = false;
    for y in 0..usize::from(out_h) {
        for x in 0..usize::from(out_w) {
            // Crucial for odd widths/heights: clamp in L1 coordinate
            // space *before* evaluating its 2x2 source box. This
            // reproduces half(half(src)) even for 1xN and Nx1 images.
            let (x0, y0) = (2 * x, 2 * y);
            let (x1, y1) = ((x0 + 1).min(max_mid_x), (y0 + 1).min(max_mid_y));
            let result = packed_average4(
                virtual_l1(x0, y0), virtual_l1(x1, y0),
                virtual_l1(x0, y1), virtual_l1(x1, y1),
            );
            may_have_transparency |= (result >> 24) != 255;
            bytes.extend_from_slice(&result.to_le_bytes());
        }
    }
    Some(Pixmap::from_parts(
        bytes, out_w, out_h,
        PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, may_have_transparency),
    ))
}


/// How many 2x box reductions keep the final scale at or above 1/2.
/// Intentionally supports only positive axis-aligned image transforms.
pub fn level_for_transform(transform: vello_cpu::kurbo::Affine) -> u8 {
    let [sx, syx, sxy, sy, _, _] = transform.as_coeffs();
    if !sx.is_finite() || !sy.is_finite() || sx <= 0.0 || sy <= 0.0
        || syx != 0.0 || sxy != 0.0
    {
        return 0;
    }
    // Both axes must be substantially minified to avoid softening an
    // unscaled axis in an anisotropic image transform.
    let mut scale = sx.max(sy);
    let mut level = 0;
    while scale < 0.5 && level < MAX_LEVELS {
        scale *= 2.0;
        level += 1;
    }
    level
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vello_cpu::color::palette::css::WHITE;
    use vello_cpu::kurbo::{Affine, Rect};
    use vello_cpu::peniko::{Brush, Extend, ImageAlphaType, ImageQuality, ImageSampler};
    use vello_cpu::{
        Image, ImageSource, RasterizerSettings, RenderContext, RenderSettings,
        Resources, TargetInit,
    };

    fn stripes(phase: usize) -> Pixmap {
        let mut pixels = Vec::with_capacity(128 * 128 * 4);
        for _y in 0..128 {
            for x in 0..128 {
                let value = if (x + phase) % 4 < 2 { 255 } else { 0 };
                pixels.extend_from_slice(&[value, value, value, 255]);
            }
        }
        Pixmap::from_parts(
            pixels, 128, 128,
            PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, false),
        )
    }

    fn render_with_reducer(
        ratio: u16, phase: usize, quality: ImageQuality, workers: u16,
        reducer: fn(&Pixmap) -> Option<Pixmap>,
    ) -> Pixmap {
        let base = stripes(phase);
        let original = Affine::scale(1.0 / f64::from(ratio));
        let levels = level_for_transform(original);
        assert!(levels > 0, "these fixtures deliberately downscale by >2x");
        let mut mip = base;
        for _ in 0..levels {
            mip = reducer(&mip).expect("128px test image allows each mipmap");
        }
        let mut ctx = RenderContext::new_with(
            128 / ratio, 128 / ratio,
            RenderSettings { num_threads: workers, ..RenderSettings::default() },
        );
        ctx.set_transform(original);
        ctx.set_paint_transform(Affine::scale(f64::from(1u16 << levels)));
        ctx.set_paint(Brush::Image(Image {
            image: ImageSource::Pixmap(Arc::new(mip)),
            sampler: ImageSampler {
                x_extend: Extend::Pad,
                y_extend: Extend::Pad,
                quality,
                alpha: 1.0,
            },
        }));
        ctx.fill_rect(&Rect::new(0.0, 0.0, 128.0, 128.0));
        ctx.flush();
        let mut out = Pixmap::new(128 / ratio, 128 / ratio);
        ctx.render_with(
            &mut out, &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(WHITE),
                ..RasterizerSettings::default()
            },
        );
        out
    }

    fn render(ratio: u16, phase: usize, quality: ImageQuality, workers: u16) -> Pixmap {
        render_with_reducer(ratio, phase, quality, workers, half)
    }

    #[test]
    fn row_reducer_vello_raster_pixels_match_scalar() {
        for workers in [0, 2] {
            for ratio in [4_u16, 8, 16] {
                for phase in [0, 1, 3] {
                    for quality in [ImageQuality::Medium, ImageQuality::High] {
                        let reference = render(ratio, phase, quality, workers);
                        let row = render_with_reducer(ratio, phase, quality, workers, half_row);
                        assert_eq!(reference.data_as_u8_slice(), row.data_as_u8_slice(),
                            "rendered pixel mismatch: workers={workers} ratio={ratio} phase={phase} quality={quality:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn row_reducer_byte_parity_odd_edges_and_alpha() {
        let mut sizes = vec![
            (1, 1), (1, 2), (2, 1), (1, 33), (33, 1),
            (2, 2), (3, 3), (3, 5), (5, 3), (17, 19),
            (128, 127), (511, 257), (1280, 884), (1281, 885),
            // Extreme u16 width/height with small total allocation.
            (65535, 1), (1, 65535),
        ];
        for width in 1..=9 {
            for height in 1..=9 {
                sizes.push((width, height));
            }
        }
        for (w, h) in sizes {
            for seed in [0_u32, 0x1234_5678] {
                for opaque in [false, true] {
                    let src = packed_test_source(w, h, seed, opaque);
                    let baseline = half(&src);
                    let candidate = half_row(&src);
                    assert_eq!(baseline.is_some(), candidate.is_some(), "None mismatch {w}x{h}");
                    if let (Some(expected), Some(actual)) = (baseline, candidate) {
                        assert_eq!((actual.width(), actual.height()), (expected.width(), expected.height()));
                        assert_eq!(actual.data_as_u8_slice(), expected.data_as_u8_slice(),
                            "RGBA byte mismatch {w}x{h} seed={seed} opaque={opaque}");
                        assert_eq!(actual.may_have_transparency(), expected.may_have_transparency(),
                            "metadata mismatch {w}x{h} seed={seed} opaque={opaque}");
                    }
                }
            }
        }
    }

    // Uses the actual Markdown city-cover PNG, odd dimensions, alpha crop,
    // full Vello CPU raster, two worker settings and two filter qualities.
    #[test]
    fn row_reducer_photo_alpha_odd_size_vello_pixels_match_scalar() {
        const W: u16 = 257;
        const H: u16 = 193;
        let photo = image::load_from_memory(include_bytes!("../../../assets/city-cover.png"))
            .expect("pinned Akula PNG").to_rgba8();
        let pixels = image::imageops::resize(
            &photo, u32::from(W), u32::from(H),
            image::imageops::FilterType::Lanczos3,
        ).into_raw();

        fn render_photo(mip: &Pixmap, ratio: u16, level: u8, quality: ImageQuality, workers: u16) -> Pixmap {
            let (out_w, out_h) = (W.div_ceil(ratio), H.div_ceil(ratio));
            let mut ctx = RenderContext::new_with(
                out_w, out_h,
                RenderSettings { num_threads: workers, ..RenderSettings::default() },
            );
            ctx.set_transform(Affine::scale(1.0 / f64::from(ratio)));
            ctx.set_paint_transform(Affine::scale(f64::from(1_u16 << level)));
            ctx.set_paint(Brush::Image(Image {
                image: ImageSource::Pixmap(Arc::new(Pixmap::from_parts(
                    mip.data_as_u8_slice().to_vec(), mip.width(), mip.height(),
                    PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, mip.may_have_transparency()),
                ))),
                sampler: ImageSampler {
                    x_extend: Extend::Pad, y_extend: Extend::Pad, quality, alpha: 1.0,
                },
            }));
            ctx.fill_rect(&Rect::new(0.0, 0.0, f64::from(W), f64::from(H)));
            ctx.flush();
            let mut out = Pixmap::new(out_w, out_h);
            ctx.render_with(
                &mut out, &mut Resources::default(),
                RasterizerSettings {
                    target_init: TargetInit::Clear(WHITE),
                    ..RasterizerSettings::default()
                },
            );
            out
        }

        for opaque in [true, false] {
            let mut premul = pixels.clone();
            for (i, rgba) in premul.chunks_exact_mut(4).enumerate() {
                let a = if opaque { 255 } else {
                    [0_u8, 64, 128, 192, 255][(i / 11 + i / usize::from(W)) % 5]
                };
                for rgb in &mut rgba[..3] {
                    *rgb = ((u16::from(*rgb) * u16::from(a) + 127) / 255) as u8;
                }
                rgba[3] = a;
            }
            for ratio in [4_u16, 8, 16] {
                let level = level_for_transform(Affine::scale(1.0 / f64::from(ratio)));
                assert!(level > 0);
                let mut scalar = Pixmap::from_parts(
                    premul.clone(), W, H,
                    PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, !opaque),
                );
                let mut row = Pixmap::from_parts(
                    premul.clone(), W, H,
                    PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, !opaque),
                );
                for _ in 0..level {
                    scalar = half(&scalar).expect("photo supports LOD");
                    row = half_row(&row).expect("row photo supports LOD");
                    assert_eq!(scalar.data_as_u8_slice(), row.data_as_u8_slice(),
                        "RGBA mismatch ratio={ratio} opaque={opaque}");
                    assert_eq!(scalar.may_have_transparency(), row.may_have_transparency());
                }
                for workers in [0, 2] {
                    for quality in [ImageQuality::Medium, ImageQuality::High] {
                        let expected = render_photo(&scalar, ratio, level, quality, workers);
                        let actual = render_photo(&row, ratio, level, quality, workers);
                        assert!(!actual.data_as_u8_slice().is_empty());
                        assert_eq!(expected.data_as_u8_slice(), actual.data_as_u8_slice(),
                            "Vello sampled-pixel mismatch ratio={ratio} opaque={opaque} workers={workers} quality={quality:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn reference_area_preserved_by_power_of_two_mipmaps() {
        let mut tested = 0;
        for workers in [0, 2] {
            for ratio in [4u16, 8, 16] {
                for phase in 0..4 {
                    for quality in [ImageQuality::Medium, ImageQuality::High] {
                        let frame = render(ratio, phase, quality, workers);
                        let side = usize::from(128 / ratio);
                        let mut worst = 0;
                        for y in 2..side - 2 {
                            for x in 2..side - 2 {
                                let gray = i32::from(frame.data()[y * side + x].r);
                                worst = worst.max((gray - 128).abs());
                            }
                        }
                        assert!(
                            worst <= 3,
                            "mipped Vello #1289 regression: ratio={ratio} phase={phase} \
                             workers={workers} quality={quality:?} worst={worst}/255"
                        );
                        tested += 1;
                    }
                }
            }
        }
        assert_eq!(tested, 48);
    }

    #[test]
    fn premultiplied_alpha_is_preserved_through_reduction() {
        let pixel = Pixmap::from_parts(
            vec![
                255, 0, 0, 255, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0,
            ],
            2, 2,
            PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, true),
        );
        let reduced = half(&pixel).expect("2px");
        assert_eq!(reduced.data_as_u8_slice(), &[64, 0, 0, 64]);
    }

    #[test]
    fn only_axis_aligned_strict_minification_uses_prefilter() {
        assert_eq!(level_for_transform(vello_cpu::kurbo::Affine::IDENTITY), 0);
        assert_eq!(level_for_transform(Affine::scale(0.5)), 0);
        assert_eq!(level_for_transform(Affine::scale(0.25)), 1);
        assert_eq!(level_for_transform(Affine::scale(0.125)), 2);
        assert_eq!(level_for_transform(Affine::scale(1.0 / 16.0)), 3);
        assert_eq!(level_for_transform(Affine::rotate(0.1)), 0);
        assert_eq!(level_for_transform(Affine::scale(-0.25)), 0);
    }

    #[test]
    fn opacity_hint_matches_reduced_pixels() {
        // Source metadata is conservatively translucent even though the data
        // is opaque. Mipmaps should recover Vello's opaque-image fast path.
        let opaque = Pixmap::from_parts(
            vec![200, 100, 50, 255].repeat(4),
            2, 2,
            PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, true),
        );
        assert!(opaque.may_have_transparency());
        let reduced = half(&opaque).expect("valid 2x2");
        assert!(!reduced.may_have_transparency());
        assert_eq!(reduced.data_as_u8_slice(), &[200, 100, 50, 255]);

        let translucent = Pixmap::from_parts(
            vec![64, 0, 0, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            2, 2,
            PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, true),
        );
        let reduced_alpha = half(&translucent).expect("valid 2x2");
        assert!(reduced_alpha.may_have_transparency());
        assert_eq!(reduced_alpha.data_as_u8_slice(), &[16, 0, 0, 16]);
    }


    #[test]
    fn odd_size_edges_follow_pad_semantics_without_losing_rows() {
        let src: Vec<u8> = [0u8, 40, 80, 120, 160, 200, 240, 220, 180]
            .into_iter()
            .flat_map(|v| [v, v, v, 255])
            .collect();
        let input = Pixmap::from_parts(src, 3, 3,
            PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, false));
        let image = half(&input).expect("odd dimensions must be supported");
        assert_eq!((image.width(), image.height()), (2, 2));
        let red: Vec<_> = image.data().iter().map(|pixel| pixel.r).collect();
        assert_eq!(red, [80, 140, 230, 180]);
        assert!(!image.may_have_transparency());
    }


    // B1: test-only, byte-identical RGBA reducer. No runtime dispatch,
    // production path, feature flag or unsafe implementation changes.
    fn half_packed_trial(source: &Pixmap) -> Option<Pixmap> {
        let (width, height) = (source.width(), source.height());
        if width == 0 || height == 0 || (width == 1 && height == 1) {
            return None;
        }
        let (dst_w, dst_h) = (width.div_ceil(2), height.div_ceil(2));
        let src = source.data_as_u8_slice();
        let stride = usize::from(width) * 4;
        let (max_x, max_y) = (usize::from(width) - 1, usize::from(height) - 1);
        let mut out = Vec::with_capacity(usize::from(dst_w) * usize::from(dst_h) * 4);
        let mut may_have_transparency = false;
        const LANES: u32 = 0x00ff_00ff;
        const ROUND: u32 = 0x0002_0002;
        for y in 0..usize::from(dst_h) {
            let row0 = (2 * y) * stride;
            let row1 = (2 * y + 1).min(max_y) * stride;
            for x in 0..usize::from(dst_w) {
                let x0 = 2 * x;
                let x1 = (x0 + 1).min(max_x);
                let rgba_word = |row: usize, column: usize| {
                    let at = row + column * 4;
                    u32::from_le_bytes(src[at..at + 4].try_into().expect("complete RGBA texel"))
                };
                let a = rgba_word(row0, x0);
                let b = rgba_word(row0, x1);
                let c = rgba_word(row1, x0);
                let d = rgba_word(row1, x1);
                // A 16-bit lane never overflows: four 8-bit values sum to
                // <=1020, and ROUND adds at most 2. Sum R/B and G/A in
                // separate 16-bit lanes to avoid cross-channel carries.
                let even = (a & LANES) + (b & LANES) + (c & LANES) + (d & LANES);
                let odd = ((a >> 8) & LANES) + ((b >> 8) & LANES)
                    + ((c >> 8) & LANES) + ((d >> 8) & LANES);
                let reduced = (((even + ROUND) >> 2) & LANES)
                    | ((((odd + ROUND) >> 2) & LANES) << 8);
                if reduced >> 24 != 255 {
                    may_have_transparency = true;
                }
                out.extend_from_slice(&reduced.to_le_bytes());
            }
        }
        Some(Pixmap::from_parts(
            out, dst_w, dst_h,
            PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, may_have_transparency),
        ))
    }

    fn packed_test_source(w: u16, h: u16, seed: u32, opaque: bool) -> Pixmap {
        let mut pixels = Vec::with_capacity(usize::from(w) * usize::from(h) * 4);
        let mut state = seed;
        for y in 0..h {
            for x in 0..w {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let alpha = if opaque {
                    255
                } else if x % 7 == 0 && y % 5 == 0 {
                    0
                } else {
                    (state >> 24) as u8
                };
                // Valid premultiplied texels even for mixed alpha.
                pixels.extend_from_slice(&[
                    (state as u8).min(alpha),
                    ((state >> 8) as u8).min(alpha),
                    ((state >> 16) as u8).min(alpha),
                    alpha,
                ]);
            }
        }
        Pixmap::from_parts(
            pixels, w, h,
            PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, !opaque),
        )
    }

    #[test]
    fn packed_reduction_is_byte_exact_for_odd_alpha_and_thin_images() {
        let sizes = [
            (1, 1), (1, 2), (2, 1), (1, 33), (33, 1), (2, 2),
            (3, 3), (3, 5), (5, 3), (17, 19), (128, 127),
            (511, 257), (1280, 884), (1281, 885),
        ];
        for (w, h) in sizes {
            for seed in [0u32, 1, 0x1234_5678, 0xffff_ffff] {
                for opaque in [true, false] {
                    let source = packed_test_source(w, h, seed, opaque);
                    let expected = half(&source);
                    let candidate = half_packed_trial(&source);
                    assert_eq!(expected.is_some(), candidate.is_some(),
                        "different None behavior: {w}x{h}, seed={seed}, opaque={opaque}");
                    if let (Some(expected), Some(candidate)) = (expected, candidate) {
                        assert_eq!((expected.width(), expected.height()),
                            (candidate.width(), candidate.height()));
                        assert_eq!(expected.data_as_u8_slice(), candidate.data_as_u8_slice(),
                            "byte mismatch: {w}x{h}, seed={seed}, opaque={opaque}");
                        assert_eq!(expected.may_have_transparency(),
                            candidate.may_have_transparency(),
                            "opacity hint mismatch: {w}x{h}");
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "opt-in Windows microbenchmark, non-gating performance figures"]
    fn packed_reduction_microbench_diagnostic() {
        use std::hint::black_box;
        use std::time::Instant;

        fn median(mut values: Vec<f64>) -> f64 {
            values.sort_by(|a, b| a.total_cmp(b));
            values[values.len() / 2]
        }

        for (w, h) in [(1280, 884), (1281, 885), (4096, 2160)] {
            let source = packed_test_source(w, h, 0xa51a_cafe, false);
            let mut scalar_ms = Vec::new();
            let mut packed_ms = Vec::new();
            // AB/BA alternating process-local order. Discard two warmup pairs.
            // No speed threshold or performance-based CI failure.
            for turn in 0..12 {
                let order = if turn % 2 == 0 { [0, 1] } else { [1, 0] };
                for which in order {
                    let started = Instant::now();
                    let pixmap = if which == 0 {
                        half(black_box(&source)).expect("nonempty scalar result")
                    } else {
                        half_packed_trial(black_box(&source)).expect("nonempty packed result")
                    };
                    black_box(pixmap.data_as_u8_slice());
                    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                    if turn >= 2 {
                        if which == 0 {
                            scalar_ms.push(elapsed_ms);
                        } else {
                            packed_ms.push(elapsed_ms);
                        }
                    }
                }
            }
            let scalar = median(scalar_ms);
            let packed = median(packed_ms);
            println!(
                "AKULA_MIP_B1_MICROBENCH source={}x{} scalar_ms={scalar:.4} packed_ms={packed:.4} ratio={:.3} device=ci-hosted-windows release=1",
                w, h, scalar / packed,
            );
        }
    }


    #[test]
    fn direct_l2_matches_two_separate_roundings_even_odd_and_premul() {
        let mut sizes: Vec<(u16, u16)> = (1..=11)
            .flat_map(|h| (1..=11).map(move |w| (w, h)))
            .collect();
        sizes.extend_from_slice(&[
            (1, 256), (256, 1), (2, 513), (513, 2), (31, 33),
            (128, 127), (511, 257), (1280, 884), (1281, 885),
        ]);
        for (w, h) in sizes {
            for seed in [0u32, 0x1234_5678] {
                for opaque in [true, false] {
                    let input = packed_test_source(w, h, seed, opaque);
                    let scalar = half(&input).and_then(|l1| half(&l1));
                    let packed_two_pass = half_packed_trial(&input)
                        .and_then(|l1| half_packed_trial(&l1));
                    let direct = direct_l2_packed(&input);
                    assert_eq!(scalar.is_some(), direct.is_some(),
                        "None mismatch at {w}x{h}");
                    assert_eq!(packed_two_pass.is_some(), direct.is_some(),
                        "packed 2-pass mismatch at {w}x{h}");
                    if let (Some(scalar), Some(packed), Some(direct)) =
                        (scalar, packed_two_pass, direct)
                    {
                        assert_eq!((scalar.width(), scalar.height()),
                            (direct.width(), direct.height()));
                        assert_eq!(scalar.data_as_u8_slice(), packed.data_as_u8_slice(),
                            "packed 2-pass byte mismatch at {w}x{h}");
                        assert_eq!(scalar.data_as_u8_slice(), direct.data_as_u8_slice(),
                            "direct L2 byte mismatch at {w}x{h}, seed={seed}, opaque={opaque}");
                        assert_eq!(scalar.may_have_transparency(),
                            direct.may_have_transparency(),
                            "opacity mismatch at {w}x{h}");
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "opt-in Windows microbenchmark; no production latency threshold"]
    fn direct_l2_microbench_diagnostic() {
        use std::hint::black_box;
        use std::time::Instant;

        fn median(mut numbers: Vec<f64>) -> f64 {
            numbers.sort_by(|a, b| a.total_cmp(b));
            numbers[numbers.len() / 2]
        }
        const ORDERS: [[usize; 3]; 6] = [
            [0, 1, 2], [1, 2, 0], [2, 0, 1],
            [2, 1, 0], [1, 0, 2], [0, 2, 1],
        ];
        for (w, h) in [(1280, 884), (1281, 885), (4096, 2160)] {
            let source = packed_test_source(w, h, 0xa51a_cafe, false);
            let mut durations = [Vec::new(), Vec::new(), Vec::new()];
            for trial in 0..14 {
                for kind in ORDERS[trial % ORDERS.len()] {
                    let start = Instant::now();
                    let result = match kind {
                        0 => half(black_box(&source)).and_then(|l1| half(&l1)),
                        1 => half_packed_trial(black_box(&source))
                            .and_then(|l1| half_packed_trial(&l1)),
                        _ => direct_l2_packed(black_box(&source)),
                    }.expect("source supports two levels");
                    black_box(result.data_as_u8_slice());
                    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
                    if trial >= 2 {
                        durations[kind].push(duration_ms);
                    }
                }
            }
            let [scalar_two, packed_two, direct] = durations.map(median);
            println!(
                "AKULA_MIP_B2_MICROBENCH source={w}x{h} scalar_two_ms={scalar_two:.4} packed_two_ms={packed_two:.4} direct_l2_ms={direct:.4} direct_vs_packed={:.3} direct_vs_scalar={:.3} device=ci-hosted-windows release=1",
                packed_two / direct, scalar_two / direct,
            );
        }
    }

}
