//! Lab-only atomic one-shot owner-read grant admission.
//! Call ONLY after independent Ed25519 signature, enrollment/epoch, live
//! authenticated QUIC receiver and actual Akula Core source ACL checks.
//! This table is NOT a source-file handle, network server, or W4 receipt.
//! Does not protect against replacing the containing SQLite directory or
//! rolling back the entire database: production must use pinned custody.
#![forbid(unsafe_code)]
use std::{fs, path::Path, time::Duration};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use crate::p2p_attachment_grant_lab::{VerifiedOwnerRead, GRANT_BYTES};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrantSpendError {
    Invalid,
    Expired,
    OutOfRange,
    AlreadyUsed,
    Conflict,
    Storage,
}
pub struct GrantSpendLedgerLab { conn: Connection }

impl GrantSpendLedgerLab {
    pub fn open(path: &Path) -> Result<Self, GrantSpendError> {
        if path.as_os_str().is_empty() { return Err(GrantSpendError::Invalid); }
        match fs::symlink_metadata(path) {
            Ok(m) if !m.is_file() => return Err(GrantSpendError::Invalid),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(GrantSpendError::Storage),
        }
        let conn = Connection::open_with_flags(path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE |
            OpenFlags::SQLITE_OPEN_FULL_MUTEX).map_err(|_| GrantSpendError::Storage)?;
        conn.busy_timeout(Duration::from_secs(5)).map_err(|_| GrantSpendError::Storage)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS p2p_grant_spent (
               owner BLOB NOT NULL CHECK(length(owner)=16),
               receiver BLOB NOT NULL CHECK(length(receiver)=16),
               epoch INTEGER NOT NULL,
               nonce BLOB NOT NULL CHECK(length(nonce)=16),
               grant_hash BLOB NOT NULL CHECK(length(grant_hash)=32),
               message BLOB NOT NULL CHECK(length(message)=16),
               object_root BLOB NOT NULL CHECK(length(object_root)=32),
               start INTEGER NOT NULL,
               end INTEGER NOT NULL,
               PRIMARY KEY(owner, epoch, nonce)
             ) WITHOUT ROWID;"
        ).map_err(|_| GrantSpendError::Storage)?;
        Ok(Self{conn})
    }

    /// One full signed grant authorizes at most ONE admitted range. If a
    /// transfer fails it must obtain a new grant for the remaining bytes.
    /// No remote retry may silently reactivate a consumed nonce.
    pub fn spend_for_range(
        &mut self, verified: &VerifiedOwnerRead, signed_wire: &[u8],
        start: u64, end: u64, now: u64,
    ) -> Result<(), GrantSpendError> {
        if signed_wire.len()!=GRANT_BYTES
            || !verified.matches_signed_wire(signed_wire)
            || verified.nonce()==[0;16] || verified.owner_epoch()==0
            || verified.owner_epoch()>i64::MAX as u64
            || now>=verified.expires_unix()
            || start>i64::MAX as u64 || end>i64::MAX as u64
        { return Err(GrantSpendError::Invalid); }
        if !verified.permits_exact_range(start,end) {
            return Err(GrantSpendError::OutOfRange);
        }
        let owner=*verified.owner().device_id.as_uuid().as_bytes();
        let receiver=*verified.receiver().device_id.as_uuid().as_bytes();
        let root=verified.root();
        let message=verified.message_id();
        let nonce=verified.nonce();
        let grant_hash=*blake3::hash(signed_wire).as_bytes();
        let tx=self.conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| GrantSpendError::Storage)?;
        let previous:Option<Vec<u8>>=tx.query_row(
            "SELECT grant_hash FROM p2p_grant_spent
             WHERE owner=?1 AND epoch=?2 AND nonce=?3",
            params![&owner[..],verified.owner_epoch() as i64,&nonce[..]],
            |r|r.get(0)).optional().map_err(|_| GrantSpendError::Storage)?;
        if let Some(old)=previous {
            return Err(if old==grant_hash {GrantSpendError::AlreadyUsed}
                else {GrantSpendError::Conflict});
        }
        tx.execute(
            "INSERT INTO p2p_grant_spent
             (owner,receiver,epoch,nonce,grant_hash,message,object_root,start,end)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![&owner[..],&receiver[..],verified.owner_epoch() as i64,
                &nonce[..],&grant_hash[..],&message[..],&root[..],
                start as i64,end as i64]
        ).map_err(|_| GrantSpendError::Storage)?;
        tx.commit().map_err(|_| GrantSpendError::Storage)
    }

    pub fn count_for_lab(&self)->Result<i64,GrantSpendError>{
        self.conn.query_row("SELECT COUNT(*) FROM p2p_grant_spent",[],|r|r.get(0))
            .map_err(|_|GrantSpendError::Storage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        p2p_attachment_contract::AttachmentNoticeV1,
        p2p_attachment_grant_lab::{issue_owner_read_grant,verify_owner_read_grant,GrantWindow,LocalReadPolicy},
        p2p_offer_lab::{DecisionAction, OfferedFile, OfferV1, PeerBinding, SCHEMA},
        sync::DeviceId,
    };
    use ring::signature::{Ed25519KeyPair,KeyPair};
    use uuid::Uuid;

    struct Policy;
    impl LocalReadPolicy for Policy {
        fn permit_exact_object(&self,_:PeerBinding,_:PeerBinding,_:[u8;32],_:u64)->bool{true}
    }
    fn fixture()->(VerifiedOwnerRead,[u8;GRANT_BYTES]){
        let sender=PeerBinding{device_id:DeviceId::from_uuid(Uuid::from_u128(11)),endpoint_id:[1;32]};
        let receiver=PeerBinding{device_id:DeviceId::from_uuid(Uuid::from_u128(12)),endpoint_id:[2;32]};
        let offer=OfferV1{schema:SCHEMA,offer_id:Uuid::from_u128(42),nonce:[3;16],
            sender,receiver,created_unix:100,expires_unix:500,
            files:vec![OfferedFile{label:"f".into(),bytes:5*1024*1024,blake3_root:[4;32]}]};
        let decision=offer.receiver_decision(DecisionAction::Accept,vec![0],101,sender,receiver).unwrap();
        let notice=AttachmentNoticeV1::from_accepted(&offer,&decision,sender,receiver,0,102).unwrap();
        let owner=Ed25519KeyPair::from_seed_unchecked(&[19;32]).unwrap();
        let win=GrantWindow{start:2*1024*1024,end:5*1024*1024,issued_unix:110,
            expires_unix:200,owner_key_epoch:7,nonce:[18;16]};
        let wire=issue_owner_read_grant(&notice,&owner,&Policy,win,111).unwrap();
        let pubkey:[u8;32]=owner.public_key().as_ref().try_into().unwrap();
        let checked=verify_owner_read_grant(&wire,&notice,sender,&pubkey,7,receiver.endpoint_id,112).unwrap();
        (checked,wire)
    }
    #[test]
    fn durable_one_time_use_rejects_double_spend_and_wrong_range(){
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("grant.sqlite");
        let (grant,wire)=fixture();
        let mut first=GrantSpendLedgerLab::open(&path).unwrap();
        assert_eq!(first.spend_for_range(&grant,&wire,0,5*1024*1024,112),Err(GrantSpendError::OutOfRange));
        assert_eq!(first.count_for_lab(),Ok(0));
        let mut wrong_wire = wire;
        wrong_wire[GRANT_BYTES-1] ^= 1;
        assert_eq!(first.spend_for_range(&grant,&wrong_wire,2*1024*1024,5*1024*1024,112),Err(GrantSpendError::Invalid));
        assert_eq!(first.count_for_lab(),Ok(0));
        assert_eq!(first.spend_for_range(&grant,&wire,2*1024*1024,5*1024*1024,112),Ok(()));
        assert_eq!(first.spend_for_range(&grant,&wire,2*1024*1024,5*1024*1024,112),Err(GrantSpendError::AlreadyUsed));
        drop(first);
        let mut cold=GrantSpendLedgerLab::open(&path).unwrap();
        assert_eq!(cold.count_for_lab(),Ok(1));
        assert_eq!(cold.spend_for_range(&grant,&wire,2*1024*1024,5*1024*1024,113),Err(GrantSpendError::AlreadyUsed));
        assert_eq!(cold.spend_for_range(&grant,&wire,2*1024*1024,5*1024*1024,200),Err(GrantSpendError::Invalid));
    }
    #[test]
    fn separate_database_handles_cannot_spend_same_grant_twice(){
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("grant.sqlite");
        let (grant,wire)=fixture();
        let mut a=GrantSpendLedgerLab::open(&path).unwrap();
        let mut b=GrantSpendLedgerLab::open(&path).unwrap();
        assert_eq!(a.spend_for_range(&grant,&wire,2*1024*1024,3*1024*1024,112),Ok(()));
        assert_eq!(b.spend_for_range(&grant,&wire,3*1024*1024,5*1024*1024,112),Err(GrantSpendError::AlreadyUsed));
    }
}
