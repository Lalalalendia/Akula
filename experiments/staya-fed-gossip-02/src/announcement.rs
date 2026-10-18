//! FED-GOSSIP-02 application-level announcement fixture.
//!
//! This is **not** the production Akula authorization system.
//! It verifies only explicitly granted announcement keys, canonical signed
//! metadata, a bounded lifetime, and per-provider replay fencing.
//! A valid announcement never proves a file has durable copies or grants a
//! right to read/write the referenced file.

use std::collections::{BTreeMap, BTreeSet};
use ed25519_dalek::{Signature, SigningKey, Signer, VerifyingKey};

const DOMAIN: &[u8] = b"akula/fed-gossip-02/announcement/v1";
const VERSION: u8 = 1;
const MAX_WIRE_BYTES: usize = 512;
const MAX_LIFETIME_MS: u64 = 300_000;
const MAX_CLOCK_SKEW_MS: u64 = 30_000;
const WIRE_BYTES: usize = 1 + 16 + 32 + 32 + 32 + 8 * 4 + 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    pub vault: [u8; 16],
    pub topic: [u8; 32],
    pub provider: [u8; 32],
    pub content_digest: [u8; 32],
    pub epoch: u64,
    pub sequence: u64,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub signature: [u8; 64],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    Oversize,
    Malformed,
    Signature,
    Forbidden,
    Expired,
    InvalidLifetime,
    Future,
    Replay,
}

impl Announcement {
    /// Deterministic test-only signing; caller supplies fixed test key.
    pub fn signed(
        signing_key: &SigningKey,
        vault: [u8; 16],
        topic: [u8; 32],
        content_digest: [u8; 32],
        epoch: u64,
        sequence: u64,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> Self {
        let mut value = Self {
            vault,
            topic,
            provider: signing_key.verifying_key().to_bytes(),
            content_digest,
            epoch,
            sequence,
            issued_at_ms,
            expires_at_ms,
            signature: [0; 64],
        };
        value.signature = signing_key.sign(&value.signing_material()).to_bytes();
        value
    }

    fn canonical_fields(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(WIRE_BYTES - 64);
        data.push(VERSION);
        data.extend_from_slice(&self.vault);
        data.extend_from_slice(&self.topic);
        data.extend_from_slice(&self.provider);
        data.extend_from_slice(&self.content_digest);
        for number in [self.epoch, self.sequence, self.issued_at_ms, self.expires_at_ms] {
            data.extend_from_slice(&number.to_be_bytes());
        }
        debug_assert_eq!(data.len(), WIRE_BYTES - 64);
        data
    }

    fn signing_material(&self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(DOMAIN.len() + WIRE_BYTES - 64);
        payload.extend_from_slice(DOMAIN);
        payload.extend_from_slice(&self.canonical_fields());
        payload
    }

    pub fn to_wire(&self) -> Vec<u8> {
        let mut out = self.canonical_fields();
        out.extend_from_slice(&self.signature);
        out
    }

    pub fn from_wire(raw: &[u8]) -> Result<Self, Reject> {
        if raw.len() > MAX_WIRE_BYTES {
            return Err(Reject::Oversize);
        }
        if raw.len() != WIRE_BYTES || raw[0] != VERSION {
            return Err(Reject::Malformed);
        }

        // Safe after the fixed length was verified.
        fn take<const N: usize>(bytes: &[u8], offset: &mut usize) -> [u8; N] {
            let mut out = [0u8; N];
            out.copy_from_slice(&bytes[*offset..*offset + N]);
            *offset += N;
            out
        }

        let mut i = 1;
        let vault = take::<16>(raw, &mut i);
        let topic = take::<32>(raw, &mut i);
        let provider = take::<32>(raw, &mut i);
        let content_digest = take::<32>(raw, &mut i);
        let epoch = u64::from_be_bytes(take::<8>(raw, &mut i));
        let sequence = u64::from_be_bytes(take::<8>(raw, &mut i));
        let issued_at_ms = u64::from_be_bytes(take::<8>(raw, &mut i));
        let expires_at_ms = u64::from_be_bytes(take::<8>(raw, &mut i));
        let signature = take::<64>(raw, &mut i);
        debug_assert_eq!(i, WIRE_BYTES);
        Ok(Self {
            vault,
            topic,
            provider,
            content_digest,
            epoch,
            sequence,
            issued_at_ms,
            expires_at_ms,
            signature,
        })
    }
}

type Scope = ([u8; 16], [u8; 32], [u8; 32]);

#[derive(Default)]
pub struct AnnouncementGate {
    allowed: BTreeSet<Scope>,
    // Only research-fixture, in-memory replay state. Must be made durable and
    // tied to Akula grant revocation before a real deployment.
    highwater: BTreeMap<Scope, (u64, u64)>,
}

impl AnnouncementGate {
    pub fn grant(&mut self, vault: [u8; 16], topic: [u8; 32], provider: [u8; 32]) {
        self.allowed.insert((vault, topic, provider));
    }

    pub fn revoke(&mut self, vault: [u8; 16], topic: [u8; 32], provider: [u8; 32]) {
        self.allowed.remove(&(vault, topic, provider));
        // Deliberately preserve highwater; re-granting must not allow replay.
    }

