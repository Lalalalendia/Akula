use super::*;
use serde_json::json;
use std::cell::{Cell, RefCell};

#[derive(Default)]
struct FakeIo {
    file: RefCell<Option<String>>,
    point: RefCell<Option<SyncPoint>>,
    copies: RefCell<Vec<(ConflictCause, String)>>,
    appended: Cell<usize>,
    updates: RefCell<Vec<Vec<u8>>>,
    fail_append: Cell<bool>,
    fail_remember: Cell<bool>,
    remembered: Cell<usize>,
    writes: Cell<usize>,
    /// Simulate a successful physical file write followed by a lost ACK.
    ambiguous_after_write: Cell<bool>,
    guarded_native_writer: Cell<bool>,
    durable_intent: RefCell<Option<(String, String)>>,
    reads: Cell<usize>,
    hash_checks: Cell<usize>,
    fail_preserve: Cell<bool>,
    changed_on_next_write: RefCell<Option<String>>,
}

fn hash_of(text: &str) -> String {
    format!("h:{text}")
}

impl FakeIo {
    fn with_file(text: &str) -> Self {
        Self { file: RefCell::new(Some(text.to_owned())), ..Self::default() }
    }

    fn file(&self) -> String {
        self.file.borrow().clone().expect("file exists")
    }

    fn edit_file(&self, text: &str) {
        *self.file.borrow_mut() = Some(text.to_owned());
    }
}

impl SessionIo for FakeIo {
    fn read(&self) -> Result<DiskSnapshot, DiskError> {
        self.reads.set(self.reads.get() + 1);
        let content = self.file.borrow().clone().ok_or(DiskError::Missing)?;
        Ok(DiskSnapshot { hash: hash_of(&content), text_hash: hash_of(&content), content })
    }

    fn hash(&self) -> Result<String, DiskError> {
        self.hash_checks.set(self.hash_checks.get() + 1);
        self.file.borrow().as_deref().map(hash_of).ok_or(DiskError::Missing)
    }

    fn write(&self, content: &str, expected_hash: &str) -> Result<String, DiskError> {
        if let Some(external) = self.changed_on_next_write.borrow_mut().take() {
            self.edit_file(&external);
        }
        let current = self.file.borrow().clone().ok_or(DiskError::Missing)?;
        if hash_of(&current) != expected_hash {
            return Err(DiskError::Conflict);
        }
        self.writes.set(self.writes.get() + 1);
        self.edit_file(content);
        if self.ambiguous_after_write.replace(false) {
            return Err(DiskError::Other {
                code: "needs_review",
                message: "native postflight ambiguous: file may be written".into(),
            });
        }
        Ok(hash_of(content))
    }

    fn text_hash(&self, text: &str) -> String {
        hash_of(text)
    }

    fn preserve(&self, body: &str, report: ConflictReport) -> Result<(), DiskError> {
        if self.fail_preserve.get() {
            return Err(DiskError::Other {
                code: "preservation_failed",
                message: "simulated full disk or permissions error".into(),
            });
        }
        self.copies.borrow_mut().push((report.cause, body.to_owned()));
        Ok(())
    }

    fn append(&self, update: &[u8]) -> Result<(), DiskError> {
        if self.fail_append.get() {
            return Err(DiskError::Other {
                code: "append_failed",
                message: "simulated SQLite I/O failure".into(),
            });
        }
        self.appended.set(self.appended.get() + 1);
        self.updates.borrow_mut().push(update.to_vec());
        Ok(())
    }

    fn sync_point(&self) -> SyncPointLookup {
        self.point.borrow().clone().map_or(SyncPointLookup::Absent, SyncPointLookup::Found)
    }

    fn remember(&self, point: SyncPoint) -> Result<(), DiskError> {
        if self.fail_remember.get() {
            return Err(DiskError::Other {
                code: "remember_failed",
                message: "simulated SQLite metadata failure".into(),
            });
        }
        *self.point.borrow_mut() = Some(point);
        self.remembered.set(self.remembered.get() + 1);
        Ok(())
    }

