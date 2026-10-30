//! Transfer dialog construction, leaving transfer.rs as a compact API.
use super::*;
#[cfg(feature = "p2p-offer-consent-qa")]
mod offer_consent_view;
mod panels;

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
    let consent_only = panels::consent_only(&extras);
    let choose = panels::choose_button(choosing);
    let choose_id = choose.id();
    let (queue, actions) = queue_view::render(if consent_only { &[] } else { rows });
    let queue = NewWidget::new(ScrollArea::new(queue));
    let (info, device_actions) = devices::render(if consent_only { &[] } else { devices }, error);
    let footer = NewWidget::new(
        Flex::row()
            .main_axis_alignment(MainAxisAlignment::End)
            .with_fixed(choose),
    )
    .with_props(PropertySet::new().with(Padding::all(Length::px(size::DIALOG_PADDING))));
    let card = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(header(&t("transfer.title"), Some(close)))
        .with_fixed(rule());
    #[cfg(feature = "p2p-device-contacts-preview")]
    let card = panels::base(card, info, contacts, consent_only);
    #[cfg(not(feature = "p2p-device-contacts-preview"))]
    let card = panels::base(card, info, consent_only);
    #[cfg(feature = "p2p-device-contacts-preview")]
    let (card, sections) = panels::render(card, extras, contacts);
    #[cfg(not(feature = "p2p-device-contacts-preview"))]
    let (card, sections) = panels::render(card, extras);
    let card = panels::finish(card, queue, footer, consent_only);
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
            identity_actions: sections.identity_actions,
            #[cfg(feature = "p2p-device-ui")]
            peer_controls: sections.peer_controls,
            #[cfg(feature = "p2p-offer-consent-qa")]
            offer_consent_actions: sections.offer_consent_actions,
            #[cfg(feature = "p2p-inbox-preview")]
            inbox_save_actions: sections.inbox_save_actions,
            #[cfg(feature = "p2p-native-send-beta")]
            native_send_actions: sections.native_send_actions,
            #[cfg(feature = "p2p-native-receive-beta")]
            native_receive_actions: sections.native_receive_actions,
            #[cfg(feature = "messaging-w4-pilot")]
            w4_actions: sections.w4_actions,
        },
    )
}
