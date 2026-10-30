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

pub mod receipt_binding;

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
    accepted_indices: Option<Vec<u8>>,
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
            accepted_indices: None,
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
                self.accepted_indices = Some(decision.selected.clone());
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
mod tests;
