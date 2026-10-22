//! Adversarial contract tests for an unpublished Dropbox object receipt.
//! Offline, with no application filesystem or remote account access.
use akula_dropbox_t0::publication::{
    Phase, ReceiptError, UnpublishedObjectReceipt, MAX_READBACK_BYTES,
};
use akula_dropbox_t0::{DropboxRevision, ObjectKey, Scope};
use serde_json::{json, Value};

const OP: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const VAULT: &str = "12345678-1234-1234-1234-123456789abc";
const CONTENT: &[u8] = b"persistent object, not yet a published note";

fn scope() -> Scope { Scope::new("dbid:alice", "12345", VAULT, "home").unwrap() }
fn key() -> ObjectKey {
    ObjectKey::parse(&format!("blobs/{}", blake3::hash(CONTENT).to_hex())).unwrap()
}
fn receipt() -> UnpublishedObjectReceipt {
    UnpublishedObjectReceipt::new(scope(), OP, key(),
        &blake3::hash(CONTENT).to_hex().to_string()).unwrap()
}
fn observe(r: &mut UnpublishedObjectReceipt) -> Result<(), ReceiptError> {
    r.inspect_bounded_readback(&scope(), &key(), "id:valid",
        DropboxRevision::parse("R2").unwrap(), CONTENT)
}

#[test]
fn uncertain_ack_requires_inspection_never_resending() {
    let mut r = receipt();
    r.mark_attempt_started().unwrap();
    r.mark_response_uncertain().unwrap();
    assert_eq!(r.phase(), Phase::AwaitingReadback);
    assert_eq!(r.mark_attempt_started(), Err(ReceiptError::InvalidPhase));
    observe(&mut r).unwrap();
    assert_eq!(r.phase(), Phase::VerifiedUnpublished);
    assert!(r.candidate().is_some());
}

#[test]
fn restart_while_prepared_fails_closed_to_inspection() {
    let original = receipt();
    let mut restarted = UnpublishedObjectReceipt::recover(&original.encode(), &receipt()).unwrap();
    assert_eq!(restarted.phase(), Phase::AwaitingReadback);
    assert_eq!(restarted.mark_attempt_started(), Err(ReceiptError::InvalidPhase));
    assert!(restarted.candidate().is_none());
}

#[test]
fn wrong_account_does_not_recover_or_inspect() {
    let second = Scope::new("dbid:bob", "12345", VAULT, "home").unwrap();
    let expected = UnpublishedObjectReceipt::new(
        second.clone(), OP, key(), &blake3::hash(CONTENT).to_hex().to_string()).unwrap();
    assert_eq!(UnpublishedObjectReceipt::recover(&receipt().encode(), &expected),
        Err(ReceiptError::ScopeChanged));
    let mut r = receipt();
    r.mark_attempt_started().unwrap();
    assert_eq!(r.inspect_bounded_readback(&second, &key(), "id:valid",
        DropboxRevision::parse("R2").unwrap(), CONTENT), Err(ReceiptError::ScopeChanged));
    assert_eq!(r.phase(), Phase::Held);
}

#[test]
fn wrong_vault_and_object_key_cannot_cross_boundary() {
    let second = Scope::new("dbid:alice", "12345",
        "ffffffff-ffff-ffff-ffff-ffffffffffff", "home").unwrap();
    let expected = UnpublishedObjectReceipt::new(
        second, OP, key(), &blake3::hash(CONTENT).to_hex().to_string()).unwrap();
    assert_eq!(UnpublishedObjectReceipt::recover(&receipt().encode(), &expected),
        Err(ReceiptError::ScopeChanged));
    let mut r = receipt();
    r.mark_attempt_started().unwrap();
    assert_eq!(r.inspect_bounded_readback(&scope(), &ObjectKey::head(),
        "id:valid", DropboxRevision::parse("R2").unwrap(), CONTENT),
        Err(ReceiptError::ObjectChanged));
    assert_eq!(r.phase(), Phase::Held);
}

#[test]
fn invalid_native_file_identity_is_not_a_publication_proof() {
    let mut r = receipt();
    r.mark_attempt_started().unwrap();
    assert_eq!(r.inspect_bounded_readback(&scope(), &key(),
        "../forged", DropboxRevision::parse("R2").unwrap(), CONTENT),
        Err(ReceiptError::InvalidProviderIdentity));
    assert!(r.candidate().is_none());
}

