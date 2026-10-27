//! Validate untrusted Bézier paths before Vello SIMD flattening.
//! Reject extreme coordinates and malformed path sequences before rasterization.

use masonry::imaging::{ClipRef, GeometryRef};
use masonry::kurbo::{Affine, BezPath, PathEl, Point, Stroke};

use super::{Painter, PaintError};
mod sequence;

const MAX_DEVICE_COORD: f64 = 10_000_000.0;
const MAX_PATH_ELEMENTS: usize = 262_144;

fn invalid() -> PaintError {
    PaintError::GeometryBudget("нечисловые или экстремальные координаты пути")
}

fn valid_point(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
        && p.x.abs() <= MAX_DEVICE_COORD && p.y.abs() <= MAX_DEVICE_COORD
}

fn guard_path(path: &BezPath, transform: Affine) -> Result<(), PaintError> {
    if !sequence::within_budget(path.elements(), MAX_PATH_ELEMENTS) { return Err(invalid()); }
    for el in path.elements() {
        let ok = match transform * *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => valid_point(p),
            PathEl::QuadTo(a, b) => valid_point(a) && valid_point(b),
            PathEl::CurveTo(a, b, c) => valid_point(a) && valid_point(b) && valid_point(c),
            PathEl::ClosePath => true,
        };
        if !ok { return Err(invalid()); }
    }
    Ok(())
}

fn guard_geometry(shape: &GeometryRef<'_>, transform: Affine) -> Result<(), PaintError> {
    if !transform.as_coeffs().iter().all(|x| x.is_finite()) { return Err(invalid()); }
    match shape {
        GeometryRef::Path(path) => guard_path(path, transform),
        GeometryRef::OwnedPath(path) => guard_path(path, transform),
        GeometryRef::Rect(r) => guard_rect(*r, transform),
        GeometryRef::RoundedRect(r) => guard_rect(r.rect(), transform),
    }
}

fn guard_rect(rect: masonry::kurbo::Rect, transform: Affine) -> Result<(), PaintError> {
    for p in [
        Point::new(rect.x0, rect.y0), Point::new(rect.x0, rect.y1),
        Point::new(rect.x1, rect.y0), Point::new(rect.x1, rect.y1),
    ] {
        if !valid_point(transform * p) { return Err(invalid()); }
    }
    Ok(())
}

impl Painter {
    pub(super) fn guard_geometry(&self, shape: &GeometryRef<'_>, transform: Affine) -> Result<(), PaintError> {
        guard_geometry(shape, transform)
    }

    pub(super) fn guard_stroke(
        &self, shape: &GeometryRef<'_>, transform: Affine, style: &Stroke,
    ) -> Result<(), PaintError> {
        super::super::stroke_style::validate(style)?;
        guard_geometry(shape, transform)
    }

    pub(super) fn guard_clip(&self, clip: &ClipRef<'_>) -> Result<(), PaintError> {
        match clip {
            ClipRef::Fill { transform, shape, .. } => guard_geometry(shape, *transform),
            ClipRef::Stroke { transform, shape, stroke } => self.guard_stroke(shape, *transform, stroke),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonry::imaging::PaintSink;
    use masonry::peniko::Fill;
    use vello_cpu::color::palette::css::RED;
    use vello_cpu::{Pixmap, RasterizerSettings};

    fn pathological_cubic() -> BezPath {
        let mut path = BezPath::new();
        path.move_to((-2.1351245891339424e19, -3.976870325297853e18));
        path.curve_to(
            (-2.1351245891339424e19, -3.976870325297879e18),
            (-2.1350093033635213e19, -3.9766555947095035e1),
            (-2.1348670911586615e19, -3.9763907111049165e1),
        );
        path.close_path();
        path
    }

    #[test]
    fn extreme_path_geometry_rejects_exact_upstream_1277_before_vello() {
        let path = pathological_cubic();
        assert!(matches!(guard_geometry(&GeometryRef::Path(&path), Affine::IDENTITY),
            Err(PaintError::GeometryBudget(_))));
        let mut painter = Painter::new(64, 64, crate::render::paint::settings(0));
        painter.begin(0, 0, 64, 64);
        painter.push_clip(ClipRef::Fill {
            shape: GeometryRef::Path(&path), transform: Affine::IDENTITY, fill_rule: Fill::NonZero,
        });
        assert!(matches!(painter.finish(), Err(PaintError::GeometryBudget(_))),
            "actual clip sink must report the unsupported path");
    }

    #[test]
    fn extreme_path_geometry_crossing_cubic_and_nonfinite_transform_are_rejected() {
        let mut path = BezPath::new();
        path.move_to((-2.1351245891339424e19, -3.976870325297853e18));
        path.curve_to((20.0, 20.0), (40.0, 40.0), (45.0, 45.0));
        assert!(guard_path(&path, Affine::IDENTITY).is_err());
        let mut ordinary = BezPath::new();
        ordinary.move_to((10.0, 10.0));
        ordinary.curve_to((20.0, 50.0), (40.0, 50.0), (50.0, 10.0));
        assert!(guard_path(&ordinary, Affine::IDENTITY).is_ok());
        assert!(guard_geometry(&GeometryRef::Path(&ordinary), Affine::scale(1e15)).is_err());
        assert!(guard_geometry(&GeometryRef::Path(&ordinary),
            Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0])).is_err());
        assert!(guard_path(&ordinary, Affine::translate((5.0, 5.0))).is_ok());
    }

    #[test]
    fn extreme_path_geometry_regular_clip_raster_remains_unchanged() {
        let mut path = BezPath::new();
        path.move_to((10.0, 10.0));
        path.line_to((50.0, 10.0));
        path.curve_to((50.0, 22.0), (50.0, 38.0), (50.0, 50.0));
        path.line_to((10.0, 50.0));
        path.close_path();
        let mut painter = Painter::new(64, 64, crate::render::paint::settings(0));
        painter.begin(0, 0, 64, 64);
        painter.push_clip(ClipRef::Fill {
            shape: GeometryRef::Path(&path), transform: Affine::IDENTITY, fill_rule: Fill::NonZero,
        });
        painter.ctx.set_paint(RED);
        painter.ctx.fill_rect(&masonry::kurbo::Rect::new(0.0, 0.0, 64.0, 64.0));
        painter.pop_clip();
        painter.finish().expect("normal clipping must remain supported");
        let mut pixels = Pixmap::new(64, 64);
        painter.ctx.render_with(&mut pixels, &mut painter.resources, RasterizerSettings::default());
        assert!(pixels.sample(25, 25).r > 200);
        assert_eq!(pixels.sample(2, 2).a, 0);
    }
}
