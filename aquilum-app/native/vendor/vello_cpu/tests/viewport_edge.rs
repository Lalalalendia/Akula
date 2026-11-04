//! Vello #2005: construct maximum one-dimensional RenderContexts safely.
use vello_cpu::RenderContext;

#[test]
fn vello_viewport_max_context_height_65535_no_panic() {
    let ctx = RenderContext::new(1, u16::MAX);
    assert_eq!((ctx.width(), ctx.height()), (1, u16::MAX));
}

#[test]
fn vello_viewport_max_context_width_65535_no_panic() {
    let ctx = RenderContext::new(u16::MAX, 1);
    assert_eq!((ctx.width(), ctx.height()), (u16::MAX, 1));
}

#[test]
fn vello_viewport_max_context_regular_dimensions_unchanged() {
    for (width, height) in [(1, 1), (640, 480), (3840, 2160)] {
        let ctx = RenderContext::new(width, height);
        assert_eq!((ctx.width(), ctx.height()), (width, height));
    }
}
