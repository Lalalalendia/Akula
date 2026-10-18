//! Research-only: pinned Roha/noq + iroh-blobs 0.103.1 against Akula Staya framing.
//! Never import remote data into the user's filesystem or grant File Gate rights.
//! Runs against independent loopback QUIC endpoints. NOT a physical WAN test.
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use akula_roha_transport::{
    transfer::{FRAME_HEADER_BYTES, MAX_CHUNK_BYTES, MAX_OBJECT_BYTES, TransferManifest, VerifiedAssembly},
    AllowedPeers, StayaTransport,
};
use iroh::{endpoint::presets, protocol::Router, Endpoint, EndpointId, RelayMode, SecretKey};
use iroh_blobs::{
    api::blobs::BlobStatus,
    get::request::get_blob,
    protocol::{ChunkRangesSeq, GetManyRequest, ObserveRequest, PushRequest},
    provider::events::{
        AbortReason, ConnectMode, EventMask, EventSender, ObserveMode, ProviderMessage,
        RequestMode, ThrottleMode,
    },
    store::mem::MemStore,
    BlobsProtocol, Hash,
};
use futures_util::StreamExt;

const TEST_DEADLINE: Duration = Duration::from_secs(45);
const NET_STEP_DEADLINE: Duration = Duration::from_secs(8);

/// Intentionally do not use EventMask::DEFAULT or ALL_READONLY.
/// ObserveMode has no Disabled variant at v0.103.1: intercept and deny it.
fn read_only_mask() -> EventMask {
    EventMask {
        connected: ConnectMode::Intercept,
        get: RequestMode::Intercept,
        get_many: RequestMode::Disabled,
        push: RequestMode::Disabled,
        observe: ObserveMode::Intercept,
        throttle: ThrottleMode::None,
    }
}

#[test]
fn explicit_cross_kind_policy_never_leaves_push_or_get_many_open() {
    let mask = read_only_mask();
    assert_eq!(mask.connected, ConnectMode::Intercept);
    assert_eq!(mask.get, RequestMode::Intercept);
    assert_eq!(mask.get_many, RequestMode::Disabled);
    assert_eq!(mask.push, RequestMode::Disabled);
    assert_eq!(mask.observe, ObserveMode::Intercept);
}

/// A QUIC key is a transport identity, not a VaultId or a signed Akula grant.
/// This artificial allowlist is only a negative security regression.
fn restricted_events(
    allowed_peer: EndpointId,
    allowed_hashes: HashSet<Hash>,
) -> (EventSender, tokio::task::JoinHandle<()>) {
    let (events, mut rx) = EventSender::channel(64, read_only_mask());
    let task = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                ProviderMessage::ClientConnected(msg) => {
                    let decision = if msg.endpoint_id == Some(allowed_peer) {
                        Ok(())
                    } else {
                        Err(AbortReason::Permission)
                    };
                    let _ = msg.tx.send(decision).await;
                }
                ProviderMessage::GetRequestReceived(msg) => {
                    // Explicitly prohibit hash-sequence requests (and thus indirect
                    // access to children) even when the root hash matches.
                    let decision = if msg.request.ranges.is_blob()
                        && allowed_hashes.contains(&msg.request.hash)
                    {
                        Ok(())
                    } else {
                        Err(AbortReason::Permission)
                    };
                    let _ = msg.tx.send(decision).await;
                }
                ProviderMessage::ObserveRequestReceived(msg) => {
                    // v0.103.1 ObserveMode cannot be Disabled, so reject each
                    // request. Never create a partial placeholder in the store.
                    let _ = msg.tx.send(Err(AbortReason::Permission)).await;
                }
                // Push and GetMany are Disabled before event dispatch.
                // All other events carry no admission power in this experiment.
                _ => {}
            }
        }
    });
    (events, task)
}

async fn local_endpoint(key: SecretKey) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .expect("private isolated endpoint")
}

fn bytes(size: usize) -> Vec<u8> {
    assert!(size <= MAX_OBJECT_BYTES);
    let mut buf = vec![0; size];
    let mut h = blake3::Hasher::new();
    h.update(b"akula-iroh-blobs-compare-isolated-public-test");
    h.update(&(size as u64).to_le_bytes());
    h.finalize_xof().fill(&mut buf);
    buf
}

