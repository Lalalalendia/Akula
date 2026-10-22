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
    peer: Option<&PeerView>,
) -> (NewWidget<Modal>, TransferDialog) {
    let close = NewWidget::new(IconButton::new(icons::X, t("transfer.close")));
    let close_id = close.id();
    let choose = dialog_button::render_choose(choosing);
    let choose_id = choose.id();
    let (queue, actions) = queue_view::render(rows);
    let (info, device_actions) = devices::render(devices, error);
    let mut identity_actions = Vec::new();
    let mut peer_controls = None;
    let footer = NewWidget::new(
        Flex::row()
            .main_axis_alignment(MainAxisAlignment::End)
            .with_fixed(choose),
    )
    .with_props(PropertySet::new().with(Padding::all(Length::px(size::DIALOG_PADDING))));
    // All contacts, public keys, pair controls and draft files scroll together.
    let mut content = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(info)
        .with_fixed(rule());
    #[cfg(feature = "p2p-device-contacts-preview")]
    {
        content = content
            .with_fixed(contact_preview::render(contacts))
            .with_fixed(rule());
    }
    if let Some(identity) = identity {
        let (panel, actions) = identity::render(identity);
        identity_actions = actions;
        content = content.with_fixed(panel).with_fixed(rule());
    }
    if let Some(peer) = peer {
        let (panel, controls) = peer_pair::render(peer);
        peer_controls = Some(controls);
        content = content.with_fixed(panel).with_fixed(rule());
    }
    let scrollable = NewWidget::new(ScrollArea::new(NewWidget::new(content.with_fixed(queue))));
    let card = NewWidget::new(
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_fixed(header(&t("transfer.title"), Some(close)))
            .with_fixed(rule())
            .with(scrollable, 1.0)
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
            peer_controls,
        },
    )
}
