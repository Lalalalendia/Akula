use aquilum_core::search::index::SearchIndex;
use aquilum_core::search::progress::IndexProgress;
use aquilum_core::search::query::Request;
use aquilum_core::search::sync::IndexSynchronizer;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn count_rows(connection: &Connection, table: &str, path: &Path) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE path=?1");
    connection
        .query_row(&sql, params![path.to_string_lossy().as_ref()], |row| row.get(0))
        .unwrap()
}

#[test]
fn full_scan_searches_txt_and_md_without_treating_txt_as_markdown() {
    let temp = tempfile::tempdir().unwrap();
    let vault = temp.path().join("vault");
    fs::create_dir_all(vault.join(".akula")).unwrap();
    let text = vault.join("東京日記.TXT");
    let markdown = vault.join("Notebook.md");
    fs::write(
        &text,
        "---\ncreated: 2001-01-02\ntype: task\n---\n北京大学 東京 unifiedsearchneedle [[Target]]\n- [ ] not a markdown task",
    ).unwrap();
    fs::write(&markdown, "[[Target]]\n- [ ] real task\nmarkdownneedle").unwrap();
    fs::write(vault.join(".akula/private.txt"), "shouldneverindex").unwrap();
    fs::write(vault.join("picture.png"), "binaryformatmarker").unwrap();
    let index = Arc::new(SearchIndex::open(&temp.path().join("index")).unwrap());
    let metadata = temp.path().join("documents.sqlite3");
    let mut sync = IndexSynchronizer::new(
        vault.clone(), Arc::clone(&index), &metadata,
        Arc::new(IndexProgress::new(0, 1)),
    ).unwrap();
    sync.full_scan(&AtomicBool::new(false)).unwrap();
    assert_eq!(index.document_count(), 2, "only visible Markdown and TXT are indexed");
    for term in ["unifiedsearchneedle", "北京大学", "東京日記"] {
        let results = index.search(&Request::new(term)).unwrap().results;
        assert_eq!(results.len(), 1, "TXT search failed for {term}");
        assert!(results[0].path.ends_with("東京日記.TXT"));
    }
    assert_eq!(index.search(&Request::new("markdownneedle")).unwrap().results.len(), 1);
    for term in ["shouldneverindex", "binaryformatmarker"] {
        assert!(index.search(&Request::new(term)).unwrap().results.is_empty(), "{term}");
    }
    let connection = Connection::open(metadata).unwrap();
    for table in ["wiki_documents", "wiki_links", "note_fields", "note_tasks"] {
        let query = if table == "wiki_links" {
            "SELECT COUNT(*) FROM wiki_links WHERE source_path=?1".to_owned()
        } else {
            format!("SELECT COUNT(*) FROM {table} WHERE path=?1")
        };
        let count: i64 = connection.query_row(
            &query, params![text.to_string_lossy().as_ref()], |row| row.get(0)
        ).unwrap();
        assert_eq!(count, 0, "TXT produced Markdown side effects in {table}");
    }
    assert_eq!(count_rows(&connection, "wiki_documents", &markdown), 1);
    assert_eq!(count_rows(&connection, "note_tasks", &markdown), 1);
    let created_source: i64 = connection.query_row(
        "SELECT created_source FROM documents WHERE path=?1",
        params![text.to_string_lossy().as_ref()], |row| row.get(0),
    ).unwrap();
    assert_ne!(created_source, 2, "TXT frontmatter must not override file timestamps");
}

#[test]
fn txt_changes_and_deletions_update_search_without_rescanning_entire_vault() {
    let temp = tempfile::tempdir().unwrap();
    let vault = temp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    let text = vault.join("Данные.TxT");
    fs::write(&text, "originalunique").unwrap();
    let index = Arc::new(SearchIndex::open(&temp.path().join("index")).unwrap());
    let mut sync = IndexSynchronizer::new(
        vault, Arc::clone(&index), &temp.path().join("documents.sqlite3"),
        Arc::new(IndexProgress::new(0, 1)),
    ).unwrap();
    sync.full_scan(&AtomicBool::new(false)).unwrap();
    assert_eq!(index.search(&Request::new("originalunique")).unwrap().results.len(), 1);

    fs::write(&text, "replacedunique with a longer body").unwrap();
    assert!(sync.apply_paths(HashSet::from([text.clone()])).unwrap());
    assert!(index.search(&Request::new("originalunique")).unwrap().results.is_empty());
    assert_eq!(index.search(&Request::new("replacedunique")).unwrap().results.len(), 1);

    fs::remove_file(&text).unwrap();
    assert!(sync.apply_paths(HashSet::from([text])).unwrap());
    assert_eq!(index.document_count(), 0);
    assert!(index.search(&Request::new("replacedunique")).unwrap().results.is_empty());
}

#[test]
fn invalid_utf8_txt_is_skipped_then_indexed_when_replaced_by_valid_text() {
    let temp = tempfile::tempdir().unwrap();
    let vault = temp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    let text = vault.join("broken.txt");
    fs::write(&text, [0xff, 0xfe, 0x00]).unwrap();
    let index = Arc::new(SearchIndex::open(&temp.path().join("index")).unwrap());
    let mut sync = IndexSynchronizer::new(
        vault, Arc::clone(&index), &temp.path().join("documents.sqlite3"),
        Arc::new(IndexProgress::new(0, 1)),
    ).unwrap();
    sync.full_scan(&AtomicBool::new(false)).unwrap();
    assert_eq!(index.document_count(), 0);
    fs::write(&text, "repairedunique").unwrap();
    assert!(sync.apply_paths(HashSet::from([text])).unwrap());
    assert_eq!(index.search(&Request::new("repairedunique")).unwrap().results.len(), 1);
}
