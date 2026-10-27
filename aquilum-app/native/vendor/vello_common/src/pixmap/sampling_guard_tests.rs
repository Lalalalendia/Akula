//! Upstream #2005: invalid Pixmap samples must not panic or alias rows.
use super::{Pixmap, PremulRgba8};
fn clear() -> PremulRgba8 { PremulRgba8::from_u32(0) }

#[test]
fn vello_pixmap_oob_zero_area_samples_return_transparent() {
    for (w, h) in [(0, 0), (0, 1), (1, 0)] {
        let p = Pixmap::new(w, h);
        assert_eq!(p.sample(0, 0), clear());
        assert_eq!(p.sample_idx(0), clear());
        assert_eq!(p.sample_idx(u32::MAX), clear());
    }
}

#[test]
fn vello_pixmap_oob_bad_x_y_must_not_alias_another_row() {
    let mut p = Pixmap::new(2, 2);
    let unique = PremulRgba8::from_u32(0x11223344);
    p.set_pixel(0, 1, unique);
    assert_eq!(p.sample(0, 1), unique);
    assert_eq!(p.sample_idx(2), unique);
    assert_eq!(p.sample(2, 0), clear(), "x overflow must not alias row 1");
    assert_eq!(p.sample(0, 2), clear());
    assert_eq!(p.sample_idx(4), clear());
}

#[test]
fn vello_pixmap_oob_valid_pixels_stay_exact() {
    let mut p = Pixmap::new(3, 2);
    for (x, y, val) in [
        (0, 0, 0x10203040), (2, 0, 0x51617181),
        (0, 1, 0x21212121), (2, 1, 0xFF0000FF),
    ] {
        let color = PremulRgba8::from_u32(val);
        p.set_pixel(x, y, color);
        assert_eq!(p.sample(x, y), color);
        assert_eq!(p.sample_idx(u32::from(y) * 3 + u32::from(x)), color);
    }
    assert_eq!(p.sample(3, 1), clear());
}
