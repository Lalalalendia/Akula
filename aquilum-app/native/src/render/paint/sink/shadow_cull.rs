//! Conservative viewport rejection for validated analytic shadows.
//! Keep the existing shadow_guard validation before calling this helper.

use masonry::imaging::BlurredRoundedRect;
use masonry::kurbo::Rect;

use super::super::Painter;

pub(super) fn outside(painter: &Painter, draw: &BlurredRoundedRect) -> bool {
    // Vello CPU expands analytic blur by 2.5 sigma. Use 3 sigma plus
    // local and device pixel margins to allow raster/rounding differences.
    let spread = 3.0 * draw.std_dev + 2.0;
    let local = draw.rect.abs().inflate(spread, spread);
    let bounds = draw.transform.transform_rect_bbox(local).inflate(2.0, 2.0);
    if ![bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|n| n.is_finite()) {
        return false; // Never cull when a conservative bound is uncertain.
    }
    painter.offscreen(bounds) // Filtered groups remain eligible for rendering.
}

#[cfg(test)]
mod tests;
