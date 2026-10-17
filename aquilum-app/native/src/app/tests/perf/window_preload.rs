use super::*;

#[test]
fn shorter_preload_keeps_the_complete_markdown_after_reopening_a_tab() {
    let mut h = Harness::new();
    let mut text = String::from("# Начало\n");
    for index in 0..12_000 {
        text.push_str(&format!("Строка {index:05}: **выделение** и русский текст.\n"));
    }
    text.push_str("# Конец документа\n");
    let first = h.write_note("long-editor.md", &text);
    let second = h.write_note("short-editor.md", "вторая заметка");

    assert!(h.app.open_in(&mut h.root, first.clone(), true));
    let _ = h.root.redraw();
    assert_eq!(h.body_text(), text, "initial virtual window must not truncate source");
    assert!(h.app.open_in(&mut h.root, second, true));
    let _ = h.root.redraw();
    assert!(h.app.open(&mut h.root, first));
    let _ = h.root.redraw();
    assert_eq!(h.body_text(), text, "returning through clean cache preserves the entire Rope");
    h.app.shutdown();
}
