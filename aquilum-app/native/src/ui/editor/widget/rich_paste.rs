use super::*;

impl TextEditor {
    /// Preserve literal input for code fences, tables and single-line fields.
    /// Returning false allows plain clipboard handling to run unchanged.
    pub fn paste_markdown(this: &mut WidgetMut<'_, Self>, text: &str) -> bool {
        let widget = &mut *this.widget;
        let in_code = widget.doc.paragraph(widget.doc.at_offset(widget.focus)).block.is_code();
        if widget.single_line || widget.doc.markdown.is_none() || widget.table_active() || in_code {
            return false;
        }
        Self::insert_text(this, text);
        true
    }
}
