//! Exact COLRv1 negative-radius raster regression (linebender/vello#1982).
//! Tiny synthetic font originally published in the upstream bug reproducer;
//! expected pixel values were cross-checked against Chromium/Skia.
use std::sync::Arc;

use vello_cpu::color::palette::css::WHITE;
use vello_cpu::kurbo::Affine;
use vello_cpu::peniko::{Blob, FontData};
use vello_cpu::{Glyph, Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};

const FONT: &[u8] = include_bytes!("fixtures/colr_radial_negative_radius.ttf");

#[test]
fn colr_radial_negative_radius_raster_matches_chrome_reference() {
    let font = FontData::new(Blob::new(Arc::new(FONT.to_vec())), 0);
    // Coordinates in the 1000-unit em square: samples are at 200 px/em.
    // A has r0 = -250; B has r1 = -250; C is the positive-radius control.
    let cases = [
        ("A: start negative", 2, [
            ("center", 500, 500, [170, 0, 85, 255]),
            ("radius 200", 700, 500, [102, 0, 153, 255]),
            ("corner", 30, 30, [0, 0, 255, 255]),
        ]),
        ("B: end negative", 3, [
            ("center", 500, 500, [85, 0, 170, 255]),
            ("radius 200", 700, 500, [153, 0, 102, 255]),
            ("corner", 30, 30, [255, 0, 0, 255]),
        ]),
        ("C: positive control", 4, [
            ("radius 200", 700, 500, [170, 0, 85, 255]),
            ("radius 350", 850, 500, [85, 0, 170, 255]),
            ("corner", 30, 30, [0, 0, 255, 255]),
        ]),
    ];
    for (label, gid, samples) in cases {
        let mut ctx = RenderContext::new(200, 200);
        let mut resources = Resources::new();
        ctx.set_transform(Affine::translate((0.0, 200.0)));
        ctx.glyph_run(&mut resources, &font)
            .font_size(200.0)
            .fill_glyphs(std::iter::once(Glyph { id: gid, x: 0.0, y: 0.0 }))
            .expect("render synthetic COLRv1 glyph");
        ctx.flush();
        let mut pixmap = Pixmap::new(200, 200);
        ctx.render_with(
            &mut pixmap,
            &mut resources,
            RasterizerSettings { target_init: TargetInit::Clear(WHITE), ..Default::default() },
        );
        let pixels = pixmap.data_as_u8_slice();
        for (point, x, y, expected) in samples {
            let idx = (((1000 - y) / 5) * 200 + x / 5) * 4;
            let actual = &pixels[idx..idx + 4];
            for (channel, (&got, &want)) in actual.iter().zip(expected.iter()).enumerate() {
                assert!(
                    (i16::from(got) - i16::from(want)).abs() <= 4,
                    "{label}, {point}, RGBA channel {channel}: got {actual:?}, expected {expected:?}"
                );
            }
        }
    }
}
