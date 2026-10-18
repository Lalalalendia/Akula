//! Research-only: pinned Roha/noq + iroh-blobs 0.103.1 against Akula Staya framing.
//! Never import remote data into the user's filesystem or grant File Gate rights.
//! Runs against independent loopback QUIC endpoints. NOT a physical WAN test.
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use akula_roha_transport::{
    AllowedPeers, StayaTransport,
    transfer::{
        FRAME_HEADER_BYTES, MAX_CHUNK_BYTES, MAX_OBJECT_BYTES, TransferManifest, VerifiedAssembly,
    },
};
use futures_util::StreamExt;
use iroh::{Endpoint, EndpointId, RelayMode, SecretKey, endpoint::presets, protocol::Router};
use iroh_blobs::{
    BlobsProtocol, Hash,
    api::blobs::BlobStatus,
    get::request::get_blob,
    protocol::{
        ChunkRanges, ChunkRangesExt, ChunkRangesSeq, GetManyRequest, GetRequest, ObserveRequest,
        PushRequest,
    },
    provider::events::{
        AbortReason, ConnectMode, EventMask, EventSender, ObserveMode, ProviderMessage,
        RequestMode, ThrottleMode,
    },
    store::{fs::FsStore, mem::MemStore},
};

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
        let secret_tag = store
            .add_slice(b"other secret blob")
            .await
            .expect("seed denied");
        let (events, worker) =
            restricted_events(allowed_key.public(), HashSet::from([allowed_tag.hash]));
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
            NET_STEP_DEADLINE,
            get_blob(connection.clone(), allowed_tag.hash).bytes(),
        )
        .await
        .expect("allowed GET bounded")
        .expect("allowed GET succeeds");
        assert_eq!(received.as_ref(), allowed_data.as_slice());

        let forbidden = tokio::time::timeout(
            NET_STEP_DEADLINE,
            get_blob(connection.clone(), secret_tag.hash).bytes(),
        )
        .await
        .expect("hash refusal timely");
        assert!(forbidden.is_err(), "unlisted blob was readable");

        let local = MemStore::new();
        let malicious = local
            .add_slice(b"unsolicited remote push")
            .await
            .expect("attacker local blob");
        let get_many = tokio::time::timeout(
            NET_STEP_DEADLINE,
            local
                .remote()
                .execute_get_many(
                    connection.clone(),
                    GetManyRequest::new(vec![allowed_tag.hash], ChunkRangesSeq::all()),
                )
                .complete(),
        )
        .await
        .expect("get-many must settle");
        assert!(
            get_many.is_err(),
            "get-many bypassed single-hash authorization"
        );

        let observe = local
            .remote()
            .observe(connection.clone(), ObserveRequest::new(allowed_tag.hash));
        futures_util::pin_mut!(observe);
        let observation = tokio::time::timeout(NET_STEP_DEADLINE, observe.next())
            .await
            .expect("observe must settle");
        assert!(!matches!(observation, Some(Ok(_))), "observe leaked state");

        // Upstream notes a denied PUSH may appear successful to the sender:
        // receiving-store non-mutation, not client status, is the ground truth.
        let _ = tokio::time::timeout(
            NET_STEP_DEADLINE,
            local
                .remote()
                .execute_push(
                    connection.clone(),
                    PushRequest::new(malicious.hash, ChunkRangesSeq::root()),
                )
                .complete(),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            matches!(
                store.blobs().status(malicious.hash).await.expect("status"),
                BlobStatus::NotFound
            ),
            "unauthorized PUSH inserted data"
        );

        let outsider = local_endpoint(unknown_key).await;
        if let Ok(unknown_connection) = outsider.connect(remote, iroh_blobs::ALPN).await {
            let attempt = tokio::time::timeout(
                NET_STEP_DEADLINE,
                get_blob(unknown_connection, allowed_tag.hash).bytes(),
            )
            .await
            .expect("unknown peer request must settle");
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
    source: &StayaTransport,
    receiver: &StayaTransport,
    payload: &[u8],
) -> (usize, u128, u64, u64, u64, u64) {
    let (incoming, outgoing) =
        tokio::try_join!(receiver.accept_one(), source.connect(receiver.address()),)
            .expect("local Staya QUIC connected");
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
                let wire = recv
                    .read_to_end(MAX_CHUNK_BYTES + FRAME_HEADER_BYTES)
                    .await
                    .expect("read Staya frame");
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
    assert_eq!(
        staged.complete_bytes(),
        Some(payload),
        "Staya restored wrong bytes"
    );
    let us = start.elapsed().as_micros();
    let traffic_after = outgoing.stats();
    let tx_bytes = traffic_after
        .udp_tx
        .bytes
        .saturating_sub(traffic_before.udp_tx.bytes);
    let rx_bytes = traffic_after
        .udp_rx
        .bytes
        .saturating_sub(traffic_before.udp_rx.bytes);
    let tx_packets = traffic_after
        .udp_tx
        .datagrams
        .saturating_sub(traffic_before.udp_tx.datagrams);
    let rx_packets = traffic_after
        .udp_rx
        .datagrams
        .saturating_sub(traffic_before.udp_rx.datagrams);
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

#[tokio::test(flavor = "current_thread")]
async fn resume_missing_verified_ranges_after_new_quic_connection() {
    tokio::time::timeout(TEST_DEADLINE, async {
        let sender_key = SecretKey::generate();
        let recipient_key = SecretKey::generate();
        let data = bytes(2 * 1024 * 1024 + 13);
        let store = MemStore::new();
        let tag = store.add_slice(&data).await.expect("seed authorized blob");
        let hash = tag.hash;
        let (events, worker) = restricted_events(recipient_key.public(), HashSet::from([hash]));
        let provider = local_endpoint(sender_key).await;
        let router = Router::builder(provider)
            .accept(iroh_blobs::ALPN, BlobsProtocol::new(&store, Some(events)))
            .spawn();
        let peer = local_endpoint(recipient_key).await;
        let local = MemStore::new();

        // A bounded 64KiB prefix is verified and staged only in the isolated MemStore.
        let first = peer.connect(router.endpoint().addr(), iroh_blobs::ALPN)
            .await.expect("first QUIC connection");
        let prefix = GetRequest::builder()
            .root(ChunkRanges::bytes(..(64 * 1024)))
            .build(hash);
        let initial_stats = tokio::time::timeout(
            NET_STEP_DEADLINE,
            local.remote().execute_get(first.clone(), prefix).complete(),
        ).await.expect("prefix fetch not stalled").expect("prefix accepted");
        first.close(0u8.into(), b"simulate reconnect");
        drop(first);

        let pre_resume = local.remote().local(hash).await.expect("inspect verified prefix");
        assert!(!pre_resume.is_complete(), "prefix unexpectedly completed whole blob");
        assert!(
            pre_resume.local_bytes() >= 64 * 1024,
            "first verified 64KiB missing from staging"
        );
        assert!(
            pre_resume.local_bytes() < data.len() as u64,
            "partial prefix was counted as a complete blob"
        );
        // The *existing locally verified* range set computes the missing
        // ranges; peer cannot claim an unauthenticated baseline.
        let missing = pre_resume.missing();

        // Open another QUIC connection, request only missing bytes, then
        // verify exact final content independently. No File Gate or filesystem.
        let second = peer.connect(router.endpoint().addr(), iroh_blobs::ALPN)
            .await.expect("replacement QUIC connection");
        let resumed_stats = tokio::time::timeout(
            NET_STEP_DEADLINE,
            local.remote().execute_get(second.clone(), missing).complete(),
        ).await.expect("resume not stalled").expect("verified missing ranges");
        assert!(
            local.remote().local(hash).await.expect("final status").is_complete(),
            "resumed object is not complete"
        );
        let restored = local.get_bytes(hash).await.expect("complete bytes");
        assert_eq!(restored.as_ref(), data.as_slice(), "resume reconstructed wrong bytes");
        println!(
            "P2P_BLOB_RESUME size={} initial_verified_bytes={} initial_protocol_bytes={} resumed_protocol_bytes={} after_reconnect=true same_store=true durable_restart=false",
            data.len(), pre_resume.local_bytes(), initial_stats.total_bytes_read(),
            resumed_stats.total_bytes_read(),
        );
        second.close(0u8.into(), b"done");
        router.shutdown().await.expect("provider shutdown");
        peer.close().await;
        worker.abort();
    }).await.expect("bounded QUIC resume fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn disk_staging_survives_graceful_store_shutdown_then_resumes() {
    tokio::time::timeout(Duration::from_secs(75), async {
        // A disposable directory only. Do not authorize any Akula workspace
        // path, user file, canonical V1 blob or File Gate mutation.
        let dir = tempfile::tempdir().expect("private disposable disk staging directory");
        let data = bytes(2 * 1024 * 1024 + 13);
        assert!(data.len() <= MAX_OBJECT_BYTES);

        let provider_key = SecretKey::generate();
        let peer_key = SecretKey::generate();
        let source = MemStore::new();
        let tag = source.add_slice(&data).await.expect("seed verified source");
        let hash = tag.hash;
        let (events, worker) = restricted_events(peer_key.public(), HashSet::from([hash]));
        let server = local_endpoint(provider_key).await;
        let router = Router::builder(server)
            .accept(iroh_blobs::ALPN, BlobsProtocol::new(&source, Some(events)))
            .spawn();
        let peer = local_endpoint(peer_key).await;

        let disk = FsStore::load(dir.path()).await.expect("create isolated FsStore");
        let initial = peer
            .connect(router.endpoint().addr(), iroh_blobs::ALPN)
            .await
            .expect("connect initial");
        let prefix_request = GetRequest::builder()
            .root(ChunkRanges::bytes(..(64 * 1024)))
            .build(hash);
        let prefix_stats = tokio::time::timeout(
            NET_STEP_DEADLINE,
            disk.remote()
                .execute_get(initial.clone(), prefix_request)
                .complete(),
        )
        .await
        .expect("prefix fetch timed out")
        .expect("prefix fetch");
        let before_close = disk
            .remote()
            .local(hash)
            .await
            .expect("inspect verified on-disk ranges");
        let recorded_bytes = before_close.local_bytes();
        assert_eq!(recorded_bytes, 64 * 1024, "verified prefix was unexpected size");
        assert!(!before_close.is_complete(), "partial prefix marked complete");
        initial.close(0u8.into(), b"close initial");
        drop(initial);

        // FsStore itself explicitly disclaims crash-durable latest writes.
        // We must shut down cleanly and allow all its state to be persisted.
        disk.shutdown().await.expect("flush and shut down disk staging");
        drop(disk);

        let reopened = FsStore::load(dir.path()).await.expect("reopen same disk staging");
        let restored_prefix = reopened
            .remote()
            .local(hash)
            .await
            .expect("recover verified on-disk prefix");
        assert_eq!(
            restored_prefix.local_bytes(),
            recorded_bytes,
            "restart lost verified committed range"
        );
        assert!(
            !restored_prefix.is_complete(),
            "partial object already claimed complete after restart"
        );
        let missing = restored_prefix.missing();
        let replacement = peer
            .connect(router.endpoint().addr(), iroh_blobs::ALPN)
            .await
            .expect("connect replacement");

        let resume_stats = tokio::time::timeout(
            NET_STEP_DEADLINE,
            reopened.remote()
                .execute_get(replacement.clone(), missing)
                .complete(),
        )
        .await
        .expect("range resume timed out")
        .expect("range resume");
        assert!(
            reopened
                .remote()
                .local(hash)
                .await
                .expect("final range observation")
                .is_complete(),
            "on-disk recovered blob is not complete"
        );
        let rebuilt = reopened.get_bytes(hash).await.expect("read restored blob");
        assert_eq!(rebuilt.as_ref(), data.as_slice(), "restored bytes changed");
        assert_eq!(Hash::new(rebuilt.as_ref()), hash, "full BLAKE3 digest changed");

        println!(
            "P2P_BLOB_FS_RESTART size={} durable_verified_prefix={} prefix_response_bytes={} resume_response_bytes={} clean_shutdown=true new_fs_store=true new_quic_connection=true crash_restart=false file_gate=false",
            data.len(),
            recorded_bytes,
            prefix_stats.total_bytes_read(),
            resume_stats.total_bytes_read()
        );
        replacement.close(0u8.into(), b"done");
        reopened.shutdown().await.expect("flush final complete disk store");
        router.shutdown().await.expect("provider shutdown");
        peer.close().await;
        worker.abort();
        drop(tag);
    })
    .await
    .expect("durable staging laboratory timed out");
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "private child of two-process P2P FsStore recovery test"]
async fn child_process_downloads_missing_verified_ranges() {
    // A distinct test binary process, invoked by the parent using --ignored.
    // No secrets or user paths are persisted; the test key is ephemeral.
    let directory = std::env::var("AKULA_TEST_BLOB_DISK").expect("child disk root");
    let remote_id: EndpointId = std::env::var("AKULA_TEST_PROVIDER_ID")
        .expect("child source id")
        .parse()
        .expect("valid provider id");
    let address_hints = std::env::var("AKULA_TEST_PROVIDER_IPS").expect("child IP hints");
    let ephemeral_key: SecretKey = std::env::var("AKULA_TEST_PEER_KEY")
        .expect("child ephemeral key")
        .parse()
        .expect("valid test-only peer key");

    let mut remote = iroh::EndpointAddr::new(remote_id);
    let mut total_hints = 0usize;
    for hint in address_hints.split(';').filter(|s| !s.is_empty()) {
        remote = remote.with_ip_addr(hint.parse().expect("valid local QUIC address hint"));
        total_hints += 1;
    }
    assert!(total_hints > 0, "no direct-only address hints");

    let size = 2 * 1024 * 1024 + 13;
    let original = bytes(size);
    let hash = Hash::new(&original);
    let disk = FsStore::load(&directory)
        .await
        .expect("separate process reopened private FsStore");
    let partial = disk
        .remote()
        .local(hash)
        .await
        .expect("read persistent verified range");
    assert_eq!(partial.local_bytes(), 64 * 1024);
    assert!(!partial.is_complete(), "prefix reported complete");

    let peer = local_endpoint(ephemeral_key).await;
    let connection = peer
        .connect(remote, iroh_blobs::ALPN)
        .await
        .expect("separate process QUIC connection");
    let stats = tokio::time::timeout(
        NET_STEP_DEADLINE,
        disk.remote()
            .execute_get(connection.clone(), partial.missing())
            .complete(),
    )
    .await
    .expect("cross-process resume time budget")
    .expect("cross-process missing range fetch");

    assert!(
        disk.remote()
            .local(hash)
            .await
            .expect("new process final status")
            .is_complete(),
        "reassembled FsStore blob still incomplete"
    );
    let finished = disk
        .get_bytes(hash)
        .await
        .expect("read complete child blob");
    assert_eq!(finished.as_ref(), original.as_slice());
    assert_eq!(Hash::new(finished.as_ref()), hash);
    println!(
        "P2P_BLOB_OS_PROCESS_RESUME size={} restored_prefix={} resumed_protocol_bytes={} physical_process_restart=true crash_durable=false granted_vault=false user_file_mutation=false",
        size,
        partial.local_bytes(),
        stats.total_bytes_read(),
    );
    connection.close(0u8.into(), b"child done");
    disk.shutdown()
        .await
        .expect("persist full bytes after child");
    peer.close().await;
}

#[tokio::test(flavor = "current_thread")]
async fn fs_verified_prefix_is_resumed_in_a_distinct_client_os_process() {
    tokio::time::timeout(Duration::from_secs(90), async {
        // Two actual client processes share only this ephemeral test directory.
        // Parent retains the QUIC source; a separate test binary reopens the
        // receiver's disk store and downloads all missing verified ranges.
        let dir = tempfile::tempdir().expect("disposable test directory");
        let data = bytes(2 * 1024 * 1024 + 13);
        assert!(data.len() <= MAX_OBJECT_BYTES);

        let provider_key = SecretKey::generate();
        let peer_key = SecretKey::generate();
        let source = MemStore::new();
        let source_tag = source.add_slice(&data).await.expect("seed source");
        let hash = source_tag.hash;
        let (events, worker) = restricted_events(peer_key.public(), HashSet::from([hash]));
        let provider = local_endpoint(provider_key).await;
        let router = Router::builder(provider)
            .accept(iroh_blobs::ALPN, BlobsProtocol::new(&source, Some(events)))
            .spawn();
        let provider_addr = router.endpoint().addr();
        let address_hints = provider_addr
            .ip_addrs()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(";");
        assert!(!address_hints.is_empty(), "local provider has no direct IP hints");

        let stage_peer = local_endpoint(peer_key.clone()).await;
        let disk = FsStore::load(dir.path()).await.expect("start parent FsStore");
        let first = stage_peer
            .connect(provider_addr.clone(), iroh_blobs::ALPN)
            .await
            .expect("first connection");
        let prefix = GetRequest::builder()
            .root(ChunkRanges::bytes(..(64 * 1024)))
            .build(hash);
        tokio::time::timeout(
            NET_STEP_DEADLINE,
            disk.remote().execute_get(first.clone(), prefix).complete(),
        )
        .await
        .expect("verified prefix deadline")
        .expect("verified prefix accepted");
        assert_eq!(
            disk.remote()
                .local(hash)
                .await
                .expect("parent prefix status")
                .local_bytes(),
            64 * 1024
        );
        first.close(0u8.into(), b"parent stage done");
        drop(first);
        disk.shutdown().await.expect("graceful durable receiver checkpoint");
        drop(disk);
        stage_peer.close().await;

        let binary = std::env::current_exe().expect("absolute integration-test binary");
        let directory = dir.path().to_path_buf();
        let provider_id = provider_addr.id.to_string();
        let peer_secret = peer_key
            .to_bytes()
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>();
        let output = tokio::task::spawn_blocking(move || {
            std::process::Command::new(binary)
                .arg("--exact")
                .arg("child_process_downloads_missing_verified_ranges")
                .arg("--ignored")
                .arg("--nocapture")
                .env("AKULA_TEST_BLOB_DISK", directory)
                .env("AKULA_TEST_PROVIDER_ID", provider_id)
                .env("AKULA_TEST_PROVIDER_IPS", address_hints)
                .env("AKULA_TEST_PEER_KEY", peer_secret)
                .output()
                .expect("spawn separate receiver process")
        })
        .await
        .expect("join isolated child process");
        assert!(
            output.status.success(),
            "child receiver failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("P2P_BLOB_OS_PROCESS_RESUME"),
            "child did not verify disk recovery"
        );

        // Parent's independent reopened store confirms the child persisted
        // complete hash-checked canonical bytes before returning.
        let final_disk = FsStore::load(dir.path())
            .await
            .expect("parent inspect child-committed FS store");
        let complete = final_disk
            .remote()
            .local(hash)
            .await
            .expect("post-process status");
        assert!(complete.is_complete(), "child did not save complete blob");
        let final_data = final_disk.get_bytes(hash).await.expect("completed bytes");
        assert_eq!(final_data.as_ref(), data.as_slice());
        final_disk.shutdown().await.expect("final storage shutdown");
        println!(
            "P2P_BLOB_PROCESS_BOUNDARY size={} verified=true independent_client_process=true graceful_previous_shutdown=true forced_crash=false quota_enforced=false file_gate=false",
            data.len()
        );
        router.shutdown().await.expect("source shutdown");
        worker.abort();
        drop(source_tag);
    })
    .await
    .expect("two-process durable laboratory timed out");
}
