//! Experimental, non-shipping resumable photo transfer over two authenticated
//! Iroh/QUIC connections. This proves a byte-offset and prefix-hash checkpoint
//! without changing the existing one-file beta wire protocol or its ACK rules.
//! The private test stage is NOT a production Windows pinned stage, the local
//! completion marker is NOT a signed WOW W4 receipt, and this is NOT a user UI.
use super::*;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr};

const PHOTO_BYTES: usize = 5 * 1024 * 1024;
const INTERRUPTION_AT: usize = 2 * 1024 * 1024;
const CHECKPOINT_BLOCK: usize = 64 * 1024;
const RESUME_REQUEST: &[u8; 8] = b"AKRESUM1";
const STAGE_ONLY: &[u8] = b"STAGED_NO_FINAL_RECEIPT";
const STAGE_OBSERVED: &[u8] = b"STAGE_ACK_NOT_DELIVERY";
const LOCAL_VERIFIED: &[u8] = b"INBOX_COLD_VERIFIED_NOT_W4";

fn synthetic_photo_bytes() -> Vec<u8> {
    // Incompressible-looking opaque photo-sized bytes, not a real JPEG or user file.
    let mut state = 0x8143_23ed_b340_ac79u64;
    let mut result = vec![0; PHOTO_BYTES];
    for byte in &mut result {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = (state >> 24) as u8;
    }
    result
}

/// Only an exactly aligned, strictly partial, byte-verified prefix may be
/// skipped on the second connection. Both size and identity are fixed by the
/// authenticated peer and the immutable sender snapshot, not the request.
fn checked_resume_offset(
    source: &[u8],
    claimed_offset: u64,
    claimed_prefix: [u8; 32],
) -> Result<usize, &'static str> {
    let offset = usize::try_from(claimed_offset).map_err(|_| "offset overflow")?;
    if offset == 0 || offset >= source.len() || offset % CHECKPOINT_BLOCK != 0 {
        return Err("invalid or unaligned resume offset");
    }
    if blake3::hash(&source[..offset]).as_bytes() != &claimed_prefix {
        return Err("staged prefix differs from authorized source");
    }
    Ok(offset)
}

#[test]
fn resume_claim_rejects_unknown_or_corrupted_prefix_before_sending_bytes() {
    let photo = synthetic_photo_bytes();
    let valid = *blake3::hash(&photo[..INTERRUPTION_AT]).as_bytes();
    assert_eq!(
        checked_resume_offset(&photo, INTERRUPTION_AT as u64, valid),
        Ok(INTERRUPTION_AT)
    );
    assert!(checked_resume_offset(&photo, 0, valid).is_err());
    assert!(checked_resume_offset(&photo, 1, valid).is_err());
    assert!(checked_resume_offset(&photo, photo.len() as u64, valid).is_err());
    assert!(checked_resume_offset(&photo, u64::MAX, valid).is_err());
    assert!(checked_resume_offset(&photo, INTERRUPTION_AT as u64, [0; 32]).is_err());
    let mut changed = photo[..INTERRUPTION_AT].to_vec();
    changed[17] ^= 1;
    let corrupt_prefix = *blake3::hash(&changed).as_bytes();
    assert!(checked_resume_offset(&photo, INTERRUPTION_AT as u64, corrupt_prefix).is_err());
}

