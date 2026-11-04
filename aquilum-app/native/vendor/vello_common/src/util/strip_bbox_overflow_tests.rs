//! Vello #2005: strip_bbox at the final representable 4x4 tile must not panic.
use super::strip_bbox;
use crate::geometry::RectU16;
use crate::strip::Strip;
use crate::tile::Tile;

fn last_index() -> u32 { u32::from(Tile::WIDTH) * u32::from(Tile::HEIGHT) }
fn edge(x: u16, y: u16) -> [Strip; 2] {
    [Strip::new(x, y, 0, false), Strip::sentinel(y, last_index())]
}

#[test]
fn vello_strip_bbox_overflow_right_edge_saturates_to_u16_max() {
    let rect = strip_bbox(&edge(u16::MAX - 3, 0));
    assert_eq!(rect, Some(RectU16::new(u16::MAX - 3, 0, u16::MAX, Tile::HEIGHT)));
}

#[test]
fn vello_strip_bbox_overflow_bottom_edge_saturates_to_u16_max() {
    let rect = strip_bbox(&edge(0, u16::MAX - 3));
    assert_eq!(rect, Some(RectU16::new(0, u16::MAX - 3, Tile::WIDTH, u16::MAX)));
}

#[test]
fn vello_strip_bbox_overflow_both_axes_saturate_without_wrap() {
    assert_eq!(strip_bbox(&edge(u16::MAX - 3, u16::MAX - 3)),
        Some(RectU16::new(u16::MAX - 3, u16::MAX - 3, u16::MAX, u16::MAX)));
}

#[test]
fn vello_strip_bbox_overflow_normal_tiles_remain_pixel_exact() {
    assert_eq!(strip_bbox(&edge(8, 4)), Some(RectU16::new(8, 4, 12, 8)));
    assert_eq!(strip_bbox(&[]), None);
    assert_eq!(strip_bbox(&[Strip::sentinel(0, 0)]), None);
}
