mod area; mod find_marks; mod methods; mod find_paint; mod measure;
pub mod bar;
mod glide;
mod input;
mod pan;
mod widget;

#[cfg(test)]
mod tests;

pub use area::ScrollArea;
pub use pan::{HScroll, Pan};

pub const LINE: f64 = 48.0;
pub const PAGE: f64 = 0.875;

/// Normalize mouse wheel, precision touchpad and full-page deltas to logical
/// pixels. Editor code/table horizontal scroll must use the same units as
/// the parent scroll container instead of silently dropping PageDelta.
pub fn pixels(delta: &masonry::core::ScrollDelta, scale: f64, page: f64) -> masonry::kurbo::Vec2 {
    use masonry::core::ScrollDelta;
    use masonry::kurbo::Vec2;
    let pixels = match *delta {
        ScrollDelta::LineDelta(x, y) => Vec2::new(f64::from(x), f64::from(y)) * LINE,
        ScrollDelta::PixelDelta(p) if scale.is_finite() && scale > 0.0 => Vec2::new(p.x, p.y) / scale,
        ScrollDelta::PageDelta(x, y) if page.is_finite() && page > 0.0 => Vec2::new(f64::from(x), f64::from(y)) * page,
        _ => Vec2::ZERO,
    };
    // Devices and test harnesses can emit invalid input. Never forward NaN
    // or infinity into table scroll offsets or animation state.
    if pixels.x.is_finite() && pixels.y.is_finite() { pixels } else { Vec2::ZERO }
}

/// A vertical wheel gesture with Shift requests horizontal scrolling. If a
/// gesture is predominantly vertical without Shift, leave it to the parent.
pub fn sideways(delta: masonry::kurbo::Vec2, shift: bool) -> f64 {
    if !delta.x.is_finite() || !delta.y.is_finite() {
        return 0.0;
    }
    if shift && delta.x == 0.0 {
        delta.y
    } else if delta.x.abs() > delta.y.abs() {
        delta.x
    } else {
        0.0
    }
}

#[derive(Debug)]
pub struct Scrolled(pub f64);
