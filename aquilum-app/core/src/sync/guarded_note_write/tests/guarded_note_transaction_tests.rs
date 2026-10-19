//! Tests of the actual cooperative writer, not a remote apply implementation.
//! All files are disposable; provider-baseline metadata is explicitly synthetic.

use super::*;
use crate::sync::journal::ReadOnlyExistingReplaceAttempt;
use crate::sync::protection_store::{BoundProtectedOriginal, SqliteProtectionStore};
use crate::sync::{DocumentId, JournalAction, JournalError, JournalPhase, OperationId,
    OperationIntent, RemoteGuard, RevisionId, SqliteSyncJournal};
use std::cell::{Cell, RefCell};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const CHILD: &str =
    "sync::guarded_note_write::tests::transaction::custody_competitor_probe";

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn custody_competitor_probe() {
    let Some(scope) = std::env::var_os("AKULA_P0G_TEST_LOCK_SCOPE") else { return; };
    let root = PinnedProbeRoot::open(Path::new(&scope)).unwrap();
    let held = root.try_lock().unwrap();
    let report = std::env::var_os("AKULA_P0G_TEST_LOCK_REPORT").unwrap();
    fs::write(report, if held.is_some() { "free" } else { "held" }).unwrap();
}

fn assert_independent_lock_state(scope: &Path, expected_free: bool) {
    let temp = tempfile::tempdir().unwrap();
    let report = temp.path().join("observation");
    let log_path = temp.path().join("child.log");
    let log = fs::File::create(&log_path).unwrap();
    let mut child = ChildGuard(Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg(CHILD).arg("--nocapture")
        .env("AKULA_P0G_TEST_LOCK_SCOPE", scope)
        .env("AKULA_P0G_TEST_LOCK_REPORT", &report)
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() { break status; }
        assert!(Instant::now() < deadline, "lock observer timeout: {}",
            fs::read_to_string(&log_path).unwrap_or_default());
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "lock observer failed: {}",
        fs::read_to_string(&log_path).unwrap_or_default());
    assert_eq!(fs::read_to_string(&report).unwrap(),
        if expected_free { "free" } else { "held" });
}

struct Fixture {
    _temp: tempfile::TempDir,
    core: Arc<Core>,
    root: PathBuf,
    appdata: PathBuf,
    scope: PathBuf,
    relative: SyncRelativePath,
    intent: OperationIntent,
    journal: RefCell<SqliteSyncJournal>,
    original: BoundProtectedOriginal,
    new_text: String,
    events: Arc<AtomicUsize>,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let appdata = temp.path().join("appdata");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&appdata).unwrap();
    let root = crate::search::paths::canonical_path(&root);
    let relative = SyncRelativePath::parse("Existing.md").unwrap();
    let old = b"original private markdown";
    fs::write(root.join(relative.as_str()), old).unwrap();
    let root_id = PinnedProbeRoot::open(&root).unwrap().identity();
    let scope = temp.path().join("external").join(scoped_leaf(root_id));
    fs::create_dir_all(&scope).unwrap();
    fs::write(scope.join(LOCK_NAME), b"fixed cooperative lock").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&scope, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(scope.join(LOCK_NAME),
            fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[cfg(unix)]
    let scope = fs::canonicalize(scope).unwrap();
    let events = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&events);
    let callback_scope = scope.clone();
    let core = Core::open(&appdata, Arc::new(move |event: CoreEvent| {
        if matches!(event, CoreEvent::NoteHistoryChanged(_)
            | CoreEvent::NoteHistorySaveFailed(_)) {
            let root = PinnedProbeRoot::open(&callback_scope).unwrap();
            assert!(root.try_lock().unwrap().is_some(),
                "UI callback must execute after custody unlock");
            observed.fetch_add(1, Ordering::SeqCst);
        }
    }));
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    let mut custody = SqliteCustodyIdentityStore::open(
        &admission, &appdata.join("custody.sqlite"),
    ).unwrap();
    let vault = VaultId::new();
    custody.bind_observed(&admission, vault, &scope).unwrap();
    let mut journal = SqliteSyncJournal::open(&appdata.join("journal.sqlite")).unwrap();
    let mut prior = OperationIntent {
        operation_id: OperationId::new(), vault_id: vault,
        document_id: DocumentId::new(), profile_id: "synthetic-test-provider".into(),
        revision_id: RevisionId::new(), action: JournalAction::UploadImmutable,
        path: relative.clone(), remote_object_id: "fixture/old".into(),
        remote_guard: Some(RemoteGuard::Absent),
        expected_content_hash: blake3::hash(old).to_hex().to_string(),
        expected_baseline: None,
    };
    journal.begin(prior.clone()).unwrap();
    journal.mark_staged(prior.operation_id, old).unwrap();
    journal.mark_applied(prior.operation_id).unwrap();
    journal.mark_verified(prior.operation_id, old, "synthetic-validator").unwrap();
    journal.commit(prior.operation_id).unwrap();
    let new_text = "new staged markdown with actual history ".repeat(120);
    prior.expected_baseline = Some(prior.revision_id);
    prior.operation_id = OperationId::new();
    prior.revision_id = RevisionId::new();
    prior.action = JournalAction::StageDownload;
    prior.remote_guard = None;
    prior.remote_object_id = "fixture/new".into();
    prior.expected_content_hash = blake3::hash(new_text.as_bytes()).to_hex().to_string();
    journal.begin(prior.clone()).unwrap();
    journal.mark_staged(prior.operation_id, new_text.as_bytes()).unwrap();
    let mut protected = SqliteProtectionStore::open(&appdata.join("originals.sqlite")).unwrap();
    admission.preserve_closed_original(&relative,
        &blake3::hash(old).to_hex().to_string(), prior.operation_id, &mut protected,
    ).unwrap();
    let original = protected.read_bound_preimage(prior.operation_id).unwrap().unwrap();
    drop(admission);
    Fixture { _temp: temp, core, root, appdata, scope, relative, intent: prior,
        journal: RefCell::new(journal), original, new_text, events }
}

