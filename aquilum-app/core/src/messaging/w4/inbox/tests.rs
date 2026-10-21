use super::*;
use super::super::{verify_receipt, Accept, Status, W4Outbox};
use crate::messaging::Descriptor;
use ring::signature::{Ed25519KeyPair, KeyPair};
use tempfile::tempdir;

struct LocalTrust {
    recipient: DeviceId,
    epoch: u64,
    key: [u8;32],
    revoked: bool,
}
impl TrustedRecipientKeys for LocalTrust {
    fn key_for(&self, device:DeviceId, epoch:u64)->Option<[u8;32]> {
        if !self.revoked && self.recipient==device && self.epoch==epoch {
            Some(self.key)
        } else { None }
    }
}
fn fixture() -> (Descriptor, DeviceId, Vec<u8>, Ed25519KeyPair, LocalTrust) {
    let signer=Ed25519KeyPair::from_seed_unchecked(&[7u8;32]).unwrap();
    let recipient=DeviceId::from_uuid(uuid::Uuid::from_bytes([1u8;16]));
    let sender=DeviceId::from_uuid(uuid::Uuid::from_bytes([3u8;16]));
    let bytes=vec![0xDE,0xAD,0x13,0x7D];
    let desc=Descriptor {
        id:[11u8;16], recipient,
        ciphertext_digest:*blake3::hash(&bytes).as_bytes(),
        policy_epoch:5, expires_at_ms:9_000,
    };
    let mut pubkey=[0u8;32];
    pubkey.copy_from_slice(signer.public_key().as_ref());
    let trust=LocalTrust { recipient,epoch:5,key:pubkey,revoked:false };
    (desc,sender,bytes,signer,trust)
}
fn authenticated<'a>(
    desc:Descriptor, sender:DeviceId, bytes:&'a[u8],
)->AuthenticatedW3<'a> {
    AuthenticatedW3 {
        sender,recipient:desc.recipient,message:desc.id,
        packet:bytes,digest:*blake3::hash(bytes).as_bytes(),
        epoch:desc.policy_epoch,expires_at_ms:desc.expires_at_ms,
    }
}

#[test]
fn receiver_returns_real_w4_receipt_only_after_durable_inbox_commit() {
    let tmp=tempdir().unwrap();
    let (desc,sender,bytes,signer,keys)=fixture();
    let sender_path=tmp.path().join("sender.sqlite");
    let receiver_path=tmp.path().join("receiver.sqlite");
    let mut outgoing=W4Outbox::open(&sender_path).unwrap();
    assert_eq!(outgoing.enqueue(desc,&bytes,1).unwrap(),super::super::Insert::New);
    let raw={
        let mut incoming=W4Inbox::open(&receiver_path).unwrap();
        let envelope=authenticated(desc,sender,&bytes);
        let receipt=incoming.accept_verified(&envelope,&signer,&keys,2).unwrap();
        assert_eq!(receipt.status,InboundStatus::Stored);
        assert_eq!(incoming.row_count().unwrap(),1);
        assert_eq!(verify_receipt(&receipt.signed_receipt,&keys).unwrap().message(),desc.id);
        receipt.signed_receipt
    };
    assert_eq!(outgoing.accept_receipt(&raw,3,&keys).unwrap(),Accept::New);
    assert_eq!(outgoing.status(desc.recipient,desc.id).unwrap(),Some(Status::Accepted));
    let mut reopened=W4Inbox::open(&receiver_path).unwrap();
    let envelope=authenticated(desc,sender,&bytes);
    let duplicate=reopened.accept_verified(&envelope,&signer,&keys,4).unwrap();
    assert_eq!(duplicate.status,InboundStatus::Duplicate);
    assert_eq!(duplicate.signed_receipt,raw);
    assert_eq!(reopened.row_count().unwrap(),1);
}

