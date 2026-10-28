//! Route IME composition to the active table cell rather than the document caret.
use masonry::core::Ime;

use super::*;

impl TextEditor {
    pub(in super::super) fn table_ime(&mut self, ctx: &mut EventCtx<'_>, ime: &Ime) -> bool {
        if !self.table.as_ref().is_some_and(|s| s.active.is_some()) {
            return false;
        }
        let Some((session, view)) = self.table_parts() else { return false };
        let outcome = match ime {
            Ime::Preedit(text, cursor) => session.preedit(view, text, *cursor),
            Ime::Commit(text) => session.insert(view, text),
            Ime::Enabled | Ime::Disabled => {
                session.compose = None;
                Outcome::None
            }
        };
        self.apply(ctx, outcome);
        ctx.set_handled();
        true
    }

    pub(in super::super) fn table_text(&mut self, ctx: &mut EventCtx<'_>, text: &str) -> bool {
        if !self.table.as_ref().is_some_and(|s| s.active.is_some()) {
            return false;
        }
        let Some((session, view)) = self.table_parts() else { return false };
        let outcome = session.insert(view, text);
        self.apply(ctx, outcome);
        ctx.set_handled();
        true
    }
}

#[cfg(test)]
mod tests {
    use masonry::core::{Ime, TextEvent};
    use crate::ui::editor::tests::support::{click, markdown_root, text, typing};

    #[test]
    fn composition_replaces_preedit_inside_clicked_cell_not_document_body() {
        let original = "| a | b |\n| --- | --- |\n| 1 | 2 |\nпосле";
        let mut root = markdown_root(original);
        click(&mut root, 60.0, 50.0);
        for (value, at) in [("こ", 3), ("候補", 6)] {
            root.handle_text_event(TextEvent::Ime(Ime::Preedit(value.into(), Some((at, at)))));
            assert_eq!(text(&root), format!("| a{value} | b |\n| --- | --- |\n| 1 | 2 |\nпосле"));
        }
        root.handle_text_event(TextEvent::Ime(Ime::Commit("漢字".into())));
        typing(&mut root, "!");
        assert_eq!(text(&root), "| a漢字! | b |\n| --- | --- |\n| 1 | 2 |\nпосле");
    }
}
