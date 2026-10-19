//! Real temporary Core/SQLite/File Gate tests. Remote metadata is synthetic;
//! this suite is not a MinIO transfer or native crash-durability proof.
use super::*;
use crate::app_core::CoreEvent;
use crate::documents::hub::HubError;
use crate::history::{Source, service::HistoryFault};
use crate::sync::filegate_custody_identity::SqliteCustodyIdentityStore;
use crate::sync::filegate_lock_custody::scoped_leaf;
use crate::sync::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
use crate::sync::guarded_note_write::{save_existing_note_with_checks_under_custody,
    save_existing_note_with_prewrite_under_custody, GuardedNoteWriteError};
use crate::sync::s3_preimage_capture::capture_registered_s3_update_preimage;
use crate::sync::{DocumentId, JournalAction, JournalError, OperationId, RemoteGuard,
    RevisionId, S3HeadPointerV1, S3ManifestV1, S3VerifiedHead, SyncRelativePath, VaultId};
use std::cell::{Cell, RefCell};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

struct Fixture {
    _temp: tempfile::TempDir,
    core: Arc<Core>,
    root: PathBuf,
    appdata: PathBuf,
    scope: PathBuf,
    profile: S3ConnectionProfile,
    keyspace: S3Keyspace,
    bindings: SqliteRegisteredS3Bindings,
    custody: SqliteCustodyIdentityStore,
    journal: RefCell<SqliteSyncJournal>,
    cache: SqliteDagStore,
    protected: SqliteProtectionStore,
    intent: OperationIntent,
    old_hash: String,
    new_text: String,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    let appdata = temp.path().join("appdata");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&appdata).unwrap();
    let root = crate::search::paths::canonical_path(&root);
    let relative = SyncRelativePath::parse("Reserved.md").unwrap();
    let original = b"unchanged private original";
    fs::write(root.join(relative.as_str()), original).unwrap();
    let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    let scope = temp.path().join("external")
        .join(scoped_leaf(PinnedProbeRoot::open(&root).unwrap().identity()));
    fs::create_dir_all(&scope).unwrap();
    fs::write(scope.join(LOCK_NAME), b"existing shared fixture lock").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&scope, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(scope.join(LOCK_NAME), fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[cfg(unix)]
    let scope = fs::canonicalize(scope).unwrap();
    let keyspace = S3Keyspace::new("akula/reservation-fixture", VaultId::new()).unwrap();
    let profile = S3ConnectionProfile {
        bucket: "reservation-fixture".into(), region: "us-east-1".into(),
        endpoint: Some("http://127.0.0.1:9000".into()), force_path_style: true,
    };
    let manifest = S3ManifestV1::new(&keyspace, OperationId::new(), 1, None, vec![]).unwrap();
    let head = S3VerifiedHead {
        pointer: S3HeadPointerV1::new(&keyspace, 1, manifest.manifest_id,
            &manifest.digest(&keyspace).unwrap(), None).unwrap(),
        etag: "\"synthetic-provider\"".into(), manifest,
    };
    let mut bindings = SqliteRegisteredS3Bindings::open(&admission, &appdata.join("profile.sqlite")).unwrap();
    bindings.bind_existing_verified_s3(&admission, "fixture", &profile, &keyspace, &head).unwrap();
    let mut custody = SqliteCustodyIdentityStore::open(&admission, &appdata.join("custody.sqlite")).unwrap();
    custody.bind_registered_s3(&admission, &bindings, &scope).unwrap();
    let mut journal = SqliteSyncJournal::open(&appdata.join("journal.sqlite")).unwrap();
    let old_hash = blake3::hash(original).to_hex().to_string();
    let prior = OperationIntent {
        operation_id: OperationId::new(), vault_id: keyspace.vault_id(),
        document_id: DocumentId::new(), profile_id: "fixture".into(),
        revision_id: RevisionId::new(), action: JournalAction::UploadImmutable,
        path: relative.clone(), remote_object_id: keyspace.immutable_blob_key(&old_hash).unwrap(),
        remote_guard: Some(RemoteGuard::Absent), expected_content_hash: old_hash.clone(),
        expected_baseline: None,
    };
    journal.begin(prior.clone()).unwrap();
    journal.mark_staged(prior.operation_id, original).unwrap();
    journal.mark_applied(prior.operation_id).unwrap();
    journal.mark_verified(prior.operation_id, original, "synthetic-previous-validator").unwrap();
    journal.commit(prior.operation_id).unwrap();
    let new_text = "remote candidate using the real local history service ".repeat(100);
    let hash = blake3::hash(new_text.as_bytes()).to_hex().to_string();
    let intent = OperationIntent {
        operation_id: OperationId::new(), revision_id: RevisionId::new(),
        action: JournalAction::StageDownload, remote_guard: None,
        expected_baseline: Some(prior.revision_id), expected_content_hash: hash.clone(),
        remote_object_id: keyspace.immutable_blob_key(&hash).unwrap(), ..prior
    };
    journal.begin(intent.clone()).unwrap();
    journal.mark_staged(intent.operation_id, new_text.as_bytes()).unwrap();
    let mut cache = SqliteDagStore::open(&appdata.join("cache.sqlite")).unwrap();
    cache.store_blob(new_text.as_bytes()).unwrap();
    let mut protected = SqliteProtectionStore::open(&appdata.join("originals.sqlite")).unwrap();
    capture_registered_s3_update_preimage(&admission, &bindings, &profile, &keyspace,
        &custody, &scope, &journal, &mut protected, intent.operation_id, &old_hash).unwrap();
    drop(admission);
    Fixture { _temp: temp, core, root, appdata, scope, profile, keyspace, bindings,
        custody, journal: RefCell::new(journal), cache, protected, intent, old_hash, new_text }
}

