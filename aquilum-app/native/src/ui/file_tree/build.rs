use masonry::app::RenderRoot;
use masonry::core::{NewWidget, PropertySet};
use masonry::layout::Length;
use masonry::properties::types::CrossAxisAlignment;
use masonry::properties::{Gap, Padding};
use masonry::layout::UnitPoint;
use masonry::widgets::{ChildAlignment, Flex, SizedBox, ZStack};

use super::model::comparable;
use super::pinned::Pinned;
use super::pins::pins;
use super::parts::{empty_state, rename_field, toolbar};
use super::row::TreeRow;
use super::{FileTree, SPAN, step};
use crate::ui::editor::TextEditor;
use crate::ui::handle::Handle;
use crate::ui::scroll::ScrollArea;
use crate::ui::tokens::size;

fn spacer(rows: usize) -> NewWidget<SizedBox> {
    NewWidget::new(SizedBox::empty().height(Length::px(rows as f64 * step() - size::SPACE_2)))
}

impl FileTree {
    pub fn widget(&mut self) -> NewWidget<Flex> {
        let scroll = NewWidget::new(ScrollArea::new(self.list()).fill(true).at(self.offset));
        self.scroll = Some(Handle::of(&scroll));
        let pinned = NewWidget::new(Pinned::new(&self.pins(), self.model.rows()));
        self.pinned = Some(Handle::of(&pinned));
        let stack = ZStack::new().with(scroll, ChildAlignment::ParentAligned).with(pinned, UnitPoint::TOP_LEFT);
        let mut column = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch);
        self.toolbar = None;
        if self.model.root().is_some() {
            let (toolbar, ids) = toolbar();
            self.toolbar = Some(ids);
            column = column.with_fixed(toolbar);
        }
        NewWidget::new(column.with(NewWidget::new(stack), 1.0))
    }

    fn list(&mut self) -> NewWidget<Flex> {
        self.row_ids.clear();
        self.create_button = None;
        self.rename = None;
        #[cfg(all(windows, feature = "p2p-inbox-preview"))]
        let has_imports = !self.imported.names().is_empty();
        #[cfg(not(all(windows, feature = "p2p-inbox-preview")))]
        let has_imports = false;
        if self.model.root().is_some() && self.model.rows().is_empty() && !has_imports {
            let (state, button) = empty_state();
            self.create_button = Some(button);
            return state;
        }
        let mut list = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch);
        let renaming = self.model.renaming().map(comparable);
        let count = self.model.rows().len();
        self.first = self.first.min(count.saturating_sub(1));
        let last = (self.first + SPAN).min(count);
        if self.first > 0 {
            list = list.with_fixed(spacer(self.first));
        }
        for row in &self.model.rows()[self.first..last] {
            if renaming.as_deref() == Some(comparable(&row.path).as_str()) {
                let (field, rename) = rename_field(row);
                self.row_ids.push(field.id());
                self.rename = Some(rename);
                list = list.with_fixed(field);
                continue;
            }
            let widget = NewWidget::new(TreeRow::new(row.clone()));
            self.row_ids.push(widget.id());
            list = list.with_fixed(widget);
        }
        if last < count {
            list = list.with_fixed(spacer(count - last));
        }
        #[cfg(all(windows, feature = "p2p-inbox-preview"))]
        if let Some(verified) = super::imported::render(self.imported.names()) {
            list = list.with_fixed(verified);
        }
        NewWidget::new(list).with_props(
            PropertySet::new()
                .with(Gap::new(Length::px(size::SPACE_2)))
                .with(Padding {
                    top: Length::ZERO,
                    left: Length::px(size::SPACE_8),
                    right: Length::px(size::SPACE_8),
                    bottom: Length::ZERO,
                }),
        )
    }

    fn pins(&self) -> Vec<usize> {
        pins(self.model.rows(), self.offset, step())
    }

    pub(super) fn update_pins(&mut self, root: &mut RenderRoot) {
        if let Some(pinned) = self.pinned {
            let pins = self.pins();
            pinned.edit(root, |mut pinned| Pinned::set(&mut pinned, &pins, self.model.rows()));
        }
    }

    pub fn refresh(&mut self, root: &mut RenderRoot) {
        let editing = self.rename.and_then(|field| {
            let focused = root.focused_widget() == Some(field.id());
            field.get(root).map(|f| (f.inner().text(), f.inner().selection(), focused))
        });
        let list = self.list();
        if let Some(scroll) = self.scroll {
            scroll.edit(root, |mut scroll| ScrollArea::set_child(&mut scroll, list));
        }
        if let (Some((text, selection, focused)), Some(field)) = (editing, self.rename) {
            field.edit(root, |mut f| {
                TextEditor::reset_text(&mut f, &text);
                TextEditor::select_byte_range(&mut f, selection.start, selection.end);
            });
            if focused {
                root.focus_on(Some(field.id()));
            }
        }
        self.update_pins(root);
    }}