    fn unresolved_native_write(&self) -> Result<Option<String>, DiskError> {
        Ok(self.durable_intent.borrow().as_ref().map(|_| "unknown native publication".into()))
    }

    fn begin_guarded_write(&self, content: &str, base_hash: &str) -> Result<bool, DiskError> {
        if !self.guarded_native_writer.get() { return Ok(false); }
        let mut intent = self.durable_intent.borrow_mut();
        if intent.is_some() {
            return Err(DiskError::Other {
                code: "needs_review", message: "unresolved previous attempt".into(),
            });
        }
        *intent = Some((base_hash.to_owned(), hash_of(content)));
        Ok(true)
    }

    fn complete_guarded_write(&self, base_hash: &str, new_hash: &str)
        -> Result<(), DiskError>
    {
        let current = self.durable_intent.borrow();
        if current.as_ref() != Some(&(base_hash.to_owned(), new_hash.to_owned())) {
            return Err(DiskError::Other {
                code: "needs_review", message: "prewrite intent not matched".into(),
            });
        }
        drop(current);
        *self.durable_intent.borrow_mut() = None;
        Ok(())
    }
}

fn replica_with(text: &str) -> Vec<u8> {
    let replica = Replica::new();
    replica.apply_edits(&[TextEdit { from: 0, to: 0, insert: text.into() }], "test");
    replica.state()
}

fn insert_at(position: usize, text: &str, length: usize) -> Value {
    edits_to_json(&[TextEdit { from: position, to: position, insert: text.into() }], length)
}

#[test]
fn a_first_open_takes_the_file_and_remembers_the_sync_point() {
    let io = FakeIo::with_file("первая\nвторая\n");
    let session = DocumentSession::open(&[], &io).expect("open");
    assert_eq!(session.text(), "первая\nвторая\n");
    assert!(!session.dirty());
    assert_eq!(io.point.borrow().as_ref().map(|point| point.file_hash.clone()), Some(hash_of("первая\nвторая\n")));
}

#[test]
fn input_that_never_reached_the_disk_is_published_after_a_crash() {
    let io = FakeIo::with_file("текст\n");
    *io.point.borrow_mut() = Some(SyncPoint { file_hash: hash_of("текст\n"), text_hash: hash_of("текст\n") });
    let mut session = DocumentSession::open(&[replica_with("текст и недописанное\n")], &io).expect("open");
    assert!(session.dirty());
    session.write(&io).expect("write");
    assert_eq!(io.file(), "текст и недописанное\n");
}

#[test]
fn both_sides_changed_while_closed_keeps_the_document_as_a_copy() {
    let io = FakeIo::with_file("правка снаружи\n");
    *io.point.borrow_mut() = Some(SyncPoint { file_hash: hash_of("было\n"), text_hash: hash_of("было\n") });
    let session = DocumentSession::open(&[replica_with("правка внутри\n")], &io).expect("open");
    assert_eq!(session.text(), "правка снаружи\n");
    assert_eq!(*io.copies.borrow(), vec![(ConflictCause::Diverged, "правка внутри\n".to_owned())]);
}

#[test]
fn a_missing_file_does_not_open() {
    let io = FakeIo::default();
    assert!(matches!(DocumentSession::open(&[], &io), Err(SessionError::Missing)));
}

#[test]
fn accepts_changes_based_on_the_current_version_and_rejects_stale_ones() {
    let io = FakeIo::with_file("abc");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    let version = session.version();

    assert!(session.push("editor", version, &[insert_at(3, "d", 3)], &io).expect("push"));
    assert!(!session.push("editor", version, &[insert_at(4, "e", 4)], &io).expect("push"));

    assert_eq!(session.text(), "abcd");
    assert_eq!(session.version(), version + 1);
    assert!(session.dirty());
}

