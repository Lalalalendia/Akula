use super::*;
use crate::frontmatter::PAGES;
use crate::vault_files::{self, Source};
use masonry::app::RenderRoot;
use std::path::PathBuf;

impl App {
    pub(in crate::app) fn on_picked(&mut self, root: &mut RenderRoot, picked: Picked) -> bool {
        let current = |note: &PathBuf| self.note.as_ref().is_some_and(|n| n.is(note));
        match picked {
            Picked::Workspace(path) => return self.switch_workspace(root, path).title,
            Picked::PdfChosen(file) => self.prompt_pdf_preview(root, file),
            Picked::PdfIndexed(Ok(note)) => {
                self.tree.paths_changed(root, std::slice::from_ref(&note));
                return self.open(root, note);
            }
            Picked::PdfIndexed(Err(error)) => {
                report!("PDF не добавлен в поиск: {error}");
                self.show_pdf_notice(root, &error);
            }
            Picked::PageCover(note, file) if current(&note) => {
                if let Some(relative) = self.import_cover(Source::Path(&file)) {
                    self.set_fields(root, &[(COVER, "true".into()), (PAGE_COVER, relative), (PAGE_COVER_POSITION, String::new())]);
                }
            }
            Picked::BookCover(note, file) if current(&note) => {
                if let Some(relative) = self.import_cover(Source::Path(&file)) {
                    self.set_fields(root, &[(BOOK_COVER, relative)]);
                }
            }
            Picked::Book(note, file) => {
                self.set_book_busy(root, false);
                if let Some(file) = file.filter(|_| current(&note)) {
                    self.import_book(root, &file);
                }
            }
            Picked::PageCover(..) | Picked::BookCover(..) => {}
        }
        false
    }

}
