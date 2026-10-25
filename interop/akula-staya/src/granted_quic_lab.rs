//! Experimental real Iroh/QUIC server for ONE strictly signed/selected Staya
//! range. It has no CLI, no listener startup, no File Gate or Vault writes.
//! Callers must establish and OS-pin both peers before passing Connections.
//! The only recipient bytes come from the authenticated QUIC handshake, never
//! from a remote-declared identity or an unauthenticated manifest.
//!
//! A grant is durably spent BEFORE the first source byte. The sender exposes
//! immutable, locally verified Core-selected bytes; a failed connection must
//! get a NEW owner-issued grant for its next missing range. The final response
//! only means range bytes read, not verified/durable Inbox or signed WOW W4.
use aquilum_core::{
    p2p_attachment_contract::AttachmentNoticeV1,
    p2p_attachment_grant_lab::GRANT_BYTES,
    p2p_attachment_grant_spend_lab::GrantSpendLedgerLab,
    p2p_attachment_source_lab::SelectedSourceSnapshotLab,
};
use iroh::endpoint::Connection;
use staya_transfer::{ContentId, TransferRequest};

pub const GRANTED_RANGE_ALPN: &[u8] = b"akula/p2p-signed-range-lab/1";
const MAGIC: &[u8; 8] = b"AKGETR01";
const RANGE_READ_ONLY: &[u8] = b"AKRANGE_READ_NOT_W4";
const REQUEST_BYTES: usize = 8 + 16 + 8 + 8 + GRANT_BYTES;
const MAX_RANGE_BYTES: usize = 8 * 1024 * 1024;
const DEADLINE_SECS: u64 = 40;

/// Real authenticated QUIC receive endpoint: sender owns the selected
/// snapshot, independently enrolled owner public key and spend journal.
/// Returns success only for *range read*, not full-file delivery.
pub async fn serve_granted_range_once(
    connection: &Connection,
    snapshot: &SelectedSourceSnapshotLab,
    owner_public_key: &[u8; 32],
    owner_epoch: u64,
    now: u64,
    journal: &mut GrantSpendLedgerLab,
    recheck_live_peer_pin: impl Fn(&Connection) -> Result<(), String>,
) -> Result<(), String> {
    // Must precede opening request streams and authorizing source bytes.
    recheck_live_peer_pin(connection)?;
    let live_receiver = *connection.remote_id().as_bytes();
    if live_receiver != snapshot.receiver_endpoint() {
        return Err("authenticated QUIC receiver differs from Core selection".into());
    }
    let mut request = tokio::time::timeout(
        std::time::Duration::from_secs(DEADLINE_SECS), connection.accept_uni()
    ).await.map_err(|_| "range request deadline")?
     .map_err(|e| format!("range request stream: {e}"))?;
    let bytes = request.read_to_end(REQUEST_BYTES)
        .await.map_err(|e| format!("bounded grant request: {e}"))?;
    if bytes.len() != REQUEST_BYTES
        || &bytes[..8] != MAGIC
        || bytes[8..24] != snapshot.message_id()
    {
        return Err("wrong range request, message binding or extra bytes".into());
    }
    let start = u64::from_be_bytes(bytes[24..32].try_into().unwrap());
    let end = u64::from_be_bytes(bytes[32..40].try_into().unwrap());
    // In particular, the verified QUIC endpoint is passed to the Core,
    // not any claim in the request body. A nonce must be recorded atomically
    // before any file content is sent.
    recheck_live_peer_pin(connection)?;
    let payload = snapshot.serve_once(
        &bytes[40..], owner_public_key, owner_epoch, live_receiver,
        start, end, now, journal
    ).map_err(|e| format!("Core signed source/range rejected: {e:?}"))?;
    let mut tx = connection.open_uni().await
        .map_err(|e| format!("open guarded stream: {e}"))?;
    for chunk in payload.chunks(64 * 1024) {
        tx.write_all(chunk).await
            .map_err(|e| format!("send authorized range: {e}"))?;
    }
    tx.finish().map_err(|e| format!("finish guarded stream: {e}"))?;
    // Transport-level finalization is not delivery. This reply verifies
    // only that all range bytes were consumed at the other end.
    let mut result = tokio::time::timeout(
        std::time::Duration::from_secs(DEADLINE_SECS), connection.accept_uni()
    ).await.map_err(|_| "range read-only reply deadline")?
     .map_err(|e| format!("range reply stream: {e}"))?;
    let reply = result.read_to_end(64).await
        .map_err(|e| format!("bounded range reply: {e}"))?;
    if reply != RANGE_READ_ONLY {
        return Err("receiver did not confirm range byte consumption".into());
    }
    Ok(())
}