#[test]
fn applies_a_batch_of_changes_in_order() {
    let io = FakeIo::with_file("abc");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    let version = session.version();
    let batch = [insert_at(0, "1", 3), insert_at(4, "2", 4)];
    assert!(session.push("editor", version, &batch, &io).expect("push"));
    assert_eq!(session.text(), "1abc2");
    assert_eq!(session.version(), version + 2);
}

#[test]
fn refuses_a_change_set_for_another_length_without_applying_any_of_the_batch() {
    let io = FakeIo::with_file("abc");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    let version = session.version();
    let batch = [insert_at(0, "1", 3), insert_at(0, "2", 99)];
    assert!(session.push("editor", version, &batch, &io).is_err());
    assert_eq!(session.text(), "abc");
    assert_eq!(session.version(), version);
}

#[path = "session_change_validation_tests.rs"]
mod change_validation;

#[test]
fn pull_returns_what_happened_after_the_given_version() {
    let io = FakeIo::with_file("abc");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    let start = session.version();
    session.push("left", start, &[insert_at(0, "x", 3)], &io).expect("push");
    session.replace_text("xabc!", SYSTEM_CLIENT, &io).unwrap();

    let Pull::Changes(changes) = session.pull(start) else { panic!("expected changes") };
    assert_eq!(changes.iter().map(|change| change.client.as_str()).collect::<Vec<_>>(), ["left", "system"]);
    assert_eq!(changes[0].changes, json!([[0, "x"], 3]));
    assert_eq!(session.pull(session.version()), Pull::Changes(Vec::new()));
    assert_eq!(session.pull(session.version() + 1), Pull::Resync);
}

#[test]
fn an_external_edit_merges_with_unsaved_input_on_another_line() {
    let io = FakeIo::with_file("один\nдва\nтри\n");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    let version = session.version();
    session.push("editor", version, &[insert_at(4, " ввод", 13)], &io).expect("push");
    io.edit_file("один\nдва\nтри от агента\n");

    session.reconcile(&io).expect("reconcile");

    assert_eq!(session.text(), "один ввод\nдва\nтри от агента\n");
    assert!(session.dirty());
    let Pull::Changes(changes) = session.pull(version + 1) else { panic!("expected changes") };
    assert_eq!(changes[0].client, DISK_CLIENT);
}

#[test]
fn an_unchanged_file_costs_only_the_hash_check() {
    let io = FakeIo::with_file("текст");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    let version = session.version();
    session.reconcile(&io).expect("reconcile");
    assert_eq!(session.version(), version);
}

#[test]
fn a_write_conflict_pulls_the_disk_and_writes_both_sides() {
    let io = FakeIo::with_file("один\nдва\n");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    let version = session.version();
    session.push("editor", version, &[insert_at(4, "!", 9)], &io).expect("push");
    *io.changed_on_next_write.borrow_mut() = Some("один\nдва и агент\n".into());

    session.write(&io).expect("write");

    assert_eq!(io.file(), "один!\nдва и агент\n");
    assert!(!session.dirty());
}

#[test]
fn emptying_a_note_keeps_its_previous_text_as_a_copy() {
    let io = FakeIo::with_file("важный текст\n");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    session.replace_text("", "editor", &io).unwrap();
    session.write(&io).expect("write");
    assert_eq!(io.file(), "");
    assert_eq!(*io.copies.borrow(), vec![(ConflictCause::Truncated, "важный текст\n".to_owned())]);
}

#[test]
fn a_deleted_note_is_not_resurrected_by_saving() {
    let io = FakeIo::with_file("текст");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    session.replace_text("текст и ещё", "editor", &io).unwrap();
    *io.file.borrow_mut() = None;
    assert!(matches!(session.write(&io), Err(SessionError::Missing)));
    assert!(io.file.borrow().is_none());
}

