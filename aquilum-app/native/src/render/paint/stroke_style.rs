//! Validate untrusted Kurbo strokes before dashing/outline expansion.
//! Invalid dash periods can cause non-finite remainders or repeated zero steps.

use masonry::kurbo::Stroke;
use super::PaintError;

const MAX_STROKE_WIDTH: f64 = 10_000_000.0;
const MAX_MITER_LIMIT: f64 = 10_000.0;
const MAX_DASH_SEGMENTS: usize = 1024;

fn invalid() -> PaintError {
    PaintError::GeometryBudget("нечисловые или некорректные интервалы штриховой линии")
}

pub(super) fn validate(style: &Stroke) -> Result<(), PaintError> {
    if !style.is_finite() || style.width < 0.0 || style.width > MAX_STROKE_WIDTH
        || style.miter_limit < 0.0 || style.miter_limit > MAX_MITER_LIMIT
        || style.dash_pattern.len() > MAX_DASH_SEGMENTS
        || style.dash_pattern.iter().any(|v| *v < 0.0)
    {
        return Err(invalid());
    }
    if !style.dash_pattern.is_empty() {
        let period: f64 = style.dash_pattern.iter().copied().sum();
        if !period.is_finite() || period <= 0.0
            || (style.dash_pattern.len() % 2 == 1 && !(period * 2.0).is_finite()) {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{Painter, settings};
    use masonry::imaging::{ClipRef, GlyphRunRef, PaintSink, StrokeRef};
    use masonry::imaging::record::Glyph as SceneGlyph;
    use masonry::kurbo::{Affine, Rect};
    use masonry::peniko::{Blob, BrushRef, Color, FontData, Style};
    use std::sync::Arc;
    use vello_cpu::Pixmap;

    #[test]
    fn stroke_dash_guard_rejects_nonfinite_negative_zero_period_and_excessive_pattern() {
        for style in [
            Stroke::new(2.0).with_dashes(f64::NAN, [2.0, 3.0]),
            Stroke::new(2.0).with_dashes(f64::INFINITY, [2.0, 3.0]),
            Stroke::new(2.0).with_dashes(0.0, [2.0, f64::NAN]),
            Stroke::new(2.0).with_dashes(0.0, [2.0, -1.0]),
            Stroke::new(2.0).with_dashes(0.0, [0.0, 0.0]),
            Stroke::new(2.0).with_dashes(0.0, [f64::MAX, f64::MAX]),
            Stroke::new(2.0).with_dashes(0.0, vec![1.0; MAX_DASH_SEGMENTS + 1]),
        ] {
            assert!(matches!(validate(&style), Err(PaintError::GeometryBudget(_))),
                "unsafe dashes were accepted: {style:?}");
        }
        assert!(validate(&Stroke::new(2.0).with_dashes(0.0, [0.0, 3.0])).is_ok(),
            "isolated zero dash lengths remain legal");
        assert!(validate(&Stroke::new(2.0).with_dashes(-17.0, [4.0, 2.0])).is_ok(),
            "negative offsets remain supported");
    }

    #[test]
    fn stroke_dash_guard_fails_stroke_and_clip_at_real_paint_sink() {
        let invalid = Stroke::new(2.0).with_dashes(f64::NAN, [4.0, 2.0]);
        for clip in [false, true] {
            let mut painter = Painter::new(64, 64, settings(0));
            painter.begin(0, 0, 64, 64);
            let bounds = Rect::new(8.0, 8.0, 56.0, 56.0);
            if clip {
                painter.push_clip(ClipRef::Stroke {
                    transform: Affine::IDENTITY,
                    shape: bounds.into(),
                    stroke: &invalid,
                });
            } else {
                painter.stroke(StrokeRef::new(bounds, &invalid, BrushRef::Solid(Color::WHITE)));
            }
            assert!(matches!(painter.finish(), Err(PaintError::GeometryBudget(_))));
        }
    }

    #[test]
    fn stroke_dash_guard_rejects_malformed_glyph_outline_before_font_parsing() {
        let font = FontData::new(Blob::new(Arc::new(vec![0_u8; 4])), 0);
        let style = Style::Stroke(Stroke::new(2.0).with_dashes(0.0, [0.0, 0.0]));
        let run = GlyphRunRef::new(&font, &style, BrushRef::Solid(Color::WHITE));
        let mut painter = Painter::new(64, 64, settings(0));
        painter.begin(0, 0, 64, 64);
        painter.glyph_run(run, &mut core::iter::once(SceneGlyph { id: 1, x: 16.0, y: 16.0 }));
        assert!(matches!(painter.finish(), Err(PaintError::GeometryBudget(_))));
    }

    #[test]
    fn stroke_dash_guard_preserves_valid_dashed_stroke_pixels() {
        let style = Stroke::new(2.0).with_dashes(-3.0, [6.0, 4.0]);
        assert!(validate(&style).is_ok());
        let mut painter = Painter::new(64, 64, settings(0));
        painter.begin(0, 0, 64, 64);
        painter.stroke(StrokeRef::new(
            Rect::new(8.0, 8.0, 56.0, 56.0), &style, BrushRef::Solid(Color::WHITE),
        ));
        painter.finish().expect("ordinary dashed strokes must be admitted");
        let mut image = Pixmap::new(64, 64);
        painter.ctx.render(&mut image, &mut painter.resources);
        assert!((8..56).any(|x| image.sample(x, 8).a > 0),
            "valid dashed outline must paint a visible segment on the top edge");
        assert_eq!(image.sample(32, 32).a, 0, "stroke interior stays transparent");
    }
}
