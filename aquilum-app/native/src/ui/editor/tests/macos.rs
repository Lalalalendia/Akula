#![cfg(target_os = "macos")]

use masonry::core::keyboard::{Key, Modifiers, NamedKey};
use super::support::{press, root, text, typing};

#[test]
fn cmd_up_down_reach_document_edges_in_the_real_editor() {
    let mut root = root("first line\nsecond line");
    press(&mut root, Key::Named(NamedKey::ArrowDown), Modifiers::META);
    typing(&mut root, "!");
    assert_eq!(text(&root), "first line\nsecond line!");
    press(&mut root, Key::Named(NamedKey::ArrowUp), Modifiers::META);
    typing(&mut root, "?");
    assert_eq!(text(&root), "?first line\nsecond line!");
}

#[test]
fn option_backspace_deletes_a_word_but_cmd_backspace_deletes_line_prefix() {
    let mut root = root("hello world");
    press(&mut root, Key::Named(NamedKey::End), Modifiers::empty());
    press(&mut root, Key::Named(NamedKey::Backspace), Modifiers::ALT);
    assert_eq!(text(&root), "hello ");
    press(&mut root, Key::Named(NamedKey::Backspace), Modifiers::META);
    assert_eq!(text(&root), "");
}
