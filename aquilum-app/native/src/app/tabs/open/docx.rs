//! Lightweight read-only DOCX tab; never constructs an editable Note.
use super::*;
use masonry::core::NewWidget;
use masonry::properties::types::CrossAxisAlignment;
use masonry::widgets::Flex;
use crate::ui::scroll::ScrollArea;
use crate::ui::widgets::{Font, Label, Leading};

impl App {
    pub(super) fn show_docx_readonly(&mut self, root: &mut RenderRoot, path: &Path) {
        self.note = None;
        self.note_ids = None;
        self.embeds = None;
        self.open_error = None;
        self.open_error_ids = None;

        let result = self.tree.root().ok_or_else(|| "Не открыта база Akula".to_owned())
            .and_then(|vault| crate::docx::read(vault, path));
        match result {
            Ok(text) => {
                let filename = path.file_name().unwrap_or_default().to_string_lossy();
                let heading = NewWidget::new(crate::ui::widgets::Caption::new(&format!(
                    "{filename} — DOCX, только чтение"
                )));
                let body = NewWidget::new(Label::new(&text, |theme| theme.text_primary)
                    .font(Font::Editor).leading(Leading::Editor).selectable());
                let column = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_fixed(heading).with_fixed(body);
                let page = NewWidget::new(ScrollArea::new(NewWidget::new(column)));
                if let Some(chrome) = &self.chrome {
                    chrome.note_page.edit(root, |mut slot| Slot::set_child(&mut slot, page.erased()));
                }
            }
            Err(error) => self.show_open_error(root, path, &error),
        }
    }
}
