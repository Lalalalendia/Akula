//! A real completed Iroh QUIC connection, signed roster and exact read grant.
//! No preview-supplied peer IDs or plaintext network fallback.
#![cfg(all(target_os = "linux", feature = "node-live-read-lab"))]

use akula_staya_integration::node_live_read::{
    NodeReadDenied, OneUseChallengeFence, read_from_live_iroh_peer,
};
use aquilum_core::node_replica_auth_lab::{
    DeviceReadProof, IndependentReadTrust, OwnerReadGrant, device_challenge_claim, owner_claim,
    signed_grant_digest,
};
use aquilum_core::node_replica_lab::{OperatorFixtureAdmission, stage_operator_fixture};
use aquilum_core::sync::device_roster_lab::{
    ClaimedDevice, IndependentlyPinnedOwner, OwnerSignedRoster, RosterStatus,
};
use aquilum_core::sync::{DeviceId, DocumentId, RevisionId, VaultId};
use iroh::endpoint::{Connection, Endpoint, presets};
use iroh::{EndpointAddr, RelayMode, SecretKey};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::collections::HashSet;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

const ALPN: &[u8] = b"akula-node-live-id-lab/v1";

#[derive(Default)]
struct TestFence(HashSet<([u8; 32], [u8; 32], [u8; 32])>);

impl OneUseChallengeFence for TestFence {
    fn consume_once(&mut self, peer: [u8; 32], grant: [u8; 32], challenge: [u8; 32]) -> bool {
        challenge != [0; 32] && self.0.insert((peer, grant, challenge))
    }
}

fn sign_roster(mut roster: OwnerSignedRoster, owner: &Ed25519KeyPair) -> OwnerSignedRoster {
    roster.signature = owner
        .sign(&roster.canonical_claim().unwrap())
        .as_ref()
        .try_into()
        .unwrap();
    roster
}

async fn establish(server: &Endpoint, secret: SecretKey) -> (Endpoint, Connection, Connection) {
    let client = Endpoint::builder(presets::Minimal)
        .secret_key(secret)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
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
    let target = EndpointAddr::new(server.id()).with_ip_addr(SocketAddr::new(ip, socket.port()));
    let (dialed, inbound) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(20), client.connect(target, ALPN)),
        async {
            let incoming = tokio::time::timeout(Duration::from_secs(20), server.accept())
                .await
                .expect("incoming QUIC deadline")
                .expect("incoming QUIC connection");
            let handshake = incoming.accept().expect("TLS handshake");
            tokio::time::timeout(Duration::from_secs(20), handshake)
                .await
                .expect("TLS handshake deadline")
        },
    );
    (client, dialed.unwrap().unwrap(), inbound.unwrap())
}

