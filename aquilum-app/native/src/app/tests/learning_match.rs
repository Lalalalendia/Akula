use super::*;
use super::learning_multi::bind_fixture;
use crate::ui::dialog::DialogAnswer;
use crate::ui::menu::MenuChoice;

fn start_matching(h: &mut Harness) {
    h.app.open_study_picker(&mut h.root);
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, choices, .. }) => {
            assert_eq!(choices.len(), 2);
            *layer
        }
        _ => panic!("picker must open from workspace"),
    };
    h.act(layer, MenuChoice::Item(0));
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, .. }) => *layer,
        _ => panic!("first collection must remain checked"),
    };
    h.act(layer, MenuChoice::Item(1));
    let layer = match &h.app.popup {
        Some(Popup::StudyPicker { layer, selected, .. }) => {
            assert_eq!(selected.len(), 2);
            *layer
        }
        _ => panic!("two study groups must be selected"),
    };
    h.act(layer, MenuChoice::Item(4)); // 2 collections + flashcards + choice + matching
}

#[test]
fn lang05_matching_native_correct_wrong_and_unchanged_markdown() {
    let mut h = Harness::new();
    let (a, b) = bind_fixture(&mut h);
    let (before_a, before_b) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    start_matching(&mut h);
    let layer = match &h.app.popup {
        Some(Popup::StudyMatchQuestion { layer, state }) => {
            assert_eq!(state.pairs.len(), 3);
            assert_eq!(state.remaining, [0, 1, 2]);
            assert_eq!(state.snapshot.sources.len(), 2);
            *layer
        }
        _ => panic!("matching question menu must be visible"),
    };
    h.act(layer, MenuChoice::Item(0)); // first term
    let layer = match &h.app.popup {
        Some(Popup::StudyMatchAnswer { layer, question_index, .. }) => {
            assert_eq!(*question_index, 0);
            *layer
        }
        _ => panic!("matching answer menu must open"),
    };
    h.act(layer, MenuChoice::Item(2)); // answers are intentionally in reverse order
    let dialog = match &h.app.popup {
        Some(Popup::StudyMatchFeedback { dialog, state }) => {
            assert_eq!(state.remaining, [1, 2]);
            assert_eq!(state.mistakes, 0);
            dialog.layer
        }
        _ => panic!("correct pair must remove one pair"),
    };
    h.act(dialog, DialogAnswer::Confirm);
    let layer = match &h.app.popup {
        Some(Popup::StudyMatchQuestion { layer, state }) => {
            assert_eq!(state.remaining.len(), 2);
            *layer
        }
        _ => panic!("remaining questions must stay available"),
    };
    h.act(layer, MenuChoice::Item(0)); // question 1
    let layer = match &h.app.popup {
        Some(Popup::StudyMatchAnswer { layer, question_index, .. }) => {
            assert_eq!(*question_index, 1);
            *layer
        }
        _ => panic!("answer menu must reopen"),
    };
    h.act(layer, MenuChoice::Item(0)); // reversed answer 2, incorrect for question 1
    match &h.app.popup {
        Some(Popup::StudyMatchFeedback { state, .. }) => {
            assert_eq!(state.remaining, [1, 2], "wrong pairs are not eliminated");
            assert_eq!(state.mistakes, 1);
        }
        _ => panic!("incorrect match must be explained"),
    }
    assert_eq!(std::fs::read(a).unwrap(), before_a);
    assert_eq!(std::fs::read(b).unwrap(), before_b);
    eprintln!("AKULA_LANG05_MATCH_PASS=unique_pairs,correct,wrong,no_markdown_writes");
}

#[test]
fn lang05_matching_external_edit_aborts_before_grading() {
    let mut h = Harness::new();
    let (_, b) = bind_fixture(&mut h);
    start_matching(&mut h);
    let layer = match &h.app.popup {
        Some(Popup::StudyMatchQuestion { layer, .. }) => *layer,
        _ => panic!("matching question must open"),
    };
    h.act(layer, MenuChoice::Item(0));
    let layer = match &h.app.popup {
        Some(Popup::StudyMatchAnswer { layer, .. }) => *layer,
        _ => panic!("matching answers must open"),
    };
    std::fs::write(b, "| Q | A |\n| --- | --- |\n| read | изменилось |").unwrap();
    h.act(layer, MenuChoice::Item(0));
    assert!(matches!(h.app.popup, Some(Popup::StudyPreview { .. })),
        "stale file cannot create a correct/incorrect result");
    eprintln!("AKULA_LANG05_MATCH_STALE_PASS=changed_source_refuses_grading");
}

#[test]
fn lang05_matching_refuses_insufficient_distinct_pairs_after_open_tab_exclusion() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a));
    start_matching(&mut h);
    assert!(matches!(h.app.popup, Some(Popup::StudyPreview { .. })),
        "open active editor must not be read from stale disk or faked into matches");
}

#[test]
fn lang05_matching_completes_full_round_without_persistent_score_or_source_writes() {
    let mut h = Harness::new();
    let (a, b) = bind_fixture(&mut h);
    let originals = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    start_matching(&mut h);
    for completed in 0..3 {
        let question = match &h.app.popup {
            Some(Popup::StudyMatchQuestion { layer, state }) => {
                assert_eq!(state.remaining.len(), 3 - completed);
                *layer
            }
            _ => panic!("next unique question should be displayed"),
        };
        h.act(question, MenuChoice::Item(0));
        let (answer, matching_answer) = match &h.app.popup {
            Some(Popup::StudyMatchAnswer { layer, state, question_index }) => {
                let chosen = state.remaining.iter().rev()
                    .position(|id| id == question_index).unwrap();
                (*layer, chosen)
            }
            _ => panic!("answers from current verified round should be visible"),
        };
        h.act(answer, MenuChoice::Item(matching_answer));
        let result = match &h.app.popup {
            Some(Popup::StudyMatchFeedback { dialog, state }) => {
                assert_eq!(state.remaining.len(), 2 - completed);
                assert_eq!(state.mistakes, 0);
                dialog.layer
            }
            _ => panic!("accepted match should be removed exactly once"),
        };
        h.act(result, DialogAnswer::Confirm);
    }
    assert!(matches!(h.app.popup, Some(Popup::StudyPreview { .. })),
        "after all distinct pairs the game must show a finished notice");
    assert_eq!(std::fs::read(a).unwrap(), originals.0);
    assert_eq!(std::fs::read(b).unwrap(), originals.1);
    eprintln!("AKULA_LANG05_MATCH_COMPLETE_PASS=three_unique_pairs,finish,no_writes");
}
