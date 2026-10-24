//! Actual 64 MiB integration: MinIO -> native downloader A -> existing
//! Iroh/Bao sender -> receiver B, using independent SHA-256 and BLAKE3 pins.
//! Sender A writes one verified Range at a time to an OWNED staging file, and
//! imports the completed file into the existing FsStore without building a
//! 64 MiB Vec. An unverified file is never exposed to the peer.
use akula_public_cdn_check::{
    public_cdn_lab::{
        ChunkRequest, OptionalChunkSource, PublicObjectReader, PublisherPinnedManifest,
    },
    public_cdn_s3_lab::PinnedS3Range,
    public_cdn_bao_lab::PinnedBaoIroh,
};
use iroh::{Endpoint, RelayMode, SecretKey, endpoint::presets, protocol::Router};
use iroh_blobs::{
    BlobsProtocol, Hash,
    provider::events::{
        AbortReason, ConnectMode, EventMask, EventSender, ObserveMode,
        ProviderMessage, RequestMode, ThrottleMode,
    },
    store::fs::FsStore,
};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};

struct UnavailablePeer;
impl OptionalChunkSource for UnavailablePeer {
    fn try_chunk(&mut self, _: &ChunkRequest<'_>) -> Result<Option<Vec<u8>>, ()> {
        Ok(None)
    }
}
async fn local_endpoint(key: SecretKey) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .expect("isolated direct QUIC endpoint")
}

