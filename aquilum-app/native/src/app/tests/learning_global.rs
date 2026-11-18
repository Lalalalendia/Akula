use super::*;
use super::learning_multi::bind_fixture;
use crate::ui::keymap::{Command, Scope, command};
use masonry::core::keyboard::{Key, Modifiers};

#[test]
fn lang05_global_study_shortcut_works_from_empty_workspace_tab() {
    let mut h = Harness::new();
    bind_fixture(&mut h);
    h.app.tabs.new_tab();
    h.app.show_active(&mut h.root);
    assert!(h.app.note.is_none());
    let outcome = h.app.command(&mut h.root, Command::StudyFlashcards);
    assert!(outcome.is_some());
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, choices, selected }) => {
            assert_eq!(choices.len(), 2);
            assert!(selected.is_empty());
            *layer
        }
        _ => panic!("global shortcut should open study picker even with no note"),
    };
    h.act(layer, crate::ui::menu::MenuChoice::Item(0));
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, selected, .. }) => {
            assert_eq!(selected.len(), 1);
            *layer
        }
        _ => panic!("global picker should retain checked group"),
    };
    h.act(layer, crate::ui::menu::MenuChoice::Item(2));
    assert!(matches!(h.app.popup, Some(Popup::StudyCard { .. })),
        "closed registered notes should work while no source is open");
}

#[test]
fn lang05_global_shortcut_is_documented_and_never_steals_plain_k() {
    let shortcut = Key::Character("k".into());
    assert_eq!(command(&[Scope::Global], &shortcut, Modifiers::empty()), None);
    let modifier = crate::test_dir::command() | Modifiers::SHIFT;
    assert_eq!(command(&[Scope::Global], &shortcut, modifier),
        Some(Command::StudyFlashcards));
    let label = crate::ui::keymap::label(Scope::Global, Command::StudyFlashcards);
    assert!(label.contains("Shift"));
}
