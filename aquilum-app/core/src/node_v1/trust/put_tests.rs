use super::*;
use crate::sync::device_roster_lab::{OwnerSignedRoster, RosterStatus};
use super::super::{DurableChallengeFence, NodeHost, NodeIdentity};
use super::super::put::{OwnerPutGrant, put_claim, put_proof_claim, signed_put_digest};
use crate::node_replica_auth_lab::{
    DeviceReadProof, OwnerReadGrant, device_challenge_claim,
    owner_claim, signed_grant_digest,
};
use crate::sync::{DeviceId, DocumentId, RevisionId};
use crate::sync::device_roster_lab::ClaimedDevice;
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const CONTENT: &[u8] = b"# Node V1 signed replica\n";

struct Fixture {
    host: NodeHost,
    roster: OwnerSignedRoster,
    put: OwnerPutGrant,
    read: OwnerReadGrant,
    device: Ed25519KeyPair,
    state: std::path::PathBuf,
    policy_dir: std::path::PathBuf,
}

fn fixture(root: &Path) -> Fixture {
    let state = root.join("state");
    let policy_dir = root.join("root-policy");
    for path in [&state, &policy_dir] {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let identity = NodeIdentity::open_or_create(&state).unwrap();
    let node_public = identity.public_key();
    let owner = Ed25519KeyPair::from_seed_unchecked(&[3; 32]).unwrap();
    let device = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    let peer: [u8; 32] = device.public_key().as_ref().try_into().unwrap();
    let vault_id = VaultId::new();
    let device_id = DeviceId::new();
    let document_id = DocumentId::new();
    let revision_id = RevisionId::new();
    let hash = blake3::hash(CONTENT).to_hex().to_string();

    let mut roster = OwnerSignedRoster {
        vault_id, epoch: 9,
        devices: vec![ClaimedDevice {
            device_id,
            device_public_key: peer,
            endpoint_public_key: peer,
            status: RosterStatus::RecordedOnly,
        }],
        signature: [0; 64],
    };
    roster.signature = owner.sign(&roster.canonical_claim().unwrap())
        .as_ref().try_into().unwrap();

    let mut put = OwnerPutGrant {
        vault_id, document_id, revision_id, filename: "note.md".into(),
        expected_blake3: hash.clone(), node_identity: node_public,
        device_id, device_public_key: peer, grant_epoch: 9,
        owner_signature: [0; 64],
    };
    put.owner_signature = owner.sign(&put_claim(&put).unwrap())
        .as_ref().try_into().unwrap();
    let mut read = OwnerReadGrant {
        vault_id, document_id, revision_id, filename: "note.md".into(),
        expected_blake3: hash, node_identity: node_public,
        device_id, device_public_key: peer, grant_epoch: 9,
        owner_signature: [0; 64],
    };
    read.owner_signature = owner.sign(&owner_claim(&read).unwrap())
        .as_ref().try_into().unwrap();

    let policy = PolicyFile {
        version: 2, node_public,
        owner_public: owner.public_key().as_ref().try_into().unwrap(),
        vault_id, grant_epoch: 9, roster_digest: roster.claim_digest().unwrap(),
        grant_digest: signed_grant_digest(&read).unwrap(),
        put_grant_digest: signed_put_digest(&put).unwrap(),
    };
    let policy_path = policy_dir.join(POLICY_NAME);
    fs::write(&policy_path, serde_json::to_vec(&policy).unwrap()).unwrap();
    fs::set_permissions(policy_path, fs::Permissions::from_mode(0o600)).unwrap();

    let trust = RootTrust::load_from(
        &policy_dir, rustix::process::geteuid().as_raw(), node_public,
    ).unwrap();
    let ledger = DurableChallengeFence::open(&state, trust.clone()).unwrap();
    Fixture {
        host: NodeHost { identity, trust, ledger, state: state.clone() },
        roster, put, read, device, state, policy_dir,
    }
}

#[test]
fn signed_put_receipt_and_independent_read_survive_process_restart() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture(temp.path());
    let peer = f.put.device_public_key;
    let nonce = f.host.issue_put_challenge(&f.roster, &f.put, peer).unwrap();
    let signed: [u8; 64] = f.device.sign(&put_proof_claim(&f.put, nonce).unwrap())
        .as_ref().try_into().unwrap();
    let receipt = f.host.commit_put(
        &f.roster, &f.put, peer, nonce, signed, CONTENT,
    ).unwrap();
    assert!(receipt.verify(f.host.public_key()).is_ok());
    assert_eq!(receipt.byte_length, CONTENT.len() as u64);
    assert_eq!(receipt.raw_blake3, f.put.expected_blake3);
    assert!(f.host.commit_put(
        &f.roster, &f.put, peer, nonce, signed, CONTENT,
    ).is_err());

    let read_nonce = f.host.issue_read_challenge(&f.roster, &f.read, peer).unwrap();
    let proof = DeviceReadProof {
        device_signature: f.device.sign(
            &device_challenge_claim(&f.read, read_nonce).unwrap(),
        ).as_ref().try_into().unwrap(),
    };
    assert_eq!(f.host.fulfill_read(
        &f.roster, &f.read, peer, read_nonce, &proof,
    ).unwrap(), CONTENT);
    assert!(f.host.fulfill_read(
        &f.roster, &f.read, peer, read_nonce, &proof,
    ).is_err());

    let state = f.state.clone();
    let policy_dir = f.policy_dir.clone();
    let key = f.host.public_key();
    drop(f.host);
    let identity = NodeIdentity::open_or_create(&state).unwrap();
    assert_eq!(key, identity.public_key());
    let trust = RootTrust::load_from(
        &policy_dir, rustix::process::geteuid().as_raw(), key,
    ).unwrap();
    let ledger = DurableChallengeFence::open(&state, trust.clone()).unwrap();
    let mut reopened = NodeHost { identity, trust, ledger, state };
    let again = reopened.issue_read_challenge(&f.roster, &f.read, peer).unwrap();
    let proof = DeviceReadProof {
        device_signature: f.device.sign(
            &device_challenge_claim(&f.read, again).unwrap(),
        ).as_ref().try_into().unwrap(),
    };
    assert_eq!(reopened.fulfill_read(
        &f.roster, &f.read, peer, again, &proof,
    ).unwrap(), CONTENT);
}