fn prepare<'a>(f: &Fixture, admission: &'a RegisteredReadAdmission<'_>)
    -> Result<ReservedS3Replace<'a>, FileCommandError>
{
    prepare_reserved_s3_replace(admission, &f.bindings, &f.profile, &f.keyspace,
        &mut f.journal.borrow_mut(), &f.cache, &f.protected,
        &f.intent, &f.new_text, &f.old_hash)
}

fn assert_editor_reserved(f: &Fixture) {
    let target = f.root.join(f.intent.path.as_str());
    assert!(matches!(f.core.documents.open(&f.core, &target, 0), Err(HubError::SyncReserved)));
}

fn assert_reservation_released(f: &Fixture) {
    let target = f.root.join(f.intent.path.as_str());
    assert!(f.core.documents.reserve_sync_closed_document(&target).is_ok());
}

fn assert_no_remote_ack(f: &Fixture) {
    let mut reopened = SqliteSyncJournal::open(&f.appdata.join("journal.sqlite")).unwrap();
    let baseline = reopened.baseline(f.intent.vault_id, f.intent.document_id).unwrap().unwrap();
    assert_eq!(Some(baseline.revision_id), f.intent.expected_baseline);
    assert_eq!(baseline.content_hash, f.old_hash);
    assert_eq!(reopened.get(f.intent.operation_id).unwrap().unwrap().phase, JournalPhase::Staged);
    assert!(matches!(reopened.mark_applied(f.intent.operation_id),
        Err(JournalError::LocalDownloadNeedsNativeReceipt)));
    assert_eq!(f.protected.read_bound_preimage(f.intent.operation_id)
        .unwrap().unwrap().original.bytes, b"unchanged private original");
}

#[test]
fn reservation_spans_actual_filegate_history_and_s3_postflight() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let result = save_existing_note_with_checks_under_custody(&admission, &f.custody,
        f.keyspace.vault_id(), &f.scope, &f.intent.path, &f.new_text, &f.old_hash,
        Source::External, Some("reserved-local-fixture"),
        || { let held = prepare(&f, &admission)?; assert_editor_reserved(&f); Ok(held) },
        |held, written| {
            assert_editor_reserved(&f);
            held.inspect_after_write(&admission, &f.bindings, &f.journal.borrow(), &f.protected, written)
        });
    assert!(result.is_ok(), "joined reserved write failed: {result:?}");
    assert_eq!(fs::read_to_string(f.root.join(f.intent.path.as_str())).unwrap(), f.new_text);
    assert_reservation_released(&f);
    assert_no_remote_ack(&f);
    let original = f.protected.read_bound_preimage(f.intent.operation_id).unwrap().unwrap();
    let mut reopened = SqliteSyncJournal::open(&f.appdata.join("journal.sqlite")).unwrap();
    assert!(matches!(reopened.record_existing_replace_attempt_once(&f.intent, &original),
        Err(JournalError::ExistingReplaceAttemptAlreadyRecorded)));
}

