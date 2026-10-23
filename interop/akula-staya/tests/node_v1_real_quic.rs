//! Exact live Iroh QUIC A -> Node -> B with the original source removed.
//! Uses temporary owner-controlled trust only via debug-only test feature.
//! Never provision /etc, root helper, external relay, or a public port.
#![cfg(all(target_os = "linux", feature = "node-v1-e2e-tests"))]

use akula_staya_integration::node_v1_runtime::{
    NodeWireDenied, get_over_authenticated_iroh, put_over_authenticated_iroh,
    serve_one_authenticated_connection,
};
use aquilum_core::node_replica_auth_lab::{OwnerReadGrant, owner_claim, signed_grant_digest};
use aquilum_core::node_v1::{NodeHost, NodeIdentity, OwnerPutGrant, put_claim, signed_put_digest};
use aquilum_core::sync::device_roster_lab::{ClaimedDevice, OwnerSignedRoster, RosterStatus};
use aquilum_core::sync::{DeviceId, DocumentId, RevisionId, VaultId};
use iroh::endpoint::{Connection, Endpoint, presets};
use iroh::{EndpointAddr, RelayMode, SecretKey};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const ALPN: &[u8] = b"akula-node-v1/quic/v1";
const CONTENT: &[u8] = b"# Akula NodeV1 owner-signed replica\nA to VPS to B, source offline\n";

fn sign_roster(mut roster: OwnerSignedRoster, owner: &Ed25519KeyPair) -> OwnerSignedRoster {
    roster.signature = owner
        .sign(&roster.canonical_claim().unwrap())
        .as_ref()
        .try_into()
        .unwrap();
    roster
}

struct Fixture {
    state: PathBuf,
    policy: PathBuf,
    node_public: [u8; 32],
    roster: OwnerSignedRoster,
    put: OwnerPutGrant,
    read: OwnerReadGrant,
    a: Ed25519KeyPair,
    b: Ed25519KeyPair,
}

