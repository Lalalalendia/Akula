//! NODE-E2E-01b: signed owner grant + device Ed25519 proof in a real
//! independent Linux reader process. Lab grants are not production enrollment.
#![cfg(all(target_os = "linux", feature = "node-replica-auth-lab"))]

use aquilum_core::node_replica_auth_lab::{
    device_challenge_claim, owner_claim, read_with_device_proof, signed_grant_digest,
    DeviceReadProof, IndependentReadTrust, OwnerReadGrant, ReadDenied,
};
use aquilum_core::node_replica_lab::{stage_operator_fixture, OperatorFixtureAdmission};
use aquilum_core::sync::{DeviceId, DocumentId, RevisionId, VaultId};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    state: PathBuf,
    owner: Ed25519KeyPair,
    device: Ed25519KeyPair,
    grant: OwnerReadGrant,
    trusted: IndependentReadTrust,
    proof: DeviceReadProof,
    bytes: Vec<u8>,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("owner-vault");
    let state = temp.path().join("node-owned-state");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let bytes = b"# authorized\r\nA separate reader must keep exact CRLF.\r\n".to_vec();
    fs::write(source.join("public.md"), &bytes).unwrap();

    let owner = Ed25519KeyPair::from_seed_unchecked(&[7u8; 32]).unwrap();
    let device = Ed25519KeyPair::from_seed_unchecked(&[17u8; 32]).unwrap();
    let vault_id = VaultId::from_uuid(uuid::Uuid::from_u128(0x1111));
    let document_id = DocumentId::from_uuid(uuid::Uuid::from_u128(0x2222));
    let revision_id = RevisionId::from_uuid(uuid::Uuid::from_u128(0x3333));
    let expected_blake3 = blake3::hash(&bytes).to_hex().to_string();
    let stage = OperatorFixtureAdmission {
        vault_id, document_id, revision_id,
        filename: "public.md".into(), expected_blake3: expected_blake3.clone(),
    };
    stage_operator_fixture(&source, &state, &stage).unwrap();
    let mut grant = OwnerReadGrant {
        vault_id, document_id, revision_id, expected_blake3,
        filename: "public.md".into(),
        node_identity: [0x51u8; 32],
        device_id: DeviceId::from_uuid(uuid::Uuid::from_u128(0x4444)),
        device_public_key: device.public_key().as_ref().try_into().unwrap(),
        grant_epoch: 7,
        owner_signature: [0; 64],
    };
    grant.owner_signature = owner.sign(&owner_claim(&grant).unwrap())
        .as_ref().try_into().unwrap();
    let trusted = IndependentReadTrust {
        trusted_vault_id: vault_id,
        trusted_owner_public_key: owner.public_key().as_ref().try_into().unwrap(),
        trusted_node_identity: grant.node_identity,
        witnessed_current_epoch: grant.grant_epoch,
        witnessed_exact_grant_digest: signed_grant_digest(&grant).unwrap(),
        current_session_challenge: [0x42u8; 32],
    };
    let proof = DeviceReadProof {
        device_signature: device.sign(&device_challenge_claim(
            &grant, trusted.current_session_challenge,
        ).unwrap()).as_ref().try_into().unwrap(),
    };
    Fixture { _temp: temp, source, state, owner, device, grant, trusted, proof, bytes }
}

fn array<const N: usize>(value: &Value) -> [u8; N] {
    value.as_array().unwrap().iter().map(|n| {
        u8::try_from(n.as_u64().unwrap()).unwrap()
    }).collect::<Vec<_>>().try_into().unwrap()
}

fn signed_case(f: &Fixture, trusted: &IndependentReadTrust) -> String {
    json!({
        "state": f.state.to_str().unwrap(),
        "vault": f.grant.vault_id.to_string(),
        "document": f.grant.document_id.to_string(),
        "revision": f.grant.revision_id.to_string(),
        "hash": f.grant.expected_blake3,
        "filename": f.grant.filename,
        "node_id": f.grant.node_identity,
        "device_id": f.grant.device_id.to_string(),
        "device_key": f.grant.device_public_key,
        "grant_epoch": f.grant.grant_epoch,
        "owner_signature": f.grant.owner_signature.to_vec(),
        "pin_vault": trusted.trusted_vault_id.to_string(),
        "pin_owner_key": trusted.trusted_owner_public_key,
        "pin_node": trusted.trusted_node_identity,
        "pin_epoch": trusted.witnessed_current_epoch,
        "pin_digest": trusted.witnessed_exact_grant_digest,
        "challenge": trusted.current_session_challenge,
        "device_signature": f.proof.device_signature.to_vec(),
    }).to_string()
}

