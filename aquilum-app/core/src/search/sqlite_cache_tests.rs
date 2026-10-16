//! SQL statement cache must preserve indexing semantics across documents and transactions.
use super::{fields, tasks, wiki};
use rusqlite::Connection;
use std::path::Path;

fn counts(connection: &Connection) -> (i64, i64, i64) {
    let count = |table: &str| -> i64 {
        connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0)).unwrap()
    };
    (count("wiki_links"), count("note_fields"), count("note_tasks"))
}

fn index(
    transaction: &rusqlite::Transaction<'_>,
    root: &Path,
    path: &Path,
    body: &str,
) {
    wiki::index_document(transaction, root, path, body).unwrap();
    fields::index_document(transaction, path, body).unwrap();
    tasks::index_document(transaction, path, body).unwrap();
}

#[test]
fn prepared_metadata_statements_never_reuse_previous_document_parameters() {
    let mut connection = Connection::open_in_memory().unwrap();
    wiki::open_schema(&connection).unwrap();
    fields::open_schema(&connection).unwrap();
    tasks::open_schema(&connection).unwrap();
    let root = Path::new("/vault");
    let alice = root.join("alice.md");
    let bob = root.join("bob.md");
    let first = "Owner:: Alice\n- [ ] task A\n[[TargetA]]\n";
    let second = "Owner:: Bob\nStatus:: active\n- [x] task B\n- [ ] task C\n[[TargetB]] and [[TargetC]]\n";

    for _ in 0..3 {
        let tx = connection.transaction().unwrap();
        index(&tx, root, &alice, first);
        index(&tx, root, &bob, second);
        tx.commit().unwrap();
        assert_eq!(counts(&connection), (3, 3, 3));
    }

    let tx = connection.transaction().unwrap();
    index(&tx, root, &bob, "plain prose without indexed metadata\n");
    tx.commit().unwrap();
    assert_eq!(counts(&connection), (1, 1, 1));

    let tx = connection.transaction().unwrap();
    wiki::remove_document(&tx, &alice).unwrap();
    fields::remove_document(&tx, &alice).unwrap();
    tasks::remove_document(&tx, &alice).unwrap();
    tx.commit().unwrap();
    assert_eq!(counts(&connection), (0, 0, 0));
}
