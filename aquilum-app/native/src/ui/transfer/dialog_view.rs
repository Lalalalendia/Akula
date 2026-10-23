//! Transfer dialog construction, leaving transfer.rs as a compact API.
use super::*;

/// Draft controls remain inert. The optional received-Inbox Save Copy button
/// invokes only an explicit local no-clobber export, never QUIC or Vault ACL.
pub fn dialog(
    rows: &[QueueRow],
    devices: &[DeviceRow],
    #[cfg(feature = "p2p-device-contacts-preview")] contacts: &TransportContactView,
    choosing: bool,
    error: Option<&str>,
    identity: Option<&IdentityView>,
    inbox: Option<&InboxView>,
    #[cfg(feature = "p2p-native-send-beta")] sender: Option<&NativeSendView>,
    #[cfg(feature = "p2p-native-send-beta")] can_send: bool,
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
    let (card, inbox_save_actions) = if let Some(inbox) = inbox {
        let (panel, buttons) = inbox_view::render(inbox);
        (
            card.with_fixed(rule())
                .with_fixed(details(&t("transfer.inboxTitle")))
                .with(NewWidget::new(ScrollArea::new(panel)), 1.0),
            buttons,
        )
    } else {
        (card, Vec::new())
    };
    #[cfg(feature = "p2p-native-send-beta")]
    let (card, native_send_actions) = if let Some(sender) = sender {
        let (section, actions) = native_send_view::render(sender, contacts, can_send);
        (card.with_fixed(rule()).with_fixed(section), actions)
    } else {
        (card, Vec::new())
    };
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
            inbox_save_actions,
            #[cfg(feature = "p2p-native-send-beta")]
            native_send_actions,
        },
    )
}
