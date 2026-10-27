//! Preflight untrusted image metadata before Vello's ImageSource conversion.
//! Vello Common assumes source dimensions fit in u16 and asserts otherwise.

use masonry::peniko::BrushRef;
use super::PaintError;

/// Bound extra RGBA copies made when Vello converts a source image.
const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;

fn invalid() -> PaintError {
    PaintError::GeometryBudget("изображение больше допустимого размера или с повреждёнными RGBA-данными")
}

fn valid_dimensions(width: u32, height: u32) -> bool {
    width <= u32::from(u16::MAX) && height <= u32::from(u16::MAX)
        && u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS
}

pub(super) fn validate(brush: &BrushRef<'_>) -> Result<(), PaintError> {
    let BrushRef::Image(image) = brush else { return Ok(()) };
    let data = &image.image;
    // Keep the established Vello #1269 transparent 1x1 fallback for empty images.
    if data.width == 0 || data.height == 0 { return Ok(()); }
    if !valid_dimensions(data.width, data.height) {
        return Err(invalid());
    }
    let expected = u64::from(data.width) * u64::from(data.height) * 4;
    if expected != data.data.data().len() as u64 {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::Painter;
    use masonry::imaging::record::{replay_transformed, Scene};
    use masonry::kurbo::{Affine, Rect};
    use masonry::peniko::{Blob, ImageAlphaType, ImageBrush, ImageData, ImageFormat};
    use std::sync::Arc;
    use vello_cpu::Pixmap;

    fn brush(width: u32, height: u32, raw: Vec<u8>, format: ImageFormat) -> ImageBrush {
        ImageBrush::new(ImageData {
            data: Blob::new(Arc::new(raw)),
            width, height, format, alpha_type: ImageAlphaType::AlphaPremultiplied,
        })
    }

    fn real_sink(brush: ImageBrush) -> Result<(), PaintError> {
        let mut scene = Scene::new();
        masonry::imaging::Painter::new(&mut scene)
            .fill_rect(Rect::new(0.0, 0.0, 12.0, 12.0), brush.as_ref());
        let mut painter = Painter::new(32, 32, crate::render::paint::settings(0));
        painter.begin(0, 0, 32, 32);
        replay_transformed(&scene, &mut painter, Affine::IDENTITY);
        painter.finish()
    }

    #[test]
    fn invalid_image_ingress_oversized_u32_dimensions_fail_before_vello_assert() {
        for (width, height) in [(u16::MAX as u32 + 1, 1), (1, u16::MAX as u32 + 1)] {
            // Only 256 KiB of pixels: valid raw RGBA, but Vello would panic
            // when narrowing ImageData's u32 dimensions to u16.
            let raw = vec![255u8; width as usize * height as usize * 4];
            let image = brush(width, height, raw, ImageFormat::Rgba8);
            assert!(matches!(validate(&BrushRef::Image(image.as_ref())),
                Err(PaintError::GeometryBudget(_))));
            assert!(matches!(real_sink(image), Err(PaintError::GeometryBudget(_))));
        }
    }

    #[test]
    fn invalid_image_ingress_bad_length_and_excessive_area_fail_closed() {
        let wrong = brush(2, 1, vec![255; 4], ImageFormat::Rgba8);
        assert!(matches!(real_sink(wrong), Err(PaintError::GeometryBudget(_))));
        assert!(!valid_dimensions(16_384, 16_384));
        assert!(valid_dimensions(7680, 4320), "8K source must be supported");
        assert!(valid_dimensions(8192, 8192), "64 Mi-pixel bound is inclusive");
        assert!(!valid_dimensions(8193, 8192));
        assert!(valid_dimensions(1, 1));
    }

    #[test]
    fn invalid_image_ingress_normal_rgba_bgra_and_empty_fallback_are_unchanged() {
        for format in [ImageFormat::Rgba8, ImageFormat::Bgra8] {
            let image = brush(1, 1, vec![255, 0, 0, 255], format);
            assert!(validate(&BrushRef::Image(image.as_ref())).is_ok());
            assert!(real_sink(image).is_ok());
        }
        let empty = brush(0, 1, vec![], ImageFormat::Rgba8);
        assert!(validate(&BrushRef::Image(empty.as_ref())).is_ok());
        assert!(real_sink(empty).is_ok());
        // Standalone conversion of normal peniko images stays viable.
        let p = Pixmap::new(1, 1);
        assert_eq!((p.width(), p.height()), (1, 1));
    }
}
