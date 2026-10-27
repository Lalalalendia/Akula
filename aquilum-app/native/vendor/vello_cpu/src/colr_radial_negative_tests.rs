//! Synthetic COLRv1 font from linebender/vello#1982, rendered at 200 px/em.
//! Tests the actual glifo -> Vello CPU glyph raster path (not just stop math).
//! Expected opaque samples follow COLRv1 radius-domain rules and Chrome/Skia.

use crate::color::palette::css::WHITE;
use crate::kurbo::Affine;
use crate::peniko::{Blob, FontData};
use crate::{Glyph, Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use alloc::sync::Arc;

#[test]
fn colr_radial_negative_radius_raster_matches_spec() {
    let font_bytes = include_bytes!("colr_radial_negative_fixture.ttf");
    let font = FontData::new(Blob::new(Arc::new(font_bytes.to_vec())), 0);
    let cases: [(u32, [(&str, u16, u16, [u8; 4]); 3]); 3] = [
        (2, [
            ("center", 500, 500, [170, 0, 85, 255]),
            ("radius 200", 700, 500, [102, 0, 153, 255]),
            ("corner", 30, 30, [0, 0, 255, 255]),
        ]),
        (3, [
            ("center", 500, 500, [85, 0, 170, 255]),
            ("radius 200", 700, 500, [153, 0, 102, 255]),
            ("corner", 30, 30, [255, 0, 0, 255]),
        ]),
        (4, [
            ("radius 200", 700, 500, [170, 0, 85, 255]),
            ("radius 350", 850, 500, [85, 0, 170, 255]),
            ("corner", 30, 30, [0, 0, 255, 255]),
        ]),
    ];
    for (gid, points) in cases {
        let mut ctx = RenderContext::new(200, 200);
        let mut resources = Resources::new();
        ctx.set_transform(Affine::translate((0.0, 200.0)));
        ctx.glyph_run(&mut resources, &font)
            .font_size(200.0)
            .fill_glyphs(core::iter::once(Glyph { id: gid, x: 0.0, y: 0.0 }))
            .expect("synthetic glyph must render");
        ctx.flush();
        let mut pixmap = Pixmap::new(200, 200);
        ctx.render_with(
            &mut pixmap,
            &mut resources,
            RasterizerSettings { target_init: TargetInit::Clear(WHITE), ..Default::default() },
        );
        for (name, x, y, expected) in points {
            let pixel = pixmap.sample(x / 5, (1000 - y) / 5);
            let rgba = [pixel.r, pixel.g, pixel.b, pixel.a];
            for (channel, (&got, &want)) in rgba.iter().zip(&expected).enumerate() {
                assert!(
                    got.abs_diff(want) <= 2,
                    "gid={gid}, {name}, channel={channel}: got {rgba:?}, expected {expected:?}"
                );
            }
        }
    }
}
