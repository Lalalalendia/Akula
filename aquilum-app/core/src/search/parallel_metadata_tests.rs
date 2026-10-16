//! Prepared metadata must produce the exact same SQLite rows as sequential parsing.
use super::created::{CreatedAt, CreatedSource};
use super::index_document::{prepare_file_with_probe, Probe};
use super::{fields, tasks, wiki};
use rusqlite::types::Value;
use rusqlite::Connection;
use std::fs;
use std::path::Path;

fn connection() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    wiki::open_schema(&connection).unwrap();
    fields::open_schema(&connection).unwrap();
    tasks::open_schema(&connection).unwrap();
    connection
}

fn snapshot(connection: &Connection) -> Vec<Vec<Vec<Value>>> {
    const TABLES: [&str; 4] = [
        "SELECT * FROM wiki_documents ORDER BY path",
        "SELECT * FROM wiki_links ORDER BY source_path, byte_offset",
        "SELECT * FROM note_fields ORDER BY path, ordinal",
        "SELECT * FROM note_tasks ORDER BY path, ordinal",
    ];
    TABLES
        .iter()
        .map(|sql| {
            let mut statement = connection.prepare(sql).unwrap();
            let columns = statement.column_count();
            statement
                .query_map([], move |row| {
                    (0..columns)
                        .map(|column| row.get::<_, Value>(column))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        })
        .collect()
}

fn store(
    connection: &mut Connection,
    root: &Path,
    path: &Path,
    body: &str,
    prepared: bool,
) {
    let transaction = connection.transaction().unwrap();
    if prepared {
        let file = prepare_file_with_probe(
            path.to_path_buf(),
            Probe {
                fingerprint: (1, body.len() as i64),
                up_to_date: false,
                previous_hash: None,
                created: CreatedAt { nanos: 1, source: CreatedSource::ModifiedTime },
            },
        )
        .unwrap();
        let parsed = file.parsed.expect("modified note must be parsed");
        assert_eq!(file.body, body);
        assert_eq!(parsed.wiki_links, wiki::parser::extract(body));
        assert_eq!(parsed.fields, fields::inline::indexed_fields(body));
        assert_eq!(parsed.tasks, tasks::parse::note_tasks(body));
        wiki::repository::index_prepared_links(
            &transaction, root, path, &parsed.wiki_links,
        )
        .unwrap();
        fields::repository::index_prepared_fields(&transaction, path, &parsed.fields).unwrap();
        tasks::repository::index_prepared_tasks(&transaction, path, &parsed.tasks).unwrap();
    } else {
        wiki::index_document(&transaction, root, path, body).unwrap();
        fields::index_document(&transaction, path, body).unwrap();
        tasks::index_document(&transaction, path, body).unwrap();
    }
    transaction.commit().unwrap();
}

#[test]
fn prepared_metadata_matches_sequential_sqlite_content_across_revisions() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("vault");
    fs::create_dir_all(&root).unwrap();
    let first_path = root.join("Alpha.md");
    let second_path = root.join("Beta.md");
    let original = r#"---
tags: [research, русский]
author: Алексей
---
# Первый документ
Owner:: Alpha
Score:: 8
- [ ] Купить книгу
- [x] Проверить результат
[[Target]] и [[Second]]
\`[Hidden:: value]\`
\`\`\`md
- [ ] not a task
[[NotALink]]
\`\`\`
"#;
    let secondary = "Title:: Beta\n- [ ] Next step\n[[Target]]\n[[Alpha|alias]]\n";
    let replacement = "Owner:: Changed\n- [x] replaced task\n[[Alternative]]\n";
    let mut sequential = connection();
    let mut parallel = connection();

    for (path, content) in [
        (&first_path, original),
        (&second_path, secondary),
        (&first_path, replacement),
        (&second_path, "ordinary prose without metadata\n"),
    ] {
        fs::write(path, content).unwrap();
        store(&mut sequential, &root, path, content, false);
        store(&mut parallel, &root, path, content, true);
        assert_eq!(
            snapshot(&sequential),
            snapshot(&parallel),
            "sequential and pre-parsed database content must match"
        );
    }

    for connection in [&mut sequential, &mut parallel] {
        let transaction = connection.transaction().unwrap();
        wiki::remove_document(&transaction, &first_path).unwrap();
        fields::remove_document(&transaction, &first_path).unwrap();
        tasks::remove_document(&transaction, &first_path).unwrap();
        transaction.commit().unwrap();
    }
    assert_eq!(snapshot(&sequential), snapshot(&parallel));
}

#[test]
fn already_indexed_identical_file_skips_reparsing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Note.md");
    fs::write(&path, "Score:: 7\n[[Target]]\n- [ ] item\n").unwrap();
    let changed = prepare_file_with_probe(
        path.clone(),
        Probe {
            fingerprint: (1, 1),
            up_to_date: false,
            previous_hash: None,
            created: CreatedAt { nanos: 1, source: CreatedSource::ModifiedTime },
        },
    )
    .unwrap();
    assert!(changed.parsed.is_some());
    let unchanged = prepare_file_with_probe(
        path,
        Probe {
            fingerprint: (1, 1),
            up_to_date: true,
            previous_hash: Some(changed.content_hash),
            created: CreatedAt { nanos: 1, source: CreatedSource::ModifiedTime },
        },
    )
    .unwrap();
    assert!(unchanged.parsed.is_none(), "same-hash file must not repeat Markdown parsing");
}
