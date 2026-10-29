use super::*;
use crate::ui::editor::markdown::Fonts;

#[test]
fn ime_preview_accepts_exact_64_paragraph_transitions_but_refuses_65() {
    let text = std::iter::repeat_n("строка", 66).collect::<Vec<_>>().join("\n");
    let mut doc = Document::new(&text);
    doc.selection = Some(0..text.len());
    assert!(doc.multiline_preview(Some((0, "に"))).is_none(), "65 transitions exceeds the bounded visual mode");

    let exact_end = text.match_indices('\n').nth(63).unwrap().0 + 1;
    doc.selection = Some(0..exact_end);
    let preview = doc.multiline_preview(Some((0, "に"))).expect("64 paragraph transitions remain eligible");
    assert_eq!(preview.start_line, 0);
    assert_eq!(preview.end_line, 64);
    assert_eq!(preview.end_byte, exact_end);
    assert_eq!(doc.copy(0..text.len()), text);
}

#[test]
fn ime_preview_respects_8_kib_selected_bytes_and_never_changes_source() {
    let text = format!("🍀\n{} хвост", "x".repeat(8200));
    let mut doc = Document::new(&text);
    doc.selection = Some(0..8193);
    assert!(doc.multiline_preview(Some((0, "日本"))).is_none(), "over-budget selection is not collapsed");

    doc.selection = Some(0..8192);
    assert!(doc.multiline_preview(Some((0, "日本"))).is_some(), "exact 8192-byte selected range is allowed");
    assert_eq!(doc.copy(0..text.len()), text, "preedit eligibility never mutates persisted bytes");
}

#[test]
fn ime_preview_refuses_markdown_and_mismatched_preedit_origin() {
    let text = "# Заголовок\n- пункт\n![image](photo.png)";
    let mut doc = Document::new(text);
    doc.selection = Some(0..text.len());
    doc.markdown = Some(Fonts {
        size: 16.0,
        mono: crate::ui::theme::editor_font("iA Writer Mono"),
        ch: 0.0,
        callouts: false,
        tags: false,
    });
    assert!(doc.multiline_preview(Some((0, "日本"))).is_none(), "Markdown and embedded images must not be virtually folded");

    doc.markdown = None;
    assert!(doc.multiline_preview(Some((1, "日本"))).is_none(), "preedit origin must match selection start");
    assert!(doc.multiline_preview(None).is_none(), "no composition means no virtual folding");
}
