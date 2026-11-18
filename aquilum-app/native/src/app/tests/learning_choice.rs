use super::*;
use super::learning_multi::bind_fixture;
use crate::ui::menu::MenuChoice;
use crate::ui::dialog::DialogAnswer;

fn begin_choice(h: &mut Harness) {
    h.app.open_study_picker(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { choices, layer, .. }) => {
            assert_eq!(choices.len(), 2);
            *layer
        }
        _ => panic!("study picker must exist"),
    };
    h.act(layer, MenuChoice::Item(0));
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, .. }) => *layer,
        _ => panic!("first group must remain selected"),
    };
    h.act(layer, MenuChoice::Item(1));
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, selected, .. }) => {
            assert_eq!(selected.len(), 2);
            *layer
        }
        _ => panic!("both collections must be selectable"),
    };
    h.act(layer, MenuChoice::Item(3)); // Two collections, flashcards at 2, choices at 3
}

#[test]
fn lang05_choice_native_options_are_from_tables_and_grade_in_memory_only() {
    let mut h = Harness::new();
    let (a, b) = bind_fixture(&mut h);
    let bytes_a = std::fs::read(&a).unwrap();
    let bytes_b = std::fs::read(&b).unwrap();
    begin_choice(&mut h);
    let (layer, correct) = match &h.app.popup {
        Some(Popup::StudyChoice { layer, question, index, progress, snapshot }) => {
            assert_eq!(*index, 0);
            assert_eq!(snapshot.deck.cards.len(), 3);
            assert_eq!(progress.answered, 0);
            assert!((2..=4).contains(&question.options.len()));
            assert!(question.options.iter().all(|s| ["есть", "ел", "читать"].contains(&s.as_str())));
            (*layer, question.correct_index)
        }
        _ => panic!("choice exercise must use existing native menu"),
    };
    h.act(layer, MenuChoice::Item(correct));
    let dialog = match &h.app.popup {
        Some(Popup::StudyChoiceResult { dialog, progress, .. }) => {
            assert_eq!(progress.answered, 1);
            assert_eq!(progress.correct, 1);
            dialog.layer
        }
        _ => panic!("answer selection must show result without saving grade"),
    };
    h.act(dialog, DialogAnswer::Confirm);
    let (layer, wrong) = match &h.app.popup {
        Some(Popup::StudyChoice { layer, question, index, .. }) => {
            assert_eq!(*index, 1);
            (*layer, (question.correct_index + 1) % question.options.len())
        }
        _ => panic!("next question must be displayed"),
    };
    h.act(layer, MenuChoice::Item(wrong));
    match &h.app.popup {
        Some(Popup::StudyChoiceResult { progress, .. }) => {
            assert_eq!(progress.answered, 2);
            assert_eq!(progress.correct, 1);
        }
        _ => panic!("wrong answer must remain a temporary result"),
    }
    assert_eq!(std::fs::read(a).unwrap(), bytes_a);
    assert_eq!(std::fs::read(b).unwrap(), bytes_b);
    eprintln!("AKULA_LANG05_CHOICE_PASS=two_collections,safe_options,correct,wrong,no_writes");
}

#[test]
fn lang05_choice_rejects_stale_external_edit_before_answer_disclosure() {
    let mut h = Harness::new();
    let (_, b) = bind_fixture(&mut h);
    begin_choice(&mut h);
    let layer = match &h.app.popup {
        Some(Popup::StudyChoice { layer, .. }) => *layer,
        _ => panic!("choice must begin"),
    };
    std::fs::write(b, "| Q | A |\n| --- | --- |\n| read | изменилось |").unwrap();
    h.act(layer, MenuChoice::Item(0));
    assert!(matches!(h.app.popup, Some(Popup::StudyPreview { .. })),
        "a stale source cannot be used to mark a quiz answer");
    eprintln!("AKULA_LANG05_CHOICE_STALE_PASS=no_stale_answer_disclosed");
}
