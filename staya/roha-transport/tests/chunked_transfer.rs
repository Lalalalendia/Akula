use akula_roha_transport::transfer::{
    FRAME_HEADER_BYTES, MAX_CHUNK_BYTES, TransferManifest, VerifiedAssembly,
};
use akula_roha_transport::{AllowedPeers, StayaTransport};
use iroh::SecretKey;
use std::time::Duration;

#[tokio::test(flavor = "current_thread")]
async fn multiple_files_and_reconnect_resume_verified_chunks() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let key_a = SecretKey::generate();
        let key_b = SecretKey::generate();
        let a = StayaTransport::bind(key_a.clone(), AllowedPeers::new([key_b.public()]))
            .await
            .unwrap();
        let b = StayaTransport::bind(key_b, AllowedPeers::new([key_a.public()]))
            .await
            .unwrap();

        for payload in [vec![], vec![7], vec![0x5a; MAX_CHUNK_BYTES * 3 + 13]] {
            let manifest = TransferManifest::for_bytes(&payload).unwrap();
            let mut assembly = VerifiedAssembly::new(manifest).unwrap();
            let mut connection_index = 0;
            let mut cursor = 0usize;
            loop {
                let (incoming, outgoing) =
                    tokio::try_join!(b.accept_one(), a.connect(b.address())).unwrap();
                for _ in 0..2 {
                    if cursor >= payload.len() && !(payload.is_empty() && cursor == 0) {
                        break;
                    }
                    let end = (cursor + MAX_CHUNK_BYTES).min(payload.len());
                    let data = &payload[cursor..end];
                    let frame = manifest.encode_chunk(cursor as u64, data).unwrap();
                    let (receiver, sender) = tokio::join!(
                        async {
                            let (mut tx, mut rx) = incoming.accept_bi().await.unwrap();
                            let data = rx
                                .read_to_end(MAX_CHUNK_BYTES + FRAME_HEADER_BYTES)
                                .await
                                .unwrap();
                            let next = assembly.accept_frame(&data).unwrap();
                            tx.write_all(&next.to_le_bytes()).await.unwrap();
                            tx.finish().unwrap();
                            next
                        },
                        async {
                            let (mut tx, mut rx) = outgoing.open_bi().await.unwrap();
                            tx.write_all(&frame).await.unwrap();
                            tx.finish().unwrap();
                            let result = rx.read_to_end(8).await.unwrap();
                            u64::from_le_bytes(result.try_into().unwrap())
                        }
                    );
                    assert_eq!(sender, receiver);
                    cursor = end;
                    if assembly.complete_bytes().is_some() {
                        break;
                    }
                }
                outgoing.close(0u8.into(), b"switch connection");
                drop(outgoing);
                drop(incoming);
                connection_index += 1;
                if assembly.complete_bytes().is_some() {
                    break;
                }
                assert!(connection_index < 10, "bounded reconnects");
            }
            assert_eq!(assembly.complete_bytes(), Some(payload.as_slice()));
            if payload.len() > MAX_CHUNK_BYTES * 2 {
                assert!(connection_index >= 2, "large file required new connection");
            }
        }
        a.close().await;
        b.close().await;
    })
    .await
    .expect("bounded transfer and reconnect");
}
