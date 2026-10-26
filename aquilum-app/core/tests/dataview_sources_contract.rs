use aquilum_core::search::dataview::{parser::parse, source::resolve};
use aquilum_core::search::error::SearchError;
use aquilum_core::search::{fields, wiki};
use rusqlite::Connection;
use std::path::Path;

fn path(root: &Path, relative: &str) -> String {
    root.join(relative).to_string_lossy().into_owned()
}

fn fixture() -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE documents(path TEXT PRIMARY KEY);").unwrap();
    fields::open_schema(&db).unwrap();
    wiki::open_schema(&db).unwrap();
    let tx = db.transaction().unwrap();
    for (relative, body) in [
        ("Books/B.md", "---\ntags: [fiction, draft]\n---\n[[Target]]"),
        ("Target.md", ""),
        ("Books/A.md", "---\ntags: [fiction]\n---\n[[B]] [[Target]]"),
        ("Archive/С.md", "---\ntags: [draft]\n---\n[[A]]"),
    ] {
        let path = path(root, relative);
        std::fs::create_dir_all(Path::new(&path).parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        tx.execute("INSERT INTO documents(path) VALUES(?1)", [&path]).unwrap();
        fields::index_document(&tx, Path::new(&path), body).unwrap();
        wiki::index_document(&tx, root, Path::new(&path), body).unwrap();
    }
    tx.commit().unwrap();
    (directory, db)
}

fn select(db: &Connection, root: &Path, source: &str) -> Result<Vec<String>, SearchError> {
    let query = parse(&format!("LIST FROM {source}")).unwrap();
    resolve(db, root, &root.join("Origin.md"), query.source.as_ref())
}

#[test]
fn sources_preserve_boolean_link_and_sql_order_contracts() {
    let (directory, db) = fixture();
    let root = directory.path();
    let cases: &[(&str, &[&str])] = &[
        ("#fiction", &["Books/A.md", "Books/B.md"]),
        ("\"Books\" and #fiction", &["Books/A.md", "Books/B.md"]),
        ("#fiction and \"Books\"", &["Books/A.md", "Books/B.md"]),
        ("#fiction and not #draft", &["Books/A.md"]),
        ("#fiction or #draft", &["Archive/С.md", "Books/A.md", "Books/B.md"]),
        ("\"Books\" or #draft", &["Archive/С.md", "Books/A.md", "Books/B.md"]),
        ("not \"Books\"", &["Archive/С.md", "Target.md"]),
        ("[[Target]]", &["Books/A.md", "Books/B.md"]),
        ("outgoing([[A]])", &["Books/B.md", "Target.md"]),
        ("\"Books\" and [[Target]]", &["Books/A.md", "Books/B.md"]),
        ("\"Books\" or outgoing([[A]])", &["Books/A.md", "Books/B.md", "Target.md"]),
        ("not [[Target]]", &["Archive/С.md", "Target.md"]),
        ("once", &[]),
        ("once or \"Books\"", &["Books/A.md", "Books/B.md"]),
        ("\"Books\" and once", &[]),
    ];
    for (source, expected) in cases {
        let expected = expected.iter().map(|relative| path(root, relative)).collect::<Vec<_>>();
        assert_eq!(select(&db, root, source).unwrap(), expected, "{source}");
    }
    let all = resolve(&db, root, root, None).unwrap();
    assert_eq!(all, fields::candidates(&db, &[], &[]).unwrap());
    let expected = ["Archive/С.md", "Books/A.md", "Books/B.md", "Target.md"];
    assert_eq!(all, expected.map(|relative| path(root, relative)));
}

#[test]
fn isolated_empty_sources_skip_unneeded_tables_but_keep_error_order() {
    let (directory, db) = fixture();
    let root = directory.path();
    db.execute_batch("DROP TABLE wiki_links; DROP TABLE wiki_documents;").unwrap();
    assert!(select(&db, root, "#missing and [[Target]]").unwrap().is_empty());
    assert!(select(&db, root, "[[Target]] and #missing").is_err());
    let empty = Connection::open_in_memory().unwrap();
    for source in ["once", "once and #fiction", "\"Books\" and once"] {
        assert!(select(&empty, root, source).unwrap().is_empty(), "{source}");
    }
    assert!(select(&empty, root, "#fiction and once").is_err());
    assert!(resolve(&empty, root, root, None).is_err());
    assert!(fields::candidates(&empty, &[], &[]).is_err());
}
