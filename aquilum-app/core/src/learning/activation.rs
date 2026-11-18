//! Fail-closed learning set assembly from two metadata registries.
//!
//! This module is NOT an access-control broker. Only the trusted Akula host
//! can provide complete current table snapshots of authorized live notes,
//! with its DocumentHub revision held or rechecked before exercises begin.
//! Neither a persisted Source string nor a fingerprint grants note access.
//!
//! Saved membership without a current, unambiguous table anchor yields an
//! unavailable item, never copied or guessed text. Metadata saves may be
//! non-atomic across the two SQLite files: this check refuses partial joins.

use std::collections::{BTreeMap, BTreeSet};

use super::collections::{
    self, Collection, CollectionError, Limits, Registry, StudySet, TableSnapshot,
};
use super::identity::Source;
use super::table_anchors::{
    self, AnchorError, NoteAnchors, NoteKey,
};

/// All parsed table models of ONE current host-authorized document snapshot.
/// Must contain all tables, not only requested ones: otherwise duplicate
/// evidence elsewhere in the note could silently steal a stable ID.
pub struct LiveNote<'a> {
    pub note: &'a NoteKey,
    pub tables: &'a [Vec<Vec<String>>],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActivationError {
    InvalidCollection(CollectionError),
    InvalidAnchors(AnchorError),
    InvalidLiveNote,
    DuplicateLiveNote,
}

