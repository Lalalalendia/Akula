//! Default-off Akula Core authority for the sender's exact W3 object and
//! durable W4 FILE_COMMITTED terminal status. No new transfer engine, no
//! file reads, no Vault or File Gate mutation and no ordinary W4 message ACK.
//!
//! Exact source ciphertext is stored once and never silently re-encrypted.
//! Only an independently verified recipient Ed25519 W4 matching THAT packet,
//! message ID and approved object can atomically stop further file retries.
//! This lab path is not OS-pinned and is not protected from whole-DB rollback.
#![forbid(unsafe_code)]
use std::{fs, path::Path, time::Duration};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use akula_wow_w3_crypto::{
    file_receipt_w4::{self, FILE_RECEIPT_BYTES},
    trust_w3::TrustRegistry,
};
use crate::p2p_attachment_contract::AttachmentNoticeV1;

const MAX_W3_BYTES: usize = 16 * 1024;
const MAX_ROWS: i64 = 100_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutboxError {
    Invalid,
    Conflict,
    NotFound,
    Full,
    UntrustedReceipt,
    Storage,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileSendState { Pending, VerifiedPrivateInbox }

pub struct FileSenderOutboxLab { db: Connection }

fn tuple(notice: &AttachmentNoticeV1) -> ([u8; 16], [u8; 16], [u8; 16]) {
    (
        *notice.sender().device_id.as_uuid().as_bytes(),
        *notice.receiver().device_id.as_uuid().as_bytes(),
        notice.message_id(),
    )
}

impl FileSenderOutboxLab {
    pub fn open(path: &Path) -> Result<Self, OutboxError> {
        if path.as_os_str().is_empty() { return Err(OutboxError::Invalid); }
        match fs::symlink_metadata(path) {
            Ok(meta) if !meta.is_file() => return Err(OutboxError::Invalid),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(OutboxError::Storage),
        }
        let db = Connection::open_with_flags(
            path, OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        ).map_err(|_| OutboxError::Storage)?;
        db.busy_timeout(Duration::from_secs(5))
            .map_err(|_| OutboxError::Storage)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS p2p_file_sender_outbox (
               sender BLOB NOT NULL CHECK(length(sender)=16),
               recipient BLOB NOT NULL CHECK(length(recipient)=16),
               message BLOB NOT NULL CHECK(length(message)=16),
               w3_packet BLOB NOT NULL CHECK(length(w3_packet) BETWEEN 74 AND 16384),
               w3_hash BLOB NOT NULL CHECK(length(w3_hash)=32),
               file_root BLOB NOT NULL CHECK(length(file_root)=32),
               file_bytes INTEGER NOT NULL CHECK(file_bytes BETWEEN 1 AND 8388608),
               expires_unix INTEGER NOT NULL,
               state INTEGER NOT NULL CHECK(state IN (0,1)),
               signed_w4 BLOB CHECK(signed_w4 IS NULL OR length(signed_w4)=193),
               PRIMARY KEY (sender,recipient,message),
               CHECK ((state=0 AND signed_w4 IS NULL) OR
                      (state=1 AND signed_w4 IS NOT NULL))
             ) WITHOUT ROWID;"
        ).map_err(|_| OutboxError::Storage)?;
        Ok(Self { db })
    }

    /// Only accept the sender's already-created exact W3 HPKE packet. The
    /// packet must have been sealed from this trusted local notice by the
    /// caller; header matching is NOT a replacement for W3 cryptography.
    /// A duplicate logical message with altered HPKE ciphertext conflicts.
    pub fn enqueue_exact_w3(
        &mut self,
        notice: &AttachmentNoticeV1,
        packet: &[u8],
        now: u64,
    ) -> Result<(), OutboxError> {
        if packet.len() < 74 || packet.len() > MAX_W3_BYTES
            || packet[0] != 1
            || packet[1..17] != notice.message_id()
            || packet[17..33] != *notice.receiver().device_id.as_uuid().as_bytes()
            || u64::from_be_bytes(packet[33..41].try_into().unwrap()) != notice.expires_unix()
            || now >= notice.expires_unix()
            || notice.expires_unix() > i64::MAX as u64
            || notice.size() == 0 || notice.size() > 8 * 1024 * 1024
            || notice.root() == [0; 32]
        {
            return Err(OutboxError::Invalid);
        }
        let (sender,recipient,message) = tuple(notice);
        let digest = *blake3::hash(packet).as_bytes();
        let root=notice.root();
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| OutboxError::Storage)?;
        let old:Option<(Vec<u8>,Vec<u8>,i64)> = tx.query_row(
            "SELECT w3_hash,file_root,file_bytes
             FROM p2p_file_sender_outbox
             WHERE sender=?1 AND recipient=?2 AND message=?3",
            params![&sender[..],&recipient[..],&message[..]],
            |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional().map_err(|_| OutboxError::Storage)?;
        if let Some((old_digest,old_root,old_bytes))=old {
            return if old_digest == digest && old_root == root
                && old_bytes == notice.size() as i64 {
                Ok(())
            } else { Err(OutboxError::Conflict) };
        }
        let count:i64=tx.query_row(
            "SELECT COUNT(*) FROM p2p_file_sender_outbox", [], |r|r.get(0)
        ).map_err(|_| OutboxError::Storage)?;
        if count>=MAX_ROWS { return Err(OutboxError::Full); }
        tx.execute(
            "INSERT INTO p2p_file_sender_outbox
             (sender,recipient,message,w3_packet,w3_hash,file_root,
              file_bytes,expires_unix,state,signed_w4)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,0,NULL)",
            params![&sender[..],&recipient[..],&message[..],packet,
                &digest[..],&root[..],notice.size() as i64,
                notice.expires_unix() as i64],
        ).map_err(|_| OutboxError::Storage)?;
        tx.commit().map_err(|_| OutboxError::Storage)
    }

    /// Returns the exact ciphertext even when no retry is required, for
    /// verifying a late W4. Never issues a new encryption for this message ID.
    pub fn exact_original_w3(
        &self, notice: &AttachmentNoticeV1,
    ) -> Result<Vec<u8>, OutboxError> {
        let (sender,recipient,message)=tuple(notice);
        let row:Option<(Vec<u8>,Vec<u8>,Vec<u8>,i64,i64)> = self.db.query_row(
            "SELECT w3_packet,w3_hash,file_root,file_bytes,expires_unix
             FROM p2p_file_sender_outbox
             WHERE sender=?1 AND recipient=?2 AND message=?3",
            params![&sender[..],&recipient[..],&message[..]],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))
        ).optional().map_err(|_| OutboxError::Storage)?;
        let (packet,digest,root,bytes,expiry)=row.ok_or(OutboxError::NotFound)?;
        if digest != blake3::hash(&packet).as_bytes()
            || root != notice.root()
            || bytes != notice.size() as i64
            || expiry != notice.expires_unix() as i64
        { return Err(OutboxError::Conflict); }
        Ok(packet)
    }

    /// The caller may retry only while pending and within W3 expiry.
    /// Signed FILE_COMMITTED moves to a terminal state; no more payload GET.
    pub fn retry_w3_if_pending(
        &self, notice: &AttachmentNoticeV1, now:u64,
    )->Result<Option<Vec<u8>>,OutboxError>{
        let (sender,recipient,message)=tuple(notice);
        let state:Option<i64>=self.db.query_row(
            "SELECT state FROM p2p_file_sender_outbox
             WHERE sender=?1 AND recipient=?2 AND message=?3",
            params![&sender[..],&recipient[..],&message[..]],
            |r|r.get(0)
        ).optional().map_err(|_| OutboxError::Storage)?;
        match state.ok_or(OutboxError::NotFound)? {
            1=>Ok(None),
            0 if now>=notice.expires_unix()=>Ok(None),
            0=>self.exact_original_w3(notice).map(Some),
            _=>Err(OutboxError::Conflict),
        }
    }

    /// Important: call only with the exact *signed W4*, not a QUIC ACK.
    /// Signature, independent recipient key/epoch and content binding are
    /// checked again inside Core before one durable state transition.
    pub fn accept_signed_file_commit(
        &mut self,
        approved: &AttachmentNoticeV1,
        signed_w4: &[u8],
        enrolled_recipient_keys: &TrustRegistry,
    )->Result<FileSendState,OutboxError>{
        if signed_w4.len()!=FILE_RECEIPT_BYTES {
            return Err(OutboxError::Invalid);
        }
        let verified=file_receipt_w4::verify_file_committed(
            signed_w4,enrolled_recipient_keys
        ).map_err(|_|OutboxError::UntrustedReceipt)?;
        if verified.recipient_id != *approved.receiver().device_id.as_uuid().as_bytes()
            || verified.message_id != approved.message_id()
            || verified.file_blake3 != approved.root()
            || verified.file_size != approved.size()
        {
            return Err(OutboxError::Conflict);
        }
        let (sender,recipient,message)=tuple(approved);
        let tx=self.db.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_|OutboxError::Storage)?;
        let old:Option<(Vec<u8>,Vec<u8>,i64,i64,Option<Vec<u8>>)> = tx.query_row(
            "SELECT w3_hash,file_root,file_bytes,state,signed_w4
             FROM p2p_file_sender_outbox
             WHERE sender=?1 AND recipient=?2 AND message=?3",
            params![&sender[..],&recipient[..],&message[..]],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))
        ).optional().map_err(|_|OutboxError::Storage)?;
        let (packet_hash,root,size,state,old_receipt)=old.ok_or(OutboxError::NotFound)?;
        if packet_hash != verified.w3_packet_digest
            || root != verified.file_blake3
            || size != verified.file_size as i64
        {return Err(OutboxError::Conflict);}
        match state {
            1 if old_receipt.as_deref()==Some(signed_w4)
                =>Ok(FileSendState::VerifiedPrivateInbox),
            1=>Err(OutboxError::Conflict),
            0 if old_receipt.is_none()=>{
                tx.execute(
                    "UPDATE p2p_file_sender_outbox SET state=1,signed_w4=?4
                     WHERE sender=?1 AND recipient=?2 AND message=?3",
                    params![&sender[..],&recipient[..],&message[..],signed_w4],
                ).map_err(|_|OutboxError::Storage)?;
                tx.commit().map_err(|_|OutboxError::Storage)?;
                Ok(FileSendState::VerifiedPrivateInbox)
            }
            _=>Err(OutboxError::Conflict),
        }
    }

    pub fn state(&self, notice:&AttachmentNoticeV1)
        ->Result<FileSendState,OutboxError>{
        let (sender,recipient,message)=tuple(notice);
        let status:i64=self.db.query_row(
            "SELECT state FROM p2p_file_sender_outbox
             WHERE sender=?1 AND recipient=?2 AND message=?3",
            params![&sender[..],&recipient[..],&message[..]],
            |r|r.get(0)
        ).map_err(|_|OutboxError::NotFound)?;
        match status {
            0=>Ok(FileSendState::Pending),
            1=>Ok(FileSendState::VerifiedPrivateInbox),
            _=>Err(OutboxError::Conflict),
        }
    }

    pub fn count_for_lab(&self)->Result<i64,OutboxError>{
        self.db.query_row(
            "SELECT COUNT(*) FROM p2p_file_sender_outbox",[],|r|r.get(0)
        ).map_err(|_|OutboxError::Storage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        p2p_offer_lab::{DecisionAction,OfferedFile,OfferV1,PeerBinding,SCHEMA},
        sync::DeviceId,
    };
    use akula_wow_w3_crypto::crypto_w3;
    use ed25519_dalek::SigningKey;
    use uuid::Uuid;
    fn fixture()->(AttachmentNoticeV1,Vec<u8>,SigningKey,TrustRegistry){
        let sender=PeerBinding {
            device_id:DeviceId::from_uuid(Uuid::from_u128(17)),endpoint_id:[7;32]
        };
        let receiver=PeerBinding {
            device_id:DeviceId::from_uuid(Uuid::from_u128(23)),endpoint_id:[8;32]
        };
        let offer=OfferV1 {
            schema:SCHEMA,offer_id:Uuid::from_u128(42),nonce:[9;16],
            sender,receiver,created_unix:100,expires_unix:500,
            files:vec![OfferedFile {
                label:"file.bin".into(),bytes:5*1024*1024,blake3_root:[43;32]
            }]
        };
        let decision=offer.receiver_decision(
            DecisionAction::Accept,vec![0],110,sender,receiver
        ).unwrap();
        let notice=AttachmentNoticeV1::from_accepted(
            &offer,&decision,sender,receiver,0,120
        ).unwrap();
        let sender_signer=SigningKey::from_bytes(&[61;32]);
        let (_recipient_secret,recipient_public)=crypto_w3::new_recipient();
        let w3=crypto_w3::seal(
            &sender_signer,&recipient_public,
            notice.message_id(),notice.wow_recipient_id(),
            notice.expires_unix(),&notice.encode()
        ).unwrap();
        let recipient_signer=SigningKey::from_bytes(&[62;32]);
        let mut registry=TrustRegistry::new();
        registry.enroll(*receiver.device_id.as_uuid().as_bytes(),
            7,recipient_signer.verifying_key()).unwrap();
        (notice,w3,recipient_signer,registry)
    }
    fn receipt(n:&AttachmentNoticeV1,w3:&[u8],signer:&SigningKey,transfer:u8)
        ->[u8;FILE_RECEIPT_BYTES]{
        let s=file_receipt_w4::FileCommitStatement {
            recipient_id:n.wow_recipient_id(),
            message_id:n.message_id(),
            w3_packet_digest:*blake3::hash(w3).as_bytes(),
            file_blake3:n.root(),file_size:n.size(),
            inbox_transfer_id:[transfer;16],
            recipient_epoch:7,
        };
        file_receipt_w4::sign_file_committed(signer,s).unwrap()
    }
    #[test]
    fn cold_sender_stops_retries_only_after_exact_signed_file_receipt(){
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("outbox.sqlite");
        let (notice,w3,signer,mut keys)=fixture();
        {
            let mut ledger=FileSenderOutboxLab::open(&path).unwrap();
            assert_eq!(ledger.state(&notice),Err(OutboxError::NotFound));
            ledger.enqueue_exact_w3(&notice,&w3,130).unwrap();
            ledger.enqueue_exact_w3(&notice,&w3,131).unwrap();
            assert_eq!(ledger.count_for_lab(),Ok(1));
            assert_eq!(ledger.retry_w3_if_pending(&notice,132),Ok(Some(w3.clone())));
            assert_eq!(ledger.retry_w3_if_pending(&notice,500),Ok(None));
            let mut altered=w3.clone();
            let index=altered.len()-1;
            altered[index]^=1;
            assert_eq!(ledger.enqueue_exact_w3(&notice,&altered,140),Err(OutboxError::Conflict));
            assert_eq!(ledger.state(&notice),Ok(FileSendState::Pending));
        }
        let mut cold=FileSenderOutboxLab::open(&path).unwrap();
        assert_eq!(cold.retry_w3_if_pending(&notice,141),Ok(Some(w3.clone())));
        let signed=receipt(&notice,&w3,&signer,55);
        let mut corrupted=signed;
        corrupted[70]^=1;
        assert_eq!(cold.accept_signed_file_commit(&notice,&corrupted,&keys),Err(OutboxError::UntrustedReceipt));
        let wrong_hash=receipt(&notice,b"wrong W3",&signer,55);
        assert_eq!(cold.accept_signed_file_commit(&notice,&wrong_hash,&keys),Err(OutboxError::Conflict));
        assert_eq!(cold.state(&notice),Ok(FileSendState::Pending));
        assert_eq!(cold.accept_signed_file_commit(&notice,&signed,&keys),Ok(FileSendState::VerifiedPrivateInbox));
        assert_eq!(cold.retry_w3_if_pending(&notice,142),Ok(None));
        assert_eq!(cold.accept_signed_file_commit(&notice,&signed,&keys),Ok(FileSendState::VerifiedPrivateInbox));
        assert_eq!(cold.accept_signed_file_commit(&notice,&receipt(&notice,&w3,&signer,56),&keys),Err(OutboxError::Conflict));
        drop(cold);
        let mut cold=FileSenderOutboxLab::open(&path).unwrap();
        assert_eq!(cold.state(&notice),Ok(FileSendState::VerifiedPrivateInbox));
        assert_eq!(cold.retry_w3_if_pending(&notice,145),Ok(None));
        keys.revoke(notice.wow_recipient_id()).unwrap();
        assert_eq!(cold.accept_signed_file_commit(&notice,&signed,&keys),Err(OutboxError::UntrustedReceipt));
        assert_eq!(cold.state(&notice),Ok(FileSendState::VerifiedPrivateInbox));
    }
    #[test]
    fn unseen_and_wrong_recipient_receipts_cannot_set_delivered(){
        let dir=tempfile::tempdir().unwrap();
        let (notice,w3,signer,keys)=fixture();
        let signed=receipt(&notice,&w3,&signer,55);
        let mut ledger=FileSenderOutboxLab::open(&dir.path().join("file.db")).unwrap();
        assert_eq!(ledger.accept_signed_file_commit(&notice,&signed,&keys),Err(OutboxError::NotFound));
        ledger.enqueue_exact_w3(&notice,&w3,130).unwrap();
        let other_signer=SigningKey::from_bytes(&[76;32]);
        assert_eq!(ledger.accept_signed_file_commit(
            &notice,&receipt(&notice,&w3,&other_signer,55),&keys
        ),Err(OutboxError::UntrustedReceipt));
        assert_eq!(ledger.accept_signed_file_commit(&notice,&signed[..137],&keys),Err(OutboxError::Invalid));
        assert_eq!(ledger.state(&notice),Ok(FileSendState::Pending));
    }
}