fn child(f: &Fixture, trusted: &IndependentReadTrust) -> Output {
    Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg("authenticated_reader_child").arg("--nocapture")
        .env("AKULA_NODE_AUTH_CHILD", signed_case(f, trusted))
        .output().unwrap()
}

#[test]
fn authenticated_reader_child() {
    let Ok(raw) = std::env::var("AKULA_NODE_AUTH_CHILD") else { return };
    let data: Value = serde_json::from_str(&raw).unwrap();
    let grant = OwnerReadGrant {
        vault_id: data["vault"].as_str().unwrap().parse().unwrap(),
        document_id: data["document"].as_str().unwrap().parse().unwrap(),
        revision_id: data["revision"].as_str().unwrap().parse().unwrap(),
        filename: data["filename"].as_str().unwrap().to_owned(),
        expected_blake3: data["hash"].as_str().unwrap().to_owned(),
        node_identity: array(&data["node_id"]),
        device_id: data["device_id"].as_str().unwrap().parse().unwrap(),
        device_public_key: array(&data["device_key"]),
        grant_epoch: data["grant_epoch"].as_u64().unwrap(),
        owner_signature: array(&data["owner_signature"]),
    };
    let trust = IndependentReadTrust {
        trusted_vault_id: data["pin_vault"].as_str().unwrap().parse().unwrap(),
        trusted_owner_public_key: array(&data["pin_owner_key"]),
        trusted_node_identity: array(&data["pin_node"]),
        witnessed_current_epoch: data["pin_epoch"].as_u64().unwrap(),
        witnessed_exact_grant_digest: array(&data["pin_digest"]),
        current_session_challenge: array(&data["challenge"]),
    };
    let proof = DeviceReadProof {
        device_signature: array(&data["device_signature"]),
    };
    let state = Path::new(data["state"].as_str().unwrap());
    let bytes = read_with_device_proof(state, &grant, &trust, &proof)
        .expect("crypto + epoch + authorized DAG read");
    assert_eq!(blake3::hash(&bytes).to_hex().as_str(), grant.expected_blake3);
    println!("NODE_E2E_01B_AUTHENTICATED_READER={}", bytes.len());
}

#[test]
fn independent_signed_reader_recovers_bytes_without_source_file() {
    let f = fixture();
    fs::rename(f.source.join("public.md"), f.source.join("original-offline.md")).unwrap();
    let response = child(&f, &f.trusted);
    assert!(response.status.success(), "reader: {}",
        String::from_utf8_lossy(&response.stderr));
    assert!(String::from_utf8_lossy(&response.stdout)
        .contains("NODE_E2E_01B_AUTHENTICATED_READER="));
    assert_eq!(fs::read(f.source.join("original-offline.md")).unwrap(), f.bytes);
}

