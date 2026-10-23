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
    extras: TransferExtras<'_>,
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
    #[cfg(feature = "p2p-device-ui")]
    let mut peer_controls = None;
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
    if let Some(identity) = extras.identity {
        let (panel, actions) = identity::render(identity);
        identity_actions = actions;
        card = card.with_fixed(rule()).with_fixed(panel);
    }
    #[cfg(feature = "p2p-device-ui")]
    if let Some(peer) = extras.peer {
        let (panel, controls) = peer_pair::render(peer);
        peer_controls = Some(controls);
        // Multiple devices, drafts and Inbox items must not push key-input
        // controls outside a modal; the peer section is independently scrollable.
        card = card.with_fixed(rule()).with(
            NewWidget::new(ScrollArea::new(panel)),
            1.0,
        );
    }
    #[cfg(feature = "p2p-inbox-preview")]
    let (card, inbox_save_actions) = if let Some(inbox) = extras.inbox {
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
    let (card, native_send_actions) = if let Some(sender) = extras.sender {
        let (section, actions) = native_send_view::render(sender, contacts, extras.can_send);
        (card.with_fixed(rule()).with_fixed(section), actions)
    } else {
        (card, Vec::new())
    };
    #[cfg(feature = "p2p-native-receive-beta")]
    let (card, native_receive_actions) = if let Some(receiver) = extras.receiver {
        let (section, actions) = native_receive_view::render(receiver, contacts);
        (card.with_fixed(rule()).with_fixed(section), actions)
    } else {
        (card, Vec::new())
    };
    #[cfg(feature = "messaging-w4-pilot")]
    let mut card = card;
    #[cfg(feature = "messaging-w4-pilot")]
    let mut w4_actions = Vec::new();
    #[cfg(feature = "messaging-w4-pilot")]
    if let Some(view) = extras.w4 {
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
            #[cfg(feature = "p2p-device-ui")]
            peer_controls,
            #[cfg(feature = "p2p-inbox-preview")]
            inbox_save_actions,
            #[cfg(feature = "p2p-native-send-beta")]
            native_send_actions,
            #[cfg(feature = "p2p-native-receive-beta")]
            native_receive_actions,
            #[cfg(feature = "messaging-w4-pilot")]
            w4_actions,
        },
    )
}