#[test]
fn p0g3a_prewrite_runs_once_inside_actual_filegate_before_primary_history() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let preflight_count = Cell::new(0usize);
    let postflight_count = Cell::new(0usize);
    let result = save_existing_note_with_prewrite_under_custody(
        &admission, &f.custody, f.keyspace.vault_id(), &f.scope, &f.intent.path,
        &f.new_text, &f.old_hash, Source::External, Some("single-filegate-prewrite"),
        || prepare(&f, &admission),
        |held| {
            preflight_count.set(preflight_count.get() + 1);
            assert!(matches!(f.core.file_mutations.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)),
                "prepublication validation must run INSIDE the real File Gate mutex");
            assert_editor_reserved(&f);
            held.inspect_before_write(&admission, &f.bindings,
                &f.journal.borrow(), &f.protected)
        },
        |held, written| {
            postflight_count.set(postflight_count.get() + 1);
            assert_editor_reserved(&f);
            held.inspect_after_write(&admission, &f.bindings,
                &f.journal.borrow(), &f.protected, written)
        },
    );
    assert!(result.is_ok(), "one File Gate publication failed: {result:?}");
    assert_eq!(preflight_count.get(), 1);
    assert_eq!(postflight_count.get(), 1);
    assert_eq!(fs::read(f.root.join(f.intent.path.as_str())).unwrap(),
        f.new_text.as_bytes());
    assert_reservation_released(&f);
    assert_no_remote_ack(&f);
    let note = f.root.join(f.intent.path.as_str());
    assert!(f.core.history.page(&|| vec![f.root.clone()], &note, 0, 8).total > 0);
}

#[test]
fn p0g3a_same_bytes_foreign_inode_after_claim_refuses_before_filegate_write() {
    let f = fixture();
    let target = f.root.join(f.intent.path.as_str());
    let original_parked = f.root.join("retained-original-after-claim.md");
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let preflight_count = Cell::new(0usize);
    let postflight_called = Cell::new(false);

    let result = save_existing_note_with_prewrite_under_custody(
        &admission, &f.custody, f.keyspace.vault_id(), &f.scope, &f.intent.path,
        &f.new_text, &f.old_hash, Source::External, Some("must-not-write-foreign-file"),
        || {
            let held = prepare(&f, &admission)?;
            assert_editor_reserved(&f);
            // Deterministic same-user identity attack only in a disposable
            // test workspace, after the durable claim but before File Gate.
            fs::rename(&target, &original_parked).unwrap();
            fs::write(&target, b"unchanged private original").unwrap();
            Ok(held)
        },
        |held| {
            preflight_count.set(preflight_count.get() + 1);
            assert_editor_reserved(&f);
            held.inspect_before_write(&admission, &f.bindings,
                &f.journal.borrow(), &f.protected)
        },
        |_, _| { postflight_called.set(true); Ok(()) },
    );
    assert!(matches!(result, Err(GuardedNoteWriteError::NeedsReview(_))),
        "postclaim inode change must be HOLD, not a successful write: {result:?}");
    assert_eq!(preflight_count.get(), 1);
    assert!(!postflight_called.get(), "File Gate must not publish foreign inode");
    assert_eq!(fs::read(&target).unwrap(), b"unchanged private original");
    assert_eq!(fs::read(&original_parked).unwrap(), b"unchanged private original");
    assert!(f.journal.borrow().read_existing_replace_attempt(f.intent.operation_id)
        .unwrap().is_some(), "immutable V7 claim survives prewrite refusal");
    assert_reservation_released(&f);
    assert_no_remote_ack(&f);
}

#[test]
fn open_editor_refuses_before_claim_and_keeps_original() {
    let f = fixture();
    let target = f.root.join(f.intent.path.as_str());
    f.core.documents.open(&f.core, &target, 0).unwrap();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let attempted_postflight = Cell::new(false);
    let result = save_existing_note_with_checks_under_custody(&admission, &f.custody,
        f.keyspace.vault_id(), &f.scope, &f.intent.path, &f.new_text, &f.old_hash,
        Source::External, None, || prepare(&f, &admission), |_, _| {
            attempted_postflight.set(true); Ok(())
        });
    assert!(matches!(result, Err(GuardedNoteWriteError::NeedsReview(_))));
    assert!(!attempted_postflight.get());
    assert!(f.journal.borrow().read_existing_replace_attempt(f.intent.operation_id).unwrap().is_none());
    assert_eq!(fs::read(target).unwrap(), b"unchanged private original");
    assert_no_remote_ack(&f);
}

