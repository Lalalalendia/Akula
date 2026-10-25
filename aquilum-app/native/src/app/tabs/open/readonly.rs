use super::*;

impl App {
    /// DOCX L0: preserve the original package; no Note/TextEditor/save path.
    pub(super) fn show_docx_readonly(&mut self, root: &mut RenderRoot, path: &Path) {
        use masonry::core::NewWidget;
        use masonry::properties::types::CrossAxisAlignment;
        use masonry::widgets::Flex;
        use crate::ui::scroll::ScrollArea;
        use crate::ui::widgets::{Font, Label, Leading};
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

    /// First-stage format view: no TextEditor and no save path for structured files.
    /// Reopening always reparses from disk rather than reusing a stale tab cache.
    pub(super) fn show_structured_readonly(&mut self, root: &mut RenderRoot, path: &Path) {
        use aquilum_core::files::structured::{Format, StructuredDocument};
        use masonry::core::NewWidget;
        use masonry::widgets::Flex;
        self.note = None;
        self.note_ids = None;
        self.embeds = None;
        self.open_error = None;
        self.open_error_ids = None;
        let result = StructuredDocument::open(path).and_then(|doc| {
            let header = match doc.format() {
                Format::Canvas => {
                    let canvas = doc.canvas()?;
                    let nodes = canvas["nodes"].as_array().map_or(0, Vec::len);
                    let edges = canvas["edges"].as_array().map_or(0, Vec::len);
                    format!("JSON Canvas — {nodes} objects, {edges} connections")
                }
                Format::Base => "Obsidian Bases — YAML source".to_owned(),
            };
            Ok((header, doc.text().chars().take(2048).collect::<String>()))
        });
        match result {
            Ok((heading, preview)) => {
                let page = NewWidget::new(
                    Flex::column()
                        .with_fixed(NewWidget::new(crate::ui::widgets::Caption::new(
                            "Read-only preview — editing is not available yet",
                        )))
                        .with_fixed(NewWidget::new(crate::ui::widgets::Caption::new(&heading)))
                        .with_fixed(NewWidget::new(crate::ui::widgets::Caption::new(&preview))),
                );
                if let Some(chrome) = &self.chrome {
                    chrome.note_page.edit(root, |mut slot| Slot::set_child(&mut slot, page.erased()));
                }
            }
            Err(error) => self.show_open_error(root, path, &error.to_string()),
        }
    }

    pub(super) fn show_image_readonly(&mut self, root: &mut RenderRoot, path: &Path) {
        self.note = None;
        self.note_ids = None;
        self.embeds = None;
        self.open_error = None;
        self.open_error_ids = None;
        if let Some(page) = self.image_tab_widget() {
            if let Some(chrome) = &self.chrome {
                chrome.note_page.edit(root, |mut slot| Slot::set_child(&mut slot, page));
            }
        } else {
            self.show_open_error(root, path, &crate::i18n::t("fileTree.imageOpenFailed"));
        }
    }
}
