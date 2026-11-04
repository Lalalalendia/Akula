use std::ops::Range;

use masonry::imaging::Painter;
use masonry::parley::Affinity;
use masonry::peniko::Color;

use super::TextEditor;
use crate::ui::editor::document::Document;
use crate::ui::editor::history::Kind;

impl TextEditor {
    pub(super) fn clear_extra_cursors(&mut self) -> bool {
        if self.extra_cursors.is_empty() {
            return false;
        }
        self.extra_cursors.clear();
        true
    }

    pub(super) fn add_cursor_at(&mut self, offset: usize) {
        if !self.extra_cursors.iter().any(|&(a, f)| a == offset && f == offset) && (self.anchor != offset || self.focus != offset) {
            self.extra_cursors.push((offset, offset));
        }
    }

    pub(super) fn paint_extra_carets(&self, painter: &mut Painter<'_>, color: Color) {
        if !self.caret_visible {
            return;
        }
        for &(_, focus) in &self.extra_cursors {
            if focus <= self.doc.len() {
                let caret = self.caret_at(focus, 0, Affinity::Downstream);
                if let Some(viewport) = self.code_viewport(self.doc.at_offset(focus)) {
                    painter.with_fill_clip(viewport, |p| { p.fill(caret, color).draw(); });
                } else {
                    painter.fill(caret, color).draw();
                }
            }
        }
    }

    pub(super) fn paint_extra_selections(&self, painter: &mut Painter<'_>, shown: std::ops::RangeInclusive<usize>, color: Color) {
        for &(anchor, focus) in &self.extra_cursors {
            let start = anchor.min(focus);
            let end = anchor.max(focus);
            if start < end {
                self.paint_range(painter, shown.clone(), start..end, color);
            }
        }
    }

    pub(super) fn apply_multicursor_typing(&mut self, text: &str) -> bool {
        self.each_cursor(Kind::Typing, |_, range| Some((range, text.to_owned())))
    }

    pub(super) fn apply_multicursor_delete(&mut self, forward: bool) -> bool {
        self.each_cursor(Kind::Deleting, |doc, range| {
            let range = if !range.is_empty() {
                range
            } else if forward {
                range.start..range.start + doc.next_grapheme(range.start)
            } else {
                range.start - doc.previous_grapheme(range.start)..range.start
            };
            (!range.is_empty()).then_some((range, String::new()))
        })
    }

    fn each_cursor(&mut self, kind: Kind, edit: impl Fn(&Document, Range<usize>) -> Option<(Range<usize>, String)>) -> bool {
        if self.extra_cursors.is_empty() {
            return false;
        }
        let mut ranges: Vec<Range<usize>> = self.extra_cursors.iter().chain([&(self.anchor, self.focus)]).map(|&(a, f)| a.min(f)..a.max(f)).collect();
        // Prefer the widest selection when two ranges begin at the same byte.
        ranges.sort_unstable_by_key(|range| (range.start, std::cmp::Reverse(range.end)));
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            if let Some(previous) = merged.last_mut() {
                // Merge overlapping selections instead of losing the part
                // extending past the earlier range. Keep adjacent selections
                // separate and coalesce identical empty cursors.
                if range.start < previous.end || range == *previous {
                    previous.end = previous.end.max(range.end);
                    continue;
                }
            }
            merged.push(range);
        }
        let edits = merged.into_iter().filter_map(|range| edit(&self.doc, range)).collect();
        self.dispatch(edits, None, kind);
        for (anchor, focus) in std::iter::once((&mut self.anchor, &mut self.focus)).chain(self.extra_cursors.iter_mut().map(|(a, f)| (a, f))) {
            let end = (*anchor).max(*focus);
            (*anchor, *focus) = (end, end);
        }
        true
    }
}


#[cfg(test)]
mod tests;