    /// Successful verification says only "authorized announcement accepted".
    pub fn accept(&mut self, raw: &[u8], now_ms: u64) -> Result<Announcement, Reject> {
        let message = Announcement::from_wire(raw)?;
        let scope = (message.vault, message.topic, message.provider);

        // Pre-authorization to avoid expensive crypto on strangers. This is
        // not a side-channel-safe privacy scheme and never grants access.
        if !self.allowed.contains(&scope) {
            return Err(Reject::Forbidden);
        }
        if message.sequence == 0
            || message.expires_at_ms <= message.issued_at_ms
            || message.expires_at_ms - message.issued_at_ms > MAX_LIFETIME_MS
        {
            return Err(Reject::InvalidLifetime);
        }
        if now_ms > message.expires_at_ms {
            return Err(Reject::Expired);
        }
        if message.issued_at_ms > now_ms.saturating_add(MAX_CLOCK_SKEW_MS) {
            return Err(Reject::Future);
        }

        let key = VerifyingKey::from_bytes(&message.provider).map_err(|_| Reject::Signature)?;
        key.verify_strict(
            &message.signing_material(),
            &Signature::from_bytes(&message.signature),
        ).map_err(|_| Reject::Signature)?;

        let event = (message.epoch, message.sequence);
        if self.highwater.get(&scope).is_some_and(|previous| *previous >= event) {
            return Err(Reject::Replay);
        }
        self.highwater.insert(scope, event);
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const VAULT: [u8; 16] = [17; 16];
    const TOPIC: [u8; 32] = [23; 32];
    const CONTENT: [u8; 32] = [34; 32];

    fn key() -> SigningKey { SigningKey::from_bytes(&[44; 32]) }
    fn mk(epoch: u64, seq: u64, issued: u64, expires: u64) -> Announcement {
        Announcement::signed(&key(), VAULT, TOPIC, CONTENT, epoch, seq, issued, expires)
    }
    fn gate() -> AnnouncementGate {
        let mut g = AnnouncementGate::default();
        g.grant(VAULT, TOPIC, key().verifying_key().to_bytes());
        g
    }

    #[test]
    fn valid_announcement_is_scoped_replay_safe_and_revocable() {
        let mut g = gate();
        let a = mk(1, 1, 1_000, 2_000);
        assert_eq!(a.to_wire().len(), WIRE_BYTES);
        assert_eq!(g.accept(&a.to_wire(), 1_001), Ok(a.clone()));
        assert_eq!(g.accept(&a.to_wire(), 1_002), Err(Reject::Replay));
        let next = mk(1, 2, 1_005, 2_005);
        assert!(g.accept(&next.to_wire(), 1_006).is_ok());
        assert_eq!(g.accept(&a.to_wire(), 1_007), Err(Reject::Replay));
        let who = key().verifying_key().to_bytes();
        g.revoke(VAULT, TOPIC, who);
        let later = mk(2, 1, 1_010, 2_010);
        assert_eq!(g.accept(&later.to_wire(), 1_011), Err(Reject::Forbidden));
        g.grant(VAULT, TOPIC, who);
        // Revocation did not clear sequence history.
        assert_eq!(g.accept(&a.to_wire(), 1_011), Err(Reject::Replay));
        assert!(g.accept(&later.to_wire(), 1_011).is_ok());
    }

    #[test]
    fn changed_content_same_sequence_and_signature_is_rejected() {
        let mut g = gate();
        let mut a = mk(7, 9, 1_000, 2_000).to_wire();
        a[90] ^= 0x01;
        assert_eq!(g.accept(&a, 1_001), Err(Reject::Signature));
    }

    #[test]
    fn wrong_vault_topic_provider_or_unsigned_stranger_are_forbidden() {
        let mut g = gate();
        let stranger = SigningKey::from_bytes(&[55; 32]);
        for (v, t, signing_key) in [
            ([18; 16], TOPIC, &key()),
            (VAULT, [24; 32], &key()),
            (VAULT, TOPIC, &stranger),
        ] {
            let m = Announcement::signed(signing_key, v, t, CONTENT, 1, 1, 1_000, 2_000);
            assert_eq!(g.accept(&m.to_wire(), 1_001), Err(Reject::Forbidden));
        }
    }

    #[test]
    fn clock_and_lifetime_checks_fail_closed() {
        let mut g = gate();
        assert_eq!(g.accept(&mk(1, 1, 1_000, 2_000).to_wire(), 2_001), Err(Reject::Expired));
        assert_eq!(g.accept(&mk(1, 1, 100_000, 100_100).to_wire(), 1_000), Err(Reject::Future));
        assert_eq!(g.accept(&mk(1, 1, 1_000, 301_001).to_wire(), 1_010), Err(Reject::InvalidLifetime));
        assert_eq!(g.accept(&mk(1, 0, 1_000, 1_500).to_wire(), 1_010), Err(Reject::InvalidLifetime));
        assert_eq!(g.accept(&mk(1, 1, 1_000, 1_000).to_wire(), 1_010), Err(Reject::InvalidLifetime));
        assert!(g.accept(&mk(1, 1, 1_000, 2_000).to_wire(), 1_010).is_ok());
    }

    #[test]
    fn malformed_version_truncation_and_oversize_are_rejected() {
        let mut g = gate();
        let good = mk(1, 1, 1_000, 2_000).to_wire();
        let mut wrong_version = good.clone();
        wrong_version[0] = 2;
        assert_eq!(g.accept(&wrong_version, 1_001), Err(Reject::Malformed));
        assert_eq!(g.accept(&good[..good.len() - 1], 1_001), Err(Reject::Malformed));
        assert_eq!(g.accept(&[0u8; 1024], 1_001), Err(Reject::Oversize));
    }

    #[test]
    fn identical_metadata_is_one_message_but_sequence_changes_message_id() {
        let a = mk(1, 1, 1_000, 2_000).to_wire();
        let b = mk(1, 1, 1_000, 2_000).to_wire();
        let c = mk(1, 2, 1_000, 2_000).to_wire();
        assert_eq!(blake3::hash(&a), blake3::hash(&b));
        assert_ne!(blake3::hash(&a), blake3::hash(&c));
    }
}
