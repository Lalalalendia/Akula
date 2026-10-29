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
