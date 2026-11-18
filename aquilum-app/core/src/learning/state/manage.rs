//! Metadata-only, revision-checked management of named study collections.
//! The trusted native host owns the app-data path and explicit user intent.
//! Renaming/deleting a collection never edits Markdown or source table anchors.

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollectionEditError {
    State(StateError),
    MissingCollection,
    ProtectedCollection,
    InvalidName,
    DuplicateName,
}

impl From<StateError> for CollectionEditError {
    fn from(value: StateError) -> Self {
        Self::State(value)
    }
}

fn clean_name(input: &str) -> Result<String, CollectionEditError> {
    if input.len() > 256 {
        return Err(CollectionEditError::InvalidName);
    }
    let name = input.trim();
    if name.is_empty()
        || name.chars().count() > 64
        || name.chars().any(|c| {
            c.is_control()
                || matches!(
                    c,
                    '\u{00ad}'
                        | '\u{200b}'..='\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2060}'..='\u{206f}'
                        | '\u{feff}'
                )
        })
    {
        return Err(CollectionEditError::InvalidName);
    }
    Ok(name.to_owned())
}

impl StudyStore {
    /// Rename one user-created group, retaining its UUID, mappings and members.
    /// A stale writer or an invalid name cannot change the persisted revision.
    pub fn rename_collection(
        &mut self,
        expected_revision: u64,
        collection_id: &str,
        requested_name: &str,
    ) -> Result<u64, CollectionEditError> {
        if collection_id == "common" {
            return Err(CollectionEditError::ProtectedCollection);
        }
        let name = clean_name(requested_name)?;
        let folded = name.to_lowercase();
        self.edit_collections(expected_revision, |state| {
            if state.registry.collections.iter().any(|group| {
                group.id != collection_id && group.name.to_lowercase() == folded
            }) {
                return Err(CollectionEditError::DuplicateName);
            }
            let group = state.registry.collections.iter_mut()
                .find(|group| group.id == collection_id)
                .ok_or(CollectionEditError::MissingCollection)?;
            if group.name == name {
                return Ok(false);
            }
            group.name = name;
            Ok(true)
        })
    }

    /// Remove only the collection and its memberships. The original Markdown,
    /// shared sources, column mappings and table anchors remain untouched.
    /// The native UI must require explicit confirmation before calling this.
    pub fn delete_collection(
        &mut self,
        expected_revision: u64,
        collection_id: &str,
    ) -> Result<u64, CollectionEditError> {
        if collection_id == "common" {
            return Err(CollectionEditError::ProtectedCollection);
        }
        self.edit_collections(expected_revision, |state| {
            let Some(index) = state.registry.collections.iter()
                .position(|group| group.id == collection_id) else {
                    return Err(CollectionEditError::MissingCollection);
                };
            state.registry.collections.remove(index);
            Ok(true)
        })
    }

