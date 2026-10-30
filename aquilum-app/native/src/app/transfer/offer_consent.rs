//! Draft-only, in-process Native gesture boundary for ONE authenticated Offer.
//! No production transport invokes present_from_authenticated_transport yet.
//! A UI reply is not a signed receipt, OS-owned journal, File Gate grant or
//! proof of an independently authenticated live QUIC session.
use crate::ui::transfer::{OfferConsentAction, OfferConsentView};
use aquilum_core::p2p_device_identity::{IdentityError, peer_pin};
use aquilum_core::p2p_offer_lab::{DecisionAction, DecisionV1, OfferError, OfferV1, PeerBinding};
use std::time::{SystemTime, UNIX_EPOCH};

struct Pending {
    offer: OfferV1,
    sender: PeerBinding,
    receiver: PeerBinding,
    revision: u64,
}

/// Not serializable or cloneable; only the Native Pressed handler may mint this.
/// The future transport bridge must independently reauthenticate both peers
/// and validate the durable sender-half and pinned Inbox facts before signing.
pub(in crate::app) struct UiReply {
    decision: DecisionV1,
    offer: OfferV1,
    sender: PeerBinding,
    receiver: PeerBinding,
}

/// Internal-only, non-cloneable result of matching a user gesture against
/// unchanged Offer bytes and freshly supplied transport identities.
/// Still neither an OS-backed consent journal nor a File Gate permission.
#[allow(dead_code)]
pub(in crate::app) struct BoundUiAccept {
    decision: DecisionV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum UiConsentError {
    Declined,
    ChangedOffer,
    WrongPeer,
    ChangedDecision,
    ClockUnavailable,
    Identity(IdentityError),
    Offer(OfferError),
}

impl UiReply {
    #[cfg(test)]
    pub(in crate::app) fn decision(&self) -> &DecisionV1 {
        &self.decision
    }

    /// Consume an actual Native gesture exactly once and revalidate it against
    /// the live peer bindings supplied by a future authenticated QUIC caller.
    /// Never derive these peers from JSON, mDNS or a clipboard invitation.
    #[allow(dead_code)]
    pub(in crate::app) fn bind_once(
        self,
        received_offer: &OfferV1,
        live_sender: PeerBinding,
        local_receiver: PeerBinding,
        selected_sender_full_id: &str,
    ) -> Result<BoundUiAccept, UiConsentError> {
        let now = OfferConsentUi::now_unix().ok_or(UiConsentError::ClockUnavailable)?;
        // Reject a changed decision, offer or live peer BEFORE OS access.
        let accepted = self.bind_once_at(received_offer, live_sender, local_receiver, now)?;
        // Fresh OS state matters: confirmed on click is not confirmed on use.
        peer_pin::require_pinned_remote(selected_sender_full_id, &live_sender.endpoint_id)
            .map_err(UiConsentError::Identity)?;
        Ok(accepted)
    }

