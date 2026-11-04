//! Upstream Vello #2005: Paint::may_have_transparency must fail closed
//! when an indexed paint does not exist, without changing valid paint flags.

use super::{EncodeExt, Gradient};
use crate::color::palette::css::BLACK;
use crate::kurbo::Affine;
use crate::paint::{IndexedPaint, Paint};
use crate::peniko::Color;
use alloc::vec;

#[test]
fn vello_indexed_paint_guard_empty_and_out_of_range_index_is_safe() {
    let indexed = Paint::Indexed(IndexedPaint::new(0));
    assert!(indexed.may_have_transparency(&[]),
        "missing paint must conservatively allow alpha blending, not panic");
    assert!(Paint::Indexed(IndexedPaint::new(100))
        .may_have_transparency(&[]));
}

#[test]
fn vello_indexed_paint_guard_existing_gradients_preserve_opacity_flags() {
    let mut paints = vec![];
    let opaque_gradient = Gradient::new_linear((0.0, 0.0), (32.0, 0.0))
        .with_stops([(0.0, Color::WHITE), (1.0, Color::BLACK)]);
    let opaque = opaque_gradient.encode_into(&mut paints, Affine::IDENTITY, None).unwrap();
    assert!(matches!(opaque, Paint::Indexed(_)), "gradient must be actually indexed");
    assert!(!opaque.may_have_transparency(&paints));
    assert!(Paint::Indexed(IndexedPaint::new(paints.len()))
        .may_have_transparency(&paints), "out-of-range indexes must fail closed");

    let transparent_gradient = Gradient::new_linear((0.0, 0.0), (32.0, 0.0))
        .with_stops([
            (0.0, Color::new([0.4, 0.2, 0.6, 0.5])),
            (1.0, Color::BLACK),
        ]);
    let translucent = transparent_gradient.encode_into(&mut paints, Affine::IDENTITY, None).unwrap();
    assert!(matches!(translucent, Paint::Indexed(_)));
    assert!(translucent.may_have_transparency(&paints));
    assert!(!opaque.may_have_transparency(&paints),
        "adding a later paint must not change previously encoded opacity");
}

#[test]
fn vello_indexed_paint_guard_solid_paint_remains_independent_of_registry() {
    let black: Paint = BLACK.into();
    assert!(!black.may_have_transparency(&[]));
    assert!(!black.may_have_transparency(&[]));
}
