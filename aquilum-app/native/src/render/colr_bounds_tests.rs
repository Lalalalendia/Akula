//! Visual regression for linebender/vello#1983 using the upstream synthetic COLRv1 font.
//! Glyph 1: amber outline; glyph 2: same shape with ClipBox; glyph 3: no ClipBox.
//! All three must cover the same pixels (within the existing 2/255 reference tolerance).

use std::sync::Arc;

use base64::Engine as _;
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::color::palette::css::WHITE;
use vello_cpu::kurbo::Affine;
use vello_cpu::peniko::{Blob, FontData};
use vello_cpu::{Glyph, Pixmap, RasterizerSettings, RenderContext, RenderSettings, Resources, TargetInit};

// Tiny synthetic test fixture supplied inline by upstream issue #1983.
const TEST_COLR_FONT_BASE64: &str = "AAEAAAAMAIAAAwBAQ09MUhJoDj0AAAJcAAAAUkNQQUzzADTBAAACsAAAABJPUy8yJZgkDQAAATAAAABOY21hcAAM4FUAAAGQAAAANGdseWYF7Y7PAAAB0AAAABpoZWFkYvpExQAAAMwAAAA2aGhlYQPpA+wAAAEEAAAAJGhtdHgPoABQAAABgAAAABBsb2NhABoADQAAAcQAAAAKbWF4cAAEUAAAAAEoAAAABm5hbWUCvxLMAAAB7AAAAE5wb3N0AAEAAAAAAjwAAAAgAAEAAAAAAADKPgVzXw889QAAA+gAAAAAAAAAAAAAAAAAAAAAAAAAAAPoA+gAAAAAAAIAAAAAAAAAAQAAA+gAAAAAA+gAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAQAAFAAAAQAAAAAAAABkAAFAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgICAgAAAAAAAAA+gAAAAAA+gAAAAAA+gAAAPoAFAD6AAAA+gAAAAAAAIAAAADAAAAFAADAAEAAAAUAAQAIAAAAAQABAABAADgAv//AADgAP//IAEAAQAAAAAAAAAAAA0ADQANAAAAAQBQAFADmAOYAAIAADclAVADSP3kUHgC0AAAAAAABAA2AAMAAQQJAAEACgAAAAMAAQQJAAIADgAKAAMAAQQJAAQACgAAAAMAAQQJAAYACgAAAFIAZQBwAHIAbwBSAGUAZwB1AGwAYQByAAAAAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAAAAAAAAAAAAAAAAAAAIgAAAAAAAAA9AAAAAAAAAAAAAAACAAIAAAAQAAMAAAAQCgAABgABAgAAQAABAAAAAQACAAIAAAwBAAAAAAPoA+gAAAAAAAEAAQABAAAADgAANLHy/wAA";

fn glyph(font: &FontData, id: u32, size: u16, workers: u16, atlas_cache: bool) -> Vec<u8> {
    let settings = RenderSettings { num_threads: workers, ..RenderSettings::default() };
    let mut resources = Resources::default();
    let use_atlas = atlas_cache && size <= 128;
    // First draw fills the atlas; second draws the same glyph again with
    // identical retained Resources and must exercise the cache-hit path.
    let passes = if use_atlas { 2 } else { 1 };
    let mut first: Option<Vec<u8>> = None;
    let mut output = Vec::new();
    for pass in 0..passes {
        let mut ctx = RenderContext::new_with(size, size, settings);
        if id == 1 {
            ctx.set_paint(AlphaColor::<Srgb>::from_rgba8(242, 177, 52, 255));
        }
        ctx.set_transform(Affine::translate((0.0, f64::from(size))));
        ctx.glyph_run(&mut resources, font)
            .font_size(f32::from(size))
            .atlas_cache(atlas_cache)
            .hint(false)
            .fill_glyphs(std::iter::once(Glyph { id, x: 0.0, y: 0.0 }))
            .expect("draw synthetic COLR glyph");
        ctx.flush();
        let mut pixmap = Pixmap::new(size, size);
        ctx.render_with(
            &mut pixmap,
            &mut resources,
            RasterizerSettings { target_init: TargetInit::Clear(WHITE), ..RasterizerSettings::default() },
        );
        let pixels = pixmap.data_as_u8_slice().to_vec();
        if pass == 0 {
            first = Some(pixels.clone());
        } else {
            assert_eq!(pixels, first.as_ref().unwrap().as_slice(),
                "atlas cache hit must match miss exactly: gid={id}, size={size}, workers={workers}");
        }
        output = pixels;
    }
    if use_atlas {
        assert!(resources.glyph_atlas_summary().is_some_and(|(_, _, glyphs)| glyphs > 0),
            "atlas_cache=true should actually populate the glyph atlas for gid={id} size={size}");
    }
    output
}

