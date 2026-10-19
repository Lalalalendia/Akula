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
        let node = tree.nodes.iter().find(|(node_id, _)| *node_id == masonry::accesskit::NodeId::from(*id))
            .expect("button is a real Masonry child with its own tree node");
        assert_eq!(node.1.role(), Role::Button);
        assert!(matches!(node.1.label(), Some("Copy code block contents" | "Копировать содержимое блока кода")));
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
    let mounted = buttons(&root);
    let [(opener, id)] = mounted.as_slice() else { panic!("one visible button"); };
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
    let mounted = buttons(&root);
    let [(opener, button)] = mounted.as_slice() else { panic!("one button"); };
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
fn actual_tab_navigation_reaches_both_visible_buttons_in_document_order() {
    let source = "~~~rust\nfirst_code\n~~~\nprose\n~~~python\nsecond_code\n~~~";
    let second = source.find("~~~python").unwrap();
    let mut root = focused(TextEditor::new(source).markdown(), 590, 320);
    let mounted = buttons(&root);
    assert_eq!(mounted.len(), 2, "both semantic buttons are mounted");

    // Do not call focus_on(button): this must exercise Masonry's real Tab
    // traversal from the editor and then activation through Enter.
    press(&mut root, Key::Named(NamedKey::Tab), Modifiers::empty());
    assert_eq!(root.focused_widget(), Some(mounted[0].1), "first Tab must focus the first real child");
    press(&mut root, Key::Named(NamedKey::Enter), Modifiers::empty());
    assert_eq!(
        root.get_widget_with_tag(TAG).unwrap().inner().copied_code_for_test(),
        Some(0),
        "first Tab must reach the first code-header button"
    );
    assert_eq!(text(&root), source);

    press(&mut root, Key::Named(NamedKey::Tab), Modifiers::empty());
    assert_eq!(root.focused_widget(), Some(mounted[1].1), "second Tab must focus the second real child");
    press(&mut root, Key::Named(NamedKey::Enter), Modifiers::empty());
    assert_eq!(
        root.get_widget_with_tag(TAG).unwrap().inner().copied_code_for_test(),
        Some(second),
        "second Tab must reach the next visible code-header button"
    );
    assert_eq!(text(&root), source);
    press(&mut root, Key::Named(NamedKey::Tab), Modifiers::SHIFT);
    assert_eq!(root.focused_widget(), Some(mounted[0].1), "Shift+Tab must traverse backward");
    assert_eq!(text(&root), source);
}


