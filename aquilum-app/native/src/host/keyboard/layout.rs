//! Upstream Aquilum 0.3.28: respect Latin logical layouts, retain physical
//! Ctrl/Cmd shortcuts when the OS delivers non-Latin characters.
use super::{Code, Key, Modifiers};
use crate::ui::keymap;

fn latin(code: Code) -> Option<char> {
    const LETTERS: [Code; 26] = [
        Code::KeyA, Code::KeyB, Code::KeyC, Code::KeyD, Code::KeyE, Code::KeyF, Code::KeyG, Code::KeyH, Code::KeyI,
        Code::KeyJ, Code::KeyK, Code::KeyL, Code::KeyM, Code::KeyN, Code::KeyO, Code::KeyP, Code::KeyQ, Code::KeyR,
        Code::KeyS, Code::KeyT, Code::KeyU, Code::KeyV, Code::KeyW, Code::KeyX, Code::KeyY, Code::KeyZ,
    ];
    LETTERS.iter().position(|&letter| letter == code).map(|i| char::from(b'a' + i as u8))
}

pub(super) fn shortcut_key(key: Key, code: Code, modifiers: Modifiers) -> Key {
    if keymap::action(modifiers) && !modifiers.alt()
        && matches!(&key, Key::Character(text) if !text.is_ascii())
        && let Some(letter) = latin(code)
    {
        return Key::Character(letter.to_string());
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyrillic_ctrl_shortcuts_use_physical_latin_letter() {
        let action = crate::test_dir::command();
        assert_eq!(shortcut_key(Key::Character("с".into()), Code::KeyC, action),
            Key::Character("c".into()));
        assert_eq!(shortcut_key(Key::Character("м".into()), Code::KeyV, action),
            Key::Character("v".into()));
    }

    #[test]
    fn latin_layout_is_not_overridden_by_physical_position() {
        let action = crate::test_dir::command();
        assert_eq!(shortcut_key(Key::Character("v".into()), Code::KeyC, action),
            Key::Character("v".into()));
        assert_eq!(shortcut_key(Key::Character("C".into()), Code::KeyC, action),
            Key::Character("C".into()));
    }

    #[test]
    fn ordinary_typing_altgr_and_named_keys_are_unchanged() {
        let action = crate::test_dir::command();
        let original = Key::Character("с".into());
        assert_eq!(shortcut_key(original.clone(), Code::KeyC, Modifiers::empty()), original);
        assert_eq!(shortcut_key(original.clone(), Code::KeyC, action | Modifiers::ALT), original);
        assert_eq!(shortcut_key(Key::Named(masonry::core::keyboard::NamedKey::Home),
            Code::KeyH, action), Key::Named(masonry::core::keyboard::NamedKey::Home));
    }
}
