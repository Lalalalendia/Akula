//! Reusable transfer layout primitives, shared by drafts and local identity.

use masonry::core::NewWidget;
use masonry::kurbo::{Axis, Size};

use crate::ui::text::selectable;
use crate::ui::theme;
use crate::ui::tokens::size;
use crate::ui::widgets::{Leading, Rule};

pub(super) fn card_size(room: Size) -> Size {
    Size::new(room.width.min(740.0), room.height.min(600.0))
}

pub(super) fn rule() -> NewWidget<Rule> {
    Rule::new(Axis::Horizontal, || theme::current().border_default)
}

pub(super) fn details(value: &str) -> NewWidget<crate::ui::widgets::Label> {
    selectable(
        value,
        |theme| theme.text_secondary,
        None,
        Leading::Px(size::SIZE_20 as f32),
    )
}

pub(super) fn hint(value: &str) -> NewWidget<crate::ui::widgets::Label> {
    selectable(
        value,
        |theme| theme.text_tertiary,
        None,
        Leading::Px(size::SIZE_20 as f32),
    )
}

pub(super) fn display_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}
