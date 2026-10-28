//! Table-cell text entry keeps the active IME composition range distinct
//! from the selection caret. Successive preedits replace, never append.

use masonry::kurbo::Point;
use masonry::parley::Cursor;

use super::{Outcome, Session};
use crate::ui::editor::table::view::{Cell, TableView};

pub(super) fn caret_in(cell: &Cell, value: &str, at: Point) -> usize {
    value.floor_char_boundary(Cursor::from_point(&cell.layout, at.x as f32, at.y as f32).index())
}

impl Session {
    pub fn insert(&mut self, view: &TableView, text: &str) -> Outcome {
        let text = text.replace(['\r', '\n', '\t'], " ");
        let composition = self.compose.take();
        let Some((r, c)) = self.active else { return Outcome::None };
        let mut model = view.model.clone();
        let mut value = model.value(r, c).to_owned();
        let selection = self.caret.min(self.anchor)..self.caret.max(self.anchor);
        let range = composition.unwrap_or(selection);
        let start = value.floor_char_boundary(range.start.min(value.len()));
        let end = value.floor_char_boundary(range.end.min(value.len()));
        value.replace_range(start..end, &text);
        model.set(r, c, &value);
        let caret = start + text.len();
        self.caret = caret.min(model.value(r, c).len());
        self.anchor = self.caret;
        Outcome::Write(model)
    }

    pub fn preedit(&mut self, view: &TableView, text: &str, cursor: Option<(usize, usize)>) -> Outcome {
        let normalized = text.replace(['\r', '\n', '\t'], " ");
        let outcome = self.insert(view, &normalized);
        if !matches!(outcome, Outcome::Write(_)) {
            return outcome;
        }
        let start = self.caret.saturating_sub(normalized.len());
        self.compose = (!normalized.is_empty()).then_some(start..self.caret);
        let requested = cursor.map_or(normalized.len(), |(at, _)| at.min(normalized.len()));
        self.caret = start + normalized.floor_char_boundary(requested);
        self.anchor = self.caret;
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::editor::table::tests::{blank_view, table};
    use crate::ui::editor::table::Table;

    fn written(outcome: Outcome) -> Table {
        match outcome {
            Outcome::Write(value) => value,
            _ => panic!("the table text edit must produce a document write"),
        }
    }

    #[test]
    fn preedit_updates_replace_previous_unicode_composition() {
        let original = table("| a | b |\n| --- | --- |\n| 1 | 2 |");
        let mut session = Session::new(0);
        session.enter((0, 0), 1);
        let first = written(session.preedit(&blank_view(&original), "こ", Some((3, 3))));
        assert_eq!(first.value(0, 0), "aこ");
        let second = written(session.preedit(&blank_view(&first), "候補", Some((3, 3))));
        assert_eq!(second.value(0, 0), "a候補");
        assert_eq!(session.compose, Some(1..7));
        assert_eq!(session.caret, 4, "the cursor remains inside the composed range");
        let committed = written(session.insert(&blank_view(&second), "漢字"));
        assert_eq!(committed.value(0, 0), "a漢字");
        assert_eq!(committed.value(0, 1), "b", "neighboring cells must not be edited");
        assert_eq!(session.compose, None);
        assert_eq!(session.caret, "a漢字".len());
    }

    #[test]
    fn empty_preedit_cancels_temporary_text_without_erasing_prior_content() {
        let original = table("| a | b |\n| --- | --- |\n| 1 | 2 |");
        let mut session = Session::new(0);
        session.enter((0, 0), 1);
        let first = written(session.preedit(&blank_view(&original), "字", None));
        let second = written(session.preedit(&blank_view(&first), "", None));
        assert_eq!(second.value(0, 0), "a");
        assert_eq!(session.compose, None);
    }

    #[test]
    fn moving_to_another_cell_discards_stale_composition_range() {
        let original = table("| a | b |\n| --- | --- |\n| 1 | 2 |");
        let mut session = Session::new(0);
        session.enter((0, 0), 1);
        let first = written(session.preedit(&blank_view(&original), "候補", None));
        session.enter((0, 1), 1);
        assert_eq!(session.compose, None);
        let second = written(session.insert(&blank_view(&first), "!"));
        assert_eq!(second.value(0, 0), "a候補");
        assert_eq!(second.value(0, 1), "b!");
    }
}
