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
        version: 3, node_public,
        iroh_endpoint_public: [51; 32],
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
    assert!(f.host.verify_iroh_endpoint([51; 32]).is_ok());
    assert!(f.host.verify_iroh_endpoint([99; 32]).is_err());
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

fn expect_hold<T>(result: Result<T, NodeV1Error>, expected: &str) {
    match result {
        Err(NodeV1Error::Hold(actual)) => assert_eq!(actual, expected),
        Err(other) => panic!("expected Hold({expected}), got {other:?}"),
        Ok(_) => panic!("unexpected admission: expected Hold({expected})"),
    }
}

fn expect_pair(
    trust: &RootTrust, roster: &OwnerSignedRoster, read: &OwnerReadGrant,
    put: &OwnerPutGrant, peer: [u8; 32], read_error: &str, put_error: &str,
) {
    expect_hold(trust.admit_read(roster, read, peer), read_error);
    expect_hold(trust.admit_put(roster, put, peer), put_error);
}

// Test-only custody: replace the independently pinned fixture policy and
// reopen it through the real protected-file loader, never through a request.
fn pin_test_policy(f: &Fixture, policy: &PolicyFile) -> RootTrust {
    fs::write(f.policy_dir.join(POLICY_NAME), serde_json::to_vec(policy).unwrap()).unwrap();
    RootTrust::load_from(&f.policy_dir, rustix::process::geteuid().as_raw(), policy.node_public)
        .unwrap()
}

fn sign_test_roster(roster: &mut OwnerSignedRoster) {
    let owner = Ed25519KeyPair::from_seed_unchecked(&[3; 32]).unwrap();
    roster.signature = owner.sign(&roster.canonical_claim().unwrap())
        .as_ref().try_into().unwrap();
}

#[test]
fn read_write_scope_and_error_precedence_are_pinned() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(temp.path());
    let trust = &f.host.trust;
    let peer = f.put.device_public_key;
    let admitted_read = trust.admit_read(&f.roster, &f.read, peer).unwrap();
    let admitted_put = trust.admit_put(&f.roster, &f.put, peer).unwrap();
    assert_eq!(admitted_read.peer(), peer);
    assert_eq!(admitted_read.grant_digest(), signed_grant_digest(&f.read).unwrap());
    assert_eq!(admitted_read.policy_digest(), trust.digest);
    assert_eq!(admitted_put.scope(), &f.put);
    assert_eq!(admitted_put.digest(), signed_put_digest(&f.put).unwrap());
    assert_eq!(admitted_put.policy_digest(), trust.digest);

    for wrong_peer in [[0; 32], [8; 32]] {
        expect_pair(trust, &f.roster, &f.read, &f.put, wrong_peer,
            "incorrect read scope", "incorrect write scope");
    }

    macro_rules! wrong_scope {
        ($field:ident, $value:expr) => {{
            let mut read = f.read.clone();
            let mut put = f.put.clone();
            read.$field = $value;
            put.$field = $value;
            // Invalid claim/signature must not mask the earlier scope rejection.
            read.filename = "../bad.md".into();
            put.filename = "../bad.md".into();
            expect_pair(trust, &f.roster, &read, &put, peer,
                "incorrect read scope", "incorrect write scope");
        }};
    }
    wrong_scope!(device_public_key, [8; 32]);
    wrong_scope!(node_identity, [8; 32]);
    wrong_scope!(vault_id, VaultId::new());
    wrong_scope!(grant_epoch, 8);

    let mut old_roster = f.roster.clone();
    old_roster.epoch -= 1;
    sign_test_roster(&mut old_roster);
    expect_pair(trust, &old_roster, &f.read, &f.put, peer,
        "incorrect read scope", "incorrect write scope");

    let mut future_roster = f.roster.clone();
    future_roster.epoch += 1;
    sign_test_roster(&mut future_roster);
    expect_pair(trust, &future_roster, &f.read, &f.put, peer,
        "incorrect read scope", "incorrect write scope");
}

