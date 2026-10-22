use super::metadata;
use crate::search::error::SearchError;
use crate::search::index::SearchIndex;
use crate::search::index_document::untracked_missing_path;
use rusqlite::Transaction;
use std::path::Path;

/// Ignore notifications that cannot affect the committed index. Keep the
/// missing-path recovery check before the unchanged-file fast path.
pub(super) fn skip(index: &SearchIndex, transaction: &Transaction<'_>, path: &Path) -> Result<bool, SearchError> {
    Ok(untracked_missing_path(index, transaction, path)? || cached(transaction, path)?)
}

fn cached(transaction: &Transaction<'_>, path: &Path) -> Result<bool, SearchError> {
    let Ok(file) = path.metadata() else {
        return Ok(false);
    };
    if !file.is_file() {
        return Ok(false);
    }
    Ok(metadata::current(transaction, &path.to_string_lossy())?.is_some_and(|stored| {
        stored.analyzer_version == crate::search::ANALYZER_VERSION
            && stored.fingerprint == metadata::fingerprint(&file)
    }))
}

#[cfg(test)]
mod tests {
    use super::super::IndexSynchronizer;
    use crate::search::index::SearchIndex;
    use crate::search::progress::IndexProgress;
    use std::collections::HashSet;
    use std::fs;
    use std::sync::Arc;

    #[test]
    fn duplicate_file_notification_does_not_acquire_index_writer() {
        let directory = tempfile::tempdir().unwrap();
        let vault = directory.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        let note = vault.join("Повтор.md");
        fs::write(&note, "Первый текст").unwrap();
        let index = Arc::new(SearchIndex::open(&directory.path().join("index")).unwrap());
        let mut sync = IndexSynchronizer::new(
            vault,
            Arc::clone(&index),
            &directory.path().join("documents.sqlite3"),
            Arc::new(IndexProgress::new(0, 1)),
        )
        .unwrap();

        assert!(sync.apply_paths(HashSet::from([note.clone()])).unwrap());
        sync.release_writer().unwrap();
        assert!(!sync.holds_writer());
        assert!(!sync.apply_paths(HashSet::from([note.clone()])).unwrap());
        assert!(!sync.holds_writer(), "an identical notification should not allocate a writer");
        assert_eq!(index.document_count(), 1);

        fs::write(&note, "Первый текст с новым окончанием").unwrap();
        assert!(sync.apply_paths(HashSet::from([note])).unwrap());
        assert_eq!(index.document_count(), 1);
    }
}
