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
    save_existing_note_with_prewrite_under_custody,
    save_existing_note_with_prewrite_strict_history_under_custody,
    GuardedNoteWriteError};
use crate::sync::journal::InertExistingReplaceReceiptCandidate;
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
    fixture_with_temp(tempfile::tempdir().unwrap())
}

/// Reuse the ordinary integration fixture inside an OS child whose enclosing
/// disposable directory is retained by the parent after std::process::abort.
fn fixture_with_temp(temp: tempfile::TempDir) -> Fixture {
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

/// Strict HistoryService is created under the real File Gate/custody callback.
/// V8 is then written once to the same SQLite journal as V7; none of these
/// observations can authorize a StageDownload apply or remote ACK.
fn publish_strict_candidate_and_record_v8(f: &Fixture) {
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let outcome = save_existing_note_with_prewrite_strict_history_under_custody(
        &admission, &f.custody, f.keyspace.vault_id(), &f.scope,
        &f.intent.path, &f.new_text, &f.old_hash,
        Source::External, Some("inert-v8-cold-fixture"),
        || prepare(f, &admission),
        |prepared| prepared.inspect_before_write(
            &admission, &f.bindings, &f.journal.borrow(), &f.protected,
        ),
        |prepared, primary, history| {
            prepared.inspect_after_write(
                &admission, &f.bindings, &f.journal.borrow(),
                &f.protected, primary,
            )?;
            let claim=f.journal.borrow()
                .read_existing_replace_attempt(f.intent.operation_id)
                .map_err(refused)?
                .ok_or_else(|| refused("V7 claim absent after strict File Gate"))?;
            let original=f.protected.read_bound_preimage(f.intent.operation_id)
                .map_err(refused)?
                .ok_or_else(|| refused("protected original absent after strict File Gate"))?;
            let pinned=PinnedReadOnlyRoot::open(&f.root).map_err(refused)?;
            let observed=pinned.read_identified_bounded(
                &f.intent.path, MAX_PROTECTED_ORIGINAL_BYTES as u64,
            ).map_err(refused)?;
            let candidate=InertExistingReplaceReceiptCandidate::from_strict_history(
                &claim, &original, history, &observed,
            ).map_err(refused)?;
            f.journal.borrow_mut()
                .record_existing_replace_receipt_candidate_once(&candidate)
                .map_err(refused)?;
            Ok(())
        },
    );
    assert!(outcome.is_ok(), "strict V8 fixture did not publish: {outcome:?}");
    drop(admission);
    assert_no_remote_ack(f);
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
    assert_eq!(review.v8_state, InertColdV8State::Missing);
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
    assert_eq!(review.v8_state, InertColdV8State::Missing);
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
    assert_eq!(review.v8_state, InertColdV8State::Missing);
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


#[test]
fn p0g3d06_strict_v8_candidate_cold_join_is_still_hold() {
    let f=fixture();
    publish_strict_candidate_and_record_v8(&f);
    let review=inert_cold_review(&f).unwrap();
    assert_eq!(review.operation_id,f.intent.operation_id);
    assert_eq!(review.file_state,
        InertColdS3FileState::CandidateVisibleWithoutReceipt);
    assert_eq!(review.v8_state,InertColdV8State::LocalCorrelationOnly);
    assert_eq!(fs::read(f.root.join(f.intent.path.as_str())).unwrap(),
        f.new_text.as_bytes());
    assert_no_remote_ack(&f);
}

#[test]
fn p0g3d06_reopened_v8_refuses_changed_inode_and_tampered_sqlite_digest() {
    let f=fixture();
    publish_strict_candidate_and_record_v8(&f);
    let target=f.root.join(f.intent.path.as_str());
    let parked=f.root.join("preserved-published-candidate.md");
    fs::rename(&target,&parked).unwrap();
    fs::write(&target,f.new_text.as_bytes()).unwrap();
    assert!(inert_cold_review(&f).is_err(),
        "same bytes at another inode cannot inherit a saved V8 receipt");
    assert_eq!(fs::read(&parked).unwrap(),f.new_text.as_bytes());
    assert_no_remote_ack(&f);
    // After restoring the correct physical identity, any tampered candidate
    // digest must reject, rather than making a missing V8 look like a clean
    // V7-only attempt or silently replaying the consumed one-shot fence.
    fs::remove_file(&target).unwrap();
    fs::rename(&parked,&target).unwrap();
    let database=f.appdata.join("journal.sqlite");
    let connection=rusqlite::Connection::open(&database).unwrap();
    connection.execute(
        "UPDATE sync_existing_replace_receipt_candidates
            SET candidate_digest='tampered' WHERE operation_id=?1",
        [f.intent.operation_id.to_string()],
    ).unwrap();
    drop(connection);
    assert!(inert_cold_review(&f).is_err(),
        "corrupt V8 digest must fail closed");
    assert_no_remote_ack(&f);
}


/// Diagnostic-only combined check against three independent read-only DBs.
fn inert_cold_full_history_review(
    f: &Fixture,
) -> Result<InertColdUnifiedReview, FileCommandError> {
    let admission = RegisteredReadAdmission::open(&f.core, &f.root)
        .map_err(refused)?;
    let bindings = SqliteRegisteredS3Bindings::open_existing_read_only(
        &admission, &f.appdata.join("profile.sqlite"),
    ).map_err(refused)?;
    let journal = SqliteSyncJournal::open_existing_read_only(
        &f.appdata.join("journal.sqlite"),
    ).map_err(refused)?;
    let originals = SqliteProtectionStore::open_existing_read_only(
        &f.appdata.join("originals.sqlite"),
    ).map_err(refused)?;
    inspect_cold_s3_replace_with_history_hold(
        &admission, &bindings, &journal, &originals, f.intent.operation_id,
    )
}

#[test]
fn p0g3d07_fresh_core_all_stores_and_history_remain_hold() {
    let f = fixture();
    publish_strict_candidate_and_record_v8(&f);
    let first = inert_cold_full_history_review(&f).unwrap();
    assert_eq!(first.status, InertColdUnifiedStatus::LocallyConsistentHold);
    assert_eq!(first.operation_id, f.intent.operation_id);
    assert_no_remote_ack(&f);

    // Destroy old Core + all write-capable SQLite handles. A new Core cannot
    // rely on previous HistoryService state, a writer permit or old admission.
    let Fixture {
        _temp: _keep_temp_dir,
        core, root, appdata, bindings, custody, journal,
        cache, protected, intent, old_hash, new_text, ..
    } = f;
    drop(bindings);
    drop(custody);
    drop(journal);
    drop(cache);
    drop(protected);
    drop(core);
    let fresh = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    let admission = RegisteredReadAdmission::open(&fresh, &root).unwrap();
    let bindings = SqliteRegisteredS3Bindings::open_existing_read_only(
        &admission, &appdata.join("profile.sqlite"),
    ).unwrap();
    let journal = SqliteSyncJournal::open_existing_read_only(
        &appdata.join("journal.sqlite"),
    ).unwrap();
    let originals = SqliteProtectionStore::open_existing_read_only(
        &appdata.join("originals.sqlite"),
    ).unwrap();

    let result = inspect_cold_s3_replace_with_history_hold(
        &admission, &bindings, &journal, &originals, intent.operation_id,
    ).unwrap();
    assert_eq!(result.status, InertColdUnifiedStatus::LocallyConsistentHold);
    assert_eq!(result.operation_id, intent.operation_id);
    assert_eq!(fs::read(root.join(intent.path.as_str())).unwrap(),
        new_text.as_bytes());
    assert_eq!(originals.read_bound_preimage(intent.operation_id)
        .unwrap().unwrap().original.content_hash, old_hash);
    assert_eq!(journal.get(intent.operation_id).unwrap().unwrap().phase,
        JournalPhase::Staged);
    let baseline = journal.baseline(intent.vault_id, intent.document_id)
        .unwrap().unwrap();
    assert_eq!(Some(baseline.revision_id), intent.expected_baseline);
    assert_eq!(baseline.content_hash, old_hash);
    let mut writable_check = SqliteSyncJournal::open(
        &appdata.join("journal.sqlite"),
    ).unwrap();
    assert!(matches!(writable_check.mark_applied(intent.operation_id),
        Err(JournalError::LocalDownloadNeedsNativeReceipt)),
        "forensic cold agreement must never grant StageDownload apply");
}

#[test]
fn p0g3d07_missing_v8_or_visible_original_cannot_produce_unified_hold() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    claim_cold_attempt_without_publication(&f, &admission);
    drop(admission);
    assert!(inert_cold_full_history_review(&f).is_err());
    assert_eq!(fs::read(f.root.join(f.intent.path.as_str())).unwrap(),
        b"unchanged private original");
    assert_no_remote_ack(&f);
}

#[test]
fn p0g3d07_same_length_history_tampering_fails_closed() {
    let f = fixture();
    publish_strict_candidate_and_record_v8(&f);
    assert!(inert_cold_full_history_review(&f).is_ok());
    let candidate = f.journal.borrow()
        .read_existing_replace_receipt_candidate(f.intent.operation_id)
        .unwrap().unwrap();
    let (id, _, _) = candidate.cold_history_fields();
    let history = f.root.join(".aquilum").join("history")
        .join(f.intent.path.as_str()).join(id);
    assert!(history.is_file(), "strict external version file must be present");
    fs::write(&history, "X".repeat(f.new_text.len())).unwrap();
    assert!(inert_cold_full_history_review(&f).is_err(),
        "matching V8 and primary bytes cannot conceal a changed history file");
    assert_eq!(fs::read(f.root.join(f.intent.path.as_str())).unwrap(),
        f.new_text.as_bytes(), "cold inspector must not rewrite the primary");
    assert_no_remote_ack(&f);
}

#[test]
fn p0g3d07_foreign_note_history_relocation_fails_closed() {
    let f = fixture();
    publish_strict_candidate_and_record_v8(&f);
    let candidate = f.journal.borrow()
        .read_existing_replace_receipt_candidate(f.intent.operation_id)
        .unwrap().unwrap();
    let (id, _, _) = candidate.cold_history_fields();
    let original = f.root.join(".aquilum").join("history")
        .join(f.intent.path.as_str()).join(id);
    let foreign = f.root.join(".aquilum").join("history")
        .join("Unrelated.md");
    fs::create_dir_all(&foreign).unwrap();
    fs::rename(&original, foreign.join(id)).unwrap();
    assert!(inert_cold_full_history_review(&f).is_err(),
        "version ID at a different note cannot replace exact registered history");
    assert_no_remote_ack(&f);
}


/// The child process is the very same Rust test executable, invoked with an
/// exact libtest filter. Unlike another Core in the same process, it has no
/// inherited HistoryService mutex, Core, SQLite connections or admission.
/// Never use this as a power-loss-durability test.
const P0G3D08_CHILD: &str =
    "sync::s3_reserved_replace::tests::p0g3d08_os_process_probe_entrypoint";

#[test]
fn p0g3d08_os_process_probe_entrypoint() {
    let Some(mode) = std::env::var_os("AKULA_P0G3D08_CHILD_MODE") else {
        return;
    };
    let mode = mode.to_string_lossy();
    let root = PathBuf::from(std::env::var_os("AKULA_P0G3D08_ROOT").unwrap());
    let appdata = PathBuf::from(std::env::var_os("AKULA_P0G3D08_APPDATA").unwrap());
    let operation: OperationId = std::env::var("AKULA_P0G3D08_OPERATION")
        .unwrap().parse().unwrap();

    let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
    let bindings = SqliteRegisteredS3Bindings::open_existing_read_only(
        &admission, &appdata.join("profile.sqlite"),
    ).unwrap();
    let journal = SqliteSyncJournal::open_existing_read_only(
        &appdata.join("journal.sqlite"),
    ).unwrap();
    let originals = SqliteProtectionStore::open_existing_read_only(
        &appdata.join("originals.sqlite"),
    ).unwrap();

    // No child writes are permitted, even on the successful HOLD path.
    match mode.as_ref() {
        "hold" => {
            let result = inspect_cold_s3_replace_with_history_hold(
                &admission, &bindings, &journal, &originals, operation,
            );
            let review = result.expect("cross-process cold evidence must match");
            assert_eq!(review.status, InertColdUnifiedStatus::LocallyConsistentHold);
            assert_eq!(review.operation_id, operation);
        }
        "deny" => {
            let result = inspect_cold_s3_replace_with_history_hold(
                &admission, &bindings, &journal, &originals, operation,
            );
            assert!(result.is_err(),
                "damaged or incomplete cross-process evidence must fail closed");
        }
        "vm-classify" => {
            let result = inspect_cold_s3_replace_with_history_hold(
                &admission, &bindings, &journal, &originals, operation,
            );
            // A real VM force-off may preserve all domains or only a subset.
            // HOLD is diagnostic; a partial image MUST be denied, never applied.
            match result {
                Ok(review) => {
                    assert_eq!(review.status, InertColdUnifiedStatus::LocallyConsistentHold);
                    assert_eq!(review.operation_id, operation);
                    println!("AKULA_P0G3D08D_VM_COLD_CLASS=HOLD");
                }
                Err(error) => {
                    println!("AKULA_P0G3D08D_VM_COLD_CLASS=DENY ({error:?})");
                }
            }
            assert!(journal.read_existing_replace_attempt(operation)
                .unwrap().is_some(), "post-V8 VM force-off lost consumed V7 fence");
        }
        "oracle-hold" | "oracle-deny" => {
            use crate::sync::cold_oracle_report::{ColdOracleReport, ColdOracleVerdict};
            let inspected = inspect_cold_s3_replace_with_history_classified(
                &admission, &bindings, &journal, &originals, operation,
            );
            let report = ColdOracleReport::from_classified_inspector(operation, &inspected);
            let expected = if mode == "oracle-hold" {
                ColdOracleVerdict::LocallyConsistentHold
            } else {
                ColdOracleVerdict::Deny
            };
            assert_eq!(report.verdict, expected,
                "the native cold inspector must select the expected fail-closed result");
            // Out-of-band diagnostics only; report contains no paths, bytes
            // or capability to authorize an S3 apply, ACK or baseline advance.
            println!("AKULA_G1_COLD_ORACLE={}", report.to_json());
        }
        other => panic!("unknown P0g-3d-08 child mode: {other}"),
    }

    let recorded = journal.get(operation).unwrap().unwrap();
    assert_eq!(recorded.phase, JournalPhase::Staged);
    let baseline = journal.baseline(
        recorded.intent.vault_id, recorded.intent.document_id,
    ).unwrap().unwrap();
    assert_eq!(Some(baseline.revision_id), recorded.intent.expected_baseline);
    assert_eq!(baseline.content_hash,
        blake3::hash(b"unchanged private original").to_hex().to_string());
    assert_eq!(originals.read_bound_preimage(operation)
        .unwrap().unwrap().original.bytes, b"unchanged private original");
}

// Deliberately isolate environment variables to an OS child. The parent
// never alters global process env (safe with parallel Rust test threads).
fn p0g3d08_run_child(
    root: &std::path::Path,
    appdata: &std::path::Path,
    operation: OperationId,
    mode: &str,
) {
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg(P0G3D08_CHILD).arg("--nocapture")
        .env("AKULA_P0G3D08_CHILD_MODE", mode)
        .env("AKULA_P0G3D08_ROOT", root)
        .env("AKULA_P0G3D08_APPDATA", appdata)
        .env("AKULA_P0G3D08_OPERATION", operation.to_string())
        .output().unwrap();
    assert!(result.status.success(),
        "real OS child rejected expected {mode}: stdout={}, stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr));
    assert!(String::from_utf8_lossy(&result.stdout)
        .contains("p0g3d08_os_process_probe_entrypoint ... ok"),
        "child must actually run the named Rust test, not pass an empty filter: {}",
        String::from_utf8_lossy(&result.stdout));
}

/// G1: reuse the existing native cold inspector in a *different* OS process
/// and require exactly one schema-versioned, non-authorizing JSON verdict.
fn p0g3d09_run_oracle_child(
    root: &std::path::Path,
    appdata: &std::path::Path,
    operation: OperationId,
    expected_verdict: &str,
    expected_reason: &str,
) -> serde_json::Value {
    assert!(matches!(expected_verdict, "LOCALLY_CONSISTENT_HOLD" | "DENY"));
    let mode = if expected_verdict == "LOCALLY_CONSISTENT_HOLD" {
        "oracle-hold"
    } else {
        "oracle-deny"
    };
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg(P0G3D08_CHILD).arg("--nocapture")
        .env("AKULA_P0G3D08_CHILD_MODE", mode)
        .env("AKULA_P0G3D08_ROOT", root)
        .env("AKULA_P0G3D08_APPDATA", appdata)
        .env("AKULA_P0G3D08_OPERATION", operation.to_string())
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(),
        "the independent native verifier failed: stdout={stdout}, stderr={stderr}");
    // The report is inserted into the libtest output by --nocapture; the
    // test name and the final result are separate anchors, not adjacent text.
    assert!(stdout.contains(&format!("test {P0G3D08_CHILD} ...")),
        "libtest did not execute the exact named cold verifier: {stdout}");
    assert!(stdout.contains("test result: ok. 1 passed; 0 failed;"),
        "libtest result was missing or not successful: {stdout}");
    let json_line = stdout.lines().find_map(|line| {
        line.split_once("AKULA_G1_COLD_ORACLE=").map(|(_, json)| json)
    }).expect("independent OS process did not emit a versioned oracle report");
    let report: serde_json::Value = serde_json::from_str(json_line)
        .expect("native oracle report must be parseable as a single JSON object");
    assert_eq!(report["schema"], "akula.sync.cold-oracle.v1");
    assert_eq!(report["operation_id"], operation.to_string());
    assert_eq!(report["verdict"], expected_verdict);
    assert_eq!(report["reason_code"], expected_reason);
    assert_eq!(report["authority"], "none");
    for key in ["receipt_issued", "automatic_apply_allowed",
                "remote_ack_allowed", "baseline_promotion_allowed"] {
        assert_eq!(report[key], false, "{key} must remain denied");
    }
    report
}

#[test]
fn p0g3d09_oracle_reports_hold_and_history_tamper_deny_across_processes() {
    let f = fixture();
    publish_strict_candidate_and_record_v8(&f);
    assert_no_remote_ack(&f);
    let version = f.journal.borrow()
        .read_existing_replace_receipt_candidate(f.intent.operation_id)
        .unwrap().unwrap();
    let history = f.root.join(".aquilum").join("history")
        .join(f.intent.path.as_str())
        .join(version.cold_history_fields().0);
    assert!(history.is_file());
    let Fixture {
        _temp: keep_fixture, core, root, appdata, bindings, custody,
        journal, cache, protected, intent, old_hash, new_text, ..
    } = f;
    drop(bindings);
    drop(custody);
    drop(journal);
    drop(cache);
    drop(protected);
    drop(core);

    let positive = p0g3d09_run_oracle_child(
        &root, &appdata, intent.operation_id,
        "LOCALLY_CONSISTENT_HOLD", "CORRELATED_NOT_DURABLE",
    );
    assert_eq!(positive["reason_code"], "CORRELATED_NOT_DURABLE");

    fs::write(&history, vec![b'X'; new_text.len()]).unwrap();
    let negative = p0g3d09_run_oracle_child(
        &root, &appdata, intent.operation_id,
        "DENY", "IMMUTABLE_HISTORY_REJECTED",
    );
    assert_eq!(negative["reason_code"], "IMMUTABLE_HISTORY_REJECTED");

    // The report is not a proof of persistence, or a permission to ACK.
    let mut cold = SqliteSyncJournal::open(&appdata.join("journal.sqlite")).unwrap();
    assert_eq!(cold.get(intent.operation_id).unwrap().unwrap().phase,
        JournalPhase::Staged);
    let baseline = cold.baseline(intent.vault_id, intent.document_id)
        .unwrap().unwrap();
    assert_eq!(Some(baseline.revision_id), intent.expected_baseline);
    assert_eq!(baseline.content_hash, old_hash);
    assert!(cold.read_existing_replace_attempt(intent.operation_id)
        .unwrap().is_some(), "the V7 one-shot fence must survive");
    assert!(matches!(cold.mark_applied(intent.operation_id),
        Err(JournalError::LocalDownloadNeedsNativeReceipt)));
    drop(cold);
    let originals = SqliteProtectionStore::open_existing_read_only(
        &appdata.join("originals.sqlite"),
    ).unwrap();
    assert_eq!(originals.read_bound_preimage(intent.operation_id)
        .unwrap().unwrap().original.bytes, b"unchanged private original");
    drop(originals);
    drop(keep_fixture);
}

#[test]
fn p0g3d08_real_os_child_reopens_full_v8_and_refuses_history_inode_corruption() {
    let f = fixture();
    publish_strict_candidate_and_record_v8(&f);
    let version = f.journal.borrow()
        .read_existing_replace_receipt_candidate(f.intent.operation_id)
        .unwrap().unwrap();
    let (version_id, _, _) = version.cold_history_fields();
    let historical = f.root.join(".aquilum").join("history")
        .join(f.intent.path.as_str()).join(version_id);
    assert!(historical.is_file());

    let Fixture {
        _temp: keep_fixture, core, root, appdata, bindings, custody,
        journal, cache, protected, intent, old_hash, new_text, ..
    } = f;
    drop(bindings);
    drop(custody);
    drop(journal);
    drop(cache);
    drop(protected);
    drop(core);

    p0g3d08_run_child(&root, &appdata, intent.operation_id, "hold");

    // Destroy the exact same-length external version in the original history
    // path, without changing the primary or either SQLite baseline.
    fs::write(&historical, vec![b'X'; new_text.len()]).unwrap();
    p0g3d08_run_child(&root, &appdata, intent.operation_id, "deny");

    // Restoring the actual history bytes allows a NEW independent child to
    // observe a forensic HOLD again; no process may reuse an earlier witness.
    fs::write(&historical, new_text.as_bytes()).unwrap();
    p0g3d08_run_child(&root, &appdata, intent.operation_id, "hold");

    // Identical content does not excuse an independently replaced physical
    // file identity, even though the user-visible note bytes are unchanged.
    let primary = root.join(intent.path.as_str());
    let parked = root.join("p0g3d08-retained-published-inode.md");
    fs::rename(&primary, &parked).unwrap();
    fs::write(&primary, new_text.as_bytes()).unwrap();
    p0g3d08_run_child(&root, &appdata, intent.operation_id, "deny");
    fs::remove_file(&primary).unwrap();
    fs::rename(&parked, &primary).unwrap();
    p0g3d08_run_child(&root, &appdata, intent.operation_id, "hold");

    // A tampered V8 digest must never be interpreted as simply absent.
    let connection = rusqlite::Connection::open(appdata.join("journal.sqlite"))
        .unwrap();
    connection.execute(
        "UPDATE sync_existing_replace_receipt_candidates
            SET candidate_digest='not-a-valid-digest' WHERE operation_id=?1",
        [intent.operation_id.to_string()],
    ).unwrap();
    drop(connection);
    p0g3d08_run_child(&root, &appdata, intent.operation_id, "deny");

    let cold = SqliteSyncJournal::open_existing_read_only(
        &appdata.join("journal.sqlite"),
    ).unwrap();
    assert_eq!(cold.get(intent.operation_id).unwrap().unwrap().phase,
        JournalPhase::Staged);
    let baseline = cold.baseline(intent.vault_id, intent.document_id)
        .unwrap().unwrap();
    assert_eq!(baseline.content_hash, old_hash);
    assert_eq!(Some(baseline.revision_id), intent.expected_baseline);
    assert_eq!(fs::read(&primary).unwrap(), new_text.as_bytes());
    let originals = SqliteProtectionStore::open_existing_read_only(
        &appdata.join("originals.sqlite"),
    ).unwrap();
    assert_eq!(originals.read_bound_preimage(intent.operation_id)
        .unwrap().unwrap().original.bytes, b"unchanged private original");
    drop(keep_fixture);
}

#[test]
fn p0g3d08_real_os_child_rejects_v7_claim_without_v8_or_publication() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    claim_cold_attempt_without_publication(&f, &admission);
    drop(admission);
    assert_no_remote_ack(&f);
    let Fixture {
        _temp: keep_fixture, core, root, appdata, bindings, custody,
        journal, cache, protected, intent, ..
    } = f;
    drop(bindings);
    drop(custody);
    drop(journal);
    drop(cache);
    drop(protected);
    drop(core);
    p0g3d08_run_child(&root, &appdata, intent.operation_id, "deny");
    let early = p0g3d09_run_oracle_child(
        &root, &appdata, intent.operation_id,
        "DENY", "PRIMARY_NOT_PUBLISHED",
    );
    assert_eq!(early["authority"], "none");
    assert_eq!(fs::read(root.join(intent.path.as_str())).unwrap(),
        b"unchanged private original");
    drop(keep_fixture);
}


