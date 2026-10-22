/// A cached source/layout identity; ordinary scrolling must not rescan matches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MarkStamp {
    pub(super) source: (u64, usize, bool, (usize, usize), f64, f64),
    pub(super) body_origin: f64,
    pub(super) scroll_height: f64,
}