#[test]
fn same_bytes_foreign_inode_refuses_without_consuming_claim() {
    let f = fixture();
    let target = f.root.join(f.intent.path.as_str());
    fs::rename(&target, f.root.join("retained-original.md")).unwrap();
    fs::write(&target, b"unchanged private original").unwrap();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let outcome = f.custody.with_cooperative_writer_lifetime(&admission,
        f.keyspace.vault_id(), &f.scope, || prepare(&f, &admission)).unwrap();
    assert!(outcome.is_err());
    assert!(f.journal.borrow().read_existing_replace_attempt(f.intent.operation_id).unwrap().is_none());
    assert_eq!(fs::read(target).unwrap(), b"unchanged private original");
    assert_reservation_released(&f);
}

#[test]
fn wrong_proposed_content_and_foreign_operation_refuse_without_claim() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let mut foreign = f.intent.clone();
    foreign.profile_id = "not-the-registered-profile".into();
    f.custody.with_cooperative_writer_lifetime(&admission, f.keyspace.vault_id(), &f.scope, || {
        for (intent, text) in [(&f.intent, "different text"), (&foreign, f.new_text.as_str())] {
            assert!(prepare_reserved_s3_replace(&admission, &f.bindings, &f.profile,
                &f.keyspace, &mut f.journal.borrow_mut(), &f.cache, &f.protected,
                intent, text, &f.old_hash).is_err());
        }
    }).unwrap();
    assert!(f.journal.borrow().read_existing_replace_attempt(f.intent.operation_id).unwrap().is_none());
    assert_reservation_released(&f);
    assert_no_remote_ack(&f);
}

#[test]
fn history_failure_releases_editor_but_never_releases_durable_claim() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    f.core.history.fail_once(HistoryFault::CreateVersion);
    let postflight = Cell::new(false);
    let result = save_existing_note_with_checks_under_custody(&admission, &f.custody,
        f.keyspace.vault_id(), &f.scope, &f.intent.path, &f.new_text, &f.old_hash,
        Source::External, Some("fail-history"), || prepare(&f, &admission), |_, _| {
            postflight.set(true); Ok(())
        });
    assert!(matches!(result, Err(GuardedNoteWriteError::NeedsReview(_))));
    assert!(!postflight.get());
    assert!(f.journal.borrow().read_existing_replace_attempt(f.intent.operation_id).unwrap().is_some());
    assert_reservation_released(&f);
    assert_no_remote_ack(&f);
}

#[test]
fn postflight_tamper_requires_review_and_releases_only_editor_reservation() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let target = f.root.join(f.intent.path.as_str());
    let result = save_existing_note_with_checks_under_custody(&admission, &f.custody,
        f.keyspace.vault_id(), &f.scope, &f.intent.path, &f.new_text, &f.old_hash,
        Source::External, None, || prepare(&f, &admission), |held, written| {
            assert_editor_reserved(&f);
            // Non-cooperating write in THIS disposable fixture only.
            fs::write(&target, b"changed after primary save").unwrap();
            held.inspect_after_write(&admission, &f.bindings, &f.journal.borrow(), &f.protected, written)
        });
    assert!(matches!(result, Err(GuardedNoteWriteError::NeedsReview(_))));
    assert!(f.journal.borrow().read_existing_replace_attempt(f.intent.operation_id).unwrap().is_some());
    assert_eq!(fs::read(target).unwrap(), b"changed after primary save");
    assert_reservation_released(&f);
    assert_no_remote_ack(&f);
}


/// Consume one V7 attempt while the required cooperative OS custody lock
/// is held; deliberately do not enter a primary File Gate publisher.
fn claim_cold_attempt_without_publication(
    f: &Fixture,
    admission: &RegisteredReadAdmission<'_>,
) {
    f.custody.with_cooperative_writer_lifetime(
        admission, f.keyspace.vault_id(), &f.scope, || {
            let prepared = prepare(f, admission).unwrap();
            assert_editor_reserved(f);
            drop(prepared);
        },
    ).unwrap();
}

