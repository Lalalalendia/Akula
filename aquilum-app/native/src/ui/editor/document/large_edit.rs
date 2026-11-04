use std::ops::Range;
use super::Document;

impl Document {
    // Large Undo/Redo or whole-note paste must not load the entire Rope into
    // the synchronous GUI layout, even when the previous window was tiny.
    pub(super) fn replace_loaded(&mut self, range: Range<usize>, insert: &str) {
        let large = self.windowed
            && (insert.len() > super::window::SPAN_BYTES
                || insert.bytes().filter(|&b| b == b'\n').take(super::window::MARGIN + 1).count()
                    > super::window::MARGIN);
        if large {
            let start = range.start;
            self.full.replace(range, insert);
            self.rescan_from(start);
            self.load(start + insert.len(), None);
        } else {
            self.splice(range, insert);
        }
    }
}
