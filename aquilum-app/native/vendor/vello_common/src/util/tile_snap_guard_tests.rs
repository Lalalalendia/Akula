//! Vello #2005: snapping viewport edges at u16::MAX does not overflow.
use super::{RectExt, RectU16};
use crate::tile::Tile;

#[test]
fn vello_viewport_edge_snap_max_width_height_no_panic() {
    let w = RectU16::new(0, 0, u16::MAX, 1).snap_to_tile_coordinates();
    assert_eq!(w, RectU16::new(0, 0, u16::MAX, Tile::HEIGHT));
    let h = RectU16::new(0, 0, 1, u16::MAX).snap_to_tile_coordinates();
    assert_eq!(h, RectU16::new(0, 0, Tile::WIDTH, u16::MAX));
    let both = RectU16::new(0, 0, u16::MAX, u16::MAX).snap_to_tile_coordinates();
    assert_eq!(both, RectU16::new(0, 0, u16::MAX, u16::MAX));
}

#[test]
fn vello_viewport_edge_snap_last_partial_tile_saturates() {
    let rect = RectU16::new(u16::MAX - 5, u16::MAX - 5, u16::MAX, u16::MAX);
    let result = rect.snap_to_tile_coordinates();
    assert_eq!(result.x0, ((u16::MAX - 5) / Tile::WIDTH) * Tile::WIDTH);
    assert_eq!(result.y0, ((u16::MAX - 5) / Tile::HEIGHT) * Tile::HEIGHT);
    assert_eq!((result.x1, result.y1), (u16::MAX, u16::MAX));
    assert!(result.width() >= rect.width() && result.height() >= rect.height());
}

#[test]
fn vello_viewport_edge_snap_normal_and_empty_rectangles_unchanged() {
    for rect in [
        RectU16::new(0, 0, 3840, 2160),
        RectU16::new(0, 0, 7680, 4320),
        RectU16::new(16, 20, 32, 36),
    ] {
        assert_eq!(rect.snap_to_tile_coordinates(), rect);
    }
    assert_eq!(RectU16::new(5, 7, 5, 7).snap_to_tile_coordinates(),
        RectU16::new(4, 4, 4, 4));
}
