use super::TextEditor;

#[test]
fn pair_typing_and_backspace_at_an_unloaded_caret_keep_exact_rope_bytes() {
    let original = format!("()а)\n{}", "строка с юникодом 🐈\n".repeat(55_000));
    let mut editor = TextEditor::windowed(crop::Rope::from(original.as_str()));
    editor.doc.include(original.len());
    assert!(editor.doc.loaded().0 > 1, "caret must be before the mounted window");

    // Original failure: before(1) would calculate 1 - base with unsigned
    // underflow, and after(1) could panic while deleting a matched pair.
    editor.anchor = 1;
    editor.focus = 1;
    assert!(editor.handle_pair_backspace());
    let without_pair = original.replacen("()", "", 1);
    assert_eq!(editor.doc.whole(), without_pair);

    // The closing delimiter is already present. Typing it must skip the
    // actual character in the persistent Rope rather than duplicating it.
    editor.doc.include(editor.doc.len());
    assert!(editor.doc.loaded().0 > 2);
    editor.anchor = "а".len();
    editor.focus = "а".len();
    assert!(editor.handle_pair_typing(")"));
    assert_eq!(editor.focus, "а)".len());
    assert_eq!(editor.doc.whole(), without_pair);

    // A quote after Cyrillic text must NOT auto-insert a second quote, even
    // though the text to the left is outside the currently loaded window.
    editor.doc.include(editor.doc.len());
    editor.anchor = "а".len();
    editor.focus = "а".len();
    assert!(!editor.handle_pair_typing("'"));
    assert_eq!(editor.doc.whole(), without_pair);
}
