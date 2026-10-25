//! Opt-in, SQLite-backed replay admission for authenticated Akula/WOW W3
//! attachment notices. Akula Core (NOT WOW) owns this policy and its durable
//! state. This module does not decrypt packets, authorize reads, write Vault
//! files, complete an Inbox commit or issue W4 delivery receipts.
//!
//! Trust boundary: only invoke `record_verified` AFTER W3 signature, HPKE,
//! key-epoch, target identity and selected Offer/Decision were verified by the
//! receiving adapter. A successful record means "seen" only, NOT delivered.
//!
//! The SQLite path is caller-owned laboratory state. There is no OS-pinned
//! directory handle or independent rollback anchor: deleting/restoring the
//! database defeats long-term replay protection. Production integration must
//! require first-party pinned root, storage ownership, rollback anchoring and
//! atomic coupling with any user-visible operation.
#![forbid(unsafe_code)]
use std::{fs, path::Path, time::Duration};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use crate::p2p_attachment_contract::AttachmentNoticeV1;

const MAX_W3_BYTES: usize = 16 * 1024;
const MAX_SEEN: i64 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayError {
    InvalidInput,
    SeenBefore,
    ReplayedDifferentObject,
    LedgerFull,
    StorageFailure,
}

pub struct AttachmentReplayLedgerLab {
    conn: Connection,
}