#[test]
fn native_lost_ack_sticks_as_hold_across_reconcile_retry_and_new_edits() {
    let io = FakeIo::with_file("abc");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    session.replace_text("abcd", "editor", &io).unwrap();
    let confirmed = io.point.borrow().as_ref().unwrap().file_hash.clone();
    assert_eq!(confirmed, hash_of("abc"));

    io.ambiguous_after_write.set(true);
    let first = session.write(&io);
    assert!(matches!(first,
        Err(SessionError::Disk { code: "needs_review", .. })));
    assert!(session.requires_review());
    assert!(session.dirty());
    assert_eq!(io.writes.get(), 1);
    // The primary file MAY already have been published by the failed call.
    assert_eq!(io.file(), "abcd");
    // Neither the presumed file nor a failed history postflight is a
    // trustworthy newly confirmed sync point.
    assert_eq!(io.point.borrow().as_ref().unwrap().file_hash, confirmed);

    // Even if an external process changes the file, neither reconcile nor
    // a repeated write can infer provenance or attempt a second publication.
    io.edit_file("outside concurrent write");
    let before_reads = io.reads.get();
    let before_hashes = io.hash_checks.get();
    assert!(matches!(session.reconcile(&io),
        Err(SessionError::Disk { code: "needs_review", .. })));
    assert_eq!(io.reads.get(), before_reads);
    assert_eq!(io.hash_checks.get(), before_hashes);
    assert_eq!(session.text(), "abcd");
    assert!(matches!(session.write(&io),
        Err(SessionError::Disk { code: "needs_review", .. })));
    assert_eq!(io.writes.get(), 1);
    assert_eq!(io.file(), "outside concurrent write");

    // New user changes are still durably appended to the local CRDT log,
    // but they cannot implicitly clear the HOLD or touch the disk.
    let appends_before = io.appended.get();
    let v = session.version();
    assert!(session.push("editor", v, &[insert_at(4, "!", 4)], &io).unwrap());
    assert_eq!(session.text(), "abcd!");
    assert_eq!(io.appended.get(), appends_before + 1);
    assert!(session.requires_review());
    assert!(session.write(&io).is_err());
    assert_eq!(io.writes.get(), 1);
    assert_eq!(io.point.borrow().as_ref().unwrap().file_hash, confirmed);
}

#[test]
fn guarded_write_completes_only_after_sync_point_is_durable() {
    let io = FakeIo::with_file("old");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    io.guarded_native_writer.set(true);
    session.replace_text("new text", "editor", &io).unwrap();
    session.write(&io).unwrap();
    assert_eq!(io.file(), "new text");
    assert!(io.durable_intent.borrow().is_none());
    assert_eq!(
        io.point.borrow().as_ref().unwrap().file_hash, hash_of("new text"),
    );
    assert!(!session.requires_review());
    assert!(!session.dirty());
}

#[test]
fn guarded_metadata_failure_preserves_crash_intent_and_blocks_reopen() {
    let io = FakeIo::with_file("old");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    io.guarded_native_writer.set(true);
    session.replace_text("new content", "editor", &io).unwrap();
    io.fail_remember.set(true);
    let result = session.write(&io);
    assert!(matches!(
        result, Err(SessionError::Disk { code: "needs_review", .. })
    ));
    assert!(session.requires_review());
    assert_eq!(io.file(), "new content");
    assert_eq!(io.writes.get(), 1);
    assert_eq!(
        io.point.borrow().as_ref().unwrap().file_hash, hash_of("old"),
    );
    assert_eq!(io.durable_intent.borrow().as_ref(), Some(&(
        hash_of("old"), hash_of("new content"),
    )));
    // Even a fresh DocumentSession cannot silently treat the visible disk
    // file as an authenticated completed write.
    let before_reads = io.reads.get();
    assert!(matches!(
        DocumentSession::open(&[], &io),
        Err(SessionError::Disk { code: "needs_review", .. })
    ));
    assert_eq!(io.reads.get(), before_reads);
    assert!(session.write(&io).is_err());
    assert_eq!(io.writes.get(), 1);
}

