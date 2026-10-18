use super::support::{unfocused, TAG};
use crate::ui::editor::TextEditor;

#[test]
fn code_line_has_an_explicit_narrow_viewport_even_when_layout_overflows() {
    let long = format!("value_{}🐱", "x".repeat(500));
    let source = format!("before\n~~~rust\n{long}\n~~~\nafter");
    let root = unfocused(TextEditor::new(&source).markdown(), 220, 400);
    let editor = root.get_widget_with_tag(TAG).expect("editor");
    let editor = editor.inner();
    let (width, lines) = editor.code_line_shape_for_test(2).expect("code body laid out");
    let clip = editor.code_viewport(2).expect("fenced block viewport");
    assert_eq!(lines, 1);
    assert!(width > clip.width() * 4.0);
    assert!(clip.x0 >= 0.0 && clip.x1 <= 220.0, "no bleed past panel");
    assert!(editor.code_viewport(0).is_none(), "prose must not be clipped");
    assert!(editor.code_viewport(4).is_none(), "following prose must not be clipped");
}