#[test]
fn p0g3d08_real_os_child_rejects_published_primary_and_history_without_v8() {
    let f = fixture();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let result = save_existing_note_with_prewrite_strict_history_under_custody(
        &admission, &f.custody, f.keyspace.vault_id(), &f.scope,
        &f.intent.path, &f.new_text, &f.old_hash,
        Source::External, Some("p0g3d08-stop-before-v8"),
        || prepare(&f, &admission),
        |held| held.inspect_before_write(
            &admission, &f.bindings, &f.journal.borrow(), &f.protected,
        ),
        |held, primary, _history| held.inspect_after_write(
            &admission, &f.bindings, &f.journal.borrow(), &f.protected, primary,
        ),
    );
    assert!(result.is_ok(), "File Gate and history setup failed: {result:?}");
    assert!(f.journal.borrow()
        .read_existing_replace_receipt_candidate(f.intent.operation_id)
        .unwrap().is_none());
    assert_no_remote_ack(&f);
    drop(admission);

    // A real process boundary after the primary+history publication but before
    // the V8 insert must NEVER infer that a complete receipt exists.
    let Fixture {
        _temp: keep_fixture, core, root, appdata, bindings, custody,
        journal, cache, protected, intent, new_text, ..
    } = f;
    drop(bindings);
    drop(custody);
    drop(journal);
    drop(cache);
    drop(protected);
    drop(core);
    p0g3d08_run_child(&root, &appdata, intent.operation_id, "deny");
    let incomplete = p0g3d09_run_oracle_child(
        &root, &appdata, intent.operation_id,
        "DENY", "V8_MISSING",
    );
    assert_eq!(incomplete["automatic_apply_allowed"], false);
    assert_eq!(fs::read(root.join(intent.path.as_str())).unwrap(),
        new_text.as_bytes());
    drop(keep_fixture);
}


