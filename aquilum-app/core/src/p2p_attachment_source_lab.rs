//! Opt-in source snapshot and complete pre-read authorization fence.
//! Only a pre-opened locally selected regular File can enter this boundary.
//! No path is parsed from a remote message. The entire <=8 MiB selected
//! source is independently hashed into a bounded immutable in-memory snapshot
//! before serving any range. This intentionally pays 8 MiB max RAM in the
//! laboratory to eliminate post-verification mutation and pathname swaps.
//!
//! Issuance must still pass the actual Akula owner ACL policy. Serving MUST
//! revalidate the signed Ed25519 grant against independently enrolled owner
//! key/epoch and a live authenticated QUIC receiver, then atomically spend
//! its nonce BEFORE exposing any bytes. A failed transfer needs a fresh grant.
//!
//! No network server, installed OS-grant authority, OS-pinned source root,
//! S3 transfer, Vault mutation, delivery W4 or production restart path exists
//! here. Default-off and only 8 MiB, awaiting a private bounded disk snapshot.
#![forbid(unsafe_code)]
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};
use crate::{
    p2p_attachment_contract::AttachmentNoticeV1,
    p2p_attachment_grant_lab::{verify_owner_read_grant, GrantError},
    p2p_attachment_grant_spend_lab::{GrantSpendError, GrantSpendLedgerLab},
};

const MAX_SNAPSHOT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SourceServeError {
    UnselectedSource,
    InvalidSource,
    WrongObjectBytes,
    SourceReadFailure,
    InvalidReceiver,
    SignedGrant(GrantError),
    GrantAlreadySpent(GrantSpendError),
}

pub struct SelectedSourceSnapshotLab {
    notice: AttachmentNoticeV1,
    bytes: Box<[u8]>,
}
impl SelectedSourceSnapshotLab {
    /// Caller owns the selected local File handle and is responsible for its
    /// provenance and source ACL. A malicious remote peer cannot pass a path.
    /// We verify a full immutable byte snapshot, not a path-based metadata
    /// assertion; no unverified content can be exposed by this object.
    pub fn from_core_selected_file(
        mut trusted_handle: File,
        notice: AttachmentNoticeV1,
    ) -> Result<Self, SourceServeError> {
        if notice.size() == 0
            || notice.size() > MAX_SNAPSHOT_BYTES
            || notice.root() == [0; 32]
        {
            return Err(SourceServeError::UnselectedSource);
        }
        let stat=trusted_handle.metadata().map_err(|_| SourceServeError::SourceReadFailure)?;
        if !stat.is_file() || stat.len() != notice.size() {
            return Err(SourceServeError::InvalidSource);
        }
        trusted_handle.seek(SeekFrom::Start(0))
            .map_err(|_| SourceServeError::SourceReadFailure)?;
        let mut bytes=Vec::with_capacity(notice.size() as usize);
        let mut reader=trusted_handle.take(notice.size()+1);
        reader.read_to_end(&mut bytes).map_err(|_| SourceServeError::SourceReadFailure)?;
        if bytes.len() as u64 != notice.size()
            || *blake3::hash(&bytes).as_bytes()!=notice.root()
        {
            return Err(SourceServeError::WrongObjectBytes);
        }
        Ok(Self{notice,bytes:bytes.into_boxed_slice()})
    }

    pub fn message_id(&self) -> [u8; 16] { self.notice.message_id() }
    pub fn size(&self) -> u64 { self.bytes.len() as u64 }
    pub fn receiver_endpoint(&self) -> [u8;32] { self.notice.receiver().endpoint_id }