fn compare_pixels(actual: &[u8], reference: &[u8], glyph: u32, size: u16, workers: u16, atlas_cache: bool) {
    assert_eq!(actual.len(), reference.len());
    let mut different_pixels = 0usize;
    let mut max_channel_difference = 0u8;
    for (i, (a, b)) in actual.chunks_exact(4).zip(reference.chunks_exact(4)).enumerate() {
        let d = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        if d > 2 {
            different_pixels += 1;
            if different_pixels <= 3 {
                eprintln!("COLR edge deviation size={size} workers={workers} atlas_cache={atlas_cache} glyph={glyph} x={} y={} max_channel_delta={d}", i % usize::from(size), i / usize::from(size));
            }
        }
        max_channel_difference = max_channel_difference.max(d);
    }
    assert_eq!(different_pixels, 0,
        "Vello #1983: glyph={glyph}, size={size}, workers={workers}, atlas_cache={atlas_cache}: {different_pixels} pixels deviate >2/255 from the identical outline; worst={max_channel_difference}");
}

#[test]
fn colr_without_clipbox_matches_identical_outline() {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(TEST_COLR_FONT_BASE64).expect("decode upstream synthetic font");
    let font = FontData::new(Blob::new(Arc::new(bytes)), 0);
    // Production uses glifo's atlas cache. Check both production-cached and
    // reference-uncached paths, not merely successful calls to draw a glyph.
    for atlas_cache in [false, true] {
        for workers in [0, 2] {
            for size in [20u16, 80u16, 177u16] {
                let outline = glyph(&font, 1, size, workers, atlas_cache);
                let with_clip_box = glyph(&font, 2, size, workers, atlas_cache);
                let without_clip_box = glyph(&font, 3, size, workers, atlas_cache);
                compare_pixels(&with_clip_box, &outline, 2, size, workers, atlas_cache);
                compare_pixels(&without_clip_box, &outline, 3, size, workers, atlas_cache);
            }
        }
    }
}

/// Render one glyph at a *noninteger pen position*. Unlike the integral
/// ClipBox regression above, these resources are supplied by the caller:
/// one resource set is shared across phases to exercise cross-phase cache hits.
fn positioned_glyph(
    font: &FontData,
    glyph_id: u32,
    size: u16,
    workers: u16,
    phase_x: f32,
    phase_y: f32,
    atlas_cache: bool,
    resources: &mut Resources,
) -> Vec<u8> {
    let canvas = size + 32;
    let settings = RenderSettings { num_threads: workers, ..RenderSettings::default() };
    let mut ctx = RenderContext::new_with(canvas, canvas, settings);
    if glyph_id == 1 {
        ctx.set_paint(AlphaColor::<Srgb>::from_rgba8(242, 177, 52, 255));
    }
    // Constant integral scene origin, fractional *glyph* pen coordinates:
    // this varies DrawProps::positioned_transform, rather than pixel copying.
    ctx.set_transform(Affine::translate((0.0, f64::from(size) + 8.0)));
    ctx.glyph_run(resources, font)
        .font_size(f32::from(size))
        .atlas_cache(atlas_cache)
        .hint(false)
        .fill_glyphs(std::iter::once(Glyph {
            id: glyph_id,
            x: 8.0 + phase_x,
            y: phase_y,
        }))
        .expect("render synthetic fractional COLRv1 glyph");
    ctx.flush();
    let mut pixmap = Pixmap::new(canvas, canvas);
    ctx.render_with(
        &mut pixmap,
        resources,
        RasterizerSettings {
            target_init: TargetInit::Clear(WHITE),
            ..RasterizerSettings::default()
        },
    );
    pixmap.data_as_u8_slice().to_vec()
}