#[test]
fn the_sync_point_is_remembered_only_after_a_successful_write() {
    let io = FakeIo::with_file("старое");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    session.replace_text("новое", "editor", &io).unwrap();
    assert_eq!(io.point.borrow().as_ref().map(|point| point.file_hash.clone()), Some(hash_of("старое")));
    session.write(&io).expect("write");
    assert_eq!(io.point.borrow().as_ref().map(|point| point.file_hash.clone()), Some(hash_of("новое")));
}

#[test]
fn every_document_change_is_persisted() {
    let io = FakeIo::with_file("abc");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    let before = io.appended.get();
    session.push("editor", session.version(), &[insert_at(0, "x", 3)], &io).expect("push");
    session.replace_text("xabcy", "editor", &io).unwrap();
    assert_eq!(io.appended.get(), before + 2);
}

#[test]
fn reverting_a_version_undoes_its_lines_and_keeps_later_ones_as_a_copy() {
    let io = FakeIo::with_file("один\nдва правка версии\nтри позже\n");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    let changed = session.revert("один\nдва правка версии\nтри\n", "один\nдва\nтри\n", SYSTEM_CLIENT, &io).unwrap();
    assert!(changed);
    assert_eq!(session.text(), "один\nдва\nтри позже\n");
    assert!(session.dirty());
}

#[test]
fn reverting_lines_changed_later_preserves_the_later_text() {
    let io = FakeIo::with_file("один\nдва ещё позже\n");
    let mut session = DocumentSession::open(&[], &io).expect("open");
    session.revert("один\nдва правка версии\n", "один\nдва\n", SYSTEM_CLIENT, &io).unwrap();
    assert_eq!(session.text(), "один\nдва\n");
    assert_eq!(*io.copies.borrow(), vec![(ConflictCause::Reverted, "два ещё позже".to_owned())]);
}


#[test]
fn a_failed_conflict_copy_blocks_open_without_publishing_over_the_document() {
    let io = FakeIo::with_file("edited outside\n");
    *io.point.borrow_mut() = Some(SyncPoint {
        file_hash: hash_of("original\n"),
        text_hash: hash_of("original\n"),
    });
    io.fail_preserve.set(true);
    let result = DocumentSession::open(&[replica_with("unsaved editor edit\n")], &io);
    assert!(matches!(
        result,
        Err(SessionError::Disk { code: "preservation_failed", .. })
    ));
    assert_eq!(io.file(), "edited outside\n");
    assert!(io.copies.borrow().is_empty());
    assert_eq!(io.writes.get(), 0);
}

#[test]
fn failed_preservation_prevents_destructive_truncation_and_keeps_retry_possible() {
    let io = FakeIo::with_file("important text\n");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    session.replace_text("", "editor", &io).unwrap();
    io.fail_preserve.set(true);
    let result = session.write(&io);
    assert!(matches!(
        result,
        Err(SessionError::Disk { code: "preservation_failed", .. })
    ));
    assert_eq!(io.file(), "important text\n");
    assert!(session.dirty());
    assert_eq!(io.writes.get(), 0);
    assert!(io.copies.borrow().is_empty());

    io.fail_preserve.set(false);
    session.write(&io).unwrap();
    assert_eq!(io.file(), "");
    assert_eq!(
        *io.copies.borrow(),
        vec![(ConflictCause::Truncated, "important text\n".into())]
    );
}

#[test]
fn failed_preservation_cancels_revert_before_changing_the_replica() {
    let io = FakeIo::with_file("one\nlater independent edit\n");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    let original = session.text();
    let version = session.version();
    io.fail_preserve.set(true);
    let result = session.revert(
        "one\nversion edit\n",
        "one\nbase\n",
        SYSTEM_CLIENT,
        &io,
    );
    assert!(matches!(
        result,
        Err(SessionError::Disk { code: "preservation_failed", .. })
    ));
    assert_eq!(session.text(), original);
    assert_eq!(session.version(), version);
    assert_eq!(io.writes.get(), 0);
}