/// P0g-3d-08c: an actual writer aborts at a precise stage in a disposable
/// fixture. The parent has never held this child's Core or SQLite handles.
/// This is still NOT a physical power-cut durability test.
const P0G3D08C_WRITER_CHILD: &str =
    "sync::s3_reserved_replace::tests::p0g3d08c_writer_abort_entrypoint";

fn p0g3d08c_abort_at(workdir: &std::path::Path, stage: &str) -> ! {
    if std::env::var("AKULA_P0G3D08D_VM_PAUSE").as_deref() == Ok("1") {
        // Only the final V8 seam is authorized for this first Linux VM lab.
        // The host receives this marker over live SSH stdout and force-kills
        // QEMU itself. A guest-side abort or shutdown is NOT the experiment.
        assert!(cfg!(target_os = "linux") && stage == "after-v8",
            "VM force-off experiment is restricted to Linux after-v8");
        println!("AKULA_P0G3D08D_VM_READY=after-v8");
        std::io::Write::flush(&mut std::io::stdout())
            .expect("flush out-of-band host witness before VM kill");
        loop {
            std::thread::park();
        }
    }
    // Ordinary #444 process-crash tests retain their disposable disk marker.
    // The VM mode above must perform ZERO guest-disk writes after V8 before
    // the external QEMU kill; only the host owns the live marker.
    fs::write(workdir.join("p0g3d08c-crash-marker"), stage)
        .expect("write independent test-only crash marker");
    std::process::abort();
}

