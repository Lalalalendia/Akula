use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use crate::ui::editor::{TextAction, TextEditor};
use crate::ui::search::Placeholder;
use crate::ui::scroll::ScrollArea;
use crate::ui::widgets::Pressed;
use super::FileTree;

impl FileTree {
    pub(super) fn on_filter_action(&mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction) -> bool {
        let Some(toolbar) = self.toolbar.as_ref() else { return false };
        let filter = toolbar.filter.id();
        let clear = toolbar.clear.id();
        if id == filter {
            if let Some(text) = action.downcast_ref::<TextAction>() {
                match text {
                    TextAction::Changed(query) => self.set_filter(root, query),
                    TextAction::Cancelled => self.filter(root, ""),
                    TextAction::Entered(_) | TextAction::Edited(_) | TextAction::Page { .. } | TextAction::Line { .. } => {}
                }
                return true;
            }
        }
        if id == clear && action.downcast_ref::<Pressed>().is_some() {
            self.filter(root, "");
            root.focus_on(Some(filter));
            return true;
        }
        false
    }

    pub fn filter(&mut self, root: &mut RenderRoot, query: &str) {
        if let Some(toolbar) = self.toolbar.as_ref() {
            toolbar.filter.edit(root, |mut field| TextEditor::reset_text(&mut field, query));
        }
        self.set_filter(root, query);
    }

    pub(super) fn set_filter(&mut self, root: &mut RenderRoot, query: &str) {
        if let Some(toolbar) = self.toolbar.as_ref() {
            toolbar.placeholder.edit(root, |mut placeholder| Placeholder::set_empty(&mut placeholder, query.is_empty()));
        }
        self.model.set_filter(query);
        if !self.model.filtering() {
            self.indexing = None;
        } else if !self.model.indexed() && self.indexing.is_none() {
            self.reindex();
        }
        self.first = 0;
        self.offset = 0.0;
        self.refresh(root);
        if let Some(scroll) = self.scroll {
            scroll.edit(root, |mut scroll| ScrollArea::set_offset(&mut scroll, 0.0));
        }
    }
}
