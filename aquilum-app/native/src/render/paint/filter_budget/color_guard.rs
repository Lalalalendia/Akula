//! Filter flood/drop-shadow colors bypass the normal paint/gradient guard.
//! Reject invalid DynamicColor channels before the Vello FilterGraph is built.

use masonry::peniko::Color;

const MAX_CHANNEL_MAGNITUDE: f32 = 1_000_000.0;

pub(super) fn valid(color: &Color) -> bool {
    color.components.iter().all(|x| x.is_finite() && x.abs() <= MAX_CHANNEL_MAGNITUDE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::{Painter, error::PaintError, settings};
    use super::super::validate;
    use masonry::imaging::Filter;
    use masonry::imaging::record::{Group, Scene, replay_transformed};
    use masonry::kurbo::{Affine, Rect};

    fn shadow(color: Color) -> Filter {
        Filter::DropShadow {
            dx: 0.0, dy: 0.0, std_deviation_x: 1.5, std_deviation_y: 1.5, color,
        }
    }
    fn render_error(filter: Filter) -> Result<(), PaintError> {
        let mut scene = Scene::new();
        scene.push_group(Group { filters: vec![filter], ..Default::default() });
        masonry::imaging::Painter::new(&mut scene)
            .fill_rect(Rect::new(3.0, 3.0, 29.0, 29.0), Color::WHITE);
        scene.pop_group();
        let mut painter = Painter::new(32, 32, settings(0));
        painter.begin(0, 0, 32, 32);
        replay_transformed(&scene, &mut painter, Affine::IDENTITY);
        painter.finish()
    }

    #[test]
    fn filter_color_guard_rejects_nan_inf_extreme_flood_and_shadow_colors() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30] {
            let mut c = Color::WHITE;
            c.components[0] = value;
            assert!(!valid(&c));
            assert!(matches!(validate(&[Filter::Flood { color: c }], Affine::IDENTITY, 32, 32),
                Err(PaintError::FilterBudget(_))));
            assert!(matches!(validate(&[shadow(c)], Affine::IDENTITY, 32, 32),
                Err(PaintError::FilterBudget(_))));
        }
    }

    #[test]
    fn filter_color_guard_real_scene_rejects_untrusted_filter_color_before_vello() {
        let mut flood = Color::WHITE;
        flood.components[3] = f32::NAN;
        let mut shadow_color = Color::WHITE;
        shadow_color.components[2] = f32::INFINITY;
        for filter in [Filter::Flood { color: flood }, shadow(shadow_color)] {
            assert!(matches!(render_error(filter), Err(PaintError::FilterBudget(_))),
                "invalid filter colors must not be passed to Vello");
        }
    }

    #[test]
    fn filter_color_guard_regular_and_extended_finite_colors_remain_accepted() {
        assert!(valid(&Color::WHITE));
        assert!(validate(&[Filter::Flood { color: Color::WHITE }],
            Affine::IDENTITY, 3840, 2160).is_ok());
        assert!(validate(&[shadow(Color::BLACK)], Affine::IDENTITY, 7680, 4320).is_ok());
        let mut extended = Color::WHITE;
        extended.components[0] = -0.5;
        extended.components[2] = 3.0;
        assert!(valid(&extended), "extended finite color channels are legal");
        assert!(render_error(Filter::Flood { color: Color::WHITE }).is_ok());
    }
}