#[test]
fn failed_preservation_blocks_an_overlapping_external_merge() {
    let io = FakeIo::with_file("one\ntwo\nthree\n");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    session.replace_text("one\ntwo typed locally\nthree\n", "editor", &io).unwrap();
    let original = session.text();
    let version = session.version();

    io.edit_file("one\ntwo updated externally\nthree\n");
    io.fail_preserve.set(true);
    assert!(matches!(
        session.reconcile(&io),
        Err(SessionError::Disk { code: "preservation_failed", .. })
    ));
    assert_eq!(session.text(), original);
    assert_eq!(session.version(), version);
    assert!(session.dirty());
    assert_eq!(io.file(), "one\ntwo updated externally\nthree\n");
    assert!(io.copies.borrow().is_empty());
}


#[test]
fn failed_append_rejects_a_push_without_mutating_or_acknowledging_it() {
    let io = FakeIo::with_file("abc");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    let before = session.text();
    let version = session.version();
    let state = session.state();
    let count = io.appended.get();
    io.fail_append.set(true);

    let result = session.push("editor", version, &[insert_at(3, "d", 3)], &io);
    assert!(matches!(
        result,
        Err(SessionError::Disk { code: "append_failed", .. })
    ));
    assert_eq!(session.text(), before);
    assert_eq!(session.version(), version);
    assert_eq!(session.state(), state);
    assert!(!session.dirty());
    assert_eq!(io.appended.get(), count);

    io.fail_append.set(false);
    assert!(session.push("editor", version, &[insert_at(3, "d", 3)], &io).unwrap());
    assert_eq!(session.text(), "abcd");
    assert_eq!(session.version(), version + 1);
    assert_eq!(io.appended.get(), count + 1);
}

#[test]
fn a_batch_is_appended_as_a_single_atomic_update() {
    let io = FakeIo::with_file("abc");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    let before = io.appended.get();
    let version = session.version();
    let changes = [insert_at(0, "x", 3), insert_at(4, "y", 4)];
    session.push("editor", version, &changes, &io).unwrap();
    assert_eq!(io.appended.get(), before + 1);
    assert_eq!(session.version(), version + 2);
    assert_eq!(session.text(), "xabcy");

    let restored = Replica::restore(&io.updates.borrow()).unwrap();
    assert_eq!(restored.text(), session.text());
}

#[test]
fn journal_failure_blocks_replace_text_before_in_memory_mutation() {
    let io = FakeIo::with_file("the original");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    let version = session.version();
    let state = session.state();
    io.fail_append.set(true);
    let result = session.replace_text("unsaved new version", "editor", &io);
    assert!(matches!(
        result,
        Err(SessionError::Disk { code: "append_failed", .. })
    ));
    assert_eq!(session.text(), "the original");
    assert_eq!(session.version(), version);
    assert_eq!(session.state(), state);
    assert_eq!(io.file(), "the original");
}

#[test]
fn journal_failure_blocks_external_reconciliation_without_losing_file_bytes() {
    let io = FakeIo::with_file("a\nb\nc\n");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    let original = session.text();
    let version = session.version();
    io.edit_file("a\nB\nc\n");
    io.fail_append.set(true);
    let result = session.reconcile(&io);
    assert!(matches!(
        result,
        Err(SessionError::Disk { code: "append_failed", .. })
    ));
    assert_eq!(session.text(), original);
    assert_eq!(session.version(), version);
    assert_eq!(io.file(), "a\nB\nc\n");

    io.fail_append.set(false);
    session.reconcile(&io).unwrap();
    assert_eq!(session.text(), "a\nB\nc\n");
}

#[test]
fn journal_failure_blocks_revert_and_preserves_original_bytes() {
    let io = FakeIo::with_file("one\nlater independent edit\n");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    let original = session.text();
    let version = session.version();
    io.fail_append.set(true);
    let result = session.revert("one\nversion edit\n", "one\nbase\n", SYSTEM_CLIENT, &io);
    assert!(matches!(
        result,
        Err(SessionError::Disk { code: "append_failed", .. })
    ));
    assert_eq!(session.text(), original);
    assert_eq!(session.version(), version);
    assert_eq!(io.file(), original);
}


