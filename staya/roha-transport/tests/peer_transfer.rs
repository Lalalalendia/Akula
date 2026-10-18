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

    let run = async {
        let receiver = async {
            let conn = b.accept_one().await.expect("accept authenticated peer");
            let (mut send, mut recv) = conn.accept_bi().await.expect("accept QUIC stream");
            let received = recv.read_to_end(256 * 1024).await.expect("receive bytes");
            assert_eq!(received, expected);
            send.write_all(&expected_reply).await.expect("send reply");
            send.finish().expect("finish reply");
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
