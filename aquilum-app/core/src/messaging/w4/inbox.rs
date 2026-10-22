//! Receiver-side durable, ciphertext-only mailbox for the built-in Akula chat.
//!
//! IMPORTANT: this module deliberately does NOT expose a public constructor
//! for AuthenticatedW3. Only the future audited WOW W3 adapter can supply
//! authenticated sender identity, recipient, exact ciphertext and permissions.
//! This prevents the lab from claiming that arbitrary network bytes are safe.
//! The tests manufacture such inputs ONLY under cfg(test).
//!
//! The W4 recipient receipt is signed after verification, and persisted in the
//! SAME SQLite transaction as the immutable ciphertext. It is returned only
//! after commit. A relay acknowledgment is never a recipient receipt.

use std::path::Path;
use std::time::Duration;

use ring::signature::{Ed25519KeyPair, KeyPair};
use rusqlite::{Connection, TransactionBehavior};

use super::{Error, TrustedRecipientKeys, MAX_ENVELOPE};
use crate::sync::DeviceId;

const MAX_INBOX_ROWS: i64 = 4096;
const MAX_INBOX_PER_SENDER: i64 = 128;

/// Not constructible by regular callers. W3 verification MUST be implemented
/// before receiving any real message; no bypass/fake production constructor.
#[derive(Clone)]
pub struct AuthenticatedW3<'a> {
    sender: DeviceId,
    recipient: DeviceId,
    message: [u8; 16],
    packet: &'a [u8],
    digest: [u8; 32],
    epoch: u64,
    expires_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InboundStatus {
    Stored,
    Duplicate,
}
#[derive(Debug, Eq, PartialEq)]
pub struct InboundResult {
    pub status: InboundStatus,
    /// Durable WOW W4 recipient proof; transport must forward it unchanged.
    pub signed_receipt: Vec<u8>,
}

/// Unlike document HistoryService, this table is scoped to sender+MessageId,
/// stores only opaque encrypted packets, and supports exact replay of receipts.
pub struct W4Inbox {
    db: Connection,
}
impl W4Inbox {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS messaging_w4_inbox(
                sender BLOB NOT NULL CHECK(length(sender)=16),
                recipient BLOB NOT NULL CHECK(length(recipient)=16),
                message BLOB NOT NULL CHECK(length(message)=16),
                digest BLOB NOT NULL CHECK(length(digest)=32),
                ciphertext BLOB NOT NULL CHECK(length(ciphertext) BETWEEN 1 AND 16384),
                epoch INTEGER NOT NULL CHECK(epoch>0),
                expiry_ms INTEGER NOT NULL CHECK(expiry_ms>0),
                receipt BLOB NOT NULL CHECK(length(receipt)=137),
                PRIMARY KEY(sender,message)
             );",
        )?;
        #[cfg(feature = "messaging-blue-w3-lab")]
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS messaging_w4_trusted_devices(
                device BLOB PRIMARY KEY CHECK(length(device)=16),
                epoch INTEGER NOT NULL CHECK(epoch>0),
                ed25519 BLOB NOT NULL CHECK(length(ed25519)=32),
                evidence BLOB NOT NULL CHECK(length(evidence)=32),
                revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN(0,1))
             );
             CREATE TABLE IF NOT EXISTS messaging_w3_allowed_routes(
                sender BLOB NOT NULL CHECK(length(sender)=16),
                recipient BLOB NOT NULL CHECK(length(recipient)=16),
                sender_epoch INTEGER NOT NULL CHECK(sender_epoch>0),
                evidence BLOB NOT NULL CHECK(length(evidence)=32),
                revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN(0,1)),
                PRIMARY KEY(sender,recipient)
             );",
        )?;
        Ok(Self { db })
    }

    /// This is NOT a network entrypoint. Caller must already possess
    /// AuthenticatedW3 obtained from genuine WOW W3 decryption/validation.
    /// The recipient signer must also be pinned by Akula's trust authority
    /// for this exact device and epoch; never trust a key from a packet.
    ///
    /// Caveat: independent TrustedRecipientKeys lookup does not provide an
    /// atomic revocation fence against a *concurrent* authority change.
    pub fn accept_verified(
        &mut self,
        input: &AuthenticatedW3<'_>,
        signer: &Ed25519KeyPair,
        keys: &impl TrustedRecipientKeys,
        now_ms: u64,
    ) -> Result<InboundResult, Error> {
        if input.message == [0; 16]
            || input.epoch == 0
            || input.epoch > i64::MAX as u64
            || input.expires_at_ms > i64::MAX as u64
            || now_ms >= input.expires_at_ms
        {
            return Err(Error::InvalidDescriptor);
        }
        if input.packet.is_empty() || input.packet.len() > MAX_ENVELOPE {
            return Err(Error::InvalidPacket);
        }
        if blake3::hash(input.packet).as_bytes() != &input.digest {
            return Err(Error::Mismatch);
        }
        let pinned = keys
            .key_for(input.recipient, input.epoch)
            .ok_or(Error::Unauthorized)?;
        if signer.public_key().as_ref() != pinned.as_slice() {
            return Err(Error::Unauthorized);
        }

        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::persist_in_transaction(tx, input, signer)
    }

    #[cfg(test)]
    fn row_count(&self) -> Result<i64, Error> {
        Ok(self
            .db
            .query_row("SELECT COUNT(*) FROM messaging_w4_inbox", [], |r| r.get(0))?)
    }
}

mod persist;

#[cfg(feature = "messaging-blue-w3-lab")]
mod blue;

#[cfg(test)]
mod tests;