#[test]
fn p0g3d08c_writer_abort_entrypoint() {
    let Some(stage) = std::env::var_os("AKULA_P0G3D08C_STAGE") else {
        return;
    };
    let stage = stage.to_string_lossy();
    let workdir = PathBuf::from(
        std::env::var_os("AKULA_P0G3D08C_WORKDIR").unwrap(),
    );
    assert!(matches!(stage.as_ref(),
        "after-v7" | "after-primary" | "after-history" | "after-v8"));
    let temp = tempfile::Builder::new()
        .prefix("writer-crash-fixture-")
        .tempdir_in(&workdir).unwrap();
    let f = fixture_with_temp(temp);
    // Serialized before any crash seam; the independent parent can locate
    // the existing vault, databases and immutable one-shot operation.
    let identity = serde_json::json!({
        "root": f.root.to_str().unwrap(),
        "appdata": f.appdata.to_str().unwrap(),
        "operation": f.intent.operation_id.to_string(),
    });
    fs::write(workdir.join("p0g3d08c-fixture.json"),
        serde_json::to_vec(&identity).unwrap()).unwrap();
    if std::env::var("AKULA_P0G3D08D_VM_PAUSE").as_deref() == Ok("1") {
        assert!(cfg!(target_os = "linux") && stage == "after-v8",
            "VM force-off test must not pause a different platform or seam");
        // The fixture (original, previous baseline, SQLite DB namespaces,
        // registration) is setup, not part of the intentionally ambiguous
        // destructive step. Flush setup before the V7 / publication window.
        let status = std::process::Command::new("sync").status()
            .expect("Linux VM fixture preflight sync");
        assert!(status.success(), "Linux VM fixture preflight sync failed");
        // Unlike fixture.json on the tested guest disk, this goes over live
        // SSH stdout to the host BEFORE any deliberate QEMU force-kill.
        println!("AKULA_P0G3D08D_VM_FIXTURE={identity}");
        std::io::Write::flush(&mut std::io::stdout())
            .expect("flush out-of-band fixture identity to host");
    }

    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    if stage == "after-v7" {
        let _ignored = f.custody.with_cooperative_writer_lifetime(
            &admission, f.keyspace.vault_id(), &f.scope,
            || -> Result<(), FileCommandError> {
                let _held = prepare(&f, &admission)?;
                assert_editor_reserved(&f);
                assert!(f.journal.borrow()
                    .read_existing_replace_attempt(f.intent.operation_id)
                    .map_err(refused)?
                    .is_some());
                // Retain editor reservation and OS custody until process abort.
                p0g3d08c_abort_at(&workdir, "after-v7");
            },
        );
        panic!("writer failed to abort immediately after V7 claim");
    }

    // The existing File Gate is still the ONLY primary publisher.
    // The after-primary seam is cfg(test) only and guarded by the disposable
    // workdir plus exact target note path in write_checked_history_under_gate.
    let outcome = save_existing_note_with_prewrite_strict_history_under_custody(
        &admission, &f.custody, f.keyspace.vault_id(), &f.scope,
        &f.intent.path, &f.new_text, &f.old_hash,
        Source::External, Some("p0g3d08c-abort-writer"),
        || prepare(&f, &admission),
        |prepared| prepared.inspect_before_write(
            &admission, &f.bindings, &f.journal.borrow(), &f.protected,
        ),
        |prepared, primary, history| {
            prepared.inspect_after_write(
                &admission, &f.bindings, &f.journal.borrow(),
                &f.protected, primary,
            )?;
            if stage == "after-history" {
                p0g3d08c_abort_at(&workdir, "after-history");
            }
            let claim = f.journal.borrow()
                .read_existing_replace_attempt(f.intent.operation_id)
                .map_err(refused)?
                .ok_or_else(|| refused("V7 claim missing in child writer"))?;
            let original = f.protected.read_bound_preimage(f.intent.operation_id)
                .map_err(refused)?
                .ok_or_else(|| refused("protected BLOB missing in child writer"))?;
            let pinned = PinnedReadOnlyRoot::open(&f.root).map_err(refused)?;
            let observed = pinned.read_identified_bounded(
                &f.intent.path, MAX_PROTECTED_ORIGINAL_BYTES as u64,
            ).map_err(refused)?;
            let candidate = InertExistingReplaceReceiptCandidate::from_strict_history(
                &claim, &original, history, &observed,
            ).map_err(refused)?;
            f.journal.borrow_mut()
                .record_existing_replace_receipt_candidate_once(&candidate)
                .map_err(refused)?;
            if stage == "after-v8" {
                p0g3d08c_abort_at(&workdir, "after-v8");
            }
            Ok(())
        },
    );
    panic!("writer unexpectedly returned instead of aborting at {stage}: {outcome:?}");
}

