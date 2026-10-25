//! Default-off Akula verified private Inbox publication after real signed
//! grant / Staya range transfer. Reuses the SAME native one-file Inbox writer,
//! including Windows pinned NT stage + SQLite custody and cold verification.
//!
//! A QUIC range-consumption ACK is never a content-delivered receipt. This
//! module publishes only after full BLAKE3 and a separate SQLite+physical-file
//! cold read. It does NOT send WOW W4, apply a File Gate change, modify Vault,
//! start a listener, or authorize local source reads.
use crate::{
    attachment_contract::CheckedResume,
    granted_quic_lab::request_granted_range_once,
};
use aquilum_core::p2p_attachment_contract::AttachmentNoticeV1;
use akula_inbox_ledger_lab::{
    ExpectedTransfer, InboxLedger, LocalStatus,
};
use iroh::endpoint::Connection;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
use staya_transfer::TransferRequest;

/// Authenticated local result, not a signed remote receipt. No API converts
/// this result directly into a W4 or Vault write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedPrivateInboxLocal {
    pub message_id: [u8; 16],
    pub committed: ExpectedTransfer,
}

fn checked_cold_prefix(
    mut private_stage: Option<&mut File>,
    resume: CheckedResume,
    expected_range_start: u64,
) -> Result<Vec<u8>, String> {
    if resume.offset != expected_range_start
        || resume.offset > 8 * 1024 * 1024
        || resume.verified_prefix == [0; 32]
    {
        return Err("private checkpoint and Staya range do not agree".into());
    }
    if resume.offset == 0 {
        if private_stage.is_some() || resume.verified_prefix != *blake3::hash(b"").as_bytes() {
            return Err("empty checkpoint must be exactly BLAKE3(empty)".into());
        }
        return Ok(Vec::new());
    }
    let stage = private_stage.as_mut().ok_or("missing authenticated private stage")?;
    let metadata = stage.metadata().map_err(|e| format!("stage metadata: {e}"))?;
    if !metadata.is_file() || metadata.len() != resume.offset {
        return Err("private stage size changed before resume".into());
    }
    stage.seek(SeekFrom::Start(0)).map_err(|e| format!("seek stage: {e}"))?;
    let mut prefix = Vec::with_capacity(resume.offset as usize);
    stage.take(resume.offset + 1).read_to_end(&mut prefix)
        .map_err(|e| format!("cold private stage read: {e}"))?;
    if prefix.len() as u64 != resume.offset
        || *blake3::hash(&prefix).as_bytes() != resume.verified_prefix
    {
        return Err("private prefix BLAKE3 changed or was truncated".into());
    }
    Ok(prefix)
}

fn publish_full_verified_payload(
    data: &[u8],
    notice: &AttachmentNoticeV1,
    inbox_root: &Path,
    authenticated_sender: [u8; 32],
    locally_authenticated_receiver: [u8; 32],
) -> Result<VerifiedPrivateInboxLocal, String> {
    if authenticated_sender != notice.sender().endpoint_id
        || locally_authenticated_receiver != notice.receiver().endpoint_id
        || data.len() as u64 != notice.size()
        || *blake3::hash(data).as_bytes() != notice.root()
    {
        return Err("complete content or enrolled endpoints differ from approved notice".into());
    }
    // One authoritative Inbox implementation, not an alternate SQL schema.
    // The existing native publisher verifies the physical full file and SQLite
    // intent and on Windows verifies pinned-root custody. No Vault writes.
    let persisted = crate::one_file_lan::persist_granted_private_inbox(
        data, inbox_root, notice.size(), notice.root(),
        authenticated_sender, locally_authenticated_receiver,
    )?;
    let cold = InboxLedger::open_existing(inbox_root)
        .map_err(|e| format!("cold read-only Inbox: {e:?}"))?
        .ok_or("missing private Inbox after publication")?;
    cold.verify_committed_intent(&persisted)
        .map_err(|e| format!("cold committed intent: {e:?}"))?;
    if cold.status(persisted.id).map_err(|e| format!("Inbox status: {e:?}"))?
        != LocalStatus::VerifiedLocal
    {
        return Err("private Inbox not physically verified after restart".into());
    }
    let (mut file, stored) = cold.open_verified_file(persisted.id)
        .map_err(|e| format!("cold reopen verified object: {e:?}"))?;
    if stored != persisted {
        return Err("private Inbox row changed during verification".into());
    }
    let mut physical = Vec::with_capacity(data.len());
    file.take(notice.size() + 1).read_to_end(&mut physical)
        .map_err(|e| format!("cold physical Inbox read: {e}"))?;
    if physical != data {
        return Err("private Inbox bytes changed before local receipt".into());
    }
    Ok(VerifiedPrivateInboxLocal {
        message_id: notice.message_id(),
        committed: persisted,
    })
}

