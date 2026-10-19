use akula_roha_transport::{AllowedPeers, StayaTransport};
use iroh::{EndpointAddr, SecretKey};
use std::time::Duration;

#[tokio::test(flavor = "current_thread")]
async fn permitted_peers_exchange_verified_bytes() {
    let a_key = SecretKey::generate();
    let b_key = SecretKey::generate();
    let a = StayaTransport::bind(a_key.clone(), AllowedPeers::new([b_key.public()]))
        .await
        .expect("bind A");
    let b = StayaTransport::bind(b_key.clone(), AllowedPeers::new([a_key.public()]))
        .await
        .expect("bind B");
    let address: EndpointAddr = b.address();
    let expected = vec![0xA5; 128 * 1024];
    let expected_reply = b"verified-receipt".to_vec();

    let (receipt_tx, receipt_rx) = tokio::sync::oneshot::channel::<()>();
    let run = async {
        let receiver = async {
            let conn = b.accept_one().await.expect("accept authenticated peer");
            let (mut send, mut recv) = conn.accept_bi().await.expect("accept QUIC stream");
            let received = recv.read_to_end(256 * 1024).await.expect("receive bytes");
            assert_eq!(received, expected);
            send.write_all(&expected_reply).await.expect("send reply");
            send.finish().expect("finish reply");
            receipt_rx.await.expect("sender received receipt");
        };
        let sender = async {
            let conn = a.connect(address).await.expect("connect to permitted peer");
            let (mut send, mut recv) = conn.open_bi().await.expect("open QUIC stream");
            send.write_all(&vec![0xA5; 128 * 1024])
                .await
                .expect("send bytes");
            send.finish().expect("finish stream");
            let answer = recv.read_to_end(1024).await.expect("read reply");
            assert_eq!(answer, b"verified-receipt");
            receipt_tx.send(()).expect("receiver remains alive");
        };
        tokio::join!(receiver, sender);
    };
    tokio::time::timeout(Duration::from_secs(20), run)
        .await
        .expect("complete transfer");
    a.close().await;
    b.close().await;
}

#[tokio::test(flavor = "current_thread")]
async fn outbound_connection_rejects_unknown_peer_before_network_io() {
    let a = StayaTransport::bind(SecretKey::generate(), AllowedPeers::default())
        .await
        .expect("bind");
    let remote = EndpointAddr::new(SecretKey::generate().public());
    assert_eq!(a.connect(remote).await.unwrap_err(), "peer not admitted");
    a.close().await;
}

#[tokio::test(flavor = "current_thread")]
async fn inbound_unknown_peer_is_closed_without_blocking_admitted_peer() {
    let server_key = SecretKey::generate();
    let admitted_key = SecretKey::generate();
    let stranger_key = SecretKey::generate();

    // These are network admission lists, not permissions to read or write files.
    let server = StayaTransport::bind(
        server_key.clone(),
        AllowedPeers::new([admitted_key.public()]),
    )
    .await
    .expect("bind server");
    let admitted = StayaTransport::bind(
        admitted_key.clone(),
        AllowedPeers::new([server_key.public()]),
    )
    .await
    .expect("bind admitted client");
    let stranger = StayaTransport::bind(
        stranger_key,
        AllowedPeers::new([server_key.public()]),
    )
    .await
    .expect("bind unknown client");
    let server_addr = server.address();

    tokio::time::timeout(Duration::from_secs(20), async {
        let incoming = server.accept_one();
        let clients = async {
            // The stranger knows the server identity and can attempt a valid
            // QUIC handshake, but the server has not admitted the stranger.
            // Once the server notices that identity, the connection must close.
            if let Ok(unadmitted) = stranger.connect(server_addr.clone()).await {
                tokio::time::timeout(Duration::from_secs(5), unadmitted.closed())
                    .await
                    .expect("unadmitted QUIC session must be closed");
            }

            // An unauthorized attempt must not consume the admission slot:
            // the next authorized client must still be accepted normally.
            admitted
                .connect(server_addr)
                .await
                .expect("admitted client can still connect")
        };

        let (server_connection, _client_connection) = tokio::join!(incoming, clients);
        let server_connection = server_connection.expect("admitted incoming connection");
        assert_eq!(server_connection.remote_id(), admitted_key.public());
    })
    .await
    .expect("reject unknown then accept known within bounded time");

    stranger.close().await;
    admitted.close().await;
    server.close().await;
}