#[test]
fn oversized_readback_is_rejected_before_digest() {
    let mut r = receipt();
    r.mark_attempt_started().unwrap();
    let oversized = vec![0_u8; MAX_READBACK_BYTES + 1];
    assert_eq!(r.inspect_bounded_readback(&scope(), &key(),
        "id:valid", DropboxRevision::parse("R2").unwrap(), &oversized),
        Err(ReceiptError::OversizedReadback));
    assert_eq!(r.phase(), Phase::Held);
}

#[test]
fn unknown_version_and_extra_fields_are_refused() {
    let r = receipt();
    let mut version: Value = serde_json::from_slice(&r.encode()).unwrap();
    version["version"] = json!(2);
    assert_eq!(UnpublishedObjectReceipt::recover(&serde_json::to_vec(&version).unwrap(), &receipt()),
        Err(ReceiptError::InvalidRecord));
    let mut extra: Value = serde_json::from_slice(&r.encode()).unwrap();
    extra["new_authority"] = json!(true);
    assert_eq!(UnpublishedObjectReceipt::recover(&serde_json::to_vec(&extra).unwrap(), &receipt()),
        Err(ReceiptError::InvalidRecord));
}

#[test]
fn forged_verified_phase_and_missing_receipt_are_rejected() {
    let r = receipt();
    let mut record: Value = serde_json::from_slice(&r.encode()).unwrap();
    record["phase"] = json!("verified_unpublished");
    assert_eq!(UnpublishedObjectReceipt::recover(&serde_json::to_vec(&record).unwrap(), &receipt()),
        Err(ReceiptError::InvalidRecord));
}

#[test]
fn verified_record_after_restart_requires_fresh_remote_bytes() {
    let mut r = receipt();
    r.mark_attempt_started().unwrap();
    observe(&mut r).unwrap();
    let original = r.candidate().unwrap();
    assert_eq!(original.operation_id, OP);
    assert_eq!(original.object_key, key());
    assert_eq!(original.provider_revision.as_str(), "R2");

    let mut recovered = UnpublishedObjectReceipt::recover(&r.encode(), &receipt()).unwrap();
    assert_eq!(recovered.phase(), Phase::AwaitingReadback);
    assert!(recovered.candidate().is_none());
    assert_eq!(recovered.mark_attempt_started(), Err(ReceiptError::InvalidPhase));
    observe(&mut recovered).unwrap();
    assert_eq!(recovered.candidate().unwrap(), original);
}

#[test]
fn held_operation_does_not_reactivate_on_restart() {
    let mut r = receipt();
    r.mark_attempt_started().unwrap();
    r.hold_after_conflict();
    let mut recovered = UnpublishedObjectReceipt::recover(&r.encode(), &receipt()).unwrap();
    assert_eq!(recovered.phase(), Phase::Held);
    assert!(recovered.candidate().is_none());
    assert_eq!(recovered.mark_attempt_started(), Err(ReceiptError::InvalidPhase));
    assert_eq!(observe(&mut recovered), Err(ReceiptError::InvalidPhase));
}

#[test]
fn cannot_claim_head_as_an_uploaded_immutable_object() {
    assert_eq!(UnpublishedObjectReceipt::new(
        scope(), OP, ObjectKey::head(), &blake3::hash(CONTENT).to_hex().to_string()),
        Err(ReceiptError::InvalidIntent));
    assert_eq!(UnpublishedObjectReceipt::new(
        scope(), "forged", key(), &blake3::hash(CONTENT).to_hex().to_string()),
        Err(ReceiptError::InvalidIntent));
}

#[test]
fn tampered_intent_cannot_be_recovered_even_with_correct_scope() {
    let mut forged: Value = serde_json::from_slice(&receipt().encode()).unwrap();
    forged["operation_id"] = json!("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    assert_eq!(UnpublishedObjectReceipt::recover(
        &serde_json::to_vec(&forged).unwrap(), &receipt()),
        Err(ReceiptError::InvalidRecord));
    let mut forged: Value = serde_json::from_slice(&receipt().encode()).unwrap();
    forged["expected_blake3"] = json!("0".repeat(64));
    assert_eq!(UnpublishedObjectReceipt::recover(
        &serde_json::to_vec(&forged).unwrap(), &receipt()),
        Err(ReceiptError::InvalidRecord));
}