/// Receives only the selected suffix, after independently rehashing the
/// caller's preopened checkpoint File. Then verifies COMPLETE bytes and
/// publishes them in the existing separately audited private Inbox.
///
/// The sender sees only a RANGE_READ_ONLY acknowledgment from the older
/// transfer helper. It does not receive this local result or a W4 receipt.
/// A future separate W4 signer must require this returned verified intent.
pub async fn request_granted_range_to_verified_inbox_once(
    connection: &Connection,
    notice: &AttachmentNoticeV1,
    request: &TransferRequest,
    checkpoint: CheckedResume,
    private_partial_stage: Option<&mut File>,
    signed_grant: &[u8; aquilum_core::p2p_attachment_grant_lab::GRANT_BYTES],
    independently_enrolled_sender_endpoint: [u8; 32],
    locally_authenticated_receiver_endpoint: [u8; 32],
    private_inbox_root: &Path,
) -> Result<VerifiedPrivateInboxLocal, String> {
    if locally_authenticated_receiver_endpoint != notice.receiver().endpoint_id
        || connection.remote_id().as_bytes() != &independently_enrolled_sender_endpoint
        || notice.sender().endpoint_id != independently_enrolled_sender_endpoint
    {
        return Err("mismatched authenticated sender or receiver identity".into());
    }
    if request.manifest.size != notice.size()
        || request.manifest.id.0 != notice.root()
        || request.range.end() != notice.size()
    {
        return Err("request does not cover approved object remainder".into());
    }
    // Verify the stage BEFORE sending the signed one-shot GET. An attacker
    // who changes the local stage cannot cause a grant to be spent.
    let mut complete = checked_cold_prefix(
        private_partial_stage, checkpoint, request.range.start(),
    )?;
    let suffix = request_granted_range_once(
        connection, notice, request, signed_grant,
        independently_enrolled_sender_endpoint
    ).await?;
    complete.extend_from_slice(&suffix);
    if complete.len() as u64 != notice.size()
        || *blake3::hash(&complete).as_bytes() != notice.root()
    {
        return Err("combined checkpoint + range failed full BLAKE3".into());
    }
    publish_full_verified_payload(
        &complete, notice, private_inbox_root,
        independently_enrolled_sender_endpoint,
        locally_authenticated_receiver_endpoint,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        attachment_contract::plan_resume,
        granted_quic_lab::{serve_granted_range_once, GRANTED_RANGE_ALPN},
    };
    use aquilum_core::{
        p2p_attachment_grant_lab::{
            issue_owner_read_grant, GrantWindow, LocalReadPolicy,
        },
        p2p_attachment_grant_spend_lab::GrantSpendLedgerLab,
        p2p_attachment_source_lab::SelectedSourceSnapshotLab,
        p2p_offer_lab::{
            DecisionAction, OfferedFile, OfferV1, PeerBinding, SCHEMA,
        },
        sync::DeviceId,
    };
    use iroh::{Endpoint, EndpointAddr, RelayMode, endpoint::presets};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        net::{IpAddr, Ipv4Addr, SocketAddr},
        time::Duration,
    };
    use uuid::Uuid;

    struct FixtureAcl;
    impl LocalReadPolicy for FixtureAcl {
        fn permit_exact_object(
            &self, _: PeerBinding, _: PeerBinding, _: [u8; 32], _: u64
        ) -> bool { true }
    }
    const TOTAL: usize = 5 * 1024 * 1024;
    const OFFSET: usize = 2 * 1024 * 1024;
    fn source() -> Vec<u8> {
        (0..TOTAL).map(|n| ((n*79) ^ (n >> 4)) as u8).collect()
    }
    fn notice(
        bytes: &[u8], sender: PeerBinding, receiver: PeerBinding,
    ) -> AttachmentNoticeV1 {
        let offer = OfferV1 {
            schema: SCHEMA, offer_id: Uuid::from_u128(420), nonce: [19; 16],
            sender, receiver, created_unix: 100, expires_unix: 500,
            files: vec![OfferedFile {
                label: "selected-photo.bin".into(),
                bytes: bytes.len() as u64,
                blake3_root: *blake3::hash(bytes).as_bytes(),
            }],
        };
        let choice = offer.receiver_decision(
            DecisionAction::Accept, vec![0], 110, sender, receiver
        ).unwrap();
        AttachmentNoticeV1::from_accepted(
            &offer, &choice, sender, receiver, 0, 120
        ).unwrap()
    }

    #[test]
    fn corrupt_checkpoint_or_unselected_content_is_never_published() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = source();
        let sender = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(17)),
            endpoint_id: [11; 32],
        };
        let receiver = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(23)),
            endpoint_id: [27; 32],
        };
        let notice = notice(&bytes, sender, receiver);
        let root = dir.path().join("inbox");
        fs::create_dir(&root).unwrap();
        let stage_path = dir.path().join("private.part");
        fs::write(&stage_path, &bytes[..OFFSET]).unwrap();
        let checked = CheckedResume {
            offset: OFFSET as u64,
            verified_prefix: *blake3::hash(&bytes[..OFFSET]).as_bytes(),
        };
        let mut handle = File::open(&stage_path).unwrap();
        assert_eq!(checked_cold_prefix(
            Some(&mut handle), checked, OFFSET as u64
        ).unwrap(), bytes[..OFFSET]);
        fs::write(&stage_path, vec![0x42; OFFSET]).unwrap();
        let mut altered = File::open(&stage_path).unwrap();
        assert!(checked_cold_prefix(
            Some(&mut altered), checked, OFFSET as u64
        ).is_err());
        assert!(checked_cold_prefix(None, checked, OFFSET as u64).is_err());
        assert!(checked_cold_prefix(
            None, checked, (OFFSET + 64 * 1024) as u64
        ).is_err());
        assert!(publish_full_verified_payload(
            &bytes[..OFFSET], &notice, &root,
            sender.endpoint_id, receiver.endpoint_id
        ).is_err());
        let mut changed = bytes.clone();
        changed[OFFSET] ^= 1;
        assert!(publish_full_verified_payload(
            &changed, &notice, &root,
            sender.endpoint_id, receiver.endpoint_id
        ).is_err());
        assert!(publish_full_verified_payload(
            &bytes, &notice, &root,
            [99; 32], receiver.endpoint_id
        ).is_err());
        assert!(publish_full_verified_payload(
            &bytes, &notice, &root,
            sender.endpoint_id, [99; 32]
        ).is_err());
        assert!(!root.join("received-inbox.sqlite3").exists(),
            "bad content cannot create a committed Inbox ledger");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_signed_quic_suffix_is_cold_verified_in_private_inbox_not_vault() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = source();
        let path = dir.path().join("source.bin");
        fs::write(&path, &bytes).unwrap();
        let private = dir.path().join("private-inbox");
        fs::create_dir(&private).unwrap();
        let stage_path = dir.path().join("private-prefix.stage");
        fs::write(&stage_path, &bytes[..OFFSET]).unwrap();

        let sender = Endpoint::builder(presets::Minimal)
            .secret_key(iroh::SecretKey::generate())
            .alpns(vec![GRANTED_RANGE_ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind().await.unwrap();
        let sender_id = sender.id();
        let port = sender.bound_sockets().into_iter()
            .find(|x| x.is_ipv4()).unwrap().port();
        let direct = EndpointAddr::new(sender_id)
            .with_ip_addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
        let receiver = Endpoint::builder(presets::Minimal)
            .secret_key(iroh::SecretKey::generate())
            .alpns(vec![GRANTED_RANGE_ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind().await.unwrap();
        let receiver_id = receiver.id();
        let owner = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(17)),
            endpoint_id: *sender_id.as_bytes(),
        };
        let target = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(23)),
            endpoint_id: *receiver_id.as_bytes(),
        };
        let notice = notice(&bytes, owner, target);
        let snapshot = SelectedSourceSnapshotLab::from_core_selected_file(
            File::open(&path).unwrap(), notice
        ).unwrap();
        let key = Ed25519KeyPair::from_seed_unchecked(&[21; 32]).unwrap();
        let pubkey: [u8; 32] = key.public_key().as_ref().try_into().unwrap();
        let grant = issue_owner_read_grant(
            &notice, &key, &FixtureAcl,
            GrantWindow {
                start: OFFSET as u64,
                end: TOTAL as u64,
                issued_unix: 130,
                expires_unix: 250,
                owner_key_epoch: 7,
                nonce: [31; 16],
            }, 150
        ).unwrap();
        let (checkpoint, request) = plan_resume(
            &notice, OFFSET as u64,
            *blake3::hash(&bytes[..OFFSET]).as_bytes()
        ).unwrap();
        let mut spent = GrantSpendLedgerLab::open(
            &dir.path().join("spent.sqlite")
        ).unwrap();

        let sender_task = async {
            let incoming = tokio::time::timeout(
                Duration::from_secs(45), sender.accept()
            ).await.unwrap().unwrap();
            let conn = incoming.accept().unwrap().await.unwrap();
            assert_eq!(conn.remote_id(), receiver_id);
            tokio::time::timeout(
                Duration::from_secs(45),
                serve_granted_range_once(
                    &conn, &snapshot, &pubkey, 7, 150,
                    &mut spent, |conn| if conn.remote_id() == receiver_id {
                        Ok(())
                    } else {
                        Err("not authenticated test receiver".into())
                    }
                )
            ).await.unwrap().unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
            conn.close(0u8.into(), b"range read only not delivery W4");
            sender.close().await;
            assert_eq!(spent.count_for_lab(), Ok(1));
        };
        let receiver_task = async {
            let conn = tokio::time::timeout(
                Duration::from_secs(45),
                receiver.connect(direct, GRANTED_RANGE_ALPN)
            ).await.unwrap().unwrap();
            assert_eq!(conn.remote_id(), sender_id);
            let mut stage = File::open(&stage_path).unwrap();
            let committed = tokio::time::timeout(
                Duration::from_secs(45),
                request_granted_range_to_verified_inbox_once(
                    &conn, &notice, &request, checkpoint,
                    Some(&mut stage), &grant,
                    *sender_id.as_bytes(), *receiver_id.as_bytes(),
                    &private,
                )
            ).await.unwrap().unwrap();
            assert_eq!(committed.message_id, notice.message_id());
            assert_eq!(committed.committed.blake3, notice.root());
            assert_eq!(committed.committed.sender_key, *sender_id.as_bytes());
            assert_eq!(committed.committed.recipient_key, *receiver_id.as_bytes());
            let cold = InboxLedger::open_existing(&private).unwrap().unwrap();
            assert_eq!(cold.count().unwrap(), 1);
            assert_eq!(cold.status(committed.committed.id).unwrap(), LocalStatus::VerifiedLocal);
            let (mut opened, exact) = cold.open_verified_file(committed.committed.id).unwrap();
            assert_eq!(exact, committed.committed);
            let mut observed = Vec::new();
            opened.read_to_end(&mut observed).unwrap();
            assert_eq!(observed, bytes);
            let outside = dir.path().join("user-vault");
            assert!(!outside.exists(), "network receipt must not write Vault");
            tokio::time::sleep(Duration::from_millis(300)).await;
            conn.close(0u8.into(), b"local verified only not signed W4");
            receiver.close().await;
        };
        tokio::time::timeout(
            Duration::from_secs(115),
            async { tokio::join!(sender_task, receiver_task); }
        ).await.unwrap();
    }
}
