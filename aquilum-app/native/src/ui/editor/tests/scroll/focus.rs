//! Regression adapted from Aquilum 0.3.19: the scrollbar may scroll the
//! document, but it must not take text keyboard focus away from the editor.

use super::*;
use super::super::support::{click, drag, editor_id, text, typing};

#[test]
fn clicking_scrollbar_track_preserves_editor_focus_and_text_input() {
    let original: String = (0..200).map(|i| format!("Абзац {i}\n")).collect();
    let mut root = scrolled(TextEditor::new(&original), 400, 300);
    let editor = editor_id(&root);
    root.focus_on(Some(editor));
    let _ = root.redraw();

    let before = root.get_widget_with_tag(PAGE).unwrap().inner().offset();
    click(&mut root, 397.0, 190.0);
    let after = root.get_widget_with_tag(PAGE).unwrap().inner().offset();
    assert!(after > before, "click on the track must actually scroll the page");
    assert_eq!(root.focused_widget(), Some(editor), "scrollbar stole text input focus");

    typing(&mut root, "!");
    let result = text(&root);
    assert_eq!(result.len(), original.len() + 1);
    assert_eq!(result.replacen('!', "", 1), original, "scrollbar click lost document content");
}

#[test]
fn dragging_scrollbar_thumb_keeps_existing_keyboard_focus() {
    let content: String = (0..200).map(|i| format!("Line {i}\n")).collect();
    let mut root = scrolled(TextEditor::new(&content), 400, 300);
    let editor = editor_id(&root);
    root.focus_on(Some(editor));
    let _ = root.redraw();
    drag(&mut root, (397.0, 15.0), (397.0, 170.0));
    assert!(root.get_widget_with_tag(PAGE).unwrap().inner().offset() > 1.0,
        "drag must move the document, not just preserve focus");
    assert_eq!(root.focused_widget(), Some(editor), "thumb drag stole text focus");
}

#[test]
fn clicking_unfocused_scrollbar_never_grants_focus_to_editor() {
    let body: String = (0..200).map(|i| format!("Line {i}\n")).collect();
    let mut root = scrolled(TextEditor::new(&body), 400, 300);
    assert_eq!(root.focused_widget(), None);
    click(&mut root, 397.0, 190.0);
    assert!(root.get_widget_with_tag(PAGE).unwrap().inner().offset() > 1.0);
    assert_eq!(root.focused_widget(), None, "scrollbar must not invent keyboard focus");
}
