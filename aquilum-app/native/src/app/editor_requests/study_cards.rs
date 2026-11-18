//! LANG-05a: opt-in, temporary native flashcards from selected collections.
//!
//! No answer history, SRS, copied vocabulary, background indexing or direct
//! file reads. Every reveal/advance revalidates the entire host-authorized
//! source set and atomic study metadata revision; if anything changed the
//! exercise ends instead of showing stale content.

use std::collections::BTreeSet;
use aquilum_core::learning::state::StudyStore;
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::Point;

use super::super::{App, Popup};
use super::study_load::StudyDeckSnapshot;
use crate::i18n::{t, t_with};
use crate::ui::dialog::{ConfirmDialog, DialogAnswer, DialogIds};
use crate::ui::menu::MenuChoice;

const MAX_PICKER_COLLECTIONS: usize = 64;
const MAX_CARD_TEXT: usize = 256;

fn compact(text: &str) -> String {
    let trimmed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.chars().count() > MAX_CARD_TEXT {
        trimmed.chars().take(MAX_CARD_TEXT).collect::<String>() + "…"
    } else {
        trimmed
    }
}

impl App {
    pub(in crate::app) fn open_study_picker(&mut self, root: &mut RenderRoot) {
        let loaded = StudyStore::open(
            &self.workspace.data_dir.join("language-study.sqlite"),
        ).and_then(|store| store.load());
        let Ok(saved) = loaded else {
            self.study_notice(root, &t("editor.table.studySaveFailed"));
            return;
        };
        let choices: Vec<(String, String)> = saved.state.registry.collections.into_iter()
            .filter(|c| !c.members.is_empty())
            .map(|c| (c.id, c.name))
            .collect();
        if choices.is_empty() {
            self.study_notice(root, &t("editor.table.studyPracticeEmpty"));
            return;
        }
        if choices.len() > MAX_PICKER_COLLECTIONS {
            self.study_notice(root, &t("editor.table.studyPracticeTooMany"));
            return;
        }
        self.show_study_picker(root, choices, BTreeSet::new());
    }

    fn show_study_picker(
        &mut self, root: &mut RenderRoot, choices: Vec<(String, String)>,
        selected: BTreeSet<String>,
    ) {
        let mut entries: Vec<(usize, String)> = choices.iter().enumerate()
            .map(|(i, (id, name))| (i, format!("{} {name}",
                if selected.contains(id) { "☑" } else { "☐" }))).collect();
        entries.push((entries.len(), t_with(
            "editor.table.studyPracticeStart",
            &[("count", &selected.len().to_string())],
        )));
        entries.push((entries.len(), t("editor.table.studyChoiceStart")));
        let layer = self.study_menu(root, &t("editor.table.studyPracticeSelect"), &entries);
        self.popup = Some(Popup::StudyPicker { layer, choices, selected });
    }

    pub(in crate::app) fn on_study_picker(
        &mut self, root: &mut RenderRoot, layer: WidgetId,
        choices: Vec<(String, String)>, mut selected: BTreeSet<String>,
        action: &ErasedAction,
    ) {
        let Some(choice) = action.downcast_ref::<MenuChoice>() else {
            self.popup = Some(Popup::StudyPicker { layer, choices, selected });
            return;
        };
        let MenuChoice::Item(index) = choice else {
            if matches!(choice, MenuChoice::Hover(_)) {
                self.popup = Some(Popup::StudyPicker { layer, choices, selected });
            }
            return;
        };
        root.remove_layer(layer);
        if *index == choices.len() || *index == choices.len() + 1 {
            if selected.is_empty() {
                self.study_notice(root, &t("editor.table.studyPracticeSelectOne"));
                return;
            }
            let ids: Vec<String> = selected.into_iter().collect();
            match self.load_study_deck(&ids) {
                Ok(deck) if !deck.deck.cards.is_empty() => {
                    if *index == choices.len() + 1 { self.show_study_choice(root, Box::new(deck), 0, Default::default()); }
                    else { self.show_study_card(root, Box::new(deck), 0, false); }
                }
                Ok(_) => self.study_notice(root, &t("editor.table.studyPracticeNoCards")),
                Err(_) => self.study_notice(root, &t("editor.table.studyPracticeChanged")),
            }
        } else if let Some((id, _)) = choices.get(*index) {
            if !selected.remove(id) { selected.insert(id.clone()); }
            self.show_study_picker(root, choices, selected);
        }
    }

    fn show_study_card(
        &mut self, root: &mut RenderRoot,
        snapshot: Box<StudyDeckSnapshot>, index: usize, revealed: bool,
    ) {
        if !self.revalidate_study_deck(&snapshot) {
            self.study_notice(root, &t("editor.table.studyPracticeChanged"));
            return;
        }
        let Some(card) = snapshot.deck.cards.get(index) else {
            self.study_notice(root, &t("editor.table.studyPracticeFinished"));
            return;
        };
        let position = (index + 1).to_string();
        let total = snapshot.deck.cards.len().to_string();
        let mut message = t_with("editor.table.studyPracticeProgress",
            &[("index", &position), ("total", &total)]);
        message.push_str("\n\n");
        message.push_str(&compact(&card.question));
        if let Some(context) = &card.context {
            message.push_str("\n\n");
            message.push_str(&compact(context));
        }
        if revealed {
            message.push_str("\n\n");
            message.push_str(&t("editor.table.studyPracticeAnswer"));
            message.push('\n');
            message.push_str(&card.accepted_answers.iter()
                .map(|s| compact(s)).collect::<Vec<_>>().join(" / "));
        }
        let (widget, dialog) = ConfirmDialog::new(
            &t("editor.table.studyPracticeTitle"),
            &message,
            &t("editor.table.studyClose"),
            &t(if revealed { "editor.table.studyPracticeNext" }
                else { "editor.table.studyPracticeReveal" }),
        );
        root.add_layer(widget, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::StudyCard { dialog, snapshot, index, revealed });
    }

    pub(in crate::app) fn on_study_card(
        &mut self, root: &mut RenderRoot, id: WidgetId, dialog: DialogIds,
        snapshot: Box<StudyDeckSnapshot>, index: usize, revealed: bool,
        action: &ErasedAction,
    ) {
        if action.downcast_ref::<DialogAnswer>() == Some(&DialogAnswer::Closed) {
            root.remove_layer(dialog.layer);
            return;
        }
        match dialog.answer(id, action) {
            Some(answer) => {
                root.remove_layer(dialog.layer);
                if answer != DialogAnswer::Confirm { return; }
                // show_study_card performs the single final pinned source
                // revalidation immediately before rendering the next state.
                let next = if revealed { index + 1 } else { index };
                if next >= snapshot.deck.cards.len() {
                    self.study_notice(root, &t("editor.table.studyPracticeFinished"));
                } else {
                    self.show_study_card(root, snapshot, next, !revealed && next == index);
                }
            }
            None => self.popup = Some(Popup::StudyCard {
                dialog, snapshot, index, revealed,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_preview_never_exposes_more_than_bounded_text() {
        assert_eq!(compact("  multiple \n lines  "), "multiple lines");
        assert_eq!(compact(&"я".repeat(400)).chars().count(), MAX_CARD_TEXT + 1);
    }
}
