//! WOW W4 v1 (137 bytes): experimental signed receipts/SQLite outbox, not transport or trust.

use ring::signature::{UnparsedPublicKey, ED25519};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::Path;
use std::time::Duration;

use super::{Descriptor, ReceiptCandidate, ReceiptVerifier};
use crate::sync::DeviceId;

const DOMAIN: &[u8] = b"Akula/WOW/recipient-ack/v1\0";
const SIGNED_LEN: usize = 73;
pub const RECEIPT_LEN: usize = 137;
pub const MAX_ENVELOPE: usize = 16_384;

/// Only independently approved device keys are trusted; never network-observed keys.
pub trait TrustedRecipientKeys {
    fn key_for(&self, recipient: DeviceId, epoch: u64) -> Option<[u8; 32]>;
}

#[derive(Debug)]
pub enum Error {
    Database(rusqlite::Error),
    InvalidDescriptor,
    InvalidPacket,
    InvalidReceipt,
    Unauthorized,
    InvalidSignature,
    Mismatch,
    Conflict,
    Expired,
    Missing,
}
impl From<rusqlite::Error> for Error {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value)
    }
}

/// Cannot be constructed by callers without signature verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedReceipt {
    recipient: DeviceId,
    message: [u8; 16],
    digest: [u8; 32],
    epoch: u64,
}
impl VerifiedReceipt {
    pub fn recipient(self) -> DeviceId {
        self.recipient
    }
    pub fn message(self) -> [u8; 16] {
        self.message
    }
    pub fn digest(self) -> [u8; 32] {
        self.digest
    }
    pub fn epoch(self) -> u64 {
        self.epoch
    }
}

/// Verify the exact domain-separated WOW W4 wire using real Ed25519.
pub fn verify_receipt(
    raw: &[u8],
    keys: &impl TrustedRecipientKeys,
) -> Result<VerifiedReceipt, Error> {
    if raw.len() != RECEIPT_LEN || raw[0] != 1 {
        return Err(Error::InvalidReceipt);
    }
    let mut recipient = [0; 16];
    let mut message = [0; 16];
    let mut digest = [0; 32];
    recipient.copy_from_slice(&raw[1..17]);
    message.copy_from_slice(&raw[17..33]);
    digest.copy_from_slice(&raw[33..65]);
    let epoch = u64::from_be_bytes(raw[65..73].try_into().map_err(|_| Error::InvalidReceipt)?);
    let recipient = DeviceId::from_uuid(uuid::Uuid::from_bytes(recipient));
    let key = keys.key_for(recipient, epoch).ok_or(Error::Unauthorized)?;
    let mut signed = Vec::with_capacity(DOMAIN.len() + SIGNED_LEN);
    signed.extend_from_slice(DOMAIN);
    signed.extend_from_slice(&raw[..SIGNED_LEN]);
    UnparsedPublicKey::new(&ED25519, key)
        .verify(&signed, &raw[SIGNED_LEN..])
        .map_err(|_| Error::InvalidSignature)?;
    Ok(VerifiedReceipt {
        recipient,
        message,
        digest,
        epoch,
    })
}

