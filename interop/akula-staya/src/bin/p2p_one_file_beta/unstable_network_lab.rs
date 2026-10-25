//! Stress lab: real authenticated QUIC streams across repeated connections,
//! varying application pacing (not actual radio impairment or Wi-Fi/LTE).
//! Only complete, hash-verified 64-KiB windows are checkpointed. A deliberately
//! short final window is discarded before the next connection. No W4 signed ACK.
use super::*;
use super::resume_checkpoint_lab::{Checkpoint, Identity};
use std::{fs::{self, OpenOptions},io::Write,net::{IpAddr,Ipv4Addr}};

const BLOCK: usize = 64 * 1024;
const TOTAL: usize = 5 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"AKSTRES1";
const VERIFY: &[u8] = b"CHECKPOINT_NOT_DELIVERED";
const COMPLETE: &[u8] = b"LOCAL_INBOX_VERIFIED_NOT_W4";

fn payload() -> Vec<u8> {
    let mut state = 0x6c21_7423_55ae_aa91u64;
    (0..TOTAL).map(|_| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 24) as u8
    }).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unstable_five_mib_multiple_drops_throttling_and_route_profile_switch() {
    // Five separate real QUIC handshakes, using one cryptographic identity.
    // Profile changes model the app pacing at the boundary, NOT a physical
    // network interface switch. The sender always revalidates a cold prefix.
    let dir = tempfile::tempdir().unwrap();
    let private_inbox = dir.path().join("received-inbox");
    fs::create_dir(&private_inbox).unwrap();
    let stage = dir.path().join("uncommitted-photo.stage");
    let all = payload();
    let expected_hash = *blake3::hash(&all).as_bytes();
    let sender = Endpoint::builder(presets::Minimal)
        .secret_key(iroh::SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind().await.unwrap();
    let sender_id = sender.id();
    let socket = sender.bound_sockets().into_iter().find(|x| x.is_ipv4()).unwrap();
    let direct = EndpointAddr::new(sender_id)
        .with_ip_addr(std::net::SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), socket.port()));
    let receiver = Endpoint::builder(presets::Minimal)
        .secret_key(iroh::SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind().await.unwrap();
    let receiver_id = receiver.id();
    let identity = Identity {
        sender: *sender_id.as_bytes(),
        receiver: *receiver_id.as_bytes(),
        full_hash: expected_hash,
        total: TOTAL as u64,
    };
    // Various amounts of useful progress and varying artificially throttled
    // sender pacing (0/1/4ms per 64KiB). Five sessions; first four deliberately
    // end with an incomplete 64KiB block, which is never checkpointed.
    let windows = [13usize, 9, 21, 15, 22];
    assert_eq!(windows.iter().sum::<usize>(), TOTAL / BLOCK);
    let pacing_ms = [0u64, 4, 1, 4, 0];
    let sender_bytes = all.clone();
    let sender_task = tokio::spawn(async move {
        let mut offset = 0usize;
        for attempt in 0..windows.len() {
            let incoming = tokio::time::timeout(Duration::from_secs(45), sender.accept())
                .await.unwrap().unwrap();
            let connection = incoming.accept().unwrap().await.unwrap();
            assert_eq!(connection.remote_id(), receiver_id);
            let mut claim = connection.accept_uni().await.unwrap();
            let raw = claim.read_to_end(200).await.unwrap();
            assert_eq!(&raw[..8], MAGIC);
            let claimed_offset = u64::from_be_bytes(raw[8..16].try_into().unwrap()) as usize;
            let claimed_hash: [u8; 32] = raw[16..48].try_into().unwrap();
            assert_eq!(claimed_offset, offset);
            assert_eq!(claimed_hash, *blake3::hash(&sender_bytes[..offset]).as_bytes());
            // Different logical route profiles use fresh QUIC connection and
            // pacing; no actual Wi-Fi/LTE channel switch is claimed.
            let expected = if attempt < 4 {
                windows[attempt] * BLOCK + BLOCK / 2
            } else {
                windows[attempt] * BLOCK
            };
            let mut tx = connection.open_uni().await.unwrap();
            for part in 0..windows[attempt] {
                let start = offset + part * BLOCK;
                tx.write_all(&sender_bytes[start..start + BLOCK]).await.unwrap();
                if pacing_ms[attempt] != 0 {
                    tokio::time::sleep(Duration::from_millis(pacing_ms[attempt])).await;
                }
            }
            if attempt < 4 {
                // Incomplete tail is received but must be discarded, not
                // acknowledged, and transferred again on next connection.
                tx.write_all(&sender_bytes[offset + windows[attempt] * BLOCK..
                    offset + expected]).await.unwrap();
            }
            tx.finish().unwrap();
            let mut ack = connection.accept_uni().await.unwrap();
            let response = ack.read_to_end(64).await.unwrap();
            assert_eq!(response, if attempt == 4 { COMPLETE } else { VERIFY });
            offset += windows[attempt] * BLOCK;
            // A completed send stream alone does not mean the receiver has
            // consumed the last bytes. Under real netem packet loss, closing
            // after a fixed 150ms destroyed ACK_READ before delivery.
            // Require a complete, bounded application-level receipt chain
            // before this *intentional* connection replacement.
            let mut consumed = connection.open_uni().await.unwrap();
            consumed.write_all(b"ACK_READ").await.unwrap();
            consumed.finish().unwrap();
            let mut observed = tokio::time::timeout(
                Duration::from_secs(20), connection.accept_uni()
            ).await.expect("receiver receipt timed out").unwrap();
            assert_eq!(
                observed.read_to_end(64).await.unwrap(),
                b"ACK_READ_RECEIVED",
            );
            let mut release = connection.open_uni().await.unwrap();
            release.write_all(b"RELEASE_ROUTE").await.unwrap();
            release.finish().unwrap();
            tokio::time::sleep(Duration::from_millis(800)).await;
            connection.close(0u8.into(), b"stress lab reconnect");
        }
        assert_eq!(offset, TOTAL);
        sender.close().await;
    });

    let mut confirmed = 0usize;
    for attempt in 0..windows.len() {
        // The *same* authenticated receiver key initiates another real session.
        // Profiles here are different app pacing/connection generations.
        let connection = tokio::time::timeout(
            Duration::from_secs(45), receiver.connect(direct.clone(), ALPN)
        ).await.unwrap().unwrap();
        assert_eq!(connection.remote_id(), sender_id);
        let mut claim = connection.open_uni().await.unwrap();
        let mut request = Vec::with_capacity(48);
        request.extend_from_slice(MAGIC);
        request.extend_from_slice(&(confirmed as u64).to_be_bytes());
        let cold = if confirmed > 0 { fs::read(&stage).unwrap() } else { Vec::new() };
        assert_eq!(cold.len(), confirmed);
        request.extend_from_slice(blake3::hash(&cold).as_bytes());
        claim.write_all(&request).await.unwrap();
        claim.finish().unwrap();

        let expected = windows[attempt] * BLOCK + if attempt < 4 { BLOCK / 2 } else { 0 };
        let mut stream = connection.accept_uni().await.unwrap();
        let window = stream.read_to_end(expected).await.unwrap();
        assert_eq!(window.len(), expected);
        assert_eq!(&window[..windows[attempt] * BLOCK],
            &all[confirmed..confirmed + windows[attempt] * BLOCK]);
        let valid = &window[..windows[attempt] * BLOCK];
        let mut file = OpenOptions::new().create(true).append(true).open(&stage).unwrap();
        file.write_all(valid).unwrap();
        file.sync_all().unwrap();
        drop(file);
        confirmed += valid.len();
        // Cold read and exact BLAKE3 prefix verification at each connection
        // boundary. Uncommitted half-blocks are never written or acknowledged.
        let cold = fs::read(&stage).unwrap();
        assert_eq!(cold.len(), confirmed);
        assert_eq!(*blake3::hash(&cold).as_bytes(),
            *blake3::hash(&all[..confirmed]).as_bytes());
        let cp = Checkpoint {
            identity,
            offset: confirmed as u64,
            prefix_hash: *blake3::hash(&cold).as_bytes(),
        };
        assert_eq!(Checkpoint::decode(&cp.wire()).unwrap(), cp);
        assert_eq!(fs::read_dir(&private_inbox).unwrap().count(), 0);
        if attempt == windows.len()-1 {
            assert_eq!(confirmed, TOTAL);
            assert_eq!(*blake3::hash(&cold).as_bytes(), expected_hash);
            let receipt = inbox::persist_received_payload(
                &cold, &private_inbox, TOTAL as u64, expected_hash,
                *sender_id.as_bytes(), *receiver_id.as_bytes()
            ).unwrap();
            let ledger = akula_inbox_ledger_lab::InboxLedger::open(&private_inbox).unwrap();
            assert_eq!(ledger.count().unwrap(), 1);
            assert_eq!(fs::read(ledger.object_path(receipt.id)).unwrap(), all);
        }
        let mut ack = connection.open_uni().await.unwrap();
        ack.write_all(if attempt == windows.len()-1 { COMPLETE } else { VERIFY })
            .await.unwrap();
        ack.finish().unwrap();
        let mut consumed = tokio::time::timeout(
            Duration::from_secs(20), connection.accept_uni()
        ).await.expect("sender receipt timed out").unwrap();
        assert_eq!(consumed.read_to_end(64).await.unwrap(), b"ACK_READ");
        let mut observed = connection.open_uni().await.unwrap();
        observed.write_all(b"ACK_READ_RECEIVED").await.unwrap();
        observed.finish().unwrap();
        let mut release = tokio::time::timeout(
            Duration::from_secs(20), connection.accept_uni()
        ).await.expect("route release timed out").unwrap();
        assert_eq!(release.read_to_end(64).await.unwrap(), b"RELEASE_ROUTE");
        // QUIC stream bytes are complete; the peer may now deliberately
        // replace this route without making any false file-delivery claim.
        tokio::time::sleep(Duration::from_millis(800)).await;
        connection.close(1u8.into(), b"test route changed");
    }
    receiver.close().await;
    tokio::time::timeout(Duration::from_secs(80), sender_task).await.unwrap().unwrap();
    assert_eq!(confirmed, TOTAL);
}
