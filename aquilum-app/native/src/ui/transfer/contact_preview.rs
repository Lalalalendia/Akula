//! Read-only preview of OS-verified Iroh transport pins. Never send authority.
use crate::i18n::t;
use masonry::core::{NewWidget, PropertySet};
use masonry::layout::Length;
use masonry::properties::types::CrossAxisAlignment;
use masonry::properties::{Gap, Padding};
use masonry::widgets::Flex;

use super::{details, hint};
use crate::ui::tokens::size;

#[derive(Clone, Debug)]
pub struct TransportContactRow {
    pub full_endpoint_id: String,
    pub revoked: bool,
}

#[derive(Default)]
pub struct TransportContactView {
    pub rows: Vec<TransportContactRow>,
    pub error: Option<String>,
}

pub(super) fn render(view: &TransportContactView) -> NewWidget<Flex> {
    let mut panel = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(details(&t("transfer.transportContacts")));
    if view.rows.is_empty() && view.error.is_none() {
        panel = panel.with_fixed(hint(&t("transfer.noTransportContacts")));
    }
    for contact in &view.rows {
        let state = t(if contact.revoked {
            "transfer.transportContactRevoked"
        } else {
            "transfer.transportContactConfirmed"
        });
        panel = panel.with_fixed(NewWidget::new(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_fixed(details(&contact.full_endpoint_id))
                .with_fixed(hint(&state)),
        ));
    }
    if let Some(error) = &view.error {
        panel = panel.with_fixed(hint(&format!(
            "{}: {error}",
            t("transfer.transportContactsUnavailable")
        )));
    }
    panel = panel.with_fixed(hint(&t("transfer.transportContactsOnly")));
    NewWidget::new(panel).with_props(
        PropertySet::new()
            .with(Gap::new(Length::px(size::SPACE_4)))
            .with(Padding::all(Length::px(size::DIALOG_PADDING))),
    )
}
