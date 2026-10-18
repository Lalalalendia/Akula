use std::ops::Range;

use super::Document;

impl Document {
    pub fn len(&self) -> usize {
        self.full.byte_len()
    }

    pub(super) fn count(&self) -> usize {
        let len = self.full.byte_len();
        self.full.line_len() + usize::from(len == 0 || self.full.byte(len - 1) == b'\n')
    }

    pub(super) fn line_of(&self, byte: usize) -> usize {
        self.full.line_of_byte(self.floor(byte))
    }

    pub(super) fn line_start(&self, line: usize) -> usize {
        self.full.byte_of_line(line)
    }

    pub(super) fn line_end(&self, line: usize) -> usize {
        if line + 1 < self.count() { self.full.byte_of_line(line + 1) - 1 } else { self.full.byte_len() }
    }

    pub fn floor(&self, at: usize) -> usize {
        crate::rope::floor(&self.full, at)
    }


    /// Return the complete body of a fenced Markdown block directly from the
    /// persistent Rope. The editor only keeps a bounded text window for large
    /// notes, so assembling the body from visible paragraphs would truncate it.
    pub(in crate::ui::editor) fn copy_fenced_body(&self, opener: usize) -> Option<String> {
        use aquilum_core::markdown::fence::{Fence, INDENT_LIMIT};
        let first = self.line_of(opener);
        if self.line_start(first) != opener {
            return None;
        }
        let open_line = self.copy(opener..self.line_end(first));
        let fence = Fence::open(&open_line, INDENT_LIMIT)?;
        let first_body = first + 1;
        if first_body >= self.count() {
            return Some(String::new());
        }
        let start = self.line_start(first_body);
        let mut end = self.len();
        for line in first_body..self.count() {
            let candidate = self.copy(self.line_start(line)..self.line_end(line));
            if fence.closes(&candidate) {
                end = if line == first_body { start } else { self.line_end(line - 1) };
                break;
            }
        }
        Some(self.copy(start..end))
    }

    pub fn whole(&self) -> String {
        self.full.to_string()
    }

    pub fn copy(&self, range: Range<usize>) -> String {
        self.full.byte_slice(range).to_string()
    }

    pub fn line_at(&self, at: usize) -> Range<usize> {
        let line = self.line_of(at);
        self.line_start(line)..self.line_end(line)
    }

    pub fn byte(&self, at: usize) -> u8 {
        self.full.byte(at)
    }

    pub fn next_grapheme(&self, at: usize) -> usize {
        self.full.byte_slice(at..).graphemes().next().map_or(0, |g| g.len())
    }

    pub fn previous_grapheme(&self, at: usize) -> usize {
        self.full.byte_slice(..at).graphemes().next_back().map_or(0, |g| g.len())
    }

    pub fn front(&self) -> usize {
        self.front
    }

    pub fn rope(&self) -> crop::Rope {
        self.full.clone()
    }
}
