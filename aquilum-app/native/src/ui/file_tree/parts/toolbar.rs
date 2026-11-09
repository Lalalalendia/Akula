use masonry::core::{NewWidget, PropertySet, WidgetId};
use masonry::layout::Length;
use masonry::properties::types::{CrossAxisAlignment, MainAxisAlignment};
use masonry::properties::{Gap, Padding};
use masonry::widgets::Flex;

use crate::i18n::t;
use crate::ui::editor::TextEditor;
use crate::ui::handle::Handle;
use crate::ui::search::{self, Placeholder};
use crate::ui::tokens::size;
use crate::ui::widgets::{IconButton, Label};
use crate::ui::icons;

pub(in crate::ui::file_tree) struct Toolbar {
    pub(in crate::ui::file_tree) new_note: WidgetId,
    pub(in crate::ui::file_tree) new_folder: WidgetId,
    pub(in crate::ui::file_tree) sort: WidgetId,
    pub(in crate::ui::file_tree) collapse: WidgetId,
    pub(in crate::ui::file_tree) filter: Handle<TextEditor>,
    pub(in crate::ui::file_tree) placeholder: Handle<Placeholder>,
    pub(in crate::ui::file_tree) clear: Handle<IconButton>,
}

pub(in crate::ui::file_tree) fn toolbar(query: &str) -> (NewWidget<Flex>, Toolbar) {
    let button = |icon, label: &str| NewWidget::new(IconButton::new(icon, t(label)).small());
    let new_note = button(icons::SQUARE_PEN, "fileTree.newNote");
    let new_folder = button(icons::FOLDER_PLUS, "fileTree.newFolder");
    let sort = button(icons::ARROW_UP_NARROW_WIDE, "fileTree.sort");
    let collapse = button(icons::CHEVRONS_DOWN_UP, "fileTree.collapseAll");
    let search = search::find_box_with_query(query, t("fileTree.filter"));
    let clear = NewWidget::new(IconButton::new(icons::X, t("search.clear")).small());
    let ids = Toolbar {
        new_note: new_note.id(), new_folder: new_folder.id(), sort: sort.id(), collapse: collapse.id(),
        filter: search.field, placeholder: search.placeholder, clear: Handle::of(&clear),
    };
    let group = Flex::row().cross_axis_alignment(CrossAxisAlignment::Center)
        .with_fixed(new_note).with_fixed(new_folder).with_fixed(sort);
    let group = NewWidget::new(group).with_props(PropertySet::new().with(Gap::new(Length::px(size::SPACE_6))));
    let row = Flex::row().main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .cross_axis_alignment(CrossAxisAlignment::Center).with_fixed(group).with_fixed(collapse);
    let row = NewWidget::new(row).with_props(PropertySet::new().with(Padding {
        top: Length::px(size::SPACE_4), bottom: Length::px(size::SPACE_4),
        left: Length::ZERO, right: Length::ZERO,
    }));
    let filter = NewWidget::new(Flex::row().cross_axis_alignment(CrossAxisAlignment::Center)
        .with(search.widget, 1.0).with_fixed(clear));
    let wrapper = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(row).with_fixed(filter);
    let wrapper = NewWidget::new(wrapper).with_props(PropertySet::new().with(Padding {
        top: Length::px(size::SPACE_8), left: Length::px(size::SPACE_8),
        right: Length::px(size::SPACE_8), bottom: Length::ZERO,
    }));
    (wrapper, ids)
}

pub(in crate::ui::file_tree) fn filtered_empty_state() -> NewWidget<Flex> {
    let empty = NewWidget::new(Label::new(&t("search.noResults"), |theme| theme.text_secondary));
    NewWidget::new(Flex::column()
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .with_fixed(empty))
}
