//! LANG-05f: native typed answers from current, authorized study tables.
//!
//! A text input is evaluated only after a fresh host-source/SQLite recheck.
//! Ambiguous meanings are explicitly ungraded. All scores stay in memory.

use aquilum_core::learning::answer_check::{check_typed_answer, TypedVerdict};
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::Point;

use super::super::{App, Popup};
use super::study_load::StudyDeckSnapshot;
use super::study::{bounded_text, feedback_answer};
use crate::i18n::{t, t_with};
use crate::ui::dialog::{ConfirmDialog, DialogAnswer, DialogIds};
use crate::ui::editor::{TextAction, TextEditor};
use crate::ui::handle::Handle;
use crate::ui::search::Placeholder;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct TypedProgress {
    pub correct: usize,
    pub answered: usize,
    pub skipped: usize,
}

fn short(value: &str) -> String { bounded_text(value, 120) }

impl App {
    pub(in crate::app) fn show_study_typed(
        &mut self, root: &mut RenderRoot,
        snapshot: Box<StudyDeckSnapshot>, index: usize, progress: TypedProgress,
    ) {
        if !self.revalidate_study_deck(&snapshot) {
            self.study_notice(root, &t("editor.table.studyPracticeChanged"));
            return;
        }
        let Some(card) = snapshot.deck.cards.get(index) else {
            self.study_notice(root, &t_with("editor.table.studyTypedFinished", &[
                ("correct", &progress.correct.to_string()),
                ("answered", &progress.answered.to_string()),
                ("skipped", &progress.skipped.to_string()),
            ]));
            return;
        };
        let title = t_with("editor.table.studyTypedPrompt", &[
            ("index", &(index + 1).to_string()),
            ("total", &snapshot.deck.cards.len().to_string()),
            ("question", &short(&card.question)),
            ("context", &card.context.as_deref().map(short).unwrap_or_default()),
        ]);
        let (dialog, field, placeholder) = super::text_input_dialog::open_input_dialog(
            root, &title, "", "editor.table.studyTypedHint",
            "editor.table.studyClose", "editor.table.studyTypedCheck",
        );
        self.popup = Some(Popup::StudyTypedEntry {
            dialog, field, placeholder, snapshot, index, progress, value: String::new(),
        });
    }

    pub(in crate::app) fn on_study_typed(
        &mut self, root: &mut RenderRoot, id: WidgetId,
        dialog: DialogIds, field: Handle<TextEditor>,
        placeholder: Handle<Placeholder>, snapshot: Box<StudyDeckSnapshot>,
        index: usize, progress: TypedProgress, mut value: String,
        action: &ErasedAction,
    ) {
        if action.downcast_ref::<DialogAnswer>() == Some(&DialogAnswer::Closed) {
            root.remove_layer(dialog.layer);
            return;
        }
        if id == field.id() {
            match action.downcast_ref::<TextAction>() {
                Some(TextAction::Changed(text)) => {
                    value = text.clone();
                    placeholder.edit(root, |mut p| {
                        Placeholder::set_empty(&mut p, value.is_empty());
                    });
                }
                Some(TextAction::Entered(text)) => {
                    root.remove_layer(dialog.layer);
                    self.grade_study_typed(root, snapshot, index, progress, text);
                    return;
                }
                Some(TextAction::Cancelled) => {
                    root.remove_layer(dialog.layer);
                    return;
                }
                _ => {}
            }
            self.popup = Some(Popup::StudyTypedEntry {
                dialog, field, placeholder, snapshot, index, progress, value,
            });
            return;
        }
        let Some(answer) = dialog.answer(id, action) else {
            self.popup = Some(Popup::StudyTypedEntry {
                dialog, field, placeholder, snapshot, index, progress, value,
            });
            return;
        };
        root.remove_layer(dialog.layer);
        if answer == DialogAnswer::Confirm {
            self.grade_study_typed(root, snapshot, index, progress, &value);
        }
    }

    fn grade_study_typed(
        &mut self, root: &mut RenderRoot,
        snapshot: Box<StudyDeckSnapshot>, index: usize,
        mut progress: TypedProgress, response: &str,
    ) {
        if !self.revalidate_study_deck(&snapshot) {
            self.study_notice(root, &t("editor.table.studyPracticeChanged"));
            return;
        }
        let verdict = check_typed_answer(&snapshot.deck, index, response);
        let description = match verdict {
            TypedVerdict::Correct => {
                progress.correct += 1;
                progress.answered += 1;
                t("editor.table.studyTypedCorrect")
            }
            TypedVerdict::Incorrect => {
                progress.answered += 1;
                let answer = snapshot.deck.cards[index].accepted_answers.iter()
                    .map(|item| short(item)).collect::<Vec<_>>().join(" / ");
                t_with("editor.table.studyTypedWrong", &[("answer", &answer)])
            }
            TypedVerdict::NeedsContext => {
                progress.skipped += 1;
                t("editor.table.studyTypedNeedsContext")
            }
            TypedVerdict::OtherKnownSense => {
                progress.skipped += 1;
                t("editor.table.studyTypedOtherSense")
            }
            TypedVerdict::Empty | TypedVerdict::TooLong | TypedVerdict::Unavailable => {
                progress.skipped += 1;
                t("editor.table.studyTypedNoGrade")
            }
        };
        let (widget, dialog) = ConfirmDialog::new(
            &t("editor.table.studyTypedTitle"), &description,
            &t("editor.table.studyClose"),
            &t("editor.table.studyChoiceNext"),
        );
        root.add_layer(widget, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::StudyTypedResult {
            dialog, snapshot, index, progress,
        });
    }

    pub(in crate::app) fn on_study_typed_result(
        &mut self, root: &mut RenderRoot, id: WidgetId,
        dialog: DialogIds, snapshot: Box<StudyDeckSnapshot>,
        index: usize, progress: TypedProgress, action: &ErasedAction,
    ) {
        match feedback_answer(root, id, &dialog, action) {
            Some(answer) => {
                if answer == DialogAnswer::Confirm {
                    self.show_study_typed(root, snapshot, index + 1, progress);
                }
            }
            None => self.popup = Some(Popup::StudyTypedResult {
                dialog, snapshot, index, progress,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displayed_answer_is_bounded_without_changing_the_source() {
        assert_eq!(short(" hello \n world "), "hello world");
        assert_eq!(short(&"я".repeat(200)).chars().count(), 121);
    }
}
