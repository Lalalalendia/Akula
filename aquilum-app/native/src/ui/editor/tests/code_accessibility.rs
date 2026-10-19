//! Real accessibility tree/actions, not just a painted pixel/keyboard shortcut.
use super::support::{editor_id, focused, press, text, unfocused, TAG};
use crate::ui::editor::TextEditor;
use masonry::accesskit::{Action, ActionRequest, Role, TreeId};
use masonry::core::keyboard::{Key, Modifiers, NamedKey};

fn buttons(root: &masonry::app::RenderRoot) -> Vec<(usize, masonry::core::WidgetId)> {
    root.get_widget_with_tag(TAG).unwrap().inner().visible_code_button_ids()
}

#[test]
fn visible_fenced_headers_expose_real_named_buttons_not_math_or_prose() {
    let source = "prose\n$$\nx + y\n$$\n~~~rust\nlet a = 3;\n~~~\nplain\n~~~python\nprint('ok')\n~~~\n";
    let mut root = unfocused(TextEditor::new(source).markdown(), 590, 420);
    root.handle_window_event(masonry::core::WindowEvent::EnableAccessTree);
    let (_, update) = root.redraw();
    let list = buttons(&root);
    assert_eq!(list.len(), 2, "one semantic button per visible real fenced header");
    assert_eq!(list[0].0, source.find("~~~rust").unwrap());
    assert_eq!(list[1].0, source.find("~~~python").unwrap());
    let tree = update.expect("accessibility tree must be emitted when enabled");
    for (_, id) in &list {
        let node = tree.nodes.iter().find(|(node_id, _)| *node_id == (*id).into())
            .expect("button is a real Masonry child with its own tree node");
        assert_eq!(node.1.role(), Role::Button);
        assert_eq!(node.1.name(), Some("Copy code block contents"));
        assert!(node.1.supports_action(Action::Click));
    }
    assert_eq!(text(&root), source);
}

#[test]
fn accesskit_click_copies_fenced_code_without_changing_markdown_or_caret() {
    let source = "Introduction\n~~~rust\nlet x = \"🐱\";\n~~~\nEnding";
    let mut root = focused(TextEditor::new(source).markdown(), 580, 310);
    root.handle_window_event(masonry::core::WindowEvent::EnableAccessTree);
    let _ = root.redraw();
    let [(opener, id)] = buttons(&root).as_slice() else { panic!("one visible button"); };
    assert_eq!(*opener, source.find("~~~rust").unwrap());
    let before_caret = root.get_widget_with_tag(TAG).unwrap().inner().caret_offset();
    root.handle_access_event(ActionRequest {
        target_tree: TreeId::ROOT,
        target_node: (*id).into(),
        action: Action::Click,
        data: None,
    });
    let _ = root.redraw();
    let editor = root.get_widget_with_tag(TAG).unwrap();
    assert_eq!(editor.inner().copied_code_for_test(), Some(*opener));
    assert_eq!(editor.inner().caret_offset(), before_caret);
    assert_eq!(text(&root), source, "accessibility Click must not create a document edit");
}

#[test]
fn focused_semantic_button_activates_on_enter_and_space_without_typing() {
    let source = "~~~rust\nvery long line 🐱\n~~~";
    let mut root = focused(TextEditor::new(source).markdown(), 530, 240);
    let [(opener, button)] = buttons(&root).as_slice() else { panic!("one button"); };
    let (opener, button) = (*opener, *button);
    root.focus_on(Some(button));
    let _ = root.redraw();
    press(&mut root, Key::Named(NamedKey::Enter), Modifiers::empty());
    assert_eq!(root.get_widget_with_tag(TAG).unwrap().inner().copied_code_for_test(), Some(opener));
    assert_eq!(text(&root), source);
    press(&mut root, Key::Character(" ".into()), Modifiers::empty());
    assert_eq!(text(&root), source, "Space on a button must not insert space into code");
}

#[test]
fn removed_code_fence_removes_old_widget_id_without_rebinding_it() {
    let source = "~~~rust\nold\n~~~\nnormal";
    let mut root = unfocused(TextEditor::new(source).markdown(), 560, 300);
    let original = buttons(&root);
    assert_eq!(original.len(), 1);
    let old_id = original[0].1;
    let eid = editor_id(&root);
    root.edit_widget(eid, |mut editor| TextEditor::reset_text(&mut editor.downcast(), "plain prose\n\nno fence"));
    let _ = root.redraw();
    let _ = root.redraw();
    assert!(buttons(&root).is_empty(), "nonexistent code cannot remain focusable");
    root.edit_widget(eid, |mut editor| TextEditor::reset_text(&mut editor.downcast(), "~~~python\nnew\n~~~"));
    let _ = root.redraw();
    let _ = root.redraw();
    let renewed = buttons(&root);
    assert_eq!(renewed.len(), 1);
    assert_ne!(renewed[0].1, old_id, "a removed WidgetId must not point at a different fence");
    assert_eq!(text(&root), "~~~python\nnew\n~~~");
}
