//! Akula image-ingress protections for upstream Vello #1269.
use super::*;
use masonry::peniko::{Blob, ImageAlphaType, ImageData, ImageFormat};
use std::sync::Arc;

#[test]
fn zero_size_rgba_input_rejected_before_pixmap_creation() {
    let alpha = ImageAlphaType::AlphaPremultiplied;
    for (width, height) in [(0, 0), (0, 5), (5, 0)] {
        assert!(
            crate::render::from_rgba(vec![], width, height, alpha).is_none(),
            "zero-dimension input {width}x{height} must not create an empty pixmap",
        );
    }
    assert!(crate::render::from_rgba(vec![100, 50, 20, 255], 1, 1, alpha).is_some());
}

#[test]
fn zero_size_pixmap_brush_becomes_safe_transparent_pixel() {
    for (width, height) in [(0, 0), (0, 4), (4, 0)] {
        let brush = crate::render::brush(vello_cpu::Pixmap::new(width, height));
        assert_eq!((brush.image.width, brush.image.height), (1, 1));
        let pixmap = crate::render::pixels::lookup(brush.image.data.id())
            .expect("brush must register its fallback pixmap");
        assert_eq!((pixmap.width(), pixmap.height()), (1, 1));
        assert_eq!(pixmap.data_as_u8_slice(), &[0, 0, 0, 0]);
        assert_eq!(pixmap.sample(0, 0).a, 0);
    }
}

#[test]
fn malformed_zero_size_image_data_stays_transparent_in_renderer() {
    for (width, height) in [(0, 0), (0, 4), (4, 0)] {
        let image = ImageData {
            data: Blob::new(Arc::new(Vec::<u8>::new())),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
            width,
            height,
        };
        let mut images = Images::default();
        let ImageSource::Pixmap(pixmap) = images.source(&image) else {
            panic!("expected transparent Pixmap fallback");
        };
        assert_eq!((pixmap.width(), pixmap.height()), (1, 1));
        assert_eq!(pixmap.sample(0, 0).a, 0);
        let ImageSource::Pixmap(cached) = images.source(&image) else { panic!("cache"); };
        assert!(Arc::ptr_eq(&pixmap, &cached), "fallback must be cached by ID");
    }
}
