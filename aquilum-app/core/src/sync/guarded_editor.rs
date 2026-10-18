//! Explicit opt-in for routing the real File Gate/editor save through the
//! existing persisted S3 profile + custody OS lock. No automatic activation.
//! This is cooperative-process serialization, NOT a hostile-writer-proof
//! capability or sync overwrite/delete authorization. Other File Gate and
//! direct HistoryService mutation routes are still outside this first slice.

use super::filegate_custody_identity::SqliteCustodyIdentityStore;
use super::guarded_note_write::{save_existing_note_under_custody, GuardedNoteWriteError};
use super::profile_binding::{RegisteredS3Binding, SqliteRegisteredS3Bindings};
use super::{RegisteredReadAdmission, SyncRelativePath};
use crate::app_core::Core;
use crate::files::error::FileCommandError;
use crate::files::models::FileWriteResult;
use crate::history::Source;
use std::fs;
use std::path::{Path, PathBuf};

// A durable SQLite row alone is NOT write authority: after a process crash
// its old (base,new) digests can still match the physical file. Only the
// DocumentSession thread that *just* committed that row may attempt a guarded
// File Gate mutation. These permits never persist or survive restart.
#[derive(Debug)]
struct LiveDocumentAttempt {
    instance_id: uuid::Uuid,
    key: String,
    base_hash: String,
    proposed_hash: String,
}

thread_local! {
    static LIVE_DOCUMENT_ATTEMPT: std::cell::RefCell<Option<LiveDocumentAttempt>> =
        const { std::cell::RefCell::new(None) };
}

pub(crate) fn arm_live_document_attempt(
    instance_id: uuid::Uuid, path: &Path, base_hash: &str, proposed_hash: &str,
) -> Result<(), FileCommandError> {
    let key = crate::search::paths::identity(path);
    LIVE_DOCUMENT_ATTEMPT.with(|cell| {
        let mut live = cell.borrow_mut();
        if live.as_ref().is_some_and(|existing| existing.instance_id == instance_id) {
            return Err(refused("document thread already holds a native write attempt"));
        }
        // Discard an ephemeral ticket from a different Core constructed on
        // this thread. Its durable SQLite record is NOT cleared or reused.
        *live = Some(LiveDocumentAttempt {
            instance_id, key, base_hash: base_hash.to_owned(),
            proposed_hash: proposed_hash.to_owned(),
        });
        Ok(())
    })
}

fn live_attempt_matches(instance_id: uuid::Uuid, path: &Path, base_hash: &str, proposed_hash: &str) -> bool {
    let key = crate::search::paths::identity(path);
    LIVE_DOCUMENT_ATTEMPT.with(|cell| cell.borrow().as_ref().is_some_and(|live| {
        live.instance_id == instance_id && live.key.as_str() == key.as_str()
            && live.base_hash == base_hash && live.proposed_hash == proposed_hash
    }))
}

/// Drop on success, denial, panic or ambiguous HistoryService postflight:
/// there is NEVER a second File Gate attempt authorized by the same ticket.
struct ConsumeDocumentAttempt(String);
impl Drop for ConsumeDocumentAttempt {
    fn drop(&mut self) {
        LIVE_DOCUMENT_ATTEMPT.with(|cell| {
            let mut live = cell.borrow_mut();
            if live.as_ref().is_some_and(|attempt| attempt.key.as_str() == self.0.as_str()) {
                *live = None;
            }
        });
    }
}



#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveEditorGuard {
    root: PathBuf,
    profile_db: PathBuf,
    custody_db: PathBuf,
    scope: PathBuf,
    binding: RegisteredS3Binding,
}

fn refused(message: impl Into<String>) -> FileCommandError {
    FileCommandError::Task { message: message.into() }
}

fn existing_real_db(path: &Path) -> Result<PathBuf, FileCommandError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(refused("cooperative editor activation requires existing regular SQLite databases"));
    }
    Ok(fs::canonicalize(path)?)
}

