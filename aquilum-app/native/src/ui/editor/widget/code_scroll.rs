//! Per-fenced-block horizontal view state, independent of document content.
//!
//! This is the bounded, edit-safe state foundation. Horizontal layout, clip,
//! hit-testing and wheel routing are deliberately a separate, gated step:
//! changing only the state must not alter current Markdown rendering.

use std::collections::BTreeMap;
use crate::ui::editor::change::Change;
use super::{Affinity, Block, Cursor, EventCtx, Point, Rect, TextEditor};

const MAX_TRACKED_BLOCKS: usize = 512;

/// A byte offset into the original note identifies the first fence line.
/// Indices of loaded paragraphs are NOT durable across document windows.
#[derive(Default, Debug)]
pub(super) struct Offsets {
    positions: BTreeMap<usize, f64>,
}

#[allow(dead_code)] // Read/update methods are exercised by tests; GUI integration follows after pixel A/B.
impl Offsets {
    pub(super) fn get(&self, opener: usize) -> f64 {
        self.positions.get(&opener).copied().unwrap_or(0.0)
    }

    /// Apply a horizontal delta only when a finite, real overflow exists.
    /// Returning false allows the parent scroll container to handle the wheel.
    pub(super) fn advance(&mut self, opener: usize, delta: f64, content: f64, viewport: f64) -> bool {
        if !delta.is_finite() || !content.is_finite() || !viewport.is_finite() || content <= viewport || viewport <= 0.0 {
            return false;
        }
        let max = (content - viewport).max(0.0);
        let before = self.get(opener);
        let after = (before + delta).clamp(0.0, max);
        if after == before {
            return false;
        }
        if after == 0.0 {
            self.positions.remove(&opener);
        } else {
            if !self.positions.contains_key(&opener) && self.positions.len() >= MAX_TRACKED_BLOCKS {
                // Bound long-lived editor state without tying it to page indices.
                self.positions.pop_first();
            }
            self.positions.insert(opener, after);
        }
        true
    }

    /// Clamp a previously scrolled block after a font/width/content change.
    /// A block with no overflow must not retain a stale offscreen offset.
    pub(super) fn clamp_to(&mut self, opener: usize, content: f64, viewport: f64) {
        let max = if content.is_finite() && viewport.is_finite() && viewport > 0.0 {
            (content - viewport).max(0.0)
        } else {
            0.0
        };
        if let Some(value) = self.positions.get_mut(&opener) {
            *value = value.min(max);
            if *value == 0.0 {
                self.positions.remove(&opener);
            }
        }
    }

    /// Preserve offsets only for fences strictly after/before an edit, never
    /// allow a destroyed/replaced opener to hand its scroll to a new block.
    pub(super) fn remap(&mut self, change: &Change) {
        let from = change.at;
        let end = from.saturating_add(change.removed.len());
        self.positions = std::mem::take(&mut self.positions)
            .into_iter()
            .filter_map(|(opener, x)| {
                // Insertions exactly at an opener can change its Markdown kind.
                // Edits including the opening byte invalidate the identity.
                if from == opener || (from < opener && end > opener) {
                    return None;
                }
                Some((change.map(opener), x))
            })
            .collect();
    }

    pub(super) fn retain_openers(&mut self, mut still_a_fence: impl FnMut(usize) -> bool) {
        self.positions.retain(|&opener, _| still_a_fence(opener));
    }