#[tokio::test(flavor = "current_thread")]
async fn a_get_can_be_allowed_without_enabling_push_or_other_reads() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let server_key = SecretKey::generate();
        let allowed_key = SecretKey::generate();
        let unknown_key = SecretKey::generate();
        let store = MemStore::new();
        let allowed_data = bytes(64 * 1024 + 17);
        let allowed_tag = store.add_slice(&allowed_data).await.expect("seed allowed");
        let secret_tag = store.add_slice(b"other secret blob").await.expect("seed denied");
        let (events, worker) = restricted_events(
            allowed_key.public(), HashSet::from([allowed_tag.hash]),
        );
        let server = local_endpoint(server_key).await;
        let router = Router::builder(server)
            .accept(iroh_blobs::ALPN, BlobsProtocol::new(&store, Some(events)))
            .spawn();
        let allowed = local_endpoint(allowed_key).await;
        let remote = router.endpoint().addr();
        let connection = allowed
            .connect(remote.clone(), iroh_blobs::ALPN)
            .await
            .expect("transport connect");

        let received = tokio::time::timeout(
            NET_STEP_DEADLINE, get_blob(connection.clone(), allowed_tag.hash).bytes(),
        ).await.expect("allowed GET bounded").expect("allowed GET succeeds");
        assert_eq!(received.as_ref(), allowed_data.as_slice());

        let forbidden = tokio::time::timeout(
            NET_STEP_DEADLINE, get_blob(connection.clone(), secret_tag.hash).bytes(),
        ).await.expect("hash refusal timely");
        assert!(forbidden.is_err(), "unlisted blob was readable");

        let local = MemStore::new();
        let malicious = local.add_slice(b"unsolicited remote push").await.expect("attacker local blob");
        let get_many = tokio::time::timeout(NET_STEP_DEADLINE, local.remote().execute_get_many(
            connection.clone(),
            GetManyRequest::new(vec![allowed_tag.hash], ChunkRangesSeq::all()),
        ).complete()).await.expect("get-many must settle");
        assert!(get_many.is_err(), "get-many bypassed single-hash authorization");

        let observe = local.remote().observe(connection.clone(), ObserveRequest::new(allowed_tag.hash));
        futures_util::pin_mut!(observe);
        let observation = tokio::time::timeout(
            NET_STEP_DEADLINE, observe.next(),
        ).await.expect("observe must settle");
        assert!(!matches!(observation, Some(Ok(_))), "observe leaked state");

        // Upstream notes a denied PUSH may appear successful to the sender:
        // receiving-store non-mutation, not client status, is the ground truth.
        let _ = tokio::time::timeout(NET_STEP_DEADLINE, local.remote().execute_push(
            connection.clone(), PushRequest::new(malicious.hash, ChunkRangesSeq::root()),
        ).complete()).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(matches!(
            store.blobs().status(malicious.hash).await.expect("status"),
            BlobStatus::NotFound
        ), "unauthorized PUSH inserted data");

        let outsider = local_endpoint(unknown_key).await;
        if let Ok(unknown_connection) = outsider.connect(remote, iroh_blobs::ALPN).await {
            let attempt = tokio::time::timeout(
                NET_STEP_DEADLINE,
                get_blob(unknown_connection, allowed_tag.hash).bytes(),
            ).await.expect("unknown peer request must settle");
            assert!(attempt.is_err(), "unknown transport peer read blob");
        }
        drop(connection);
        router.shutdown().await.expect("provider shutdown");
        allowed.close().await;
        outsider.close().await;
        worker.abort();
    })
    .await
    .expect("security fixture did not finish");
}

/// Uses the EXACT existing Staya encoder/receiver and QUIC stream framing,
/// with one application-level receipt per 64KiB fragment.
async fn roundtrip_staya(
    source: &StayaTransport, receiver: &StayaTransport, payload: &[u8],
) -> (usize, u128, u64, u64, u64, u64) {
    let (incoming, outgoing) = tokio::try_join!(
        receiver.accept_one(), source.connect(receiver.address()),
    ).expect("local Staya QUIC connected");
    let manifest = TransferManifest::for_bytes(payload).expect("bounded source");
    let mut staged = VerifiedAssembly::new(manifest).expect("staging");
    let mut accounting = 0usize;
    let traffic_before = outgoing.stats();
    let start = Instant::now();
    for (i, data) in payload.chunks(MAX_CHUNK_BYTES).enumerate() {
        let offset = (i * MAX_CHUNK_BYTES) as u64;
        let frame = manifest.encode_chunk(offset, data).expect("encode");
        accounting += frame.len() + 8; // explicitly count the 8-byte receipt
        let (rx_offset, tx_offset) = tokio::join!(
            async {
                let (mut send, mut recv) = incoming.accept_bi().await.expect("accept");
                let wire = recv.read_to_end(MAX_CHUNK_BYTES + FRAME_HEADER_BYTES)
                    .await.expect("read Staya frame");
                let result = staged.accept_frame(&wire).expect("verify Staya frame");
                send.write_all(&result.to_le_bytes()).await.expect("ack");
                send.finish().expect("finish receipt");
                result
            },
            async {
                let (mut send, mut recv) = outgoing.open_bi().await.expect("open");
                send.write_all(&frame).await.expect("send");
                send.finish().expect("finish data");
                let ack = recv.read_to_end(8).await.expect("read receipt");
                u64::from_le_bytes(ack.try_into().expect("8-byte receipt"))
            },
        );
        assert_eq!(rx_offset, tx_offset, "Staya wrong receipt");
    }
    assert_eq!(staged.complete_bytes(), Some(payload), "Staya restored wrong bytes");
    let us = start.elapsed().as_micros();
    let traffic_after = outgoing.stats();
    let tx_bytes = traffic_after.udp_tx.bytes.saturating_sub(traffic_before.udp_tx.bytes);
    let rx_bytes = traffic_after.udp_rx.bytes.saturating_sub(traffic_before.udp_rx.bytes);
    let tx_packets = traffic_after.udp_tx.datagrams.saturating_sub(traffic_before.udp_tx.datagrams);
    let rx_packets = traffic_after.udp_rx.datagrams.saturating_sub(traffic_before.udp_rx.datagrams);
    outgoing.close(0u8.into(), b"test done");
    incoming.close(0u8.into(), b"test done");
    (accounting, us, tx_bytes, rx_bytes, tx_packets, rx_packets)
}

