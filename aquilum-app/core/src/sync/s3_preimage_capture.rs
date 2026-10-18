//! SYNC-APPLY-P0a: S3-bound protective ORIGINAL capture, with no local apply.
//!
//! Reuses host RegisteredReadAdmission and SqliteProtectionStore. Holding the
//! external custody lock over the existing pinned File Gate read and SQLite
//! FULL/WAL preimage commit makes this a cooperative preparation step ONLY.
//!
//! A recovered preimage is NOT a native write receipt, protection from a
//! hostile same-user process, permission to overwrite, or a local baseline
//! acknowledgment. No filesystem mutation or journal advancement is exposed.

use std::path::Path;

use super::{
    ConfirmedBaseline, JournalAction, JournalError, JournalPhase, JournalRecord,
    OperationId, S3ConnectionProfile, S3Keyspace, SqliteSyncJournal,
    VaultPortError,
};
use super::filegate_custody_identity::{
    CustodyBindingError, SqliteCustodyIdentityStore,
};
use super::host_admission::{BoundCaptureError, RegisteredReadAdmission};
use super::profile_binding::{
    self, RegisteredS3Binding, SqliteRegisteredS3Bindings,
};
use super::protection_store::{
    ProtectionOutcome, ProtectionStoreError, SqliteProtectionStore,
};

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum S3ProtectedPreimageError {
    InvalidRegisteredProfile,
    InvalidStagedUpdate,
    PreviousBaselineChanged,
    UnsafeAppdata,
    Admission(VaultPortError),
    Journal(JournalError),
    Capture(BoundCaptureError),
    Protection(ProtectionStoreError),
    Custody(CustodyBindingError),
}

impl From<VaultPortError> for S3ProtectedPreimageError {
    fn from(v: VaultPortError) -> Self { Self::Admission(v) }
}
impl From<JournalError> for S3ProtectedPreimageError {
    fn from(v: JournalError) -> Self { Self::Journal(v) }
}
impl From<ProtectionStoreError> for S3ProtectedPreimageError {
    fn from(v: ProtectionStoreError) -> Self { Self::Protection(v) }
}
impl From<BoundCaptureError> for S3ProtectedPreimageError {
    fn from(v: BoundCaptureError) -> Self { Self::Capture(v) }
}
impl From<CustodyBindingError> for S3ProtectedPreimageError {
    fn from(v: CustodyBindingError) -> Self { Self::Custody(v) }
}

/// Check the persisted operation AND previous confirmed document baseline.
/// A cloud HEAD/ETag, metadata-only V2 preview or arbitrarily staged bytes
/// are not proof of an eligible replacement. This is merely a protective
/// snapshot gate; the future native executor must re-verify ancestry, heads,
/// bytes, editor state, OS identities and mutation authority independently.
fn eligible_update(
    journal: &SqliteSyncJournal,
    operation: OperationId,
    binding: &RegisteredS3Binding,
    keyspace: &S3Keyspace,
    expected_local_hash: &str,
) -> Result<JournalRecord, S3ProtectedPreimageError> {
    let invalid = || S3ProtectedPreimageError::InvalidStagedUpdate;
    let record = journal.get(operation)?.ok_or_else(invalid)?;
    let intent = &record.intent;
    if record.phase != JournalPhase::Staged
        || intent.action != JournalAction::StageDownload
        || intent.remote_guard.is_some()
        || intent.operation_id != operation
        || intent.vault_id != binding.vault_id
        || intent.profile_id != binding.profile_id
        || intent.expected_baseline.is_none()
        || intent.revision_id == intent.expected_baseline.unwrap()
        || record.staged_hash.as_deref()
            != Some(intent.expected_content_hash.as_str())
    {
        return Err(invalid());
    }
    let blob_key = keyspace.immutable_blob_key(&intent.expected_content_hash)
        .map_err(|_| invalid())?;
    if intent.remote_object_id != blob_key {
        return Err(invalid());
    }
    if intent.path.as_str().split('/').any(|part|
        part.eq_ignore_ascii_case(".aquilum")
            || part.eq_ignore_ascii_case(".trash")
            || part.eq_ignore_ascii_case(".git"))
    {
        return Err(invalid());
    }
    let baseline: ConfirmedBaseline = journal.baseline(
        intent.vault_id, intent.document_id,
    )?.ok_or(S3ProtectedPreimageError::PreviousBaselineChanged)?;
    if Some(baseline.revision_id) != intent.expected_baseline
        || baseline.profile_id != intent.profile_id
        || baseline.content_hash != expected_local_hash
    {
        return Err(S3ProtectedPreimageError::PreviousBaselineChanged);
    }
    // The baseline table does not store a logical pathname. Never use an
    // apparently matching revision/hash to capture the wrong local file:
    // recover the original committed operation and bind its exact path,
    // document identity and revision to this staged update. Renames need
    // their own explicit DAG admission, not this preimage bridge.
    let previous = journal.get(baseline.operation_id)?
        .ok_or(S3ProtectedPreimageError::PreviousBaselineChanged)?;
    if previous.phase != JournalPhase::Committed
        || previous.intent.operation_id != baseline.operation_id
        || previous.intent.vault_id != baseline.vault_id
        || previous.intent.document_id != baseline.document_id
        || previous.intent.profile_id != baseline.profile_id
        || previous.intent.revision_id != baseline.revision_id
        || previous.intent.expected_content_hash != baseline.content_hash
        || previous.intent.path != intent.path
    {
        return Err(S3ProtectedPreimageError::PreviousBaselineChanged);
    }
    Ok(record)
}

