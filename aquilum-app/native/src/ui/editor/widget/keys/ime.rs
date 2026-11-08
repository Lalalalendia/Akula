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
        self.invalidate_ime_region();
        ctx.set_handled();
        if edited {
            self.changed(ctx);
        } else {
            ctx.request_layout();
            ctx.request_render();
        }
    }
}

impl TextEditor {
    /// Invalidate all temporarily folded plain-text paragraphs, including on
    /// cancellation, so the original height and cached layouts are restored.
    fn invalidate_ime_region(&mut self) {
        let range = self.selection();
        let first = self.doc.at_offset(range.start);
        let last = self.doc.at_offset(range.end);
        if self.doc.markdown.is_none() && last > first && last - first <= 64 && range.end - range.start <= 8192 {
            for i in first..=last {
                let at = self.doc.paragraph(i).start;
                self.doc.invalidate(at);
            }
        } else {
            self.doc.invalidate(range.start);
            if range.start != self.focus {
                self.doc.invalidate(self.focus);
            }
        }
    }

    /// Blur cancels transient composition without touching document bytes or Undo.
    pub(in crate::ui::editor::widget) fn cancel_transient_ime(&mut self) -> bool {
        if self.compose.take().is_none() {
            return false;
        }
        self.invalidate_ime_region();
        true
    }

    pub(in crate::ui::editor::widget) fn blur_editor(&mut self) {
        self.hide_suggest();
        self.cancel_transient_ime();
    }

    pub(in crate::ui::editor::widget) fn handle_window_focus_change(&mut self, ctx: &mut EventCtx<'_>, focused: bool) {
        if !focused && self.cancel_transient_ime() {
            ctx.request_layout();
        }
        ctx.request_render();
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;

    #[test]
    fn blurring_ime_clears_only_transient_preedit_and_never_changes_selected_unicode() {
        let original = "до 世界 после";
        let mut editor = TextEditor::new(original);
        let start = original.find("世界").unwrap();
        let end = start + "世界".len();
        editor.anchor = end;
        editor.focus = start;
        editor.compose = Some(("日本".into(), Some((6, 6))));
        assert!(editor.cancel_transient_ime());
        assert!(editor.compose.is_none());
        assert_eq!(editor.doc.copy(start..end), "世界");
        assert_eq!(editor.selection(), start..end);
        assert!(!editor.cancel_transient_ime(), "repeated blur must be idempotent");
        assert_eq!(editor.text(), original);
    }
}
