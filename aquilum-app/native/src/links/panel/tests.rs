use std::sync::Arc;

use super::*;

fn indexed(files: &[(&str, &str)]) -> (tempfile::TempDir, tempfile::TempDir, Arc<Core>) {
    let vault = crate::test_dir::tempdir();
    let data = crate::test_dir::tempdir();
    for (name, text) in files {
        std::fs::write(vault.path().join(name), text).unwrap();
    }
    let core = Core::open(data.path(), Arc::new(|_| {}));
    let workspace = vault.path().to_string_lossy().into_owned();
    core.search.prepare(&workspace).unwrap();
    for _ in 0..200 {
        let status = core.search.status(Some(&workspace));
        if status.state == aquilum_core::search::models::SearchIndexState::Ready && !status.updating {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    (vault, data, core)
}

fn request(vault: &Path, document: &str, mode: Mode) -> Request {
    Request { generation: 1, workspace: vault.to_path_buf(), document: vault.join(document), mode, method: Method::Bm25f, text: String::new() }
}

#[test]
fn backlinks_and_outgoing_from_index() {
    let (vault, _data, core) = indexed(&[("a.md", "Ссылка на [[b]], снова [[b]] и [[Нет такой]]"), ("b.md", "Текст")]);
    let back = run(&core, &request(vault.path(), "b.md", Mode::Backlinks));
    assert_eq!(back.items.len(), 1);
    assert_eq!(back.items[0].counter.as_deref(), Some("2"));
    assert!(matches!(&back.items[0].target, Target::Note(p, _) if p.ends_with("a.md")));
    let out = run(&core, &request(vault.path(), "a.md", Mode::Outgoing));
    assert_eq!(out.items.len(), 2);
    assert_eq!(out.items[0].counter.as_deref(), Some("2"));
    assert!(out.items.iter().any(|i| matches!(&i.target, Target::Wiki(t) if t == "Нет такой")));
    let none = run(&core, &request(vault.path(), "a.md", Mode::Backlinks));
    assert!(none.items.is_empty() && none.notice.is_some());
    core.shutdown();
}

#[cfg(feature = "extension-view-lab")]
#[test]
fn first_party_task_view_is_scoped_and_read_only() {
    let (vault, _data, core) = indexed(&[
        ("a.md", "- [ ] alpha\n- [x] beta\n"),
        ("b.md", "- [ ] private\n"),
    ]);
    let path = vault.path().join("a.md");
    let before = std::fs::read(&path).unwrap();
    let listing = run(&core, &request(vault.path(), "a.md", Mode::Tasks));
    assert_eq!(listing.items.len(), 2);
    assert_eq!(listing.items[0].title, "[ ] alpha");
    assert_eq!(listing.items[1].title, "[x] beta");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    core.shutdown();
}

#[cfg(feature = "extension-view-lab")]
#[test]
fn task_view_rejects_foreign_hidden_and_missing_documents() {
    let (vault, _data, core) = indexed(&[
        ("visible.md", "- [ ] public\n"),
        (".hidden.md", "- [ ] hidden\n"),
    ]);
    let other = crate::test_dir::tempdir();
    std::fs::write(other.path().join("foreign.md"), "- [ ] private\n").unwrap();
    let workspace = vault.path().to_string_lossy().into_owned();
    let missing = vault.path().join("never-created.md");
    assert!(
        core.search.task_preview_for_document(&workspace, &missing).is_err(),
        "a non-existent note must not retrieve stale indexed tasks"
    );

    for path in [
        other.path().join("foreign.md"),
        vault.path().join(".hidden.md"),
        vault.path().join("missing.md"),
    ] {
        let mut req = request(vault.path(), "visible.md", Mode::Tasks);
        req.document = path;
        let blocked = run(&core, &req);
        assert!(blocked.items.is_empty(), "unexpected task data outside approved note");
        assert!(blocked.notice.is_some(), "missing denial reason");
    }

    let allowed = run(&core, &request(vault.path(), "visible.md", Mode::Tasks));
    assert_eq!(allowed.items.len(), 1);
    assert_eq!(allowed.items[0].title, "[ ] public");
    core.shutdown();
}

#[cfg(feature = "extension-view-lab")]
#[test]
fn task_view_caps_results_and_reports_truncation() {
    let body = (0..80)
        .map(|n| format!("- [ ] task {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (vault, _data, core) = indexed(&[("many.md", &body)]);
    let listing = run(&core, &request(vault.path(), "many.md", Mode::Tasks));
    assert_eq!(listing.items.len(), 64);
    assert_eq!(listing.items[0].title, "[ ] task 0");
    assert_eq!(listing.items[63].title, "[ ] task 63");
    assert!(listing.notice.is_some(), "64-row cap should be explained");
    core.shutdown();
}

#[test]
fn anchors_stay_in_the_target_and_attachments_are_not_listed() {
    let (vault, _data, core) = indexed(&[("a.md", "[[b#Раздел]] ![[pic.png|200]] [[#Тут]] [[Нет#Раздел]]"), ("b.md", "# Раздел")]);
    let out = run(&core, &request(vault.path(), "a.md", Mode::Outgoing));
    assert_eq!(out.items.len(), 2);
    assert!(matches!(&out.items[0].target, Target::Note(p, None) if p.ends_with("b.md")));
    assert!(matches!(&out.items[1].target, Target::Wiki(t) if t == "Нет#Раздел"));
    let back = run(&core, &request(vault.path(), "b.md", Mode::Backlinks));
    assert_eq!(back.items.len(), 1);
    core.shutdown();
}
