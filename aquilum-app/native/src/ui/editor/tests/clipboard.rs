use masonry::core::keyboard::NamedKey;
use masonry::core::{Ime, TextEvent};

use super::support::{command_key, editor_id, focused, named, paste, root, text};
use crate::ui::editor::TextEditor;

#[test]
fn paste_and_ime_insert_text() {
    let mut root = root("ab");
    named(&mut root, NamedKey::ArrowRight);
    paste(&mut root, "1\n2");
    assert_eq!(text(&root), "a1\n2b");
    root.handle_text_event(TextEvent::Ime(Ime::Preedit("ко".into(), Some((4, 4)))));
    let _ = root.redraw();
    assert_eq!(text(&root), "a1\n2b");
    root.handle_text_event(TextEvent::Ime(Ime::Commit("кот".into())));
    let _ = root.redraw();
    assert_eq!(text(&root), "a1\n2котb");
}

#[test]
fn pasted_url_becomes_titled_link() {
    let mut root = focused(TextEditor::new("a_b").markdown(), 400, 300);
    command_key(&mut root, "a");
    paste(&mut root, "https://example.com");
    assert_eq!(text(&root), r"[a\_b](https://example.com)");
    named(&mut root, NamedKey::End);
    paste(&mut root, " https://x.org/a ");
    let pasted = text(&root);
    let placeholder = pasted.split('[').nth(2).and_then(|s| s.split(']').next()).expect("заглушка").to_owned();
    assert!(placeholder.starts_with("Fetching Title#"));
    let id = editor_id(&root);
    root.edit_widget(id, |mut w| TextEditor::resolve_link_title(&mut w.downcast(), &placeholder, "X*Y"));
    assert_eq!(text(&root), r"[a\_b](https://example.com)[X\*Y](https://x.org/a)");
    paste(&mut root, "https://x.org/p.png");
    assert!(text(&root).ends_with(")https://x.org/p.png"));
}

#[test]
fn select_all_and_paste_replaces_long_windowed_note_without_crash_or_data_loss() {
    let original: String = (0..30_000).map(|n| format!("строка {n} — исходный текст\n")).collect();
    let mut root = focused(TextEditor::windowed(crop::Rope::from(original.as_str())).markdown(), 600, 400);

    // The native keyboard path leaves a global selection 0..len, while the
    // ClipboardPaste path moves the bounded text window to the caret at EOF.
    command_key(&mut root, "a");
    paste(&mut root, "Новый текст\nВторая строка");
    assert_eq!(text(&root), "Новый текст\nВторая строка");

    // Undo must restore the complete original Rope, not only the old window.
    command_key(&mut root, "z");
    assert_eq!(text(&root), original);
}

// Real Windows can deliver another Ctrl chord before its next composition
// frame. Do not require RenderRoot::redraw() between paste and copy/undo.
#[test]
fn source_shortcuts_work_without_a_layout_frame_after_long_select_all_paste() {
    use masonry::core::keyboard::{Code, Key, KeyState, KeyboardEvent, Location};
    use super::support::selected;

    fn shortcut_unpainted(root: &mut masonry::app::RenderRoot, key: &str) {
        for state in [KeyState::Down, KeyState::Up] {
            root.handle_text_event(TextEvent::Keyboard(KeyboardEvent {
                state,
                key: Key::Character(key.into()),
                code: Code::Unidentified,
                location: Location::Standard,
                modifiers: crate::test_dir::command(),
                repeat: false,
                is_composing: false,
            }));
        }
    }

    let original: String = (0..30_000)
        .map(|n| format!("строка {n} — большой документ\n")).collect();
    let mut root = focused(
        TextEditor::windowed(crop::Rope::from(original.as_str())).markdown(),
        600, 400,
    );
    command_key(&mut root, "a");
    let replacement = "AKULA_CLIPBOARD_PROBE";
    root.handle_text_event(TextEvent::ClipboardPaste(replacement.into()));
    assert_eq!(text(&root), replacement, "paste must replace the source");

    // No redraw: content-only chords must survive a dirty layout.
    shortcut_unpainted(&mut root, "a");
    assert_eq!(selected(&root), replacement, "Ctrl+A must select the new source");
    shortcut_unpainted(&mut root, "z");
    assert_eq!(text(&root), original, "Ctrl+Z must restore the full Rope");
    let _ = root.redraw();
    assert_eq!(text(&root), original);
}

#[test]
fn select_all_and_ime_commit_replaces_long_windowed_note() {
    let original = "разделы документа\n".repeat(12_000);
    let mut root = focused(TextEditor::windowed(crop::Rope::from(original.as_str())).markdown(), 600, 400);
    command_key(&mut root, "a");
    // Real text-entry on platforms using IME arrives as a commit, not
    // necessarily a Key::Character(" ") keyboard event.
    root.handle_text_event(TextEvent::Ime(Ime::Commit(" ".into())));
    let _ = root.redraw();
    assert_eq!(text(&root), " ");
}