    pub(super) fn clear(&mut self) {
        self.positions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{Kind, TextEditor};

    fn change(at: usize, removed: &str, inserted: &str) -> Change {
        Change { at, removed: removed.into(), inserted: inserted.into() }
    }

    #[test]
    fn independent_offsets_are_clamped_and_are_not_document_edits() {
        let mut editor = TextEditor::new("before\n~~~rust\nlong code\n~~~\n~~~python\nother\n~~~").markdown();
        let original = editor.doc.whole();
        let a = original.find("~~~rust").unwrap();
        let b = original.find("~~~python").unwrap();
        assert!(editor.code_scrolls.advance(a, 180.0, 520.0, 200.0));
        assert!(editor.code_scrolls.advance(b, 75.0, 520.0, 200.0));
        assert_eq!((editor.code_scrolls.get(a), editor.code_scrolls.get(b)), (180.0, 75.0));
        assert!(editor.code_scrolls.advance(a, 1_000.0, 520.0, 200.0));
        assert_eq!((editor.code_scrolls.get(a), editor.code_scrolls.get(b)), (320.0, 75.0));
        assert!(!editor.code_scrolls.advance(a, 1.0, 520.0, 200.0));
        editor.code_scrolls.clamp_to(a, 180.0, 200.0);
        assert_eq!(editor.code_scrolls.get(a), 0.0);
        assert_eq!(editor.code_scrolls.get(b), 75.0);
        assert_eq!(editor.doc.whole(), original);
    }

    #[test]
    fn invalid_or_non_overflowing_wheel_does_not_capture_parent_scroll() {
        let mut offsets = Offsets::default();
        for delta in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(!offsets.advance(10, delta, 400.0, 200.0));
        }
        assert!(!offsets.advance(10, 50.0, 200.0, 200.0));
        assert!(!offsets.advance(10, 50.0, f64::NAN, 200.0));
        assert!(!offsets.advance(10, 50.0, 400.0, 0.0));
        assert_eq!(offsets.get(10), 0.0);
    }

    #[test]
    fn inserted_prose_before_fences_rebases_both_without_crossing() {
        let mut editor = TextEditor::new("one\n~~~rust\nx\n~~~\n~~~python\ny\n~~~").markdown();
        let source = editor.doc.whole();
        let a = source.find("~~~rust").unwrap();
        let b = source.find("~~~python").unwrap();
        assert!(editor.code_scrolls.advance(a, 20.0, 300.0, 120.0));
        assert!(editor.code_scrolls.advance(b, 70.0, 300.0, 120.0));
        editor.apply_change(0..0, "start\n".to_owned());
        assert_eq!(editor.code_scrolls.get(a + 6), 20.0);
        assert_eq!(editor.code_scrolls.get(b + 6), 70.0);
        assert_eq!(editor.code_scrolls.get(a), 0.0);
    }

    #[test]
    fn replacing_an_opener_invalidates_only_its_own_scroll() {
        let source = "~~~rust\nx\n~~~\n~~~python\ny\n~~~";
        let mut editor = TextEditor::new(source).markdown();
        let a = 0;
        let b = source.find("~~~python").unwrap();
        assert!(editor.code_scrolls.advance(a, 30.0, 400.0, 120.0));
        assert!(editor.code_scrolls.advance(b, 50.0, 400.0, 120.0));
        editor.apply_change(0..3, "prose".to_owned());
        assert_eq!(editor.code_scrolls.get(a), 0.0);
        assert_eq!(editor.code_scrolls.get(b + 2), 50.0);
    }

    #[test]
    fn edits_inside_body_keep_the_same_opening_fence_scroll() {
        let mut editor = TextEditor::new("~~~rust\nabc\n~~~").markdown();
        assert!(editor.code_scrolls.advance(0, 70.0, 200.0, 100.0));
        editor.replace(8..9, "🐱", Kind::Other);
        assert_eq!(editor.code_scrolls.get(0), 70.0);
        editor.undo(false);
        assert_eq!(editor.code_scrolls.get(0), 70.0);
    }

    #[test]
    fn resets_clear_state_and_zero_offset_is_not_retained() {
        let mut offsets = Offsets::default();
        assert!(offsets.advance(2, 35.0, 250.0, 100.0));
        assert!(offsets.advance(2, -100.0, 250.0, 100.0));
        assert_eq!(offsets.positions.len(), 0);
        assert!(offsets.advance(2, 35.0, 250.0, 100.0));
        offsets.clear();
        assert_eq!(offsets.get(2), 0.0);
    }

    #[test]
    fn deleting_newline_before_a_fence_clears_its_stale_scroll() {
        let original = "before\n~~~rust\nx\n~~~";
        let mut editor = TextEditor::new(original).markdown();
        let opener = original.find("~~~rust").unwrap();
        assert!(editor.code_scrolls.advance(opener, 65.0, 300.0, 100.0));
        editor.apply_change(opener - 1..opener, String::new());
        assert_eq!(editor.code_scrolls.get(opener - 1), 0.0, "joined prose must not own the old fence offset");
    }

