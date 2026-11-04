//! Regressions: do not push an unmasked Vello layer after mask rendering fails.
//! The intermediate Painter must return a typed error and release budget instead.

use super::super::{Painter, error::PaintError, settings};
use masonry::imaging::{Filter, MaskMode};
use masonry::imaging::record::{
    AppliedMask, Group, Mask as RecordedMask, Scene, replay_transformed,
};
use masonry::kurbo::{Affine, Rect};
use masonry::peniko::Color;
use vello_cpu::Pixmap;

fn mask_scene(invalid: bool) -> Scene {
    let mut inner = Scene::new();
    let transform = if invalid { Affine::scale(f64::INFINITY) } else { Affine::IDENTITY };
    masonry::imaging::Painter::new(&mut inner)
        .fill(Rect::new(5.0, 5.0, 32.0, 32.0), Color::WHITE)
        .transform(transform)
        .draw();
    inner
}

fn parent(inner: Scene, filtered: bool) -> Scene {
    let mut outer = Scene::new();
    let mask = outer.define_mask(RecordedMask::new(MaskMode::Alpha, inner));
    outer.push_group(Group {
        mask: Some(AppliedMask::new(mask)),
        filters: if filtered { vec![Filter::Offset { dx: 0.0, dy: 0.0 }] } else { vec![] },
        ..Default::default()
    });
    masonry::imaging::Painter::new(&mut outer)
        .fill_rect(Rect::new(0.0, 0.0, 48.0, 48.0), Color::WHITE);
    outer.pop_group();
    outer
}

#[test]
fn vello_failed_mask_ingress_does_not_push_unmasked_filter_layer() {
    for filtered in [false, true] {
        let scene = parent(mask_scene(true), filtered);
        let mut painter = Painter::new(48, 48, settings(0));
        painter.begin(0, 0, 48, 48);
        replay_transformed(&scene, &mut painter, Affine::IDENTITY);
        assert!(painter.groups.is_empty(),
            "failed mask must not leave Vello layers open (filtered={filtered})");
        assert!(matches!(painter.finish(), Err(PaintError::GeometryBudget(_))),
            "a malformed nested mask must propagate its original typed error");
        painter.begin(0, 0, 48, 48);
        masonry::imaging::Painter::new(&mut painter)
            .fill_rect(Rect::new(2.0, 2.0, 24.0, 24.0), Color::WHITE);
        painter.finish().expect("following ordinary frame must recover");
    }
}

#[test]
fn vello_failed_mask_ingress_valid_mask_keeps_ordinary_coverage() {
    for filtered in [false, true] {
        let scene = parent(mask_scene(false), filtered);
        let mut painter = Painter::new(48, 48, settings(0));
        painter.begin(0, 0, 48, 48);
        replay_transformed(&scene, &mut painter, Affine::IDENTITY);
        painter.finish().expect("valid mask group must stay renderable");
        assert!(painter.groups.is_empty());
        let mut pixels = Pixmap::new(48, 48);
        painter.ctx.render(&mut pixels, &mut painter.resources);
        assert!(pixels.sample(15, 15).a > 0,
            "valid alpha mask must retain covered pixels");
        assert_eq!(pixels.sample(40, 40).a, 0,
            "outside mask must stay transparent");
    }
}
