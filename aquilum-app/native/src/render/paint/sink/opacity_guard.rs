//! Validate opacity before Peniko's debug-asserting multiply_alpha().

use masonry::peniko::BrushRef;
use super::PaintError;

fn invalid() -> PaintError {
    PaintError::GeometryBudget("нечисловая или отрицательная прозрачность кисти/группы")
}

/// Preserve existing legal positive >1 factors; reject only unsupported values.
pub(super) fn validate(alpha: f32) -> Result<(), PaintError> {
    if alpha.is_finite() && alpha >= 0.0 { Ok(()) } else { Err(invalid()) }
}

pub(super) fn validate_draw(brush: &BrushRef<'_>, composite_alpha: f32) -> Result<(), PaintError> {
    validate(composite_alpha)?;
    match brush {
        BrushRef::Image(img) => validate(img.sampler.alpha),
        BrushRef::Solid(color) if !color.components.iter().all(|c| c.is_finite()) => Err(invalid()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::Painter;
    use masonry::imaging::{Composite, PaintSink};
    use masonry::imaging::record::{Group, Scene, replay_transformed};
    use masonry::kurbo::{Affine, Rect};
    use masonry::peniko::{Blob, Color, ImageAlphaType, ImageBrush, ImageData, ImageFormat};
    use std::sync::Arc;
    use vello_cpu::Pixmap;

    fn scene(alpha: f32) -> Scene {
        let mut scene = Scene::new();
        scene.push_group(Group {
            composite: Composite::new(Default::default(), alpha),
            ..Default::default()
        });
        masonry::imaging::Painter::new(&mut scene).fill_rect(
            Rect::new(8.0, 8.0, 24.0, 24.0), Color::WHITE,
        );
        scene.pop_group();
        scene
    }

    fn render(scene: &Scene) -> (Result<(), PaintError>, Pixmap) {
        let mut painter = Painter::new(32, 32, crate::render::paint::settings(0));
        painter.begin(0, 0, 32, 32);
        replay_transformed(scene, &mut painter, Affine::IDENTITY);
        let result = painter.finish();
        let mut pixmap = Pixmap::new(32, 32);
        if result.is_ok() { painter.ctx.render(&mut pixmap, &mut painter.resources); }
        (result, pixmap)
    }

    #[test]
    fn opacity_guard_rejects_nonfinite_group_alpha_in_actual_sink() {
        for alpha in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.5] {
            assert!(validate(alpha).is_err());
            assert!(matches!(render(&scene(alpha)).0, Err(PaintError::GeometryBudget(_))));
        }
    }

    #[test]
    fn opacity_guard_rejects_invalid_image_sampler_before_vello_math() {
        let data = ImageData {
            data: Blob::new(Arc::new(vec![80, 40, 20, 255])),
            width: 1, height: 1,
            format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::AlphaPremultiplied,
        };
        let mut image = ImageBrush::new(data);
        for alpha in [f32::NAN, f32::INFINITY, -1.0] {
            image.sampler.alpha = alpha;
            assert!(validate_draw(&BrushRef::Image(image.as_ref()), 1.0).is_err());
        }
        image.sampler.alpha = 1.0;
        assert!(validate_draw(&BrushRef::Image(image.as_ref()), 1.0).is_ok());
        assert!(validate_draw(&BrushRef::Image(image.as_ref()), f32::NAN).is_err());
    }

    #[test]
    fn opacity_guard_valid_group_alpha_preserves_visible_pixels() {
        for alpha in [0.25, 0.5, 1.0] {
            assert!(validate(alpha).is_ok());
            let (result, pixels) = render(&scene(alpha));
            result.expect("valid group alpha");
            assert!(pixels.sample(12, 12).a > 0, "nonempty group must paint");
            assert_eq!(pixels.sample(0, 0).a, 0);
        }
        assert!(validate(2.0).is_ok());
    }
}
