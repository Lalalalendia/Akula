//! Diagnostic reproducer for linebender/vello#1289 in Akula's actual CPU
//! image-brush rasterizer. No product image path/renderer modifications.
//!
//! At integer downscale ratios, the mean of a two-pixel alternating stripe
//! over each covered block is 127.5. A fixed-size 2x2/4x4 sampling footprint
//! cannot integrate an 8x8 or 16x16 source region and may produce phase
//! dependent dark/light bands instead.
//!
//! The test intentionally fails with measured error if image sampling exceeds
//! an area-filtered reference; it is ignored outside the dedicated diagnostic
//! workflow. This isn't a gate on unrelated application PRs.

use std::sync::Arc;

use vello_cpu::kurbo::{Affine, Rect};
use vello_cpu::peniko::{Brush, Extend, ImageAlphaType, ImageQuality, ImageSampler};
use vello_cpu::{
    Image, ImageSource, PixelMetadata, Pixmap, RasterizerSettings, RenderContext,
    RenderSettings, Resources, TargetInit,
};
use vello_cpu::color::palette::css::WHITE;

const SOURCE: u16 = 128;
const REFERENCE_GRAY: i32 = 128;
const MAX_ACCEPTABLE_ERROR: i32 = 24;

fn source_stripes(phase: usize) -> Pixmap {
    let mut rgba = Vec::with_capacity(usize::from(SOURCE) * usize::from(SOURCE) * 4);
    for _y in 0..usize::from(SOURCE) {
        for x in 0..usize::from(SOURCE) {
            // Period four; any aligned 4x4, 8x8 or 16x16 block has
            // exactly 50% white coverage irrespective of pattern phase.
            let value = if (x + phase) % 4 < 2 { 255 } else { 0 };
            rgba.extend_from_slice(&[value, value, value, 255]);
        }
    }
    Pixmap::from_parts(
        rgba, SOURCE, SOURCE,
        PixelMetadata::new(ImageAlphaType::AlphaPremultiplied, false),
    )
}

fn downsample(ratio: u16, phase: usize, quality: ImageQuality, workers: u16) -> Pixmap {
    let output = SOURCE / ratio;
    let source = Arc::new(source_stripes(phase));
    let image = Image {
        image: ImageSource::Pixmap(source),
        sampler: ImageSampler {
            x_extend: Extend::Pad,
            y_extend: Extend::Pad,
            quality,
            alpha: 1.0,
        },
    };
    let mut ctx = RenderContext::new_with(
        output, output,
        RenderSettings { num_threads: workers, ..RenderSettings::default() },
    );
    ctx.set_transform(Affine::scale(1.0 / f64::from(ratio)));
    ctx.set_paint(Brush::Image(image));
    ctx.fill_rect(&Rect::new(0.0, 0.0, f64::from(SOURCE), f64::from(SOURCE)));
    ctx.flush();
    let mut result = Pixmap::new(output, output);
    ctx.render_with(
        &mut result, &mut Resources::default(),
        RasterizerSettings {
            target_init: TargetInit::Clear(WHITE),
            ..RasterizerSettings::default()
        },
    );
    result
}

#[test]
#[ignore = "upstream #1289 diagnostic expected to fail until high-quality minification is implemented"]
fn vello_downscale_area_reference_falsifier() {
    let mut broken = Vec::new();
    let mut checked = 0usize;

    // 4x, 8x and 16x reduction represent icons/covers and thumbnails.
    for workers in [0u16, 2] {
        for ratio in [4u16, 8, 16] {
            for phase in 0..4usize {
                for quality in [ImageQuality::Medium, ImageQuality::High] {
                    let image = downsample(ratio, phase, quality, workers);
                    let size = usize::from(SOURCE / ratio);
                    let mut worst = 0i32;
                    let mut failing_pixels = 0usize;
                    let mut sum = 0u64;
                    let mut samples = 0usize;
                    // Crop the boundary: test the sampling kernel, not
                    // antialiasing at the outer transformed path edge.
                    for y in 2..size.saturating_sub(2) {
                        for x in 2..size.saturating_sub(2) {
                            let pixel = image.data()[y * size + x];
                            let delta = (i32::from(pixel.r) - REFERENCE_GRAY).abs();
                            worst = worst.max(delta);
                            failing_pixels += usize::from(delta > MAX_ACCEPTABLE_ERROR);
                            sum += delta as u64;
                            samples += 1;
                        }
                    }
                    assert!(samples > 0, "too small a downscale fixture");
                    checked += 1;
                    eprintln!(
                        "VELLO_DOWNSCALE_1289 ratio={ratio} phase={phase} workers={workers} \\
                         quality={quality:?} pixels={samples} failing={failing_pixels} \\
                         peak_error={worst} mean_abs_error_x100={}",
                        sum * 100 / samples as u64,
                    );
                    if failing_pixels > 0 {
                        broken.push((ratio, phase, workers, quality, failing_pixels, worst));
                    }
                }
            }
        }
    }

    assert_eq!(checked, 2 * 3 * 4 * 2);
    assert!(
        broken.is_empty(),
        "Vello #1289: {}/{} configurations exceed {} gray levels from \\
         a known area-average reference; see VELLO_DOWNSCALE_1289 diagnostics",
        broken.len(), checked, MAX_ACCEPTABLE_ERROR,
    );
}
