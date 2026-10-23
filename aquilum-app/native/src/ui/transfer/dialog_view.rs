//! Transfer dialog construction, leaving transfer.rs as a compact API.
use super::*;

pub fn dialog(
    rows: &[QueueRow],
    devices: &[DeviceRow],
    #[cfg(feature = "p2p-device-contacts-preview")] contacts: &TransportContactView,
    choosing: bool,
    error: Option<&str>,
    identity: Option<&IdentityView>,
    #[cfg(feature = "messaging-w4-pilot")] w4: Option<&PilotView>,
) -> (NewWidget<Modal>, TransferDialog) {
    let close = NewWidget::new(IconButton::new(icons::X, t("transfer.close")));
    let close_id = close.id();
    let label = t(choosing.then_some("transfer.picking").unwrap_or("transfer.addFile"));
    let choose = NewWidget::new(TextButton::new(label, Variant::Primary).with_icon(icons::UPLOAD));
    let choose_id = choose.id();
    let (queue, actions) = queue_view::render(rows);
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
    #[cfg(feature = "messaging-w4-pilot")]
    let mut w4_actions = Vec::new();
    #[cfg(feature = "messaging-w4-pilot")]
    if let Some(view) = w4 {
        let (panel, actions) = w4_pilot::render(view);
        w4_actions = actions;
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
            #[cfg(feature = "messaging-w4-pilot")]
            w4_actions,
        },
    )
}