fn fractional_pixel_difference(a: &[u8], b: &[u8]) -> (usize, u8) {
    assert_eq!(a.len(), b.len(), "comparison must use the same framebuffer size");
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .map(|(a, b)| a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap())
        .fold((0, 0), |(count, worst), difference| {
            (count + usize::from(difference > 2), worst.max(difference))
        })
}

/// Falsifier for the remaining production risk after Akula PR #289:
///
/// A COLR atlas key uses a constant SUBPIXEL_COLR, while a real text cursor
/// may position the same color glyph at different fractional x/y pen offsets.
/// Test true cold/cache-hit rendering against the uncached reference, for all
/// four quarter-pixel phases in both axes. Do not substitute a pixel-shifted
/// integer render for an actual fractional glyph pen coordinate.
#[test]
fn colr_fractional_pen_atlas_matches_uncached() {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(TEST_COLR_FONT_BASE64).expect("decode upstream synthetic font");
    let font = FontData::new(Blob::new(Arc::new(bytes)), 0);
    const PHASES: [f32; 4] = [0.0, 0.25, 0.5, 0.75];
    let mut mismatches = Vec::new();
    let mut examined = 0usize;
    for workers in [0u16, 2] {
        for size in [20u16, 80u16] {
            for glyph_id in [2u32, 3] {
                let mut cached_resources = Resources::default();
                let mut integer_phase_atlas_entries: Option<usize> = None;
                for phase_x in PHASES {
                    for phase_y in PHASES {
                        let mut direct_resources = Resources::default();
                        let direct = positioned_glyph(
                            &font, glyph_id, size, workers,
                            phase_x, phase_y, false, &mut direct_resources,
                        );
                        let first = positioned_glyph(
                            &font, glyph_id, size, workers,
                            phase_x, phase_y, true, &mut cached_resources,
                        );
                        let first_stats = cached_resources.glyph_atlas_summary()
                            .expect("atlas must be initialized for cached COLR");
                        assert!(first_stats.2 > 0, "pixel-aligned COLR should have an atlas entry");
                        if phase_x == 0.0 && phase_y == 0.0 {
                            integer_phase_atlas_entries = Some(first_stats.2);
                        } else {
                            assert_eq!(
                                Some(first_stats.2), integer_phase_atlas_entries,
                                "fractional COLR must not allocate/cache an unkeyed phase"
                            );
                        }
                        let second = positioned_glyph(
                            &font, glyph_id, size, workers,
                            phase_x, phase_y, true, &mut cached_resources,
                        );
                        let second_stats = cached_resources.glyph_atlas_summary()
                            .expect("atlas still initialized");
                        assert_eq!(first_stats.2, second_stats.2,
                            "repeating a pen phase must not allocate more atlas slots");
                        // For integral phase this is the cache miss/hit pair;
                        // at fractional phase both calls intentionally bypass
                        // the atlas because the key has no X/Y phase slots.
                        let hit_parity = fractional_pixel_difference(&first, &second);
                        let direct_parity = fractional_pixel_difference(&direct, &second);
                        examined += 1;
                        if hit_parity.0 > 0 || direct_parity.0 > 0 {
                            if mismatches.len() < 24 {
                                eprintln!(
                                    "COLR_FRACTIONAL_MISMATCH gid={glyph_id} size={size} \
                                     workers={workers} x={phase_x:.2} y={phase_y:.2} \
                                     first_vs_hit_pixels={} first_vs_hit_worst={} \
                                     direct_vs_hit_pixels={} direct_vs_hit_worst={}",
                                    hit_parity.0, hit_parity.1,
                                    direct_parity.0, direct_parity.1,
                                );
                            }
                            mismatches.push((glyph_id, size, workers, phase_x, phase_y,
                                hit_parity, direct_parity));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(examined, 2 * 2 * 2 * 4 * 4, "must exercise entire phase matrix");
    assert!(
        mismatches.is_empty(),
        "COLR cached vs uncached fractional pen mismatch in {} / {examined} cases \
         (channel tolerance 2/255); first examples above",
        mismatches.len(),
    );
}
