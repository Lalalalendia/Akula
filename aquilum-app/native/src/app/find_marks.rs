/// A cached source/layout identity; ordinary scrolling must not rescan matches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MarkStamp {
    source: (u64, usize, bool, (usize, usize), f64, f64),
    body_origin: f64,
    scroll_height: f64,
}
