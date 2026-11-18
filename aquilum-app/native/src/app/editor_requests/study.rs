//! Read-only user-facing study-card preview for the active native table.
//! No membership persistence, file writes, plugin authority or network calls.

use aquilum_core::learning::editor_snapshot::EditorTableSnapshot;
use aquilum_core::learning::{ColumnMapping, preview_table};
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::{Point, Rect};

use super::super::{App, Popup};
use crate::i18n::{t, t_with};
use crate::ui::dialog::{ConfirmDialog, DialogAnswer};
use crate::ui::menu::{Menu, MenuChoice};

const PREVIEW_ROWS: usize = 64;
const SHOWN_EXAMPLES: usize = 5;
const SHOWN_CHARS: usize = 72;

fn compact(value: &str) -> String {
    let mut visible = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if visible.chars().count() > SHOWN_CHARS {
        visible = visible.chars().take(SHOWN_CHARS).collect::<String>() + "…";
    }
    visible
}

fn labels(snapshot: &EditorTableSnapshot, skip: Option<usize>) -> Vec<(usize, String)> {
    snapshot.cells[0].iter().enumerate().filter_map(|(index, header)| {
        if skip == Some(index) {
            return None;
        }
        let value = compact(header);
        let value = if value.is_empty() { t("editor.table.studyUnnamedColumn") } else { value };
        Some((index, format!("{} · {}", index + 1, value)))
    }).collect()
}

fn preview_text(
    snapshot: &EditorTableSnapshot,
    question: usize,
    answer: usize,
) -> Option<(String, Option<String>)> {
    let mapping = ColumnMapping { question, answers: vec![answer], context: None };
    let result = preview_table(&snapshot.cells, &mapping, PREVIEW_ROWS).ok()?;
    if result.items.is_empty() {
        return Some((t("editor.table.studyNoPairs"), None));
    }
    let count = result.items.len().to_string();
    let skipped = result.skipped.len().to_string();
    let mut message = t_with(
        "editor.table.studySummary",
        &[("count", &count), ("skipped", &skipped)],
    );
    for item in result.items.iter().take(SHOWN_EXAMPLES) {
        message.push_str(&format!(
            "\n\n{} → {}",
            compact(&item.question),
            compact(&item.accepted_answers[0]),
        ));
    }
    if result.truncated {
        message.push_str(&format!("\n\n{}", t("editor.table.studyTruncated")));
    }
    message.push_str(&format!("\n\n{}", t("editor.table.studyTemporary")));
    let text = result.items.iter().map(|item| {
        format!("{}\t{}", item.question, item.accepted_answers[0])
    }).collect::<Vec<_>>().join("\n");
    Some((message, Some(text)))
}

impl App {
    fn study_menu(&mut self, root: &mut RenderRoot, title: &str, entries: &[(usize, String)]) -> WidgetId {
        let labels = entries.iter().map(|(_, label)| label.as_str()).collect::<Vec<_>>();
        let center = self.window.width * 0.5;
        let top = (self.window.height * 0.25).max(16.0);
        let trigger = Rect::new((center - 95.0).max(8.0), top, (center + 95.0).max(198.0), top + 28.0);
        let menu = Menu::list(title, &labels, 0, trigger);
        let layer = menu.id();
        root.add_layer(menu, Menu::list_origin(trigger, labels.len(), self.window));
        root.focus_on(Some(layer));
        layer
    }

    pub(in crate::app) fn open_study_question(&mut self, root: &mut RenderRoot, snapshot: EditorTableSnapshot) {
        if snapshot.cells[0].len() < 2 {
            self.study_notice(root, &t("editor.table.studyNeedColumns"));
            return;
        }
        let entries = labels(&snapshot, None);
        let layer = self.study_menu(root, &t("editor.table.studyQuestion"), &entries);
        self.popup = Some(Popup::StudyQuestion { layer, snapshot });
    }

    pub(in crate::app) fn on_study_question(
        &mut self, root: &mut RenderRoot, layer: WidgetId,
        snapshot: EditorTableSnapshot, action: &ErasedAction,
    ) {
        let Some(choice) = action.downcast_ref::<MenuChoice>() else {
            self.popup = Some(Popup::StudyQuestion { layer, snapshot });
            return;
        };
        let MenuChoice::Item(index) = choice else {
            if matches!(choice, MenuChoice::Hover(_)) {
                self.popup = Some(Popup::StudyQuestion { layer, snapshot });
            }
            return;
        };
        root.remove_layer(layer);
        let columns = labels(&snapshot, None);
        let Some((question, _)) = columns.get(*index) else { return };
        let question = *question;
        let entries = labels(&snapshot, Some(question));
        let layer = self.study_menu(root, &t("editor.table.studyAnswer"), &entries);
        self.popup = Some(Popup::StudyAnswer { layer, snapshot, question });
    }

