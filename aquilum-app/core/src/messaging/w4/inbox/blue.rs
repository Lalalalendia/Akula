//! Blue W3 v2 -> Akula W4 inbox boundary with a single-database revocation
//! fence. Available only behind messaging-blue-w3-lab (NOT production UI).
//!
//! The W4 key ledger and scoped W3 route policy MUST live in the SAME
//! SQLite file as W4Inbox. Separate SQLite files, a TOFU registry or a
//! network-provided signing key cannot enforce the commit/revocation fence.
//!
//! An authorized host must eventually mint ApprovedInboxRoute and
//! ApprovedInboxRevocation. Neither has a public constructor. Until a real
//! host identity/enrollment flow exists, production network callers cannot
//! authorize any route. Tests create proofs only inside this private module.

use std::cell::Cell;

use akula_blue_w3::{
    self as blue, OpenContext, RecipientSecret, TrustedSenderKeys, VerifiedW3, W3Error,
};
use ed25519_dalek::VerifyingKey;
use ring::signature::{Ed25519KeyPair, KeyPair};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use super::{AuthenticatedW3, InboundResult, W4Inbox};
use crate::messaging::w4::Error;
use crate::sync::DeviceId;

/// Privileged approval from the future Akula host identity/permission flow.
/// Ordinary network callers CANNOT construct this value.
#[derive(Clone, Copy, Debug)]
pub struct ApprovedInboxRoute {
    sender: DeviceId,
    recipient: DeviceId,
    sender_epoch: u64,
    evidence_digest: [u8; 32],
}

/// Privileged route revocation, never inferred from unauthenticated gossip.
#[derive(Clone, Copy, Debug)]
pub struct ApprovedInboxRevocation {
    sender: DeviceId,
    recipient: DeviceId,
    sender_epoch: u64,
    evidence_digest: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteOutcome {
    Allowed,
    Duplicate,
    Rotated,
    Revoked,
}

/// A Blue key lookup is a crypto-verification SNAPSHOT. It is deliberately
/// not sufficient for later SQLite receipt acceptance.
struct SqliteSenderSnapshot<'db> {
    db: &'db Connection,
    observed: Cell<Option<[u8; 32]>>,
}