fn fixture(root: &Path) -> Fixture {
    let state = root.join("state");
    let policy = root.join("policy");
    for path in [&state, &policy] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let node_public = NodeIdentity::open_or_create(&state).unwrap().public_key();
    let owner = Ed25519KeyPair::from_seed_unchecked(&[12; 32]).unwrap();
    let a = Ed25519KeyPair::from_seed_unchecked(&[21; 32]).unwrap();
    let b = Ed25519KeyPair::from_seed_unchecked(&[22; 32]).unwrap();
    let a_endpoint = *SecretKey::from_bytes(&[21; 32]).public().as_bytes();
    let b_endpoint = *SecretKey::from_bytes(&[22; 32]).public().as_bytes();
    assert_eq!(a.public_key().as_ref(), &a_endpoint);
    assert_eq!(b.public_key().as_ref(), &b_endpoint);
    let vault_id = VaultId::new();
    let doc = DocumentId::new();
    let rev = RevisionId::new();
    let a_device = DeviceId::new();
    let b_device = DeviceId::new();
    let mut roster = OwnerSignedRoster {
        vault_id,
        epoch: 9,
        devices: vec![
            ClaimedDevice {
                device_id: a_device,
                device_public_key: a_endpoint,
                endpoint_public_key: a_endpoint,
                status: RosterStatus::RecordedOnly,
            },
            ClaimedDevice {
                device_id: b_device,
                device_public_key: b_endpoint,
                endpoint_public_key: b_endpoint,
                status: RosterStatus::RecordedOnly,
            },
        ],
        signature: [0; 64],
    };
    // Canonical signed order is DeviceId order, not QUIC arrival order.
    roster.devices.sort_by_key(|device| device.device_id);
    let roster = sign_roster(roster, &owner);

    let hash = blake3::hash(CONTENT).to_hex().to_string();
    let mut put = OwnerPutGrant {
        vault_id,
        document_id: doc,
        revision_id: rev,
        filename: "sample.md".to_string(),
        expected_blake3: hash.clone(),
        node_identity: node_public,
        device_id: a_device,
        device_public_key: a_endpoint,
        grant_epoch: 9,
        owner_signature: [0; 64],
    };
    put.owner_signature = owner
        .sign(&put_claim(&put).unwrap())
        .as_ref()
        .try_into()
        .unwrap();
    let mut read = OwnerReadGrant {
        vault_id,
        document_id: doc,
        revision_id: rev,
        filename: "sample.md".to_string(),
        expected_blake3: hash,
        node_identity: node_public,
        device_id: b_device,
        device_public_key: b_endpoint,
        grant_epoch: 9,
        owner_signature: [0; 64],
    };
    read.owner_signature = owner
        .sign(&owner_claim(&read).unwrap())
        .as_ref()
        .try_into()
        .unwrap();
    let owner_public: [u8; 32] = owner.public_key().as_ref().try_into().unwrap();
    let config = serde_json::json!({
        "version": 2,
        "node_public": node_public,
        "owner_public": owner_public,
        "vault_id": vault_id,
        "grant_epoch": 9,
        "roster_digest": roster.claim_digest().unwrap(),
        "grant_digest": signed_grant_digest(&read).unwrap(),
        "put_grant_digest": signed_put_digest(&put).unwrap(),
    });
    let file = policy.join("owner-policy.json");
    fs::write(&file, serde_json::to_vec(&config).unwrap()).unwrap();
    fs::set_permissions(file, fs::Permissions::from_mode(0o600)).unwrap();
    Fixture {
        state,
        policy,
        node_public,
        roster,
        put,
        read,
        a,
        b,
    }
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
        .unwrap();
    let ip = match socket {
        SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
    };
    let target = EndpointAddr::new(server.id()).with_ip_addr(SocketAddr::new(ip, socket.port()));
    let (outbound, inbound) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(20), client.connect(target, ALPN),),
        async {
            let incoming = tokio::time::timeout(Duration::from_secs(20), server.accept())
                .await
                .unwrap()
                .unwrap();
            let handshake = incoming.accept().unwrap();
            tokio::time::timeout(Duration::from_secs(20), handshake)
                .await
                .unwrap()
        }
    );
    (client, outbound.unwrap().unwrap(), inbound.unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_quic_put_reopen_node_then_get_on_other_device() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(temp.path());
    let source = temp.path().join("source.md");
    fs::write(&source, CONTENT).unwrap();

    let server = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::from_bytes(&[44; 32]))
        .relay_mode(RelayMode::Disabled)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .unwrap();
    let server_endpoint = *server.id().as_bytes();
    let mut node = NodeHost::open_integration_fixture(&f.state, &f.policy).unwrap();
    assert_eq!(f.node_public, node.public_key());
    let (client_a, outgoing_a, incoming_a) =
        establish(&server, SecretKey::from_bytes(&[21; 32])).await;

    let (served, put) = tokio::join!(
        serve_one_authenticated_connection(&incoming_a, &mut node),
        put_over_authenticated_iroh(
            &outgoing_a,
            server_endpoint,
            &f.roster,
            &f.put,
            CONTENT,
            |claim| Ok(f.a.sign(claim).as_ref().try_into().unwrap()),
        ),
    );
    assert_eq!(served, Ok(()));
    let receipt = put.unwrap();
    assert_eq!(receipt.raw_blake3, f.put.expected_blake3);
    assert!(receipt.verify(f.node_public).is_ok());
    drop(incoming_a);
    drop(outgoing_a);
    client_a.close().await;
    // The source machine is genuinely unavailable before client B connects.
    fs::remove_file(&source).unwrap();
    assert!(!source.exists());
    drop(node);

    let mut reopened = NodeHost::open_integration_fixture(&f.state, &f.policy).unwrap();
    assert_eq!(reopened.public_key(), receipt.node_public);
    let (client_b, outgoing_b, incoming_b) =
        establish(&server, SecretKey::from_bytes(&[22; 32])).await;
    let (served, get) = tokio::join!(
        serve_one_authenticated_connection(&incoming_b, &mut reopened),
        get_over_authenticated_iroh(
            &outgoing_b,
            server_endpoint,
            &f.roster,
            &f.read,
            |claim| Ok(f.b.sign(claim).as_ref().try_into().unwrap()),
        ),
    );
    assert_eq!(served, Ok(()));
    assert_eq!(get.unwrap(), CONTENT);
    drop(incoming_b);
    drop(outgoing_b);
    client_b.close().await;

    // B cannot reuse A's write grant, even with a signed owner roster.
    let (rogue_endpoint, rogue_out, rogue_in) =
        establish(&server, SecretKey::from_bytes(&[22; 32])).await;
    let (rejected, attempt) = tokio::join!(
        serve_one_authenticated_connection(&rogue_in, &mut reopened),
        put_over_authenticated_iroh(
            &rogue_out,
            server_endpoint,
            &f.roster,
            &f.put,
            CONTENT,
            |claim| Ok(f.b.sign(claim).as_ref().try_into().unwrap()),
        ),
    );
    assert_eq!(rejected, Err(NodeWireDenied));
    assert!(attempt.is_err());
    rogue_endpoint.close().await;
    server.close().await;
}
