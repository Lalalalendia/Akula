use super::*;

impl TextEditor {
    pub fn text(&self) -> String {
        self.doc.whole()
    }

    pub fn len(&self) -> usize {
        self.doc.len()
    }

    pub fn rope(&self) -> crop::Rope {
        self.doc.rope()
    }

    #[cfg(test)]
    pub fn caret_rect(&self) -> masonry::kurbo::Rect {
        self.caret()
    }

    pub fn caret_offset(&self) -> usize {
        self.selection().start
    }

    pub fn offset_at(&self, window_to_local: Affine, window: Point) -> usize {
        self.hit(window_to_local * window).0
    }

    pub fn anchor_at(&self, y: f64) -> Option<(usize, f64)> {
        if !self.laid_out() {
            return None;
        }
        let i = self.doc.at_y(y.max(0.0));
        Some((self.doc.paragraph(i).start, y - self.doc.top(i)))
    }

    pub fn top_of(&self, offset: usize) -> Option<f64> {
        self.laid_out().then(|| self.doc.y_of(offset.min(self.doc.len())))
    }

    fn repaint(this: &mut WidgetMut<'_, Self>) {
        this.ctx.request_layout();
        this.ctx.request_render();
    }

    fn edited(this: &mut WidgetMut<'_, Self>) {
        this.ctx.submit_action::<TextAction>(this.widget.doc.changed());
        Self::repaint(this);
    }

    pub fn refresh_embeds(this: &mut WidgetMut<'_, Self>) {
        let id = this.ctx.widget_id();
        this.widget.poll_suggest(id);
        Self::repaint(this);
    }

    pub fn reset_text(this: &mut WidgetMut<'_, Self>, text: &str) {
        Self::reset_rope(this, crop::Rope::from(text));
    }

    pub fn reset_rope(this: &mut WidgetMut<'_, Self>, text: crop::Rope) {
        // Reset replaces the entire source identity. Even when the next note
        // has a fence at the same byte offset, it must receive a NEW WidgetId.
        // Remove existing child widgets from Masonry's arena rather than only
        // clearing the Rust vector and leaving ghost accessibility nodes.
        for button in std::mem::take(&mut this.widget.access_code_buttons) {
            this.ctx.remove_child(button.widget);
        }
        this.widget.access_code_window_size = None;
        let widget = &mut *this.widget;
        widget.videos.stop();
        widget.doc.set_rope(text);
        widget.history.clear();
        widget.refind(&[]);
        widget.table_scrolls.clear();
        widget.hovered_code_copy = None;
        widget.copied_code = None;
        widget.code_scrolls.clear();
        widget.code_window_base = None;
        widget.code_window_opener = None;
        widget.code_last_viewport = None;
        widget.compose = None;
        widget.extra_cursors.clear();
        widget.media = Default::default();
        (widget.table, widget.hovered_table, widget.hovered_link, widget.unit) = (None, None, None, None);
        widget.anchor = widget.doc.floor(widget.anchor);
        widget.focus = widget.doc.floor(widget.focus);
        Self::repaint(this);
    }

    pub fn select_byte_range(this: &mut WidgetMut<'_, Self>, start: usize, end: usize) {
        let widget = &mut *this.widget;
        widget.anchor = widget.doc.floor(start);
        widget.focus = widget.doc.floor(end);
        widget.column = None;
        Self::repaint(this);
    }

    pub fn insert_block(this: &mut WidgetMut<'_, Self>, at: usize, lines: &[String]) {
        let at = at.min(this.widget.doc.len());
        this.widget.doc.include(at);
        let Range { start: line_start, end: line_end } = this.widget.doc.line_at(at);
        let prefix = if at == line_start { "" } else { "\n" };
        let tail = if this.widget.doc.slice(at..line_end).trim().is_empty() { "" } else { "\n" };
        let insert = format!("{prefix}{}\n{tail}", lines.join("\n\n"));
        this.widget.replace(at..at, &insert, Kind::Other);
        Self::edited(this);
    }

    pub fn apply_external(this: &mut WidgetMut<'_, Self>, range: Range<usize>, insert: &str) {
        let widget = &mut *this.widget;
        let change = widget.apply_change(range, insert.to_owned());
        (widget.anchor, widget.focus) = (change.map(widget.anchor), change.map(widget.focus));
        Self::repaint(this);
    }

    pub fn insert_text(this: &mut WidgetMut<'_, Self>, text: &str) {
        let range = this.widget.selection();
        this.widget.replace(range, text, Kind::Other);
        Self::edited(this);
    }
}