/// This call does NOT import to Vault, write an Inbox record, or ACK W4.
/// The returned suffix must be merged with a *separately verified* on-disk
/// checkpoint and checked against the complete trusted BLAKE3 root before
/// a local verified Inbox commit can occur.
pub async fn request_granted_range_once(
    connection: &Connection,
    approved_notice: &AttachmentNoticeV1,
    planned: &TransferRequest,
    signed_grant: &[u8; GRANT_BYTES],
    independently_enrolled_sender_endpoint: [u8; 32],
) -> Result<Vec<u8>, String> {
    if connection.remote_id().as_bytes() != &independently_enrolled_sender_endpoint
        || approved_notice.sender().endpoint_id != independently_enrolled_sender_endpoint
    {
        return Err("real sender QUIC key is not the pinned owner".into());
    }
    let start = planned.range.start();
    let end = planned.range.end();
    let wanted = end.checked_sub(start).ok_or("invalid Staya range")?;
    if wanted == 0
        || wanted > MAX_RANGE_BYTES as u64
        || planned.manifest.size != approved_notice.size()
        || planned.manifest.id != ContentId(approved_notice.root())
        || end > approved_notice.size()
    {
        return Err("requested Staya bytes differ from approved Akula object".into());
    }
    let mut request = [0u8; REQUEST_BYTES];
    request[..8].copy_from_slice(MAGIC);
    request[8..24].copy_from_slice(&approved_notice.message_id());
    request[24..32].copy_from_slice(&start.to_be_bytes());
    request[32..40].copy_from_slice(&end.to_be_bytes());
    request[40..].copy_from_slice(signed_grant);
    let mut tx = connection.open_uni().await
        .map_err(|e| format!("create range request: {e}"))?;
    tx.write_all(&request).await
        .map_err(|e| format!("write bounded grant request: {e}"))?;
    tx.finish().map_err(|e| format!("finish grant request: {e}"))?;
    let mut stream = tokio::time::timeout(
        std::time::Duration::from_secs(DEADLINE_SECS), connection.accept_uni()
    ).await.map_err(|_| "authorized file stream deadline")?
     .map_err(|e| format!("authorized file stream: {e}"))?;
    let bytes = stream.read_to_end(wanted as usize).await
        .map_err(|e| format!("read bounded authorized range: {e}"))?;
    if bytes.len() != wanted as usize {
        return Err("short or extra authorized range".into());
    }
    let mut consumed = connection.open_uni().await
        .map_err(|e| format!("range consumed stream: {e}"))?;
    consumed.write_all(RANGE_READ_ONLY).await
        .map_err(|e| format!("range consumed reply: {e}"))?;
    consumed.finish().map_err(|e| format!("finish consumed reply: {e}"))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::{
        p2p_attachment_grant_lab::{
            issue_owner_read_grant, GrantWindow, LocalReadPolicy,
        },
        p2p_offer_lab::{DecisionAction, OfferedFile, OfferV1, PeerBinding, SCHEMA},
        sync::DeviceId,
    };
    use iroh::{Endpoint, EndpointAddr, RelayMode, endpoint::presets};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::{
        fs::{self, File},
        net::{IpAddr, Ipv4Addr, SocketAddr},
        time::Duration,
    };
    use uuid::Uuid;

    struct ExplicitLabReadPolicy;
    impl LocalReadPolicy for ExplicitLabReadPolicy {
        fn permit_exact_object(
            &self, _: PeerBinding, _: PeerBinding, _: [u8; 32], _: u64
        ) -> bool { true }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_quic_five_mib_signed_three_mib_suffix_and_replay_rejection() {
        const TOTAL: usize = 5 * 1024 * 1024;
        const OFFSET: usize = 2 * 1024 * 1024;
        const NOW: u64 = 200;
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("selected-file.bin");
        let source = (0..TOTAL).map(|i| ((i * 37) ^ (i >> 3)) as u8)
            .collect::<Vec<u8>>();
        fs::write(&source_path, &source).unwrap();
        let root = *blake3::hash(&source).as_bytes();

        // Disposable QUIC keys; shipping Native requires a protected local
        // identity, OS pin and a real Core owner ACL (not this test policy).
        let sender = Endpoint::builder(presets::Minimal)
            .secret_key(iroh::SecretKey::generate())
            .alpns(vec![GRANTED_RANGE_ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind().await.unwrap();
        let sender_id=sender.id();
        let port=sender.bound_sockets().into_iter()
            .find(|socket| socket.is_ipv4()).unwrap().port();
        let direct=EndpointAddr::new(sender_id)
            .with_ip_addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST),port));
        let receiver=Endpoint::builder(presets::Minimal)
            .secret_key(iroh::SecretKey::generate())
            .alpns(vec![GRANTED_RANGE_ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind().await.unwrap();
        let receiver_id=receiver.id();
        let peer_sender=PeerBinding{
            device_id:DeviceId::from_uuid(Uuid::from_u128(17)),
            endpoint_id:*sender_id.as_bytes(),
        };
        let peer_receiver=PeerBinding{
            device_id:DeviceId::from_uuid(Uuid::from_u128(23)),
            endpoint_id:*receiver_id.as_bytes(),
        };
        let offer=OfferV1 {
            schema:SCHEMA,offer_id:Uuid::from_u128(42),nonce:[19;16],
            sender:peer_sender,receiver:peer_receiver,
            created_unix:100,expires_unix:350,
            files:vec![OfferedFile{
                label:"photo.bin".into(),bytes:TOTAL as u64,
                blake3_root:root,
            }],
        };
        let decision=offer.receiver_decision(
            DecisionAction::Accept,vec![0],110,peer_sender,peer_receiver
        ).unwrap();
        let notice=AttachmentNoticeV1::from_accepted(
            &offer,&decision,peer_sender,peer_receiver,0,120
        ).unwrap();
        let snapshot=SelectedSourceSnapshotLab::from_core_selected_file(
            File::open(&source_path).unwrap(), notice
        ).unwrap();
        let owner_key=Ed25519KeyPair::from_seed_unchecked(&[41;32]).unwrap();
        let owner_pub:[u8;32]=owner_key.public_key().as_ref().try_into().unwrap();
        let grant=issue_owner_read_grant(
            &notice,&owner_key,&ExplicitLabReadPolicy,
            GrantWindow{
                start:OFFSET as u64,end:TOTAL as u64,issued_unix:190,
                expires_unix:300,owner_key_epoch:7,nonce:[33;16],
            },NOW,
        ).unwrap();
        let (_,planned) = crate::attachment_contract::plan_resume(
            &notice, OFFSET as u64, *blake3::hash(&source[..OFFSET]).as_bytes()
        ).unwrap();
        let mut spent=GrantSpendLedgerLab::open(
            &dir.path().join("spent-grants.sqlite")
        ).unwrap();

        // A repeated request over a *new authenticated* QUIC session cannot
        // use the original grant. Only one is allowed to access source bytes.
        let sender_job = async {
            for attempt in 0..2 {
                let incoming=tokio::time::timeout(
                    Duration::from_secs(35), sender.accept()
                ).await.unwrap().unwrap();
                let conn=incoming.accept().unwrap().await.unwrap();
                assert_eq!(conn.remote_id(),receiver_id);
                let reply=serve_granted_range_once(
                    &conn,&snapshot,&owner_pub,7,NOW,&mut spent,
                    |connection| if connection.remote_id()==receiver_id {
                        Ok(())
                    } else { Err("not a pinned test peer".into()) }
                ).await;
                if attempt==0 { reply.unwrap(); }
                else {
                    assert!(reply.is_err(), "already-spent grant cannot stream again");
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
                conn.close(0u8.into(),b"test only grant range closed");
            }
            sender.close().await;
            assert_eq!(spent.count_for_lab(),Ok(1));
        };
        let receiver_job = async {
            let connection=tokio::time::timeout(
                Duration::from_secs(35),receiver.connect(direct.clone(),GRANTED_RANGE_ALPN)
            ).await.unwrap().unwrap();
            assert_eq!(connection.remote_id(),sender_id);
            let suffix=tokio::time::timeout(
                Duration::from_secs(35),
                request_granted_range_once(
                    &connection,&notice,&planned,&grant,*sender_id.as_bytes()
                )
            ).await.unwrap().unwrap();
            assert_eq!(suffix.len(),TOTAL-OFFSET);
            let mut joined=source[..OFFSET].to_vec();
            joined.extend_from_slice(&suffix);
            assert_eq!(joined.len(),TOTAL);
            assert_eq!(*blake3::hash(&joined).as_bytes(),root);
            tokio::time::sleep(Duration::from_millis(350)).await;
            connection.close(0u8.into(),b"range bytes checked not W4");

            let second=tokio::time::timeout(
                Duration::from_secs(35),receiver.connect(direct,GRANTED_RANGE_ALPN)
            ).await.unwrap().unwrap();
            // Fresh QUIC session with the *same* grant: server must reject it.
            let repeated=tokio::time::timeout(
                Duration::from_secs(35),
                request_granted_range_once(
                    &second,&notice,&planned,&grant,*sender_id.as_bytes()
                )
            ).await;
            assert!(matches!(repeated,Ok(Err(_))|Err(_)),
                "no second exposure of a spent grant");
            second.close(1u8.into(),b"replay rejected");
            receiver.close().await;
        };
        tokio::time::timeout(
            Duration::from_secs(115),async {tokio::join!(sender_job,receiver_job);}
        ).await.unwrap();
    }
}