    #[test]
    fn destroying_a_middle_fence_marker_clears_scroll() {
        let mut editor = TextEditor::new("~~~rust\ncode\n~~~").markdown();
        assert!(editor.code_scrolls.advance(0, 80.0, 400.0, 180.0));
        editor.apply_change(1..2, "x".to_owned());
        assert_eq!(editor.code_scrolls.get(0), 0.0, "old opener is no longer fenced code");
    }

    #[test]
    fn paint_pass_resolves_long_code_runs_once_and_keeps_neighboring_offsets_independent() {
        let mut source = String::from("prose\n~~~rust\n");
        for n in 0..1_200 {
            source.push_str(&format!("line{n} : длинный код\n"));
        }
        source.push_str("~~~\nafter\n~~~python\nexample\n~~~");
        let mut editor = TextEditor::new(&source).markdown();
        let first = source.find("~~~rust").unwrap();
        let second = source.find("~~~python").unwrap();
        assert!(editor.code_scrolls.advance(first, 80.0, 320.0, 160.0));
        assert!(editor.code_scrolls.advance(second, 30.0, 320.0, 160.0));
        let count = editor.doc.line_count();
        editor.prepare_code_paint_offsets(0, count - 1);
        assert_eq!(editor.paint_code_offsets.len(), count);
        assert_eq!(editor.code_offset_at(0), 0.0);
        assert_eq!(editor.code_offset_at(800), 80.0);
        let second_line = editor.doc.at_offset(second);
        assert_eq!(editor.code_offset_at(second_line), 30.0);
        assert_eq!(editor.code_offset_at(second_line - 1), 0.0);
        editor.paint_code_offsets.clear();
    }

    #[test]
    fn direct_edit_map_can_preserve_before_and_delete_crossing_fence() {
        let mut offsets = Offsets::default();
        assert!(offsets.advance(100, 45.0, 400.0, 200.0));
        offsets.remap(&change(0, "", "abc"));
        assert_eq!(offsets.get(103), 45.0);
        offsets.remap(&change(101, "very long removed", ""));
        assert_eq!(offsets.get(103), 0.0);
    }
}


impl TextEditor {
    pub(super) fn refresh_code_window_opener(&mut self) {
        let base = self.doc.loaded().0;
        // A pending checkpoint can become available on a later layout.
        // Once resolved, the byte identity survives until the window shifts.
        if self.code_window_base != Some(base)
            || (self.code_window_opener.is_none() && base > 0 && self.doc.fenced_code(0))
        {
            self.code_window_base = Some(base);
            self.code_window_opener = self.doc.window_fence_opener();
        }
    }

    /// Text-only viewport: the code panel background/border are not clipped.
    /// It remains in editor-local coordinates, even as document bytes shift.
    pub(in crate::ui::editor) fn code_viewport(&self, line: usize) -> Option<Rect> {
        if self.doc.markdown.is_none() || !self.doc.fenced_code(line) {
            return None;
        }
        const PAD: f64 = 12.0; // Same as markdown::metrics::CODE_PADDING.
        let left = self.shift().x + PAD;
        let right = self.shift().x + self.inner_width() - PAD;
        let top = self.doc.top(line);
        let bottom = self.doc.bottom(line);
        (right > left && bottom > top).then(|| Rect::new(left, top, right, bottom))
    }


    /// Find the opening fence in the mounted document window. Never borrow an
    /// offset from a previous code block if the actual opener is not mounted.
    fn code_opener_at(&self, line: usize) -> Option<usize> {
        if !self.doc.fenced_code(line) {
            return None;
        }
        let mut at = line;
        loop {
            if matches!(self.doc.block(at), Block::Code { first: true, fence: true, .. }) {
                return Some(self.doc.paragraph(at).start);
            }
            if at == 0 {
                return self.code_window_opener;
            }
            if !self.doc.fenced_code(at - 1) {
                return None;
            }
            at -= 1;
        }
    }