impl AttachmentReplayLedgerLab {
    /// Explicit non-production filesystem path. The application must supply
    /// an already trusted private root; we reject direct symlinks but cannot
    /// securely pin ancestor directories or detect whole-ledger rollback.
    pub fn open(path: &Path) -> Result<Self, ReplayError> {
        if path.as_os_str().is_empty() {
            return Err(ReplayError::InvalidInput);
        }
        match fs::symlink_metadata(path) {
            Ok(meta) if !meta.file_type().is_file() => return Err(ReplayError::InvalidInput),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ReplayError::StorageFailure),
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        ).map_err(|_| ReplayError::StorageFailure)?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|_| ReplayError::StorageFailure)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS p2p_attachment_w3_seen (
               receiver BLOB NOT NULL CHECK (length(receiver)=16),
               sender BLOB NOT NULL CHECK (length(sender)=16),
               message BLOB NOT NULL CHECK (length(message)=16),
               content_root BLOB NOT NULL CHECK (length(content_root)=32),
               packet_hash BLOB NOT NULL CHECK (length(packet_hash)=32),
               content_bytes INTEGER NOT NULL CHECK (content_bytes>0),
               key_epoch INTEGER NOT NULL CHECK (key_epoch>0),
               expires_unix INTEGER NOT NULL CHECK (expires_unix>0),
               PRIMARY KEY (receiver, sender, message)
             ) WITHOUT ROWID;"
        ).map_err(|_| ReplayError::StorageFailure)?;
        Ok(Self { conn })
    }

    /// Single SQLite IMMEDIATE transaction: either a new notice is recorded
    /// durably, or it is refused without granting application authority.
    /// A fresh W3 HPKE nonce with the SAME logical message ID is a replay.
    /// This call MUST NOT stand in for Inbox/transaction commit or a W4 ACK.
    pub fn record_verified(
        &mut self,
        notice: &AttachmentNoticeV1,
        sender_epoch: u64,
        authenticated_w3_packet: &[u8],
        now: u64,
    ) -> Result<(), ReplayError> {
        if sender_epoch == 0
            || sender_epoch > i64::MAX as u64
            || notice.expires_unix() <= now
            || notice.expires_unix() > i64::MAX as u64
            || notice.size() == 0
            || notice.size() > i64::MAX as u64
            || notice.root() == [0;32]
            || authenticated_w3_packet.len() > MAX_W3_BYTES
            || authenticated_w3_packet.is_empty()
        {
            return Err(ReplayError::InvalidInput);
        }
        let receiver = notice.wow_recipient_id();
        let sender = *notice.sender().device_id.as_uuid().as_bytes();
        let message = notice.message_id();
        let root = notice.root();
        let packet_hash = *blake3::hash(authenticated_w3_packet).as_bytes();
        let size = notice.size() as i64;
        let epoch = sender_epoch as i64;
        let expires = notice.expires_unix() as i64;

        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| ReplayError::StorageFailure)?;
        let previous: Option<(Vec<u8>, i64)> = tx.query_row(
            "SELECT content_root, content_bytes
             FROM p2p_attachment_w3_seen
             WHERE receiver=?1 AND sender=?2 AND message=?3",
            params![&receiver[..], &sender[..], &message[..]],
            |row| Ok((row.get(0)?, row.get(1)?))
        ).optional().map_err(|_| ReplayError::StorageFailure)?;
        if let Some((stored_root, stored_size)) = previous {
            return Err(if stored_root != root || stored_size != size {
                ReplayError::ReplayedDifferentObject
            } else {
                ReplayError::SeenBefore
            });
        }
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM p2p_attachment_w3_seen", [], |row| row.get(0)
        ).map_err(|_| ReplayError::StorageFailure)?;
        if count >= MAX_SEEN {
            return Err(ReplayError::LedgerFull);
        }
        tx.execute(
            "INSERT INTO p2p_attachment_w3_seen
             (receiver,sender,message,content_root,packet_hash,content_bytes,key_epoch,expires_unix)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                &receiver[..], &sender[..], &message[..], &root[..],
                &packet_hash[..], size, epoch, expires
            ]
        ).map_err(|_| ReplayError::StorageFailure)?;
        tx.commit().map_err(|_| ReplayError::StorageFailure)
    }

    pub fn count_for_lab(&self) -> Result<i64, ReplayError> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM p2p_attachment_w3_seen", [], |row| row.get(0)
        ).map_err(|_| ReplayError::StorageFailure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p2p_offer_lab::{
        DecisionAction, OfferedFile, OfferV1, PeerBinding, SCHEMA,
    };
    use crate::p2p_attachment_contract::AttachmentNoticeV1;
    use crate::sync::DeviceId;
    use uuid::Uuid;

    fn fixture() -> AttachmentNoticeV1 {
        let sender = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(15)),
            endpoint_id: [5;32],
        };
        let receiver = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(16)),
            endpoint_id: [6;32],
        };
        let offer = OfferV1 {
            schema:SCHEMA, offer_id:Uuid::from_u128(43), nonce:[8;16],
            sender, receiver, created_unix:100, expires_unix:210,
            files:vec![OfferedFile {
                label:"photo.bin".into(), bytes:5*1024*1024, blake3_root:[9;32],
            }],
        };
        let decision = offer.receiver_decision(
            DecisionAction::Accept, vec![0], 101, sender, receiver
        ).unwrap();
        AttachmentNoticeV1::from_accepted(
            &offer, &decision, sender, receiver, 0, 102
        ).unwrap()
    }

    #[test]
    fn reboot_and_new_ciphertext_same_message_id_do_not_reapply() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w3_replay_lab.sqlite");
        let n = fixture();
        {
            let mut ledger = AttachmentReplayLedgerLab::open(&path).unwrap();
            assert_eq!(ledger.count_for_lab(), Ok(0));
            assert_eq!(ledger.record_verified(&n, 7, b"signed-W3-packet-A", 105), Ok(()));
            assert_eq!(ledger.record_verified(&n, 7, b"signed-W3-packet-A", 106), Err(ReplayError::SeenBefore));
        }
        let mut reopened = AttachmentReplayLedgerLab::open(&path).unwrap();
        assert_eq!(reopened.count_for_lab(), Ok(1));
        assert_eq!(reopened.record_verified(&n, 7, b"different-HPKE-ciphertext-same-message", 107), Err(ReplayError::SeenBefore));
        assert_eq!(reopened.record_verified(&n, 8, b"new-epoch-same-logical-operation", 107), Err(ReplayError::SeenBefore));
        assert_eq!(reopened.count_for_lab(), Ok(1));
        assert_eq!(reopened.record_verified(&n, 7, b"more", 210), Err(ReplayError::InvalidInput));
    }

    #[test]
    fn no_zero_epoch_or_blank_ciphertext_and_symlink_path_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lab.sqlite");
        let mut ledger = AttachmentReplayLedgerLab::open(&path).unwrap();
        assert_eq!(ledger.record_verified(&fixture(), 0, b"sealed", 105), Err(ReplayError::InvalidInput));
        assert_eq!(ledger.record_verified(&fixture(), 1, b"", 105), Err(ReplayError::InvalidInput));
        assert_eq!(ledger.count_for_lab(), Ok(0));
        drop(ledger);
        #[cfg(unix)]
        {
            let link = dir.path().join("symlink.sqlite");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(matches!(
                AttachmentReplayLedgerLab::open(&link),
                Err(ReplayError::InvalidInput)
            ));
        }
    }

    #[test]
    fn two_independent_handles_never_both_admit_same_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("concurrent.sqlite");
        let mut first = AttachmentReplayLedgerLab::open(&path).unwrap();
        let mut second = AttachmentReplayLedgerLab::open(&path).unwrap();
        let n = fixture();
        assert_eq!(first.record_verified(&n, 1, b"packet-a", 105), Ok(()));
        assert_eq!(second.record_verified(&n, 1, b"packet-b", 105), Err(ReplayError::SeenBefore));
        assert_eq!(first.count_for_lab(), Ok(1));
    }
}
