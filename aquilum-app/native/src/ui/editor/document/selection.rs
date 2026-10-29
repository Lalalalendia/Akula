use std::ops::Range;

use super::{Document, laid::PLAIN};

impl Document {
    /// Selected inline images show their source for the whole selection,
    /// not only while the moving caret happens to intersect the image span.
    pub(super) fn local_selection(&self, line: usize) -> Option<Range<usize>> {
        let paragraph = self.span(line);
        self.selection.as_ref().and_then(|selection| {
            let start = selection.start.max(paragraph.start);
            let end = selection.end.min(paragraph.end);
            (start < end).then(|| start - paragraph.start..end - paragraph.start)
        })
    }

    pub(super) fn presentation_changed(&self, line: usize, caret: Option<usize>) -> bool {
        let decor = self.laid.get(line).map_or(&PLAIN, |laid| &laid.decor);
        self.local_caret(line, caret) != decor.caret
            || !decor.pictures.is_empty() && self.local_selection(line) != decor.source_selection
    }
}
