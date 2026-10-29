//! Validate analytic blurred-rounded-rectangle draws before Vello encoding.
//! Vello uses inflated geometry and f32-valued blur math without input preflight.

use masonry::imaging::BlurredRoundedRect;
use masonry::kurbo::Point;

use super::PaintError;

const MAX_DEVICE_COORD: f64 = 10_000_000.0;
const MAX_SHADOW_SIGMA: f64 = 16_384.0;

fn invalid() -> PaintError {
    PaintError::FilterBudget("нечисловые или чрезмерные параметры размытой тени")
}

pub(super) fn validate(draw: &BlurredRoundedRect) -> Result<(), PaintError> {
    if !draw.radius.is_finite() || !(0.0..=MAX_DEVICE_COORD).contains(&draw.radius)
        || !draw.std_dev.is_finite() || !(0.0..=MAX_SHADOW_SIGMA).contains(&draw.std_dev)
        || !draw.composite.alpha.is_finite() || !(0.0..=1.0).contains(&draw.composite.alpha)
        || !draw.color.components.iter().all(|c| c.is_finite())
        || !draw.transform.as_coeffs().iter().all(|c| c.is_finite())
    {
        return Err(invalid());
    }
    // Rect::abs() normalizes via min/max, which can hide NaN on some inputs.
    let raw = draw.rect;
    if ![raw.x0, raw.y0, raw.x1, raw.y1].iter().all(|v| v.is_finite()) {
        return Err(invalid());
    }
    let rect = raw.abs();
    // Match the exact 2.5 sigma expansion used by Vello CPU before flattening.
    let spread = 2.5 * draw.std_dev;
    let enlarged = rect.inflate(spread, spread);
    for p in [
        Point::new(enlarged.x0, enlarged.y0),
        Point::new(enlarged.x0, enlarged.y1),
        Point::new(enlarged.x1, enlarged.y0),
        Point::new(enlarged.x1, enlarged.y1),
    ] {
        let point = draw.transform * p;
        if !point.x.is_finite() || !point.y.is_finite()
            || point.x.abs() > MAX_DEVICE_COORD || point.y.abs() > MAX_DEVICE_COORD
        {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::Painter;
    use masonry::imaging::PaintSink;
    use masonry::kurbo::{Affine, Rect};
    use masonry::peniko::Color;
    use vello_cpu::{Pixmap, RenderContext, Resources};

    fn ordinary() -> BlurredRoundedRect {
        BlurredRoundedRect {
            transform: Affine::IDENTITY,
            rect: Rect::new(12.0, 12.0, 38.0, 38.0),
            color: Color::from_rgb8(255, 0, 0),
            radius: 6.0,
            std_dev: 4.0,
            composite: Default::default(),
        }
    }

    #[test]
    fn blurred_rounded_shadow_guard_rejects_nonfinite_parameters_in_real_sink() {
        let baseline = ordinary();
        for bad in [
            BlurredRoundedRect { std_dev: f64::NAN, ..baseline },
            BlurredRoundedRect { std_dev: f64::INFINITY, ..baseline },
            BlurredRoundedRect { std_dev: -1.0, ..baseline },
            BlurredRoundedRect { std_dev: f64::MAX, ..baseline },
            BlurredRoundedRect { radius: f64::NEG_INFINITY, ..baseline },
            BlurredRoundedRect { radius: -1.0, ..baseline },
            BlurredRoundedRect { radius: f64::MAX, ..baseline },
            BlurredRoundedRect { rect: Rect::new(0.0, 0.0, f64::NAN, 10.0), ..baseline },
            BlurredRoundedRect { rect: Rect::new(f64::INFINITY, 0.0, 30.0, 10.0), ..baseline },
            BlurredRoundedRect { transform: Affine::scale(f64::INFINITY), ..baseline },
        ] {
            assert!(matches!(validate(&bad), Err(PaintError::FilterBudget(_))), "accepted {bad:?}");
            let mut painter = Painter::new(64, 64, crate::render::paint::settings(0));
            painter.begin(0, 0, 64, 64);
            painter.blurred_rounded_rect(bad);
            assert!(matches!(painter.finish(), Err(PaintError::FilterBudget(_))));
        }
    }

    #[test]
    fn blurred_rounded_shadow_guard_checks_effective_geometry_and_8k() {
        let normal = ordinary();
        assert!(validate(&normal).is_ok());
        let large = BlurredRoundedRect {
            rect: Rect::new(0.0, 0.0, 7680.0, 4320.0),
            std_dev: 120.0,
            ..normal
        };
        assert!(validate(&large).is_ok());
        assert!(validate(&BlurredRoundedRect { transform: Affine::scale(1e8), ..normal }).is_err());
        assert!(validate(&BlurredRoundedRect { std_dev: 16_385.0, ..normal }).is_err());
        assert!(validate(&BlurredRoundedRect {
            composite: masonry::imaging::Composite::new(Default::default(), f32::NAN),
            ..normal
        }).is_err());
    }

    #[test]
    fn blurred_rounded_shadow_guard_preserves_normal_pixels_exactly() {
        let normal = ordinary();
        let settings = crate::render::paint::settings(0);
        let mut painter = Painter::new(64, 64, settings);
        painter.begin(0, 0, 64, 64);
        painter.blurred_rounded_rect(normal);
        painter.finish().expect("ordinary shadow must be accepted");
        let mut guarded = Pixmap::new(64, 64);
        painter.ctx.render(&mut guarded, &mut painter.resources);

        let mut direct = RenderContext::new_with(64, 64, settings);
        direct.set_transform(normal.transform);
        direct.set_paint(normal.color.multiply_alpha(normal.composite.alpha));
        direct.set_blend_mode(normal.composite.blend);
        direct.fill_blurred_rounded_rect(&normal.rect, normal.radius as f32, normal.std_dev as f32, false);
        direct.flush();
        let mut reference = Pixmap::new(64, 64);
        direct.render(&mut reference, &mut Resources::new());
        assert_eq!(guarded.data_as_u8_slice(), reference.data_as_u8_slice());
        assert!(guarded.sample(24, 24).a > 0, "ordinary shadow must paint real pixels");
        assert_eq!(guarded.sample(0, 0).a, 0);
    }
}