/// The host must explicitly activate this AFTER user-reviewed S3 profile
/// binding and separately provisioned physical custody identity. No S3 IO,
/// new OS lock object, rebind or file mutation occurs here. This is a runtime
/// flag, not persisted UI preference: hosts must review/revalidate on restart.
#[allow(dead_code)]
pub(crate) fn activate_registered_s3_editor(
    core: &Core,
    selected_root: &Path,
    profile_db: &Path,
    custody_db: &Path,
    custody_scope: &Path,
) -> Result<(), FileCommandError> {
    let admission = RegisteredReadAdmission::open(core, selected_root)
        .map_err(|error| refused(format!("unregistered editor workspace: {error:?}")))?;
    let profile_db = existing_real_db(profile_db)?;
    let custody_db = existing_real_db(custody_db)?;
    let profiles = SqliteRegisteredS3Bindings::open(&admission, &profile_db)
        .map_err(|error| refused(format!("invalid S3 binding database: {error:?}")))?;
    let binding = profiles.load_for_registered(&admission)
        .map_err(|error| refused(format!("S3 binding mismatch: {error:?}")))?
        .ok_or_else(|| refused("registered workspace has no bound S3 profile"))?;
    let custody = SqliteCustodyIdentityStore::open(&admission, &custody_db)
        .map_err(|error| refused(format!("invalid custody database: {error:?}")))?;
    let verified = custody.verify_registered_s3(&admission, &profiles, custody_scope)
        .map_err(|error| refused(format!("custody identity mismatch: {error:?}")))?;
    if verified.workspace_id != binding.workspace_id || verified.vault_id != binding.vault_id {
        return Err(refused("cooperative editor VaultId is not consistent"));
    }
    let config = ActiveEditorGuard {
        root: admission.registered_root_path().to_path_buf(),
        profile_db,
        custody_db,
        scope: fs::canonicalize(custody_scope)?,
        binding,
    };
    // DocumentHub writers hold sessions while waiting for File Gate. Arming
    // under PROCESS_FILE_MUTATIONS would invert that order and deadlock.
    // First reject existing buffers and install the fail-closed *prewrite*
    // root. If later custody enrollment fails, new DocumentSession writers
    // still cannot fall back: they require a one-shot receipt, and the
    // unactivated guarded route explicitly refuses that receipt.
    core.documents.arm_durable_intents_for_root(&config.root)?;
    // Close the admission transition against an ordinary File Gate operation
    // already inside its process-wide primary-file critical section. Take
    // this FIRST, before the HistoryService barrier, to preserve the
    // established File Gate -> HistoryFiles ordering.
    let _file_gate_fence = core.file_mutation_guard()?;
    let mut registry = core.guarded_editors.lock()
        .map_err(|_| refused("cooperative editor registry poisoned"))?;
    for previous in registry.iter() {
        if previous.root.starts_with(&config.root) || config.root.starts_with(&previous.root) {
            // Never switch an armed root to a different custody config in
            // memory. Host must restart and undergo explicit new admission.
            return if previous == &config {
                Ok(())
            } else {
                Err(refused("ambiguous or conflicting cooperative editor root"))
            };
        }
    }
    // This must succeed BEFORE any guarded File Gate route becomes reachable.
    // In particular, it refuses activation with open CRDT editor sessions
    // or an unreadable v2 SQLite write-intent journal.
    core.history.with_participant_enrollment_fence(|| {
        // Holding both the process File Gate and HistoryFiles barriers means
        // no in-flight ordinary primary mutation or direct history mutation
        // is actively touching those files when HOLD becomes visible.
        // DocumentHub prewrite admission was armed BEFORE taking File Gate.
        // Commit the appdata participant marker before publishing the writer
        // route. A failed insert keeps the Core on HOLD with no silent retry.
        core.documents.enroll_guarded_participant(
            &config.root, &config.binding.vault_id.to_string(),
            &config.profile_db, &config.custody_db, &config.scope,
        )?;
        registry.push(config);
        Ok(())
    })
}

