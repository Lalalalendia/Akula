//! Reject malformed gradient coordinates/stops before Vello color-ramp interpolation.
//! Keep legal negative radial radii and non-[0,1] stop offsets unchanged.
use masonry::kurbo::Point;
use masonry::peniko::{Gradient, GradientKind};
use super::super::PaintError;

const MAX_GRADIENT_COORD: f64 = 10_000_000.0;
const MAX_GRADIENT_STOPS: usize = 8192;
const MAX_COLOR_COMPONENT: f32 = 1_000_000.0;

fn invalid() -> PaintError {
    PaintError::GeometryBudget("нечисловые или чрезмерные параметры градиента")
}
fn valid_point(point: Point) -> bool {
    point.x.is_finite() && point.y.is_finite()
        && point.x.abs() <= MAX_GRADIENT_COORD && point.y.abs() <= MAX_GRADIENT_COORD
}
fn valid_scalar(x: f32) -> bool {
    x.is_finite() && f64::from(x).abs() <= MAX_GRADIENT_COORD
}

pub(super) fn validate(gradient: &Gradient) -> Result<(), PaintError> {
    let valid_kind = match gradient.kind {
        GradientKind::Linear(g) => valid_point(g.start) && valid_point(g.end),
        GradientKind::Radial(g) => {
            valid_point(g.start_center) && valid_point(g.end_center)
                && valid_scalar(g.start_radius) && valid_scalar(g.end_radius)
        }
        GradientKind::Sweep(g) => {
            valid_point(g.center) && valid_scalar(g.start_angle) && valid_scalar(g.end_angle)
        }
    };
    let valid_stops = gradient.stops.len() <= MAX_GRADIENT_STOPS
        && gradient.stops.iter().all(|stop| {
            valid_scalar(stop.offset)
                && stop.color.components.iter().all(|c| c.is_finite() && c.abs() <= MAX_COLOR_COMPONENT)
        });
    if valid_kind && valid_stops { Ok(()) } else { Err(invalid()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonry::imaging::record::{replay_transformed, Scene};
    use masonry::kurbo::{Affine, Rect};
    use masonry::peniko::{BrushRef, Color, ColorStop};
    use crate::render::paint::{Painter, settings};
    use vello_cpu::Pixmap;

    fn ordinary() -> Gradient {
        Gradient::new_linear((0.0, 0.0), (32.0, 0.0))
            .with_stops([(0.0, Color::WHITE), (1.0, Color::BLACK)])
    }
    fn real_sink(gradient: &Gradient) -> Result<Pixmap, PaintError> {
        let mut scene = Scene::new();
        masonry::imaging::Painter::new(&mut scene).fill_rect(
            Rect::new(4.0, 4.0, 28.0, 28.0), gradient,
        );
        let mut painter = Painter::new(32, 32, settings(0));
        painter.begin(0, 0, 32, 32);
        replay_transformed(&scene, &mut painter, Affine::IDENTITY);
        painter.finish()?;
        let mut pixels = Pixmap::new(32, 32);
        painter.ctx.render(&mut pixels, &mut painter.resources);
        Ok(pixels)
    }
    #[test]
    fn gradient_input_guard_rejects_nonfinite_stops_before_raster() {
        let mut g = ordinary();
        g.stops[0].offset = f32::NAN;
        assert!(validate(&g).is_err());
        assert!(matches!(real_sink(&g), Err(PaintError::GeometryBudget(_))));
        let mut g = ordinary();
        g.stops[1].color.components[3] = f32::INFINITY;
        assert!(validate(&g).is_err());
        assert!(matches!(real_sink(&g), Err(PaintError::GeometryBudget(_))));
        assert!(super::super::validate_draw(&BrushRef::Gradient(&g), 1.0).is_err());
    }
    #[test]
    fn gradient_input_guard_rejects_nonfinite_linear_radial_sweep_geometry() {
        let mut linear = ordinary();
        linear.kind = GradientKind::Linear(masonry::peniko::LinearGradientPosition {
            start: Point::new(f64::INFINITY, 0.0),
            end: Point::new(32.0, 0.0),
        });
        assert!(validate(&linear).is_err());
        let radial = Gradient::new_radial((2.0, 2.0), f32::INFINITY);
        assert!(validate(&radial).is_err());
        let sweep = Gradient::new_sweep((3.0, 3.0), 0.0, f32::NAN);
        assert!(validate(&sweep).is_err());
        let negative_radius = Gradient::new_radial((3.0, 3.0), -2.0);
        assert!(validate(&negative_radius).is_ok(),
            "negative COLRv1-style radial tips remain legal");
    }
    #[test]
    fn gradient_input_guard_limits_stops_without_silent_truncation() {
        let mut g = ordinary();
        let stop = ColorStop::from((0.5, Color::WHITE));
        while g.stops.len() <= MAX_GRADIENT_STOPS { g.stops.push(stop); }
        assert!(validate(&g).is_err());
        assert!(matches!(real_sink(&g), Err(PaintError::GeometryBudget(_))));
    }
    #[test]
    fn gradient_input_guard_normal_pixels_and_extended_offsets_remain_unchanged() {
        let g = ordinary();
        assert!(validate(&g).is_ok());
        let pixels = real_sink(&g).expect("ordinary linear ramp must render");
        assert!(pixels.sample(5, 16).a > 0);
        assert_eq!(pixels.sample(1, 1).a, 0);
        let mut extended = g.clone();
        extended.stops[0].offset = -0.5;
        extended.stops[1].offset = 1.5;
        assert!(validate(&extended).is_ok(), "valid extended gradient must be accepted");
    }
}
