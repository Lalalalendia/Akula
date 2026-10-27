//! Non-shipping read-only HTML preview using native Masonry text widgets.
//! No web engine, JavaScript execution, external images or remote resources.
use super::*;
use masonry::core::NewWidget;
use masonry::properties::types::CrossAxisAlignment;
use masonry::widgets::Flex;
use crate::ui::scroll::ScrollArea;
use crate::ui::widgets::{Font, Label, Leading};

impl App {
    pub(super) fn show_html_tab(&mut self, root: &mut RenderRoot, path: PathBuf) {
        self.leave_note();
        self.drop_note_page(root);
        self.show_html_readonly(root, &path);
        self.tree.set_active(root, Some(path));
        self.show_page(root, 0);
    }

    fn show_html_readonly(&mut self, root: &mut RenderRoot, path: &Path) {
        self.note = None;
        self.note_ids = None;
        self.embeds = None;
        self.open_error = None;
        self.open_error_ids = None;
        let result = self.tree.root()
            .ok_or_else(|| "Не открыта база Akula".to_owned())
            .and_then(|vault| crate::html_preview::read(vault, path));
        match result {
            Ok(text) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let heading = NewWidget::new(crate::ui::widgets::Caption::new(&format!(
                    "{name} — HTML, упрощённый просмотр (только чтение)"
                )));
                let body = NewWidget::new(Label::new(&text, |theme| theme.text_primary)
                    .font(Font::Editor).leading(Leading::Editor).selectable());
                let column = NewWidget::new(Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_fixed(heading).with_fixed(body));
                let page = NewWidget::new(ScrollArea::new(column));
                if let Some(chrome) = &self.chrome {
                    chrome.note_page.edit(root, |mut slot| Slot::set_child(&mut slot, page.erased()));
                }
            }
            Err(error) => {
                self.open_error = Some((path.to_path_buf(), error.clone()));
                self.show_open_error(root, path, &error);
            }
        }
    }
}