#[test]
fn tab_in_fenced_code_body_keeps_editing_focus_instead_of_jumping_to_copy() {
    let source = "~~~rust\nlet x = 1;\n~~~";
    let at = source.find("let x").unwrap();
    let mut root = focused(TextEditor::new(source).markdown(), 590, 300);
    let eid = editor_id(&root);
    assert_eq!(buttons(&root).len(), 1, "there is a visible Copy affordance");
    root.edit_widget(eid, |mut widget| {
        TextEditor::select_byte_range(&mut widget.downcast(), at, at);
    });
    press(&mut root, Key::Named(NamedKey::Tab), Modifiers::empty());
    assert_eq!(root.focused_widget(), Some(eid), "Tab inside the code body retains the editable focus");
    assert_eq!(root.get_widget_with_tag(TAG).unwrap().inner().copied_code_for_test(), None);
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


#[test]
fn replacing_a_note_at_the_same_fence_byte_creates_a_fresh_accessibility_identity() {
    let mut root = unfocused(TextEditor::new("~~~rust\nold\n~~~").markdown(), 560, 290);
    root.handle_window_event(masonry::core::WindowEvent::EnableAccessTree);
    let _ = root.redraw();
    let old = buttons(&root);
    assert_eq!(old.len(), 1);

    let id = editor_id(&root);
    root.edit_widget(id, |mut editor| {
        TextEditor::reset_text(&mut editor.downcast(), "~~~python\nreplacement\n~~~");
    });
    for _ in 0..3 {
        let _ = root.redraw();
    }
    let current = buttons(&root);
    assert_eq!(current.len(), 1, "the new document still has one header");
    assert_eq!(current[0].0, old[0].0, "the test intentionally reuses opener byte 0");
    assert_ne!(current[0].1, old[0].1, "old UIA/AccessKit identity must never copy from a different document");
    assert_eq!(text(&root), "~~~python\nreplacement\n~~~");
}


#[test]
fn delayed_accesskit_click_after_scroll_cannot_copy_an_offscreen_fence() {
    use super::support::{scrolled, PAGE};
    use crate::ui::scroll::ScrollArea;

    let source = format!(
        "~~~rust\nFIRST_SECRET\n~~~\n{}~~~python\nSECOND_PUBLIC\n~~~\n",
        "ordinary paragraph\n".repeat(350)
    );
    let second = source.find("~~~python").unwrap();
    let mut root = scrolled(TextEditor::new(&source).markdown(), 570, 280);
    root.handle_window_event(masonry::core::WindowEvent::EnableAccessTree);
    let _ = root.redraw();
    let before = buttons(&root);
    assert_eq!(before.len(), 1);
    let stale_id = before[0].1;
    assert_eq!(before[0].0, 0);

    let y = root.get_widget_with_tag(TAG).unwrap().inner()
        .top_of(second).expect("second header layout");
    let page = root.get_widget_with_tag(PAGE).unwrap().id();
    root.edit_widget(page, |mut widget| {
        ScrollArea::set_offset(
            &mut widget.downcast::<ScrollArea<TextEditor>>(),
            (y - 40.0).max(0.0),
        );
    });

    // Scroll mutations can reclaim a child before the next explicit redraw.
    // A late UIA action referencing that deleted NodeId must be ignored,
    // not panic inside the vendored Masonry event dispatcher.
    root.handle_access_event(ActionRequest {
        target_tree: TreeId::ROOT,
        target_node: stale_id.into(),
        action: Action::Click,
        data: None,
    });
    for _ in 0..3 { let _ = root.redraw(); }
    let now = buttons(&root);
    assert_eq!(now.len(), 1);
    assert_eq!(now[0].0, second);
    assert_ne!(now[0].1, stale_id);
    assert_eq!(
        root.get_widget_with_tag(TAG).unwrap().inner().copied_code_for_test(),
        None,
        "an offscreen queued Click must never copy an obsolete code body"
    );
    assert_eq!(text(&root), source);
}

#[test]
fn scrolling_between_distant_headers_removes_offscreen_accessibility_buttons() {
    use super::support::{scrolled, PAGE};
    use crate::ui::scroll::ScrollArea;

    let source = format!(
        "~~~rust\nold\n~~~\n{}~~~python\nnew\n~~~\n",
        "ordinary paragraph\n".repeat(350)
    );
    let second = source.find("~~~python").unwrap();
    let mut root = scrolled(TextEditor::new(&source).markdown(), 570, 280);
    root.handle_window_event(masonry::core::WindowEvent::EnableAccessTree);
    for _ in 0..3 {
        let _ = root.redraw();
    }
    let initial = buttons(&root);
    assert_eq!(initial.len(), 1, "only the first header begins inside the viewport");
    assert_eq!(initial[0].0, 0);

    let y = root.get_widget_with_tag(TAG).unwrap().inner()
        .top_of(second).expect("second header position");
    let page = root.get_widget_with_tag(PAGE).unwrap().id();
    root.edit_widget(page, |mut w| {
        ScrollArea::set_offset(&mut w.downcast::<ScrollArea<TextEditor>>(), (y - 40.0).max(0.0));
    });
    for _ in 0..6 {
        let _ = root.redraw();
    }

    let final_buttons = buttons(&root);
    assert_eq!(final_buttons.len(), 1, "the first header must no longer exist in the access tree");
    assert_eq!(final_buttons[0].0, second);
    assert_ne!(final_buttons[0].1, initial[0].1, "a second visible fence needs a distinct WidgetId");
    assert_eq!(text(&root), source);
}
