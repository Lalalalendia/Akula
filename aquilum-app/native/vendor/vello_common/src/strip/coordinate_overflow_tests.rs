//! Regressions for Vello #2005: tile to pixel coordinate conversion overflow.
//! Match the existing saturating semantics of StripFillSegment::pixel_rect.

use super::{StripFillSegment, Tile};

#[test]
fn vello_strip_pixel_coords_u16_max_match_existing_pixel_rect_saturation() {
    // Exact upstream cases: x0(), x1(), and y() previously panicked on u16::MAX.
    for segment in [
        StripFillSegment { tile_x0: u16::MAX, tile_x1: 0, tile_y: 0 },
        StripFillSegment { tile_x0: 0, tile_x1: u16::MAX, tile_y: 0 },
        StripFillSegment { tile_x0: 0, tile_x1: 0, tile_y: u16::MAX },
        StripFillSegment { tile_x0: u16::MAX, tile_x1: u16::MAX, tile_y: u16::MAX },
    ] {
        let rect = segment.pixel_rect();
        assert_eq!(segment.x0(), rect.x0);
        assert_eq!(segment.x1(), rect.x1);
        assert_eq!(segment.y(), rect.y0);
    }
}

#[test]
fn vello_strip_pixel_coords_8k_and_max_valid_bounds_stay_exact() {
    for (x0, x1, y) in [(0, 1, 0), (1919, 1920, 1079), (2047, 2048, 2047), (16382, 16383, 16383)] {
        let segment = StripFillSegment { tile_x0: x0, tile_x1: x1, tile_y: y };
        assert_eq!(segment.x0(), x0 * Tile::WIDTH);
        assert_eq!(segment.x1(), x1 * Tile::WIDTH);
        assert_eq!(segment.y(), y * Tile::HEIGHT);
        let rect = segment.pixel_rect();
        assert_eq!(segment.x0(), rect.x0);
        assert_eq!(segment.x1(), rect.x1);
        assert_eq!(segment.y(), rect.y0);
    }
}

#[test]
fn vello_strip_pixel_coords_first_overflow_saturates_not_wraps() {
    // 16_384 * 4 is just above u16::MAX: never wrap to zero.
    let segment = StripFillSegment { tile_x0: 16_384, tile_x1: 16_385, tile_y: 16_384 };
    assert_eq!((segment.x0(), segment.x1(), segment.y()), (u16::MAX, u16::MAX, u16::MAX));
}
