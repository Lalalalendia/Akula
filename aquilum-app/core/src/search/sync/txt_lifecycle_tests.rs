use super::super::sync::IndexSynchronizer;
use crate::search::index::SearchIndex;
use crate::search::progress::IndexProgress;
use crate::search::query::Request;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn assert_single_result(index: &SearchIndex, term: &str, path: &Path, title: &str) {
    let results = index.search(&Request::new(term)).unwrap().results;
    assert_eq!(results.len(), 1, "expected one indexed document for {term}");
    assert_eq!(results[0].path, path.to_string_lossy());
    assert_eq!(results[0].title, title);
}

fn assert_no_result(index: &SearchIndex, term: &str) {
    let results = index.search(&Request::new(term)).unwrap().results;
    assert!(results.is_empty(), "unexpected indexed document for {term}");
}

/// Exercises the same full-scan and changed-path ingestion used by the
/// filesystem watcher. This is NOT a test of OS notification delivery.
#[test]
fn txt_create_edit_rename_restart_delete_preserves_search_and_source_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let vault = directory.path().join("vault");
    let secret_dir = vault.join(".aquilum").join("history");
    fs::create_dir_all(&secret_dir).unwrap();
    let vault = crate::search::paths::canonical_path(&vault);
    let md = vault.join("Control.md");
    let md_bytes = "Markdown desk10controlmarkdown only\n".as_bytes();
    fs::write(&md, md_bytes).unwrap();
    let shadow = secret_dir.join("Secret.TXT");
    let secret_bytes = "desk10hiddensecret never index this\n".as_bytes();
    fs::write(&shadow, secret_bytes).unwrap();

    let index_path = directory.path().join("index");
    let metadata_path = directory.path().join("documents.sqlite3");
    let index = Arc::new(SearchIndex::open(&index_path).unwrap());
    let mut sync = IndexSynchronizer::new(
        vault.clone(),
        Arc::clone(&index),
        &metadata_path,
        Arc::new(IndexProgress::new(0, 1)),
    )
    .unwrap();
    let cancelled = AtomicBool::new(false);
    sync.full_scan(&cancelled).unwrap();
    assert_eq!(index.document_count(), 1);
    assert_single_result(&index, "desk10controlmarkdown", &md, "Control");
    assert_no_result(&index, "desk10hiddensecret");

    // Creating .TXT after the initial scan must be visible on a path event.
    let txt = vault.join("東京日記.TXT");
    let original = "日本語と中文\ndesk10initialnote\n".as_bytes();
    fs::write(&txt, original).unwrap();
    assert!(sync.apply_paths(HashSet::from([txt.clone()])).unwrap());
    assert_eq!(index.document_count(), 2);
    assert_single_result(&index, "desk10initialnote", &txt, "東京日記");
    assert_eq!(fs::read(&txt).unwrap(), original);
    assert!(!sync.apply_paths(HashSet::from([shadow.clone()])).unwrap());
    assert_no_result(&index, "desk10hiddensecret");

    // A changed TXT body must replace the old search hit without a rebuild.
    // Different lengths avoid relying on the filesystem's mtime resolution.
    let edited = "日本語と中文と한국어\ndesk10updatedbody\nsecond line\n".as_bytes();
    fs::write(&txt, edited).unwrap();
    assert!(sync.apply_paths(HashSet::from([txt.clone()])).unwrap());
    assert_no_result(&index, "desk10initialnote");
    assert_single_result(&index, "desk10updatedbody", &txt, "東京日記");
    assert_eq!(fs::read(&txt).unwrap(), edited);

    // Rename is delivered as an old-path removal plus new-path addition.
    let renamed = vault.join("東京日記改.txt");
    fs::rename(&txt, &renamed).unwrap();
    assert!(sync
        .apply_paths(HashSet::from([txt.clone(), renamed.clone()]))
        .unwrap());
    assert_eq!(index.document_count(), 2);
    assert_single_result(&index, "desk10updatedbody", &renamed, "東京日記改");
    assert_eq!(fs::read(&renamed).unwrap(), edited);

    sync.release_writer().unwrap();
    drop(sync);
    drop(index);

    // A fresh process reopens the same Tantivy/SQLite files and rescans.
    let index = Arc::new(SearchIndex::open(&index_path).unwrap());
    let mut reopened = IndexSynchronizer::new(
        vault,
        Arc::clone(&index),
        &metadata_path,
        Arc::new(IndexProgress::new(2, 2)),
    )
    .unwrap();
    reopened.full_scan(&cancelled).unwrap();
    assert_eq!(index.document_count(), 2);
    assert_single_result(&index, "desk10updatedbody", &renamed, "東京日記改");
    assert_single_result(&index, "desk10controlmarkdown", &md, "Control");
    assert_no_result(&index, "desk10hiddensecret");

    fs::remove_file(&renamed).unwrap();
    assert!(reopened
        .apply_paths(HashSet::from([renamed.clone()]))
        .unwrap());
    assert_eq!(index.document_count(), 1);
    assert_no_result(&index, "desk10updatedbody");
    assert_single_result(&index, "desk10controlmarkdown", &md, "Control");
    assert_eq!(fs::read(&md).unwrap(), md_bytes);
    assert_eq!(fs::read(&shadow).unwrap(), secret_bytes);
}
