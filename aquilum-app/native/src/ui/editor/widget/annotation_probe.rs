//! Test-only readback of rendered annotation projections.
//! Production consumers never receive this method.
use std::ops::Range;
use super::TextEditor;

impl TextEditor {
    pub(crate) fn annotation_ranges_for_test(&self) -> Vec<Range<usize>> {
        self.annotation_overlay.iter().map(|span| span.bytes.clone()).collect()
    }
}
