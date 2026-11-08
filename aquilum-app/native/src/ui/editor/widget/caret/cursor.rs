use unicode_segmentation::GraphemeCursor;
use super::{Affinity, TextEditor};

pub(super) fn on_boundary(text: &str, index: usize) -> bool {
    GraphemeCursor::new(index, text.len(), true).is_boundary(text, 0).unwrap_or(true)
}

pub(super) fn nearest_boundary(text: &str, index: usize) -> usize {
    let index = index.min(text.len());
    if on_boundary(text, index) {
        return index;
    }
    let before = GraphemeCursor::new(index, text.len(), true).prev_boundary(text, 0).ok().flatten().unwrap_or(0);
    let after = GraphemeCursor::new(index, text.len(), true).next_boundary(text, 0).ok().flatten().unwrap_or(text.len());
    if index - before <= after - index { before } else { after }
}


/// The candidate caret follows the provisional text at the start of a
/// selection, not the original endpoint of the persistent document.
pub(super) fn target(editor: &TextEditor) -> (usize, usize, Affinity) {
    let Some((text, cursor)) = &editor.compose else {
        return (editor.focus, 0, editor.affinity);
    };
    let mut offset = cursor.map_or(0, |(start, _)| start.min(text.len()));
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    (editor.selection().start, offset, Affinity::Downstream)
}
