//! Explicit opt-in P2P and W4 sections inside the existing transfer modal.
use super::super::*;

pub(super) struct SectionActions {
    pub(super) identity_actions: Vec<(WidgetId, IdentityAction)>,
    #[cfg(feature = "p2p-device-ui")]
    pub(super) peer_controls: Option<PeerControls>,
    #[cfg(feature = "p2p-offer-consent-qa")]
    pub(super) offer_consent_actions: Vec<(WidgetId, OfferConsentAction)>,
    #[cfg(feature = "p2p-inbox-preview")]
    pub(super) inbox_save_actions: Vec<(WidgetId, InboxAction)>,
    #[cfg(feature = "p2p-native-send-beta")]
    pub(super) native_send_actions: Vec<(WidgetId, NativeSendAction)>,
    #[cfg(feature = "p2p-native-receive-beta")]
    pub(super) native_receive_actions: Vec<(WidgetId, NativeReceiveAction)>,
    #[cfg(feature = "messaging-w4-pilot")]
    pub(super) w4_actions: Vec<(WidgetId, PilotAction)>,
}

pub(super) fn choose_button(choosing: bool) -> NewWidget<TextButton> {
    NewWidget::new(
        TextButton::new(
            if choosing {
                t("transfer.picking")
            } else {
                t("transfer.addFile")
            },
            Variant::Primary,
        )
        .with_icon(icons::UPLOAD),
    )
}

pub(super) fn render(
    card: Flex,
    extras: TransferExtras<'_>,
    #[cfg(feature = "p2p-device-contacts-preview")] contacts: &TransportContactView,
) -> (Flex, SectionActions) {
    let mut identity_actions = Vec::new();
    #[cfg(feature = "p2p-device-ui")]
    let mut peer_controls = None;
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
        card = card
            .with_fixed(rule())
            .with(NewWidget::new(ScrollArea::new(panel)), 1.0);
    }
    #[cfg(feature = "p2p-offer-consent-qa")]
    let (card, offer_consent_actions) = if let Some(offer) = extras.offer_consent {
        let (panel, actions) = super::offer_consent_view::render(offer);
        (
            card.with_fixed(rule())
                .with(NewWidget::new(ScrollArea::new(panel)), 1.0),
            actions,
        )
    } else {
        (card, Vec::new())
    };
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
    (
        card,
        SectionActions {
            identity_actions,
            #[cfg(feature = "p2p-device-ui")]
            peer_controls,
            #[cfg(feature = "p2p-offer-consent-qa")]
            offer_consent_actions,
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
