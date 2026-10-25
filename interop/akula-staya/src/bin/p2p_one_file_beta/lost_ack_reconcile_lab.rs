//! Lab: the sender loses BOTH the partial-stage notice and the final Inbox
//! receipt. Each QUIC connection is independently authenticated. On the next
//! connection a cold checkpoint supplies the suffix offset, or an independently
//! verified local Inbox object supplies a content-bound completion proof.
//!
//! This is not a shipping protocol, signed WOW W4, permanent remote receipt,
//! user-facing UI, or an automatic retry daemon. In particular, a proof that
//! a file exists in a local Inbox is not File Gate APPLIED.
use super::*;
use super::resume_checkpoint_lab::{self, Checkpoint, Identity};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
};

const FILE_BYTES: usize = 5 * 1024 * 1024;
const STAGED_BYTES: usize = 2 * 1024 * 1024;
const RESUME: &[u8; 8] = b"AKLOST01";
const PROBE: &[u8; 8] = b"AKPROBE1";
const COMPLETE: &[u8; 8] = b"AKLOCAL1";
const RECEIPT_BYTES: usize = 8 + 8 + 32;
const CHECKPOINT_BYTES: usize = 8 + 8 + 32;

fn photo() -> Vec<u8> {
    (0..FILE_BYTES).map(|i| ((i * 31) ^ (i >> 6)) as u8).collect()
}

fn wire(label: &[u8; 8], size: u64, hash: [u8; 32]) -> [u8; RECEIPT_BYTES] {
    let mut bytes = [0u8; RECEIPT_BYTES];
    bytes[..8].copy_from_slice(label);
    bytes[8..16].copy_from_slice(&size.to_be_bytes());
    bytes[16..48].copy_from_slice(&hash);
    bytes
}

/// Read-only reconciliation: no fresh Inbox row may be created or promoted
/// by asking for the status. Check every immutable bound field and rehash
/// the physical file before describing it as received locally.
fn cold_verified_receipt(
    inbox_root: &Path,
    sender: [u8; 32],
    receiver: [u8; 32],
    len: u64,
    full_hash: [u8; 32],
) -> Result<[u8; RECEIPT_BYTES], String> {
    let ledger = akula_inbox_ledger_lab::InboxLedger::open_existing(inbox_root)
        .map_err(|e| format!("open existing ledger: {e:?}"))?
        .ok_or("no committed Inbox ledger")?;
    let checked = ledger
        .list_recent(64)
        .map_err(|e| format!("cold receipt scan: {e:?}"))?;
    let (record, status) = checked.into_iter().find(|(record, _)| {
        record.sender_key == sender
            && record.recipient_key == receiver
            && record.bytes == len
            && record.blake3 == full_hash
            && record.source == akula_inbox_ledger_lab::Transport::P2p
    }).ok_or("no matching sender-bound and content-bound local receipt")?;
    if status != akula_inbox_ledger_lab::LocalStatus::VerifiedLocal {
        return Err("Inbox record exists but physical bytes are not verified".into());
    }
    ledger
        .verify_committed_intent(&record)
        .map_err(|e| format!("uncommitted or changed Inbox intent: {e:?}"))?;
    let (_handle, opened) = ledger
        .open_verified_file(record.id)
        .map_err(|e| format!("cold full-byte verification: {e:?}"))?;
    if opened != record {
        return Err("Inbox object changed during cold reconciliation".into());
    }
    Ok(wire(COMPLETE, len, full_hash))
}