fn state_and_grants(
    root: &Path,
    device_seed: [u8; 32],
    owner: &Ed25519KeyPair,
) -> (
    PathBuf,
    Vec<u8>,
    OwnerSignedRoster,
    IndependentlyPinnedOwner,
    OwnerReadGrant,
    IndependentReadTrust,
    DeviceReadProof,
) {
    let source = root.join("source");
    let state = root.join("node-state");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let original = b"# Node authorized immutable replica\n".to_vec();
    fs::write(source.join("note.md"), &original).unwrap();

    let vault_id = VaultId::from_uuid(Uuid::from_u128(11));
    let document_id = DocumentId::from_uuid(Uuid::from_u128(22));
    let revision_id = RevisionId::from_uuid(Uuid::from_u128(33));
    let device_id = DeviceId::from_uuid(Uuid::from_u128(44));
    let digest = blake3::hash(&original).to_hex().to_string();
    stage_operator_fixture(
        &source,
        &state,
        &OperatorFixtureAdmission {
            vault_id,
            document_id,
            revision_id,
            filename: "note.md".into(),
            expected_blake3: digest.clone(),
        },
    )
    .unwrap();
    fs::remove_file(source.join("note.md")).unwrap();

    let device_key = Ed25519KeyPair::from_seed_unchecked(&device_seed).unwrap();
    let endpoint = *SecretKey::from_bytes(&device_seed).public().as_bytes();
    assert_eq!(device_key.public_key().as_ref(), endpoint.as_slice());

    let roster = sign_roster(
        OwnerSignedRoster {
            vault_id,
            epoch: 7,
            devices: vec![ClaimedDevice {
                device_id,
                device_public_key: endpoint,
                endpoint_public_key: endpoint,
                status: RosterStatus::RecordedOnly,
            }],
            signature: [0; 64],
        },
        owner,
    );
    let roster_pin = IndependentlyPinnedOwner {
        vault_id,
        owner_public_key: owner.public_key().as_ref().try_into().unwrap(),
        witnessed_min_epoch: 7,
        witnessed_epoch_digest: Some(roster.claim_digest().unwrap()),
    };
    let mut grant = OwnerReadGrant {
        vault_id,
        document_id,
        revision_id,
        expected_blake3: digest,
        filename: "note.md".into(),
        node_identity: [0x71; 32],
        device_id,
        device_public_key: endpoint,
        grant_epoch: 7,
        owner_signature: [0; 64],
    };
    grant.owner_signature = owner
        .sign(&owner_claim(&grant).unwrap())
        .as_ref()
        .try_into()
        .unwrap();
    let grant_pin = IndependentReadTrust {
        trusted_vault_id: vault_id,
        trusted_owner_public_key: owner.public_key().as_ref().try_into().unwrap(),
        trusted_node_identity: grant.node_identity,
        witnessed_current_epoch: 7,
        witnessed_exact_grant_digest: signed_grant_digest(&grant).unwrap(),
        current_session_challenge: [0x91; 32],
    };
    let proof = DeviceReadProof {
        device_signature: device_key
            .sign(&device_challenge_claim(&grant, [0x91; 32]).unwrap())
            .as_ref()
            .try_into()
            .unwrap(),
    };
    (state, original, roster, roster_pin, grant, grant_pin, proof)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_live_quic_remote_id_then_roster_then_exact_replica() {
    let temp = tempfile::tempdir().unwrap();
    let owner = Ed25519KeyPair::from_seed_unchecked(&[12; 32]).unwrap();
    let device_seed = [21u8; 32];
    let (state, original, roster, roster_pin, grant, grant_pin, proof) =
        state_and_grants(temp.path(), device_seed, &owner);

    let server = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::from_bytes(&[3; 32]))
        .relay_mode(RelayMode::Disabled)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .unwrap();
    let (_device, _outbound, inbound) =
        establish(&server, SecretKey::from_bytes(&device_seed)).await;
    assert_eq!(
        *inbound.remote_id().as_bytes(),
        grant.device_public_key,
        "authenticated transport key must equal signed owner roster key"
    );

    let mut fence = TestFence::default();
    assert_eq!(
        read_from_live_iroh_peer(
            &inbound,
            &roster,
            &roster_pin,
            &grant,
            &grant_pin,
            &proof,
            &mut fence,
            &state,
        ),
        Ok(original),
    );
    assert_eq!(
        read_from_live_iroh_peer(
            &inbound,
            &roster,
            &roster_pin,
            &grant,
            &grant_pin,
            &proof,
            &mut fence,
            &state,
        ),
        Err(NodeReadDenied),
        "same authenticated connection and challenge must never replay"
    );

    // Another real authenticated QUIC peer cannot adopt a valid DeviceId by
    // replaying the signed roster, read grant, and exact device signature.
    let (_rogue, _rogue_outbound, rogue_connection) =
        establish(&server, SecretKey::from_bytes(&[33; 32])).await;
    assert_ne!(rogue_connection.remote_id(), inbound.remote_id());
    assert_eq!(
        read_from_live_iroh_peer(
            &rogue_connection,
            &roster,
            &roster_pin,
            &grant,
            &grant_pin,
            &proof,
            &mut TestFence::default(),
            &state,
        ),
        Err(NodeReadDenied)
    );

    // Signed membership is NOT a File/Vault permission. Revoked rows remain
    // denied even when the independent witness pins the new valid signature.
    let mut revoked = roster.clone();
    revoked.devices[0].status = RosterStatus::Revoked;
    let revoked = sign_roster(revoked, &owner);
    let mut revoked_pin = roster_pin;
    revoked_pin.witnessed_epoch_digest = Some(revoked.claim_digest().unwrap());
    assert_eq!(
        read_from_live_iroh_peer(
            &inbound,
            &revoked,
            &revoked_pin,
            &grant,
            &grant_pin,
            &proof,
            &mut TestFence::default(),
            &state,
        ),
        Err(NodeReadDenied)
    );

    let mut stale = roster_pin;
    stale.witnessed_min_epoch = 8;
    assert_eq!(
        read_from_live_iroh_peer(
            &inbound,
            &roster,
            &stale,
            &grant,
            &grant_pin,
            &proof,
            &mut TestFence::default(),
            &state,
        ),
        Err(NodeReadDenied),
        "older valid roster cannot restore access"
    );

    let mut wrong_revision = grant.clone();
    wrong_revision.revision_id = RevisionId::new();
    assert_eq!(
        read_from_live_iroh_peer(
            &inbound,
            &roster,
            &roster_pin,
            &wrong_revision,
            &grant_pin,
            &proof,
            &mut TestFence::default(),
            &state,
        ),
        Err(NodeReadDenied),
        "a different revision never inherits a signed read grant"
    );
    println!("NODE_LIVE_ID_01_QUIC_ROSTER_EXACT_READ=true production=false");
}
