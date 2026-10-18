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
    let mut ctx = RenderContext::new_with(size, size, settings);
    let mut resources = Resources::default();
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
    pixmap.data_as_u8_slice().to_vec()
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
