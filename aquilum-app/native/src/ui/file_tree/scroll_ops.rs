use masonry::app::RenderRoot;

use super::{FileTree, MARGIN, SPAN, step};
use crate::ui::scroll::ScrollArea;
use crate::ui::tokens::size;

impl FileTree {
    pub(super) fn first_near(offset: f64) -> usize {
        ((offset / step()) as usize).saturating_sub(MARGIN)
    }

    pub(super) fn show_rows_from(&mut self, root: &mut RenderRoot, offset: f64) {
        let visible = (offset / step()) as usize;
        let first = Self::first_near(offset);
        if visible < self.first || first.abs_diff(self.first) > MARGIN / 2 {
            self.first = first;
            self.refresh(root);
        }
    }

    pub(super) fn show_row(&mut self, root: &mut RenderRoot, index: Option<usize>) {
        if let Some(index) = index
            && !(self.first..self.first + SPAN - MARGIN).contains(&index)
        {
            self.first = index.saturating_sub(MARGIN);
        }
        self.refresh(root);
        if let Some(index) = index {
            self.scroll_to(root, index);
        }
    }

    fn scroll_to(&mut self, root: &mut RenderRoot, index: usize) {
        let top = index as f64 * step();
        let bottom = top + step() - size::SPACE_2;
        if let Some(offset) = self.scroll.and_then(|scroll| {
            scroll.edit(root, |mut scroll| {
                ScrollArea::reveal(&mut scroll, top, bottom);
                scroll.widget.offset()
            })
        }) {
            self.offset = offset;
        }
    }
}