/// A matching known path must never fall back to the unguarded writer just
/// because its SQLite records/OS lock are missing or corrupted.
fn selected(
    core: &Core,
    path: &Path,
) -> Result<Option<(ActiveEditorGuard, SyncRelativePath)>, FileCommandError> {
    let registry = core.guarded_editors.lock()
        .map_err(|_| refused("cooperative editor registry poisoned"))?;
    if registry.is_empty() {
        return Ok(None);
    }
    let canonical = fs::canonicalize(path).ok();
    let matching: Vec<&ActiveEditorGuard> = registry
        .iter()
        .filter(|config| {
            path.starts_with(&config.root)
                || canonical.as_ref().is_some_and(|resolved| resolved.starts_with(&config.root))
        })
        .collect();
    let [config] = matching.as_slice() else {
        if matching.is_empty() { return Ok(None); }
        return Err(refused("overlapping cooperative editor registration"));
    };
    let scoped = if path.starts_with(&config.root) {
        path
    } else {
        canonical.as_deref().ok_or_else(|| refused("cannot identify guarded editor target"))?
    };
    let relative = scoped.strip_prefix(&config.root)
        .map_err(|_| refused("guarded editor path escapes its root"))?;
    let relative = relative.to_str()
        .ok_or_else(|| refused("invalid guarded editor filename encoding"))?;
    let relative = SyncRelativePath::parse(relative)
        .map_err(|error| refused(format!("unsafe guarded editor relative path: {error:?}")))?;
    Ok(Some(((**config).clone(), relative)))
}

pub(crate) fn active_for(core: &Core, path: &Path) -> Result<bool, FileCommandError> {
    selected(core, path).map(|item| item.is_some())
}

