//! LANG-03g: direct single-click access to a current visible annotation.
//!
//! A colored span is only a paint projection, not a filesystem permission.
//! Emit an opaque editor/range request; the host must independently authorize
//! the active note and verify the current source digest before revealing text.

use std::ops::Range;
use masonry::core::EventCtx;
use masonry::kurbo::Point;
use crate::ui::editor::resolve::Request;

use super::{AnnotationPaintSpan, TextEditor};

/// At a shared boundary or overlap do not guess which meaning the reader
/// intended; leave the click to normal cursor placement.
fn unique_span_at(spans: &[AnnotationPaintSpan], offset: usize) -> Option<Range<usize>> {
    let mut found = spans.iter().filter(|span|
        span.bytes.start <= offset && offset < span.bytes.end
    );
    let first = found.next()?.bytes.clone();
    found.next().is_none().then_some(first)
}

impl TextEditor {
    pub(super) fn open_annotation_hit(
        &self, ctx: &mut EventCtx<'_>, point: Point,
    ) -> bool {
        let (offset, _) = self.hit(point);
        let Some(range) = unique_span_at(&self.annotation_overlay, offset) else {
            return false;
        };
        let Some(resolver) = self.doc.resolver.as_ref() else {
            return false;
        };
        resolver.request(Request::AnnotationPressed { editor: ctx.widget_id(), range });
        ctx.set_handled();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonry::peniko::Color;

    fn span(range: Range<usize>) -> AnnotationPaintSpan {
        AnnotationPaintSpan { bytes: range, color: Color::from_rgba8(255, 192, 0, 255) }
    }

    #[test]
    fn only_unique_visible_range_is_a_click_target() {
        let spans = [span(0..3), span(4..7)];
        assert_eq!(unique_span_at(&spans, 0), Some(0..3));
        assert_eq!(unique_span_at(&spans, 2), Some(0..3));
        assert_eq!(unique_span_at(&spans, 3), None);
        assert_eq!(unique_span_at(&spans, 5), Some(4..7));
        assert_eq!(unique_span_at(&spans, 7), None);
    }

    #[test]
    fn overlapping_and_duplicate_highlights_are_not_guessed() {
        let spans = [span(0..3), span(2..5)];
        assert_eq!(unique_span_at(&spans, 2), None);
        assert_eq!(unique_span_at(&spans, 0), Some(0..3));
        assert_eq!(unique_span_at(&[span(0..3), span(0..3)], 1), None);
        assert_eq!(unique_span_at(&[], 1), None);
    }
}