impl Fixture {
    fn claim(&self, admission: &RegisteredReadAdmission<'_>)
        -> Result<ReadOnlyExistingReplaceAttempt, FileCommandError>
    {
        let protected = SqliteProtectionStore::open(
            &self.appdata.join("originals.sqlite"),
        ).unwrap();
        admission.revalidate_bound_preimage(&self.relative,
            &self.original.original.content_hash, self.intent.operation_id, &protected,
        ).map_err(|e| deny(format!("preimage: {e:?}")))?;
        self.journal.borrow_mut().record_existing_replace_attempt_once(
            &self.intent, &self.original,
        ).map_err(|e| deny(format!("one-shot claim: {e:?}")))
    }

    fn assert_unconfirmed_and_non_replayable(&self) {
        let mut reopened = SqliteSyncJournal::open(&self.appdata.join("journal.sqlite")).unwrap();
        assert!(reopened.read_existing_replace_attempt(self.intent.operation_id)
            .unwrap().is_some());
        assert_eq!(reopened.get(self.intent.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
        let old = reopened.baseline(self.intent.vault_id, self.intent.document_id)
            .unwrap().unwrap();
        assert_eq!(Some(old.revision_id), self.intent.expected_baseline);
        assert_eq!(old.content_hash, self.original.original.content_hash);
        assert!(matches!(reopened.record_existing_replace_attempt_once(
            &self.intent, &self.original,
        ), Err(JournalError::ExistingReplaceAttemptAlreadyRecorded)));
        assert!(matches!(reopened.mark_applied(self.intent.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)));
        let protected = SqliteProtectionStore::open(
            &self.appdata.join("originals.sqlite"),
        ).unwrap();
        assert_eq!(protected.read_bound_preimage(self.intent.operation_id)
            .unwrap().unwrap(), self.original);
    }
}

struct Prepared<'a> {
    claim: ReadOnlyExistingReplaceAttempt,
    dropped: &'a Cell<bool>,
}
impl Drop for Prepared<'_> {
    fn drop(&mut self) { self.dropped.set(true); }
}

#[test]
fn one_custody_lifetime_spans_durable_claim_filegate_history_and_postflight() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let custody = SqliteCustodyIdentityStore::open(
        &admission, &f.appdata.join("custody.sqlite"),
    ).unwrap();
    let dropped = Cell::new(false);
    let result = save_existing_note_with_checks_under_custody(
        &admission, &custody, f.intent.vault_id, &f.scope, &f.relative,
        &f.new_text, &f.original.original.content_hash, Source::External,
        Some("one-custody-lifetime"),
        || {
            assert_independent_lock_state(&f.scope, false);
            Ok(Prepared { claim: f.claim(&admission)?, dropped: &dropped })
        },
        |prepared, written| {
            assert!(!dropped.get(), "preparation state was released too soon");
            assert_eq!(prepared.claim.operation_id, f.intent.operation_id);
            assert_independent_lock_state(&f.scope, false);
            assert_eq!(written.hash, f.intent.expected_content_hash);
            let note = f.root.join(f.relative.as_str());
            assert_eq!(fs::read(&note).unwrap(), f.new_text.as_bytes());
            assert!(f.core.history.page(&|| vec![f.root.clone()], &note, 0, 8).total > 0);
            assert_eq!(f.events.load(Ordering::SeqCst), 0,
                "no UI event before postflight/custody completion");
            Ok(())
        },
    );
    assert!(result.is_ok(), "joined lifetime failed: {result:?}");
    assert!(dropped.get());
    assert!(f.events.load(Ordering::SeqCst) > 0);
    assert_independent_lock_state(&f.scope, true);
    f.assert_unconfirmed_and_non_replayable();
}