#[test]
fn wrong_bytes_rogue_peer_read_write_confusion_and_revoke_are_denied() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture(temp.path());
    let peer = f.put.device_public_key;
    assert!(f.host.issue_put_challenge(&f.roster, &f.put, [8; 32]).is_err());
    let mut forged = f.put.clone();
    forged.owner_signature[0] ^= 1;
    assert!(f.host.issue_put_challenge(&f.roster, &forged, peer).is_err());

    let nonce = f.host.issue_put_challenge(&f.roster, &f.put, peer).unwrap();
    let proof: [u8;64] = f.device.sign(&put_proof_claim(&f.put, nonce).unwrap())
        .as_ref().try_into().unwrap();
    let read_proof = DeviceReadProof {
        device_signature: f.device.sign(
            &device_challenge_claim(&f.read, nonce).unwrap(),
        ).as_ref().try_into().unwrap(),
    };
    assert!(f.host.fulfill_read(
        &f.roster, &f.read, peer, nonce, &read_proof,
    ).is_err());
    assert!(f.host.commit_put(
        &f.roster, &f.put, peer, nonce, proof, b"wrong bytes",
    ).is_err());
    assert!(f.host.commit_put(
        &f.roster, &f.put, peer, nonce, proof, CONTENT,
    ).is_err());
    let revoked_path = f.policy_dir.join(POLICY_NAME);
    fs::write(&revoked_path, b"revoked").unwrap();
    assert!(f.host.issue_put_challenge(&f.roster, &f.put, peer).is_err());
    assert!(f.host.issue_read_challenge(&f.roster, &f.read, peer).is_err());
}

#[test]
fn damaged_durable_blob_cannot_be_served_or_confirmed() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture(temp.path());
    let peer = f.put.device_public_key;
    let challenge = f.host.issue_put_challenge(&f.roster, &f.put, peer).unwrap();
    let proof: [u8; 64] = f.device.sign(
        &put_proof_claim(&f.put, challenge).unwrap(),
    ).as_ref().try_into().unwrap();
    f.host.commit_put(
        &f.roster, &f.put, peer, challenge, proof, CONTENT,
    ).unwrap();

    let db_path = f.state.join(format!(
        "akula-node-replica-{}.sqlite", f.put.vault_id,
    ));
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute(
        "UPDATE sync_blobs SET data = ?1", rusqlite::params![b"tampered".as_slice()],
    ).unwrap();
    drop(conn);

    let read_challenge = f.host.issue_read_challenge(
        &f.roster, &f.read, peer,
    ).unwrap();
    let signed = DeviceReadProof {
        device_signature: f.device.sign(
            &device_challenge_claim(&f.read, read_challenge).unwrap(),
        ).as_ref().try_into().unwrap(),
    };
    assert!(f.host.fulfill_read(
        &f.roster, &f.read, peer, read_challenge, &signed,
    ).is_err());
}
