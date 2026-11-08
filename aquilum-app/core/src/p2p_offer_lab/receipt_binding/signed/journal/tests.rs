//! Independent SQLite connection tests, not OS-owned anti-rollback evidence.
use super::*;
use uuid::Uuid;

fn checked() -> CheckedColdReceipt {
    CheckedColdReceipt {
        offer_id: Uuid::from_u128(11),
        receipt_id: Uuid::from_u128(22),
        selected_index: 0,
        manifest_commitment: [3; 32],
        proof_digest: [4; 32],
    }
}

#[test]
fn two_open_sqlite_handles_cannot_consume_same_receipt_twice() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("signed-proof-qa.sqlite3");
    let mut first = Connection::open(&path).unwrap();
    initialize(&first).unwrap();
    let mut second = Connection::open(&path).unwrap();
    initialize(&second).unwrap();
    let proof = checked();

    register_verified(&mut first, proof).unwrap();
    register_verified(&mut second, proof).unwrap();
    consume_once(&mut second, proof).unwrap();
    assert!(matches!(
        consume_once(&mut first, proof),
        Err(JournalError::AlreadyConsumed)
    ));

    drop(first);
    drop(second);
    let mut cold = Connection::open(&path).unwrap();
    initialize(&cold).unwrap();
    assert!(matches!(
        consume_once(&mut cold, proof),
        Err(JournalError::AlreadyConsumed)
    ));
}

#[test]
fn independent_handle_refuses_offer_alias_and_changed_proof() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("signed-proof-qa.sqlite3");
    let mut first = Connection::open(&path).unwrap();
    initialize(&first).unwrap();
    let mut second = Connection::open(&path).unwrap();
    initialize(&second).unwrap();
    let proof = checked();

    register_verified(&mut first, proof).unwrap();
    let mut alias = proof;
    alias.receipt_id = Uuid::from_u128(23);
    assert!(matches!(
        register_verified(&mut second, alias),
        Err(JournalError::Conflict)
    ));

    let mut changed = proof;
    changed.proof_digest[0] ^= 1;
    assert!(matches!(
        consume_once(&mut second, changed),
        Err(JournalError::Conflict)
    ));
    consume_once(&mut first, proof).unwrap();
    assert!(matches!(
        register_verified(&mut second, proof),
        Err(JournalError::AlreadyConsumed)
    ));
}
