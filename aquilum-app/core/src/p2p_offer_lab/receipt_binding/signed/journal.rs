//! QA-only SQLite consumed-proof fence for cooperating local writers.
//!
//! Call only after verify_cold_receipt_v2 has checked an independently pinned
//! endpoint pair and freshly rehashed local file. The Connection's database
//! must be independently protected and pinned by its OS custodian. SQLite/WAL
//! alone does NOT resist hostile DB rollback, path swaps, or stolen device keys.
//! Never grant UI, file-open, Vault import or deletion rights from this table.
use super::CheckedColdReceipt;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

#[derive(Debug)]
pub enum JournalError {
    Database(rusqlite::Error),
    Conflict,
    NotRegistered,
    AlreadyConsumed,
}

impl From<rusqlite::Error> for JournalError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

/// Initialize only a specifically selected QA database; no paths or directories
/// are created here. WAL plus FULL sync protects cooperating crashes but cannot
/// supply OS-owned custody or anti-rollback security.
pub fn initialize(db: &Connection) -> Result<(), JournalError> {
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;
         CREATE TABLE IF NOT EXISTS p2p_signed_proof_qa (
            receipt_id TEXT PRIMARY KEY NOT NULL,
            offer_id TEXT NOT NULL,
            selected_index INTEGER NOT NULL CHECK(selected_index >= 0 AND selected_index < 8),
            manifest BLOB NOT NULL CHECK(length(manifest) = 32),
            proof_digest BLOB NOT NULL CHECK(length(proof_digest) = 32),
            consumed INTEGER NOT NULL DEFAULT 0 CHECK(consumed IN (0,1)),
            UNIQUE(offer_id, selected_index)
         );",
    )?;
    Ok(())
}

type Stored = (String, i64, Vec<u8>, Vec<u8>, i64);

fn checked_row(
    tx: &rusqlite::Transaction<'_>,
    proof: CheckedColdReceipt,
) -> Result<Option<Stored>, JournalError> {
    let row = tx
        .query_row(
            "SELECT offer_id, selected_index, manifest, proof_digest, consumed
             FROM p2p_signed_proof_qa WHERE receipt_id = ?1",
            [proof.receipt_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?;
    Ok(row)
}

fn validate_row(stored: &Stored, proof: CheckedColdReceipt) -> Result<(), JournalError> {
    if stored.0 != proof.offer_id.to_string()
        || stored.1 != i64::from(proof.selected_index)
        || stored.2.as_slice() != &proof.manifest_commitment[..]
        || stored.3.as_slice() != &proof.proof_digest[..]
        || !matches!(stored.4, 0 | 1)
    {
        return Err(JournalError::Conflict);
    }
    Ok(())
}

/// Register a previously verified immutable proof. Re-registration before use
/// is idempotent only for byte-identical signed proof digests and one offer/index.
pub fn register_verified(
    db: &mut Connection,
    checked: CheckedColdReceipt,
) -> Result<(), JournalError> {
    let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute(
        "INSERT OR IGNORE INTO p2p_signed_proof_qa
         (receipt_id, offer_id, selected_index, manifest, proof_digest, consumed)
         VALUES (?1, ?2, ?3, ?4, ?5, 0)",
        params![
            checked.receipt_id.to_string(),
            checked.offer_id.to_string(),
            i64::from(checked.selected_index),
            &checked.manifest_commitment[..],
            &checked.proof_digest[..],
        ],
    )?;
    let row = checked_row(&transaction, checked)?.ok_or(JournalError::Conflict)?;
    validate_row(&row, checked)?;
    if row.4 != 0 {
        return Err(JournalError::AlreadyConsumed);
    }
    transaction.commit()?;
    Ok(())
}

/// Commit consumed=1 atomically BEFORE any external side effect. After a crash,
/// never reset this row automatically: retain the Inbox original and request
/// manual review instead of replaying an uncertain Vault action.
pub fn consume_once(db: &mut Connection, checked: CheckedColdReceipt) -> Result<(), JournalError> {
    let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row = checked_row(&transaction, checked)?.ok_or(JournalError::NotRegistered)?;
    validate_row(&row, checked)?;
    if row.4 != 0 {
        return Err(JournalError::AlreadyConsumed);
    }
    let changed = transaction.execute(
        "UPDATE p2p_signed_proof_qa
         SET consumed = 1 WHERE receipt_id = ?1 AND consumed = 0",
        [checked.receipt_id.to_string()],
    )?;
    if changed != 1 {
        return Err(JournalError::AlreadyConsumed);
    }
    transaction.commit()?;
    Ok(())
}
