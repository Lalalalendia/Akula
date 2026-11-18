//! LANG-05d: temporary, source-verified multiple-choice exercises.
//!
//! Core supplies all options from already authorized tables, excludes known
//! alternate correct meanings, and may offer fewer than four distinct choices.
//! No invented distractors, persistent grades, SRS, or file writes.

use aquilum_core::learning::practice::{self, ChoiceQuestion};
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::Point;

use super::super::{App, Popup};
use super::study_load::StudyDeckSnapshot;
use crate::i18n::{t, t_with};
use crate::ui::dialog::{ConfirmDialog, DialogAnswer, DialogIds};
use crate::ui::menu::MenuChoice;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct ChoiceProgress {
    pub correct: usize,
    pub answered: usize,
}

fn display(value: &str) -> String {
    let trimmed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.chars().count() > 180 {
        trimmed.chars().take(180).collect::<String>() + "…"
    } else {
        trimmed
    }
}

impl App {
    pub(in crate::app) fn show_study_choice(
        &mut self, root: &mut RenderRoot,
        snapshot: Box<StudyDeckSnapshot>, from: usize, progress: ChoiceProgress,
    ) {
        if !self.revalidate_study_deck(&snapshot) {
            self.study_notice(root, &t("editor.table.studyPracticeChanged"));
            return;
        }
        let next = (from..snapshot.deck.cards.len()).find_map(|i| {
            practice::multiple_choice(&snapshot.deck, i, 4).map(|question| (i, question))
        });
        let Some((index, question)) = next else {
            let message = if progress.answered == 0 {
                t("editor.table.studyChoiceNoOptions")
            } else {
                t_with("editor.table.studyChoiceFinish", &[
                    ("correct", &progress.correct.to_string()),
                    ("answered", &progress.answered.to_string()),
                ])
            };
            self.study_notice(root, &message);
            return;
        };
        let title = t_with("editor.table.studyChoicePrompt", &[
            ("index", &(index + 1).to_string()),
            ("total", &snapshot.deck.cards.len().to_string()),
            ("question", &display(&question.question)),
        ]);
        let entries = question.options.iter().enumerate()
            .map(|(i, option)| (i, display(option))).collect::<Vec<_>>();
        let layer = self.study_menu(root, &title, &entries);
        self.popup = Some(Popup::StudyChoice {
            layer, snapshot, index, question, progress,
        });
    }

    pub(in crate::app) fn on_study_choice(
        &mut self, root: &mut RenderRoot, layer: WidgetId,
        snapshot: Box<StudyDeckSnapshot>, index: usize,
        question: ChoiceQuestion, progress: ChoiceProgress,
        action: &ErasedAction,
    ) {
        let Some(choice) = action.downcast_ref::<MenuChoice>() else {
            self.popup = Some(Popup::StudyChoice {
                layer, snapshot, index, question, progress,
            });
            return;
        };
        let MenuChoice::Item(picked) = choice else {
            if matches!(choice, MenuChoice::Hover(_)) {
                self.popup = Some(Popup::StudyChoice {
                    layer, snapshot, index, question, progress,
                });
            }
            return;
        };
        root.remove_layer(layer);
        if *picked >= question.options.len() { return; }
        if !self.revalidate_study_deck(&snapshot) {
            self.study_notice(root, &t("editor.table.studyPracticeChanged"));
            return;
        }
        let correct = *picked == question.correct_index;
        let answer = question.options.get(question.correct_index)
            .map(|s| display(s)).unwrap_or_default();
        let description = if correct {
            t("editor.table.studyChoiceCorrect")
        } else {
            t_with("editor.table.studyChoiceWrong", &[("answer", &answer)])
        };
        let (widget, dialog) = ConfirmDialog::new(
            &t("editor.table.studyChoiceTitle"), &description,
            &t("editor.table.studyClose"), &t("editor.table.studyChoiceNext"),
        );
        root.add_layer(widget, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        let next_progress = ChoiceProgress {
            correct: progress.correct + if correct { 1 } else { 0 },
            answered: progress.answered + 1,
        };
        self.popup = Some(Popup::StudyChoiceResult {
            dialog, snapshot, index, progress: next_progress,
        });
    }

    pub(in crate::app) fn on_study_choice_result(
        &mut self, root: &mut RenderRoot, id: WidgetId, dialog: DialogIds,
        snapshot: Box<StudyDeckSnapshot>, index: usize,
        progress: ChoiceProgress, action: &ErasedAction,
    ) {
        if action.downcast_ref::<DialogAnswer>() == Some(&DialogAnswer::Closed) {
            root.remove_layer(dialog.layer);
            return;
        }
        match dialog.answer(id, action) {
            Some(answer) => {
                root.remove_layer(dialog.layer);
                if answer == DialogAnswer::Confirm {
                    self.show_study_choice(root, snapshot, index + 1, progress);
                }
            }
            None => self.popup = Some(Popup::StudyChoiceResult {
                dialog, snapshot, index, progress,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_choice_labels_never_show_full_unbounded_table_cells() {
        assert_eq!(display("two \n words"), "two words");
        assert_eq!(display(&"ю".repeat(300)).chars().count(), 181);
    }
}