#[test]
fn missing_partial_or_corrupt_final_file_never_yields_verified_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = dir.path().join("received");
    fs::create_dir(&inbox).unwrap();
    let payload = photo();
    let hash = *blake3::hash(&payload).as_bytes();
    let sender = [7; 32];
    let receiver = [8; 32];

    assert!(cold_verified_receipt(&inbox, sender, receiver, FILE_BYTES as u64, hash).is_err());
    let existing = inbox::persist_received_payload(
        &payload, &inbox, FILE_BYTES as u64, hash, sender, receiver
    ).unwrap();
    let proof = cold_verified_receipt(
        &inbox, sender, receiver, FILE_BYTES as u64, hash
    ).unwrap();
    assert_eq!(proof, wire(COMPLETE, FILE_BYTES as u64, hash));
    assert!(cold_verified_receipt(&inbox, [9; 32], receiver, FILE_BYTES as u64, hash).is_err());
    assert!(cold_verified_receipt(&inbox, sender, [9; 32], FILE_BYTES as u64, hash).is_err());
    assert!(cold_verified_receipt(&inbox, sender, receiver, FILE_BYTES as u64 - 1, hash).is_err());
    assert!(cold_verified_receipt(&inbox, sender, receiver, FILE_BYTES as u64, [9; 32]).is_err());

    let ledger = akula_inbox_ledger_lab::InboxLedger::open(&inbox).unwrap();
    let mut physical = OpenOptions::new()
        .write(true)
        .open(ledger.object_path(existing.id)).unwrap();
    physical.write_all(b"corrupted").unwrap();
    physical.sync_all().unwrap();
    drop(physical);
    drop(ledger);
    assert!(
        cold_verified_receipt(&inbox, sender, receiver, FILE_BYTES as u64, hash).is_err(),
        "no positive receipt may survive corruption of stored physical bytes",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_lost_receipts_reconnect_without_duplicate_or_full_retransmit() {
    let temp = tempfile::tempdir().unwrap();
    let inbox_root = temp.path().join("received");
    fs::create_dir(&inbox_root).unwrap();
    let stage_path = temp.path().join("partial.stage");
    let sidecar_path = temp.path().join("partial.checkpoint");
    let bytes = photo();
    let hash = *blake3::hash(&bytes).as_bytes();

    let sender = Endpoint::builder(presets::Minimal)
        .secret_key(iroh::SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind().await.unwrap();
    let sender_id = sender.id();
    let port = sender.bound_sockets().into_iter()
        .find(|addr| addr.is_ipv4()).expect("IPv4 peer").port();
    let direct = EndpointAddr::new(sender_id)
        .with_ip_addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));

    let receiver = Endpoint::builder(presets::Minimal)
        .secret_key(iroh::SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind().await.unwrap();
    let receiver_id = receiver.id();
    let identity = Identity {
        sender: *sender_id.as_bytes(),
        receiver: *receiver_id.as_bytes(),
        full_hash: hash,
        total: FILE_BYTES as u64,
    };
    let outgoing = bytes.clone();
    let sender_job = tokio::spawn(async move {
        let mut transmitted = 0usize;
        for attempt in 0..3 {
            let incoming = tokio::time::timeout(Duration::from_secs(40), sender.accept())
                .await.expect("reconnect waiting deadline").unwrap();
            let conn = incoming.accept().unwrap().await.unwrap();
            assert_eq!(conn.remote_id(), receiver_id);

            match attempt {
                0 => {
                    let mut stream = conn.open_uni().await.unwrap();
                    stream.write_all(&outgoing[..STAGED_BYTES]).await.unwrap();
                    stream.finish().unwrap();
                    transmitted += STAGED_BYTES;
                }
                1 => {
                    let mut stream = conn.accept_uni().await.unwrap();
                    let request = stream.read_to_end(CHECKPOINT_BYTES).await.unwrap();
                    assert_eq!(request.len(), CHECKPOINT_BYTES);
                    assert_eq!(&request[..8], RESUME);
                    assert_eq!(
                        u64::from_be_bytes(request[8..16].try_into().unwrap()),
                        STAGED_BYTES as u64
                    );
                    assert_eq!(
                        &request[16..48],
                        blake3::hash(&outgoing[..STAGED_BYTES]).as_bytes()
                    );
                    let mut transfer = conn.open_uni().await.unwrap();
                    transfer.write_all(&outgoing[STAGED_BYTES..]).await.unwrap();
                    transfer.finish().unwrap();
                    transmitted += FILE_BYTES - STAGED_BYTES;
                }
                _ => {
                    // The sender does not retransmit the 5 MiB because it
                    // is uncertain about the previous ACK. It queries status
                    // on a fresh authenticated QUIC connection instead.
                    let mut request = conn.accept_uni().await.unwrap();
                    let query = request.read_to_end(RECEIPT_BYTES).await.unwrap();
                    assert_eq!(query, wire(PROBE, FILE_BYTES as u64, hash));
                    let mut confirmation = conn.accept_uni().await.unwrap();
                    let proof = confirmation.read_to_end(RECEIPT_BYTES).await.unwrap();
                    assert_eq!(proof, wire(COMPLETE, FILE_BYTES as u64, hash));
                }
            }

            if attempt < 2 {
                // Receiver deliberately discards the receipt path and
                // CLOSES. Absence of ACK must be inconclusive, not success.
                let no_receipt = tokio::time::timeout(
                    Duration::from_secs(20), conn.accept_uni()
                ).await.expect("lost receipt must close QUIC");
                assert!(no_receipt.is_err(), "no positive ACK was sent");
            }
            conn.close(0u8.into(), b"old session not required for new status");
        }
        sender.close().await;
        transmitted
    });

    // Session #1: receive 2 MiB, cold-durable checkpoint, discard its
    // receipt. No completed file has entered the Inbox.
    let first = tokio::time::timeout(Duration::from_secs(40),
        receiver.connect(direct.clone(), ALPN)).await.unwrap().unwrap();
    assert_eq!(first.remote_id(), sender_id);
    let mut stream = first.accept_uni().await.unwrap();
    let part = stream.read_to_end(STAGED_BYTES).await.unwrap();
    assert_eq!(part.len(), STAGED_BYTES);
    let mut file = OpenOptions::new().create_new(true).write(true)
        .open(&stage_path).unwrap();
    file.write_all(&part).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let checkpoint = Checkpoint {
        identity,
        offset: STAGED_BYTES as u64,
        prefix_hash: *blake3::hash(&part).as_bytes(),
    };
    let mut sidecar = OpenOptions::new().create_new(true).write(true)
        .open(&sidecar_path).unwrap();
    sidecar.write_all(&checkpoint.wire()).unwrap();
    sidecar.sync_all().unwrap();
    drop(sidecar);
    assert_eq!(fs::read_dir(&inbox_root).unwrap().count(), 0);
    first.close(1u8.into(), b"inject lost checkpoint ACK");
    drop(first);

    // Session #2: reopen stage and verify BLAKE3 against the saved, peer-
    // bound sidecar. Transfer only the remaining 3 MiB.
    let cold = resume_checkpoint_lab::recover(&stage_path, &sidecar_path, identity)
        .expect("valid cold checkpoint; previous QUIC close irrelevant");
    assert_eq!(cold.offset, STAGED_BYTES as u64);
    let second = tokio::time::timeout(Duration::from_secs(40),
        receiver.connect(direct.clone(), ALPN)).await.unwrap().unwrap();
    assert_eq!(second.remote_id(), sender_id);
    let mut request = second.open_uni().await.unwrap();
    request.write_all(&wire(RESUME, cold.offset, cold.prefix_hash)).await.unwrap();
    request.finish().unwrap();
    let mut stream = second.accept_uni().await.unwrap();
    let tail = stream.read_to_end(FILE_BYTES - STAGED_BYTES).await.unwrap();
    assert_eq!(tail.len(), FILE_BYTES - STAGED_BYTES);
    let mut stage = OpenOptions::new().append(true).open(&stage_path).unwrap();
    stage.write_all(&tail).unwrap();
    stage.sync_all().unwrap();
    drop(stage);
    let complete = fs::read(&stage_path).unwrap();
    assert_eq!(complete.len(), FILE_BYTES);
    assert_eq!(*blake3::hash(&complete).as_bytes(), hash);
    let saved = inbox::persist_received_payload(
        &complete, &inbox_root, FILE_BYTES as u64, hash,
        *sender_id.as_bytes(), *receiver_id.as_bytes()
    ).unwrap();
    let cold_inbox = akula_inbox_ledger_lab::InboxLedger::open(&inbox_root).unwrap();
    assert_eq!(cold_inbox.count().unwrap(), 1);
    drop(cold_inbox);
    // Another ACK is lost AFTER physical bytes were verified and committed.
    second.close(1u8.into(), b"inject lost final local receipt");
    drop(second);

    // Session #3: no data stream. A fresh authenticated peer asks for
    // status; receiver independently opens its durable ledger and rehashes
    // exactly the peer/size/root-correlated physical blob.
    let third = tokio::time::timeout(Duration::from_secs(40),
        receiver.connect(direct, ALPN)).await.unwrap().unwrap();
    assert_eq!(third.remote_id(), sender_id);
    let mut query = third.open_uni().await.unwrap();
    query.write_all(&wire(PROBE, FILE_BYTES as u64, hash)).await.unwrap();
    query.finish().unwrap();
    let proof = cold_verified_receipt(
        &inbox_root, *sender_id.as_bytes(), *receiver_id.as_bytes(),
        FILE_BYTES as u64, hash
    ).unwrap();
    let mut confirmation = third.open_uni().await.unwrap();
    confirmation.write_all(&proof).await.unwrap();
    confirmation.finish().unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    third.close(0u8.into(), b"local verified status safely re-queried");
    receiver.close().await;

    let transmitted = tokio::time::timeout(Duration::from_secs(40), sender_job)
        .await.unwrap().unwrap();
    assert_eq!(transmitted, FILE_BYTES, "no 5MiB retransmit after lost ACK");
    let ledger = akula_inbox_ledger_lab::InboxLedger::open(&inbox_root).unwrap();
    assert_eq!(ledger.count().unwrap(), 1, "lost receipts must not duplicate Inbox");
    assert_eq!(
        ledger.status(saved.id).unwrap(),
        akula_inbox_ledger_lab::LocalStatus::VerifiedLocal
    );
    let (mut opened, _record) = ledger.open_verified_file(saved.id).unwrap();
    let mut final_bytes = Vec::new();
    opened.read_to_end(&mut final_bytes).unwrap();
    assert_eq!(final_bytes, bytes);
}
