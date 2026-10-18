//! Research-only: pinned Roha/noq + iroh-blobs 0.103.1 against Akula Staya framing.
//! Never import remote data into the user's filesystem or grant File Gate rights.
//! Runs against independent loopback QUIC endpoints. NOT a physical WAN test.
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use akula_roha_transport::{
    AllowedPeers, StayaTransport,
    admission::{AdmissionError, AdmissionLimits, StageBudget},
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

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "current_thread")]
#[ignore = "private child, forcibly SIGKILLed by parent before Store::shutdown"]
async fn child_receives_unflushed_ranges_until_sigkill() {
    use std::path::PathBuf;
    let root = PathBuf::from(std::env::var("AKULA_CRASH_DISK").expect("private child directory"));
    let marker = root.join("verified-prefix.ready");
    let server_id: EndpointId = std::env::var("AKULA_CRASH_PROVIDER_ID")
        .expect("remote id")
        .parse()
        .expect("valid remote id");
    let key: SecretKey = std::env::var("AKULA_CRASH_PEER_SECRET")
        .expect("ephemeral secret")
        .parse()
        .expect("valid ephemeral secret");
    let addrs = std::env::var("AKULA_CRASH_PROVIDER_IPS").expect("private address hints");
    let mut remote = iroh::EndpointAddr::new(server_id);
    let mut count = 0;
    for hint in addrs.split(';').filter(|s| !s.is_empty()) {
        remote = remote.with_ip_addr(hint.parse().expect("valid local UDP address"));
        count += 1;
    }
    assert!(
        count > 0,
        "direct address required: never use public discovery"
    );
    const CRASH_SIZE: usize = 8 * 1024 * 1024 + 13;
    let expected_hash = Hash::new(&bytes(CRASH_SIZE));
    let store = FsStore::load(&root)
        .await
        .expect("child private disk store");
    let peer = local_endpoint(key).await;
    let conn = peer
        .connect(remote, iroh_blobs::ALPN)
        .await
        .expect("child QUIC connection");
    let initial = GetRequest::builder()
        .root(ChunkRanges::bytes(..(64 * 1024)))
        .build(expected_hash);
    tokio::time::timeout(
        NET_STEP_DEADLINE,
        store.remote().execute_get(conn.clone(), initial).complete(),
    )
    .await
    .expect("initial range timeout")
    .expect("initial verified range");

    let local = store
        .remote()
        .local(expected_hash)
        .await
        .expect("verified range status");
    assert_eq!(local.local_bytes(), 64 * 1024);
    assert!(!local.is_complete());
    // Parent sees this marker only after Bao verification. No Store::shutdown
    // or FsStore actor flush is called on this execution path.
    std::fs::write(&marker, b"verified-and-not-shutdown").expect("notify supervising process");
    // Keep receiving bounded ranges, with minimum 20ms between requests,
    // so the supervisor kills a real incomplete transfer, not a completed blob.
    for offset in (64 * 1024..CRASH_SIZE).step_by(64 * 1024) {
        let end = (offset + 64 * 1024).min(CRASH_SIZE);
        let next = GetRequest::builder()
            .root(ChunkRanges::bytes((offset as u64)..(end as u64)))
            .build(expected_hash);
        tokio::time::timeout(
            NET_STEP_DEADLINE,
            store.remote().execute_get(conn.clone(), next).complete(),
        )
        .await
        .expect("subsequent range timeout")
        .expect("subsequent range verification");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("supervisor failed to SIGKILL the active partial receiver");
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "current_thread")]
async fn kill_receiver_during_unflushed_partial_and_fail_closed() {
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};

    tokio::time::timeout(Duration::from_secs(95), async {
        const CRASH_SIZE: usize = 8 * 1024 * 1024 + 13;
        assert!(CRASH_SIZE < MAX_OBJECT_BYTES);
        let dir = tempfile::tempdir().expect("disposable crash lab only");
        let data = bytes(CRASH_SIZE);
        let hash = Hash::new(&data);
        let provider_key = SecretKey::generate();
        let peer_key = SecretKey::generate();
        let source = MemStore::new();
        let source_tag = source.add_slice(&data).await.expect("seed accepted source");
        let (events, worker) = restricted_events(peer_key.public(), HashSet::from([hash]));
        let provider = local_endpoint(provider_key).await;
        let router = Router::builder(provider)
            .accept(iroh_blobs::ALPN, BlobsProtocol::new(&source, Some(events)))
            .spawn();
        let provider_addr = router.endpoint().addr();
        let addr_hints = provider_addr
            .ip_addrs()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(";");
        assert!(!addr_hints.is_empty(), "expected private direct-only address");

        let child_binary = std::env::current_exe().expect("absolute test binary");
        let secret = peer_key
            .to_bytes()
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>();
        let mut receiver = Command::new(child_binary)
            .arg("--exact")
            .arg("child_receives_unflushed_ranges_until_sigkill")
            .arg("--ignored")
            .arg("--nocapture")
            .env("AKULA_CRASH_DISK", dir.path())
            .env("AKULA_CRASH_PROVIDER_ID", provider_addr.id.to_string())
            .env("AKULA_CRASH_PROVIDER_IPS", &addr_hints)
            .env("AKULA_CRASH_PEER_SECRET", &secret)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn separate crashable receiver OS process");

        let mut observed_verified_prefix = false;
        for _ in 0..1200 {
            if dir.path().join("verified-prefix.ready").is_file() {
                observed_verified_prefix = true;
                break;
            }
            if let Some(status) = receiver.try_wait().expect("child process status") {
                panic!("receiver exited before SIGKILL test: {status}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Always terminate a child before panic/cleanup if it missed readiness.
        if !observed_verified_prefix {
            let _ = receiver.kill();
            let _ = receiver.wait();
            panic!("crashable receiver never acknowledged first verified range");
        }
        receiver.kill().expect("force SIGKILL of incomplete receiver");
        let killed = receiver.wait().expect("reap killed receiver");
        assert_eq!(killed.signal(), Some(9), "receiver must die by SIGKILL");

        // FsStore docs explicitly warn of losing recent writes on a hard kill.
        // Accepted recovery: use only validated local ranges, or QUARANTINE
        // crashed staging and request the entire object into a new directory.
        let reopened = tokio::time::timeout(Duration::from_secs(10), FsStore::load(dir.path()))
            .await
            .ok()
            .and_then(Result::ok);
        let mut recovered_bytes = 0u64;
        let mut discarded_crash_state = reopened.is_none();
        let mut disk = if let Some(reopened) = reopened {
            reopened
        } else {
            FsStore::load(dir.path().join("fresh-fallback"))
                .await
                .expect("isolated fallback FsStore")
        };
        let observation = tokio::time::timeout(
            Duration::from_secs(10),
            disk.remote().local(hash),
        )
        .await;
        let mut request = match observation {
            Ok(Ok(local)) if !local.is_complete() && local.local_bytes() <= CRASH_SIZE as u64 => {
                recovered_bytes = local.local_bytes();
                local.missing()
            }
            _ => {
                // The crashed store may be unopenable or report invalid state.
                // Never mutate a user's file or treat this as a complete blob.
                let _ = disk.shutdown().await;
                discarded_crash_state = true;
                disk = FsStore::load(dir.path().join("fresh-after-invalid"))
                    .await
                    .expect("separate fallback store");
                GetRequest::builder().root(ChunkRanges::all()).build(hash)
            }
        };
        let peer = local_endpoint(peer_key.clone()).await;
        let connection = peer
            .connect(provider_addr.clone(), iroh_blobs::ALPN)
            .await
            .expect("recovery QUIC");

        let mut valid = false;
        let mut response_bytes = 0u64;
        if let Ok(Ok(stats)) = tokio::time::timeout(
            Duration::from_secs(25),
            disk.remote().execute_get(connection.clone(), request.clone()).complete(),
        )
        .await
        {
            response_bytes = stats.total_bytes_read() as u64;
            if let Ok(finished) = disk.get_bytes(hash).await {
                valid = finished.as_ref() == data.as_slice()
                    && Hash::new(finished.as_ref()) == hash;
            }
        }
        if !valid {
            // If the recovered bitfield was incomplete/incorrect after the
            // crash, explicitly discard it and re-fetch from known hash.
            let _ = disk.shutdown().await;
            discarded_crash_state = true;
            request = GetRequest::builder().root(ChunkRanges::all()).build(hash);
            disk = FsStore::load(dir.path().join("verified-full-fallback"))
                .await
                .expect("new isolated fallback after invalid partial");
            let stats = tokio::time::timeout(
                Duration::from_secs(25),
                disk.remote().execute_get(connection.clone(), request).complete(),
            )
            .await
            .expect("full fallback deadline")
            .expect("full fallback verified download");
            response_bytes += stats.total_bytes_read() as u64;
        }

        let result = disk.get_bytes(hash).await.expect("full reconstructed bytes");
        assert_eq!(result.as_ref(), data.as_slice(), "crashed data was never trusted");
        assert_eq!(Hash::new(result.as_ref()), hash, "wrong BLAKE3 after crash");
        assert!(
            disk.remote().local(hash).await.expect("recovery status").is_complete(),
            "partial blob is not accepted as complete"
        );
        disk.shutdown().await.expect("persist verified completed fallback");
        println!(
            "P2P_BLOB_SIGKILL size={} killed_signal=9 marker_after_verified_prefix=true recovered_verified_bytes={} fallback_used={} receiver_protocol_bytes={} exact_b3=true user_file_gate=false force_killed_before_store_shutdown=true",
            data.len(), recovered_bytes, discarded_crash_state, response_bytes,
        );
        connection.close(0u8.into(), b"end crash fixture");
        peer.close().await;
        router.shutdown().await.expect("source shutdown");
        worker.abort();
        drop(source_tag);
    })
    .await
    .expect("SIGKILL crash-recovery fixture exceeded hard deadline");
}

#[tokio::test(flavor = "current_thread")]
async fn quota_requires_preflight_before_quic_get() {
    tokio::time::timeout(TEST_DEADLINE, async {
        // These descriptors are local fixed test metadata. No remote-provided
        // size is accepted as an authority for quota or VaultId permissions.
        let provider_key = SecretKey::generate();
        let client_key = SecretKey::generate();
        let bytes = bytes(64 * 1024 + 17);
        let store = MemStore::new();
        let tag = store.add_slice(&bytes).await.expect("seed immutable known blob");
        let hash = tag.hash;
        let expected = *blake3::hash(&bytes).as_bytes();
        assert_eq!(expected, *hash.as_bytes(), "same BLAKE3 contents");
        let budget = StageBudget::new(AdmissionLimits {
            max_object_bytes: 128 * 1024,
            max_tracked_bytes: 128 * 1024,
            max_peer_bytes: 128 * 1024,
            max_active: 1,
            max_peer_active: 1,
        })
        .expect("valid independent quota policy");
        let client_id = client_key.public();

        assert!(matches!(
            budget.reserve(client_id, expected, (128 * 1024) + 1),
            Err(AdmissionError::ObjectTooLarge)
        ));
        assert_eq!(budget.snapshot().expect("empty quota").tracked_objects, 0);
        let mut lease = budget
            .reserve(client_id, expected, bytes.len() as u64)
            .expect("authorized object fits quota");
        assert!(matches!(
            budget.reserve(client_id, *blake3::hash(b"another").as_bytes(), 1),
            Err(AdmissionError::GlobalSlotsExhausted)
        ));
        assert!(matches!(
            budget.reserve(client_id, expected, bytes.len() as u64),
            Err(AdmissionError::DuplicateObject)
        ));

        let (events, worker) = restricted_events(client_id, HashSet::from([hash]));
        let server = local_endpoint(provider_key).await;
        let router = Router::builder(server)
            .accept(iroh_blobs::ALPN, BlobsProtocol::new(&store, Some(events)))
            .spawn();
        let client = local_endpoint(client_key).await;
        let connection = client
            .connect(router.endpoint().addr(), iroh_blobs::ALPN)
            .await
            .expect("authenticated private QUIC connection");

        let downloaded = tokio::time::timeout(
            NET_STEP_DEADLINE,
            get_blob(connection.clone(), hash).bytes(),
        )
        .await
        .expect("bounded GET")
        .expect("verified iroh-blobs");
        assert_eq!(downloaded.as_ref(), bytes.as_slice());
        lease
            .verify(downloaded.as_ref())
            .expect("independent full BLAKE3 and size");
        assert_eq!(budget.snapshot().expect("reservation held").verified_objects, 1);
        assert_eq!(
            budget.snapshot().expect("reservation held").tracked_bytes,
            bytes.len() as u64,
            "completed download still consumes capacity"
        );

        // The future production adapter must delete the corresponding
        // disk staging data BEFORE releasing a retained reservation.
        drop(downloaded);
        drop(lease);
        assert_eq!(budget.snapshot().expect("released").tracked_bytes, 0);
        println!(
            "P2P_BLOB_ADMISSION size={} single_object_bounded=true global_slots_bounded=true duplicate_denied=true completed_bytes_held_until_cleanup=true verified=true hard_disk_quota=false file_gate=false",
            bytes.len()
        );
        connection.close(0u8.into(), b"quota lab done");
        router.shutdown().await.expect("provider shutdown");
        client.close().await;
        worker.abort();
        drop(tag);
    })
    .await
    .expect("admitted QUIC transfer time budget");
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "current_thread")]
async fn durable_admission_stays_charged_after_verified_quic_and_process_reopen() {
    use akula_roha_transport::durable_admission::{DurableError, DurableLedger};
    use std::{fs, os::unix::fs::PermissionsExt};

    tokio::time::timeout(TEST_DEADLINE, async {
        // Completely isolated Linux fixture. Directory containing the ledger
        // is not the user's workspace or the FsStore data root.
        let temp = tempfile::tempdir().expect("private test directory");
        let meta = temp.path().join("reservation-metadata");
        let stage = temp.path().join("verified-staging");
        fs::create_dir(&meta).expect("create test-only journal directory");
        fs::create_dir(&stage).expect("create test-only data directory");
        fs::set_permissions(&meta, fs::Permissions::from_mode(0o700))
            .expect("enforce private metadata permissions");
        let source_data = bytes(64 * 1024 + 17);
        let expected = *blake3::hash(&source_data).as_bytes();
        let policy = AdmissionLimits {
            max_object_bytes: source_data.len() as u64,
            max_tracked_bytes: source_data.len() as u64,
            max_peer_bytes: source_data.len() as u64,
            max_active: 1,
            max_peer_active: 1,
        };
        let server_key = SecretKey::generate();
        let peer_key = SecretKey::generate();
        let peer = peer_key.public();
        let store = MemStore::new();
        let tag = store.add_slice(&source_data).await.expect("trusted test object");
        assert_eq!(tag.hash.as_bytes(), &expected);
        let (events, worker) = restricted_events(peer, HashSet::from([tag.hash]));
        let server = local_endpoint(server_key).await;
        let router = Router::builder(server)
            .accept(iroh_blobs::ALPN, BlobsProtocol::new(&store, Some(events)))
            .spawn();

        let mut durable = DurableLedger::open(&meta, policy).expect("strict private Linux ledger");
        // Persist to disk BEFORE allocating memory or opening QUIC.
        durable.reserve(peer, expected, source_data.len() as u64)
            .expect("fsync reservation before network");
        let live = StageBudget::new(policy).expect("independent in-memory cap");
        let mut live_lease = live.reserve(peer, expected, source_data.len() as u64)
            .expect("memory quota");
        let receiver = local_endpoint(peer_key).await;
        let connection = receiver.connect(router.endpoint().addr(), iroh_blobs::ALPN)
            .await.expect("direct private QUIC");
        let returned = tokio::time::timeout(
            NET_STEP_DEADLINE,
            get_blob(connection.clone(), tag.hash).bytes(),
        ).await.expect("GET budget").expect("verified network fetch");
        assert_eq!(returned.as_ref(), source_data.as_slice());
        live_lease.verify(returned.as_ref()).expect("in-memory full hash");
        let temp_file = stage.join("payload");
        fs::write(&temp_file, returned.as_ref()).expect("test-only physical stage");
        fs::File::open(&temp_file)
            .expect("open stage")
            .sync_all()
            .expect("flush complete physical stage");
        fs::File::open(&stage)
            .expect("open staging directory")
            .sync_all()
            .expect("flush staging directory entry");
        durable.verify_full(expected, returned.as_ref())
            .expect("durably record verified byte length and BLAKE3");
        drop(returned);
        drop(durable);

        // New independent file handle/ledger instance; the old verified
        // staging still consumes the ENTIRE declared capacity.
        let mut recovered = DurableLedger::open(&meta, policy).expect("reopen disk journal");
        let snapshot = recovered.snapshot().expect("read persisted reservation");
        assert_eq!(snapshot.objects, 1);
        assert_eq!(snapshot.verified_objects, 1);
        assert_eq!(snapshot.reserved_bytes, source_data.len() as u64);
        assert!(matches!(
            recovered.reserve(peer, *blake3::hash(b"competing").as_bytes(), 1),
            Err(DurableError::SlotsExhausted)
        ));
        let repeated = fs::read(&temp_file).expect("read staged payload");
        assert_eq!(repeated, source_data, "restarted staging must match trusted bytes");
        // This callback can only be used by a real custody owner that verifies
        // FULL backing-data removal. This fixture has only one payload file.
        recovered.release_after_cleanup(expected, || {
            fs::remove_file(&temp_file)?;
            fs::File::open(&stage)?.sync_all()
        }).expect("persist release only after physical test file cleanup");
        assert_eq!(recovered.snapshot().expect("ledger after cleanup").reserved_bytes, 0);
        drop(recovered);
        drop(live_lease);
        assert_eq!(live.snapshot().expect("local cleanup").tracked_bytes, 0);
        println!(
            "P2P_BLOB_DURABLE_LEDGER size={} before_network_fsync=true reopened_after_transfer=true retained_verified_bytes={} duplicate_and_excess_denied=true removed_before_release=true filesystem_quota=false file_gate=false",
            source_data.len(), source_data.len()
        );
        connection.close(0u8.into(), b"end staging admission fixture");
        router.shutdown().await.expect("source server shutdown");
        receiver.close().await;
        worker.abort();
        drop(tag);
    }).await.expect("durable ledger QUIC fixture timeout");
}

/// This test is intentionally ignored in ordinary CI: it may write only to
/// a dedicated, *fresh* ext4 loopback volume under RUNNER_TEMP, never to a
/// user workspace or a non-mounted host path. The separate GitHub workflow
/// must mount and validate the finite image before invoking it.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires dedicated isolated 64MiB ext4 loopback mount"]
async fn bounded_ext4_fsstore_seed() {
    use akula_roha_transport::bounded_staging::StagingVolumeGuard;

    let root = std::path::PathBuf::from(
        std::env::var("AKULA_BOUNDED_EXT4_ROOT").expect("explicit lab mount root"),
    );
    let volume = StagingVolumeGuard::open(&root).expect("isolated private filesystem required");
    volume.check().expect("mountpoint not replaced");
    let path = root.join("real-fsstore");
    std::fs::create_dir(&path).expect("private real FsStore staging inside bounded mount");
    let payload = bytes(1024 * 1024 + 17);
    let hash = Hash::new(&payload);
    let store = FsStore::load(&path)
        .await
        .expect("start real FsStore on bounded ext4");
    let tag = store
        .add_slice(&payload)
        .await
        .expect("write content and Bao outboard");
    assert_eq!(tag.hash, hash);
    let exact = store.get_bytes(hash).await.expect("read Bao-verified seed");
    assert_eq!(exact.as_ref(), payload.as_slice());
    store.shutdown().await.expect("flush bounded FsStore");
    drop(tag);
    drop(store);
    volume.check().expect("still on same isolated FS");
    println!(
        "P2P_BLOB_EXT4_FSSTORE_SEED size={} physical_mount_present=true real_fsstore=true full_b3_verified=true user_file_gate=false",
        payload.len()
    );
}

/// Reopen the same FsStore *after the CI script has unmounted and remounted
/// the actual finite ext4 image*, to ensure the real backend persists.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires remounted dedicated ext4 image previously seeded by fixture"]
async fn bounded_ext4_fsstore_reopen() {
    use akula_roha_transport::bounded_staging::StagingVolumeGuard;

    let root = std::path::PathBuf::from(
        std::env::var("AKULA_BOUNDED_EXT4_ROOT").expect("explicit lab remount root"),
    );
    let volume = StagingVolumeGuard::open(&root).expect("separate FS after remount");
    volume.check().expect("mount identity during reopen");
    let payload = bytes(1024 * 1024 + 17);
    let hash = Hash::new(&payload);
    let store = FsStore::load(root.join("real-fsstore"))
        .await
        .expect("reopen actual FsStore database after unmount");
    let status = store
        .remote()
        .local(hash)
        .await
        .expect("reopen local verified status");
    assert!(
        status.is_complete(),
        "on-disk FsStore metadata lost across remount"
    );
    let data = store
        .get_bytes(hash)
        .await
        .expect("read persisted verified data");
    assert_eq!(data.as_ref(), payload.as_slice());
    assert_eq!(Hash::new(data.as_ref()), hash);
    store.shutdown().await.expect("flush reopened FsStore");
    volume.check().expect("bounded mount still present");
    println!(
        "P2P_BLOB_EXT4_FSSTORE_REOPEN size={} remounted=true persisted_exact_b3=true OS_enforced_capacity_tested_separately=true",
        payload.len()
    );
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires intentionally unmounted isolated test directory"]
fn bounded_ext4_missing_mount_fails_closed() {
    use akula_roha_transport::bounded_staging::{StagingVolumeGuard, VolumeError};

    let root = std::path::PathBuf::from(
        std::env::var("AKULA_BOUNDED_EXT4_ROOT").expect("explicit unmounted lab root"),
    );
    assert!(matches!(
        StagingVolumeGuard::open(&root),
        Err(VolumeError::NotSeparateFilesystem)
    ));
    println!("P2P_BLOB_EXT4_MISSING_MOUNT refused=true no_fallback_to_host_disk=true");
}
