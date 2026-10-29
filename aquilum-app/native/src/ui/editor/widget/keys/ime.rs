use super::*;

impl TextEditor {
    pub(in crate::ui::editor::widget) fn ime(&mut self, ctx: &mut EventCtx<'_>, ime: &Ime) {
        let mut edited = false;
        match ime {
            Ime::Enabled => {}
            Ime::Disabled => self.compose = None,
            Ime::Preedit(text, _) if text.is_empty() => self.compose = None,
            Ime::Preedit(text, cursor) => {
                // Preedit is visual only: never edit the selection or create Undo.
                self.compose = Some((text.clone(), *cursor));
            }
            // An empty commit (including cancellation from some OS IMEs)
            // must never erase the selected text or create an Undo entry.
            Ime::Commit(text) if text.is_empty() => self.compose = None,
            Ime::Commit(text) => {
                self.compose = None;
                self.insert(text, Kind::Other);
                edited = true;
            }
        }
        if edited {
            self.update_suggest(ctx.widget_id());
        }
        // Invalidate the visual preedit origin as well as the live caret.
        // They differ for forward and multiline selections.
        let at = self.selection().start;
        self.doc.invalidate(at);
        if at != self.focus {
            self.doc.invalidate(self.focus);
        }
        ctx.set_handled();
        if edited {
            self.changed(ctx);
        } else {
            ctx.request_layout();
            ctx.request_render();
        }
    }
}
