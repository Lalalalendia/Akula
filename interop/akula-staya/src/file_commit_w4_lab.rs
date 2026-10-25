//! Lab-only signed WOW W4 FILE_COMMITTED receipt, distinct from ordinary W4.
//! Never claim this from a range ACK, W3 notice, SQLite metadata alone or
//! unauthenticated bytes. Recheck the existing private Inbox from disk before
//! signing; no new file database and no Vault/File Gate authority.
use akula_inbox_ledger_lab::{InboxLedger,LocalStatus};
use akula_wow_w3_crypto::{
    file_receipt_w4::{self,FileCommitStatement,FILE_RECEIPT_BYTES},
    trust_w3::TrustRegistry,
};
use aquilum_core::p2p_attachment_contract::AttachmentNoticeV1;
use ed25519_dalek::{SigningKey,VerifyingKey};
use std::path::Path;
use crate::granted_inbox_lab::VerifiedPrivateInboxLocal;

/// Return a stable, signed statement only after cold SQLite+physical receipt
/// validation and separately trusted recipient signer enrollment. The supplied
/// W3 ciphertext digest is bound to the exact immutable authenticated packet.
pub fn sign_cold_file_commit(
    local:&VerifiedPrivateInboxLocal,
    approved:&AttachmentNoticeV1,
    inbox_root:&Path,
    original_authenticated_w3_packet:&[u8],
    recipient_signer:&SigningKey,
    trusted_signer_key:&VerifyingKey,
    registry:&TrustRegistry,
    recipient_epoch:u64,
)->Result<[u8;FILE_RECEIPT_BYTES],String>{
    if local.message_id!=approved.message_id()
        || local.committed.bytes!=approved.size()
        || local.committed.blake3!=approved.root()
        || local.committed.sender_key!=approved.sender().endpoint_id
        || local.committed.recipient_key!=approved.receiver().endpoint_id
        || original_authenticated_w3_packet.is_empty()
        || original_authenticated_w3_packet.len()>16*1024
        || recipient_epoch==0
        || recipient_signer.verifying_key()!=*trusted_signer_key
    {return Err("W4 content intent mismatch or wrong signer".into());}
    let recipient=*approved.receiver().device_id.as_uuid().as_bytes();
    registry.verify(recipient,recipient_epoch,trusted_signer_key)
        .map_err(|e|format!("W4 independent enrollment: {e:?}"))?;
    let cold=InboxLedger::open_existing(inbox_root)
        .map_err(|e|format!("cold Inbox open: {e:?}"))?
        .ok_or("Inbox does not exist")?;
    cold.verify_committed_intent(&local.committed)
        .map_err(|e|format!("cold immutable transfer: {e:?}"))?;
    if cold.status(local.committed.id)
        .map_err(|e|format!("cold Inbox BLAKE3: {e:?}"))?
        !=LocalStatus::VerifiedLocal
    {return Err("local file not verified".into());}
    let (_pinned,physical)=cold.open_verified_file(local.committed.id)
        .map_err(|e|format!("cold pinned object verification: {e:?}"))?;
    if physical!=local.committed{return Err("physical object differs".into());}
    let statement=FileCommitStatement{
        recipient_id:recipient,
        message_id:local.message_id,
        w3_packet_digest:*blake3::hash(original_authenticated_w3_packet).as_bytes(),
        file_blake3:approved.root(),
        file_size:approved.size(),
        inbox_transfer_id:*local.committed.id.as_bytes(),
        recipient_epoch,
    };
    file_receipt_w4::sign_file_committed(recipient_signer,statement)
        .map_err(|e|format!("W4 signing: {e:?}"))
}

