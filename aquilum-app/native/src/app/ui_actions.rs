//! Common application UI transitions. File admission and persistence stay with callers.
use masonry::app::RenderRoot;
use masonry::core::{NewWidget, WidgetId};
use masonry::kurbo::Point;
use masonry::properties::types::CrossAxisAlignment;
use masonry::widgets::Flex;
use crate::ui::{dialog::{self, Modal}, editor::TextEditor, handle::Handle, menu::Menu, scroll::ScrollArea, widgets::{Caption, Font, IconButton, Label, Leading}};
use super::App;

pub(super) fn mount_modal(root: &mut RenderRoot, modal: NewWidget<Modal>, focus: WidgetId) -> WidgetId {
    let layer = modal.id();
    root.add_layer(modal, Point::ORIGIN);
    root.focus_on(Some(focus));
    layer
}

pub(super) fn mount_menu(root: &mut RenderRoot, menu: NewWidget<Menu>, origin: Point, button: Option<WidgetId>) -> WidgetId {
    let layer = menu.id();
    root.add_layer(menu, origin);
    if let Some(button) = button {
        root.edit_widget(button, |mut w| IconButton::set_pressed(&mut w.downcast(), true));
    }
    root.focus_on(Some(layer));
    layer
}

pub(super) fn focus_rename(root: &mut RenderRoot, field: Handle<TextEditor>) {
    field.edit(root, |mut field| {
        let len = field.widget.len();
        TextEditor::select_byte_range(&mut field, 0, len);
    });
    root.focus_on(Some(field.id()));
}

pub(super) fn readonly_text_page(title: &str, text: &str) -> NewWidget<ScrollArea<Flex>> {
    let heading = NewWidget::new(Caption::new(title));
    let body = NewWidget::new(Label::new(text, |theme| theme.text_primary)
        .font(Font::Editor).leading(Leading::Editor).selectable());
    let column = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(heading).with_fixed(body);
    NewWidget::new(ScrollArea::new(NewWidget::new(column)))
}

impl App {
    pub(super) fn close_picker(&self, root: &mut RenderRoot, layer: WidgetId) {
        dialog::close(root, layer);
        if let Some(ids) = &self.note_ids {
            root.focus_on(Some(ids.body.id()));
        }
    }

    pub(super) fn reset_note_view_for_preview(&mut self) {
        self.note = None;
        self.note_ids = None;
        self.embeds = None;
        self.open_error = None;
        self.open_error_ids = None;
    }
}
