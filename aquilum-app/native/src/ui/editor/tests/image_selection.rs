use super::images::editor_with_images;
use super::support::{editor_id, focused, right_click, selected, text, unfocused, TAG};
use crate::ui::editor::TextEditor;

#[test]
fn long_html_image_source_remains_visible_after_select_and_context_blur() {
    // The exact kind of long HTML tag reported by the user. FakeMedia uses
    // an in-memory image; this test needs no HTTP access to githubusercontent.
    let source = r#"<img width="647" src="https://user-images.githubusercontent.com/71256/29091486-fa38524c-7c37-11e7-895f-e7ec8e1039b6.png">"#;
    let mut root = unfocused(editor_with_images(source), 440, 420);
    let id = editor_id(&root);
    let boxes = |root: &masonry::app::RenderRoot| {
        root.get_widget_with_tag(TAG).unwrap().inner().inline_boxes(0)
    };
    assert_eq!(boxes(&root), 1, "without selection an inline image is previewed");

    root.edit_widget(id, |mut editor| {
        TextEditor::select_byte_range(&mut editor.downcast(), 0, source.len());
    });
    let _ = root.redraw();
    assert_eq!(boxes(&root), 0, "selection must reveal copyable HTML text");
    assert_eq!(selected(&root), source);

    // The real native context menu transfers focus to its popup layer.
    // This simulation also removes editor focus but keeps the selection.
    root.focus_on(None);
    let _ = root.redraw();
    assert_eq!(boxes(&root), 0, "losing focus must not replace selected text with an image");
    right_click(&mut root, 80.0, 18.0);
    assert_eq!(boxes(&root), 0);
    assert_eq!(selected(&root), source);
    assert_eq!(text(&root), source, "copying cannot change the document");

    root.edit_widget(id, |mut editor| {
        TextEditor::select_byte_range(&mut editor.downcast(), 0, 0);
    });
    let _ = root.redraw();
    assert_eq!(boxes(&root), 1, "preview returns when the selection is cleared");
}

#[test]
fn dragging_selection_across_inline_html_image_does_not_collapse_source() {
    let tag = r#"<img src="local.png" width="120">"#;
    let source = format!("Before {tag} after");
    let mut root = focused(editor_with_images(&source), 600, 350);
    let id = editor_id(&root);
    let tag_start = source.find(tag).unwrap();
    // Selection has its moving focus endpoint OUTSIDE the image tag.
    root.edit_widget(id, |mut editor| {
        TextEditor::select_byte_range(&mut editor.downcast(), tag_start + 3, source.len());
    });
    let _ = root.redraw();
    assert_eq!(root.get_widget_with_tag(TAG).unwrap().inner().inline_boxes(0), 0);
    // Expanding the selection past both sides must not toggle the preview.
    root.edit_widget(id, |mut editor| {
        TextEditor::select_byte_range(&mut editor.downcast(), 0, source.len());
    });
    let _ = root.redraw();
    assert_eq!(root.get_widget_with_tag(TAG).unwrap().inner().inline_boxes(0), 0);
    root.focus_on(None);
    let _ = root.redraw();
    assert_eq!(root.get_widget_with_tag(TAG).unwrap().inner().inline_boxes(0), 0);
    assert_eq!(selected(&root), source);
}