#[test]
fn read_write_roster_admission_is_independently_pinned() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(temp.path());
    let peer = f.put.device_public_key;
    let original = &f.host.trust;

    let mut forged = f.roster.clone();
    forged.signature[0] ^= 1;
    expect_pair(original, &forged, &f.read, &f.put, peer,
        "invalid signed roster", "invalid signed device roster");

    let mut ambiguous = f.roster.clone();
    ambiguous.devices.push(f.roster.devices[0].clone());
    expect_pair(original, &ambiguous, &f.read, &f.put, peer,
        "invalid signed roster", "invalid signed device roster");

    let mut different_vault = f.roster.clone();
    different_vault.vault_id = VaultId::new();
    sign_test_roster(&mut different_vault);
    expect_pair(original, &different_vault, &f.read, &f.put, peer,
        "invalid signed roster", "invalid signed device roster");

    let mut revoked = f.roster.clone();
    revoked.devices[0].status = RosterStatus::Revoked;
    sign_test_roster(&mut revoked);
    // Even a correctly signed roster cannot replace the independently pinned digest.
    expect_pair(original, &revoked, &f.read, &f.put, peer,
        "invalid signed roster", "invalid signed device roster");

    let mut absent = f.roster.clone();
    absent.devices.clear();
    let mut wrong_endpoint = f.roster.clone();
    wrong_endpoint.devices[0].endpoint_public_key = [8; 32];
    let mut wrong_device_key = f.roster.clone();
    wrong_device_key.devices[0].device_public_key = [8; 32];
    let mut wrong_device_id = f.roster.clone();
    wrong_device_id.devices[0].device_id = DeviceId::new();

    for (mut roster, read_error, put_error) in [
        (absent, "device is not enrolled", "write device is not enrolled"),
        (wrong_endpoint, "device is not enrolled", "write device is not enrolled"),
        (revoked, "revoked or ambiguous device", "revoked write device"),
        (wrong_device_key, "revoked or ambiguous device", "revoked write device"),
        (wrong_device_id, "revoked or ambiguous device", "revoked write device"),
    ] {
        sign_test_roster(&mut roster);
        let mut policy = original.policy.clone();
        policy.roster_digest = roster.claim_digest().unwrap();
        let trust = pin_test_policy(&f, &policy);
        expect_pair(&trust, &roster, &f.read, &f.put, peer, read_error, put_error);
    }
}

#[test]
fn read_write_claim_signatures_and_exact_digests_remain_distinct() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(temp.path());
    let peer = f.put.device_public_key;
    let original = &f.host.trust;
    let owner = Ed25519KeyPair::from_seed_unchecked(&[3; 32]).unwrap();
    let mut read = f.read.clone();
    let mut put = f.put.clone();
    read.filename = "../bad.md".into();
    put.filename = "../bad.md".into();
    expect_pair(original, &f.roster, &read, &put, peer,
        "invalid signed grant claim", "invalid write claim");

    let mut read = f.read.clone();
    let mut put = f.put.clone();
    read.owner_signature = f.put.owner_signature;
    put.owner_signature = f.read.owner_signature;
    expect_pair(original, &f.roster, &read, &put, peer,
        "invalid owner signature", "invalid owner write signature");

    let mut read = f.read.clone();
    let mut put = f.put.clone();
    let other_document = DocumentId::new();
    read.document_id = other_document;
    put.document_id = other_document;
    read.owner_signature = owner.sign(&owner_claim(&read).unwrap()).as_ref().try_into().unwrap();
    put.owner_signature = owner.sign(&put_claim(&put).unwrap()).as_ref().try_into().unwrap();
    expect_pair(original, &f.roster, &read, &put, peer,
        "grant is no longer current", "owner write grant is no longer current");

    let mut policy = original.policy.clone();
    assert_ne!(policy.grant_digest, policy.put_grant_digest);
    std::mem::swap(&mut policy.grant_digest, &mut policy.put_grant_digest);
    let trust = pin_test_policy(&f, &policy);
    expect_pair(&trust, &f.roster, &f.read, &f.put, peer,
        "grant is no longer current", "owner write grant is no longer current");

    let mut policy = original.policy.clone();
    policy.grant_digest = [91; 32];
    let trust = pin_test_policy(&f, &policy);
    expect_hold(trust.admit_read(&f.roster, &f.read, peer), "grant is no longer current");
    assert!(trust.admit_put(&f.roster, &f.put, peer).is_ok());

    let mut policy = original.policy.clone();
    policy.put_grant_digest = [92; 32];
    let trust = pin_test_policy(&f, &policy);
    assert!(trust.admit_read(&f.roster, &f.read, peer).is_ok());
    expect_hold(trust.admit_put(&f.roster, &f.put, peer), "owner write grant is no longer current");
}

#[test]
fn read_write_policy_rotation_precedes_untrusted_request_checks() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(temp.path());
    let original = &f.host.trust;
    let peer = f.put.device_public_key;
    let mut policy = original.policy.clone();
    policy.iroh_endpoint_public = [93; 32];
    let reloaded = pin_test_policy(&f, &policy);
    expect_pair(original, &f.roster, &f.read, &f.put, [0; 32],
        "owner policy changed: reload and review", "owner policy changed: reload and review");
    expect_pair(original, &f.roster, &f.read, &f.put, peer,
        "owner policy changed: reload and review", "owner policy changed: reload and review");
    assert!(reloaded.admit_read(&f.roster, &f.read, peer).is_ok());
    assert!(reloaded.admit_put(&f.roster, &f.put, peer).is_ok());
}
