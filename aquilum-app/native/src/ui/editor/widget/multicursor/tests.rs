use super::TextEditor;

fn cursors(editor: &TextEditor) -> (usize, Vec<usize>) {
    let mut extra: Vec<usize> = editor.extra_cursors.iter().map(|&(_, f)| f).collect();
    extra.sort_unstable();
    (editor.focus, extra)
}

#[test]
fn every_cursor_keeps_its_place_after_typing_and_deleting() {
    let mut editor = TextEditor::new("abcdefghij");
    (editor.anchor, editor.focus) = (2, 2);
    editor.add_cursor_at(5);
    editor.add_cursor_at(8);
    editor.apply_multicursor_typing("XY");
    assert_eq!(editor.doc.whole(), "abXYcdeXYfghXYij");
    assert_eq!(cursors(&editor), (4, vec![9, 14]));
    editor.apply_multicursor_delete(false);
    assert_eq!(editor.doc.whole(), "abXcdeXfghXij");
    assert_eq!(cursors(&editor), (3, vec![7, 11]));
}

#[test]
fn every_cursor_deletes_a_whole_grapheme() {
    let text = "a👍🏽b e\u{301} c";
    let mut editor = TextEditor::new(text);
    let (thumb, accent) = ("a👍🏽".len(), "a👍🏽b e\u{301}".len());
    (editor.anchor, editor.focus) = (thumb, thumb);
    editor.add_cursor_at(accent);
    editor.apply_multicursor_delete(false);
    assert_eq!(editor.doc.whole(), "ab  c");
}

#[test]
fn overlapping_selections_replace_entire_union_once() {
    let mut editor = TextEditor::new("abcdefghij");
    (editor.anchor, editor.focus) = (2, 5);
    editor.extra_cursors.push((4, 8));
    assert!(editor.apply_multicursor_typing("X"));
    assert_eq!(editor.doc.whole(), "abXij");
    assert!(editor.undo(false));
    assert_eq!(editor.doc.whole(), "abcdefghij");
    assert!(editor.undo(true));
    assert_eq!(editor.doc.whole(), "abXij");
}

#[test]
fn chained_and_reversed_selections_replace_entire_union() {
    let mut editor = TextEditor::new("abcdefghij");
    (editor.anchor, editor.focus) = (4, 1);
    editor.extra_cursors.push((6, 3));
    editor.extra_cursors.push((5, 8));
    assert!(editor.apply_multicursor_typing("Z"));
    assert_eq!(editor.doc.whole(), "aZij");
}

#[test]
fn overlapping_selections_delete_entire_union() {
    let mut editor = TextEditor::new("abcdefghij");
    (editor.anchor, editor.focus) = (2, 5);
    editor.extra_cursors.push((4, 8));
    assert!(editor.apply_multicursor_delete(true));
    assert_eq!(editor.doc.whole(), "abij");
    assert!(editor.undo(false));
    assert_eq!(editor.doc.whole(), "abcdefghij");
}

#[test]
fn nested_selections_do_not_duplicate_edits() {
    let mut editor = TextEditor::new("abcdefghij");
    (editor.anchor, editor.focus) = (2, 8);
    editor.extra_cursors.push((3, 5));
    assert!(editor.apply_multicursor_typing("Q"));
    assert_eq!(editor.doc.whole(), "abQij");
}

#[test]
fn touching_selections_are_distinct_edits() {
    let mut editor = TextEditor::new("abcdefghij");
    (editor.anchor, editor.focus) = (2, 4);
    editor.extra_cursors.push((4, 6));
    assert!(editor.apply_multicursor_typing("X"));
    assert_eq!(editor.doc.whole(), "abXXghij");
}

#[test]
fn identical_cursors_insert_only_once() {
    let mut editor = TextEditor::new("abcd");
    (editor.anchor, editor.focus) = (2, 2);
    editor.extra_cursors.push((2, 2));
    assert!(editor.apply_multicursor_typing("X"));
    assert_eq!(editor.doc.whole(), "abXcd");
}