#[test]
fn missing_sync_point_commit_fails_open_without_mutating_the_document() {
    let io = FakeIo::with_file("on-disk text\n");
    let stored = replica_with("on-disk text\n");
    io.fail_remember.set(true);
    let result = DocumentSession::open(&[stored.clone()], &io);
    assert!(matches!(
        result,
        Err(SessionError::Disk { code: "remember_failed", .. })
    ));
    assert_eq!(io.file(), "on-disk text\n");
    assert_eq!(io.appended.get(), 0);
    assert!(io.point.borrow().is_none());
    assert_eq!(io.writes.get(), 0);

    io.fail_remember.set(false);
    let opened = DocumentSession::open(&[stored], &io).unwrap();
    assert_eq!(opened.text(), "on-disk text\n");
    assert!(!opened.dirty());
    assert_eq!(io.point.borrow().as_ref().unwrap().file_hash, hash_of("on-disk text\n"));
}

#[test]
fn metadata_failure_after_successful_write_is_retryable_without_duplicate_file_write() {
    let io = FakeIo::with_file("original\n");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    let before = io.point.borrow().clone();
    session.replace_text("updated\n", "editor", &io).unwrap();
    let version = session.version();

    io.fail_remember.set(true);
    let error = session.write(&io);
    assert!(matches!(
        error,
        Err(SessionError::Disk { code: "remember_failed", .. })
    ));
    // Bytes have reached disk but the sync point was NOT acknowledged.
    assert_eq!(io.file(), "updated\n");
    assert!(session.dirty());
    assert_eq!(session.version(), version);
    assert_eq!(*io.point.borrow(), before);
    assert_eq!(io.writes.get(), 1);

    // The expected old hash forces a readback/reconcile; the retry records
    // metadata without another destructive write.
    io.fail_remember.set(false);
    session.write(&io).unwrap();
    assert!(!session.dirty());
    assert_eq!(io.writes.get(), 1);
    assert_eq!(io.file(), "updated\n");
    assert_eq!(io.point.borrow().as_ref().unwrap().file_hash, hash_of("updated\n"));
}

#[test]
fn failed_metadata_commit_during_external_reconcile_retries_without_a_second_delta() {
    let io = FakeIo::with_file("base\n");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    let appended_before = io.appended.get();
    io.edit_file("external\n");
    io.fail_remember.set(true);

    assert!(matches!(
        session.reconcile(&io),
        Err(SessionError::Disk { code: "remember_failed", .. })
    ));
    assert_eq!(io.file(), "external\n");
    assert_eq!(session.text(), "external\n");
    assert!(session.dirty());
    assert_eq!(io.appended.get(), appended_before + 1);
    assert_eq!(io.point.borrow().as_ref().unwrap().file_hash, hash_of("base\n"));

    io.fail_remember.set(false);
    session.reconcile(&io).unwrap();
    assert_eq!(io.appended.get(), appended_before + 1);
    assert_eq!(session.text(), "external\n");
    assert!(!session.dirty());
    assert_eq!(io.point.borrow().as_ref().unwrap().file_hash, hash_of("external\n"));
}

#[test]
fn changed_file_after_metadata_failure_must_preserve_conflicting_local_bytes() {
    let io = FakeIo::with_file("base\n");
    let mut session = DocumentSession::open(&[], &io).unwrap();
    session.replace_text("local\n", "editor", &io).unwrap();
    io.fail_remember.set(true);
    assert!(matches!(
        session.write(&io),
        Err(SessionError::Disk { code: "remember_failed", .. })
    ));
    assert_eq!(io.file(), "local\n");
    assert_eq!(io.writes.get(), 1);

    // A different program now edits the disk after our successful file write.
    io.edit_file("external\n");
    io.fail_remember.set(false);
    session.write(&io).unwrap();
    assert_eq!(io.file(), "external\n");
    assert_eq!(io.writes.get(), 1);
    assert!(io.copies.borrow().iter().any(|(_, body)| body.contains("local")));
}