    pub(super) fn code_offset_at(&self, line: usize) -> f64 {
        if let Some(index) = line.checked_sub(self.paint_code_first)
            && let Some(value) = self.paint_code_offsets.get(index)
        {
            return *value;
        }
        self.code_opener_at(line).map_or(0.0, |opener| self.code_scrolls.get(opener))
    }

    /// Resolve the opening fence once for the first rendered line and move
    /// forward over the visible range. Layout rendering stays O(N) in the
    /// number of shown paragraphs, not O(N²) on long fenced blocks.
    pub(super) fn prepare_code_paint_offsets(&mut self, first: usize, last: usize) {
        self.paint_code_first = first;
        self.paint_code_offsets.clear();
        if first > last || self.doc.markdown.is_none() {
            return;
        }
        let mut opener = self.code_opener_at(first);
        for line in first..=last {
            if !self.doc.fenced_code(line) {
                opener = None;
            } else if matches!(self.doc.block(line), Block::Code { first: true, fence: true, .. }) {
                opener = Some(self.doc.paragraph(line).start);
            }
            let offset = opener.map_or(0.0, |at| self.code_scrolls.get(at));
            self.paint_code_offsets.push(offset);
        }
    }

    /// Natural, unwrapped width of mounted paragraphs in this one fence run.
    fn code_content_width(&self, opener: usize) -> f64 {
        let mut max: f64 = 0.0;
        let mut at = self.doc.at_offset(opener);
        while at < self.doc.line_count() && self.doc.fenced_code(at) {
            if at != self.doc.at_offset(opener)
                && matches!(self.doc.block(at), Block::Code { first: true, .. })
            {
                break;
            }
            if let Some(layout) = self.doc.paragraph(at).laid() {
                max = max.max(f64::from(layout.width()));
            }
            at += 1;
        }
        max
    }

    pub(super) fn code_scroll_wheel(&mut self, ctx: &mut EventCtx<'_>, point: Point, delta: f64) -> bool {
        let Some(line) = self.index_at(point) else { return false };
        let Some(viewport) = self.code_viewport(line).filter(|rect| rect.contains(point)) else { return false };
        let Some(opener) = self.code_opener_at(line) else { return false };
        let content = self.code_content_width(opener);
        if self.code_scrolls.advance(opener, delta, content, viewport.width()) {
            ctx.request_render();
            return true;
        }
        false
    }

    /// Keep a code caret visible after a keyboard edit or navigation. Unlike
    /// the global single-line scroll, this adjusts only the enclosing fence.
    pub(super) fn reveal_code_caret(&mut self) {
        if !self.laid_out() {
            return;
        }
        let index = self.doc.at_offset(self.focus);
        let Some(viewport) = self.code_viewport(index) else { return };
        let Some(opener) = self.code_opener_at(index) else { return };
        let paragraph = self.doc.paragraph(index);
        let Some(layout) = paragraph.laid() else { return };
        let local = self.focus.saturating_sub(paragraph.start).min(paragraph.len as usize);
        let natural = f64::from(Cursor::from_byte_index(layout, local, Affinity::Downstream)
            .geometry(layout, 0.0).x0)
            + self.shift().x + self.doc.origin(index).x;
        let before = self.code_scrolls.get(opener);
        let visible = natural - before;
        let gutter = 4.0;
        let delta = if visible < viewport.x0 + gutter {
            visible - (viewport.x0 + gutter)
        } else if visible > viewport.x1 - gutter {
            visible - (viewport.x1 - gutter)
        } else {
            0.0
        };
        if delta != 0.0 {
            let content = self.code_content_width(opener);
            self.code_scrolls.advance(opener, delta, content, viewport.width());
        }
    }

    #[cfg(test)]
    pub(in crate::ui::editor) fn code_scroll_for_test(&self, opener: usize) -> f64 {
        self.code_scrolls.get(opener)
    }

    #[cfg(test)]
    pub(in crate::ui::editor) fn code_line_shape_for_test(&self, line: usize) -> Option<(f64, usize)> {
        let layout = self.doc.paragraph(line).laid()?;
        Some((f64::from(layout.width()), layout.lines().count()))
    }
}
