use super::*;
use super::learning_multi::bind_fixture;
use crate::ui::dialog::DialogAnswer;
use crate::ui::editor::TextAction;
use crate::ui::menu::MenuChoice;

fn start_typed(h: &mut Harness) {
    h.app.open_study_picker(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, choices, selected }) => {
            assert_eq!(choices.len(), 2);
            assert!(selected.is_empty());
            *layer
        }
        _ => panic!("study collections must be selectable"),
    };
    h.act(layer, MenuChoice::Item(0));
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, .. }) => *layer,
        _ => panic!("picker must stay open"),
    };
    h.act(layer, MenuChoice::Item(1));
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, selected, .. }) => {
            assert_eq!(selected.len(), 2);
            *layer
        }
        _ => panic!("both collections must be available"),
    };
    h.act(layer, MenuChoice::Item(5)); // Two collections + three older modes.
}

#[test]
fn lang05_typed_native_correct_wrong_and_no_persistent_grades_or_md_writes() {
    let mut h = Harness::new();
    let (a, b) = bind_fixture(&mut h);
    let before_a = std::fs::read(&a).unwrap();
    let before_b = std::fs::read(&b).unwrap();
    start_typed(&mut h);
    let (field, layer, expected_answer) = match &h.app.popup {
        Some(Popup::StudyTypedEntry {
            field, dialog, snapshot, index, progress, ..
        }) => {
            assert_eq!(*index, 0);
            assert_eq!(snapshot.deck.cards.len(), 3);
            assert_eq!(progress.answered, 0);
            // Source identities are UUIDs, not a stable exercise ordering.
            // Grade the actual first card rather than assuming "eat" is first.
            let answer = snapshot.deck.cards[*index].accepted_answers[0].clone();
            assert!(["есть", "ел", "читать"].contains(&answer.as_str()));
            (field.id(), dialog.layer, answer)
        }
        _ => panic!("native input dialog must open"),
    };
    h.act(field, TextAction::Changed(expected_answer));
    h.act(layer, DialogAnswer::Confirm);
    let layer = match &h.app.popup {
        Some(Popup::StudyTypedResult { dialog, progress, .. }) => {
            assert_eq!((progress.correct, progress.answered, progress.skipped), (1, 1, 0));
            dialog.layer
        }
        _ => panic!("correct answer is graded temporarily"),
    };
    h.act(layer, DialogAnswer::Confirm);
    let (field, layer) = match &h.app.popup {
        Some(Popup::StudyTypedEntry { field, dialog, index, snapshot, .. }) => {
            assert_eq!(*index, 1);
            assert!(!snapshot.deck.cards[*index].accepted_answers.iter()
                .any(|s| s == "definitely-not-a-translation"));
            (field.id(), dialog.layer)
        }
        _ => panic!("second typed card must open"),
    };
    h.act(field, TextAction::Changed("definitely-not-a-translation".into()));
    h.act(layer, DialogAnswer::Confirm);
    match &h.app.popup {
        Some(Popup::StudyTypedResult { progress, .. }) => {
            assert_eq!((progress.correct, progress.answered, progress.skipped), (1, 2, 0));
        }
        _ => panic!("incorrect answer gets ephemeral feedback"),
    }
    assert_eq!(std::fs::read(&a).unwrap(), before_a);
    assert_eq!(std::fs::read(&b).unwrap(), before_b);
    eprintln!("AKULA_LANG05_TYPED_PASS=correct,wrong,no_markdown_or_grades");
}

#[test]
fn lang05_typed_disallows_stale_answer_after_external_source_change() {
    let mut h = Harness::new();
    let (_, b) = bind_fixture(&mut h);
    start_typed(&mut h);
    let (field, layer) = match &h.app.popup {
        Some(Popup::StudyTypedEntry { field, dialog, .. }) => (field.id(), dialog.layer),
        _ => panic!("typed exercise must start"),
    };
    h.act(field, TextAction::Changed("есть".into()));
    std::fs::write(&b, "| Q | A |\n| --- | --- |\n| read | изменено |\n").unwrap();
    h.act(layer, DialogAnswer::Confirm);
    assert!(matches!(h.app.popup, Some(Popup::StudyPreview { .. })),
        "external file change must abort rather than grade an old response");
    eprintln!("AKULA_LANG05_TYPED_STALE_PASS=changed_source_refuses_grading");
}
