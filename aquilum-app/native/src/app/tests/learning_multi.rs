use super::*;
use aquilum_core::learning::ColumnMapping;
use aquilum_core::learning::table_anchors::NoteKey;
use aquilum_core::learning::state::StudyStore;
use crate::ui::editor::TextEditor;

pub(super) fn bind_fixture(h: &mut Harness) -> (std::path::PathBuf, std::path::PathBuf) {
    let a = h.vault.path().join("a.md");
    let b = h.vault.path().join("b.md");
    std::fs::write(&a, "| Q | A |\n| --- | --- |\n| eat | есть |\n\n| Q | A |\n| --- | --- |\n| ate | ел |").unwrap();
    std::fs::write(&b, "| Q | A |\n| --- | --- |\n| read | читать |").unwrap();
    let session = h.app.session.as_ref().unwrap();
    let core = &h.app.workspace.core;
    let mut store = StudyStore::open(
        &h.app.workspace.data_dir.join("language-study.sqlite"),
    ).unwrap();
    let a_id = session.document(core, &a).unwrap();
    let b_id = session.document(core, &b).unwrap();
    let a_key = NoteKey {
        vault_id: session.workspace_id.to_string(), note_id: a_id.to_string(),
    };
    let b_key = NoteKey {
        vault_id: session.workspace_id.to_string(), note_id: b_id.to_string(),
    };
    let mapping = ColumnMapping { question: 0, answers: vec![1], context: None };
    let a_tables = TextEditor::scan_study_source(
        &std::fs::read_to_string(&a).unwrap(),
    ).unwrap();
    let b_tables = TextEditor::scan_study_source(
        &std::fs::read_to_string(&b).unwrap(),
    ).unwrap();
    let mut rev = 0;
    rev = store.bind(rev, &a_key, &a_tables, 0, mapping.clone(),
        "english", "English", "id-a").unwrap().revision;
    rev = store.bind(rev, &a_key, &a_tables, 1, mapping.clone(),
        "english", "English", "id-b").unwrap().revision;
    rev = store.bind(rev, &a_key, &a_tables, 1, mapping.clone(),
        "verbs", "Verbs", "unused").unwrap().revision;
    store.bind(rev, &b_key, &b_tables, 0, mapping,
        "verbs", "Verbs", "id-c").unwrap();
    (a, b)
}

#[test]
fn lang02_two_closed_markdown_notes_merge_without_copy_and_stale_files_are_held() {
    let mut h = Harness::new();
    let (a, b) = bind_fixture(&mut h);
    let a_before = std::fs::read(&a).unwrap();
    let b_before = std::fs::read(&b).unwrap();
    let collections = ["english".into(), "verbs".into()];
    let snapshot = h.app.load_study_deck(&collections).unwrap();
    assert_eq!(snapshot.deck.cards.len(), 3);
    assert!(snapshot.deck.unavailable.is_empty());
    assert!(snapshot.unavailable_notes.is_empty());
    assert_eq!(snapshot.sources.len(), 2, "one note must be read once");
    assert!(h.app.revalidate_study_deck(&snapshot));
    assert_eq!(std::fs::read(&a).unwrap(), a_before);
    assert_eq!(std::fs::read(&b).unwrap(), b_before);

    std::fs::write(&b, "| Q | A |\n| --- | --- |\n| read | новое |").unwrap();
    assert!(!h.app.revalidate_study_deck(&snapshot));
    let rebuilt = h.app.load_study_deck(&collections).unwrap();
    assert_eq!(rebuilt.deck.cards.len(), 2);
    assert_eq!(rebuilt.deck.unavailable.len(), 1);
    assert!(!rebuilt.deck.cards.iter().any(|c| c.question == "read"));
}

#[test]
fn lang02_native_app_uses_only_active_memory_and_never_closed_disk() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a));
    let deck = h.app.load_study_deck(&["english".into()]).unwrap();
    assert_eq!((deck.deck.cards.len(), deck.sources.len(), deck.unavailable_notes.len()), (2, 0, 0));
    assert!(deck.active.is_some());
    assert!(h.app.revalidate_study_deck(&deck));
}

#[test]
fn lang02_stale_atomic_registry_revision_invalidates_a_prepared_deck() {
    let mut h = Harness::new();
    bind_fixture(&mut h);
    let collections = ["english".into()];
    let deck = h.app.load_study_deck(&collections).unwrap();
    let mut store = StudyStore::open(
        &h.app.workspace.data_dir.join("language-study.sqlite"),
    ).unwrap();
    let rev = store.load().unwrap().revision;
    store.rename_collection(rev, "english", "Another name").unwrap();
    assert!(!h.app.revalidate_study_deck(&deck));
}