    pub(in crate::app) fn on_study_answer(
        &mut self, root: &mut RenderRoot, layer: WidgetId,
        snapshot: EditorTableSnapshot, question: usize, action: &ErasedAction,
    ) {
        let Some(choice) = action.downcast_ref::<MenuChoice>() else {
            self.popup = Some(Popup::StudyAnswer { layer, snapshot, question });
            return;
        };
        let MenuChoice::Item(index) = choice else {
            if matches!(choice, MenuChoice::Hover(_)) {
                self.popup = Some(Popup::StudyAnswer { layer, snapshot, question });
            }
            return;
        };
        root.remove_layer(layer);
        let choices = labels(&snapshot, Some(question));
        let Some((answer, _)) = choices.get(*index) else { return };
        match preview_text(&snapshot, question, *answer) {
            Some((description, copy)) => self.study_dialog(root, &description, copy),
            None => self.study_notice(root, &t("editor.table.studyInvalidMapping")),
        }
    }

    pub(in crate::app) fn study_notice(&mut self, root: &mut RenderRoot, message: &str) {
        self.study_dialog(root, message, None);
    }

    fn study_dialog(&mut self, root: &mut RenderRoot, message: &str, copy: Option<String>) {
        let copy_label = if copy.is_some() { t("editor.table.studyCopy") } else { t("common.ok") };
        let (widget, dialog) = ConfirmDialog::new(
            &t("editor.table.studyTitle"), message, &t("common.close"), &copy_label,
        );
        root.add_layer(widget, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::StudyPreview { dialog, copy });
    }

    pub(in crate::app) fn on_study_preview(
        &mut self, root: &mut RenderRoot, id: WidgetId,
        dialog: crate::ui::dialog::DialogIds, copy: Option<String>, action: &ErasedAction,
    ) {
        if action.downcast_ref::<DialogAnswer>() == Some(&DialogAnswer::Closed) {
            root.remove_layer(dialog.layer);
            return;
        }
        match dialog.answer(id, action) {
            Some(answer) => {
                crate::ui::dialog::close(root, dialog.layer);
                if answer == DialogAnswer::Confirm
                    && let Some(copy) = copy
                    && let Ok(mut clipboard) = arboard::Clipboard::new()
                {
                    let _ = clipboard.set_text(copy);
                }
            }
            None => self.popup = Some(Popup::StudyPreview { dialog, copy }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::learning::editor_snapshot::capture;

    #[test]
    fn reverse_mapping_is_explicit() {
        let cells = vec![
            vec!["EN".into(), "RU".into()], vec!["eat".into(), "есть".into()],
            vec!["run".into(), "бегать".into()],
        ];
        let snap = capture(&cells, 0..20, 20, false).unwrap();
        let (_, forward) = preview_text(&snap, 0, 1).unwrap();
        let (_, reverse) = preview_text(&snap, 1, 0).unwrap();
        assert_eq!(forward.as_deref(), Some("eat\tесть\nrun\tбегать"));
        assert_eq!(reverse.as_deref(), Some("есть\teat\nбегать\trun"));
    }

    #[test]
    fn arbitrary_term_and_genre_columns_are_supported() {
        let cells = vec![vec!["Термин".into(), "Определение".into(), "Жанр".into()],
                         vec!["Сянься".into(), "Культивация".into(), "Фэнтези".into()]];
        let snap = capture(&cells, 5..32, 33, false).unwrap();
        assert_eq!(labels(&snap, Some(1)).iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 2]);
        let (_, value) = preview_text(&snap, 2, 0).unwrap();
        assert_eq!(value.as_deref(), Some("Фэнтези\tСянься"));
    }

    #[test]
    fn incomplete_answer_is_skipped() {
        let cells = vec![
            vec!["Q".into(), "A".into()], vec!["run".into(), "".into()],
            vec!["eat".into(), "есть".into()],
        ];
        let snap = capture(&cells, 0..15, 15, false).unwrap();
        let (_, text) = preview_text(&snap, 0, 1).unwrap();
        assert_eq!(text.as_deref(), Some("eat\tесть"));
    }

    #[test]
    fn display_bound_does_not_truncate_copied_text() {
        let long = "я".repeat(90);
        let cells = vec![vec!["Q".into(), "A".into()], vec!["question".into(), long.clone()]];
        let snap = capture(&cells, 0..20, 20, false).unwrap();
        let (notice, text) = preview_text(&snap, 0, 1).unwrap();
        assert!(notice.contains('…'));
        assert_eq!(text.as_deref(), Some(format!("question\t{long}").as_str()));
    }
}
