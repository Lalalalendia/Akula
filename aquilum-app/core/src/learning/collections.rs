//! LANG-02: transient study collections over trusted in-memory table snapshots.
//!
//! Collection membership stores ONLY table identities. No vocabulary text is
//! duplicated into a collection; source content is supplied afresh by the host.
//! This module cannot grant file permissions, open notes, or persist a registry.
//! The host must resolve current authorized snapshots through its normal
//! DocumentHub/File Gate boundary before calling assemble().

use std::collections::{BTreeMap, BTreeSet};

use super::identity::Source;
use super::{preview_table, ColumnMapping, MappingError, TablePreview};

/// User-named group of source table IDs. One table can join multiple groups.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub members: Vec<Source>,
}

/// Mapping belongs to the source, not to each collection membership.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredTable {
    pub source: Source,
    pub columns: ColumnMapping,
}

/// Only metadata: never store word/translation/cell contents here.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub collections: Vec<Collection>,
    pub tables: Vec<ConfiguredTable>,
}

/// A currently available and authorized table already parsed by the host.
/// This is NOT an authorization token. Constructing it performs no file IO.
pub struct TableSnapshot<'a> {
    pub source: &'a Source,
    pub rows: &'a [Vec<String>],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_tables: usize,
    pub max_rows_per_table: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollectionError {
    EmptyCollectionId,
    EmptyCollectionName,
    DuplicateCollectionId,
    EmptySourceId,
    DuplicateMembership,
    DuplicateTableMapping,
    MissingTableMapping,
    UnknownCollection,
    DuplicateSnapshot,
    TooManyTables,
    InvalidColumns {
        source: Source,
        reason: MappingError,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedTable {
    pub source: Source,
    /// Ephemeral snapshot projection. Row offsets are NOT review-history IDs.
    pub preview: TablePreview,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudySet {
    pub tables: Vec<SelectedTable>,
    /// Membership remains unchanged when source is absent or access revoked.
    pub unavailable: Vec<Source>,
}

impl StudySet {
    pub fn item_count(&self) -> usize {
        self.tables.iter().map(|table| table.preview.items.len()).sum()
    }
}

fn valid_source(source: &Source) -> bool {
    !source.vault_id.trim().is_empty()
        && !source.note_id.trim().is_empty()
        && !source.table_id.trim().is_empty()
}

/// Validate a sidecar registry from the trusted host before using it.
/// A duplicate membership within a single collection is malformed. A shared
/// membership between DIFFERENT collections is legitimate.
pub fn validate(registry: &Registry) -> Result<(), CollectionError> {
    let mut groups = BTreeSet::new();
    for collection in &registry.collections {
        if collection.id.trim().is_empty() {
            return Err(CollectionError::EmptyCollectionId);
        }
        if collection.name.trim().is_empty() {
            return Err(CollectionError::EmptyCollectionName);
        }
        if !groups.insert(collection.id.as_str()) {
            return Err(CollectionError::DuplicateCollectionId);
        }
        let mut members = BTreeSet::new();
        for source in &collection.members {
            if !valid_source(source) {
                return Err(CollectionError::EmptySourceId);
            }
            if !members.insert(source) {
                return Err(CollectionError::DuplicateMembership);
            }
        }
    }

    let mut mapped = BTreeSet::new();
    for table in &registry.tables {
        if !valid_source(&table.source) {
            return Err(CollectionError::EmptySourceId);
        }
        if !mapped.insert(&table.source) {
            return Err(CollectionError::DuplicateTableMapping);
        }
    }
    for collection in &registry.collections {
        for member in &collection.members {
            if !mapped.contains(member) {
                return Err(CollectionError::MissingTableMapping);
            }
        }
    }
    Ok(())
}

/// Build a union of selected collections without copying source tables.
///
/// Source rows come only from the caller's current snapshots. Missing or
/// revoked sources are reported without exposing content. The function never
/// reads a document, creates a permission grant or writes to Markdown.
/// The selected table set is bounded before any projection work is performed.
pub fn assemble(
    registry: &Registry,
    selected_collection_ids: &[String],
    snapshots: &[TableSnapshot<'_>],
    limits: Limits,
) -> Result<StudySet, CollectionError> {
    validate(registry)?;

    let groups: BTreeMap<&str, &Collection> =
        registry.collections.iter().map(|group| (group.id.as_str(), group)).collect();
    let mappings: BTreeMap<&Source, &ColumnMapping> =
        registry.tables.iter().map(|table| (&table.source, &table.columns)).collect();

    let mut requested = BTreeSet::new();
    for id in selected_collection_ids {
        let group = groups.get(id.as_str()).ok_or(CollectionError::UnknownCollection)?;
        requested.extend(group.members.iter().cloned());
    }
    if requested.len() > limits.max_tables {
        return Err(CollectionError::TooManyTables);
    }

    let mut available = BTreeMap::new();
    for snapshot in snapshots {
        if !valid_source(snapshot.source) {
            return Err(CollectionError::EmptySourceId);
        }
        if available.insert(snapshot.source, snapshot.rows).is_some() {
            return Err(CollectionError::DuplicateSnapshot);
        }
    }

    let mut tables = Vec::new();
    let mut unavailable = Vec::new();
    for source in requested {
        let Some(rows) = available.get(&source) else {
            unavailable.push(source);
            continue;
        };
        let columns = mappings.get(&source).ok_or(CollectionError::MissingTableMapping)?;
        let preview = preview_table(rows, columns, limits.max_rows_per_table)
            .map_err(|reason| CollectionError::InvalidColumns {
                source: source.clone(),
                reason,
            })?;
        tables.push(SelectedTable { source, preview });
    }
    Ok(StudySet { tables, unavailable })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(note: &str, table: &str) -> Source {
        Source {
            vault_id: "vault-1".into(),
            note_id: note.into(),
            table_id: table.into(),
        }
    }

    fn cells(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter().map(|row| row.iter().map(|c| (*c).into()).collect()).collect()
    }

    fn cfg(id: Source) -> ConfiguredTable {
        ConfiguredTable {
            source: id,
            columns: ColumnMapping { question: 0, answers: vec![1], context: None },
        }
    }

    fn limits() -> Limits {
        Limits { max_tables: 100, max_rows_per_table: 50 }
    }
    #[test]
    fn four_tables_three_notes_two_collections_one_shared_source() {
        let a = source("lesson", "a");
        let b = source("lesson", "b");
        let c = source("verbs", "c");
        let d = source("literature", "d");
        let registry = Registry {
            collections: vec![
                Collection {
                    id: "English".into(), name: "Английский".into(),
                    members: vec![a.clone(), b.clone(), c.clone()],
                },
                Collection {
                    id: "Subjects".into(), name: "Темы и жанры".into(),
                    members: vec![b.clone(), d.clone()],
                },
            ],
            tables: vec![cfg(a.clone()), cfg(b.clone()), cfg(c.clone()), cfg(d.clone())],
        };
        let rows = cells(&[&["Term", "Meaning"], &["word", "слово"]]);
        let snapshots = [
            TableSnapshot { source: &a, rows: &rows },
            TableSnapshot { source: &b, rows: &rows },
            TableSnapshot { source: &c, rows: &rows },
            TableSnapshot { source: &d, rows: &rows },
        ];
        let set = assemble(
            &registry, &["English".into(), "Subjects".into(), "English".into()],
            &snapshots, limits(),
        ).unwrap();
        assert_eq!(set.tables.len(), 4);
        assert_eq!(set.item_count(), 4);
        assert!(set.unavailable.is_empty());
        assert_eq!(registry.collections[0].members.len(), 3);
    }
    #[test]
    fn changed_source_cell_is_visible_without_registry_duplication() {
        let key = source("lesson", "a");
        let registry = Registry {
            collections: vec![Collection { id: "words".into(), name: "Слова".into(), members: vec![key.clone()] }],
            tables: vec![cfg(key.clone())],
        };
        let old = cells(&[&["English", "Russian"], &["eat", "есть"]]);
        let new = cells(&[&["English", "Russian"], &["eat", "кушать"]]);
        let old_set = assemble(
            &registry, &["words".into()],
            &[TableSnapshot { source: &key, rows: &old }], limits(),
        ).unwrap();
        let new_set = assemble(
            &registry, &["words".into()],
            &[TableSnapshot { source: &key, rows: &new }], limits(),
        ).unwrap();
        assert_eq!(old_set.tables[0].preview.items[0].accepted_answers[0], "есть");
        assert_eq!(new_set.tables[0].preview.items[0].accepted_answers[0], "кушать");
        assert_eq!(registry.tables.len(), 1);
        assert_eq!(old[1][1], "есть");
    }

    #[test]
    fn absent_source_remains_in_membership_but_is_not_leaked() {
        let key = source("private-note", "t1");
        let registry = Registry {
            collections: vec![Collection { id: "vocab".into(), name: "Terms".into(), members: vec![key.clone()] }],
            tables: vec![cfg(key.clone())],
        };
        let set = assemble(&registry, &["vocab".into()], &[], limits()).unwrap();
        assert!(set.tables.is_empty());
        assert_eq!(set.unavailable, [key.clone()]);
        assert_eq!(registry.collections[0].members, [key]);
    }

    #[test]
    fn source_identity_includes_vault_and_note() {
        let key = source("note-a", "t1");
        let wrong_note = source("note-b", "t1");
        let wrong_vault = Source { vault_id: "another".into(), ..key.clone() };
        let registry = Registry {
            collections: vec![Collection { id: "vocab".into(), name: "Terms".into(), members: vec![key.clone()] }],
            tables: vec![cfg(key.clone())],
        };
        let data = cells(&[&["Q", "A"], &["secret", "never show"]]);
        let set = assemble(
            &registry, &["vocab".into()],
            &[
                TableSnapshot { source: &wrong_note, rows: &data },
                TableSnapshot { source: &wrong_vault, rows: &data },
            ], limits(),
        ).unwrap();
        assert!(set.tables.is_empty());
        assert_eq!(set.unavailable, [key]);
    }

    #[test]
    fn selected_collection_must_exist() {
        let registry = Registry { collections: vec![], tables: vec![] };
        assert_eq!(
            assemble(&registry, &["missing".into()], &[], limits()),
            Err(CollectionError::UnknownCollection)
        );
    }

    #[test]
    fn duplicate_mapping_or_membership_fails_closed() {
        let key = source("lesson", "table");
        let group = Collection { id: "one".into(), name: "One".into(), members: vec![key.clone()] };
        let registry = Registry {
            collections: vec![group.clone()],
            tables: vec![cfg(key.clone()), cfg(key.clone())],
        };
        assert_eq!(validate(&registry), Err(CollectionError::DuplicateTableMapping));
        let registry = Registry {
            collections: vec![Collection { members: vec![key.clone(), key.clone()], ..group }],
            tables: vec![cfg(key)],
        };
        assert_eq!(validate(&registry), Err(CollectionError::DuplicateMembership));
    }

    #[test]
    fn duplicate_snapshot_fails_even_if_both_have_identical_content() {
        let key = source("lesson", "table");
        let registry = Registry {
            collections: vec![Collection { id: "c".into(), name: "C".into(), members: vec![key.clone()] }],
            tables: vec![cfg(key.clone())],
        };
        let data = cells(&[&["Q", "A"], &["eat", "есть"]]);
        assert_eq!(
            assemble(
                &registry, &["c".into()],
                &[TableSnapshot { source: &key, rows: &data }, TableSnapshot { source: &key, rows: &data }],
                limits(),
            ),
            Err(CollectionError::DuplicateSnapshot)
        );
    }

    #[test]
    fn user_defined_question_column_and_multilingual_names_work() {
        let key = source("biology", "terms");
        let registry = Registry {
            collections: vec![Collection { id: "علوم".into(), name: "Биология".into(), members: vec![key.clone()] }],
            tables: vec![ConfiguredTable {
                source: key.clone(),
                columns: ColumnMapping { question: 1, answers: vec![0], context: Some(2) },
            }],
        };
        let data = cells(&[&["Description", "Term", "Example"], &["органоид", "Рибосома", "клетка"]]);
        let set = assemble(
            &registry, &["علوم".into()],
            &[TableSnapshot { source: &key, rows: &data }], limits(),
        ).unwrap();
        assert_eq!(set.tables[0].preview.items[0].question, "Рибосома");
        assert_eq!(set.tables[0].preview.items[0].accepted_answers, ["органоид"]);
        assert_eq!(set.tables[0].preview.items[0].context.as_deref(), Some("клетка"));
    }

    #[test]
    fn table_limit_applied_before_any_materialization() {
        let a = source("n", "a");
        let b = source("n", "b");
        let registry = Registry {
            collections: vec![Collection { id: "c".into(), name: "C".into(), members: vec![a.clone(), b.clone()] }],
            tables: vec![cfg(a), cfg(b)],
        };
        assert_eq!(
            assemble(&registry, &["c".into()], &[], Limits { max_tables: 1, max_rows_per_table: 100 }),
            Err(CollectionError::TooManyTables)
        );
    }

    #[test]
    fn malformed_mapping_or_empty_source_is_rejected() {
        let key = source("lesson", "a");
        let registry = Registry {
            collections: vec![Collection { id: "c".into(), name: "C".into(), members: vec![key.clone()] }],
            tables: vec![ConfiguredTable {
                source: key.clone(),
                columns: ColumnMapping { question: 0, answers: vec![0], context: None },
            }],
        };
        let data = cells(&[&["Q", "A"], &["eat", "есть"]]);
        assert_eq!(
            assemble(&registry, &["c".into()], &[TableSnapshot { source: &key, rows: &data }], limits()),
            Err(CollectionError::InvalidColumns {
                source: key.clone(), reason: MappingError::ColumnUsedTwice { index: 0 },
            })
        );
        let mut broken = registry;
        broken.collections[0].members[0].table_id.clear();
        assert_eq!(validate(&broken), Err(CollectionError::EmptySourceId));
    }

    #[test]
    fn row_bound_is_preserved_through_collection_projection() {
        let key = source("n", "t");
        let registry = Registry {
            collections: vec![Collection { id: "c".into(), name: "C".into(), members: vec![key.clone()] }],
            tables: vec![cfg(key.clone())],
        };
        let data = cells(&[&["Q", "A"], &["eat", "есть"], &["ate", "ел"], &["run", "бегать"]]);
        let set = assemble(
            &registry, &["c".into()],
            &[TableSnapshot { source: &key, rows: &data }],
            Limits { max_tables: 1, max_rows_per_table: 2 },
        ).unwrap();
        assert_eq!(set.item_count(), 2);
        assert!(set.tables[0].preview.truncated);
    }
}