impl TrustedSenderKeys for SqliteSenderSnapshot<'_> {
    fn key_for(&self, sender_id: [u8; 16], epoch: u64) -> Option<VerifyingKey> {
        if epoch == 0 || epoch > i64::MAX as u64 {
            return None;
        }
        let row: Option<(i64, Vec<u8>, i64)> = self
            .db
            .query_row(
                "SELECT epoch,ed25519,revoked FROM messaging_w4_trusted_devices
                 WHERE device=?1",
                params![sender_id.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .ok()
            .flatten();
        let (stored_epoch, key, revoked) = row?;
        if stored_epoch != epoch as i64 || revoked != 0 {
            return None;
        }
        let exact: [u8; 32] = key.try_into().ok()?;
        let verifying_key = VerifyingKey::from_bytes(&exact).ok()?;
        self.observed.set(Some(exact));
        Some(verifying_key)
    }
}

fn current_key(
    tx: &Transaction<'_>,
    device: [u8; 16],
    epoch: u64,
) -> Result<Option<[u8; 32]>, Error> {
    if epoch == 0 || epoch > i64::MAX as u64 {
        return Ok(None);
    }
    let row: Option<(i64, Vec<u8>, i64)> = tx
        .query_row(
            "SELECT epoch,ed25519,revoked FROM messaging_w4_trusted_devices
             WHERE device=?1",
            params![device.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((stored_epoch, key, revoked)) = row else {
        return Ok(None);
    };
    if stored_epoch != epoch as i64 || revoked != 0 {
        return Ok(None);
    }
    Ok(key.try_into().ok())
}

fn route_is_current(
    tx: &Transaction<'_>,
    sender: [u8; 16],
    recipient: [u8; 16],
    epoch: u64,
) -> Result<bool, Error> {
    if epoch == 0 || epoch > i64::MAX as u64 {
        return Ok(false);
    }
    let route: Option<(i64, i64)> = tx
        .query_row(
            "SELECT sender_epoch,revoked FROM messaging_w3_allowed_routes
             WHERE sender=?1 AND recipient=?2",
            params![sender.as_slice(), recipient.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(matches!(route, Some((stored, 0)) if stored == epoch as i64))
}

fn valid_identity_pair(sender: DeviceId, recipient: DeviceId) -> bool {
    sender.as_uuid().into_bytes() != [0; 16]
        && recipient.as_uuid().into_bytes() != [0; 16]
}

fn map_crypto_failure(error: W3Error) -> Error {
    match error {
        W3Error::Oversized | W3Error::InvalidPacket | W3Error::UnsupportedVersion => {
            Error::InvalidPacket
        }
        W3Error::InvalidDescriptor | W3Error::WrongRecipient => Error::InvalidDescriptor,
        W3Error::Expired => Error::Expired,
        W3Error::UntrustedSender => Error::Unauthorized,
        W3Error::InvalidSignature | W3Error::DecryptFailed => Error::InvalidSignature,
    }
}

impl W4Inbox {
    /// A route may be approved ONLY by host code possessing a sealed proof.
    /// The current sender's pinned signing key must already be in the shared
    /// W4 key ledger. Revocation is sticky for the same epoch.
    pub fn allow_route(
        &mut self,
        approved: &ApprovedInboxRoute,
    ) -> Result<RouteOutcome, Error> {
        if !valid_identity_pair(approved.sender, approved.recipient)
            || approved.sender_epoch == 0
            || approved.sender_epoch > i64::MAX as u64
            || approved.evidence_digest == [0; 32]
        {
            return Err(Error::InvalidDescriptor);
        }
        let sender = approved.sender.as_uuid().into_bytes();
        let recipient = approved.recipient.as_uuid().into_bytes();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if current_key(&tx, sender, approved.sender_epoch)?.is_none() {
            return Err(Error::Unauthorized);
        }
        let previous: Option<(i64, Vec<u8>, i64)> = tx
            .query_row(
                "SELECT sender_epoch,evidence,revoked FROM messaging_w3_allowed_routes
                 WHERE sender=?1 AND recipient=?2",
                params![sender.as_slice(), recipient.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let outcome = match previous {
            None => {
                tx.execute(
                    "INSERT INTO messaging_w3_allowed_routes
                     (sender,recipient,sender_epoch,evidence,revoked)
                     VALUES(?1,?2,?3,?4,0)",
                    params![
                        sender.as_slice(),
                        recipient.as_slice(),
                        approved.sender_epoch as i64,
                        approved.evidence_digest.as_slice(),
                    ],
                )?;
                RouteOutcome::Allowed
            }
            Some((epoch, evidence, revoked)) => {
                if epoch <= 0 || approved.sender_epoch < epoch as u64 {
                    return Err(Error::Unauthorized);
                }
                if approved.sender_epoch == epoch as u64 {
                    if revoked != 0 {
                        return Err(Error::Unauthorized);
                    }
                    if evidence.as_slice() != approved.evidence_digest.as_slice() {
                        return Err(Error::Conflict);
                    }
                    RouteOutcome::Duplicate
                } else {
                    if evidence.as_slice() == approved.evidence_digest.as_slice() {
                        return Err(Error::Unauthorized);
                    }
                    tx.execute(
                        "UPDATE messaging_w3_allowed_routes
                         SET sender_epoch=?3,evidence=?4,revoked=0
                         WHERE sender=?1 AND recipient=?2",
                        params![
                            sender.as_slice(),
                            recipient.as_slice(),
                            approved.sender_epoch as i64,
                            approved.evidence_digest.as_slice(),
                        ],
                    )?;
                    RouteOutcome::Rotated
                }
            }
        };
        tx.commit()?;
        Ok(outcome)
    }

    /// Host-only, epoch-scoped revoke; serialized with receipt commit by
    /// BEGIN IMMEDIATE against the same SQLite file.
    pub fn revoke_route(
        &mut self,
        approved: &ApprovedInboxRevocation,
    ) -> Result<RouteOutcome, Error> {
        if !valid_identity_pair(approved.sender, approved.recipient)
            || approved.sender_epoch == 0
            || approved.sender_epoch > i64::MAX as u64
            || approved.evidence_digest == [0; 32]
        {
            return Err(Error::InvalidDescriptor);
        }
        let sender = approved.sender.as_uuid().into_bytes();
        let recipient = approved.recipient.as_uuid().into_bytes();
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<(i64, i64)> = tx
            .query_row(
                "SELECT sender_epoch,revoked FROM messaging_w3_allowed_routes
                 WHERE sender=?1 AND recipient=?2",
                params![sender.as_slice(), recipient.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((epoch, revoked)) = previous else {
            return Err(Error::Unauthorized);
        };
        if epoch != approved.sender_epoch as i64 {
            return Err(Error::Unauthorized);
        }
        if revoked != 0 {
            tx.commit()?;
            return Ok(RouteOutcome::Duplicate);
        }
        tx.execute(
            "UPDATE messaging_w3_allowed_routes SET revoked=1,evidence=?3
             WHERE sender=?1 AND recipient=?2 AND sender_epoch=?4",
            params![
                sender.as_slice(),
                recipient.as_slice(),
                approved.evidence_digest.as_slice(),
                approved.sender_epoch as i64,
            ],
        )?;
        tx.commit()?;
        Ok(RouteOutcome::Revoked)
    }

    /// Genuine Blue verification of the supplied W3 ciphertext, followed by
    /// policy checks under the SAME SQLite transaction as W4 receipt storage.
    ///
    /// The signer is the local RECIPIENT's W4 key. Its epoch is intentionally
    /// independent of the W3 sender key's epoch. Transport ACKs never count.
    /// There is no production route approval constructor, hence this cannot
    /// become an unreviewed network ingress route.
    pub fn accept_blue_packet(
        &mut self,
        packet: &[u8],
        local_secret: &RecipientSecret,
        local_device: DeviceId,
        recipient_signer: &Ed25519KeyPair,
        recipient_epoch: u64,
        now_ms: u64,
    ) -> Result<InboundResult, Error> {
        let snapshot = SqliteSenderSnapshot {
            db: &self.db,
            observed: Cell::new(None),
        };
        let verified = blue::open_verified(
            packet,
            OpenContext {
                secret: local_secret,
                recipient_id: local_device.as_uuid().into_bytes(),
                now_ms,
                trusted_sender_keys: &snapshot,
            },
        )
        .map_err(map_crypto_failure)?;
        let signer_key = snapshot.observed.get().ok_or(Error::Unauthorized)?;
        self.commit_blue_verified(
            verified,
            signer_key,
            local_device,
            recipient_signer,
            recipient_epoch,
            now_ms,
        )
    }

    /// Private transition from cryptographically verified Blue W3 to the
    /// equally private AuthenticatedW3, then to a durable recipient W4 ACK.
    /// Rechecks the exact sender key, scoped route, and local recipient key
    /// under the same IMMEDIATE write lock as the inbox insert/receipt.
    fn commit_blue_verified(
        &mut self,
        verified: VerifiedW3<'_>,
        verified_sender_key: [u8; 32],
        local_device: DeviceId,
        recipient_signer: &Ed25519KeyPair,
        recipient_epoch: u64,
        now_ms: u64,
    ) -> Result<InboundResult, Error> {
        let header = verified.header();
        if header.recipient_id != local_device.as_uuid().into_bytes()
            || header.message_id == [0; 16]
            || header.epoch == 0
            || header.epoch > i64::MAX as u64
            || recipient_epoch == 0
            || recipient_epoch > i64::MAX as u64
            || header.expires_at_ms > i64::MAX as u64
            || now_ms >= header.expires_at_ms
        {
            return Err(Error::InvalidDescriptor);
        }
        if verified.original_packet().len() > super::MAX_ENVELOPE
            || verified.original_packet().is_empty()
            || *blake3::hash(verified.original_packet()).as_bytes() != verified.packet_digest()
        {
            return Err(Error::Mismatch);
        }

        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if current_key(&tx, header.sender_id, header.epoch)?
            != Some(verified_sender_key)
            || !route_is_current(
                &tx,
                header.sender_id,
                header.recipient_id,
                header.epoch,
            )?
        {
            return Err(Error::Unauthorized);
        }
        let expected_recipient = current_key(&tx, header.recipient_id, recipient_epoch)?
            .ok_or(Error::Unauthorized)?;
        if recipient_signer.public_key().as_ref() != expected_recipient.as_slice() {
            return Err(Error::Unauthorized);
        }
        // W3 sender epoch and W4 recipient-signing epoch are DISTINCT.
        let input = AuthenticatedW3 {
            sender: DeviceId::from_uuid(uuid::Uuid::from_bytes(header.sender_id)),
            recipient: local_device,
            message: header.message_id,
            packet: verified.original_packet(),
            digest: verified.packet_digest(),
            epoch: recipient_epoch,
            expires_at_ms: header.expires_at_ms,
        };
        W4Inbox::persist_in_transaction(tx, &input, recipient_signer)
    }
}

#[cfg(test)]
mod tests;