/// Take a durable copy of an EXISTING, closed, hash-identical local original.
///
/// Neither protected bytes nor any custody proof are accepted from S3. The
/// target is derived from the previously journaled relative path and the
/// local baseline hash, read from a pinned registered Core workspace.
/// The existing protected-original database is never placed in the vault.
/// The OS custody lock spans all local reads and the SQLite FULL commit.
///
/// Safe failure: a backup may already have committed if a final check fails.
/// Such a record is forensic evidence only; callers must not interpret a
/// returned Ok or Err as authorizing local file replacement or baseline
/// progression. A later implementation needs a separately verified native
/// protected mutation transaction.
#[allow(dead_code)]
pub(crate) fn capture_registered_s3_update_preimage(
    admission: &RegisteredReadAdmission<'_>,
    bindings: &SqliteRegisteredS3Bindings,
    profile: &S3ConnectionProfile,
    keyspace: &S3Keyspace,
    custody: &SqliteCustodyIdentityStore,
    custody_scope: &Path,
    journal: &SqliteSyncJournal,
    protected: &mut SqliteProtectionStore,
    operation: OperationId,
    expected_local_hash: &str,
) -> Result<ProtectionOutcome, S3ProtectedPreimageError> {
    admission.recheck_registered_root()?;
    let root = admission.registered_root_path();
    // A SQLite WAL/SHM shadow inside the sync vault is never acceptable.
    // Both pre-existing files must be validated BEFORE protection writes.
    journal.require_appdata_outside_vault(root)?;
    profile_binding::validate_appdata_location(
        admission, protected.database_path(),
    ).map_err(|_| S3ProtectedPreimageError::UnsafeAppdata)?;
    let binding = bindings.load_for_registered(admission)
        .map_err(|_| S3ProtectedPreimageError::InvalidRegisteredProfile)?
        .ok_or(S3ProtectedPreimageError::InvalidRegisteredProfile)?;
    if !binding.matches_remote(profile, keyspace) {
        return Err(S3ProtectedPreimageError::InvalidRegisteredProfile);
    }
    let original_record = eligible_update(
        journal, operation, &binding, keyspace, expected_local_hash,
    )?;
    let outcome = custody.with_cooperative_writer_lifetime(
        admission, binding.vault_id, custody_scope, || {
            if bindings.load_for_registered(admission).ok().flatten().as_ref()
                != Some(&binding)
            {
                return Err(S3ProtectedPreimageError::InvalidRegisteredProfile);
            }
            if eligible_update(
                journal, operation, &binding, keyspace, expected_local_hash,
            )?.intent != original_record.intent
            {
                return Err(S3ProtectedPreimageError::InvalidStagedUpdate);
            }
            let saved = admission.preserve_closed_original(
                &original_record.intent.path, expected_local_hash,
                operation, protected,
            )?;
            // A post-commit error must remain HOLD, not a forged receipt.
            if eligible_update(
                journal, operation, &binding, keyspace, expected_local_hash,
            )?.intent != original_record.intent
                || bindings.load_for_registered(admission).ok().flatten().as_ref()
                    != Some(&binding)
            {
                return Err(S3ProtectedPreimageError::PreviousBaselineChanged);
            }
            let copy = protected.read_bound_preimage(operation)?
                .ok_or(ProtectionStoreError::UnboundOriginal)?;
            if copy.original.path != original_record.intent.path
                || copy.original.content_hash != expected_local_hash
                || blake3::hash(&copy.original.bytes).to_hex().as_str()
                    != expected_local_hash
            {
                return Err(S3ProtectedPreimageError::Protection(
                    ProtectionStoreError::CorruptedRecord
                ));
            }
            Ok(saved)
        },
    )?;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{
        DocumentId, OperationIntent, RemoteGuard, RevisionId, S3HeadPointerV1,
        S3ManifestV1, S3VerifiedHead, SyncRelativePath, VaultId,
    };
    use super::super::filegate_lock_custody::scoped_leaf;
    use super::super::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
    use crate::app_core::{Core, CoreEvent};
    use std::{fs, path::PathBuf, sync::Arc};

    struct Fixture {
        _dir: tempfile::TempDir,
        core: Arc<Core>,
        root: PathBuf,
        appdata: PathBuf,
        scope: PathBuf,
        profile: S3ConnectionProfile,
        keyspace: S3Keyspace,
        operation: OperationId,
        old_hash: String,
    }

    impl Fixture {
        fn admission(&self) -> RegisteredReadAdmission<'_> {
            RegisteredReadAdmission::open(&self.core, &self.root).unwrap()
        }
    }

    fn fixture() -> Fixture {
        fixture_with_previous_path("notes/source.md")
    }

    fn fixture_with_previous_path(previous_path: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let appdata = dir.path().join("appdata");
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::create_dir_all(&appdata).unwrap();
        fs::write(root.join("notes/source.md"), b"unchanged original").unwrap();
        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();

        let keyspace = S3Keyspace::new("akula/s3-protected-test", VaultId::new()).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-preimage-test".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://127.0.0.1:9000".into()),
            force_path_style: true,
        };
        let manifest = S3ManifestV1::new(
            &keyspace, OperationId::new(), 1, None, vec![],
        ).unwrap();
        let head = S3VerifiedHead {
            pointer: S3HeadPointerV1::new(
                &keyspace, 1, manifest.manifest_id,
                &manifest.digest(&keyspace).unwrap(), None,
            ).unwrap(),
            etag: "\"pinned-head\"".into(),
            manifest,
        };
        let scope = dir.path().join("external-lock")
            .join(scoped_leaf(PinnedProbeRoot::open(&root).unwrap().identity()));
        fs::create_dir_all(&scope).unwrap();
        let lock = scope.join(LOCK_NAME);
        fs::write(&lock, b"existing OS lock").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&scope, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        }
        #[cfg(unix)]
        let scope = fs::canonicalize(scope).unwrap();

        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut bindings = SqliteRegisteredS3Bindings::open(
            &admission, &appdata.join("profile.sqlite"),
        ).unwrap();
        bindings.bind_existing_verified_s3(
            &admission, "my-s3-profile", &profile, &keyspace, &head,
        ).unwrap();
        let mut custody = SqliteCustodyIdentityStore::open(
            &admission, &appdata.join("custody.sqlite"),
        ).unwrap();
        custody.bind_registered_s3(&admission, &bindings, &scope).unwrap();

        let mut journal = SqliteSyncJournal::open(
            &appdata.join("operation.sqlite"),
        ).unwrap();
        let old_hash = blake3::hash(b"unchanged original").to_hex().to_string();
        let document = DocumentId::new();
        let old_revision = RevisionId::new();
        let original = OperationIntent {
            operation_id: OperationId::new(),
            vault_id: keyspace.vault_id(),
            document_id: document,
            profile_id: "my-s3-profile".into(),
            revision_id: old_revision,
            action: JournalAction::UploadImmutable,
            path: SyncRelativePath::parse(previous_path).unwrap(),
            remote_object_id: format!("objects/{old_revision}"),
            remote_guard: Some(RemoteGuard::Absent),
            expected_content_hash: old_hash.clone(),
            expected_baseline: None,
        };
        journal.begin(original.clone()).unwrap();
        journal.mark_staged(original.operation_id, b"unchanged original").unwrap();
        journal.mark_applied(original.operation_id).unwrap();
        journal.mark_verified(
            original.operation_id, b"unchanged original", "provider-1",
        ).unwrap();
        journal.commit(original.operation_id).unwrap();

        let next_bytes = b"new cloud content";
        let next_hash = blake3::hash(next_bytes).to_hex().to_string();
        let operation = OperationId::new();
        journal.begin(OperationIntent {
            operation_id: operation,
            vault_id: keyspace.vault_id(),
            document_id: document,
            profile_id: "my-s3-profile".into(),
            revision_id: RevisionId::new(),
            action: JournalAction::StageDownload,
            path: SyncRelativePath::parse("notes/source.md").unwrap(),
            remote_object_id: keyspace.immutable_blob_key(&next_hash).unwrap(),
            remote_guard: None,
            expected_content_hash: next_hash,
            expected_baseline: Some(old_revision),
        }).unwrap();
        journal.mark_staged(operation, next_bytes).unwrap();
        drop(admission);

        Fixture { _dir: dir, core, root, appdata, scope, profile, keyspace,
            operation, old_hash }
    }

    #[test]
    fn s3_protected_preimage_is_durable_read_only_and_idempotent() {
        let f = fixture();
        let admission = f.admission();
        let bindings = SqliteRegisteredS3Bindings::open(
            &admission, &f.appdata.join("profile.sqlite"),
        ).unwrap();
        let custody = SqliteCustodyIdentityStore::open(
            &admission, &f.appdata.join("custody.sqlite"),
        ).unwrap();
        let mut journal = SqliteSyncJournal::open(
            &f.appdata.join("operation.sqlite"),
        ).unwrap();
        let copy_db = f.appdata.join("originals.sqlite");
        {
            let mut copy = SqliteProtectionStore::open(&copy_db).unwrap();
            assert_eq!(capture_registered_s3_update_preimage(
                &admission, &bindings, &f.profile, &f.keyspace,
                &custody, &f.scope, &journal, &mut copy,
                f.operation, &f.old_hash,
            ).unwrap(), ProtectionOutcome::Stored);
            assert_eq!(capture_registered_s3_update_preimage(
                &admission, &bindings, &f.profile, &f.keyspace,
                &custody, &f.scope, &journal, &mut copy,
                f.operation, &f.old_hash,
            ).unwrap(), ProtectionOutcome::AlreadyStored);
        }
        let copy = SqliteProtectionStore::open(&copy_db).unwrap();
        let saved = copy.read_bound_preimage(f.operation).unwrap().unwrap();
        assert_eq!(saved.original.bytes, b"unchanged original");
        assert_eq!(saved.original.content_hash, f.old_hash);
        assert_eq!(fs::read(f.root.join("notes/source.md")).unwrap(),
            b"unchanged original");
        assert_eq!(journal.get(f.operation).unwrap().unwrap().phase,
            JournalPhase::Staged);
        assert!(matches!(
            journal.mark_applied(f.operation),
            Err(JournalError::LocalDownloadNeedsNativeReceipt)
        ));
    }

    #[test]
    fn refuses_a_baseline_from_another_path_even_with_identical_bytes() {
        let f = fixture_with_previous_path("notes/some-other.md");
        let admission = f.admission();
        let bindings = SqliteRegisteredS3Bindings::open(
            &admission, &f.appdata.join("profile.sqlite"),
        ).unwrap();
        let custody = SqliteCustodyIdentityStore::open(
            &admission, &f.appdata.join("custody.sqlite"),
        ).unwrap();
        let journal = SqliteSyncJournal::open(
            &f.appdata.join("operation.sqlite"),
        ).unwrap();
        let mut copy = SqliteProtectionStore::open(
            &f.appdata.join("originals.sqlite"),
        ).unwrap();
        assert!(matches!(capture_registered_s3_update_preimage(
            &admission, &bindings, &f.profile, &f.keyspace,
            &custody, &f.scope, &journal, &mut copy,
            f.operation, &f.old_hash,
        ), Err(S3ProtectedPreimageError::PreviousBaselineChanged)));
        assert!(copy.read_bound_preimage(f.operation).unwrap().is_none());
        assert_eq!(fs::read(f.root.join("notes/source.md")).unwrap(),
            b"unchanged original");
    }

    #[test]
    fn s3_protected_preimage_refuses_pending_dirty_and_mismatched_local() {
        let f = fixture();
        let admission = f.admission();
        let bindings = SqliteRegisteredS3Bindings::open(
            &admission, &f.appdata.join("profile.sqlite"),
        ).unwrap();
        let custody = SqliteCustodyIdentityStore::open(
            &admission, &f.appdata.join("custody.sqlite"),
        ).unwrap();
        let journal = SqliteSyncJournal::open(
            &f.appdata.join("operation.sqlite"),
        ).unwrap();
        let mut copy = SqliteProtectionStore::open(
            &f.appdata.join("originals.sqlite"),
        ).unwrap();
        assert!(matches!(capture_registered_s3_update_preimage(
            &admission, &bindings, &f.profile, &f.keyspace,
            &custody, &f.scope, &journal, &mut copy,
            OperationId::new(), &f.old_hash,
        ), Err(S3ProtectedPreimageError::InvalidStagedUpdate)));

        let wrong = S3ConnectionProfile { region: "eu-west-1".into(),
            ..f.profile.clone() };
        assert!(matches!(capture_registered_s3_update_preimage(
            &admission, &bindings, &wrong, &f.keyspace,
            &custody, &f.scope, &journal, &mut copy,
            f.operation, &f.old_hash,
        ), Err(S3ProtectedPreimageError::InvalidRegisteredProfile)));

        let path = f.root.join("notes/source.md");
        f.core.documents.open(&f.core, &path, 0).unwrap();
        assert!(capture_registered_s3_update_preimage(
            &admission, &bindings, &f.profile, &f.keyspace,
            &custody, &f.scope, &journal, &mut copy,
            f.operation, &f.old_hash,
        ).is_err());
        f.core.documents.release(&f.core, &path);
        assert!(copy.read_bound_preimage(f.operation).unwrap().is_none());

        fs::write(&path, b"external concurrent edit").unwrap();
        assert!(matches!(capture_registered_s3_update_preimage(
            &admission, &bindings, &f.profile, &f.keyspace,
            &custody, &f.scope, &journal, &mut copy,
            f.operation, &f.old_hash,
        ), Err(S3ProtectedPreimageError::Capture(
            BoundCaptureError::OriginalHashMismatch
        ))));
        assert!(copy.read_bound_preimage(f.operation).unwrap().is_none());
    }

    #[test]
    fn s3_protected_preimage_refuses_replaced_custody_lock() {
        let f = fixture();
        let admission = f.admission();
        let bindings = SqliteRegisteredS3Bindings::open(
            &admission, &f.appdata.join("profile.sqlite"),
        ).unwrap();
        let custody = SqliteCustodyIdentityStore::open(
            &admission, &f.appdata.join("custody.sqlite"),
        ).unwrap();
        let journal = SqliteSyncJournal::open(
            &f.appdata.join("operation.sqlite"),
        ).unwrap();
        let mut copy = SqliteProtectionStore::open(
            &f.appdata.join("originals.sqlite"),
        ).unwrap();
        fs::rename(f.scope.join(LOCK_NAME), f.scope.join("old-lock")).unwrap();
        fs::write(f.scope.join(LOCK_NAME), b"replacement lock").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(f.scope.join(LOCK_NAME),
                fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(capture_registered_s3_update_preimage(
            &admission, &bindings, &f.profile, &f.keyspace,
            &custody, &f.scope, &journal, &mut copy,
            f.operation, &f.old_hash,
        ).is_err());
        assert!(copy.read_bound_preimage(f.operation).unwrap().is_none());
        assert_eq!(fs::read(f.root.join("notes/source.md")).unwrap(),
            b"unchanged original");
    }
}