#[test]
fn forged_signer_other_device_wrong_node_and_scope_are_denied() {
    let f = fixture();
    assert_eq!(read_with_device_proof(&f.state, &f.grant, &f.trusted, &f.proof),
        Ok(f.bytes.clone()));

    let mut owner_wrong = f.trusted.clone();
    owner_wrong.trusted_owner_public_key = [9u8; 32];
    let mut fake_sig = f.grant.clone();
    fake_sig.owner_signature[0] ^= 1;
    let mut other_device = f.proof.clone();
    let rogue = Ed25519KeyPair::from_seed_unchecked(&[88u8; 32]).unwrap();
    other_device.device_signature = rogue.sign(&device_challenge_claim(
        &f.grant, f.trusted.current_session_challenge,
    ).unwrap()).as_ref().try_into().unwrap();
    let mut new_nonce = f.trusted.clone();
    new_nonce.current_session_challenge = [0x43u8; 32];
    let mut wrong_node = f.trusted.clone();
    wrong_node.trusted_node_identity = [0x52u8; 32];
    let mut wrong_vault = f.trusted.clone();
    wrong_vault.trusted_vault_id = VaultId::from_uuid(uuid::Uuid::from_u128(0x5555));
    let mut nil_challenge = f.trusted.clone();
    nil_challenge.current_session_challenge = [0u8; 32];

    assert_eq!(read_with_device_proof(&f.state, &f.grant, &owner_wrong, &f.proof),
        Err(ReadDenied));
    assert_eq!(read_with_device_proof(&f.state, &fake_sig, &f.trusted, &f.proof),
        Err(ReadDenied));
    assert_eq!(read_with_device_proof(&f.state, &f.grant, &f.trusted, &other_device),
        Err(ReadDenied));
    assert_eq!(read_with_device_proof(&f.state, &f.grant, &new_nonce, &f.proof),
        Err(ReadDenied));
    assert_eq!(read_with_device_proof(&f.state, &f.grant, &wrong_node, &f.proof),
        Err(ReadDenied));
    assert_eq!(read_with_device_proof(&f.state, &f.grant, &wrong_vault, &f.proof),
        Err(ReadDenied));
    assert_eq!(read_with_device_proof(&f.state, &f.grant, &nil_challenge, &f.proof),
        Err(ReadDenied));
}

#[test]
fn witnessed_revoke_epoch_and_same_epoch_fork_reject_old_grant() {
    let f = fixture();
    let mut revoked = f.trusted.clone();
    revoked.witnessed_current_epoch = 8;
    let stopped = child(&f, &revoked);
    assert!(!stopped.status.success(), "external revoke epoch must deny");
    assert_eq!(read_with_device_proof(&f.state, &f.grant, &revoked, &f.proof),
        Err(ReadDenied));

    let mut wrong_digest = f.trusted.clone();
    wrong_digest.witnessed_exact_grant_digest = [0x33u8; 32];
    assert_eq!(read_with_device_proof(&f.state, &f.grant, &wrong_digest, &f.proof),
        Err(ReadDenied));

    // Even if the legitimate owner signs a different same-epoch scope,
    // the independent exact-claim witness must reject the unrecognized fork.
    let mut fork = f.grant.clone();
    fork.device_id = DeviceId::from_uuid(uuid::Uuid::from_u128(0x8888));
    fork.owner_signature = f.owner.sign(&owner_claim(&fork).unwrap())
        .as_ref().try_into().unwrap();
    let fork_proof = DeviceReadProof {
        device_signature: f.device.sign(&device_challenge_claim(
            &fork, f.trusted.current_session_challenge,
        ).unwrap()).as_ref().try_into().unwrap(),
    };
    assert_eq!(read_with_device_proof(&f.state, &fork, &f.trusted, &fork_proof),
        Err(ReadDenied));
}

#[test]
fn signed_grant_does_not_override_missing_or_tampered_original_blobs() {
    let f = fixture();
    let mut wrong_revision = f.grant.clone();
    wrong_revision.revision_id = RevisionId::from_uuid(uuid::Uuid::from_u128(0x9999));
    wrong_revision.owner_signature = f.owner.sign(&owner_claim(&wrong_revision).unwrap())
        .as_ref().try_into().unwrap();
    let mut trust = f.trusted.clone();
    trust.witnessed_exact_grant_digest = signed_grant_digest(&wrong_revision).unwrap();
    let proof = DeviceReadProof {
        device_signature: f.device.sign(&device_challenge_claim(
            &wrong_revision, trust.current_session_challenge,
        ).unwrap()).as_ref().try_into().unwrap(),
    };
    assert_eq!(read_with_device_proof(&f.state, &wrong_revision, &trust, &proof),
        Err(ReadDenied));

    let filename = f.state.join(format!("akula-node-replica-{}.sqlite", f.grant.vault_id));
    let connection = rusqlite::Connection::open(&filename).unwrap();
    connection.execute("UPDATE sync_blobs SET data = ?1 WHERE hash = ?2",
        rusqlite::params![b"spoofed".as_slice(), f.grant.expected_blake3],
    ).unwrap();
    drop(connection);
    assert_eq!(read_with_device_proof(&f.state, &f.grant, &f.trusted, &f.proof),
        Err(ReadDenied));
}
