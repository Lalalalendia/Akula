use super::*;
use super::learning_multi::bind_fixture;
use crate::ui::dialog::DialogAnswer;
use crate::ui::menu::MenuChoice;

fn select_two(h: &mut Harness) {
    h.app.open_study_picker(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, choices, selected }) => {
            assert_eq!(choices.len(), 2);
            assert!(selected.is_empty());
            *layer
        }
        _ => panic!("collection picker must open"),
    };
    h.act(layer, MenuChoice::Item(0));
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, selected, .. }) => {
            assert_eq!(selected.len(), 1);
            *layer
        }
        _ => panic!("first collection selection must persist"),
    };
    h.act(layer, MenuChoice::Item(1));
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, selected, .. }) => {
            assert_eq!(selected.len(), 2);
            *layer
        }
        _ => panic!("two collections must be selected"),
    };
    h.act(layer, MenuChoice::Item(2));
}

#[test]
fn lang05_native_picker_reveals_and_advances_cards_without_changing_markdown() {
    let mut h = Harness::new();
    let (a, b) = bind_fixture(&mut h);
    let original_a = std::fs::read(&a).unwrap();
    let original_b = std::fs::read(&b).unwrap();

    select_two(&mut h);
    let first = match &h.app.popup {
        Some(Popup::StudyCard { dialog, snapshot, index, revealed }) => {
            assert_eq!(*index, 0);
            assert!(!revealed);
            assert_eq!(snapshot.deck.cards.len(), 3, "overlap must not duplicate rows");
            assert_eq!(snapshot.sources.len(), 2, "read each source once");
            dialog.layer
        }
        _ => panic!("first verified card should be visible"),
    };
    h.act(first, DialogAnswer::Confirm);
    let revealed = match &h.app.popup {
        Some(Popup::StudyCard { dialog, index, revealed, .. }) => {
            assert_eq!(*index, 0);
            assert!(*revealed);
            dialog.layer
        }
        _ => panic!("Show answer should reveal first card"),
    };
    h.act(revealed, DialogAnswer::Confirm);
    match &h.app.popup {
        Some(Popup::StudyCard { index, revealed, .. }) => {
            assert_eq!(*index, 1);
            assert!(!revealed);
        }
        _ => panic!("Next should advance one card"),
    };
    assert_eq!(std::fs::read(a).unwrap(), original_a);
    assert_eq!(std::fs::read(b).unwrap(), original_b);
    eprintln!("AKULA_LANG05_NATIVE_CARDS_PASS=multi_select,show_answer,next,no_markdown_write");
}

#[test]
fn lang05_stale_file_invalidates_the_visible_card_before_answer_reveal() {
    let mut h = Harness::new();
    let (_, b) = bind_fixture(&mut h);
    select_two(&mut h);
    let dialog = match &h.app.popup {
        Some(Popup::StudyCard { dialog, revealed, .. }) => {
            assert!(!*revealed);
            dialog.layer
        }
        _ => panic!("front of card should be visible"),
    };
    std::fs::write(&b, "| Q | A |\n| --- | --- |\n| read | другое |").unwrap();
    h.act(dialog, DialogAnswer::Confirm);
    assert!(matches!(h.app.popup, Some(Popup::StudyPreview { .. })),
        "stale source must close the exercise, not reveal an old answer");
    eprintln!("AKULA_LANG05_STALE_PASS=no_old_answer_after_external_edit");
}

#[test]
fn lang05_open_native_editor_tab_is_excluded_without_disabling_other_notes() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a));
    select_two(&mut h);
    match &h.app.popup {
        Some(Popup::StudyCard { snapshot, .. }) => {
            assert_eq!(snapshot.unavailable_notes.len(), 1);
            assert_eq!(snapshot.deck.cards.len(), 1);
            assert_eq!(snapshot.deck.cards[0].question, "read");
        }
        _ => panic!("closed source remains usable when another source is open"),
    }
}

#[test]
fn lang05_start_without_selected_collection_cannot_open_a_quiz() {
    let mut h = Harness::new();
    bind_fixture(&mut h);
    h.app.open_study_picker(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, .. }) => *layer,
        _ => panic!("picker must open"),
    };
    h.act(layer, MenuChoice::Item(2));
    assert!(matches!(h.app.popup, Some(Popup::StudyPreview { .. })));
}

#[test]
fn lang05_note_actions_menu_starts_study_without_changing_existing_action_positions() {
    let mut h = Harness::new();
    bind_fixture(&mut h);
    h.app.open_note_actions_menu(&mut h.root, masonry::kurbo::Point::new(20.0, 20.0));
    let layer = match &h.app.popup {
        Some(Popup::NoteActionsMenu { layer }) => *layer,
        _ => panic!("note actions menu must exist"),
    };
    // This is the append-only command at index 7; old actions keep indices.
    h.act(layer, MenuChoice::Item(7));
    assert!(matches!(h.app.popup, Some(Popup::StudyPicker { .. })),
        "Study flashcards action must open the collection picker");
}
