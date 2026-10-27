#[cfg(target_os = "macos")]
use masonry::core::keyboard::{Key, Modifiers, NamedKey};

use super::chord::Os;
use super::table::BINDINGS;
use super::Command;
#[cfg(target_os = "macos")]
use super::{command, Scope};

#[test]
fn mac_navigation_aliases_do_not_replace_non_mac_primary_bindings() {
    let get = |command| BINDINGS.iter().find(|b| b.command == command).unwrap();
    for command in [
        Command::WordLeft, Command::WordRight,
        Command::DocumentStart, Command::DocumentEnd,
        Command::DeleteWord, Command::DeleteWordForward,
    ] {
        let binding = get(command);
        assert!(binding.chords.iter().any(|chord| chord.os == Os::Mac));
        assert!(binding.chords.iter().any(|chord| chord.os == Os::Other));
    }
    for command in [
        Command::DocumentStart, Command::DocumentEnd,
        Command::DeleteWord, Command::DeleteWordForward,
    ] {
        assert!(get(command).hidden.iter().all(|chord| chord.os == Os::Mac));
        assert!(!get(command).hidden.is_empty());
    }
    assert!(get(Command::LineDeleteBackward).chords.iter().all(|chord| chord.os == Os::Mac));
}

#[cfg(target_os = "macos")]
#[test]
fn mac_keyboard_chords_obey_platform_navigation_and_word_deletion() {
    let nav = |key, modifiers| command(
        &[Scope::Text, Scope::Editor, Scope::Navigation],
        &Key::Named(key), modifiers,
    );
    let meta = Modifiers::META;
    let alt = Modifiers::ALT;
    let ctrl = Modifiers::CONTROL;
    assert_eq!(nav(NamedKey::ArrowUp, meta), Some(Command::DocumentStart));
    assert_eq!(nav(NamedKey::ArrowDown, meta), Some(Command::DocumentEnd));
    assert_eq!(nav(NamedKey::Home, meta), Some(Command::DocumentStart));
    assert_eq!(nav(NamedKey::End, meta), Some(Command::DocumentEnd));
    assert_eq!(nav(NamedKey::ArrowUp, meta | Modifiers::SHIFT), Some(Command::DocumentStart));
    assert_eq!(nav(NamedKey::ArrowLeft, alt), Some(Command::WordLeft));
    assert_eq!(nav(NamedKey::ArrowRight, alt), Some(Command::WordRight));
    assert_eq!(nav(NamedKey::Backspace, alt), Some(Command::DeleteWord));
    assert_eq!(nav(NamedKey::Backspace, ctrl), Some(Command::DeleteWord));
    assert_eq!(nav(NamedKey::Backspace, meta), Some(Command::LineDeleteBackward));
    assert_eq!(nav(NamedKey::Delete, alt), Some(Command::DeleteWordForward));
    assert_eq!(nav(NamedKey::Delete, ctrl), Some(Command::DeleteWordForward));
    assert_eq!(nav(NamedKey::Backspace, Modifiers::empty()), None);
}
