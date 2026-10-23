use super::*;
use crate::sync::{DeviceId, DocumentId, RevisionId};
use crate::sync::device_roster_lab::ClaimedDevice;
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::os::unix::fs::PermissionsExt;

#[test]
fn root_policy_rotation_and_symlink_are_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("root-policy");
    let trusted = policy_fixture(&parent, [5; 32]);
    assert!(trusted.require_current().is_ok());
    let file = parent.join(POLICY_NAME);
    let original = fs::read(&file).unwrap();
    fs::write(&file, b"corrupt").unwrap();
    assert!(trusted.require_current().is_err());
    fs::remove_file(&file).unwrap();
    let substituted = parent.join("untrusted.json");
    fs::write(&substituted, original).unwrap();
    std::os::unix::fs::symlink(substituted, &file).unwrap();
    assert!(trusted.require_current().is_err());
}

#[test]
fn grant_requires_current_signed_roster_and_owner_signature() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("policy");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let owner = Ed25519KeyPair::from_seed_unchecked(&[3; 32]).unwrap();
    let device = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    let peer: [u8; 32] = device.public_key().as_ref().try_into().unwrap();
    let node = [13; 32];
    let vault_id = VaultId::new();
    let device_id = DeviceId::new();
    let mut roster = OwnerSignedRoster {
        vault_id,
        epoch: 3,
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
    let mut grant = OwnerReadGrant {
        vault_id,
        document_id: DocumentId::new(),
        revision_id: RevisionId::new(),
        expected_blake3: "a".repeat(64),
        filename: "note.md".to_string(),
        node_identity: node,
        device_id,
        device_public_key: peer,
        grant_epoch: 3,
        owner_signature: [0; 64],
    };
    grant.owner_signature = owner.sign(&owner_claim(&grant).unwrap())
        .as_ref().try_into().unwrap();
    let policy = PolicyFile {
        version: 1,
        node_public: node,
        owner_public: owner.public_key().as_ref().try_into().unwrap(),
        vault_id,
        grant_epoch: 3,
        roster_digest: roster.claim_digest().unwrap(),
        grant_digest: signed_grant_digest(&grant).unwrap(),
    };
    let path = parent.join(POLICY_NAME);
    fs::write(&path, serde_json::to_vec(&policy).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let trust = RootTrust::load_from(
        &parent, rustix::process::geteuid().as_raw(), node,
    ).unwrap();
    let admitted = trust.admit_read(&roster, &grant, peer).unwrap();
    assert_eq!(admitted.peer(), peer);
    assert_eq!(admitted.grant_digest(), policy.grant_digest);
    assert!(trust.admit_read(&roster, &grant, [8; 32]).is_err());

    let mut forged = grant.clone();
    forged.owner_signature[0] ^= 1;
    assert!(trust.admit_read(&roster, &forged, peer).is_err());

    let mut revoked = roster;
    revoked.devices[0].status = RosterStatus::Revoked;
    revoked.signature = owner.sign(&revoked.canonical_claim().unwrap())
        .as_ref().try_into().unwrap();
    assert!(trust.admit_read(&revoked, &grant, peer).is_err());

    fs::write(&path, b"revoked").unwrap();
    assert!(trust.admit_read(&roster_fixture(&policy, &owner, device_id, peer), &grant, peer).is_err());
}

/// A new valid roster after the root policy changed must not revive old
/// in-memory admission. This fixture exists only in unit tests.
fn roster_fixture(
    policy: &PolicyFile, owner: &Ed25519KeyPair, device_id: DeviceId, peer: [u8; 32],
) -> OwnerSignedRoster {
    let mut roster = OwnerSignedRoster {
        vault_id: policy.vault_id,
        epoch: policy.grant_epoch,
        devices: vec![ClaimedDevice {
            device_id, device_public_key: peer,
            endpoint_public_key: peer, status: RosterStatus::RecordedOnly,
        }],
        signature: [0; 64],
    };
    roster.signature = owner.sign(&roster.canonical_claim().unwrap())
        .as_ref().try_into().unwrap();
    roster
}
