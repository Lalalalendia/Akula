//! Local draft file picker button. Never initiates peer networking.
use super::*;

pub(super) fn render_choose(choosing: bool) -> NewWidget<TextButton> {
    let caption = if choosing {
        t("transfer.picking")
    } else {
        t("transfer.addFile")
    };
    NewWidget::new(TextButton::new(caption, Variant::Primary).with_icon(icons::UPLOAD))
}