/// Sender side MUST separately verify the original local W3 packet, the
/// selected notice and recipient key. A valid W4 means only private Inbox
/// commit, never a Vault/File Gate import.
pub fn verify_cold_file_commit(
    receipt:&[u8],
    approved:&AttachmentNoticeV1,
    authenticated_original_w3_packet:&[u8],
    independently_trusted_receivers:&TrustRegistry,
)->Result<FileCommitStatement,String>{
    if authenticated_original_w3_packet.is_empty()
        || authenticated_original_w3_packet.len()>16*1024
    {return Err("invalid original W3 packet".into());}
    let statement=file_receipt_w4::verify_file_committed(
        receipt,independently_trusted_receivers,
    ).map_err(|e|format!("invalid W4: {e:?}"))?;
    if statement.recipient_id!=*approved.receiver().device_id.as_uuid().as_bytes()
        || statement.message_id!=approved.message_id()
        || statement.file_blake3!=approved.root()
        || statement.file_size!=approved.size()
        || statement.w3_packet_digest!=*blake3::hash(authenticated_original_w3_packet).as_bytes()
    {return Err("signed W4 does not bind original content and W3".into());}
    Ok(statement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::granted_inbox_lab::VerifiedPrivateInboxLocal;
    use akula_inbox_ledger_lab::InboxLedger;
    use aquilum_core::{
        p2p_offer_lab::{DecisionAction,OfferedFile,OfferV1,PeerBinding,SCHEMA},
        sync::DeviceId,
    };
    use uuid::Uuid;
    use std::fs;
    #[test]
    fn w4_recovered_after_lost_ack_but_never_from_corrupt_or_uncommitted_file() {
        let tmp=tempfile::tempdir().unwrap();
        let root=tmp.path().join("inbox");
        fs::create_dir(&root).unwrap();
        let contents=vec![7u8;1024*1024];
        let digest=*blake3::hash(&contents).as_bytes();
        let sender=PeerBinding{
            device_id:DeviceId::from_uuid(Uuid::from_u128(41)),endpoint_id:[11;32]
        };
        let receiver=PeerBinding{
            device_id:DeviceId::from_uuid(Uuid::from_u128(42)),endpoint_id:[22;32]
        };
        let offer=OfferV1{schema:SCHEMA,offer_id:Uuid::from_u128(77),
            nonce:[3;16],sender,receiver,created_unix:100,expires_unix:500,
            files:vec![OfferedFile{
                label:"photo".into(),bytes:contents.len() as u64,blake3_root:digest
            }]
        };
        let decision=offer.receiver_decision(
            DecisionAction::Accept,vec![0],101,sender,receiver
        ).unwrap();
        let notice=AttachmentNoticeV1::from_accepted(
            &offer,&decision,sender,receiver,0,102
        ).unwrap();
        let signer=SigningKey::from_bytes(&[31;32]);
        let vk=signer.verifying_key();
        let mut trust=TrustRegistry::new();
        trust.enroll(*receiver.device_id.as_uuid().as_bytes(),7,vk).unwrap();
        let w3=b"synthetic-W3-ciphertext-digest-fixture";
        let fake=VerifiedPrivateInboxLocal {
            message_id:notice.message_id(),
            committed:akula_inbox_ledger_lab::ExpectedTransfer {
                id:Uuid::from_u128(55),sender_key:sender.endpoint_id,
                recipient_key:receiver.endpoint_id,
                source:akula_inbox_ledger_lab::Transport::P2p,
                bytes:contents.len() as u64,blake3:digest,
            }
        };
        assert!(sign_cold_file_commit(
            &fake,&notice,&root,w3,&signer,&vk,&trust,7
        ).is_err(),"no W4 before verified Inbox");
        let row=crate::one_file_lan::persist_granted_private_inbox(
            &contents,&root,contents.len() as u64,digest,
            sender.endpoint_id,receiver.endpoint_id
        ).unwrap();
        let valid=VerifiedPrivateInboxLocal{committed:row.clone(),..fake};
        let signed=sign_cold_file_commit(
            &valid,&notice,&root,w3,&signer,&vk,&trust,7
        ).unwrap();
        // A lost receipt can be regenerated deterministically, without
        // retransmitting content and without creating a second Inbox row.
        let retransmit=sign_cold_file_commit(
            &valid,&notice,&root,w3,&signer,&vk,&trust,7
        ).unwrap();
        assert_eq!(signed,retransmit);
        assert_eq!(InboxLedger::open_existing(&root).unwrap().unwrap().count().unwrap(),1);
        assert_eq!(verify_cold_file_commit(&signed,&notice,w3,&trust).unwrap().file_blake3,digest);
        assert!(verify_cold_file_commit(&signed,&notice,b"other-W3",&trust).is_err());
        let mut tampered=signed;
        tampered[64]^=1;
        assert!(verify_cold_file_commit(&tampered,&notice,w3,&trust).is_err());
        let ledger=InboxLedger::open_existing(&root).unwrap().unwrap();
        fs::write(ledger.object_path(row.id),b"corrupted").unwrap();
        drop(ledger);
        assert!(sign_cold_file_commit(
            &valid,&notice,&root,w3,&signer,&vk,&trust,7
        ).is_err(),"no W4 when physical file is altered after SQLite commit");
        trust.revoke(*receiver.device_id.as_uuid().as_bytes()).unwrap();
        assert!(verify_cold_file_commit(&signed,&notice,w3,&trust).is_err());
    }
}
