//! LANG-05e: matching game on current, unambiguous authorized table pairs.
//!
//! The Core projection skips duplicate questions/answers and multi-answer
//! rows. Native GUI never guesses matches or persists scores/source content.
//! Every question, answer and feedback transition revalidates all sources.

use aquilum_core::learning::practice::{self, MatchPair};
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::Point;

use super::super::{App, Popup};
use super::study_load::StudyDeckSnapshot;
use super::study::{bounded_text, feedback_answer};
use crate::i18n::{t, t_with};
use crate::ui::dialog::{ConfirmDialog, DialogAnswer, DialogIds};
use crate::ui::menu::MenuChoice;

pub(in crate::app) struct MatchRound {
    pub snapshot: Box<StudyDeckSnapshot>,
    pub pairs: Vec<MatchPair>,
    /// Indices into the verified, unambiguous pair vector.
    pub remaining: Vec<usize>,
    pub mistakes: usize,
}

fn compact(value: &str) -> String { bounded_text(value, 180) }

impl App {
    pub(in crate::app) fn start_study_matching(
        &mut self, root: &mut RenderRoot, snapshot: Box<StudyDeckSnapshot>,
    ) {
        if !self.revalidate_study_deck(&snapshot) {
            self.study_notice(root, &t("editor.table.studyPracticeChanged"));
            return;
        }
        let pairs = practice::matching_pairs(&snapshot.deck, 6);
        if pairs.len() < 2 {
            self.study_notice(root, &t("editor.table.studyMatchFewPairs"));
            return;
        }
        let remaining = (0..pairs.len()).collect();
        self.show_study_match_question(root, Box::new(MatchRound {
            snapshot, pairs, remaining, mistakes: 0,
        }));
    }

    fn show_study_match_question(
        &mut self, root: &mut RenderRoot, state: Box<MatchRound>,
    ) {
        if !self.revalidate_study_deck(&state.snapshot) {
            self.study_notice(root, &t("editor.table.studyPracticeChanged"));
            return;
        }
        if state.remaining.is_empty() {
            let score = state.pairs.len().to_string();
            let mistakes = state.mistakes.to_string();
            self.study_notice(root, &t_with("editor.table.studyMatchFinished", &[
                ("pairs", &score), ("mistakes", &mistakes),
            ]));
            return;
        }
        let entries = state.remaining.iter().enumerate()
            .map(|(i, id)| (i, compact(&state.pairs[*id].question)))
            .collect::<Vec<_>>();
        let remaining = state.remaining.len().to_string();
        let title = t_with("editor.table.studyMatchQuestions",
            &[("count", &remaining)]);
        let layer = self.study_menu(root, &title, &entries);
        self.popup = Some(Popup::StudyMatchQuestion { layer, state });
    }

    pub(in crate::app) fn on_study_match_question(
        &mut self, root: &mut RenderRoot, layer: WidgetId,
        state: Box<MatchRound>, action: &ErasedAction,
    ) {
        let Some(choice) = action.downcast_ref::<MenuChoice>() else {
            self.popup = Some(Popup::StudyMatchQuestion { layer, state });
            return;
        };
        let MenuChoice::Item(index) = choice else {
            if matches!(choice, MenuChoice::Hover(_)) {
                self.popup = Some(Popup::StudyMatchQuestion { layer, state });
            }
            return;
        };
        root.remove_layer(layer);
        let Some(&question_index) = state.remaining.get(*index) else { return };
        self.show_study_match_answers(root, state, question_index);
    }

    fn show_study_match_answers(
        &mut self, root: &mut RenderRoot,
        state: Box<MatchRound>, question_index: usize,
    ) {
        if !self.revalidate_study_deck(&state.snapshot) {
            self.study_notice(root, &t("editor.table.studyPracticeChanged"));
            return;
        }
        let entries = state.remaining.iter().rev().enumerate()
            .map(|(i, id)| (i, compact(&state.pairs[*id].answer)))
            .collect::<Vec<_>>();
        let title = t_with("editor.table.studyMatchAnswers",
            &[("question", &compact(&state.pairs[question_index].question))]);
        let layer = self.study_menu(root, &title, &entries);
        self.popup = Some(Popup::StudyMatchAnswer { layer, state, question_index });
    }

    pub(in crate::app) fn on_study_match_answer(
        &mut self, root: &mut RenderRoot, layer: WidgetId,
        mut state: Box<MatchRound>, question_index: usize,
        action: &ErasedAction,
    ) {
        let Some(choice) = action.downcast_ref::<MenuChoice>() else {
            self.popup = Some(Popup::StudyMatchAnswer { layer, state, question_index });
            return;
        };
        let MenuChoice::Item(index) = choice else {
            if matches!(choice, MenuChoice::Hover(_)) {
                self.popup = Some(Popup::StudyMatchAnswer { layer, state, question_index });
            }
            return;
        };
        root.remove_layer(layer);
        let Some(answer_index) = state.remaining.iter().rev().nth(*index).copied()
            else { return };
        if !self.revalidate_study_deck(&state.snapshot) {
            self.study_notice(root, &t("editor.table.studyPracticeChanged"));
            return;
        }
        let correct = answer_index == question_index;
        if correct {
            state.remaining.retain(|i| *i != question_index);
        } else {
            state.mistakes += 1;
        }
        let (widget, dialog) = ConfirmDialog::new(
            &t("editor.table.studyMatchTitle"),
            &t(if correct {
                "editor.table.studyMatchCorrect"
            } else { "editor.table.studyMatchWrong" }),
            &t("editor.table.studyClose"),
            &t("editor.table.studyMatchContinue"),
        );
        root.add_layer(widget, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::StudyMatchFeedback { dialog, state });
    }

    pub(in crate::app) fn on_study_match_feedback(
        &mut self, root: &mut RenderRoot, id: WidgetId,
        dialog: DialogIds, state: Box<MatchRound>, action: &ErasedAction,
    ) {
        match feedback_answer(root, id, &dialog, action) {
            Some(answer) => {
                if answer == DialogAnswer::Confirm {
                    self.show_study_match_question(root, state);
                }
            }
            None => self.popup = Some(Popup::StudyMatchFeedback { dialog, state }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_are_bounded_and_utf8_safe() {
        assert_eq!(compact("some\nwords"), "some words");
        assert_eq!(compact(&"я".repeat(300)).chars().count(), 181);
    }
}
