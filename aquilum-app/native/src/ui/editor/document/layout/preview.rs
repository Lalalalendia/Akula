use super::super::{Document, Fold};

/// A bounded visual-only collapse of selected plain-text paragraphs.
/// Long/windowed selections and Markdown/embedded blocks deliberately fall back.
#[derive(Clone, Copy)]
pub(in crate::ui::editor::document) struct Preview {
    pub start_line: usize,
    pub end_line: usize,
    pub end_byte: usize,
}

impl Document {
    pub(in crate::ui::editor::document) fn multiline_preview(&self, compose: Option<(usize, &str)>) -> Option<Preview> {
        let (at, _) = compose?;
        let range = self.selection.as_ref()?;
        if self.markdown.is_some() || at != range.start || range.end <= at
            || at < self.base || range.end > self.end() || range.end - at > 8192
        {
            return None;
        }
        let (start_line, end_line) = (self.at_offset(at), self.at_offset(range.end));
        if end_line <= start_line || end_line - start_line > 64 {
            return None;
        }
        let end_span = self.span(end_line);
        self.slice(range.end..end_span.end).get(..)?;
        Some(Preview { start_line, end_line, end_byte: range.end })
    }

    pub(super) fn collapse_preview(&mut self, compose: Option<(usize, &str)>) {
        if let Some(preview) = self.multiline_preview(compose) {
            for i in preview.start_line + 1..=preview.end_line {
                self.heights.set(i, 0.0, Fold::Member);
            }
        }
    }
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
