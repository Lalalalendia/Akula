//! Shared native text input for study names, typed answers and annotation notes.
//! The callers retain their independent persistence and conflict policies.

use masonry::accesskit::Role;
use masonry::app::RenderRoot;
use masonry::core::{NewWidget, PropertySet};
use masonry::kurbo::{Axis, Point};
use masonry::layout::Length;
use masonry::properties::types::{CrossAxisAlignment, MainAxisAlignment};
use masonry::properties::{Gap, Padding};
use masonry::widgets::Flex;

use crate::i18n::t;
use crate::ui::dialog::{self, CardSize, DialogIds, Modal};
use crate::ui::editor::TextEditor;
use crate::ui::handle::Handle;
use crate::ui::search::{self, Placeholder};
use crate::ui::theme;
use crate::ui::tokens::size;
use crate::ui::widgets::{Rule, TextButton, Variant};

pub(super) fn open_input_dialog(
    root: &mut RenderRoot, title: &str, initial: &str, hint_key: &str, cancel_key: &str, confirm_key: &str,
) -> (DialogIds, Handle<TextEditor>, Handle<Placeholder>) {
    let header = dialog::header(title, None::<NewWidget<Flex>>);
    let (entry, field, placeholder) = search::field(initial, t(hint_key));
    let content = NewWidget::new(
        Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch).with_fixed(entry)
    ).with_props(PropertySet::new().with(Padding::all(Length::px(size::SPACE_24))));
    let cancel = NewWidget::new(TextButton::new(t(cancel_key), Variant::Ghost));
    let confirm = NewWidget::new(TextButton::new(t(confirm_key), Variant::Primary));
    let ids = (cancel.id(), confirm.id());
    let footer = NewWidget::new(
        Flex::row().main_axis_alignment(MainAxisAlignment::End)
            .with_fixed(cancel).with_fixed(confirm)
    ).with_props(PropertySet::new()
        .with(Gap::new(Length::px(size::DIALOG_GAP)))
        .with(Padding::all(Length::px(size::DIALOG_PADDING))));
    let border = || Rule::new(Axis::Horizontal, || theme::current().border_default);
    let card = NewWidget::new(Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(header).with_fixed(border())
        .with_fixed(content).with_fixed(border()).with_fixed(footer));
    let modal = Modal::new(card, title, CardSize::Fit(size::CONFIRM_DIALOG_WIDTH), Role::Dialog, true);
    let dialog = DialogIds::of(&modal, ids);
    root.add_layer(modal, Point::ORIGIN);
    root.focus_on(Some(field.id()));
    (dialog, field, placeholder)
}
