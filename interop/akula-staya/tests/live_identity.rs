#![cfg(feature = "p2p-live-identity")]
//! Real Iroh handshake: no simulated remote_id and no new OS credentials.
use akula_staya_integration::live_identity::{
    require_pinned_live_peer, with_protected_iroh_secret,
};
use aquilum_core::p2p_device_identity::IdentityError;
#[cfg(windows)]
use aquilum_core::p2p_device_identity::{self, peer_pin};
#[cfg(windows)]
use akula_staya_integration::roster_observation_lab::{
    ObservationError, observe_live_pinned_roster_peer,
};
#[cfg(windows)]
use aquilum_core::sync::{
    DeviceId, VaultId,
    device_roster_lab::{
        ClaimedDevice, IndependentlyPinnedOwner, OwnerSignedRoster, RosterStatus,
        inspect_signed_roster,
    },
};
#[cfg(windows)]
use ring::signature::{Ed25519KeyPair, KeyPair};
use iroh::{Endpoint, EndpointAddr, RelayMode, SecretKey, endpoint::presets};
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

// Explicit Windows OS identity → actual TLS peer → persisted confirmation
// and live revocation. Only a short-lived GitHub-hosted Windows runner may
// initialize canonical test OS credentials. Never mutate a developer machine.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protected_identity_accepts_confirmed_live_peer_then_rejects_revoke() {
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
        return;
    }
    assert_eq!(
        std::env::var("RUNNER_ENVIRONMENT").as_deref(),
        Ok("github-hosted"),
        "never mutate a long-lived Windows runner credential store"
    );
    assert_eq!(
        std::env::var("AKULA_P2P_OS_PIN_CI").as_deref(),
        Ok("1"),
        "test must be expressly armed by its dedicated CI step"
    );

    let local = match p2p_device_identity::load() {
        Ok(identity) => identity,
        Err(IdentityError::NotInitialized) => p2p_device_identity::initialize().unwrap(),
        Err(error) => panic!("OS credential cannot be loaded: {error}"),
    };
    let local_id = local.endpoint_id_hex();
    let persistent_secret = with_protected_iroh_secret(|secret| secret).unwrap();
    let server = Endpoint::builder(presets::Minimal)
        .secret_key(persistent_secret)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    assert_eq!(server.id().to_string(), local_id);
    let socket = server
        .bound_sockets()
        .into_iter()
        .find(|socket| socket.is_ipv4())
        .or_else(|| server.bound_sockets().into_iter().next())
        .expect("OS-backed QUIC UDP listener");
    let address = match socket {
        SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
    };
    let target =
        EndpointAddr::new(server.id()).with_ip_addr(SocketAddr::new(address, socket.port()));

    let outsider_key = SecretKey::generate();
    let outsider_id = outsider_key.public();
    let outsider_hex = outsider_id.to_string();
    let client = Endpoint::builder(presets::Minimal)
        .secret_key(outsider_key)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();

    let listener = tokio::spawn(async move {
        let incoming = tokio::time::timeout(Duration::from_secs(20), server.accept())
            .await
            .expect("incoming OS-backed QUIC deadline")
            .expect("incoming connection exists");
        let pending = incoming.accept().expect("TLS handshake accepted");
        let live = tokio::time::timeout(Duration::from_secs(20), pending)
            .await
            .expect("authenticated QUIC handshake deadline")
            .expect("authenticated QUIC connection");
        assert_eq!(live.remote_id(), outsider_id);
        assert_eq!(
            require_pinned_live_peer(&live, &outsider_hex),
            Err(IdentityError::PeerNotFound),
            "a live, valid QUIC key alone has no permission"
        );

        // The same real TLS peer is also present in a separately witnessed,
        // owner-signed Vault roster. Membership alone must not bypass OS pin.
        let owner = Ed25519KeyPair::from_seed_unchecked(&[11; 32]).unwrap();
        let vault_id = VaultId::new();
        let device_id = DeviceId::new();
        let mut roster = OwnerSignedRoster {
            vault_id,
            epoch: 7,
            devices: vec![ClaimedDevice {
                device_id,
                device_public_key: *outsider_id.as_bytes(),
                endpoint_public_key: *outsider_id.as_bytes(),
                status: RosterStatus::RecordedOnly,
            }],
            signature: [0; 64],
        };
        let claim = roster.canonical_claim().unwrap();
        let signature = owner.sign(&claim);
        roster.signature.copy_from_slice(signature.as_ref());
        let pin = IndependentlyPinnedOwner {
            vault_id,
            owner_public_key: owner.public_key().as_ref().try_into().unwrap(),
            witnessed_min_epoch: roster.epoch,
            witnessed_epoch_digest: Some(roster.claim_digest().unwrap()),
        };
        let verified = inspect_signed_roster(&roster, &pin).unwrap();
        assert_eq!(
            observe_live_pinned_roster_peer(&live, &outsider_hex, &verified, vault_id, device_id),
            Err(ObservationError::OsPinNotConfirmed),
            "a signed roster cannot self-authorize transport admission"
        );

        // CI synthetic peer confirmation is NOT a product human pairing.
        peer_pin::confirm_peer(&outsider_hex, &outsider_hex).unwrap();
        assert_eq!(require_pinned_live_peer(&live, &outsider_hex), Ok(()));
        let observed =
            observe_live_pinned_roster_peer(&live, &outsider_hex, &verified, vault_id, device_id)
                .unwrap();
        assert_eq!(observed.metadata_peer_binding().device_id, device_id);
        assert_eq!(
            observed.metadata_peer_binding().endpoint_id,
            *outsider_id.as_bytes()
        );
        assert_eq!(observed.vault_id(), vault_id);
        assert_eq!(observed.roster_epoch(), 7);

        peer_pin::revoke_peer(&outsider_hex).unwrap();
        assert_eq!(
            require_pinned_live_peer(&live, &outsider_hex),
            Err(IdentityError::PeerRevoked),
            "same live QUIC session cannot start a new operation after revoke"
        );
        assert_eq!(
            observe_live_pinned_roster_peer(&live, &outsider_hex, &verified, vault_id, device_id),
            Err(ObservationError::OsPinNotConfirmed),
            "a previously valid signed roster cannot bypass a later OS revoke"
        );
        live.close(0u8.into(), b"revoke checked");
        server.close().await;
    });

    let outgoing = tokio::time::timeout(Duration::from_secs(20), client.connect(target, ALPN))
        .await
        .expect("outgoing QUIC deadline")
        .expect("outgoing QUIC handshake");
    assert_eq!(outgoing.remote_id().to_string(), local_id);
    listener.await.unwrap();
    client.close().await;
}
