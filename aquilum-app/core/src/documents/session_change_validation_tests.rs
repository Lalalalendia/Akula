use super::*;

#[test]
fn utf16_surrogate_split_is_rejected_before_durable_journal_append() {
    let io = FakeIo::with_file("a😀b");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    let initial_updates = io.appended.get();
    let version = session.version();

    let invalid = edits_to_json(
        &[TextEdit { from: 2, to: 2, insert: "X".into() }],
        4,
    );
    assert!(session.push("editor", version, &[invalid], &io).is_err());
    assert_eq!(io.appended.get(), initial_updates);
    assert_eq!(session.text(), "a😀b");
    assert_eq!(session.version(), version);

    let valid = edits_to_json(
        &[TextEdit { from: 1, to: 3, insert: "🎉".into() }],
        4,
    );
    assert!(session.push("editor", version, &[valid], &io).expect("valid edit"));
    assert_eq!(session.text(), "a🎉b");
    assert_eq!(io.appended.get(), initial_updates + 1);
}


#[test]
fn overflowing_change_sections_never_reach_the_journal() {
    let io = FakeIo::with_file("abc");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    let version = session.version();
    let state = session.state();
    let updates = io.updates.borrow().clone();
    let appended = io.appended.get();
    for changes in [json!([u64::MAX, 1, 3]), json!([1, [u64::MAX], 3])] {
        assert!(session.push("editor", version, &[changes], &io).is_err());
        assert_eq!(session.state(), state);
        assert_eq!(session.version(), version);
        assert_eq!(session.pull(version), Pull::Changes(Vec::new()));
        assert_eq!(io.appended.get(), appended);
        assert_eq!(*io.updates.borrow(), updates);
        assert_eq!(io.file(), "abc");
        assert!(!session.dirty());
    }
    assert!(session.push("editor", version, &[insert_at(3, "!", 3)], &io).unwrap());
    assert_eq!(session.text(), "abc!");
    assert_eq!(io.appended.get(), appended + 1);
}

#[test]
fn late_surrogate_split_in_newly_inserted_text_rejects_the_entire_push() {
    let io = FakeIo::with_file("abc");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    let version = session.version();
    let state = session.state();
    let updates = io.updates.borrow().clone();
    // Position 2 is scalar-aligned in the live text, but splits the emoji
    // introduced by the first batch. Validate against each intermediate text.
    let first = insert_at(1, "😀", 3);
    let invalid = insert_at(2, "X", 5);
    assert!(session.push("editor", version, &[first.clone(), invalid], &io).is_err());
    assert_eq!(session.state(), state);
    assert_eq!(session.version(), version);
    assert_eq!(*io.updates.borrow(), updates);
    assert_eq!(session.pull(version), Pull::Changes(Vec::new()));
    assert!(!session.dirty());
    let valid = insert_at(3, "!", 5);
    assert!(session.push("editor", version, &[first, valid], &io).unwrap());
    assert_eq!(session.text(), "a😀!bc");
    assert_eq!(io.updates.borrow().len(), updates.len() + 1);
    let restored = Replica::restore(&io.updates.borrow()).expect("journal replay");
    assert_eq!(restored.text(), "a😀!bc");
}
