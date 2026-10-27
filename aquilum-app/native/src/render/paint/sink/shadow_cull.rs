//! Conservative viewport rejection for validated analytic shadows.
//! Keep the existing shadow_guard validation before calling this helper.

use masonry::imaging::BlurredRoundedRect;

use super::super::Painter;

// Test-only switch: compare both algorithms through the same Painter sink.
#[cfg(test)]
thread_local! {
    static TEST_CULL_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

#[cfg(test)]
pub(super) fn with_test_culling<T>(enabled: bool, run: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_CULL_ENABLED.with(|slot| slot.set(self.0));
        }
    }
    let prior = TEST_CULL_ENABLED.with(|slot| slot.replace(enabled));
    let _restore = Restore(prior);
    run()
}

pub(super) fn outside(painter: &Painter, draw: &BlurredRoundedRect) -> bool {
    #[cfg(test)]
    if !TEST_CULL_ENABLED.with(|slot| slot.get()) {
        return false;
    }
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
