use masonry::imaging::{BlurredRoundedRect, Composite, Painter};
use masonry::kurbo::{Affine, Rect, Stroke};
use masonry::peniko::Color;

use super::Tracker;

fn scene(with_outside: bool) -> (Vec<u64>, super::Damage) {
    let mut tracker = Tracker::new();
    tracker.begin(64, 64, 1.0);
    {
        let mut painter = Painter::new(&mut tracker);
        if with_outside {
            painter.fill_rect(Rect::new(128.0, 128.0, 160.0, 160.0), Color::from_rgb8(255, 0, 0));
        }
        painter.fill_rect(Rect::new(4.0, 4.0, 20.0, 20.0), Color::from_rgb8(0, 0, 255));
    }
    let hashes = tracker.draws.iter().map(|draw| draw.hash).collect();
    let damage = tracker.damage();
    (hashes, damage)
}

#[test]
fn damage_offscreen_fill_does_not_change_visible_hashes_or_areas() {
    let (expected_hashes, expected) = scene(false);
    let (actual_hashes, actual) = scene(true);
    assert_eq!(actual_hashes, expected_hashes);
    assert_eq!(actual.areas, expected.areas);
    assert_eq!(actual.moves, expected.moves);
    assert_eq!(actual_hashes.len(), 1);
}

#[test]
fn damage_offscreen_fill_does_not_create_draws() {
    let mut tracker = Tracker::new();
    tracker.begin(64, 64, 1.0);
    Painter::new(&mut tracker).fill_rect(
        Rect::new(150.0, 150.0, 180.0, 180.0),
        Color::from_rgb8(255, 0, 0),
    );
    assert!(tracker.draws.is_empty());
    assert!(tracker.sprites.is_empty());
    assert!(tracker.live.is_empty());
}

#[test]
fn damage_clipped_stroke_does_not_change_visible_hashes_or_areas() {
    let run = |outside: bool| {
        let mut tracker = Tracker::new();
        tracker.begin(64, 64, 1.0);
        let line = Stroke::new(3.0);
        {
            let mut painter = Painter::new(&mut tracker);
            if outside {
                painter.stroke(Rect::new(150.0, 150.0, 170.0, 170.0), &line, Color::from_rgb8(255, 0, 0)).draw();
            }
            painter.stroke(Rect::new(8.0, 8.0, 16.0, 16.0), &line, Color::from_rgb8(0, 0, 255)).draw();
        }
        let hashes: Vec<_> = tracker.draws.iter().map(|draw| draw.hash).collect();
        (hashes, tracker.damage())
    };
    let (before_hashes, before) = run(false);
    let (after_hashes, after) = run(true);
    assert_eq!(after_hashes, before_hashes);
    assert_eq!(after_hashes.len(), 1);
    assert_eq!(after.areas, before.areas);
    assert_eq!(after.moves, before.moves);
}

#[test]
fn damage_blurred_shadow_is_kept_when_spread_overlaps_frame() {
    let mut tracker = Tracker::new();
    tracker.begin(64, 64, 1.0);
    let mut painter = Painter::new(&mut tracker);
    let shadow = |rect| BlurredRoundedRect {
        transform: Affine::IDENTITY,
        rect,
        color: Color::from_rgb8(100, 110, 120),
        radius: 4.0,
        std_dev: 4.0,
        composite: Composite::default(),
    };
    painter.blurred_rounded_rect(shadow(Rect::new(100.0, 100.0, 108.0, 108.0)));
    painter.blurred_rounded_rect(shadow(Rect::new(66.0, 8.0, 78.0, 24.0)));
    assert_eq!(tracker.draws.len(), 1, "shadow halo overlaps the frame despite offscreen rect");
    let bounds = tracker.draws[0].rect;
    assert!(bounds.width() > 0.0 && bounds.height() > 0.0);
}
