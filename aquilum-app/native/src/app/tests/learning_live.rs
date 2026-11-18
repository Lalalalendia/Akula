use super::*;
use super::learning_multi::bind_fixture;

#[test]
fn lang05_live_active_note_and_closed_note_join_without_copying_disk_text() {
    let mut h = Harness::new();
    let (a, b) = bind_fixture(&mut h);
    let disk_a = std::fs::read(&a).unwrap();
    let disk_b = std::fs::read(&b).unwrap();
    assert!(h.app.open(&mut h.root, a.clone()));
    let text = h.app.text.to_string();
    h.app.commit_text(&mut h.root, crop::Rope::from(
        format!("{text}\n\nUnsaved editor-only annotation."),
    ));

    let snapshot = h.app.load_study_deck(&["english".into(), "verbs".into()]).unwrap();
    assert_eq!(snapshot.deck.cards.len(), 3, "live and closed source rows join once");
    assert_eq!(snapshot.sources.len(), 1, "only the closed note uses File Gate");
    assert!(snapshot.unavailable_notes.is_empty());
    assert!(snapshot.active.is_some(), "active note comes from in-memory Rope");
    assert!(h.app.revalidate_study_deck(&snapshot));
    assert_eq!(std::fs::read(&a).unwrap(), disk_a, "unsaved edit stays in memory");
    assert_eq!(std::fs::read(&b).unwrap(), disk_b);
    eprintln!("AKULA_LANG05_LIVE_PASS=unsaved_buffer,closed_source,no_copy");
}

#[test]
fn lang05_live_editor_edit_invalidates_card_before_answer_reveal() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a));
    let before = h.app.load_study_deck(&["english".into()]).unwrap();
    assert_eq!(before.deck.cards.len(), 2);
    assert!(before.active.is_some());
    h.app.commit_text(&mut h.root, crop::Rope::from(
        format!("{}\n\nA new unsaved paragraph", h.app.text.to_string()),
    ));
    assert!(!h.app.revalidate_study_deck(&before),
        "a question cannot reveal an answer from the prior editor buffer");
    eprintln!("AKULA_LANG05_LIVE_STALE_PASS=revision_changed,no_old_answer");
}

#[test]
fn lang05_inactive_open_tab_stays_unavailable_while_active_tab_uses_memory() {
    let mut h = Harness::new();
    let (a, b) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a));
    assert!(h.app.open(&mut h.root, b));
    let snapshot = h.app.load_study_deck(&["english".into(), "verbs".into()]).unwrap();
    assert_eq!(snapshot.deck.cards.len(), 1, "inactive tab is not read from disk");
    assert_eq!(snapshot.deck.cards[0].question, "read");
    assert_eq!(snapshot.sources.len(), 0);
    assert_eq!(snapshot.unavailable_notes.len(), 1);
    assert!(snapshot.active.is_some());

    let a = h.vault.path().join("a.md");
    assert!(h.app.open(&mut h.root, a));
    assert!(!h.app.revalidate_study_deck(&snapshot),
        "switching editor tabs invalidates the previous in-memory authority");
}

#[test]
fn lang05_conflicted_active_editor_stays_unavailable_and_never_reads_stale_disk() {
    let mut h = Harness::new();
    let (a, b) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a));
    h.app.note.as_mut().unwrap().save_error = Some("manual review".into());
    let snapshot = h.app.load_study_deck(&["english".into(), "verbs".into()]).unwrap();
    assert!(snapshot.active.is_none());
    assert_eq!(snapshot.unavailable_notes.len(), 1);
    assert_eq!(snapshot.deck.cards.len(), 1);
    assert_eq!(snapshot.deck.cards[0].question, "read");
    assert!(b.is_file());
}

#[test]
fn lang05_external_change_to_active_backing_file_invalidates_memory_cards() {
    let mut h = Harness::new();
    let (a, _) = bind_fixture(&mut h);
    assert!(h.app.open(&mut h.root, a.clone()));
    let snapshot = h.app.load_study_deck(&["english".into()]).unwrap();
    assert_eq!(snapshot.deck.cards.len(), 2);
    assert!(snapshot.active.is_some());
    std::fs::write(&a, "| Q | A |\n| --- | --- |\n| eat | другой |\n").unwrap();
    assert!(!h.app.revalidate_study_deck(&snapshot),
        "externally rewritten active file must not reveal old editor answers");
    let updated = h.app.load_study_deck(&["english".into()]).unwrap();
    assert!(updated.active.is_none(), "no fallback to stale disk or editor text");
    assert!(updated.deck.cards.is_empty());
    assert_eq!(updated.unavailable_notes.len(), 1);
    eprintln!("AKULA_LANG05_LIVE_DISK_STALE_PASS=hash_mismatch,no_old_cards");
}