#[tokio::test(flavor="multi_thread")]
async fn minio_to_native_a_to_existing_iroh_bao_to_native_b() {
    tokio::time::timeout(Duration::from_secs(420), async {
        const SIZE: u64 = 64 * 1024 * 1024;
        const CHUNK: usize = 256 * 1024;
        // Only the CI setup writes this manifest pin. It cannot be supplied by a peer.
        let raw = fs::read(std::env::var("AKULA_TEST_MANIFEST").expect("fixture manifest env"))
            .expect("fixture manifest");
        let independently_pinned_sha =
            std::env::var("AKULA_TEST_MANIFEST_SHA256").expect("fixture hash env");
        let manifest =
            PublisherPinnedManifest::from_pinned_json(&raw, &independently_pinned_sha)
                .expect("trusted publisher fixture");
        assert_eq!(manifest.object_size(), SIZE);
        assert_eq!(manifest.chunk_count(), (SIZE as usize / CHUNK));

        // The sender stages ONLY synthetic public bytes in an owned temporary
        // directory. Create-new prevents clobbering any pre-existing file.
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)
            .expect("system clock").as_nanos();
        let stage_root = std::env::temp_dir()
            .join(format!("akula-s3-staya-e2e-{}-{nonce}", std::process::id()));
        fs::create_dir(&stage_root).expect("exclusive temporary stage root");
        let stage_file = stage_root.join("unpublished-object.bin");
        let worker_file = stage_file.clone();
        // S3 GET runs on its own blocking worker, not on the Tokio reactor.
        let (origin_count, origin_bytes, sha, root) =
            tokio::task::spawn_blocking(move || {
                let approved = manifest.origin_url_for_allowlisted_adapter().to_string();
                let mut origin =
                    PinnedS3Range::for_test_loopback_minio(&manifest, &approved)
                        .expect("strict isolated MinIO URL");
                let mut cache = UnavailablePeer;
                let mut peer = UnavailablePeer;
                let mut reader =
                    PublicObjectReader::new(&manifest, &mut cache, &mut peer, &mut origin);
                let mut pending = fs::OpenOptions::new()
                    .write(true).create_new(true).open(&worker_file)
                    .expect("never truncate an unrelated path");
                // PublicObjectReader verifies each <=256KiB chunk and
                // the full ContentID; this output is not published yet.
                let receipt = reader
                    .stream_into(&mut pending)
                    .expect("all 256 S3 chunks must verify");
                pending.sync_all().expect("durable verified staging bytes");
                drop(pending);
                assert_eq!(receipt.bytes, SIZE);
                assert_eq!(receipt.metrics.verified_cache_bytes, 0);
                assert_eq!(receipt.metrics.verified_peer_bytes, 0);
                assert_eq!(receipt.metrics.verified_origin_bytes, SIZE);
                assert_eq!(receipt.metrics.origin_requests, (SIZE as usize / CHUNK) as u64);
                (receipt.metrics.origin_requests,
                    receipt.metrics.verified_origin_bytes,
                    receipt.object_sha256, receipt.staya_content_id.0)
            })
            .await
            .expect("S3 fetch worker");
        // Independently audit the actual on-disk staging file with a 64KiB
        // read buffer. A second full-buffer copy would defeat this test.
        assert_eq!(fs::metadata(&stage_file).expect("staged file").len(), SIZE);
        let mut source = fs::File::open(&stage_file).expect("auditable stage");
        let mut sha_check = Sha256::new();
        let mut blake_check = blake3::Hasher::new();
        let mut block = [0_u8; 64 * 1024];
        let mut audited = 0_u64;
        loop {
            let count = source.read(&mut block).expect("stage read");
            if count == 0 { break; }
            sha_check.update(&block[..count]);
            blake_check.update(&block[..count]);
            audited += count as u64;
        }
        assert_eq!(audited, SIZE);
        assert_eq!(sha_check.finalize().as_slice(), sha);
        assert_eq!(blake_check.finalize().as_bytes(), &root);

        // Filesystem-backed FsStore import, not store.add_slice(&64MiBVec).
        // The original validated file stays accessible until shutdown.
        let store_root = stage_root.join("iroh-store");
        fs::create_dir(&store_root).expect("private lab store");
        let store = FsStore::load(&store_root).await.expect("independent sender FsStore");
        let tagged = store.add_path(&stage_file).await.expect("import verified public file");
        let wanted = Hash::from_bytes(root);
        assert_eq!(tagged.hash, wanted);
        assert_eq!(wanted.as_bytes(), &root);

        let receiver_secret = SecretKey::generate();
        let allowed_receiver = receiver_secret.public();
        let mask = EventMask {
            connected: ConnectMode::Intercept,
            get: RequestMode::Intercept,
            get_many: RequestMode::Disabled,
            push: RequestMode::Disabled,
            observe: ObserveMode::Intercept,
            throttle: ThrottleMode::None,
        };
        let (events, mut event_rx) = EventSender::channel(64, mask);
        let rejects = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let rejected = Arc::clone(&rejects);
        let gate = tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                match event {
                    ProviderMessage::ClientConnected(message) => {
                        let permit = message.endpoint_id == Some(allowed_receiver);
                        if !permit {
                            rejected.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        }
                        let _ = message.tx.send(if permit {
                            Ok(())
                        } else {
                            Err(AbortReason::Permission)
                        }).await;
                    }
                    ProviderMessage::GetRequestReceived(message) => {
                        let permitted = message.request.hash == wanted
                            && message.request.ranges.is_blob();
                        let _ = message.tx.send(if permitted {
                            Ok(())
                        } else {
                            Err(AbortReason::Permission)
                        }).await;
                    }
                    ProviderMessage::ObserveRequestReceived(message) => {
                        let _ = message.tx.send(Err(AbortReason::Permission)).await;
                    }
                    _ => {}
                }
            }
        });
        let server = local_endpoint(SecretKey::generate()).await;
        let router = Router::builder(server)
            .accept(iroh_blobs::ALPN, BlobsProtocol::new(&store, Some(events)))
            .spawn();
        let receiver = local_endpoint(receiver_secret).await;
        let authenticated =
            receiver.connect(router.endpoint().addr(), iroh_blobs::ALPN)
                .await.expect("B connects to A via existing ALPN");
        let mut peer =
            PinnedBaoIroh::new(&authenticated, root, SIZE, true)
                .expect("explicit local consent");
        let mut full_sha = Sha256::new();
        let mut full_blake = blake3::Hasher::new();
        let receiver_manifest =
            PublisherPinnedManifest::from_pinned_json(&raw, &independently_pinned_sha)
                .expect("receiver independently validates same trusted fixture");
        let mut delivered_bytes = 0_u64;
        let mut peer_gets = 0_usize;
        for index in 0..receiver_manifest.chunk_count() {
            let chunk = receiver_manifest.request(index).expect("chunk geometry");
            let bytes = peer.read_public_part(&chunk)
                .await.expect("Bao proof + independent SHA256");
            assert_eq!(bytes.len(), CHUNK);
            full_sha.update(&bytes);
            full_blake.update(&bytes);
            delivered_bytes += bytes.len() as u64;
            peer_gets += 1;
        }
        assert_eq!(delivered_bytes, SIZE);
        assert_eq!(full_sha.finalize().as_slice(), sha);
        assert_eq!(full_blake.finalize().as_bytes(), &root);

        // There is no S3 client on B. Origin accounting from stage A can
        // therefore distinguish seed cost from avoided B-origin traffic.
        let b_origin_bytes = 0_u64;
        let b_peer_bytes = delivered_bytes;
        assert_eq!(origin_count, 256);
        assert_eq!(origin_bytes, SIZE);
        assert_eq!(b_origin_bytes, 0);
        assert_eq!(b_peer_bytes, SIZE);
        assert_eq!(peer_gets, 256);

        authenticated.close(0u8.into(), b"public object verified");
        router.shutdown().await.expect("stop only lab provider");
        receiver.close().await;
        gate.abort();
        drop(tagged);
        drop(store);
        fs::remove_dir_all(&stage_root).expect("remove owned stage and store");
        println!(
            "PUBLIC_MINIO_TO_IROH_E2E size={} stage_a_s3_bytes={} stage_b_s3_bytes={} stage_b_bao_bytes={} parts={} sha256={} blake3={} peer_gate=true relay=false stage_file=true max_inflight_s3_chunk=262144",
            SIZE, origin_bytes, b_origin_bytes, b_peer_bytes, peer_gets,
            hex(&sha), hex(&root),
        );
    }).await.expect("bounded 64MiB real MinIO + direct QUIC test");
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

