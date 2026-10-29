use super::*;

impl TextEditor {
    pub fn selected(&self) -> String {
        self.selected_text().unwrap_or_default()
    }

    pub fn image_source_shown(&self) -> bool {
        self.doc.source.is_some()
    }

    pub fn inline_boxes(&self, line: usize) -> usize {
        self.doc.paragraph(line).decor().boxes.len()
    }

    pub fn brush_at(&self, offset: usize) -> Option<BrushIndex> {
        let (_, local, layout) = self.local(offset);
        layout?.lines().flat_map(|line| line.items().collect::<Vec<_>>()).find_map(|item| match item {
            masonry::parley::PositionedLayoutItem::GlyphRun(run) if run.run().text_range().contains(&local) => Some(run.style().brush.clone()),
            _ => None,
        })
    }
}

impl TextEditor {
    /// Neither a selected range nor an additional caret may carry Parley
    /// layout state into a different widget. Source Rope caching remains
    /// independent and is still guarded by the on-disk BLAKE3 hash.
    pub(crate) fn parley_trial_snapshot(&self) -> Option<super::super::LayoutTrialSnapshot> {
        if self.compose.is_some() || self.anchor != self.focus || !self.extra_cursors.is_empty() {
            return None;
        }
        self.doc.trial_snapshot(&self.styles)
    }

    pub(crate) fn add_secondary_caret_for_trial_test(this: &mut WidgetMut<'_, Self>, byte: usize) {
        this.widget.extra_cursors.push((byte, byte));
        this.ctx.request_layout();
    }

    pub(crate) fn with_parley_trial(mut self, snapshot: super::super::LayoutTrialSnapshot) -> Self {
        self.doc.trial_offer(snapshot);
        self
    }
}


#[test]
fn pair_typing_and_backspace_at_an_unloaded_caret_keep_exact_rope_bytes() {
    let original = format!("()а)\n{}", "строка с юникодом 🐈\n".repeat(55_000));
    let mut editor = TextEditor::windowed(crop::Rope::from(original.as_str()));
    editor.doc.include(original.len());
    assert!(editor.doc.loaded().0 > 1, "caret must be before the mounted window");

    // Original failure: before(1) would calculate 1 - base with unsigned
    // underflow, and after(1) could panic while deleting a matched pair.
    editor.anchor = 1;
    editor.focus = 1;
    assert!(editor.handle_pair_backspace());
    let without_pair = original.replacen("()", "", 1);
    assert_eq!(editor.doc.whole(), without_pair);

    // The closing delimiter is already present. Typing it must skip the
    // actual character in the persistent Rope rather than duplicating it.
    editor.doc.include(editor.doc.len());
    assert!(editor.doc.loaded().0 > 2);
    editor.anchor = "а".len();
    editor.focus = "а".len();
    assert!(editor.handle_pair_typing(")"));
    assert_eq!(editor.focus, "а)".len());
    assert_eq!(editor.doc.whole(), without_pair);

    // A quote after Cyrillic text must NOT auto-insert a second quote, even
    // though the text to the left is outside the currently loaded window.
    editor.doc.include(editor.doc.len());
    editor.anchor = "а".len();
    editor.focus = "а".len();
    assert!(!editor.handle_pair_typing("'"));
    assert_eq!(editor.doc.whole(), without_pair);
}