#[test]
fn preparation_error_after_claim_is_needs_review_without_primary_write() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let custody = SqliteCustodyIdentityStore::open(
        &admission, &f.appdata.join("custody.sqlite"),
    ).unwrap();
    let result = save_existing_note_with_checks_under_custody(
        &admission, &custody, f.intent.vault_id, &f.scope, &f.relative,
        &f.new_text, &f.original.original.content_hash, Source::External, None,
        || -> Result<(), FileCommandError> {
            f.claim(&admission)?;
            Err(deny("injected failure after durable claim"))
        },
        |_, _| panic!("postflight must not run after preparation failure"),
    );
    assert!(matches!(result, Err(GuardedNoteWriteError::NeedsReview(_))));
    assert_eq!(fs::read(f.root.join(f.relative.as_str())).unwrap(), f.original.original.bytes);
    assert_eq!(f.events.load(Ordering::SeqCst), 0);
    assert_independent_lock_state(&f.scope, true);
    f.assert_unconfirmed_and_non_replayable();
}

#[test]
fn history_failure_skips_postflight_and_preserves_uncertain_claim() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let custody = SqliteCustodyIdentityStore::open(
        &admission, &f.appdata.join("custody.sqlite"),
    ).unwrap();
    f.core.history.fail_once(HistoryFault::CreateVersion);
    let result = save_existing_note_with_checks_under_custody(
        &admission, &custody, f.intent.vault_id, &f.scope, &f.relative,
        &f.new_text, &f.original.original.content_hash, Source::External, None,
        || f.claim(&admission),
        |_, _| panic!("failed history cannot produce a postflight receipt"),
    );
    assert!(matches!(result, Err(GuardedNoteWriteError::NeedsReview(_))));
    assert_eq!(fs::read(f.root.join(f.relative.as_str())).unwrap(), f.new_text.as_bytes());
    assert!(f.events.load(Ordering::SeqCst) > 0);
    assert_independent_lock_state(&f.scope, true);
    f.assert_unconfirmed_and_non_replayable();
}

#[test]
fn postflight_failure_announces_real_history_only_after_unlock_without_sync_ack() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let custody = SqliteCustodyIdentityStore::open(
        &admission, &f.appdata.join("custody.sqlite"),
    ).unwrap();
    let result = save_existing_note_with_checks_under_custody(
        &admission, &custody, f.intent.vault_id, &f.scope, &f.relative,
        &f.new_text, &f.original.original.content_hash, Source::External,
        Some("history committed but postflight failed"),
        || f.claim(&admission),
        |_, _| {
            assert_eq!(f.events.load(Ordering::SeqCst), 0);
            Err(deny("injected postflight readback/receipt failure"))
        },
    );
    assert!(matches!(result, Err(GuardedNoteWriteError::NeedsReview(_))));
    assert_eq!(fs::read(f.root.join(f.relative.as_str())).unwrap(), f.new_text.as_bytes());
    assert!(f.events.load(Ordering::SeqCst) > 0,
        "actual committed history must be announced even after failed postflight");
    assert_independent_lock_state(&f.scope, true);
    f.assert_unconfirmed_and_non_replayable();
}

#[test]
fn missing_custody_refuses_before_preparation_and_primary_write() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let missing = SqliteCustodyIdentityStore::open(
        &admission, &f.appdata.join("unbound-custody.sqlite"),
    ).unwrap();
    let result = save_existing_note_with_checks_under_custody(
        &admission, &missing, f.intent.vault_id, &f.scope, &f.relative,
        &f.new_text, &f.original.original.content_hash, Source::External, None,
        || -> Result<(), FileCommandError> { panic!("unbound custody cannot prepare") },
        |_, _| panic!("unbound custody cannot postflight"),
    );
    assert!(matches!(result, Err(GuardedNoteWriteError::Denied(_))));
    assert!(f.journal.borrow().read_existing_replace_attempt(f.intent.operation_id)
        .unwrap().is_none());
    assert_eq!(fs::read(f.root.join(f.relative.as_str())).unwrap(), f.original.original.bytes);
}
