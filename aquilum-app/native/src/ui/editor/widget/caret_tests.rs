use super::*;
use masonry::parley::{FontContext, LayoutContext};

#[test]
fn ime_candidate_caret_stays_at_selection_start_in_both_directions() {
    let source = "left 世界 right";
    let start = "left ".len();
    let end = start + "世界".len();
    for (anchor, focus) in [(start, end), (end, start)] {
        let mut editor = TextEditor::new(source);
        editor.anchor = anchor;
        editor.focus = focus;
        editor.compose = Some(("日本".into(), Some((6, 6))));
        editor.doc.keep = (0.0, 1000.0);
        editor.ensure_layout(&mut FontContext::default(), &mut LayoutContext::new(), 400.0);
        let actual = editor.caret();
        let expected = editor.caret_at(start, "日本".len(), Affinity::Downstream);
        assert!((actual.x0 - expected.x0).abs() < 0.01, "candidate X must align with selection start: {anchor}..{focus}");
        assert!((actual.y0 - expected.y0).abs() < 0.01, "candidate Y must align with selection start: {anchor}..{focus}");
        assert_eq!(editor.doc.copy(start..end), "世界");
    }
}

#[test]
fn ime_candidate_cursor_clamps_mid_codepoint_and_oversized_offsets() {
    let source = "abc世界";
    let mut editor = TextEditor::new(source);
    editor.focus = 3;
    editor.anchor = 3;
    editor.doc.keep = (0.0, 1000.0);
    for (cursor, expected_offset) in [(1, 0), (usize::MAX, "に".len())] {
        editor.compose = Some(("に".into(), Some((cursor, cursor))));
        editor.ensure_layout(&mut FontContext::default(), &mut LayoutContext::new(), 400.0);
        let actual = editor.caret();
        let expected = editor.caret_at(3, expected_offset, Affinity::Downstream);
        assert!((actual.x0 - expected.x0).abs() < 0.01);
    }
}

#[test]
fn ime_multiline_preview_visually_collapses_plain_paragraphs_and_restores_them_on_cancel() {
    let text = "слева 🍀\nсередина 世界\nпосле ✅ хвост";
    let start = text.find("🍀").unwrap();
    let end = text.find(" хвост").unwrap();
    let mut editor = TextEditor::new(text);
    editor.doc.keep = (0.0, 1000.0);
    editor.ensure_layout(&mut FontContext::default(), &mut LayoutContext::new(), 400.0);
    let original_height = editor.doc.height();
    editor.anchor = end;
    editor.focus = start;
    editor.compose = Some(("日本".into(), Some((6, 6))));
    editor.ensure_layout(&mut FontContext::default(), &mut LayoutContext::new(), 400.0);
    assert_eq!(editor.doc.paragraph(1).fold, Fold::Member);
    assert_eq!(editor.doc.paragraph(2).fold, Fold::Member);
    assert!(editor.doc.height() < original_height, "selected paragraphs must be visually collapsed");
    assert_eq!(editor.text(), text, "temporary rendering may not mutate persistent bytes");
    let actual = editor.caret();
    let expected = editor.caret_at(start, "日本".len(), Affinity::Downstream);
    assert!((actual.x0 - expected.x0).abs() < 0.01);
    assert!(editor.cancel_transient_ime());
    editor.ensure_layout(&mut FontContext::default(), &mut LayoutContext::new(), 400.0);
    assert_eq!(editor.doc.paragraph(1).fold, Fold::None);
    assert_eq!(editor.doc.paragraph(2).fold, Fold::None);
    assert!((editor.doc.height() - original_height).abs() < 0.5, "cancel must restore measured height");
    assert_eq!(editor.text(), text);
}
