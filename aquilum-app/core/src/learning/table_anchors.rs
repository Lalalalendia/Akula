//! Safe table ID reconciliation within one host-verified note.
//!
//! Markdown has no durable table identifiers. The trusted host must mint
//! table IDs, maintain stable vault/note identity and deliver authorized live
//! table snapshots. A byte fingerprint is evidence, NEVER a table ID.
//! Changed or duplicated tables are held until explicit confirmation.
//! This module does not read vault files, issue permissions or modify notes.

use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};

use super::editor_snapshot::{MAX_TABLE_CELL_BYTES, MAX_TABLE_COLUMNS, MAX_TABLE_ROWS};

const MAX_TABLES_PER_NOTE: usize = 256;
const MAX_TOTAL_CELL_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_REGISTERED_NOTES: usize = 1024;
pub const MAX_REGISTERED_TABLES: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteKey {
    pub vault_id: String,
    pub note_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableAnchor {
    /// Assigned and checked by the trusted host, not inferred from contents.
    pub table_id: String,
    /// Index among tables in this note, starting at 0. Ephemeral evidence.
    pub previous_index: usize,
    pub full_digest: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteAnchors {
    pub note: NoteKey,
    pub tables: Vec<TableAnchor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorError {
    InvalidIdentity,
    DuplicateNote,
    DuplicateTableId,
    DuplicatePreviousIndex,
    TooManyAnchors,
    NoteChanged,
    EmptyTable,
    RaggedTable,
    TooManyTables,
    TooManyRows,
    TooManyColumns,
    TooMuchText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldReason {
    MissingOrEdited,
    Ambiguous,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachedTable {
    pub table_id: String,
    pub current_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldTable {
    pub table_id: String,
    pub previous_index: usize,
    pub reason: HoldReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableReconciliation {
    pub attached: Vec<AttachedTable>,
    pub held: Vec<HeldTable>,
    pub unbound_indices: Vec<usize>,
}

fn id_valid(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= 128
}
fn note_valid(note: &NoteKey) -> bool {
    id_valid(&note.vault_id) && id_valid(&note.note_id)
}

pub fn validate_anchors(anchors: &[NoteAnchors]) -> Result<(), AnchorError> {
    if anchors.len() > MAX_REGISTERED_NOTES {
        return Err(AnchorError::TooManyAnchors);
    }
    let mut seen_notes = BTreeSet::new();
    let mut total_tables = 0usize;
    for record in anchors {
        if !note_valid(&record.note) {
            return Err(AnchorError::InvalidIdentity);
        }
        if !seen_notes.insert(&record.note) {
            return Err(AnchorError::DuplicateNote);
        }
        total_tables = total_tables.saturating_add(record.tables.len());
        if record.tables.len() > MAX_TABLES_PER_NOTE || total_tables > MAX_REGISTERED_TABLES {
            return Err(AnchorError::TooManyAnchors);
        }
        let mut ids = BTreeSet::new();
        let mut indices = BTreeSet::new();
        for table in &record.tables {
            if !id_valid(&table.table_id) {
                return Err(AnchorError::InvalidIdentity);
            }
            if !ids.insert(&table.table_id) {
                return Err(AnchorError::DuplicateTableId);
            }
            if !indices.insert(table.previous_index) {
                return Err(AnchorError::DuplicatePreviousIndex);
            }
        }
    }
    Ok(())
}

/// Hash the complete parsed table with length-prefixed UTF-8 cell boundaries.
/// A full-row or cell edit invalidates this automatic evidence; the host
/// needs a verified editor delta or explicit rebind to preserve the table ID.
fn evidence(cells: &[Vec<String>]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"akula-language-table-evidence-v1");
    hash.update(&(cells.len() as u64).to_le_bytes());
    for row in cells {
        hash.update(&(row.len() as u64).to_le_bytes());
        for cell in row {
            hash.update(&(cell.len() as u64).to_le_bytes());
            hash.update(cell.as_bytes());
        }
    }
    *hash.finalize().as_bytes()
}

pub fn fingerprint_tables(tables: &[Vec<Vec<String>>]) -> Result<Vec<[u8; 32]>, AnchorError> {
    if tables.len() > MAX_TABLES_PER_NOTE {
        return Err(AnchorError::TooManyTables);
    }
    let mut text_bytes = 0usize;
    let mut digests = Vec::with_capacity(tables.len());
    for rows in tables {
        if rows.len() < 2 || rows.first().is_none_or(Vec::is_empty) {
            return Err(AnchorError::EmptyTable);
        }
        if rows.len() > MAX_TABLE_ROWS {
            return Err(AnchorError::TooManyRows);
        }
        let width = rows[0].len();
        if width > MAX_TABLE_COLUMNS {
            return Err(AnchorError::TooManyColumns);
        }
        let mut table_bytes = 0usize;
        for row in rows {
            if row.len() != width {
                return Err(AnchorError::RaggedTable);
            }
            for cell in row {
                table_bytes = table_bytes.saturating_add(cell.len());
                text_bytes = text_bytes.saturating_add(cell.len());
                if table_bytes > MAX_TABLE_CELL_BYTES || text_bytes > MAX_TOTAL_CELL_BYTES {
                    return Err(AnchorError::TooMuchText);
                }
            }
        }
        digests.push(evidence(rows));
    }
    Ok(digests)
}

/// Reconcile only the same authenticated logical note identity.
///
/// Full fingerprints must be unique in BOTH snapshots. Do not match by
/// position, table title, header or a subset of values; those are ambiguous.
/// No persisted record is changed. User confirmation and sidecar CAS follow.
pub fn reconcile_tables(
    saved: &NoteAnchors,
    current_note: &NoteKey,
    current_tables: &[Vec<Vec<String>>],
) -> Result<TableReconciliation, AnchorError> {
    validate_anchors(std::slice::from_ref(saved))?;
    if !note_valid(current_note) {
        return Err(AnchorError::InvalidIdentity);
    }
    if saved.note != *current_note {
        return Err(AnchorError::NoteChanged);
    }
    let new_digests = fingerprint_tables(current_tables)?;
    let mut old_counts = BTreeMap::<[u8; 32], usize>::new();
    for anchor in &saved.tables {
        *old_counts.entry(anchor.full_digest).or_default() += 1;
    }
    let mut new_indices = BTreeMap::<[u8; 32], Vec<usize>>::new();
    for (index, digest) in new_digests.iter().enumerate() {
        new_indices.entry(*digest).or_default().push(index);
    }

    let mut attached = Vec::new();
    let mut held = Vec::new();
    for anchor in &saved.tables {
        let previous_count = old_counts.get(&anchor.full_digest).copied().unwrap_or(0);
        let matches = new_indices.get(&anchor.full_digest);
        if previous_count == 1 && matches.is_some_and(|m| m.len() == 1) {
            attached.push(AttachedTable {
                table_id: anchor.table_id.clone(),
                current_index: matches.expect("verified unique current table")[0],
            });
        } else {
            held.push(HeldTable {
                table_id: anchor.table_id.clone(),
                previous_index: anchor.previous_index,
                reason: if previous_count > 1 || matches.is_some_and(|m| m.len() > 1) {
                    HoldReason::Ambiguous
                } else {
                    HoldReason::MissingOrEdited
                },
            });
        }
    }
    let unbound_indices = new_digests.iter().enumerate().filter_map(|(index, digest)| {
        let old_unique = old_counts.get(digest).copied() == Some(1);
        let new_unique = new_indices.get(digest).is_some_and(|m| m.len() == 1);
        (!old_unique || !new_unique).then_some(index)
    }).collect();
    Ok(TableReconciliation { attached, held, unbound_indices })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(note: &str) -> NoteKey {
        NoteKey { vault_id: "vault-A".into(), note_id: note.into() }
    }
    fn tab(word: &str, meaning: &str) -> Vec<Vec<String>> {
        vec![
            vec!["Word".into(), "Meaning".into()],
            vec![word.into(), meaning.into()],
        ]
    }
    fn prior(note: &NoteKey, ids: &[&str], tables: &[Vec<Vec<String>>]) -> NoteAnchors {
        let fingerprints = fingerprint_tables(tables).unwrap();
        NoteAnchors {
            note: note.clone(),
            tables: ids.iter().enumerate().map(|(i, id)| TableAnchor {
                table_id: (*id).into(), previous_index: i, full_digest: fingerprints[i],
            }).collect(),
        }
    }

    #[test]
    fn sorting_and_insertion_preserve_unique_table_id_evidence() {
        let note = key("note-1");
        let a = tab("run", "бежать");
        let b = tab("eat", "есть");
        let saved = prior(&note, &["table-A", "table-B"], &[a.clone(), b.clone()]);
        let current = vec![tab("new", "новый"), b, a];
        let result = reconcile_tables(&saved, &note, &current).unwrap();
        assert_eq!(result.attached, vec![
            AttachedTable { table_id: "table-A".into(), current_index: 2 },
            AttachedTable { table_id: "table-B".into(), current_index: 1 },
        ]);
        assert!(result.held.is_empty());
        assert_eq!(result.unbound_indices, [0]);
    }

    #[test]
    fn editing_any_cell_holds_old_table_identity() {
        let note = key("note-1");
        let old = prior(&note, &["table-A"], &[tab("run", "бежать")]);
        let result = reconcile_tables(&old, &note, &[tab("run", "управлять")]).unwrap();
        assert!(result.attached.is_empty());
        assert_eq!(result.held[0].reason, HoldReason::MissingOrEdited);
        assert_eq!(result.unbound_indices, [0]);
    }

    #[test]
    fn duplicated_old_and_new_tables_are_ambiguous() {
        let note = key("note-1");
        let a = tab("run", "бежать");
        let prev = prior(&note, &["one", "two"], &[a.clone(), a.clone()]);
        let current = vec![a.clone()];
        let res = reconcile_tables(&prev, &note, &current).unwrap();
        assert_eq!(res.held.len(), 2);
        assert!(res.attached.is_empty());
        assert_eq!(res.unbound_indices, [0]);

        let prev = prior(&note, &["one"], std::slice::from_ref(&a));
        let res = reconcile_tables(&prev, &note, &[a.clone(), a]).unwrap();
        assert!(res.attached.is_empty());
        assert_eq!(res.held[0].reason, HoldReason::Ambiguous);
        assert_eq!(res.unbound_indices, [0, 1]);
    }

    #[test]
    fn wrong_note_or_vault_fails_without_leaking_an_id() {
        let source = key("note-1");
        let old = prior(&source, &["table-1"], &[tab("eat", "есть")]);
        let other = key("note-2");
        assert_eq!(reconcile_tables(&old, &other, &[]), Err(AnchorError::NoteChanged));
        let other_vault = NoteKey { vault_id: "vault-B".into(), ..source.clone() };
        assert_eq!(reconcile_tables(&old, &other_vault, &[]), Err(AnchorError::NoteChanged));
    }

    #[test]
    fn malformed_registry_is_rejected_before_matching() {
        let note = key("note-1");
        let a = tab("run", "бежать");
        let mut prev = prior(&note, &["first", "second"], &[a.clone(), tab("eat", "есть")]);
        prev.tables[1].table_id = "first".into();
        assert_eq!(reconcile_tables(&prev, &note, &[a.clone()]), Err(AnchorError::DuplicateTableId));
        prev.tables[1].table_id = "second".into();
        prev.tables[1].previous_index = 0;
        assert_eq!(reconcile_tables(&prev, &note, &[a]), Err(AnchorError::DuplicatePreviousIndex));
    }

    #[test]
    fn fingerprints_respect_cell_boundaries_and_unicode() {
        let a = tab("ab", "c");
        let b = tab("a", "bc");
        assert_ne!(fingerprint_tables(&[a]).unwrap(), fingerprint_tables(&[b]).unwrap());
        let composed = tab("résumé", "резюме");
        let plain = tab("resume", "резюме");
        assert_ne!(fingerprint_tables(&[composed]).unwrap(), fingerprint_tables(&[plain]).unwrap());
    }

    #[test]
    fn text_budget_and_ragged_table_fail_closed() {
        let mut oversized = tab("word", "meaning");
        oversized[1][1] = "x".repeat(MAX_TABLE_CELL_BYTES + 1);
        assert_eq!(fingerprint_tables(&[oversized]), Err(AnchorError::TooMuchText));
        let ragged = vec![vec!["Word".into(), "Meaning".into()], vec!["word".into()]];
        assert_eq!(fingerprint_tables(&[ragged]), Err(AnchorError::RaggedTable));
    }

    #[test]
    fn identical_table_rename_does_not_depend_on_file_path() {
        let note = key("opaque-document-id");
        let table = tab("eat", "есть");
        let old = prior(&note, &["stable-table"], std::slice::from_ref(&table));
        let result = reconcile_tables(&old, &note, &[table]).unwrap();
        assert_eq!(result.attached[0].table_id, "stable-table");
    }
}
