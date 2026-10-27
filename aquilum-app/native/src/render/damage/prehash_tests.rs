use masonry::imaging::Painter;
use masonry::kurbo::Rect;
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
