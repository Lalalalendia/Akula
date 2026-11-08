//! Real transfer-modal Pressed routing; not a signed human-presence proof.
use super::*;
use crate::ui::transfer::OfferConsentAction;
use crate::ui::widgets::Pressed;
use aquilum_core::p2p_offer_lab::{DecisionAction, OfferV1, OfferedFile, PeerBinding, SCHEMA};
use aquilum_core::sync::DeviceId;
use uuid::Uuid;

fn offer(now: u64) -> OfferV1 {
    OfferV1 {
        schema: SCHEMA,
        offer_id: Uuid::from_u128(800),
        nonce: [7; 16],
        sender: PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(1)),
            endpoint_id: [3; 32],
        },
        receiver: PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(2)),
            endpoint_id: [4; 32],
        },
        created_unix: now - 1,
        expires_unix: now + 120,
        files: vec![OfferedFile {
            label: "shown-as-untrusted.txt".into(),
            bytes: 5,
            blake3_root: *blake3::hash(b"alpha").as_bytes(),
        }],
    }
}

fn button(h: &Harness, is_accept: bool) -> masonry::core::WidgetId {
    let Some(Popup::Transfers(dialog)) = &h.app.popup else {
        panic!("transfer dialog missing");
    };
    dialog
        .offer_consent_actions
        .iter()
        .find(|(_, action)| matches!(action, OfferConsentAction::Accept(_)) == is_accept)
        .expect("missing Offer action")
        .0
}

#[test]
fn modal_accept_comes_only_from_explicit_button_press_and_consumes_once() {
    let mut h = Harness::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let offer = offer(now);
    h.app
        .transfer
        .offer_consent
        .present_from_authenticated_transport(
            &offer.encode(now).unwrap(),
            offer.sender,
            offer.receiver,
            now,
        )
        .unwrap();
    assert!(h.app.transfer.offer_consent.take_response().is_none());
    let transfer_button = h.app.chrome.as_ref().unwrap().transfer_button;
    h.act(transfer_button, Pressed);
    let accept = button(&h, true);
    h.act(accept, Pressed);
    let reply = h.app.transfer.offer_consent.take_response().unwrap();
    assert_eq!(reply.decision().action, DecisionAction::Accept);
    assert_eq!(reply.decision().selected, vec![0]);
    assert_eq!(reply.decision().manifest_commitment, offer.commitment());
    assert!(h.app.transfer.offer_consent.take_response().is_none());
    h.act(accept, Pressed);
    assert!(h.app.transfer.offer_consent.take_response().is_none());
}

#[test]
fn closing_modal_or_stale_widget_id_cannot_grant_consent() {
    let mut h = Harness::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let offer = offer(now);
    let wire = offer.encode(now).unwrap();
    h.app
        .transfer
        .offer_consent
        .present_from_authenticated_transport(&wire, offer.sender, offer.receiver, now)
        .unwrap();
    let transfer_button = h.app.chrome.as_ref().unwrap().transfer_button;
    h.act(transfer_button, Pressed);
    let stale_accept = button(&h, true);
    let close = match &h.app.popup {
        Some(Popup::Transfers(dialog)) => dialog.close,
        _ => panic!("transfer dialog missing"),
    };
    h.act(close, Pressed);
    assert!(h.app.transfer.offer_consent.take_response().is_none());
    h.app
        .transfer
        .offer_consent
        .present_from_authenticated_transport(&wire, offer.sender, offer.receiver, now)
        .unwrap();
    let transfer_button = h.app.chrome.as_ref().unwrap().transfer_button;
    h.act(transfer_button, Pressed);
    h.act(stale_accept, Pressed);
    assert!(h.app.transfer.offer_consent.take_response().is_none());
    let decline = button(&h, false);
    h.act(decline, Pressed);
    let reply = h.app.transfer.offer_consent.take_response().unwrap();
    assert_eq!(reply.decision().action, DecisionAction::Decline);
    assert!(reply.decision().selected.is_empty());
}

#[test]
fn actual_pointer_event_on_visible_accept_button_creates_one_decision() {
    let mut h = Harness::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let offer = offer(now);
    h.app
        .transfer
        .offer_consent
        .present_from_authenticated_transport(
            &offer.encode(now).unwrap(),
            offer.sender,
            offer.receiver,
            now,
        )
        .unwrap();
    let rail = h.app.chrome.as_ref().unwrap().transfer_button;
    let position = h
        .root
        .get_widget(rail)
        .expect("transfer rail button")
        .ctx()
        .bounding_box()
        .center();
    h.click(position.x, position.y);
    let accept = button(&h, true);
    let position = h
        .root
        .get_widget(accept)
        .expect("consent button must be mounted")
        .ctx()
        .bounding_box()
        .center();
    h.click(position.x, position.y);
    let reply = h.app.transfer.offer_consent.take_response().unwrap();
    assert_eq!(reply.decision().action, DecisionAction::Accept);
    assert_eq!(reply.decision().selected, vec![0]);
    assert_eq!(reply.decision().manifest_commitment, offer.commitment());
    assert!(h.app.transfer.offer_consent.take_response().is_none());
}

#[test]
fn pending_offer_hides_background_file_request_controls() {
    let mut h = Harness::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let offer = offer(now);
    h.app
        .transfer
        .offer_consent
        .present_from_authenticated_transport(
            &offer.encode(now).unwrap(),
            offer.sender,
            offer.receiver,
            now,
        )
        .unwrap();
    let transfer_button = h.app.chrome.as_ref().unwrap().transfer_button;
    h.act(transfer_button, Pressed);
    let Some(Popup::Transfers(dialog)) = &h.app.popup else {
        panic!("Offer consent modal missing");
    };
    assert_eq!(dialog.offer_consent_actions.len(), 2);
    assert!(dialog.actions.is_empty());
    assert!(dialog.device_actions.is_empty());
    assert!(dialog.identity_actions.is_empty());
    assert!(dialog.peer_controls.is_none());
    #[cfg(feature = "p2p-inbox-preview")]
    assert!(dialog.inbox_save_actions.is_empty());
    #[cfg(feature = "p2p-native-send-beta")]
    assert!(dialog.native_send_actions.is_empty());
    #[cfg(feature = "p2p-native-receive-beta")]
    assert!(dialog.native_receive_actions.is_empty());
    assert!(h.app.transfer.offer_consent.take_response().is_none());
}
