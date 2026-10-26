//! AK-P2P-OFFER-01: metadata-only consent negotiation, no file authority.
//!
//! Opt-in lab. Does not read/write files, connect to Iroh, issue grants, apply
//! File Gate or persist sessions. PeerBinding MUST be independently derived
//! from enrollment and the authenticated QUIC endpoint, never mDNS or JSON.
//! A SelectionDraft is NOT permission to read or publish bytes.

use crate::sync::DeviceId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

pub const SCHEMA: u16 = 1;
pub const MAX_OFFER_WIRE: usize = 8192;
pub const MAX_DECISION_WIRE: usize = 1024;
pub const MAX_FILES: usize = 8;
pub const MAX_ITEM_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_BATCH_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_TTL_SECS: u64 = 600;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OfferError {
    InvalidEncoding,
    UnsupportedSchema,
    InvalidMetadata,
    TooLarge,
    NotYetValid,
    Expired,
    WrongIdentity,
    WrongCommitment,
    InvalidSelection,
    AlreadyResolved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerBinding {
    pub device_id: DeviceId,
    pub endpoint_id: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferedFile {
    /// Untrusted display-only name. Never a filesystem destination.
    pub label: String,
    pub bytes: u64,
    pub blake3_root: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferV1 {
    pub schema: u16,
    pub offer_id: Uuid,
    pub nonce: [u8; 16],
    pub sender: PeerBinding,
    pub receiver: PeerBinding,
    pub created_unix: u64,
    pub expires_unix: u64,
    pub files: Vec<OfferedFile>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAction {
    Accept,
    Decline,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionV1 {
    pub schema: u16,
    pub offer_id: Uuid,
    pub manifest_commitment: [u8; 32],
    pub action: DecisionAction,
    /// Unique, strictly ascending indices. No implicit accept-all.
    pub selected: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Pending,
    Accepted,
    Declined,
    Cancelled,
    Expired,
}

/// UI/negotiation draft; no read, write, stage, grant or File Gate methods.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionDraft {
    pub offer_id: Uuid,
    pub commitment: [u8; 32],
    pub files: Vec<OfferedFile>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    Selected(SelectionDraft),
    Declined,
}

/// Sender-side, single-use and explicitly non-durable consent state.
pub struct OfferSession {
    offer: OfferV1,
    commitment: [u8; 32],
    phase: Phase,
}

impl OfferV1 {
    pub fn validate(&self, now: u64) -> Result<(), OfferError> {
        if self.schema != SCHEMA {
            return Err(OfferError::UnsupportedSchema);
        }
        let ttl = self
            .expires_unix
            .checked_sub(self.created_unix)
            .ok_or(OfferError::InvalidMetadata)?;
        if ttl == 0
            || ttl > MAX_TTL_SECS
            || self.offer_id.is_nil()
            || self.nonce.iter().all(|&b| b == 0)
            || self.sender.device_id.as_uuid().is_nil()
            || self.receiver.device_id.as_uuid().is_nil()
            || self.sender.device_id == self.receiver.device_id
            || self.sender.endpoint_id == self.receiver.endpoint_id
            || self.sender.endpoint_id.iter().all(|&b| b == 0)
            || self.receiver.endpoint_id.iter().all(|&b| b == 0)
            || self.files.is_empty()
        {
            return Err(OfferError::InvalidMetadata);
        }
        if self.files.len() > MAX_FILES {
            return Err(OfferError::TooLarge);
        }
        if now < self.created_unix {
            return Err(OfferError::NotYetValid);
        }
        if now >= self.expires_unix {
            return Err(OfferError::Expired);
        }
        let mut labels = BTreeSet::new();
        let mut roots = BTreeSet::new();
        let mut total = 0u64;
        for file in &self.files {
            if !valid_label(&file.label)
                || file.blake3_root.iter().all(|&b| b == 0)
                || !labels.insert(file.label.to_lowercase())
                || !roots.insert(file.blake3_root)
            {
                return Err(OfferError::InvalidMetadata);
            }
            if file.bytes == 0 || file.bytes > MAX_ITEM_BYTES {
                return Err(OfferError::TooLarge);
            }
            total = total.checked_add(file.bytes).ok_or(OfferError::TooLarge)?;
            if total > MAX_BATCH_BYTES {
                return Err(OfferError::TooLarge);
            }
        }
        Ok(())
    }

    fn check_peers(&self, sender: PeerBinding, receiver: PeerBinding) -> Result<(), OfferError> {
        if self.sender != sender || self.receiver != receiver {
            return Err(OfferError::WrongIdentity);
        }
        Ok(())
    }

    pub fn decode_for_receiver(
        wire: &[u8],
        now: u64,
        authenticated_sender: PeerBinding,
        trusted_local_receiver: PeerBinding,
    ) -> Result<Self, OfferError> {
        if wire.is_empty() || wire.len() > MAX_OFFER_WIRE {
            return Err(OfferError::TooLarge);
        }
        let offer: Self = serde_json::from_slice(wire).map_err(|_| OfferError::InvalidEncoding)?;
        offer.validate(now)?;
        offer.check_peers(authenticated_sender, trusted_local_receiver)?;
        Ok(offer)
    }

    pub fn encode(&self, now: u64) -> Result<Vec<u8>, OfferError> {
        self.validate(now)?;
        let data = serde_json::to_vec(self).map_err(|_| OfferError::InvalidEncoding)?;
        if data.len() > MAX_OFFER_WIRE {
            return Err(OfferError::TooLarge);
        }
        Ok(data)
    }

    /// Domain-separated semantic hash, NOT a signature or permission grant.
    pub fn commitment(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new_derive_key("Akula metadata P2P offer v1");
        h.update(&self.schema.to_be_bytes());
        h.update(self.offer_id.as_bytes());
        h.update(&self.nonce);
        for peer in [self.sender, self.receiver] {
            h.update(peer.device_id.as_uuid().as_bytes());
            h.update(&peer.endpoint_id);
        }
        h.update(&self.created_unix.to_be_bytes());
        h.update(&self.expires_unix.to_be_bytes());
        h.update(&[self.files.len() as u8]);
        for file in &self.files {
            h.update(&(file.label.len() as u16).to_be_bytes());
            h.update(file.label.as_bytes());
            h.update(&file.bytes.to_be_bytes());
            h.update(&file.blake3_root);
        }
        *h.finalize().as_bytes()
    }

    /// Recipient constructs only a decision proposal. Sender MUST revalidate
    /// on a separately authenticated connection; policy authority is absent.
    pub fn receiver_decision(
        &self,
        action: DecisionAction,
        selected: Vec<u8>,
        now: u64,
        authenticated_sender: PeerBinding,
        trusted_local_receiver: PeerBinding,
    ) -> Result<DecisionV1, OfferError> {
        self.validate(now)?;
        self.check_peers(authenticated_sender, trusted_local_receiver)?;
        selection(self, action, &selected)?;
        Ok(DecisionV1 {
            schema: SCHEMA,
            offer_id: self.offer_id,
            manifest_commitment: self.commitment(),
            action,
            selected,
        })
    }
}

impl DecisionV1 {
    pub fn decode(wire: &[u8]) -> Result<Self, OfferError> {
        if wire.is_empty() || wire.len() > MAX_DECISION_WIRE {
            return Err(OfferError::TooLarge);
        }
        serde_json::from_slice(wire).map_err(|_| OfferError::InvalidEncoding)
    }

    pub fn encode(&self) -> Result<Vec<u8>, OfferError> {
        let data = serde_json::to_vec(self).map_err(|_| OfferError::InvalidEncoding)?;
        if data.len() > MAX_DECISION_WIRE {
            return Err(OfferError::TooLarge);
        }
        Ok(data)
    }
}

impl OfferSession {
    pub fn new_sender(
        offer: OfferV1,
        now: u64,
        trusted_sender: PeerBinding,
        enrolled_receiver: PeerBinding,
    ) -> Result<Self, OfferError> {
        offer.validate(now)?;
        offer.check_peers(trusted_sender, enrolled_receiver)?;
        Ok(Self {
            commitment: offer.commitment(),
            offer,
            phase: Phase::Pending,
        })
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn cancel(&mut self) -> Result<(), OfferError> {
        if self.phase != Phase::Pending {
            return Err(OfferError::AlreadyResolved);
        }
        self.phase = Phase::Cancelled;
        Ok(())
    }

    pub fn resolve(
        &mut self,
        decision: &DecisionV1,
        authenticated_receiver: PeerBinding,
        now: u64,
    ) -> Result<Outcome, OfferError> {
        if self.phase != Phase::Pending {
            return Err(OfferError::AlreadyResolved);
        }
        if now >= self.offer.expires_unix {
            self.phase = Phase::Expired;
            return Err(OfferError::Expired);
        }
        self.offer.validate(now)?;
        if authenticated_receiver != self.offer.receiver {
            return Err(OfferError::WrongIdentity);
        }
        if decision.schema != SCHEMA
            || decision.offer_id != self.offer.offer_id
            || decision.manifest_commitment != self.commitment
        {
            return Err(OfferError::WrongCommitment);
        }
        selection(&self.offer, decision.action, &decision.selected)?;
        match decision.action {
            DecisionAction::Decline => {
                self.phase = Phase::Declined;
                Ok(Outcome::Declined)
            }
            DecisionAction::Accept => {
                let files = decision
                    .selected
                    .iter()
                    .map(|&i| self.offer.files[usize::from(i)].clone())
                    .collect();
                self.phase = Phase::Accepted;
                Ok(Outcome::Selected(SelectionDraft {
                    offer_id: self.offer.offer_id,
                    commitment: self.commitment,
                    files,
                }))
            }
        }
    }
}

fn selection(offer: &OfferV1, action: DecisionAction, indices: &[u8]) -> Result<(), OfferError> {
    match action {
        DecisionAction::Decline if indices.is_empty() => return Ok(()),
        DecisionAction::Decline => return Err(OfferError::InvalidSelection),
        DecisionAction::Accept if indices.is_empty() || indices.len() > offer.files.len() => {
            return Err(OfferError::InvalidSelection)
        }
        DecisionAction::Accept => {}
    }
    let mut previous = None;
    for &index in indices {
        if usize::from(index) >= offer.files.len() || previous.is_some_and(|prev| index <= prev) {
            return Err(OfferError::InvalidSelection);
        }
        previous = Some(index);
    }
    Ok(())
}

fn valid_label(label: &str) -> bool {
    if label.is_empty() || label.len() > 180 || label == "." || label == ".."
        || label.ends_with(' ') || label.ends_with('.')
        || label.chars().any(|c| c.is_control()
            || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
            || matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
        return false;
    }
    let upper = label.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return false;
    }
    let bytes = upper.as_bytes();
    if bytes.len() == 4
        && (&bytes[..3] == b"COM" || &bytes[..3] == b"LPT")
        && (b'1'..=b'9').contains(&bytes[3])
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;
    fn peer(id: u128, key: u8) -> PeerBinding {
        PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(id)),
            endpoint_id: [key; 32],
        }
    }
    fn offer() -> OfferV1 {
        OfferV1 {
            schema: SCHEMA,
            offer_id: Uuid::from_u128(99),
            nonce: [7; 16],
            sender: peer(1, 3),
            receiver: peer(2, 4),
            created_unix: NOW,
            expires_unix: NOW + 120,
            files: vec![
                OfferedFile {
                    label: "first.txt".into(),
                    bytes: 3,
                    blake3_root: [1; 32],
                },
                OfferedFile {
                    label: "second.bin".into(),
                    bytes: 7,
                    blake3_root: [2; 32],
                },
                OfferedFile {
                    label: "third.txt".into(),
                    bytes: 11,
                    blake3_root: [3; 32],
                },
            ],
        }
    }
    fn decision(x: &OfferV1, selected: Vec<u8>) -> DecisionV1 {
        x.receiver_decision(
            DecisionAction::Accept,
            selected,
            NOW + 1,
            x.sender,
            x.receiver,
        )
        .unwrap()
    }

    #[test]
    fn roundtrip_partial_accept_has_only_ui_draft() {
        let x = offer();
        let wire = x.encode(NOW).unwrap();
        let receiver = OfferV1::decode_for_receiver(&wire, NOW + 1, x.sender, x.receiver).unwrap();
        let d = decision(&receiver, vec![0, 2]);
        let d = DecisionV1::decode(&d.encode().unwrap()).unwrap();
        let mut session = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
        let Outcome::Selected(draft) = session.resolve(&d, x.receiver, NOW + 2).unwrap() else {
            panic!("not a selection");
        };
        assert_eq!(
            draft
                .files
                .iter()
                .map(|x| x.label.as_str())
                .collect::<Vec<_>>(),
            vec!["first.txt", "third.txt"]
        );
        assert_eq!(session.phase(), Phase::Accepted);
    }

    #[test]
    fn forgery_of_transport_identity_or_application_device_is_rejected() {
        let x = offer();
        let wire = x.encode(NOW).unwrap();
        assert_eq!(
            OfferV1::decode_for_receiver(&wire, NOW, peer(44, 3), x.receiver),
            Err(OfferError::WrongIdentity)
        );
        assert_eq!(
            OfferV1::decode_for_receiver(&wire, NOW, x.sender, peer(2, 77)),
            Err(OfferError::WrongIdentity)
        );
        let mut session = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
        assert_eq!(
            session.resolve(&decision(&x, vec![0]), peer(44, 44), NOW + 1),
            Err(OfferError::WrongIdentity)
        );
        assert_eq!(session.phase(), Phase::Pending);
    }

    #[test]
    fn changing_manifest_size_root_or_offer_id_invalidates_decision() {
        let x = offer();
        let d = decision(&x, vec![0]);
        for n in 0..3 {
            let mut altered = x.clone();
            if n == 0 {
                altered.files[0].bytes += 1;
            }
            if n == 1 {
                altered.files[0].blake3_root = [88; 32];
            }
            if n == 2 {
                altered.offer_id = Uuid::from_u128(100);
            }
            let mut s =
                OfferSession::new_sender(altered.clone(), NOW, altered.sender, altered.receiver)
                    .unwrap();
            assert_eq!(
                s.resolve(&d, altered.receiver, NOW + 1),
                Err(OfferError::WrongCommitment)
            );
        }
    }

    #[test]
    fn accept_is_single_use_and_cancellation_is_terminal() {
        let x = offer();
        let mut s = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
        let d = decision(&x, vec![0]);
        s.resolve(&d, x.receiver, NOW + 1).unwrap();
        assert_eq!(
            s.resolve(&d, x.receiver, NOW + 2),
            Err(OfferError::AlreadyResolved)
        );
        let mut other = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
        other.cancel().unwrap();
        assert_eq!(
            other.resolve(&d, x.receiver, NOW + 1),
            Err(OfferError::AlreadyResolved)
        );
    }

    #[test]
    fn decline_is_terminal_and_carries_no_selected_files() {
        let x = offer();
        let d = x
            .receiver_decision(DecisionAction::Decline, vec![], NOW, x.sender, x.receiver)
            .unwrap();
        let mut s = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
        assert_eq!(s.resolve(&d, x.receiver, NOW + 1), Ok(Outcome::Declined));
        assert_eq!(
            s.resolve(&decision(&x, vec![0]), x.receiver, NOW + 2),
            Err(OfferError::AlreadyResolved)
        );
    }

    #[test]
    fn nil_application_device_identity_is_rejected() {
        let mut bad = offer();
        bad.sender.device_id = DeviceId::from_uuid(Uuid::nil());
        assert_eq!(bad.validate(NOW), Err(OfferError::InvalidMetadata));

        bad = offer();
        bad.receiver.device_id = DeviceId::from_uuid(Uuid::nil());
        assert_eq!(bad.validate(NOW), Err(OfferError::InvalidMetadata));
    }

    #[test]
    fn expiry_and_future_time_fail_closed() {
        let x = offer();
        assert_eq!(x.validate(NOW - 1), Err(OfferError::NotYetValid));
        assert_eq!(x.validate(NOW + 120), Err(OfferError::Expired));
        let mut s = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
        assert_eq!(
            s.resolve(&decision(&x, vec![0]), x.receiver, NOW + 120),
            Err(OfferError::Expired)
        );
        assert_eq!(s.phase(), Phase::Expired);
    }

    #[test]
    fn invalid_selection_is_rejected() {
        let x = offer();
        for bad in [vec![], vec![1, 1], vec![2, 0], vec![3], vec![0, 1, 2, 3]] {
            assert_eq!(
                x.receiver_decision(DecisionAction::Accept, bad, NOW, x.sender, x.receiver),
                Err(OfferError::InvalidSelection)
            );
        }
        assert_eq!(
            x.receiver_decision(DecisionAction::Decline, vec![0], NOW, x.sender, x.receiver),
            Err(OfferError::InvalidSelection)
        );
    }

    #[test]
    fn reject_quota_duplicate_and_dangerous_labels() {
        let x = offer();
        let mut bad = x.clone();
        bad.files[0].bytes = MAX_ITEM_BYTES + 1;
        assert_eq!(bad.validate(NOW), Err(OfferError::TooLarge));
        bad = x.clone();
        bad.files[1].label = "FIRST.TXT".into();
        assert_eq!(bad.validate(NOW), Err(OfferError::InvalidMetadata));
        bad = x.clone();
        bad.files[1].blake3_root = [1; 32];
        assert_eq!(bad.validate(NOW), Err(OfferError::InvalidMetadata));
        for label in [
            "../x",
            "folder\\x",
            "CON",
            "LPT9.txt",
            "file.",
            "a\u{202e}txt",
        ] {
            bad = x.clone();
            bad.files[0].label = label.into();
            assert_eq!(
                bad.validate(NOW),
                Err(OfferError::InvalidMetadata),
                "{label}"
            );
        }
        bad = x;
        bad.files = (0u8..9)
            .map(|i| OfferedFile {
                label: format!("file{i}"),
                bytes: 1,
                blake3_root: [i + 1; 32],
            })
            .collect();
        assert_eq!(bad.validate(NOW), Err(OfferError::TooLarge));
    }

    #[test]
    fn unknown_fields_bad_schema_and_oversized_frame_refused() {
        let x = offer();
        let mut doc = serde_json::to_value(&x).unwrap();
        doc["files"][0]["writeVault"] = serde_json::json!(true);
        assert_eq!(
            OfferV1::decode_for_receiver(
                &serde_json::to_vec(&doc).unwrap(),
                NOW,
                x.sender,
                x.receiver
            ),
            Err(OfferError::InvalidEncoding)
        );
        let mut x2 = x.clone();
        x2.schema = 9;
        assert_eq!(x2.validate(NOW), Err(OfferError::UnsupportedSchema));
        assert_eq!(
            OfferV1::decode_for_receiver(
                &vec![b' '; MAX_OFFER_WIRE + 1],
                NOW,
                x.sender,
                x.receiver
            ),
            Err(OfferError::TooLarge)
        );
        assert_eq!(
            DecisionV1::decode(&vec![b' '; MAX_DECISION_WIRE + 1]),
            Err(OfferError::TooLarge)
        );
    }

    #[test]
    fn wrong_decision_binding_and_unknown_authority_field_fail() {
        let x = offer();
        let mut d = decision(&x, vec![0]);
        let mut s = OfferSession::new_sender(x.clone(), NOW, x.sender, x.receiver).unwrap();
        d.manifest_commitment = [99; 32];
        assert_eq!(
            s.resolve(&d, x.receiver, NOW + 1),
            Err(OfferError::WrongCommitment)
        );
        let mut json = serde_json::to_value(&d).unwrap();
        json["grant"] = serde_json::json!(true);
        assert_eq!(
            DecisionV1::decode(&serde_json::to_vec(&json).unwrap()),
            Err(OfferError::InvalidEncoding)
        );
        assert_eq!(s.phase(), Phase::Pending);
    }
}
