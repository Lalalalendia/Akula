//! Bound filter-layer memory across nested groups and chains.
//! Reject unsafe geometry before allocation; never silently omit effects.
//! Preserve the unfiltered fast path and normal pixels.

use masonry::imaging::Filter;
use masonry::kurbo::Affine;

use super::{Painter, convert, error::PaintError};
mod nested;
mod color_guard;
pub(super) use nested::FilterBudgetStack;

/// Total permitted filtered-layer working set (~384 MiB at 4 bytes/pixel).
/// It is deliberately a cumulative budget: nested filter chains each allocate
/// an intermediate image. Window/compositor storage is accounted separately.
const FILTER_WORK_PIXELS: f64 = 96.0 * 1024.0 * 1024.0;
/// Vello's tile-alignment works in u16; staying below this value leaves room
/// for the final alignment step on all supported CPU tile widths.
const MAX_FILTER_AXIS: f64 = 65_000.0;
const TILE_ALIGNMENT_SLACK: f64 = 64.0;

impl Painter {
    pub(super) fn check_filter_budget(&mut self, filters: &[Filter]) -> Result<(), PaintError> {
        let pixels = validate(filters, *self.ctx.transform(), self.ctx.width(), self.ctx.height())?;
        self.filter_budget_stack.admit(pixels, FILTER_WORK_PIXELS)
    }
}

fn validate(filters: &[Filter], transform: Affine, width: u16, height: u16) -> Result<f64, PaintError> {
    if filters.is_empty() {
        return Ok(0.0);
    }
    let invalid = || PaintError::FilterBudget("нечисловые параметры или чрезмерный буфер");
    if !transform.as_coeffs().iter().all(|coefficient| coefficient.is_finite()) {
        return Err(invalid());
    }
    let mut w = f64::from(width);
    let mut h = f64::from(height);
    let mut total_pixels = 0.0;
    for filter in filters {
        let valid_values = match *filter {
            Filter::Flood { color } => color_guard::valid(&color),
            Filter::Offset { dx, dy } => [dx, dy].iter().all(|x| x.is_finite()),
            Filter::Blur { std_deviation_x, std_deviation_y } => {
                [std_deviation_x, std_deviation_y]
                    .iter().all(|x| x.is_finite() && *x >= 0.0)
            }
            Filter::DropShadow { dx, dy, std_deviation_x, std_deviation_y, color } => {
                [dx, dy, std_deviation_x, std_deviation_y]
                    .iter().all(|x| x.is_finite())
                    && std_deviation_x >= 0.0 && std_deviation_y >= 0.0
                    && color_guard::valid(&color)
            }
        };
        if !valid_values {
            return Err(invalid());
        }

        let converted = convert::filters_to_vello(core::slice::from_ref(filter))
            .ok_or_else(invalid)?;
        let output = converted.filter_expansion(&transform);
        let input = converted.source_expansion(&transform);
        let bounds = [
            output.x0, output.y0, output.x1, output.y1,
            input.x0, input.y0, input.x1, input.y1,
        ];
        if !bounds.iter().all(|v| v.is_finite()) {
            return Err(invalid());
        }

        // Include both output and source expansion in the layer budget.
        let left = (-output.x0).max(-input.x0).max(0.0);
        let top = (-output.y0).max(-input.y0).max(0.0);
        let right = output.x1.max(input.x1).max(0.0);
        let bottom = output.y1.max(input.y1).max(0.0);
        w += left + right + TILE_ALIGNMENT_SLACK;
        h += top + bottom + TILE_ALIGNMENT_SLACK;
        total_pixels += w * h;

        if !w.is_finite() || !h.is_finite()
            || w > MAX_FILTER_AXIS || h > MAX_FILTER_AXIS
            || total_pixels > FILTER_WORK_PIXELS
        {
            return Err(invalid());
        }
    }
    Ok(total_pixels)
}
#[cfg(test)]
mod tests {
    use super::*;
    use masonry::imaging::record::{replay_transformed, Group, Scene};
    use masonry::kurbo::Rect;
    use masonry::peniko::Color;

    fn blur(sigma: f32) -> Filter {
        Filter::Blur { std_deviation_x: sigma, std_deviation_y: sigma }
    }

    #[test]
    fn filter_geometry_budget_admits_normal_blurs_and_large_gui_frames() {
        assert!(validate(&[], Affine::IDENTITY, u16::MAX, u16::MAX).is_ok());
        let supported = [
            blur(2.0),
            Filter::Offset { dx: 14.0, dy: -8.0 },
            blur(4.0),
        ];
        assert!(validate(&supported, Affine::IDENTITY, 3840, 2160).is_ok());
        assert!(validate(&[blur(5.0)], Affine::IDENTITY, 7680, 4320).is_ok());
        assert!(validate(&[blur(10.0)], Affine::scale(2.0), 1920, 1080).is_ok());
    }

    #[test]
    fn filter_geometry_budget_rejects_nonfinite_and_extreme_values() {
        for sigma in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN, 1e20, 65_535.0] {
            assert!(validate(&[blur(sigma)], Affine::IDENTITY, 128, 128).is_err(),
                "unsafe sigma={sigma:?} was admitted");
        }
        for value in [f32::NAN, f32::INFINITY, 100_000.0] {
            assert!(validate(&[Filter::Offset { dx: value, dy: 0.0 }],
                Affine::IDENTITY, 128, 128).is_err(),
                "unsafe offset={value:?} was admitted");
        }
        assert!(validate(&[blur(-1.0)], Affine::IDENTITY, 128, 128).is_err());
        assert!(validate(&[blur(30.0)], Affine::scale(1e15), 128, 128).is_err());
        assert!(validate(&[blur(30.0)], Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]), 128, 128).is_err());
    }

    #[test]
    fn filter_geometry_budget_counts_full_chain_not_only_first_filter() {
        // A single intermediate fits; adding more layers must not silently
        // bypass the cumulative memory gate.
        let large = blur(100.0);
        assert!(validate(&[large], Affine::IDENTITY, 6000, 6000).is_ok());
        assert!(validate(&[blur(100.0), blur(100.0), blur(100.0)],
            Affine::IDENTITY, 6000, 6000).is_err());
    }

    #[test]
    fn filter_geometry_budget_returns_paint_error_without_rasterizing() {
        let mut scene = Scene::new();
        scene.push_group(Group {
            filters: vec![Filter::Offset { dx: 100_000.0, dy: 0.0 }],
            ..Default::default()
        });
        masonry::imaging::Painter::new(&mut scene)
            .fill_rect(Rect::new(10.0, 10.0, 30.0, 30.0), Color::WHITE);
        scene.pop_group();
        let mut painter = Painter::new(80, 80, super::super::settings(0));
        replay_transformed(&scene, &mut painter, Affine::IDENTITY);
        assert!(matches!(painter.finish(), Err(PaintError::FilterBudget(_))),
            "malformed geometry must fail with a typed render error");
    }
}