    fn edit_collections(
        &mut self,
        expected_revision: u64,
        edit: impl FnOnce(&mut StudyState) -> Result<bool, CollectionEditError>,
    ) -> Result<u64, CollectionEditError> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| CollectionEditError::State(StateError::Storage))?;
        let (rev, bytes): (i64, Vec<u8>) = tx.query_row(
            "SELECT revision,payload FROM language_study_state WHERE id=1",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).map_err(|_| CollectionEditError::State(StateError::Corrupt))?;
        let mut state = decode(&bytes)?;
        let revision = u64::try_from(rev)
            .map_err(|_| CollectionEditError::State(StateError::Corrupt))?;
        if revision != expected_revision {
            return Err(CollectionEditError::State(StateError::RevisionConflict));
        }
        if !edit(&mut state)? {
            return Ok(revision);
        }
        let payload = encode(&state)?;
        let next = rev.checked_add(1)
            .ok_or(CollectionEditError::State(StateError::RevisionExhausted))?;
        let affected = tx.execute(
            "UPDATE language_study_state SET revision=?1,payload=?2 \
             WHERE id=1 AND revision=?3",
            params![next, payload, rev],
        ).map_err(|_| CollectionEditError::State(StateError::Storage))?;
        if affected != 1 {
            return Err(CollectionEditError::State(StateError::RevisionConflict));
        }
        tx.commit().map_err(|_| CollectionEditError::State(StateError::Storage))?;
        u64::try_from(next)
            .map_err(|_| CollectionEditError::State(StateError::RevisionExhausted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(store: &mut StudyStore, revision: u64, id: &str, name: &str) -> u64 {
        let note = NoteKey { vault_id: "vault-1".into(), note_id: "note-1".into() };
        let tables = [vec![
            vec!["English".into(), "Russian".into()],
            vec!["write".into(), "писать".into()],
        ]];
        let columns = ColumnMapping { question: 0, answers: vec![1], context: None };
        store.bind(revision, &note, &tables, 0, columns, id, name, "table-1")
            .unwrap().revision
    }

    #[test]
    fn rename_is_durable_and_preserves_identity_and_sources() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("study.sqlite");
        let mut store = StudyStore::open(&path).unwrap();
        let revision = seed(&mut store, 0, "verbs", "Глаголы");
        let before = store.load().unwrap().state;
        assert_eq!(store.rename_collection(revision, "verbs", "  Действия  "), Ok(2));
        let after = store.load().unwrap().state;
        assert_eq!(after.registry.collections[0].id, "verbs");
        assert_eq!(after.registry.collections[0].name, "Действия");
        assert_eq!(after.registry.collections[0].members, before.registry.collections[0].members);
        assert_eq!(after.registry.tables, before.registry.tables);
        assert_eq!(after.anchors, before.anchors);
        assert_eq!(store.rename_collection(2, "verbs", "Действия"), Ok(2));
        drop(store);
        assert_eq!(StudyStore::open(&path).unwrap().load().unwrap().revision, 2);
    }

    #[test]
    fn deletion_never_removes_shared_table_or_default_collection() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = StudyStore::open(&dir.path().join("study.sqlite")).unwrap();
        let rev = seed(&mut store, 0, "common", "Общая база");
        let rev = seed(&mut store, rev, "verbs", "Глаголы");
        let original = store.load().unwrap().state;
        assert_eq!(store.delete_collection(rev, "common"),
            Err(CollectionEditError::ProtectedCollection));
        assert_eq!(store.delete_collection(rev, "verbs"), Ok(rev + 1));
        let after = store.load().unwrap().state;
        assert_eq!(after.registry.collections.len(), 1);
        assert_eq!(after.registry.collections[0].id, "common");
        assert_eq!(after.registry.tables, original.registry.tables);
        assert_eq!(after.anchors, original.anchors);
        assert_eq!(store.delete_collection(rev + 1, "verbs"),
            Err(CollectionEditError::MissingCollection));
    }

    #[test]
    fn invalid_names_and_unicode_collisions_fail_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = StudyStore::open(&dir.path().join("study.sqlite")).unwrap();
        let rev = seed(&mut store, 0, "verbs", "Глаголы");
        let rev = seed(&mut store, rev, "nouns", "Существительные");
        let too_long = "я".repeat(65);
        for invalid in ["", "  ", "a\nb", "a\u{202e}b", "a\u{200b}b",
                        "a\u{feff}b", too_long.as_str()] {
            assert_eq!(store.rename_collection(rev, "verbs", invalid),
                Err(CollectionEditError::InvalidName));
        }
        assert_eq!(store.rename_collection(rev, "verbs", "сУщЕсТвИтЕлЬнЫе"),
            Err(CollectionEditError::DuplicateName));
        assert_eq!(store.load().unwrap().revision, rev);
    }

    #[test]
    fn stale_writer_and_corrupt_payload_do_not_overwrite_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("study.sqlite");
        let mut first = StudyStore::open(&path).unwrap();
        let mut stale = StudyStore::open(&path).unwrap();
        let rev = seed(&mut first, 0, "verbs", "Verbs");
        assert_eq!(stale.delete_collection(0, "verbs"),
            Err(CollectionEditError::State(StateError::RevisionConflict)));
        first.connection.execute(
            "UPDATE language_study_state SET payload=?1 WHERE id=1",
            params![&b"not-json"[..]],
        ).unwrap();
        assert_eq!(first.rename_collection(rev, "verbs", "New"),
            Err(CollectionEditError::State(StateError::Corrupt)));
    }
}
