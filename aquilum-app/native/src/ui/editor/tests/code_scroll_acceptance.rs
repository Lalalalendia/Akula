//! Native widget-level acceptance of the code-scroll and fixed-header feature.
//! The helpers send actual Masonry pointer/keyboard events through RenderRoot.
//! These tests are NOT a substitute for an installed Windows/macOS smoke test.

use masonry::kurbo::{Affine, Point};
use masonry::core::keyboard::Modifiers;

use super::support::{
    click, command_key, drag, editor_id, focused, selected, text, typing,
    wheel, wheel_with_modifiers, TAG,
};
use crate::ui::editor::TextEditor;

fn body_y(root: &masonry::app::RenderRoot, start: usize) -> f64 {
    root.get_widget_with_tag(TAG).unwrap().inner()
        .top_of(start).expect("code body laid out") + 7.0
}

#[test]
fn mouse_drag_selects_the_correct_unicode_bytes_after_horizontal_wheel() {
    let code = "alpha beta 🐱 gamma delta ".repeat(65);
    let source = format!("before\n~~~rust\n{code}\n~~~\nafter");
    let opener = source.find("~~~rust").unwrap();
    let body = source.find(&code).unwrap();
    let mut root = focused(TextEditor::new(&source).markdown(), 280, 250);
    let y = body_y(&root, body);
    let from = Point::new(72.0, y);
    let to = Point::new(200.0, y);

    let before = root.get_widget_with_tag(TAG).unwrap().inner().offset_at(Affine::IDENTITY, from);
    wheel(&mut root, (from.x, from.y), (-180.0, 0.0));
    let editor = root.get_widget_with_tag(TAG).unwrap();
    assert!(editor.inner().code_scroll_for_test(opener) > 0.0);
    let start = editor.inner().offset_at(Affine::IDENTITY, from);
    let end = editor.inner().offset_at(Affine::IDENTITY, to);
    assert!(start > before, "the pointer must now hit later UTF-8 bytes");
    assert!(body <= start && start < end && end <= body + code.len());
    assert!(source.is_char_boundary(start) && source.is_char_boundary(end));

    drag(&mut root, (from.x, from.y), (to.x, to.y));
    assert_eq!(selected(&root), &source[start..end], "real pointer drag must use translated glyph coordinates");
    assert_eq!(text(&root), source, "selection must not modify the note");
}

#[test]
fn mouse_click_then_type_and_undo_in_scrolled_code_preserves_neighboring_block() {
    let rust = "one_two_three_four_five ".repeat(60);
    let python = "python_value = 123456789 ".repeat(35);
    let source = format!("~~~rust\n{rust}\n~~~\n~~~python\n{python}\n~~~");
    let rust_body = source.find(&rust).unwrap();
    let python_opener = source.find("~~~python").unwrap();
    let mut root = focused(TextEditor::new(&source).markdown(), 280, 300);
    let y = body_y(&root, rust_body);

    wheel(&mut root, (85.0, y), (-220.0, 0.0));
    assert!(root.get_widget_with_tag(TAG).unwrap().inner().code_scroll_for_test(0) > 0.0);
    let target = root.get_widget_with_tag(TAG).unwrap().inner()
        .offset_at(Affine::IDENTITY, Point::new(85.0, y));
    assert!(target > rust_body && target < rust_body + rust.len());
    click(&mut root, 85.0, y);
    assert_eq!(
        root.get_widget_with_tag(TAG).unwrap().inner().selection(),
        target..target,
        "real click must target the horizontally translated caret"
    );

    typing(&mut root, "Q");
    let expected = format!("{}Q{}", &source[..target], &source[target..]);
    assert_eq!(text(&root), expected, "typing must edit the clicked code byte, not the old screen X");
    assert_eq!(root.get_widget_with_tag(TAG).unwrap().inner().code_scroll_for_test(python_opener + 1), 0.0);

    command_key(&mut root, "z");
    assert_eq!(text(&root), source, "Undo must restore the exact Markdown source");
    assert_eq!(root.get_widget_with_tag(TAG).unwrap().inner().code_scroll_for_test(python_opener), 0.0);
}

#[test]
fn copy_button_stays_fixed_and_preserves_selection_when_code_body_scrolls() {
    let code = "scrollable_line_with_unicode_🐱 ".repeat(70);
    let source = format!("intro\n~~~rust\n{code}\n~~~\noutro");
    let opener = source.find("~~~rust").unwrap();
    let body = source.find(&code).unwrap();
    let mut root = focused(TextEditor::new(&source).markdown(), 320, 280);
    let id = editor_id(&root);
    root.edit_widget(id, |mut widget| {
        TextEditor::select_byte_range(&mut widget.downcast(), 0, "intro".len());
    });
    let _ = root.redraw();

    let button_before = root.get_widget_with_tag(TAG).unwrap().inner()
        .code_header_button_for_test(opener).expect("visible fixed button");
    let y = body_y(&root, body);
    wheel(&mut root, (90.0, y), (-150.0, 0.0));
    let editor = root.get_widget_with_tag(TAG).unwrap();
    let scroll = editor.inner().code_scroll_for_test(opener);
    let button_after = editor.inner().code_header_button_for_test(opener).unwrap();
    assert!(scroll > 0.0, "body actually scrolled");
    assert_eq!(button_before, button_after, "header control must not drift with code text");

    click(&mut root, button_after.center().x, button_after.center().y);
    let editor = root.get_widget_with_tag(TAG).unwrap();
    assert_eq!(editor.inner().copied_code_for_test(), Some(opener));
    assert_eq!(editor.inner().code_scroll_for_test(opener), scroll);
    assert_eq!(selected(&root), "intro");
    assert_eq!(text(&root), source);
}

#[test]
fn shift_vertical_wheel_scrolls_only_the_target_fence() {
    let code = "0123456789abcdef".repeat(80);
    let source = format!("before\n~~~rust\n{code}\n~~~\nafter");
    let body = source.find(&code).unwrap();
    let mut root = focused(TextEditor::new(&source).markdown(), 260, 220);
    let y = body_y(&root, body);

    wheel_with_modifiers(&mut root, (80.0, y), (0.0, -140.0), Modifiers::SHIFT);
    let amount = root.get_widget_with_tag(TAG).unwrap().inner().code_scroll_for_test(source.find("~~~rust").unwrap());
    assert!(amount > 0.0, "shift+vertical wheel must act as code horizontal wheel");

    wheel(&mut root, (80.0, y), (0.0, -140.0));
    let same = root.get_widget_with_tag(TAG).unwrap().inner().code_scroll_for_test(source.find("~~~rust").unwrap());
    assert_eq!(amount, same, "normal vertical wheel must not silently move code X");
    assert_eq!(text(&root), source);
}