/// One real 5-MiB QUIC payload, interruption after 2 MiB, file checkpoint
/// fsync, connection closed, then a NEW QUIC connection delivering ONLY the
/// missing 3 MiB. The final file enters the existing Akula private Inbox only
/// after a full BLAKE3 match. All keys and files are disposable test fixtures.
///
/// NOT proven: native GUI, Windows secure stage ownership, background retries,
/// signed W3/W4, actual Internet/NAT, hardware power loss or two physical PCs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_quic_five_mib_photo_resumes_after_interruption_without_false_delivery() {
    let root = tempfile::tempdir().unwrap();
    let inbox_root = root.path().join("received-inbox");
    fs::create_dir(&inbox_root).unwrap();
    let stage_path = root.path().join("uncommitted-photo.stage");
    let source = synthetic_photo_bytes();
    let full_digest = *blake3::hash(&source).as_bytes();
    let sender_source = source.clone();

    let sender = Endpoint::builder(presets::Minimal)
        .secret_key(iroh::SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let sender_id = sender.id();
    let port = sender
        .bound_sockets()
        .into_iter()
        .find(|address| address.is_ipv4())
        .expect("real IPv4 loopback QUIC socket")
        .port();
    let direct = EndpointAddr::new(sender_id)
        .with_ip_addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));

    let receiver = Endpoint::builder(presets::Minimal)
        .secret_key(iroh::SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let receiver_id = receiver.id();

    let sender_job = tokio::spawn(async move {
        for attempt in 0..2 {
            let incoming = tokio::time::timeout(Duration::from_secs(60), sender.accept())
                .await
                .unwrap()
                .expect("real incoming QUIC peer");
            let connection = incoming.accept().unwrap().await.unwrap();
            assert_eq!(connection.remote_id(), receiver_id);
            if attempt == 0 {
                // This session deliberately ends with only a partial transfer.
                let mut tx = connection.open_uni().await.unwrap();
                tx.write_all(&sender_source[..INTERRUPTION_AT]).await.unwrap();
                tx.finish().unwrap();
                let mut checkpoint = connection.accept_uni().await.unwrap();
                assert_eq!(checkpoint.read_to_end(64).await.unwrap(), STAGE_ONLY);
                assert_ne!(STAGE_ONLY, LOCAL_VERIFIED);
                let mut observed = connection.open_uni().await.unwrap();
                observed.write_all(STAGE_OBSERVED).await.unwrap();
                observed.finish().unwrap();
                // QUIC stream finish is not proof the other endpoint
                // consumed the bytes. A bounded test-only handshake keeps
                // the sender open until the receiver observed the marker.
                let mut consumed = connection.accept_uni().await.unwrap();
                assert_eq!(
                    consumed.read_to_end(64).await.unwrap(),
                    b"STAGE_OBSERVED_BY_RECEIVER"
                );
            } else {
                let mut request = connection.accept_uni().await.unwrap();
                let claim = request.read_to_end(64).await.unwrap();
                assert_eq!(claim.len(), 48);
                assert_eq!(&claim[..8], RESUME_REQUEST);
                let offset = u64::from_be_bytes(claim[8..16].try_into().unwrap());
                let claimed_prefix: [u8; 32] = claim[16..48].try_into().unwrap();
                let offset = checked_resume_offset(&sender_source, offset, claimed_prefix)
                    .expect("sender must reject forged/corrupt checkpoint");
                assert_eq!(offset, INTERRUPTION_AT);
                let mut tx = connection.open_uni().await.unwrap();
                tx.write_all(&sender_source[offset..]).await.unwrap();
                tx.finish().unwrap();
                let mut receipt = connection.accept_uni().await.unwrap();
                let ack = receipt.read_to_end(64).await.unwrap();
                assert_eq!(ack, LOCAL_VERIFIED);
                // Under kernel rate limits an application close can discard
                // the sender's observed result unless consumption is
                // acknowledged on a distinct stream.
                let mut observed = connection.open_uni().await.unwrap();
                observed.write_all(b"VERIFIED_RECEIPT_OBSERVED").await.unwrap();
                observed.finish().unwrap();
                let mut synced = tokio::time::timeout(
                    Duration::from_secs(30), connection.accept_uni()
                ).await.expect("receipt consumption deadline").unwrap();
                assert_eq!(synced.read_to_end(64).await.unwrap(), b"RECEIPT_SYNCED");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            connection.close(0u8.into(), b"bounded resume lab session");
        }
        sender.close().await;
    });

    // First connection stores a private partial file, but no Inbox row and
    // no verified receipt exist. Power loss is NOT injected by this test.
    let first = tokio::time::timeout(Duration::from_secs(60), receiver.connect(direct.clone(), ALPN))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.remote_id(), sender_id);
    let mut stream = first.accept_uni().await.unwrap();
    let partial = stream.read_to_end(INTERRUPTION_AT).await.unwrap();
    assert_eq!(partial.len(), INTERRUPTION_AT);
    let mut stage = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stage_path)
        .unwrap();
    stage.write_all(&partial).unwrap();
    stage.sync_all().unwrap();
    drop(stage);
    let mut checkpoint = first.open_uni().await.unwrap();
    checkpoint.write_all(STAGE_ONLY).await.unwrap();
    checkpoint.finish().unwrap();
    let mut observed = first.accept_uni().await.unwrap();
    assert_eq!(observed.read_to_end(64).await.unwrap(), STAGE_OBSERVED);
    let mut consumed = first.open_uni().await.unwrap();
    consumed
        .write_all(b"STAGE_OBSERVED_BY_RECEIVER")
        .await
        .unwrap();
    consumed.finish().unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    first.close(1u8.into(), b"deliberate interruption before complete transfer");
    assert_eq!(fs::read_dir(&inbox_root).unwrap().count(), 0);

    // New session: independently reopen the physical checkpoint and ask for
    // exactly the verified missing suffix; no sender-chosen path is accepted.
    let mut reopened = File::open(&stage_path).unwrap();
    let mut actual_prefix = Vec::new();
    reopened.read_to_end(&mut actual_prefix).unwrap();
    assert_eq!(actual_prefix.len(), INTERRUPTION_AT);
    let prefix_digest = *blake3::hash(&actual_prefix).as_bytes();
    let second = tokio::time::timeout(Duration::from_secs(60), receiver.connect(direct, ALPN))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.remote_id(), sender_id);
    let mut claim = Vec::with_capacity(48);
    claim.extend_from_slice(RESUME_REQUEST);
    claim.extend_from_slice(&(actual_prefix.len() as u64).to_be_bytes());
    claim.extend_from_slice(&prefix_digest);
    let mut request = second.open_uni().await.unwrap();
    request.write_all(&claim).await.unwrap();
    request.finish().unwrap();
    let mut stream = second.accept_uni().await.unwrap();
    let suffix = stream.read_to_end(PHOTO_BYTES - INTERRUPTION_AT).await.unwrap();
    assert_eq!(suffix.len(), PHOTO_BYTES - INTERRUPTION_AT);
    let mut stage = OpenOptions::new().append(true).open(&stage_path).unwrap();
    stage.write_all(&suffix).unwrap();
    stage.sync_all().unwrap();
    drop(stage);

    // Only a complete and cold-readable 5 MiB BLAKE3 match may advance the
    // existing Inbox ledger. A checkpoint is never published as a full file.
    let complete = fs::read(&stage_path).unwrap();
    assert_eq!(complete.len(), PHOTO_BYTES);
    assert_eq!(*blake3::hash(&complete).as_bytes(), full_digest);
    let receipt = inbox::persist_received_payload(
        &complete,
        &inbox_root,
        PHOTO_BYTES as u64,
        full_digest,
        *sender_id.as_bytes(),
        *receiver_id.as_bytes(),
    )
    .unwrap();
    let cold = akula_inbox_ledger_lab::InboxLedger::open(&inbox_root).unwrap();
    assert_eq!(cold.count().unwrap(), 1);
    assert_eq!(
        cold.status(receipt.id).unwrap(),
        akula_inbox_ledger_lab::LocalStatus::VerifiedLocal
    );
    assert_eq!(fs::read(cold.object_path(receipt.id)).unwrap(), complete);
    let mut ack = second.open_uni().await.unwrap();
    ack.write_all(LOCAL_VERIFIED).await.unwrap();
    ack.finish().unwrap();
    let mut observed = tokio::time::timeout(
        Duration::from_secs(30), second.accept_uni()
    ).await.expect("sender receipt observation deadline").unwrap();
    assert_eq!(observed.read_to_end(64).await.unwrap(), b"VERIFIED_RECEIPT_OBSERVED");
    let mut synced = second.open_uni().await.unwrap();
    synced.write_all(b"RECEIPT_SYNCED").await.unwrap();
    synced.finish().unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    second.close(0u8.into(), b"local private Inbox receipt observed");
    receiver.close().await;
    tokio::time::timeout(Duration::from_secs(60), sender_job)
        .await
        .unwrap()
        .unwrap();
    // 2 MiB first attempt + 3 MiB second attempt. Retransmission of the
    // entire 5-MiB payload was not needed in this isolated QUIC experiment.
    assert_eq!(partial.len() + suffix.len(), PHOTO_BYTES);
}
