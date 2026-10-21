use super::*;
use crate::messaging::w4::{verify_receipt, Error as W4Error, RECEIPT_LEN};
use ring::signature::{Ed25519KeyPair, KeyPair};
use tempfile::tempdir;

fn keypair(seed: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
}
fn device() -> DeviceId {
    DeviceId::from_uuid(uuid::Uuid::from_bytes([1; 16]))
}
fn binding(
    device: DeviceId,
    signer: &Ed25519KeyPair,
    epoch: u64,
    evidence: u8,
) -> ApprovedEnrollment {
    let mut key = [0u8; 32];
    key.copy_from_slice(signer.public_key().as_ref());
    ApprovedEnrollment {
        device,
        epoch,
        ed25519: key,
        evidence_digest: [evidence; 32],
    }
}
fn revocation(device: DeviceId, epoch: u64) -> ApprovedRevocation {
    ApprovedRevocation {
        device,
        epoch,
        evidence_digest: [88; 32],
    }
}
fn signed_w4(device: DeviceId, epoch: u64, signer: &Ed25519KeyPair) -> Vec<u8> {
    let mut raw = Vec::with_capacity(RECEIPT_LEN);
    raw.push(1);
    raw.extend_from_slice(device.as_uuid().as_bytes());
    raw.extend_from_slice(&[2; 16]);
    raw.extend_from_slice(&[0xA5; 32]);
    raw.extend_from_slice(&epoch.to_be_bytes());
    let mut tbs = b"Akula/WOW/recipient-ack/v1\0".to_vec();
    tbs.extend_from_slice(&raw);
    raw.extend_from_slice(signer.sign(&tbs).as_ref());
    raw
}

#[test]
fn no_tofu_durable_device_key_and_signed_receipt_fail_closed_after_revocation() {
    let folder = tempdir().unwrap();
    let db = folder.path().join("trusted.sqlite");
    let d = device();
    let signer = keypair(4);
    let before = signed_w4(d, 7, &signer);
    {
        let mut ledger = W4TrustLedger::open(&db).unwrap();
        assert!(ledger.key_for(d, 7).is_none());
        assert!(matches!(
            verify_receipt(&before, &ledger),
            Err(W4Error::Unauthorized)
        ));
        assert_eq!(
            ledger.enroll(&binding(d, &signer, 7, 1)).unwrap(),
            LedgerOutcome::Enrolled
        );
        assert!(verify_receipt(&before, &ledger).is_ok());
    }
    let mut ledger = W4TrustLedger::open(&db).unwrap();
    assert!(verify_receipt(&before, &ledger).is_ok());
    assert_eq!(
        ledger.revoke(&revocation(d, 7)).unwrap(),
        LedgerOutcome::Revoked
    );
    assert!(ledger.key_for(d, 7).is_none());
    assert!(matches!(
        verify_receipt(&before, &ledger),
        Err(W4Error::Unauthorized)
    ));
    drop(ledger);
    let ledger = W4TrustLedger::open(&db).unwrap();
    assert!(ledger.key_for(d, 7).is_none());
}

#[test]
fn monotonic_epochs_forbid_key_replacement_and_stale_revival() {
    let folder = tempdir().unwrap();
    let mut ledger = W4TrustLedger::open(&folder.path().join("epoch.sqlite")).unwrap();
    let d = device();
    let first = keypair(4);
    let second = keypair(9);
    let original = binding(d, &first, 7, 1);
    assert_eq!(ledger.enroll(&original).unwrap(), LedgerOutcome::Enrolled);
    assert_eq!(ledger.enroll(&original).unwrap(), LedgerOutcome::Duplicate);
    assert!(matches!(
        ledger.enroll(&binding(d, &second, 7, 2)),
        Err(LedgerError::Conflict)
    ));
    assert!(matches!(
        ledger.enroll(&binding(d, &second, 6, 2)),
        Err(LedgerError::StaleEpoch)
    ));
    assert!(matches!(
        ledger.revoke(&revocation(d, 6)),
        Err(LedgerError::StaleEpoch)
    ));
    assert_eq!(
        ledger.revoke(&revocation(d, 7)).unwrap(),
        LedgerOutcome::Revoked
    );
    assert_eq!(
        ledger.revoke(&revocation(d, 7)).unwrap(),
        LedgerOutcome::Duplicate
    );
    assert!(matches!(
        ledger.enroll(&original),
        Err(LedgerError::Revoked)
    ));
    assert_eq!(
        ledger.enroll(&binding(d, &second, 8, 3)).unwrap(),
        LedgerOutcome::Rotated
    );
    assert!(ledger.key_for(d, 7).is_none());
    assert!(ledger.key_for(d, 8).is_some());
    assert!(matches!(
        verify_receipt(&signed_w4(d, 7, &first), &ledger),
        Err(W4Error::Unauthorized)
    ));
    assert!(verify_receipt(&signed_w4(d, 8, &second), &ledger).is_ok());
}

#[test]
fn missing_invalid_and_corrupted_records_never_grant_trust() {
    let folder = tempdir().unwrap();
    let mut ledger = W4TrustLedger::open(&folder.path().join("no-tofu.sqlite")).unwrap();
    let d = device();
    let signer = keypair(7);
    assert!(matches!(
        ledger.revoke(&revocation(d, 2)),
        Err(LedgerError::Missing)
    ));
    let mut input = binding(d, &signer, 7, 1);
    input.epoch = 0;
    assert!(matches!(ledger.enroll(&input), Err(LedgerError::Invalid)));
    input.epoch = 7;
    input.evidence_digest = [0; 32];
    assert!(matches!(ledger.enroll(&input), Err(LedgerError::Invalid)));
    assert!(ledger.key_for(d, 7).is_none());
    ledger.enroll(&binding(d, &signer, 7, 1)).unwrap();
    // An OS-level attacker can mutate this database. We fail closed on bad
    // data, but this is NOT a substitute for secure file custody.
    let uuid = d.as_uuid().into_bytes();
    ledger
        .db
        .execute(
            "UPDATE messaging_w4_trusted_devices SET revoked=1 WHERE device=?1",
            params![uuid.as_slice()],
        )
        .unwrap();
    assert!(ledger.key_for(d, 7).is_none());
}

#[test]
fn rotation_requires_new_evidence_and_cannot_rollback_epoch_after_restart() {
    let folder = tempdir().unwrap();
    let db = folder.path().join("rollover.sqlite");
    let d = device();
    let s1 = keypair(3);
    let s2 = keypair(4);
    {
        let mut ledger = W4TrustLedger::open(&db).unwrap();
        ledger.enroll(&binding(d, &s1, 11, 1)).unwrap();
        ledger.revoke(&revocation(d, 11)).unwrap();
        ledger.enroll(&binding(d, &s2, 12, 2)).unwrap();
    }
    let mut restarted = W4TrustLedger::open(&db).unwrap();
    assert!(restarted.key_for(d, 11).is_none());
    assert!(restarted.key_for(d, 12).is_some());
    assert!(matches!(
        restarted.enroll(&binding(d, &s1, 11, 1)),
        Err(LedgerError::StaleEpoch)
    ));
    assert!(matches!(
        restarted.enroll(&binding(d, &s1, 12, 5)),
        Err(LedgerError::Conflict)
    ));
}