#[test]
fn p0g3d08c_real_writer_abort_matrix_has_no_automatic_replay() {
    for stage in ["after-v7", "after-primary", "after-history", "after-v8"] {
        let workdir = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact").arg(P0G3D08C_WRITER_CHILD).arg("--nocapture")
            .env("AKULA_P0G3D08C_STAGE", stage)
            .env("AKULA_P0G3D08C_WORKDIR", workdir.path())
            .output().unwrap();
        assert!(!output.status.success(),
            "writer must abort at {stage} rather than complete: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr));
        let marker = fs::read_to_string(
            workdir.path().join("p0g3d08c-crash-marker"),
        ).unwrap_or_else(|_| panic!(
            "writer crashed before reaching {stage}; stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ));
        assert_eq!(marker, stage, "writer must reach the exact crash seam");

        let record: serde_json::Value = serde_json::from_slice(&fs::read(
            workdir.path().join("p0g3d08c-fixture.json"),
        ).unwrap()).unwrap();
        let root = PathBuf::from(record["root"].as_str().unwrap());
        let appdata = PathBuf::from(record["appdata"].as_str().unwrap());
        let operation: OperationId = record["operation"].as_str().unwrap()
            .parse().unwrap();
        // Compare resolved components, not raw Windows path spellings.
        // Require the precise disposable fixture/vault hierarchy as well.
        let canonical_workdir = crate::search::paths::canonical_path(workdir.path());
        let canonical_root = crate::search::paths::canonical_path(&root);
        let relative = crate::search::paths::strip_root(&canonical_workdir, &canonical_root)
            .expect("writer may only use its disposable test vault");
        let components: Vec<_> = relative.components().collect();
        assert_eq!(components.len(), 2, "writer root must be fixture/vault");
        assert!(components[0].as_os_str().to_string_lossy()
            .starts_with("writer-crash-fixture-"),
            "writer root must use a disposable crash fixture");
        assert_eq!(components[1].as_os_str(), std::ffi::OsStr::new("vault"));
        let primary = root.join("Reserved.md");
        let original = b"unchanged private original";
        let target = "remote candidate using the real local history service "
            .repeat(100);
        let visible = fs::read(&primary).unwrap();
        if stage == "after-v7" {
            assert_eq!(visible, original);
        } else {
            assert_eq!(visible, target.as_bytes());
        }

        // A NEW process cannot reuse any previous in-memory history witness.
        // Before V8 commit it must fail closed; after V8 only forensic HOLD.
        let classification = if stage == "after-v8" { "hold" } else { "deny" };
        p0g3d08_run_child(&root, &appdata, operation, classification);

        let journal = SqliteSyncJournal::open_existing_read_only(
            &appdata.join("journal.sqlite"),
        ).unwrap();
        assert_eq!(journal.get(operation).unwrap().unwrap().phase,
            JournalPhase::Staged);
        assert!(journal.read_existing_replace_attempt(operation)
            .unwrap().is_some());
        let v8 = journal.read_existing_replace_receipt_candidate(operation)
            .unwrap();
        assert_eq!(v8.is_some(), stage == "after-v8");
        let protected = SqliteProtectionStore::open_existing_read_only(
            &appdata.join("originals.sqlite"),
        ).unwrap();
        assert_eq!(protected.read_bound_preimage(operation)
            .unwrap().unwrap().original.bytes, original);
        let intent = journal.get(operation).unwrap().unwrap().intent;
        let committed = journal.baseline(intent.vault_id, intent.document_id)
            .unwrap().unwrap();
        assert_eq!(Some(committed.revision_id), intent.expected_baseline);
        assert_eq!(committed.content_hash,
            blake3::hash(original).to_hex().to_string());
    }
}


/// P0g-3d-08d-1: selectively remove one independently persisted domain AFTER
/// a complete V8 candidate. This models possible post-power-loss *images*,
/// not a real VM hard-off or a proven storage durability barrier.
///
/// Each disposable case starts from a fully correlated, cold-reopenable V8
/// candidate. After dropping ALL writer/Core handles, only one domain is
/// changed; a second OS process must refuse the now-incomplete evidence.
/// No production publisher, auto-retry, S3 ACK or baseline promotion.
#[test]
fn p0g3d08d_partial_persistence_domains_fail_closed_after_v8() {
    for lost in ["v8_row", "history_name", "primary_rollback"] {
        let f = fixture();
        publish_strict_candidate_and_record_v8(&f);
        assert_no_remote_ack(&f);

        let version_id = f.journal.borrow()
            .read_existing_replace_receipt_candidate(f.intent.operation_id)
            .unwrap().unwrap().cold_history_fields().0.to_owned();
        let historical = f.root.join(".aquilum").join("history")
            .join(f.intent.path.as_str()).join(version_id);
        let primary = f.root.join(f.intent.path.as_str());
        assert!(historical.is_file(), "fresh strict history must exist");
        assert_eq!(fs::read(&primary).unwrap(), f.new_text.as_bytes());

        let Fixture {
            _temp: keep_fixture, core, root, appdata, bindings, custody,
            journal, cache, protected, intent, old_hash, ..
        } = f;
        drop(bindings);
        drop(custody);
        drop(journal);
        drop(cache);
        drop(protected);
        drop(core);

        // Read-only control from an independent OS process: all domains
        // match before the laboratory mutates exactly one.
        p0g3d08_run_child(&root, &appdata, intent.operation_id, "hold");

        match lost {
            "v8_row" => {
                // V7 and the protected original remain, V8 disappears from
                // its *own* WAL/FULL domain after a committed transaction.
                let db = rusqlite::Connection::open(appdata.join("journal.sqlite"))
                    .unwrap();
                let rows = db.execute(
                    "DELETE FROM sync_existing_replace_receipt_candidates WHERE operation_id=?1",
                    [intent.operation_id.to_string()],
                ).unwrap();
                assert_eq!(rows, 1, "the completed candidate must exist");
                drop(db);
            }
            "history_name" => {
                // The published primary + V8 remain, but the immutable
                // HistoryService directory entry is no longer visible.
                fs::remove_file(&historical).unwrap();
                assert!(!historical.exists());
            }
            "primary_rollback" => {
                // Model a visible pre-publication name/content after V8:
                // retain the published candidate separately in this fixture.
                let parked = root.join(".p0g3d08d-parked-candidate");
                fs::rename(&primary, &parked).unwrap();
                fs::write(&primary, b"unchanged private original").unwrap();
                assert_eq!(fs::read(&primary).unwrap(), b"unchanged private original");
                assert!(parked.is_file());
            }
            _ => unreachable!(),
        }

        // A *new* Core/HistoryService + three SQLite read-only stores must
        // reject the incomplete image, without attempting replay/repair.
        p0g3d08_run_child(&root, &appdata, intent.operation_id, "deny");

        // Independent failure domains must NOT silently rewrite the logical
        // baseline or make StageDownload eligible for remote ACK.
        let mut cold = SqliteSyncJournal::open(&appdata.join("journal.sqlite"))
            .unwrap();
        assert_eq!(cold.get(intent.operation_id).unwrap().unwrap().phase,
            JournalPhase::Staged);
        let baseline = cold.baseline(intent.vault_id, intent.document_id)
            .unwrap().unwrap();
        assert_eq!(Some(baseline.revision_id), intent.expected_baseline);
        assert_eq!(baseline.content_hash, old_hash);
        assert!(cold.read_existing_replace_attempt(intent.operation_id)
            .unwrap().is_some(), "V7 must not be replayed or cleared");
        assert!(matches!(cold.mark_applied(intent.operation_id),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)),
            "partial durable image must never authorize StageDownload");
        drop(cold);

        let originals = SqliteProtectionStore::open_existing_read_only(
            &appdata.join("originals.sqlite"),
        ).unwrap();
        assert_eq!(originals.read_bound_preimage(intent.operation_id)
            .unwrap().unwrap().original.bytes, b"unchanged private original",
            "independent original must remain recoverable");
        drop(originals);
        drop(keep_fixture);
    }
}

include!("s3_reserved_replace_g2_fault_tests.rs");