#[tokio::test(flavor = "current_thread")]
async fn compare_real_quic_verified_stream_against_existing_staya_frames() {
    tokio::time::timeout(Duration::from_secs(90), async {
        let blob_peer_key = SecretKey::generate();
        let blob_server_key = SecretKey::generate();
        let mut hashes = HashSet::new();
        let mut inputs = Vec::new();
        let store = MemStore::new();
        let mut tags = Vec::new();
        // >64KiB captures the Staya frame boundary; 8MiB exercises a larger blob.
        for size in [1, 64 * 1024 + 17, 1024 * 1024, 8 * 1024 * 1024] {
            let data = bytes(size);
            let tag = store.add_slice(&data).await.expect("seed");
            hashes.insert(tag.hash);
            inputs.push((data, tag.hash));
            tags.push(tag);
        }
        let (events, worker) = restricted_events(blob_peer_key.public(), hashes);
        let blob_server = local_endpoint(blob_server_key).await;
        let router = Router::builder(blob_server)
            .accept(iroh_blobs::ALPN, BlobsProtocol::new(&store, Some(events)))
            .spawn();
        let blob_peer = local_endpoint(blob_peer_key).await;

        let key_a = SecretKey::generate();
        let key_b = SecretKey::generate();
        let a = StayaTransport::bind(key_a.clone(), AllowedPeers::new([key_b.public()]))
            .await.expect("Staya A");
        let b = StayaTransport::bind(key_b, AllowedPeers::new([key_a.public()]))
            .await.expect("Staya B");

        for (payload, hash) in inputs {
            // Both measurements exclude connection setup, store import,
            // File Gate, network-layer IP/TLS overhead, and durable receipt.
            let (staya_bytes, staya_us, staya_udp_tx, staya_udp_rx, staya_udp_tx_packets, staya_udp_rx_packets) =
                roundtrip_staya(&a, &b, &payload).await;
            let connection = blob_peer
                .connect(router.endpoint().addr(), iroh_blobs::ALPN)
                .await.expect("blob QUIC");
            let traffic_before = connection.stats();
            let started = Instant::now();
            let (received, stats) = get_blob(connection.clone(), hash)
                .bytes_and_stats().await.expect("verified blob fetch");
            let blob_us = started.elapsed().as_micros();
            let traffic_after = connection.stats();
            let blob_udp_tx = traffic_after.udp_tx.bytes.saturating_sub(traffic_before.udp_tx.bytes);
            let blob_udp_rx = traffic_after.udp_rx.bytes.saturating_sub(traffic_before.udp_rx.bytes);
            let blob_udp_tx_packets = traffic_after.udp_tx.datagrams.saturating_sub(traffic_before.udp_tx.datagrams);
            let blob_udp_rx_packets = traffic_after.udp_rx.datagrams.saturating_sub(traffic_before.udp_rx.datagrams);
            assert_eq!(received.as_ref(), payload.as_slice());
            // These counters have distinct scopes and MUST NOT be compared
            // as equivalent full IP/UDP wire bytes.
            println!(
                "P2P_BLOB_AB source=isolated_quic size={} staya_application_bytes_including_receipts={} staya_elapsed_us={} blobs_receiver_protocol_bytes={} blobs_elapsed_us={} staya_udp_tx_bytes={} staya_udp_rx_bytes={} staya_udp_tx_datagrams={} staya_udp_rx_datagrams={} blobs_udp_tx_bytes={} blobs_udp_rx_bytes={} blobs_udp_tx_datagrams={} blobs_udp_rx_datagrams={} verified=true direct_only=true",
                payload.len(), staya_bytes, staya_us,
                stats.total_bytes_read(), blob_us,
                staya_udp_tx, staya_udp_rx, staya_udp_tx_packets, staya_udp_rx_packets,
                blob_udp_tx, blob_udp_rx, blob_udp_tx_packets, blob_udp_rx_packets,
            );
            connection.close(0u8.into(), b"done");
        }
        drop(tags);
        router.shutdown().await.expect("blobs shutdown");
        blob_peer.close().await;
        a.close().await;
        b.close().await;
        worker.abort();
    })
    .await
    .expect("bounded benchmark did not finish");
}
