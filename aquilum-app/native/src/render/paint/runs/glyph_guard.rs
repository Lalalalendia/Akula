//! Preflight untrusted glyph geometry before Vello atlas allocation or outline flattening.
//! Reject unsafe runs rather than approximating or silently dropping visible text.
use masonry::imaging::{GlyphRunRef, record::Glyph as SceneGlyph};
use masonry::kurbo::{Affine, Point, Rect};
use masonry::peniko::Style;
use vello_cpu::Glyph;
use super::super::PaintError;

const MAX_DEVICE_COORD: f64 = 10_000_000.0;
const MAX_FONT_SIZE: f32 = 16_384.0;
const MAX_GLYPHS_PER_RUN: usize = 262_144;

fn invalid() -> PaintError {
    PaintError::GeometryBudget("нечисловая, чрезмерная или слишком длинная геометрия текста")
}
fn valid_affine(matrix: Affine) -> bool {
    matrix.as_coeffs().iter().all(|v| v.is_finite() && v.abs() <= MAX_DEVICE_COORD)
}
fn linear_max(matrix: Affine) -> f64 {
    let [a,b,c,d,_,_] = matrix.as_coeffs();
    a.abs().max(b.abs()).max(c.abs()).max(d.abs())
}
pub(super) fn validate_run(run: &GlyphRunRef<'_>) -> Result<(), PaintError> {
    if !run.font_size.is_finite() || !(0.0..=MAX_FONT_SIZE).contains(&run.font_size)
        || !valid_affine(run.transform)
        || run.glyph_transform.is_some_and(|v| !valid_affine(v))
        || run.brush_transform.is_some_and(|v| !valid_affine(v))
        || !run.font_embolden.x.is_finite() || !run.font_embolden.y.is_finite()
        || run.font_embolden.x.abs() > MAX_DEVICE_COORD
        || run.font_embolden.y.abs() > MAX_DEVICE_COORD
    {
        return Err(invalid());
    }
    let font_scale = f64::from(run.font_size) * linear_max(run.transform)
        * run.glyph_transform.map_or(1.0, linear_max);
    if !font_scale.is_finite() || font_scale > MAX_DEVICE_COORD {
        return Err(invalid());
    }
    if let Style::Stroke(stroke) = run.style {
        super::super::stroke_style::validate(stroke)?;
    }
    Ok(())
}
pub(super) fn validate_glyph(transform: Affine, glyph: &SceneGlyph) -> Result<(), PaintError> {
    if !glyph.x.is_finite() || !glyph.y.is_finite() { return Err(invalid()); }
    let p = transform * Point::new(f64::from(glyph.x), f64::from(glyph.y));
    if !p.x.is_finite() || !p.y.is_finite()
        || p.x.abs() > MAX_DEVICE_COORD || p.y.abs() > MAX_DEVICE_COORD {
        return Err(invalid());
    }
    Ok(())
}
pub(super) fn collect(
    source: impl Iterator<Item = SceneGlyph>, transform: Affine, area: Rect, filtered: bool,
) -> Result<Vec<Glyph>, PaintError> {
    let mut accepted = Vec::new();
    for (idx, g) in source.enumerate() {
        if idx >= MAX_GLYPHS_PER_RUN { return Err(invalid()); }
        validate_glyph(transform, &g)?;
        if filtered || area.contains(transform * Point::new(f64::from(g.x), f64::from(g.y))) {
            accepted.push(Glyph { id: g.id, x: g.x, y: g.y });
        }
    }
    Ok(accepted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::{settings, Painter};
    use masonry::imaging::{GlyphRunRef, PaintSink};
    use masonry::peniko::{Blob, BrushRef, Color, Fill, FontData, Style};
    use std::sync::Arc;

    fn font() -> FontData {
        FontData::new(Blob::new(Arc::new(vec![0_u8; 4])), 0)
    }
    fn glyph(x: f32, y: f32) -> SceneGlyph {
        SceneGlyph { id: 1, x, y }
    }
    #[test]
    fn vello_glyph_guard_nonfinite_run_metadata_is_rejected_before_font_access() {
        let f = font();
        let style = Style::Fill(Fill::NonZero);
        let mut run = GlyphRunRef::new(&f, &style, BrushRef::Solid(Color::WHITE));
        assert!(validate_run(&run).is_ok());
        for size in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 20_000.0] {
            run.font_size = size;
            assert!(validate_run(&run).is_err(), "unsafe font size {size:?}");
        }
        run.font_size = 16.0;
        run.transform = Affine::scale(f64::INFINITY);
        assert!(validate_run(&run).is_err());
        run.transform = Affine::IDENTITY;
        run.glyph_transform = Some(Affine::scale(1e20));
        assert!(validate_run(&run).is_err());
        run.glyph_transform = None;
        run.font_embolden.x = f64::NAN;
        assert!(validate_run(&run).is_err());
    }
    #[test]
    fn vello_glyph_guard_nonfinite_positions_fail_through_real_paint_sink() {
        let f = font();
        let style = Style::Fill(Fill::NonZero);
        let run = GlyphRunRef::new(&f, &style, BrushRef::Solid(Color::WHITE));
        for g in [glyph(f32::NAN, 2.0), glyph(f32::INFINITY, 3.0), glyph(3.0, f32::NEG_INFINITY), glyph(20_000_000.0, 0.0)] {
            let mut painter = Painter::new(64, 64, settings(0));
            painter.begin(0, 0, 64, 64);
            painter.glyph_run(run.clone(), &mut core::iter::once(g));
            assert!(matches!(painter.finish(), Err(PaintError::GeometryBudget(_))),
                "unsafe glyph {g:?} reached Vello");
        }
    }
    #[test]
    fn vello_glyph_guard_large_run_is_rejected_before_atlas_allocation() {
        let f = font();
        let style = Style::Fill(Fill::NonZero);
        let run = GlyphRunRef::new(&f, &style, BrushRef::Solid(Color::WHITE));
        let mut painter = Painter::new(64, 64, settings(0));
        painter.begin(0, 0, 64, 64);
        let mut inputs = core::iter::repeat(glyph(8.0, 8.0)).take(MAX_GLYPHS_PER_RUN + 1);
        painter.glyph_run(run, &mut inputs);
        assert!(matches!(painter.finish(), Err(PaintError::GeometryBudget(_))));
    }
    #[test]
    fn vello_glyph_guard_valid_font_geometry_keeps_original_filtering_order() {
        let f = font();
        let style = Style::Fill(Fill::NonZero);
        let run = GlyphRunRef::new(&f, &style, BrushRef::Solid(Color::WHITE));
        assert!(validate_run(&run).is_ok());
        let allowed = collect([glyph(10.0, 10.0), glyph(80.0, 80.0), glyph(20.0, 20.0)].into_iter(),
            Affine::IDENTITY, Rect::new(0.0, 0.0, 64.0, 64.0), false).unwrap();
        assert_eq!(allowed.iter().map(|g| g.id).collect::<Vec<_>>(), [1,1]);
        assert_eq!(allowed.iter().map(|g| (g.x,g.y)).collect::<Vec<_>>(), [(10.0,10.0),(20.0,20.0)]);
        let included = collect([glyph(10.0, 10.0), glyph(80.0, 80.0)].into_iter(),
            Affine::IDENTITY, Rect::new(0.0, 0.0, 64.0, 64.0), true).unwrap();
        assert_eq!(included.len(), 2);
    }
}