    fn bind_once_at(
        self,
        received_offer: &OfferV1,
        live_sender: PeerBinding,
        local_receiver: PeerBinding,
        now: u64,
    ) -> Result<BoundUiAccept, UiConsentError> {
        if self.decision.action != DecisionAction::Accept {
            return Err(UiConsentError::Declined);
        }
        if self.offer != *received_offer {
            return Err(UiConsentError::ChangedOffer);
        }
        if self.sender != live_sender || self.receiver != local_receiver {
            return Err(UiConsentError::WrongPeer);
        }
        let expected = received_offer
            .receiver_decision(
                DecisionAction::Accept,
                vec![0],
                now,
                live_sender,
                local_receiver,
            )
            .map_err(UiConsentError::Offer)?;
        if self.decision != expected {
            return Err(UiConsentError::ChangedDecision);
        }
        Ok(BoundUiAccept {
            decision: self.decision,
        })
    }
}

#[derive(Default)]
pub(in crate::app) struct OfferConsentUi {
    pub(super) view: Option<OfferConsentView>,
    pending: Option<Pending>,
    response: Option<UiReply>,
    revision: u64,
}

impl OfferConsentUi {
    pub(in crate::app) fn now_unix() -> Option<u64> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|time| time.as_secs())
    }

    /// Inputs MUST come from the verified live QUIC endpoint and local device,
    /// not from Offer JSON. This method has no production caller by design.
    #[allow(dead_code)]
    pub(in crate::app) fn present_from_authenticated_transport(
        &mut self,
        wire: &[u8],
        authenticated_sender: PeerBinding,
        trusted_local_receiver: PeerBinding,
        now: u64,
    ) -> Result<(), OfferError> {
        // Even a failed replacement invalidates previously displayed buttons.
        self.cancel();
        let offer =
            OfferV1::decode_for_receiver(wire, now, authenticated_sender, trusted_local_receiver)?;
        if offer.files.len() != 1 {
            return Err(OfferError::InvalidSelection);
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(OfferError::AlreadyResolved)?;
        let file = &offer.files[0];
        let hex =
            |bytes: &[u8]| -> String { bytes.iter().map(|byte| format!("{byte:02x}")).collect() };
        self.view = Some(OfferConsentView {
            revision: self.revision,
            sender_endpoint: hex(&authenticated_sender.endpoint_id),
            offer_id: offer.offer_id.to_string(),
            file_label: file.label.clone(),
            bytes: file.bytes,
            digest: hex(&file.blake3_root),
        });
        self.pending = Some(Pending {
            offer,
            sender: authenticated_sender,
            receiver: trusted_local_receiver,
            revision: self.revision,
        });
        Ok(())
    }

    /// Called only for an actual Pressed action whose WidgetId belongs to the
    /// current modal. A stale button, invalid time or repeated reply fails closed.
    pub(super) fn respond(&mut self, action: OfferConsentAction) -> bool {
        let (revision, choice) = match action {
            OfferConsentAction::Accept(revision) => (revision, DecisionAction::Accept),
            OfferConsentAction::Decline(revision) => (revision, DecisionAction::Decline),
        };
        let Some(pending) = self.pending.as_ref() else {
            return false;
        };
        if pending.revision != revision {
            return false;
        }
        // Only the host clock is used; no network-provided accepted_at.
        let decision = Self::now_unix().and_then(|now| {
            pending
                .offer
                .receiver_decision(
                    choice,
                    if choice == DecisionAction::Accept {
                        vec![0]
                    } else {
                        Vec::new()
                    },
                    now,
                    pending.sender,
                    pending.receiver,
                )
                .ok()
        });
        let Some(pending) = self.pending.take() else {
            return false;
        };
        self.view = None;
        self.response = decision.map(|decision| UiReply {
            decision,
            offer: pending.offer,
            sender: pending.sender,
            receiver: pending.receiver,
        });
        true
    }

    #[allow(dead_code)]
    pub(in crate::app) fn take_response(&mut self) -> Option<UiReply> {
        self.response.take()
    }

    pub(super) fn cancel(&mut self) {
        self.pending = None;
        self.view = None;
        self.response = None;
        self.revision = self.revision.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::p2p_offer_lab::{OfferedFile, SCHEMA};
    use aquilum_core::sync::DeviceId;
    use uuid::Uuid;

    fn make_offer(now: u64) -> OfferV1 {
        OfferV1 {
            schema: SCHEMA,
            offer_id: Uuid::from_u128(42),
            nonce: [8; 16],
            sender: PeerBinding {
                device_id: DeviceId::from_uuid(Uuid::from_u128(1)),
                endpoint_id: [1; 32],
            },
            receiver: PeerBinding {
                device_id: DeviceId::from_uuid(Uuid::from_u128(2)),
                endpoint_id: [2; 32],
            },
            created_unix: now - 1,
            expires_unix: now + 120,
            files: vec![OfferedFile {
                label: "untrusted-label.txt".into(),
                bytes: 5,
                blake3_root: *blake3::hash(b"alpha").as_bytes(),
            }],
        }
    }

    #[test]
    fn forged_peer_or_multiple_files_never_show_accept() {
        let now = OfferConsentUi::now_unix().unwrap();
        let mut ui = OfferConsentUi::default();
        let offer = make_offer(now);
        let wire = offer.encode(now).unwrap();
        assert_eq!(
            ui.present_from_authenticated_transport(&wire, offer.receiver, offer.receiver, now),
            Err(OfferError::WrongIdentity)
        );
        assert!(ui.view.is_none());
        let mut batch = offer.clone();
        batch.files.push(OfferedFile {
            label: "second.txt".into(),
            bytes: 4,
            blake3_root: *blake3::hash(b"beta").as_bytes(),
        });
        assert_eq!(
            ui.present_from_authenticated_transport(
                &batch.encode(now).unwrap(),
                batch.sender,
                batch.receiver,
                now,
            ),
            Err(OfferError::InvalidSelection)
        );
        assert!(ui.view.is_none());
        assert!(ui.take_response().is_none());
    }

    #[test]
    fn old_revision_cancel_and_duplicate_press_never_create_a_reply() {
        let now = OfferConsentUi::now_unix().unwrap();
        let mut ui = OfferConsentUi::default();
        let offer = make_offer(now);
        let wire = offer.encode(now).unwrap();
        ui.present_from_authenticated_transport(&wire, offer.sender, offer.receiver, now)
            .unwrap();
        let old = ui.view.as_ref().unwrap().revision;
        assert!(!ui.respond(OfferConsentAction::Accept(old + 1)));
        assert!(ui.take_response().is_none());
        ui.cancel();
        assert!(!ui.respond(OfferConsentAction::Accept(old)));
        ui.present_from_authenticated_transport(&wire, offer.sender, offer.receiver, now)
            .unwrap();
        let fresh = ui.view.as_ref().unwrap().revision;
        assert_ne!(old, fresh);
        assert!(!ui.respond(OfferConsentAction::Accept(old)));
        assert!(ui.take_response().is_none());
        assert!(ui.respond(OfferConsentAction::Decline(fresh)));
        assert_eq!(
            ui.take_response().unwrap().decision().action,
            DecisionAction::Decline
        );
        assert!(!ui.respond(OfferConsentAction::Accept(fresh)));
        assert!(ui.take_response().is_none());
    }

    #[test]
    fn expired_offer_never_mints_accept() {
        let now = OfferConsentUi::now_unix().unwrap();
        let mut offer = make_offer(now);
        offer.expires_unix = now;
        let mut ui = OfferConsentUi::default();
        assert_eq!(
            ui.present_from_authenticated_transport(
                &serde_json::to_vec(&offer).unwrap(),
                offer.sender,
                offer.receiver,
                now,
            ),
            Err(OfferError::Expired)
        );
        assert!(!ui.respond(OfferConsentAction::Accept(1)));
        assert!(ui.take_response().is_none());
    }

    fn accepted_reply() -> (OfferV1, UiReply, u64) {
        let now = OfferConsentUi::now_unix().unwrap();
        let offer = make_offer(now);
        let mut ui = OfferConsentUi::default();
        ui.present_from_authenticated_transport(
            &offer.encode(now).unwrap(),
            offer.sender,
            offer.receiver,
            now,
        )
        .unwrap();
        let revision = ui.view.as_ref().unwrap().revision;
        assert!(ui.respond(OfferConsentAction::Accept(revision)));
        (offer, ui.take_response().unwrap(), now)
    }

    #[test]
    fn accepted_ui_reply_binds_once_to_unchanged_offer_and_live_peers() {
        let (offer, reply, now) = accepted_reply();
        let bound = reply
            .bind_once_at(&offer, offer.sender, offer.receiver, now)
            .unwrap();
        assert_eq!(bound.decision.action, DecisionAction::Accept);
        assert_eq!(bound.decision.selected, vec![0]);
        assert_eq!(bound.decision.offer_id, offer.offer_id);
        assert_eq!(bound.decision.manifest_commitment, offer.commitment());
    }

    #[test]
    fn changed_offer_and_swapped_peer_cannot_consume_ui_reply() {
        let (offer, reply, now) = accepted_reply();
        let mut changed = offer.clone();
        changed.files[0].label = "changed-after-click.txt".into();
        assert!(matches!(
            reply.bind_once_at(&changed, offer.sender, offer.receiver, now),
            Err(UiConsentError::ChangedOffer)
        ));
        let (offer, reply, now) = accepted_reply();
        assert!(matches!(
            reply.bind_once_at(&offer, offer.receiver, offer.sender, now),
            Err(UiConsentError::WrongPeer)
        ));
    }

    #[test]
    fn forged_offer_preflight_fails_before_os_peer_pin_lookup() {
        let (offer, reply, _) = accepted_reply();
        let mut changed = offer.clone();
        changed.files[0].bytes += 1;
        assert!(matches!(
            reply.bind_once(&changed, offer.sender, offer.receiver, "invalid-id"),
            Err(UiConsentError::ChangedOffer)
        ));
    }

    #[test]
    fn bad_peer_pin_identifier_cannot_use_accepted_ui_reply() {
        let (offer, reply, _) = accepted_reply();
        assert!(matches!(
            reply.bind_once(&offer, offer.sender, offer.receiver, "invalid-id"),
            Err(UiConsentError::Identity(IdentityError::InvalidPeerId))
        ));
    }

    #[test]
    fn accepted_ui_reply_expires_before_future_signature() {
        let (offer, reply, _) = accepted_reply();
        assert!(matches!(
            reply.bind_once_at(&offer, offer.sender, offer.receiver, offer.expires_unix),
            Err(UiConsentError::Offer(OfferError::Expired))
        ));
    }

    #[test]
    fn declined_ui_reply_cannot_become_an_acceptance() {
        let now = OfferConsentUi::now_unix().unwrap();
        let offer = make_offer(now);
        let mut ui = OfferConsentUi::default();
        ui.present_from_authenticated_transport(
            &offer.encode(now).unwrap(),
            offer.sender,
            offer.receiver,
            now,
        )
        .unwrap();
        let revision = ui.view.as_ref().unwrap().revision;
        assert!(ui.respond(OfferConsentAction::Decline(revision)));
        let reply = ui.take_response().unwrap();
        assert!(matches!(
            reply.bind_once_at(&offer, offer.sender, offer.receiver, now),
            Err(UiConsentError::Declined)
        ));
    }
}
