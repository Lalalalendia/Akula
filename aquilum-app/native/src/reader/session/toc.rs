use super::{Anchor, Session};
use crate::reader::{Point, TocEntry};

impl Session {
    pub fn contents(&self) -> &[TocEntry] {
        self.book.epub.as_ref().map_or(&[], |epub| epub.toc.as_slice())
    }

    pub fn jump_to_contents(&mut self, index: usize) -> bool {
        let Some(entry) = self.contents().get(index).cloned() else { return false };
        self.enter(entry.section, Anchor::Start);
        if let Some(node) = entry.fragment.as_deref().and_then(|id| self.book.document(entry.section)?.by_id(id))
            && let Some((block, byte)) = self.locate(Point::Before(node))
        {
            self.anchor = Anchor::At(block, byte);
            self.place();
        }
        true
    }
}
