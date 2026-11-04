//! Validate optional fill/stroke paint-space transforms before Vello accepts them.
//! A valid geometric outline does not imply its independently transformed
//! gradient or image sampler has safe floating-point coordinates.

use masonry::kurbo::Affine;
use super::PaintError;

const MAX_PAINT_COORD: f64 = 10_000_000.0;

fn invalid() -> PaintError {
    PaintError::GeometryBudget("нечисловая или чрезмерная трансформация кисти")
}

/// Preserve absent brush transforms and normal non-singular / singular finite
/// matrices. Prevent NaN/Infinity and overflow when composing brush+geometry.
pub(super) fn validate(geometry: Affine, paint: Option<Affine>) -> Result<(), PaintError> {
    let Some(paint) = paint else { return Ok(()) };
    let combined = geometry * paint;
    let safe = paint.as_coeffs().into_iter().chain(combined.as_coeffs()).all(|v| {
        v.is_finite() && v.abs() <= MAX_PAINT_COORD
    });
    if safe { Ok(()) } else { Err(invalid()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::Painter;
    use masonry::imaging::{FillRef, StrokeRef, PaintSink};
    use masonry::kurbo::{Affine, Rect, Stroke};
    use masonry::peniko::{BrushRef, Color, Gradient};
    use vello_cpu::Pixmap;

    fn painter() -> Painter {
        let mut painter = Painter::new(48, 48, crate::render::paint::settings(0));
        painter.begin(0, 0, 48, 48);
        painter
    }

    fn invalid_matrices() -> [Affine; 4] {
        [
            Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]),
            Affine::new([1.0, 0.0, 0.0, 1.0, f64::INFINITY, 0.0]),
            Affine::scale(f64::NEG_INFINITY),
            Affine::scale(1e20),
        ]
    }

    #[test]
    fn brush_transform_guard_rejects_malformed_fill_on_real_paint_sink() {
        for matrix in invalid_matrices() {
            let mut p = painter();
            let draw = FillRef::new(
                Rect::new(4.0, 4.0, 44.0, 44.0),
                BrushRef::Solid(Color::WHITE),
            ).brush_transform(Some(matrix));
            p.fill(draw);
            assert!(matches!(p.finish(), Err(PaintError::GeometryBudget(_))),
                "invalid fill brush transform {matrix:?} reached Vello");
        }
    }

    #[test]
    fn brush_transform_guard_rejects_malformed_stroke_on_real_paint_sink() {
        let stroke = Stroke::new(2.0);
        for matrix in invalid_matrices() {
            let mut p = painter();
            let draw = StrokeRef::new(
                Rect::new(4.0, 4.0, 44.0, 44.0),
                &stroke, BrushRef::Solid(Color::WHITE),
            ).brush_transform(Some(matrix));
            p.stroke(draw);
            assert!(matches!(p.finish(), Err(PaintError::GeometryBudget(_))),
                "invalid stroke brush transform {matrix:?} reached Vello");
        }
    }

    #[test]
    fn brush_transform_guard_bounds_composition_but_accepts_8k() {
        assert!(validate(Affine::IDENTITY, None).is_ok());
        assert!(validate(Affine::scale(1e6), Some(Affine::scale(1e6))).is_err(),
            "two finite matrices can compose to an enormous paint transform");
        assert!(validate(Affine::IDENTITY,
            Some(Affine::translate((7680.0, 4320.0)))).is_ok());
        assert!(validate(Affine::IDENTITY, Some(Affine::scale(0.0))).is_ok(),
            "existing degenerate but finite brush transforms are preserved");
    }

    #[test]
    fn brush_transform_guard_normal_gradient_pixels_match_identity_reference() {
        let gradient = Gradient::new_linear((0.0, 0.0), (48.0, 0.0))
            .with_stops([(0.0, Color::WHITE), (1.0, Color::BLACK)]);
        let render = |transform: Option<Affine>| {
            let mut p = painter();
            p.fill(FillRef::new(
                Rect::new(4.0, 4.0, 44.0, 44.0),
                BrushRef::Gradient(&gradient),
            ).brush_transform(transform));
            p.finish().expect("valid gradient brush transform");
            let mut pixmap = Pixmap::new(48, 48);
            p.ctx.render(&mut pixmap, &mut p.resources);
            pixmap.data_as_u8_slice().to_vec()
        };
        assert_eq!(render(None), render(Some(Affine::IDENTITY)));
        let shifted = render(Some(Affine::translate((4.0, 0.0))));
        assert_ne!(shifted, render(None), "nontrivial valid brush transforms must remain effective");
    }
}
