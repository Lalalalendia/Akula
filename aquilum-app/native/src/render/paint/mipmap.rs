//! Experimental Vello #1289 prefilter for Akula-owned Markdown image brushes.
//!
//! No glyph-atlas image interception. No global change to Vello sampling.
//! A 2x2 premultiplied RGBA box reduction costs O(source pixels) at each level.
//! Only whole-power-of-two dimensions are admitted; odd dimensions fall back
//! to the original sampler instead of silently distorting image geometry.

use vello_cpu::{PixelMetadata, Pixmap};
use vello_cpu::peniko::ImageAlphaType;

pub const MAX_LEVELS: u8 = 4;
pub const MAX_PIXELS: usize = 16_000_000;
pub const MAX_EXTRA_BYTES: usize = 32 * 1024 * 1024;

pub fn half(source: &Pixmap) -> Option<Pixmap> {
    let (width, height) = (source.width(), source.height());
    if width < 2 || height < 2 || width % 2 != 0 || height % 2 != 0 {
        return None;
    }
    let (dst_w, dst_h) = (width / 2, height / 2);
    let src = source.data_as_u8_slice();
    let stride = usize::from(width) * 4;
    let mut out = Vec::with_capacity(usize::from(dst_w) * usize::from(dst_h) * 4);
    // Derive the opacity hint from the reduced pixels, not from stale source
    // metadata. Vello's opaque-image fast path depends on this exact flag.
    let mut may_have_transparency = false;
    for y in 0..usize::from(dst_h) {
        for x in 0..usize::from(dst_w) {
            let top = (y * 2) * stride + x * 8;
            for channel in 0..4 {
                let sum = u16::from(src[top + channel])
                    + u16::from(src[top + 4 + channel])
                    + u16::from(src[top + stride + channel])
                    + u16::from(src[top + stride + 4 + channel]);
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

    fn render(ratio: u16, phase: usize, quality: ImageQuality, workers: u16) -> Pixmap {
        let base = stripes(phase);
        let original = Affine::scale(1.0 / f64::from(ratio));
        let levels = level_for_transform(original);
        assert!(levels > 0, "these fixtures deliberately downscale by >2x");
        let mut mip = base;
        for _ in 0..levels {
            mip = half(&mip).expect("128px test image allows each mipmap");
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

}