// P0g-3d-02 tests exercise a fresh, strictly read-only SQLite recovery-open.
// They intentionally do NOT assert crash durability or issue S3 apply authority.
fn inert_cold_review(f: &Fixture) -> Result<InertColdS3Review, FileCommandError> {
    let admission = RegisteredReadAdmission::open(&f.core, &f.root)
        .map_err(refused)?;
    let bindings = SqliteRegisteredS3Bindings::open_existing_read_only(
        &admission, &f.appdata.join("profile.sqlite"),
    ).map_err(refused)?;
    let journal = SqliteSyncJournal::open_existing_read_only(
        &f.appdata.join("journal.sqlite"),
    ).map_err(refused)?;
    let protected = SqliteProtectionStore::open_existing_read_only(
        &f.appdata.join("originals.sqlite"),
    ).map_err(refused)?;
    inspect_cold_s3_replace_hold(
        &admission, &bindings, &journal, &protected, f.intent.operation_id,
    )
}

#[test]
fn p0g3d02_cold_review_original_still_visible_remains_hold() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    claim_cold_attempt_without_publication(&f, &admission);
    drop(admission);
    let review = inert_cold_review(&f).unwrap();
    assert_eq!(review.operation_id, f.intent.operation_id);
    assert_eq!(review.file_state, InertColdS3FileState::OriginalStillVisible);
    assert_eq!(fs::read(f.root.join(f.intent.path.as_str())).unwrap(),
        b"unchanged private original");
    assert_no_remote_ack(&f);
}

#[test]
fn p0g3d02_cold_review_after_real_filegate_publication_remains_hold() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let written = save_existing_note_with_checks_under_custody(
        &admission, &f.custody, f.keyspace.vault_id(), &f.scope,
        &f.intent.path, &f.new_text, &f.old_hash,
        Source::External, Some("cold-review-test"),
        || prepare(&f, &admission),
        |prepared, result| prepared.inspect_after_write(
            &admission, &f.bindings, &f.journal.borrow(), &f.protected, result,
        ),
    );
    assert!(written.is_ok(), "fixture File Gate publication: {written:?}");
    drop(admission);
    let review = inert_cold_review(&f).unwrap();
    assert_eq!(review.file_state,
        InertColdS3FileState::CandidateVisibleWithoutReceipt);
    assert_no_remote_ack(&f);
}

#[test]
fn p0g3d02_cold_review_inplace_modification_is_needs_review_not_receipt() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    claim_cold_attempt_without_publication(&f, &admission);
    drop(admission);
    let path = f.root.join(f.intent.path.as_str());
    // Same inode / file ID, but neither protected original nor staged target:
    // cannot be mistaken for an authorized atomic replacement.
    fs::write(&path, b"untrusted direct local edit").unwrap();
    let review = inert_cold_review(&f).unwrap();
    assert_eq!(review.file_state, InertColdS3FileState::NeedsReview);
    assert_no_remote_ack(&f);
}

#[test]
fn p0g3d02_cold_review_missing_claim_and_corrupted_original_fail_closed() {
    let f = fixture();
    // A protected BLOB without a V7 claim is NOT a clean restart permit.
    assert!(inert_cold_review(&f).is_err());
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    claim_cold_attempt_without_publication(&f, &admission);
    drop(admission);
    assert_no_remote_ack(&f);
    let corrupted = rusqlite::Connection::open(
        f.appdata.join("originals.sqlite"),
    ).unwrap();
    corrupted.execute(
        "UPDATE bound_preimages SET source_file = 'invalid' WHERE operation_id = ?1",
        [f.intent.operation_id.to_string()],
    ).unwrap();
    drop(corrupted);
    assert!(inert_cold_review(&f).is_err());
    assert!(f.protected.read_bound_preimage(f.intent.operation_id).is_err(),
        "tampered protected identity must not be accepted after restart");
    // Do NOT call assert_no_remote_ack here: it intentionally assumes that
    // protected BLOB provenance is still sound. Instead independently inspect
    // the untouched journal and primary after corrupting that other database.
    let mut journal = SqliteSyncJournal::open_existing_read_only(
        &f.appdata.join("journal.sqlite"),
    ).unwrap();
    let baseline = journal.baseline(f.intent.vault_id, f.intent.document_id)
        .unwrap().unwrap();
    assert_eq!(Some(baseline.revision_id), f.intent.expected_baseline);
    assert_eq!(baseline.content_hash, f.old_hash);
    assert_eq!(journal.get(f.intent.operation_id).unwrap().unwrap().phase,
        JournalPhase::Staged);
    assert!(journal.mark_applied(f.intent.operation_id).is_err(),
        "read-only inspection must never turn an invalid preimage into apply");
    assert_eq!(fs::read(f.root.join(f.intent.path.as_str())).unwrap(),
        b"unchanged private original");
}