#[test]
fn no_receipt_if_receiver_write_fails_even_after_signing() {
    let tmp=tempdir().unwrap();
    let (desc,sender,bytes,signer,keys)=fixture();
    let mut inbox=W4Inbox::open(&tmp.path().join("broken.sqlite")).unwrap();
    inbox.db.execute_batch(
        "CREATE TRIGGER fail_commit BEFORE INSERT ON messaging_w4_inbox
         BEGIN SELECT RAISE(ABORT,'simulated write failure'); END;"
    ).unwrap();
    let envelope=authenticated(desc,sender,&bytes);
    assert!(matches!(inbox.accept_verified(&envelope,&signer,&keys,2),Err(Error::Database(_))));
    assert_eq!(inbox.row_count().unwrap(),0);
    inbox.db.execute_batch("DROP TRIGGER fail_commit;").unwrap();
    let valid=inbox.accept_verified(&envelope,&signer,&keys,3).unwrap();
    assert_eq!(valid.status,InboundStatus::Stored);
    assert_eq!(inbox.row_count().unwrap(),1);
}

#[test]
fn unknown_recipient_impostor_signer_revocation_and_expiry_fail_closed() {
    let tmp=tempdir().unwrap();
    let (desc,sender,bytes,signer,mut keys)=fixture();
    let mut inbox=W4Inbox::open(&tmp.path().join("protected.sqlite")).unwrap();
    let envelope=authenticated(desc,sender,&bytes);
    keys.revoked=true;
    assert!(matches!(inbox.accept_verified(&envelope,&signer,&keys,2),Err(Error::Unauthorized)));
    keys.revoked=false;
    let impostor=Ed25519KeyPair::from_seed_unchecked(&[4;32]).unwrap();
    assert!(matches!(inbox.accept_verified(&envelope,&impostor,&keys,3),Err(Error::Unauthorized)));
    assert!(matches!(inbox.accept_verified(&envelope,&signer,&keys,9_000),
                     Err(Error::InvalidDescriptor)));
    assert_eq!(inbox.row_count().unwrap(),0);
}

#[test]
fn immutable_sender_scoped_inbox_rejects_changed_content_and_late_replay() {
    let tmp=tempdir().unwrap();
    let (desc,sender,bytes,signer,keys)=fixture();
    let path=tmp.path().join("inbox.sqlite");
    let mut inbox=W4Inbox::open(&path).unwrap();
    let verified=authenticated(desc,sender,&bytes);
    assert_eq!(inbox.accept_verified(&verified,&signer,&keys,1).unwrap().status,InboundStatus::Stored);
    let different=vec![0xEE,0xF1];
    let mutated=authenticated(desc,sender,&different);
    assert!(matches!(inbox.accept_verified(&mutated,&signer,&keys,2),Err(Error::Conflict)));
    let wrong_epoch=AuthenticatedW3 {epoch:desc.policy_epoch+1,..verified.clone()};
    assert!(matches!(inbox.accept_verified(&wrong_epoch,&signer,&keys,3),Err(Error::Unauthorized)));
    let wrong_digest=AuthenticatedW3 {digest:[0;32],..verified.clone()};
    assert!(matches!(inbox.accept_verified(&wrong_digest,&signer,&keys,4),Err(Error::Mismatch)));
    assert_eq!(inbox.row_count().unwrap(),1);
}

#[test]
fn single_sender_is_bounded_by_explicit_receiver_quota() {
    let tmp=tempdir().unwrap();
    let (desc,sender,bytes,signer,keys)=fixture();
    let mut inbox=W4Inbox::open(&tmp.path().join("quota.sqlite")).unwrap();
    for seq in 0..(MAX_INBOX_PER_SENDER as u8) {
        let mut d=desc;
        d.id[0]=seq;
        let input=authenticated(d,sender,&bytes);
        assert_eq!(inbox.accept_verified(&input,&signer,&keys,1).unwrap().status,
                   InboundStatus::Stored);
    }
    assert_eq!(inbox.row_count().unwrap(),MAX_INBOX_PER_SENDER);
    let mut too_many=desc;
    too_many.id[0]=MAX_INBOX_PER_SENDER as u8;
    let input=authenticated(too_many,sender,&bytes);
    assert!(matches!(inbox.accept_verified(&input,&signer,&keys,2),
                     Err(Error::MailboxFull)));
    assert_eq!(inbox.row_count().unwrap(),MAX_INBOX_PER_SENDER);
}
