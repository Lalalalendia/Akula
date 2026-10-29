//! Akula's Vello CPU tile-extent guard (upstream Vello #2005).
//! Vello's RectU16::snap_to_tile_coordinates() rounds the viewport OUTWARD
//! using checked_next_multiple_of(Tile::WIDTH/HEIGHT). A u16::MAX dimension
//! cannot be rounded to a whole tile without overflowing. Reject before a
//! native RenderContext is created; do not crop or clamp visible pixels.

use super::PaintError;

// Keep this in sync with vendored vello_common::tile::Tile::{WIDTH,HEIGHT}.
const TILE_WIDTH: u16 = 4;
const TILE_HEIGHT: u16 = 4;
const MAX_WIDTH: u16 = u16::MAX - u16::MAX % TILE_WIDTH;
const MAX_HEIGHT: u16 = u16::MAX - u16::MAX % TILE_HEIGHT;
/// Cap the entire screen buffer at 64 Mi RGBA pixels (~256 MiB).
/// The filter intermediate working set has its own independent budget.
const MAX_PIXELS: u64 = 64 * 1024 * 1024;

pub(super) fn validate(width: u16, height: u16) -> Result<(), PaintError> {
    if width > MAX_WIDTH || height > MAX_HEIGHT {
        return Err(PaintError::ViewportBudget("размер нельзя выровнять до тайла в u16"));
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(PaintError::ViewportBudget("превышен бюджет полного кадра"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::Renderer;
    use crate::output::{Canvas, Target};
    use masonry::app::{RenderRoot, RenderRootOptions, WindowSizePolicy};
    use masonry::core::NewWidget;
    use masonry::dpi::PhysicalSize;
    use masonry::peniko::Color;
    use masonry::theme::default_property_set;
    use masonry::widgets::TextArea;
    use std::sync::Arc;

    struct NoRaster;
    impl Canvas for NoRaster {
        fn with_target(&mut self, _: &mut dyn FnMut(Target<'_>)) {
            panic!("invalid viewport must return Err before touching a render target");
        }
    }

    #[test]
    fn viewport_tile_guard_accepts_normal_4k_8k_and_aligned_bounds() {
        for (w, h) in [
            (0, 0), (3840, 2160), (7680, 4320), (8192, 8192),
            (u16::MAX - 3, 1), (1, u16::MAX - 3),
        ] {
            assert!(validate(w, h).is_ok(), "valid viewport {w}x{h}");
        }
    }

    #[test]
    fn viewport_tile_guard_rejects_u16_roundup_overflow_and_enormous_frames() {
        for (w, h) in [
            (u16::MAX, 1), (1, u16::MAX),
            (u16::MAX - 2, 1), (1, u16::MAX - 2),
            (8193, 8192), (12_000, 12_000),
        ] {
            assert!(matches!(validate(w, h), Err(PaintError::ViewportBudget(_))),
                "invalid viewport {w}x{h} was admitted");
        }
    }

    #[test]
    fn viewport_tile_guard_actual_renderer_draw_is_fail_closed_without_allocating() {
        let options = RenderRootOptions {
            default_properties: Arc::new(default_property_set()),
            use_system_fonts: true,
            size_policy: WindowSizePolicy::User,
            size: PhysicalSize::new(160, 80),
            scale_factor: 1.0,
            test_font: None,
        };
        let (layers, _) = RenderRoot::new(
            NewWidget::new(TextArea::new_editable("viewport guard")).erased(),
            |_| (),
            options,
        ).redraw();
        let mut renderer = Renderer::new(0);
        for (w, h) in [(u16::MAX, 1), (1, u16::MAX), (12_000, 12_000)] {
            assert!(matches!(renderer.draw(
                &layers, w, h, 1.0, Color::WHITE, true, &mut NoRaster,
            ), Err(PaintError::ViewportBudget(_))));
            assert!(renderer.params.is_none(),
                "invalid viewport must not mutate the renderer's frame state");
        }
        // Valid geometry still reaches Vello and draws a real scene.
        renderer.render(&layers, 160, 80, 1.0, Color::WHITE)
            .expect("ordinary frame must retain its raster output");
        assert_eq!(renderer.frame_pixels().len(), 160 * 80);
    }
}