/// The same real signature verifier can drive CORE-01A's pure state projection.
pub struct W4Verifier<'a, K: TrustedRecipientKeys> {
    pub keys: &'a K,
}
impl<K: TrustedRecipientKeys> ReceiptVerifier for W4Verifier<'_, K> {
    fn verify(&self, candidate: &ReceiptCandidate<'_>) -> bool {
        let Ok(r) = verify_receipt(candidate.signature, self.keys) else {
            return false;
        };
        r.recipient == candidate.recipient
            && r.message == candidate.id
            && r.digest == candidate.ciphertext_digest
            && r.epoch == candidate.policy_epoch
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Insert {
    New,
    Duplicate,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Accept {
    New,
    Duplicate,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Queued,
    Accepted,
}

/// One authoritative, local, durable sender outbox. Not Sync Core's file DB.
pub struct W4Outbox {
    db: Connection,
}
impl W4Outbox {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS messaging_w4_outbox(
                recipient BLOB NOT NULL CHECK(length(recipient)=16),
                message BLOB NOT NULL CHECK(length(message)=16),
                digest BLOB NOT NULL CHECK(length(digest)=32),
                ciphertext BLOB NOT NULL CHECK(length(ciphertext) BETWEEN 1 AND 16384),
                epoch INTEGER NOT NULL CHECK(epoch>0),
                expiry_ms INTEGER NOT NULL CHECK(expiry_ms>0),
                delivered INTEGER NOT NULL DEFAULT 0 CHECK(delivered IN(0,1)),
                receipt BLOB CHECK(receipt IS NULL OR length(receipt)=137),
                PRIMARY KEY(recipient,message),
                CHECK((delivered=0 AND receipt IS NULL) OR (delivered=1 AND receipt IS NOT NULL))
             );",
        )?;
        Ok(Self { db })
    }

    /// Preconditions: caller already validated the encrypted WOW W3 envelope,
    /// recipient/epoch authorization and the source of the virtual/real time.
    /// This adapter validates length and BLAKE3 only; it CANNOT detect plaintext.
    pub fn enqueue(&mut self, d: Descriptor, packet: &[u8], now: u64) -> Result<Insert, Error> {
        if d.id == [0; 16]
            || d.policy_epoch == 0
            || d.expires_at_ms <= now
            || d.policy_epoch > i64::MAX as u64
            || d.expires_at_ms > i64::MAX as u64
        {
            return Err(Error::InvalidDescriptor);
        }
        if packet.is_empty() || packet.len() > MAX_ENVELOPE {
            return Err(Error::InvalidPacket);
        }
        let digest = *blake3::hash(packet).as_bytes();
        if digest != d.ciphertext_digest {
            return Err(Error::Mismatch);
        }
        let recipient = d.recipient.as_uuid().into_bytes();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(Vec<u8>, Vec<u8>, i64, i64)> = tx
            .query_row(
                "SELECT digest,ciphertext,epoch,expiry_ms FROM messaging_w4_outbox
             WHERE recipient=?1 AND message=?2",
                params![recipient.as_slice(), d.id.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        if let Some((old_digest, old_packet, old_epoch, old_expiry)) = old {
            if old_digest.as_slice() != digest.as_slice()
                || old_packet.as_slice() != packet
                || old_epoch != d.policy_epoch as i64
                || old_expiry != d.expires_at_ms as i64
            {
                return Err(Error::Conflict);
            }
            tx.commit()?;
            return Ok(Insert::Duplicate);
        }
        tx.execute(
            "INSERT INTO messaging_w4_outbox(recipient,message,digest,ciphertext,epoch,expiry_ms)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                recipient.as_slice(),
                d.id.as_slice(),
                digest.as_slice(),
                packet,
                d.policy_epoch as i64,
                d.expires_at_ms as i64
            ],
        )?;
        tx.commit()?;
        Ok(Insert::New)
    }

    /// Return identical original bytes only while current trusted epoch allows it.
    pub fn retry(
        &self,
        recipient: DeviceId,
        message: [u8; 16],
        now: u64,
        keys: &impl TrustedRecipientKeys,
    ) -> Result<Option<Vec<u8>>, Error> {
        let device = recipient.as_uuid().into_bytes();
        let row: Option<(Vec<u8>, Vec<u8>, i64, i64, i64)> = self
            .db
            .query_row(
                "SELECT ciphertext,digest,epoch,expiry_ms,delivered FROM messaging_w4_outbox
             WHERE recipient=?1 AND message=?2",
                params![device.as_slice(), message.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (ciphertext, digest, epoch, expiry, delivered) = row.ok_or(Error::Missing)?;
        if epoch <= 0 || keys.key_for(recipient, epoch as u64).is_none() {
            return Err(Error::Unauthorized);
        }
        if delivered != 0 || now >= expiry as u64 {
            return Ok(None);
        }
        if blake3::hash(&ciphertext).as_bytes().as_slice() != digest.as_slice() {
            return Err(Error::Conflict);
        }
        Ok(Some(ciphertext))
    }

    /// A relay ACK CANNOT call this final acceptance method. Exact W4 signed
    /// recipient receipt and current Akula trusted signing-key mapping required.
    pub fn accept_receipt(
        &mut self,
        raw: &[u8],
        now: u64,
        keys: &impl TrustedRecipientKeys,
    ) -> Result<Accept, Error> {
        let verified = verify_receipt(raw, keys)?;
        let device = verified.recipient.as_uuid().into_bytes();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(Vec<u8>, i64, i64, i64, Option<Vec<u8>>)> = tx
            .query_row(
                "SELECT digest,epoch,expiry_ms,delivered,receipt FROM messaging_w4_outbox
             WHERE recipient=?1 AND message=?2",
                params![device.as_slice(), verified.message.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (digest, epoch, expiry, delivered, saved) = old.ok_or(Error::Missing)?;
        if digest.as_slice() != verified.digest.as_slice()
            || epoch <= 0
            || epoch as u64 != verified.epoch
        {
            return Err(Error::Mismatch);
        }
        if delivered == 1 {
            if saved.as_deref() != Some(raw) {
                return Err(Error::Conflict);
            }
            tx.commit()?;
            return Ok(Accept::Duplicate);
        }
        if now >= expiry as u64 {
            return Err(Error::Expired);
        }
        tx.execute(
            "UPDATE messaging_w4_outbox SET delivered=1,receipt=?3
             WHERE recipient=?1 AND message=?2 AND delivered=0",
            params![device.as_slice(), verified.message.as_slice(), raw],
        )?;
        tx.commit()?;
        Ok(Accept::New)
    }

    pub fn status(&self, recipient: DeviceId, message: [u8; 16]) -> Result<Option<Status>, Error> {
        let device = recipient.as_uuid().into_bytes();
        let status: Option<i64> = self
            .db
            .query_row(
                "SELECT delivered FROM messaging_w4_outbox WHERE recipient=?1 AND message=?2",
                params![device.as_slice(), message.as_slice()],
                |r| r.get(0),
            )
            .optional()?;
        match status {
            None => Ok(None),
            Some(0) => Ok(Some(Status::Queued)),
            Some(1) => Ok(Some(Status::Accepted)),
            _ => Err(Error::Conflict),
        }
    }
}

#[cfg(feature = "messaging-key-ledger-lab")]
pub mod key_ledger;

#[cfg(test)]
mod tests;
