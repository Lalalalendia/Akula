#![cfg(feature = "p2p-live-identity")]
//! Real Iroh handshake: no simulated remote_id and no new OS credentials.
use akula_staya_integration::live_identity::{
    require_pinned_live_peer, with_protected_iroh_secret,
};
use aquilum_core::p2p_device_identity::IdentityError;
use iroh::{endpoint::presets, Endpoint, EndpointAddr, RelayMode, SecretKey};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

const ALPN: &[u8] = b"akula/p2p-live-pin/lab/1";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_genuine_quic_identity_cannot_create_its_own_os_trust() {
    let server = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let server_id = server.id();
    let socket = server
        .bound_sockets()
        .into_iter()
        .find(|socket| socket.is_ipv4())
        .or_else(|| server.bound_sockets().into_iter().next())
        .expect("bound Iroh QUIC UDP socket");
    let ip = match socket {
        SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
    };
    let addr = EndpointAddr::new(server_id).with_ip_addr(SocketAddr::new(ip, socket.port()));
    let client = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::generate())
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let client_id = client.id();
    let server_task = tokio::spawn(async move {
        let incoming = tokio::time::timeout(Duration::from_secs(20), server.accept())
            .await
            .unwrap()
            .expect("server waiting for handshake");
        let handshake = incoming.accept().expect("real TLS connection");
        let connection = tokio::time::timeout(Duration::from_secs(20), handshake)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(connection.remote_id(), client_id);
        connection.close(0u8.into(), b"lab complete");
        server.close().await;
    });
    let connection = tokio::time::timeout(Duration::from_secs(20), client.connect(addr, ALPN))
        .await
        .expect("direct QUIC timeout")
        .expect("real authenticated QUIC connection");
    assert_eq!(connection.remote_id(), server_id);
    assert_eq!(
        require_pinned_live_peer(&connection, &client_id.to_string()),
        Err(IdentityError::PeerMismatch)
    );
    assert!(
        require_pinned_live_peer(&connection, &server_id.to_string()).is_err(),
        "a randomly generated unconfirmed peer must never be admitted"
    );
    connection.close(0u8.into(), b"lab complete");
    client.close().await;
    server_task.await.unwrap();
}

#[cfg(not(windows))]
#[test]
fn missing_os_backend_does_not_fall_back_to_plaintext_secret() {
    assert!(matches!(
        with_protected_iroh_secret(|_| ()),
        Err(IdentityError::UnsupportedPlatform)
    ));
}

#[cfg(windows)]
#[test]
fn existing_protected_seed_if_present_is_identical_to_iroh_key() {
    match with_protected_iroh_secret(|key| key.public().to_string()) {
        Ok(public) => assert_eq!(
            public,
            aquilum_core::p2p_device_identity::load()
                .unwrap()
                .endpoint_id_hex()
        ),
        Err(IdentityError::NotInitialized) => {}
        Err(error) => panic!("OS-protected identity cannot be used: {error}"),
    }
}
