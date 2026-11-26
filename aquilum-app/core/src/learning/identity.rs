//! Conservative identity reconciliation for authorized table snapshots.
//!
//! A raw Markdown table has no stable row IDs. This pure helper can reattach
//! PREVIOUSLY minted IDs only for unique, byte-identical rows of the SAME
//! trusted source table. Edited or duplicated rows are held for explicit
//! confirmation rather than inheriting someone else's review history.
//! No filesystem access, ID issuance, persistence or permissions are here.

use std::collections::BTreeSet;

pub(super) mod exact_match;
use exact_match::EvidenceIndex;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub vault_id: String,
    pub note_id: String,
    pub table_id: String,
}

/// Digest is matching evidence, not the permanent ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Digest(pub [u8; 32]);

pub fn digest(cells: &[String]) -> Digest {
    let mut hash = blake3::Hasher::new();
    hash.update(b"akula-lang-row-v1");
    hash.update(&(cells.len() as u64).to_le_bytes());
    for value in cells {
        hash.update(&(value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    Digest(*hash.finalize().as_bytes())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviousRow {
    pub id: String,
    /// Physical data row, starting at 1 (0 denotes the header).
    pub position: usize,
    pub digest: Digest,
}

#[derive(Clone, Debug)]
pub struct Previous {
    pub source: Source,
    pub rows: Vec<PreviousRow>,
}

pub struct Current<'a> {
    pub source: &'a Source,
    /// Data rows only; the caller removes the header.
    pub rows: &'a [Vec<String>],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidSource,
    SourceMismatch,
    InvalidPreviousRow,
    DuplicateRowId,
    DuplicatePreviousPosition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldReason {
    MissingOrEdited,
    Ambiguous,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attached {
    pub id: String,
    pub position: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    pub id: String,
    pub previous_position: usize,
    pub reason: HoldReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultRows {
    pub attached: Vec<Attached>,
    pub held: Vec<Held>,
    /// Still-unbound positions. Never auto-mint IDs for ambiguous rows.
    pub unbound: Vec<usize>,
}

fn validate(previous: &Previous, current: &Current<'_>) -> Result<(), Error> {
    let a = &previous.source;
    let b = current.source;
    let parts = [
        &a.vault_id, &a.note_id, &a.table_id,
        &b.vault_id, &b.note_id, &b.table_id,
    ];
    if parts.iter().any(|part| part.trim().is_empty()) {
        return Err(Error::InvalidSource);
    }
    if a != b {
        return Err(Error::SourceMismatch);
    }
    let mut ids = BTreeSet::new();
    let mut positions = BTreeSet::new();
    for row in &previous.rows {
        if row.id.trim().is_empty() || row.position == 0 {
            return Err(Error::InvalidPreviousRow);
        }
        if !ids.insert(&row.id) {
            return Err(Error::DuplicateRowId);
        }
        if !positions.insert(row.position) {
            return Err(Error::DuplicatePreviousPosition);
        }
    }
    Ok(())
}

/// Reattach only when a row digest is unique on both sides.
/// An edited value has no exact evidence and therefore stays on HOLD.
/// Matching by row index, English spelling or a partial translation is unsafe.
pub fn reconcile(previous: &Previous, current: Current<'_>) -> Result<ResultRows, Error> {
    validate(previous, &current)?;
    let digests: Vec<Digest> = current.rows.iter().map(|row| digest(row)).collect();
    let matches = EvidenceIndex::new(previous.rows.iter().map(|row| row.digest), &digests);
    let mut attached = Vec::new();
    let mut held = Vec::new();
    for row in &previous.rows {
        match matches.find(&row.digest, HoldReason::MissingOrEdited, HoldReason::Ambiguous) {
            Ok(index) => attached.push(Attached { id: row.id.clone(), position: index + 1 }),
            Err(reason) => held.push(Held { id: row.id.clone(), previous_position: row.position, reason }),
        }
    }
    let unbound = matches.unbound(&digests, 1);
    Ok(ResultRows { attached, held, unbound })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> Source {
        Source { vault_id: "vault-1".into(), note_id: "note-1".into(), table_id: "table-1".into() }
    }
    fn cells(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }
    fn old(source: &Source, rows: &[(&str, Vec<String>)]) -> Previous {
        Previous {
            source: source.clone(),
            rows: rows.iter().enumerate().map(|(i, (id, cells))| PreviousRow {
                id: (*id).into(), position: i + 1, digest: digest(cells),
            }).collect(),
        }
    }

    #[test]
    fn reorder_and_insert_keep_existing_unchanged_ids() {
        let s = source();
        let p = old(&s, &[("a", cells(&["eat", "есть"])), ("b", cells(&["ate", "съел"]))]);
        let rows = vec![cells(&["new", "новый"]), cells(&["ate", "съел"]), cells(&["eat", "есть"])];
        let r = reconcile(&p, Current { source: &s, rows: &rows }).unwrap();
        assert_eq!(r.attached, vec![
            Attached { id: "a".into(), position: 3 },
            Attached { id: "b".into(), position: 2 },
        ]);
        assert!(r.held.is_empty());
        assert_eq!(r.unbound, [1]);
    }

    #[test]
    fn changed_answer_never_inherits_prior_history_silently() {
        let s = source();
        let p = old(&s, &[("id", cells(&["run", "бегать"]))]);
        let rows = vec![cells(&["run", "управлять"])];
        let r = reconcile(&p, Current { source: &s, rows: &rows }).unwrap();
        assert!(r.attached.is_empty());
        assert_eq!(r.held[0].reason, HoldReason::MissingOrEdited);
        assert_eq!(r.unbound, [1]);
    }

    #[test]
    fn identical_old_rows_are_ambiguous_even_when_new_is_unique() {
        let s = source();
        let row = cells(&["night", "ночь"]);
        let p = old(&s, &[("a", row.clone()), ("b", row.clone())]);
        let rows = vec![row];
        let r = reconcile(&p, Current { source: &s, rows: &rows }).unwrap();
        assert!(r.attached.is_empty());
        assert_eq!(r.held.len(), 2);
        assert!(r.held.iter().all(|row| row.reason == HoldReason::Ambiguous));
        assert_eq!(r.unbound, [1]);
    }

    #[test]
    fn duplicate_new_rows_never_steal_an_old_id() {
        let s = source();
        let row = cells(&["night", "ночь"]);
        let p = old(&s, &[("a", row.clone())]);
        let rows = vec![row.clone(), row];
        let r = reconcile(&p, Current { source: &s, rows: &rows }).unwrap();
        assert!(r.attached.is_empty());
        assert_eq!(r.held[0].reason, HoldReason::Ambiguous);
        assert_eq!(r.unbound, [1, 2]);
    }

    #[test]
    fn run_with_different_senses_survives_reordering() {
        let s = source();
        let p = old(&s, &[
            ("walk", cells(&["run", "бегать", "exercise"])),
            ("work", cells(&["run", "работать", "program"])),
        ]);
        let rows = vec![
            cells(&["run", "работать", "program"]),
            cells(&["run", "бегать", "exercise"]),
        ];
        let r = reconcile(&p, Current { source: &s, rows: &rows }).unwrap();
        assert_eq!(r.attached[0].position, 2);
        assert_eq!(r.attached[1].position, 1);
        assert!(r.held.is_empty());
    }

    #[test]
    fn changing_vault_or_table_never_cross_binds() {
        let s = source();
        let p = old(&s, &[("id", cells(&["run", "бежать"]))]);
        let rows = vec![cells(&["run", "бежать"])];
        let other = Source { vault_id: "vault-2".into(), ..s.clone() };
        assert_eq!(reconcile(&p, Current { source: &other, rows: &rows }), Err(Error::SourceMismatch));
        let other = Source { table_id: "table-2".into(), ..s.clone() };
        assert_eq!(reconcile(&p, Current { source: &other, rows: &rows }), Err(Error::SourceMismatch));
    }

    #[test]
    fn corrupt_registry_fails_closed() {
        let s = source();
        let row = cells(&["eat", "есть"]);
        let rows = vec![row.clone()];
        let p = old(&s, &[("id", row.clone()), ("id", cells(&["ate", "ел"]))]);
        assert_eq!(reconcile(&p, Current { source: &s, rows: &rows }), Err(Error::DuplicateRowId));
        let mut p = old(&s, &[("a", row), ("b", cells(&["ate", "ел"]))]);
        p.rows[1].position = 1;
        assert_eq!(reconcile(&p, Current { source: &s, rows: &rows }), Err(Error::DuplicatePreviousPosition));
    }

    #[test]
    fn empty_source_or_row_id_never_produces_a_match() {
        let s = source();
        let row = cells(&["eat", "есть"]);
        let rows = vec![row.clone()];
        let p = old(&s, &[("a", row)]);
        let other = Source { note_id: String::new(), ..s.clone() };
        assert_eq!(reconcile(&p, Current { source: &other, rows: &rows }), Err(Error::InvalidSource));
        let mut p = p;
        p.rows[0].id.clear();
        assert_eq!(reconcile(&p, Current { source: &s, rows: &rows }), Err(Error::InvalidPreviousRow));
    }

    #[test]
    fn missing_row_and_changed_context_never_reassign_history() {
        let s = source();
        let p = old(&s, &[("run", cells(&["run", "управлять", "company"]))]);
        let removed = reconcile(&p, Current { source: &s, rows: &[] }).unwrap();
        assert!(removed.attached.is_empty());
        assert_eq!(removed.held[0].reason, HoldReason::MissingOrEdited);
        assert!(removed.unbound.is_empty());
        let changed = vec![cells(&["run", "управлять", "program"])];
        let next = reconcile(&p, Current { source: &s, rows: &changed }).unwrap();
        assert!(next.attached.is_empty());
        assert_eq!(next.held[0].reason, HoldReason::MissingOrEdited);
        assert_eq!(next.unbound, [1]);
    }

    #[test]
    fn distinct_cell_boundaries_and_unicode_do_not_collide() {
        assert_ne!(digest(&cells(&["ab", "c"])), digest(&cells(&["a", "bc"])));
        assert_ne!(digest(&cells(&["run|бегать"])), digest(&cells(&["run", "бегать"])));
        assert_ne!(digest(&cells(&["résumé"])), digest(&cells(&["resume"])));
    }
}