    /// After a fresh authenticated QUIC handshake, supply its actual remote
    /// EndpointId. The key/epoch must come from local trusted enrollment,
    /// not the request. This is a bounded synchronous laboratory adapter.
    pub fn serve_once<'a>(
        &'a self,
        signed_grant: &[u8],
        independently_enrolled_owner_pubkey: &[u8; 32],
        enrolled_owner_epoch: u64,
        authenticated_quic_receiver_endpoint: [u8; 32],
        start: u64,
        end: u64,
        now: u64,
        ledger: &mut GrantSpendLedgerLab,
    ) -> Result<&'a [u8], SourceServeError> {
        if self.notice.receiver().endpoint_id != authenticated_quic_receiver_endpoint {
            return Err(SourceServeError::InvalidReceiver);
        }
        let verified=verify_owner_read_grant(
            signed_grant,
            &self.notice,
            self.notice.sender(),
            independently_enrolled_owner_pubkey,
            enrolled_owner_epoch,
            authenticated_quic_receiver_endpoint,
            now,
        ).map_err(SourceServeError::SignedGrant)?;
        if verified.root()!=self.notice.root()
            || verified.size()!=self.size()
            || verified.message_id()!=self.notice.message_id()
            || verified.owner()!=self.notice.sender()
            || verified.receiver()!=self.notice.receiver()
        {
            return Err(SourceServeError::UnselectedSource);
        }
        if !verified.permits_exact_range(start,end) {
            return Err(SourceServeError::SignedGrant(GrantError::InvalidBounds));
        }
        let from=usize::try_from(start).map_err(|_| SourceServeError::InvalidSource)?;
        let to=usize::try_from(end).map_err(|_| SourceServeError::InvalidSource)?;
        if self.bytes.get(from..to).is_none() {
            return Err(SourceServeError::InvalidSource);
        }
        // Durably spend BEFORE revealing bytes. Even lost final receipts and
        // reconnects cannot reauthorize the same grant; issue a new one only
        // after genuine Core consent+ACL checks on the remaining range.
        ledger.spend_for_range(&verified,signed_grant,start,end,now)
            .map_err(SourceServeError::GrantAlreadySpent)?;
        Ok(&self.bytes[from..to])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use crate::{
        p2p_attachment_grant_lab::{
            issue_owner_read_grant, DenyAllReads, GrantWindow, LocalReadPolicy,
        },
        p2p_offer_lab::{
            DecisionAction, OfferedFile, OfferV1, PeerBinding, SCHEMA,
        },
        sync::DeviceId,
    };
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use uuid::Uuid;

    const TOTAL:usize=5*1024*1024;
    const MID:usize=2*1024*1024;
    struct LocalFixtureConsent;
    impl LocalReadPolicy for LocalFixtureConsent {
        fn permit_exact_object(
            &self,_:PeerBinding,_:PeerBinding,_:[u8;32],_:u64
        )->bool{true}
    }
    fn selected(data:&[u8])->AttachmentNoticeV1{
        let sender=PeerBinding {
            device_id:DeviceId::from_uuid(Uuid::from_u128(17)),endpoint_id:[11;32]
        };
        let receiver=PeerBinding {
            device_id:DeviceId::from_uuid(Uuid::from_u128(23)),endpoint_id:[27;32]
        };
        let offer=OfferV1{schema:SCHEMA,offer_id:Uuid::from_u128(42),
            nonce:[91;16],sender,receiver,created_unix:100,expires_unix:500,
            files:vec![OfferedFile{
                label:"selected-photo.bin".into(),bytes:data.len() as u64,
                blake3_root:*blake3::hash(data).as_bytes(),
            }],
        };
        let decision=offer.receiver_decision(
            DecisionAction::Accept,vec![0],101,sender,receiver
        ).unwrap();
        AttachmentNoticeV1::from_accepted(
            &offer,&decision,sender,receiver,0,102
        ).unwrap()
    }
    fn window(nonce:u8,start:u64,end:u64)->GrantWindow{
        GrantWindow{start,end,issued_unix:110,expires_unix:200,
            owner_key_epoch:7,nonce:[nonce;16]}
    }

    #[test]
    fn live_receiver_and_one_shot_grant_fence_exact_selected_bytes(){
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("photo");
        let bytes=(0..TOTAL).map(|i|((i*29)^(i>>5)) as u8).collect::<Vec<_>>();
        fs::write(&path,&bytes).unwrap();
        let notice=selected(&bytes);
        let snap=SelectedSourceSnapshotLab::from_core_selected_file(
            File::open(&path).unwrap(),notice
        ).unwrap();
        let signer=Ed25519KeyPair::from_seed_unchecked(&[37;32]).unwrap();
        let pk:[u8;32]=signer.public_key().as_ref().try_into().unwrap();
        assert_eq!(
            issue_owner_read_grant(
                &notice,&signer,&DenyAllReads,
                window(15,MID as u64,TOTAL as u64),111
            ),Err(GrantError::Unauthorized)
        );
        let grant=issue_owner_read_grant(
            &notice,&signer,&LocalFixtureConsent,
            window(15,MID as u64,TOTAL as u64),111
        ).unwrap();
        let mut ledger=GrantSpendLedgerLab::open(
            &dir.path().join("spent.sqlite")
        ).unwrap();
        assert!(matches!(
            snap.serve_once(&grant,&pk,7,[66;32],
                MID as u64,TOTAL as u64,112,&mut ledger),
            Err(SourceServeError::InvalidReceiver)
        ));
        assert_eq!(ledger.count_for_lab(),Ok(0));
        let mut changed=grant;
        changed[crate::p2p_attachment_grant_lab::GRANT_BYTES-1]^=1;
        assert!(matches!(
            snap.serve_once(&changed,&pk,7,notice.receiver().endpoint_id,
                MID as u64,TOTAL as u64,112,&mut ledger),
            Err(SourceServeError::SignedGrant(GrantError::InvalidSignature))
        ));
        assert_eq!(ledger.count_for_lab(),Ok(0));
        assert_eq!(
            snap.serve_once(&grant,&pk,7,notice.receiver().endpoint_id,
                0,TOTAL as u64,112,&mut ledger),
            Err(SourceServeError::SignedGrant(GrantError::InvalidBounds))
        );
        assert_eq!(ledger.count_for_lab(),Ok(0));
        // Same disk path becomes unrelated bytes after the snapshot.
        // Network access never follows that mutable path.
        fs::write(&path,vec![0x99;TOTAL]).unwrap();
        let range=snap.serve_once(&grant,&pk,7,
            notice.receiver().endpoint_id,MID as u64,TOTAL as u64,112,&mut ledger
        ).unwrap();
        assert_eq!(range,&bytes[MID..]);
        assert_eq!(ledger.count_for_lab(),Ok(1));
        assert_eq!(
            snap.serve_once(&grant,&pk,7,notice.receiver().endpoint_id,
                MID as u64,TOTAL as u64,113,&mut ledger),
            Err(SourceServeError::GrantAlreadySpent(GrantSpendError::AlreadyUsed))
        );
    }

    #[test]
    fn changed_source_and_unselected_file_never_produce_snapshot(){
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("photo");
        let bytes=vec![0x35;TOTAL];
        fs::write(&path,&bytes).unwrap();
        let n=selected(&bytes);
        let other=vec![0x17;TOTAL];
        fs::write(&path,&other).unwrap();
        assert!(matches!(
            SelectedSourceSnapshotLab::from_core_selected_file(
                File::open(&path).unwrap(),n),
            Err(SourceServeError::WrongObjectBytes)
        ));
        fs::write(&path,&bytes[..TOTAL-1]).unwrap();
        assert!(matches!(
            SelectedSourceSnapshotLab::from_core_selected_file(
                File::open(&path).unwrap(),n),
            Err(SourceServeError::InvalidSource)
        ));
        // Unix allows opening a directory handle (which the boundary must
        // reject); Windows may refuse File::open on a directory entirely.
        match File::open(dir.path()) {
            Ok(directory) => assert!(matches!(
                SelectedSourceSnapshotLab::from_core_selected_file(directory,n),
                Err(SourceServeError::InvalidSource)
            )),
            Err(error) => assert!(matches!(
                error.kind(),
                std::io::ErrorKind::PermissionDenied |
                std::io::ErrorKind::IsADirectory
            )),
        }
    }

    #[test]
    fn fresh_nonce_after_interruption_allows_only_remaining_range(){
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("photo");
        let bytes=vec![0x46;TOTAL];
        fs::write(&path,&bytes).unwrap();
        let n=selected(&bytes);
        let snap=SelectedSourceSnapshotLab::from_core_selected_file(
            File::open(&path).unwrap(),n
        ).unwrap();
        let signer=Ed25519KeyPair::from_seed_unchecked(&[37;32]).unwrap();
        let pk:[u8;32]=signer.public_key().as_ref().try_into().unwrap();
        let mut journal=GrantSpendLedgerLab::open(
            &dir.path().join("spent.sqlite")
        ).unwrap();
        let first=issue_owner_read_grant(&n,&signer,&LocalFixtureConsent,
            window(10,0,MID as u64),111).unwrap();
        let initial=snap.serve_once(&first,&pk,7,n.receiver().endpoint_id,
            0,MID as u64,112,&mut journal).unwrap();
        assert_eq!(initial,&bytes[..MID]);
        assert_eq!(
            snap.serve_once(&first,&pk,7,n.receiver().endpoint_id,
                0,MID as u64,112,&mut journal),
            Err(SourceServeError::GrantAlreadySpent(GrantSpendError::AlreadyUsed))
        );
        let second=issue_owner_read_grant(&n,&signer,&LocalFixtureConsent,
            window(11,MID as u64,TOTAL as u64),112).unwrap();
        assert_eq!(
            snap.serve_once(&second,&pk,7,n.receiver().endpoint_id,
                0,MID as u64,113,&mut journal),
            Err(SourceServeError::SignedGrant(GrantError::InvalidBounds))
        );
        assert_eq!(
            snap.serve_once(&second,&pk,7,n.receiver().endpoint_id,
                MID as u64,TOTAL as u64,113,&mut journal).unwrap(),
            &bytes[MID..]
        );
        assert_eq!(journal.count_for_lab(),Ok(2));
    }
}