#[test]
fn p0g3d02_cold_review_same_bytes_foreign_inode_is_needs_review() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    claim_cold_attempt_without_publication(&f, &admission);
    drop(admission);
    let target = f.root.join(f.intent.path.as_str());
    let parked = f.root.join("physically-retained-original.md");
    fs::rename(&target, &parked).unwrap();
    fs::write(&target, b"unchanged private original").unwrap();
    let review = inert_cold_review(&f).unwrap();
    assert_eq!(review.file_state, InertColdS3FileState::NeedsReview,
        "a new file with identical bytes cannot inherit original identity");
    assert_eq!(fs::read(&parked).unwrap(), b"unchanged private original");
    assert_no_remote_ack(&f);
}

#[test]
fn p0g3d02_cold_review_changed_baseline_refuses_without_mutating_file() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    claim_cold_attempt_without_publication(&f, &admission);
    drop(admission);
    // A changed durable baseline is disqualifying even if the primary bytes
    // and the independent protected-original BLOB are still identical.
    let connection = rusqlite::Connection::open(
        f.appdata.join("journal.sqlite"),
    ).unwrap();
    connection.execute(
        "UPDATE sync_baselines SET revision_id = ?1 WHERE vault_id = ?2 AND document_id = ?3",
        rusqlite::params![
            RevisionId::new().to_string(),
            f.intent.vault_id.to_string(),
            f.intent.document_id.to_string(),
        ],
    ).unwrap();
    drop(connection);
    assert!(inert_cold_review(&f).is_err(),
        "stale baseline must reject cold forensic binding");
    assert_eq!(fs::read(f.root.join(f.intent.path.as_str())).unwrap(),
        b"unchanged private original");
    let journal = SqliteSyncJournal::open_existing_read_only(
        &f.appdata.join("journal.sqlite"),
    ).unwrap();
    assert_eq!(journal.get(f.intent.operation_id).unwrap().unwrap().phase,
        JournalPhase::Staged);
}


#[test]
fn p0g3d02_separate_core_after_all_connections_close_is_forensic_only() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    claim_cold_attempt_without_publication(&f, &admission);
    drop(admission);

    // Release the old Core and ALL write-capable SQLite connections, retain
    // only paths and immutable operation data for the replacement process.
    let Fixture {
        _temp: _hold_temp_dir, core, root, appdata,
        bindings, custody, journal, cache, protected, intent, old_hash, ..
    } = f;
    drop(bindings);
    drop(custody);
    drop(journal);
    drop(cache);
    drop(protected);
    drop(core);

    let restarted = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    let admission = RegisteredReadAdmission::open(&restarted, &root).unwrap();
    let profiles = SqliteRegisteredS3Bindings::open_existing_read_only(
        &admission, &appdata.join("profile.sqlite"),
    ).unwrap();
    let journal = SqliteSyncJournal::open_existing_read_only(
        &appdata.join("journal.sqlite"),
    ).unwrap();
    let originals = SqliteProtectionStore::open_existing_read_only(
        &appdata.join("originals.sqlite"),
    ).unwrap();
    let review = inspect_cold_s3_replace_hold(
        &admission, &profiles, &journal, &originals, intent.operation_id,
    ).unwrap();
    assert_eq!(review.file_state, InertColdS3FileState::OriginalStillVisible);
    assert_eq!(originals.read_bound_preimage(intent.operation_id)
        .unwrap().unwrap().original.content_hash, old_hash);
    assert_eq!(journal.get(intent.operation_id).unwrap().unwrap().phase,
        JournalPhase::Staged);
    let baseline = journal.baseline(intent.vault_id, intent.document_id)
        .unwrap().unwrap();
    assert_eq!(baseline.content_hash, old_hash);
    assert_eq!(Some(baseline.revision_id), intent.expected_baseline);
    assert_eq!(fs::read(root.join(intent.path.as_str())).unwrap(),
        b"unchanged private original");
    // A new Core, and matching original bytes, grant no remote ACK and no
    // permission to replay an already consumed V7 existing-replace claim.
}
