use masonry::core::{NewWidget, PropertySet, WidgetId};
use masonry::layout::Length;
use masonry::properties::types::{CrossAxisAlignment, MainAxisAlignment};
use masonry::properties::{Gap, Padding};
use masonry::widgets::{Flex, SizedBox};

mod toolbar;
pub(super) use toolbar::{Toolbar, toolbar, filtered_empty_state};

use super::model::Row;
use crate::i18n::t;
use crate::ui::editor::TextEditor;
use crate::ui::handle::Handle;
use crate::ui::tokens::size;
use crate::ui::widgets::{IconView, Label, Leading, TextButton, Variant};
use crate::ui::{icons, style, theme};

pub(super) fn empty_state() -> (NewWidget<Flex>, WidgetId) {
    let icon = IconView::new(icons::FILE_TEXT, size::SIZE_40, size::SIZE_28, 1.4, || theme::current().icon_muted);
    let title = Label::new(&t("fileTree.empty"), |t| t.text_secondary)
        .leading(Leading::Times(crate::ui::tokens::number::FONT_LINE_HEIGHT_SNUG as f32));
    let title = NewWidget::new(title);
    let button = NewWidget::new(TextButton::new(t("editor.createNote"), Variant::Primary));
    let id = button.id();
    let state = Flex::column()
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .with_fixed(icon)
        .with_fixed(title)
        .with_fixed(button);
    let state = NewWidget::new(state).with_props(
        PropertySet::new()
            .with(Gap::new(Length::px(size::EMPTY_STATE_GAP)))
            .with(Padding::from_vh(Length::px(size::SPACE_16), Length::px(size::SPACE_8))),
    );
    (state, id)
}

pub(super) fn rename_field(row: &Row) -> (NewWidget<Flex>, Handle<TextEditor>) {
    let indent = size::SIDEBAR_ITEM_PADDING
        + row.depth as f64 * size::SIDEBAR_TREE_INDENT
        + size::SIDEBAR_ICON_SIZE
        + size::SIDEBAR_ITEM_GAP;
    let name = if row.folder {
        row.name.clone()
    } else {
        row.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    };
    let field = NewWidget::new(TextEditor::single_line(&name).styled(|| style::ui_text(size::SIDEBAR_ICON_SIZE as f32)))
        .with_class(style::SIDEBAR_FIELD);
    let handle = Handle::of(&field);
    let row = Flex::row().with_fixed(NewWidget::new(SizedBox::empty().width(Length::px(indent)))).with(field, 1.0);
    let row = NewWidget::new(row).with_props(PropertySet::new().with(Padding {
        top: Length::px(size::SIDEBAR_ITEM_PADDING),
        bottom: Length::px(size::SIDEBAR_ITEM_PADDING),
        left: Length::ZERO,
        right: Length::px(size::SIDEBAR_ITEM_PADDING),
    }));
    (row, handle)
}
