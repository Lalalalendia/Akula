use super::preedit::preview_preedit;

#[test]
fn ime_preview_replaces_single_paragraph_unicode_selection_without_mutating_source() {
    let source = "left 世界 right";
    let start = "left ".len();
    let end = start + "世界".len();
    let (shown, underline) = preview_preedit(source, 0..source.len(), start, Some(&(start..end)), "日本")
        .expect("a valid Unicode selection must produce a temporary preview");
    assert_eq!(shown, "left 日本 right");
    assert_eq!(underline, start..start + "日本".len());
    assert_eq!(source, "left 世界 right", "IME preview must not mutate document bytes");
}

#[test]
fn ime_preview_inserts_at_caret_without_selection_and_respects_global_paragraph_offset() {
    let source = "abc 中文";
    let global_start = 100;
    let at = global_start + 4;
    let (shown, underline) = preview_preedit(source, global_start..global_start + source.len(), at, None, "に")
        .unwrap();
    assert_eq!(shown, "abc に中文");
    assert_eq!(underline, 4..7);
}

#[test]
fn ime_preview_does_not_consume_multiline_selection_or_split_utf8() {
    let source = "ab猫cd";
    let at = 2;
    let (shown, underline) = preview_preedit(source, 0..source.len(), at, Some(&(at..50)), "に")
        .expect("multiline selection must retain source rather than delete cross-paragraph bytes");
    assert_eq!(shown, "abに猫cd");
    assert_eq!(underline, 2..5);
    assert!(preview_preedit(source, 0..source.len(), 3, None, "に").is_none(), "invalid UTF-8 boundary must fail closed");
}
