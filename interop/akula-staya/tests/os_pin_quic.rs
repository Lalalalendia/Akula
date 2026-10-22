//! Real QUIC proof with the OS-persisted local identity.
//! Synthetic peer confirmation runs only in a disposable hosted CI account.
#[cfg(windows)]
use akula_staya_integration::os_peer::{OS_PIN_ALPN, OsPinnedEndpoint};
#[cfg(windows)]
use aquilum_core::p2p_device_identity::{self, IdentityError, peer_pin};
#[cfg(windows)]
use iroh::{Endpoint, RelayMode, SecretKey, endpoint::presets};
#[cfg(windows)]
use std::time::Duration;

#[cfg(not(windows))]
#[test]
fn unsupported_os_denies_identity() {
    assert!(matches!(
        aquilum_core::p2p_device_identity::load(),
        Err(aquilum_core::p2p_device_identity::IdentityError::UnsupportedPlatform)
    ));
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn os_key_live_quic_and_persistent_revoke() {
    if !matches!(std::env::var("AKULA_P2P_OS_PIN_CI").as_deref(), Ok("1")) {
        return;
    }
    let identity = match p2p_device_identity::load() {
        Ok(identity) => identity,
        Err(IdentityError::NotInitialized) => p2p_device_identity::initialize().unwrap(),
        Err(error) => panic!("OS credential unavailable: {error}"),
    };
    let server = OsPinnedEndpoint::bind_loopback_lab().await.unwrap();
    assert_eq!(server.id().as_bytes(), &identity.endpoint_public_bytes());

    let peer_secret = SecretKey::generate();
    let peer_id = peer_secret.public();
    let peer_hex = peer_id
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let client = Endpoint::builder(presets::Minimal)
        .secret_key(peer_secret)
        .alpns(vec![OS_PIN_ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .bind()
        .await
        .unwrap();
    assert!(matches!(
        peer_pin::require_pinned_remote(&peer_hex, peer_id.as_bytes()),
        Err(IdentityError::PeerNotFound)
    ));
    assert!(server.accept_selected(&peer_hex).await.is_err());
    // Test-only, not production human pairing: exact remote ID was generated
    // by this CI process, not discovered through an untrusted message.
    peer_pin::confirm_peer(&peer_hex, &peer_hex).unwrap();

    let target = server.loopback_address_lab().unwrap();
    let (accepted, connected) = tokio::time::timeout(Duration::from_secs(20), async {
        tokio::join!(
            server.accept_selected(&peer_hex),
            client.connect(target, OS_PIN_ALPN)
        )
    })
    .await
    .expect("live QUIC handshake deadline");
    let accepted = accepted.unwrap();
    let connected = connected.unwrap();
    assert_eq!(accepted.remote_id(), peer_id);
    assert_eq!(connected.remote_id(), server.id());

    let (mut tx, mut rx) = connected.open_bi().await.unwrap();
    tx.write_all(b"AKULA_HELLO_V1").await.unwrap();
    tx.finish().unwrap();
    let (mut reply, mut request) = accepted
        .authorize_new_operation()
        .unwrap()
        .accept_bi()
        .await
        .unwrap();
    assert_eq!(request.read_to_end(32).await.unwrap(), b"AKULA_HELLO_V1");
    reply.write_all(b"AKULA_OK_V1").await.unwrap();
    reply.finish().unwrap();
    assert_eq!(rx.read_to_end(32).await.unwrap(), b"AKULA_OK_V1");

    peer_pin::revoke_peer(&peer_hex).unwrap();
    assert!(matches!(
        peer_pin::peer_state(&peer_hex),
        Ok(peer_pin::PeerState::Revoked)
    ));
    assert!(accepted.authorize_new_operation().is_err());
    client.close().await;
    server.close().await;
}