/// The host must have checked read authorization for every LiveNote,
/// canonical document ownership and its *current* revision. Caller must
/// recheck both source-store revisions if they may change concurrently.
///
/// Unbound, moved-with-edits, absent and ambiguous tables are never passed
/// to the study projection. This helper performs no filesystem or DB IO.
pub fn assemble_verified(
    registry: &Registry,
    anchor_notes: &[NoteAnchors],
    collection_ids: &[String],
    live_notes: &[LiveNote<'_>],
    limits: Limits,
) -> Result<StudySet, ActivationError> {
    collections::validate(registry).map_err(ActivationError::InvalidCollection)?;
    table_anchors::validate_anchors(anchor_notes).map_err(ActivationError::InvalidAnchors)?;

    let mut desired = BTreeSet::<Source>::new();
    for id in collection_ids {
        let group: &Collection = registry.collections.iter()
            .find(|group| group.id == *id)
            .ok_or(ActivationError::InvalidCollection(CollectionError::UnknownCollection))?;
        desired.extend(group.members.iter().cloned());
    }
    if desired.len() > limits.max_tables {
        return Err(ActivationError::InvalidCollection(CollectionError::TooManyTables));
    }

    let anchor_map: BTreeMap<NoteKey, &NoteAnchors> =
        anchor_notes.iter().map(|record| (record.note.clone(), record)).collect();
    let mut live_map = BTreeMap::<NoteKey, &[Vec<Vec<String>>]>::new();
    for live in live_notes {
        let key = live.note;
        if key.vault_id.trim().is_empty() || key.note_id.trim().is_empty()
            || key.vault_id.len() > 128 || key.note_id.len() > 128
        {
            return Err(ActivationError::InvalidLiveNote);
        }
        if live_map.insert(key.clone(), live.tables).is_some() {
            return Err(ActivationError::DuplicateLiveNote);
        }
    }

    let selected_notes: BTreeSet<NoteKey> = desired.iter().map(|source| NoteKey {
        vault_id: source.vault_id.clone(),
        note_id: source.note_id.clone(),
    }).collect();

    let mut available = Vec::<TableSnapshot<'_>>::new();
    for note in selected_notes {
        let (Some(saved), Some(current)) = (anchor_map.get(&note), live_map.get(&note)) else {
            continue;
        };
        let matches = table_anchors::reconcile_tables(saved, &note, current)
            .map_err(ActivationError::InvalidAnchors)?;
        for matched in matches.attached {
            let source = Source {
                vault_id: note.vault_id.clone(),
                note_id: note.note_id.clone(),
                table_id: matched.table_id,
            };
            if let Some(owned) = desired.get(&source) {
                // current_index came from the same complete table array.
                available.push(TableSnapshot {
                    source: owned,
                    rows: &current[matched.current_index],
                });
            }
        }
    }
    // The existing assembler reports all unmatched source IDs as unavailable.
    // It cannot open notes or override a missing/ambiguous binding.
    collections::assemble(registry, collection_ids, &available, limits)
        .map_err(ActivationError::InvalidCollection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{ColumnMapping, collections::ConfiguredTable};
    use super::super::table_anchors::{TableAnchor, fingerprint_tables};

    fn note(id: &str) -> NoteKey {
        NoteKey { vault_id: "vault-1".into(), note_id: id.into() }
    }
    fn data(word: &str, answer: &str) -> Vec<Vec<String>> {
        vec![
            vec!["Q".into(), "A".into()],
            vec![word.into(), answer.into()],
        ]
    }
    fn anchored(n: &NoteKey, tables: &[Vec<Vec<String>>], ids: &[&str]) -> NoteAnchors {
        let digests = fingerprint_tables(tables).unwrap();
        NoteAnchors {
            note: n.clone(),
            tables: ids.iter().enumerate().map(|(i, id)| TableAnchor {
                table_id: (*id).into(),
                previous_index: i,
                full_digest: digests[i],
            }).collect(),
        }
    }
    fn source(n: &NoteKey, table_id: &str) -> Source {
        Source {
            vault_id: n.vault_id.clone(),
            note_id: n.note_id.clone(),
            table_id: table_id.into(),
        }
    }
    fn cfg(s: Source) -> ConfiguredTable {
        ConfiguredTable {
            source: s,
            columns: ColumnMapping { question: 0, answers: vec![1], context: None },
        }
    }
    fn registry(choices: &[(&str, Vec<Source>)]) -> Registry {
        let mut tables = BTreeSet::new();
        for (_, items) in choices {
            tables.extend(items.iter().cloned());
        }
        Registry {
            collections: choices.iter().map(|(id, members)| Collection {
                id: (*id).into(), name: (*id).into(), members: members.clone(),
            }).collect(),
            tables: tables.into_iter().map(cfg).collect(),
        }
    }
    fn limits() -> Limits {
        Limits { max_tables: 32, max_rows_per_table: 16 }
    }

    #[test]
    fn four_tables_three_notes_two_collections_use_only_verified_anchors() {
        let n1 = note("A");
        let n2 = note("B");
        let n3 = note("C");
        let a = data("eat", "есть");
        let b = data("ate", "съел");
        let c = data("read", "читать");
        let d = data("write", "писать");
        let s1 = source(&n1, "t1");
        let s2 = source(&n1, "t2");
        let s3 = source(&n2, "t3");
        let s4 = source(&n3, "t4");
        let registry = registry(&[
            ("English", vec![s1, s2.clone(), s3]),
            ("Verbs", vec![s2, s4]),
        ]);
        let n1_old = [a.clone(), b.clone()];
        let n1_now = [b, a]; // moved table, same verified full content
        let n2_tables = [c];
        let n3_tables = [d];
        let anchors = [
            anchored(&n1, &n1_old, &["t1", "t2"]),
            anchored(&n2, &n2_tables, &["t3"]),
            anchored(&n3, &n3_tables, &["t4"]),
        ];
        let notes = [
            LiveNote { note: &n1, tables: &n1_now },
            LiveNote { note: &n2, tables: &n2_tables },
            LiveNote { note: &n3, tables: &n3_tables },
        ];
        let output = assemble_verified(
            &registry, &anchors,
            &["English".into(), "Verbs".into(), "English".into()],
            &notes, limits(),
        ).unwrap();
        assert_eq!(output.item_count(), 4);
        assert_eq!(output.tables.len(), 4);
        assert!(output.unavailable.is_empty());
    }

    #[test]
    fn edited_table_stays_unavailable_not_reassigned_to_old_history() {
        let n = note("A");
        let old = [data("run", "бегать")];
        let changed = [data("run", "управлять")];
        let anchor = anchored(&n, &old, &["stable"]);
        let registry = registry(&[("English", vec![source(&n, "stable")])]);
        let set = assemble_verified(
            &registry, &[anchor], &["English".into()],
            &[LiveNote { note: &n, tables: &changed }], limits(),
        ).unwrap();
        assert!(set.tables.is_empty());
        assert_eq!(set.unavailable, [source(&n, "stable")]);
    }

    #[test]
    fn duplicate_table_content_held_even_if_only_one_is_member() {
        let n = note("A");
        let table = data("run", "бегать");
        let old = [table.clone()];
        let current = [table.clone(), table];
        let registry = registry(&[("English", vec![source(&n, "stable")])]);
        let set = assemble_verified(
            &registry, &[anchored(&n, &old, &["stable"])],
            &["English".into()],
            &[LiveNote { note: &n, tables: &current }], limits(),
        ).unwrap();
        assert!(set.tables.is_empty());
        assert_eq!(set.unavailable.len(), 1);
    }

    #[test]
    fn missing_anchor_or_denied_note_cannot_leak_cell_contents() {
        let n = note("A");
        let tables = [data("private", "не раскрывать")];
        let registry = registry(&[("English", vec![source(&n, "stable")])]);
        let no_anchor = assemble_verified(
            &registry, &[], &["English".into()],
            &[LiveNote { note: &n, tables: &tables }], limits(),
        ).unwrap();
        assert_eq!(no_anchor.unavailable.len(), 1);
        assert_eq!(no_anchor.item_count(), 0);
        let no_permission = assemble_verified(
            &registry, &[anchored(&n, &tables, &["stable"])],
            &["English".into()], &[], limits(),
        ).unwrap();
        assert_eq!(no_permission.unavailable.len(), 1);
        assert_eq!(no_permission.item_count(), 0);
    }

    #[test]
    fn another_vaults_table_never_satisfies_requested_source() {
        let n = note("A");
        let foreign = NoteKey { vault_id: "vault-2".into(), ..n.clone() };
        let table = [data("secret", "hidden")];
        let reg = registry(&[("English", vec![source(&n, "t1")])]);
        let anchors = [anchored(&n, &table, &["t1"])];
        let output = assemble_verified(
            &reg, &anchors, &["English".into()],
            &[LiveNote { note: &foreign, tables: &table }], limits(),
        ).unwrap();
        assert!(output.tables.is_empty());
        assert_eq!(output.unavailable, [source(&n, "t1")]);
    }

    #[test]
    fn duplicate_live_snapshot_fails_closed() {
        let n = note("A");
        let table = [data("run", "бежать")];
        let reg = registry(&[("English", vec![source(&n, "t1")])]);
        let anchors = [anchored(&n, &table, &["t1"])];
        let notes = [
            LiveNote { note: &n, tables: &table },
            LiveNote { note: &n, tables: &table },
        ];
        assert!(matches!(
            assemble_verified(&reg, &anchors, &["English".into()], &notes, limits()),
            Err(ActivationError::DuplicateLiveNote)
        ));
    }

    #[test]
    fn corrupt_anchor_registry_is_rejected_even_if_content_matches() {
        let n = note("A");
        let table = [data("eat", "есть")];
        let reg = registry(&[("English", vec![source(&n, "t1")])]);
        let mut anchor = anchored(&n, &table, &["t1"]);
        anchor.tables.push(anchor.tables[0].clone());
        assert_eq!(
            assemble_verified(
                &reg, &[anchor], &["English".into()],
                &[LiveNote { note: &n, tables: &table }], limits(),
            ),
            Err(ActivationError::InvalidAnchors(AnchorError::DuplicateTableId))
        );
    }

    #[test]
    fn selected_table_limit_applies_before_any_live_table_hashing() {
        let n = note("A");
        let a = source(&n, "t1");
        let b = source(&n, "t2");
        let reg = registry(&[("English", vec![a, b])]);
        assert_eq!(
            assemble_verified(
                &reg, &[], &["English".into()], &[],
                Limits { max_tables: 1, max_rows_per_table: 10 },
            ),
            Err(ActivationError::InvalidCollection(CollectionError::TooManyTables))
        );
    }
}
