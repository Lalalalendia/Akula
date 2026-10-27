use super::*;

impl App {
    pub(super) fn toggle_reader_contents(&mut self, root: &mut RenderRoot, state: &mut ReaderState) {
        state.bindings.clear();
        root.edit_widget(state.layer, |mut widget| {
            let mut view = widget.downcast();
            ReaderView::set_popover(&mut view, None);
            ReaderView::toggle_contents(&mut view);
        });
        root.focus_on(Some(state.layer));
    }
}