/// Routing point invoked by the REAL File Gate write before its process mutex.
/// A false/missing receipt or newly busy lock is a denial, not unguarded
/// fallback. The outer checked path owns the lock through HistoryService.
pub(crate) fn maybe_write(
    core: &Core,
    path: &Path,
    content: &str,
    expected_hash: Option<&str>,
    source: Source,
    version_name: Option<&str>,
) -> Option<Result<FileWriteResult, FileCommandError>> {
    // Even if the guarded root has not been activated, a document thread
    // with an already FULL-synced prewrite intent must NOT fall back to the
    // unguarded legacy File Gate writer.
    // The ordinary no-sync editor fast path must not canonicalize/hash a
    // pathname for every save. Only a live attempt requires this comparison.
    let live_key = LIVE_DOCUMENT_ATTEMPT.with(|cell| {
        cell.borrow().as_ref().map(|attempt| attempt.key.clone())
    });
    let matching_key = live_key.filter(|live| {
        crate::search::paths::identity(path).as_str() == live.as_str()
    });
    let has_live_prewrite = matching_key.is_some();
    // Consume only the permit for THIS document, even on an early refusal.
    let _consume = matching_key.map(ConsumeDocumentAttempt);
    let selected = match selected(core, path) {
        Ok(None) if has_live_prewrite => return Some(Err(refused(
            "durable DocumentSession intent has no active S3 custody; refusing unguarded fallback",
        ))),
        Ok(None) => return None,
        Ok(Some(selected)) => selected,
        Err(error) => return Some(Err(error)),
    };
    Some((|| {
        let (config, relative) = selected;
        let expected = expected_hash.ok_or_else(|| {
            refused("cooperative editor save requires a disk-content CAS hash")
        })?;
        // No direct native/MCP File Gate save may mutate an armed root without
        // the crash-durable pre-intent that DocumentSession commits first.
        // This also verifies the EXACT old and intended new BLAKE3 digests.
        let proposed = blake3::hash(content.as_bytes()).to_hex().to_string();
        if !live_attempt_matches(core.guarded_instance_nonce, path, expected, &proposed) {
            return Err(refused("guarded File Gate requires a fresh one-shot DocumentSession attempt"));
        }
        core.documents.require_matching_durable_intent(path, expected, &proposed)?;
        let admission = RegisteredReadAdmission::open(core, &config.root)
            .map_err(|error| refused(format!("workspace registration changed: {error:?}")))?;
        let profiles = SqliteRegisteredS3Bindings::open(&admission, &config.profile_db)
            .map_err(|error| refused(format!("S3 profile admission failed: {error:?}")))?;
        let observed = profiles.load_for_registered(&admission)
            .map_err(|error| refused(format!("S3 profile recheck failed: {error:?}")))?;
        if observed.as_ref() != Some(&config.binding) {
            return Err(refused("cooperative S3 binding changed since activation"));
        }
        let custody = SqliteCustodyIdentityStore::open(&admission, &config.custody_db)
            .map_err(|error| refused(format!("custody store recheck failed: {error:?}")))?;
        let verified = custody.verify_registered_s3(&admission, &profiles, &config.scope)
            .map_err(|error| refused(format!("custody recheck failed: {error:?}")))?;
        if verified.vault_id != config.binding.vault_id ||
            verified.workspace_id != config.binding.workspace_id {
            return Err(refused("cooperative editor physical identity drift"));
        }
        save_existing_note_under_custody(
            &admission, &custody, config.binding.vault_id, &config.scope,
            &relative, content, expected, source, version_name,
        ).map_err(|error| match error {
            GuardedNoteWriteError::Denied(message) => refused(message),
            GuardedNoteWriteError::NeedsReview(message) =>
                FileCommandError::NeedsReview { message },
        })
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{
        filegate_lock_custody::scoped_leaf, filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME},
        OperationId, S3ConnectionProfile, S3HeadPointerV1, S3ManifestV1,
        S3Keyspace, S3VerifiedHead, VaultId,
    };
    use crate::app_core::{Core, CoreEvent};
    use crate::files::error::FileCommandError;
    use crate::files::gate;
    use crate::history::service::HistoryFault;
    use std::sync::Arc;


    #[test]
    fn prepared_prewrite_without_activated_custody_cannot_use_legacy_gate() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let data = temp.path().join("appdata");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&data).unwrap();
        let root = crate::search::paths::canonical_path(&root);
        let path = root.join("NoRoute.md");
        fs::write(&path, b"previous").unwrap();
        let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        core.documents.arm_durable_intents_for_root(&root).unwrap();
        let before = blake3::hash(b"previous").to_hex().to_string();
        core.documents.test_seed_guarded_intent(&core, &path, &before, "new text");
        let denied = gate::write(
            &core, &path, "new text", Some(&before), Source::Me, None,
        );
        assert!(matches!(denied, Err(FileCommandError::Task { .. })));
        assert_eq!(fs::read(&path).unwrap(), b"previous");
        assert!(!live_attempt_matches(
            core.guarded_instance_nonce, &path, &before,
            &blake3::hash(b"new text").to_hex().to_string(),
        ), "failed route selection must consume the one-shot ticket");
        // The FULL-synced record is still present for restart/review.
        assert!(core.documents.require_matching_durable_intent(
            &path, &before, &blake3::hash(b"new text").to_hex().to_string(),
        ).is_ok());
    }

    #[test]
    fn optin_file_gate_requires_durable_document_intent_and_pinned_os_lock() {
        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let data = temp.path().join("appdata");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&data).unwrap();
        let vault = crate::search::paths::canonical_path(&vault);
        let note = vault.join("Shared.md");
        fs::write(&note, b"original").unwrap();
        let mismatch_note = vault.join("Mismatch.md");
        fs::write(&mismatch_note, b"mismatch original").unwrap();
        let lock_note = vault.join("Lock.md");
        fs::write(&lock_note, b"lock original").unwrap();
        let core = Core::open(&data, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();

        let root_id = PinnedProbeRoot::open(&vault).unwrap().identity();
        let scope = temp.path().join("external").join(scoped_leaf(root_id));
        fs::create_dir_all(&scope).unwrap();
        let lock = scope.join(LOCK_NAME);
        fs::write(&lock, b"stable existing OS lock").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&scope, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        }
        // macOS tempfile paths often use /var -> /private/var.
        // The production custody gate correctly rejects aliased ancestors;
        // canonicalize the fixture's pre-existing scope, never the gate.
        #[cfg(unix)]
        let scope = fs::canonicalize(&scope).unwrap();
        let namespace = S3Keyspace::new("akula/optin-editor", VaultId::new()).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-guarded-editor-test".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://127.0.0.1:9000".into()),
            force_path_style: true,
        };
        let manifest = S3ManifestV1::new(
            &namespace, OperationId::new(), 1, None, vec![],
        ).unwrap();
        let pointer = S3HeadPointerV1::new(
            &namespace, 1, manifest.manifest_id,
            &manifest.digest(&namespace).unwrap(), None,
        ).unwrap();
        let verified = S3VerifiedHead {
            pointer, manifest, etag: "\"test-only-etag\"".into(),
        };
        let profile_db = data.join("registered-s3.sqlite");
        let custody_db = data.join("custody.sqlite");
        {
            let admission = RegisteredReadAdmission::open(&core, &vault).unwrap();
            let mut profiles = SqliteRegisteredS3Bindings::open(&admission, &profile_db).unwrap();
            profiles.bind_existing_verified_s3(
                &admission, "optin-editor", &profile, &namespace, &verified,
            ).unwrap();
            let mut custody = SqliteCustodyIdentityStore::open(&admission, &custody_db).unwrap();
            custody.bind_registered_s3(&admission, &profiles, &scope).unwrap();
        }
        // An already-open CRDT editor cannot be switched mid-save to the
        // guarded route: its unsent in-memory changes have not been fenced.
        core.documents.open(&core, &note, 0).unwrap();
        assert!(activate_registered_s3_editor(
            &core, &vault, &profile_db, &custody_db, &scope,
        ).is_err(), "live editor admission must refuse activation");

        // Even while another thread owns the primary File Gate, a live
        // DocumentHub session must cause immediate refusal. Acquiring Gate
        // BEFORE checking sessions creates an ABBA deadlock with save_entry.
        let gate_held_with_editor = core.file_mutation_guard().unwrap();
        let (blocked_tx, blocked_rx) = std::sync::mpsc::channel();
        let other_core = Arc::clone(&core);
        let root_arg = vault.clone();
        let profile_arg = profile_db.clone();
        let custody_arg = custody_db.clone();
        let scope_arg = scope.clone();
        let rejected = std::thread::spawn(move || {
            let reply = activate_registered_s3_editor(
                &other_core, &root_arg, &profile_arg, &custody_arg, &scope_arg,
            );
            blocked_tx.send(reply.is_err()).unwrap();
        });
        assert!(blocked_rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
            "activation waited on File Gate while holding a live editor");
        rejected.join().unwrap();
        drop(gate_held_with_editor);
        core.documents.release(&core, &note);

        // Real activation must wait until an already-entered, ordinary File
        // Gate critical section has fully released PROCESS_FILE_MUTATIONS.
        // A concurrent opt-in cannot flip the participant flag mid-primary
        // write or install the guarded route before the in-flight gate exits.
        let in_flight_primary = core.file_mutation_guard().unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let concurrent_core = Arc::clone(&core);
        let concurrent_root = vault.clone();
        let concurrent_profile = profile_db.clone();
        let concurrent_custody = custody_db.clone();
        let concurrent_scope = scope.clone();
        let enrollment = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let enrolled = activate_registered_s3_editor(
                &concurrent_core, &concurrent_root, &concurrent_profile,
                &concurrent_custody, &concurrent_scope,
            );
            finished_tx.send(enrolled).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(finished_rx.recv_timeout(std::time::Duration::from_millis(60)).is_err(),
            "activation passed while an ordinary File Gate operation owned the mutex");
        assert!(!core.documents.requires_guarded_participant_hold(),
            "enrollment exposed participant HOLD before its Gate barrier");
        drop(in_flight_primary);
        let activated = finished_rx.recv_timeout(std::time::Duration::from_secs(20))
            .expect("activation could not finish after releasing the existing File Gate");
        assert!(activated.is_ok(), "enrollment after Gate quiescence: {activated:?}");
        enrollment.join().unwrap();
        assert!(core.documents.requires_guarded_participant_hold());

        // The external writer still works through its one-shot + persisted
        // intent. Direct history labels and retention are denied under HOLD.
        assert!(core.history.maintain_retention(&vault, 30).is_err());

        let first = "first guarded edit ".repeat(90);
        let hash = blake3::hash(b"original").to_hex().to_string();
        let without_intent = gate::write(
            &core, &note, &first, Some(&hash), Source::Me, None,
        );
        assert!(matches!(without_intent, Err(FileCommandError::Task { .. })),
            "native/MCP File Gate must not bypass the durable prewrite journal");
        assert_eq!(fs::read(&note).unwrap(), b"original");

        let mismatch_hash = blake3::hash(b"mismatch original").to_hex().to_string();
        core.documents.test_seed_guarded_intent(
            &core, &mismatch_note, &mismatch_hash, "allowed content",
        );
        let wrong_attempt = gate::write(
            &core, &mismatch_note, "other text", Some(&mismatch_hash), Source::Me, None,
        );
        assert!(matches!(wrong_attempt, Err(FileCommandError::Task { .. })),
            "a one-shot ticket for different content cannot authorize mutation");
        assert_eq!(fs::read(&mismatch_note).unwrap(), b"mismatch original");

        core.documents.test_seed_guarded_intent(&core, &note, &hash, &first);
        let result = gate::write(&core, &note, &first, Some(&hash), Source::Me, None);
        assert!(result.is_ok(), "guarded note write with exact intent: {result:?}");
        assert_eq!(fs::read_to_string(&note).unwrap(), first);
        assert!(core.history.page(&|| vec![vault.clone()], &note, 0, 8).total > 0);
        // In production DocumentSession removes this exact ticket only AFTER
        // HistoryService and durable sync-point confirmation.
        core.documents.test_confirm_guarded_intent(&note, &hash, &first);

        let current = blake3::hash(first.as_bytes()).to_hex().to_string();
        core.history.fail_once(HistoryFault::CreateVersion);
        let second = "second guarded edit ".repeat(90);
        core.documents.test_seed_guarded_intent(&core, &note, &current, &second);
        let ambiguous = gate::write(
            &core, &note, &second, Some(&current), Source::Me, None,
        );
        assert!(matches!(ambiguous, Err(FileCommandError::NeedsReview { .. })),
            "history failure must never acknowledge an uncertain writer: {ambiguous:?}");
        assert_eq!(fs::read_to_string(&note).unwrap(), second);
        // Its durable SQLite intent is still pending. The consumed ephemeral
        // permit must NOT authorize any replay, even with identical digests.
        let stale = gate::write(&core, &note, &second, Some(&current), Source::Me, None);
        assert!(matches!(stale, Err(FileCommandError::Task { .. })),
            "stale SQLite intent after uncertain result must not be reusable");

        // A changed inode under the same lock name cannot turn a registered
        // editor write into an ordinary unguarded File Gate write.
        let old = scope.join("retained-old-lock");
        fs::rename(&lock, &old).unwrap();
        fs::write(&lock, b"replaced lock").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let lock_hash = blake3::hash(b"lock original").to_hex().to_string();
        core.documents.test_seed_guarded_intent(&core, &lock_note, &lock_hash, "lock change");
        let denied = gate::write(
            &core, &lock_note, "lock change", Some(&lock_hash), Source::Me, None,
        );
        assert!(matches!(denied, Err(FileCommandError::Task { .. })));
        assert_eq!(fs::read(&lock_note).unwrap(), b"lock original");
        assert_eq!(fs::read(&old).unwrap(), b"stable existing OS lock");
    }
}
