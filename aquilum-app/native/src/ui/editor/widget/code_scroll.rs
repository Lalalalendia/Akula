//! Per-fenced-block horizontal view state, independent of document content.
//!
//! This is the bounded, edit-safe state foundation. Horizontal layout, clip,
//! hit-testing and wheel routing are deliberately a separate, gated step:
//! changing only the state must not alter current Markdown rendering.

use std::collections::BTreeMap;
use crate::ui::editor::change::Change;

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
    fn direct_edit_map_can_preserve_before_and_delete_crossing_fence() {
        let mut offsets = Offsets::default();
        assert!(offsets.advance(100, 45.0, 400.0, 200.0));
        offsets.remap(&change(0, "", "abc"));
        assert_eq!(offsets.get(103), 45.0);
        offsets.remap(&change(101, "very long removed", ""));
        assert_eq!(offsets.get(103), 0.0);
    }
}
