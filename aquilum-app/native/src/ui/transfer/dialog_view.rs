//! Transfer dialog construction, leaving transfer.rs as a compact API.
use super::*;

/// The only actionable controls modify local drafts or open the local picker.
/// No button can reach QUIC, File Gate, YDB or any source of peer authority.
pub fn dialog(
    rows: &[QueueRow],
    devices: &[DeviceRow],
    #[cfg(feature = "p2p-device-contacts-preview")] contacts: &TransportContactView,
    choosing: bool,
    error: Option<&str>,
    identity: Option<&IdentityView>,
) -> (NewWidget<Modal>, TransferDialog) {
    let close = NewWidget::new(IconButton::new(icons::X, t("transfer.close")));
    let close_id = close.id();
    let choose = NewWidget::new(
        TextButton::new(
            if choosing {
                t("transfer.picking")
            } else {
                t("transfer.addFile")
            },
            Variant::Primary,
        )
        .with_icon(icons::UPLOAD),
    );
    let choose_id = choose.id();
    let mut actions = Vec::new();
    let mut queue = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch);
    if rows.is_empty() {
        queue = queue.with_fixed(hint(&t("transfer.empty")));
    }
    for row in rows {
        let state = if row.paused {
            t("transfer.paused")
        } else {
            t("transfer.pending")
        };
        let summary = t_with(
            "transfer.summary",
            &[("size", &display_size(row.bytes)), ("state", &state)],
        );
        let labels = NewWidget::new(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_fixed(details(&row.name))
                .with_fixed(hint(&summary)),
        );
        let toggle = NewWidget::new(TextButton::new(
            if row.paused {
                t("transfer.resume")
            } else {
                t("transfer.pause")
            },
            Variant::Ghost,
        ));
        let remove = NewWidget::new(TextButton::new(t("transfer.remove"), Variant::Ghost));
        actions.push((toggle.id(), row.index, QueueAction::Toggle));
        actions.push((remove.id(), row.index, QueueAction::Remove));
        let buttons = NewWidget::new(Flex::row().with_fixed(toggle).with_fixed(remove));
        let entry = NewWidget::new(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .with(labels, 1.0)
                .with_fixed(buttons),
        )
        .with_props(
            PropertySet::new()
                .with(Gap::new(Length::px(size::SPACE_8)))
                .with(Padding::all(Length::px(size::SPACE_8))),
        );
        queue = queue.with_fixed(entry);
        queue = queue.with_fixed(rule());
    }
    let queue = NewWidget::new(queue).with_props(
        PropertySet::new()
            .with(Gap::new(Length::px(size::SPACE_4)))
            .with(Padding::all(Length::px(size::DIALOG_PADDING))),
    );
    let queue = NewWidget::new(ScrollArea::new(queue));

    let (info, device_actions) = devices::render(devices, error);
    let mut identity_actions = Vec::new();
    let footer = NewWidget::new(
        Flex::row()
            .main_axis_alignment(MainAxisAlignment::End)
            .with_fixed(choose),
    )
    .with_props(PropertySet::new().with(Padding::all(Length::px(size::DIALOG_PADDING))));
    let card = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(header(&t("transfer.title"), Some(close)))
        .with_fixed(rule())
        .with_fixed(info);
    #[cfg(feature = "p2p-device-contacts-preview")]
    let card = card.with(
        NewWidget::new(ScrollArea::new(contact_preview::render(contacts))),
        1.0,
    );
    let mut card = card;
    if let Some(identity) = identity {
        let (panel, actions) = identity::render(identity);
        identity_actions = actions;
        card = card.with_fixed(rule()).with_fixed(panel);
    }
    let card = NewWidget::new(
        card.with_fixed(rule())
            .with(queue, 1.0)
            .with_fixed(rule())
            .with_fixed(footer),
    );
    let layer = Modal::new(
        card,
        &t("transfer.title"),
        CardSize::Window(card_size),
        Role::Dialog,
        false,
    );
    let layer_id = layer.id();
    (
        layer,
        TransferDialog {
            layer: layer_id,
            close: close_id,
            choose: choose_id,
            actions,
            device_actions,
            identity_actions,
        },
    )
}
