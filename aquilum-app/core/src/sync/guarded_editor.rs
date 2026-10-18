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
    registry.push(config);
    Ok(())
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
    let selected = match selected(core, path) {
        Ok(None) => return None,
        Ok(Some(selected)) => selected,
        Err(error) => return Some(Err(error)),
    };
    Some((|| {
        let (config, relative) = selected;
        let expected = expected_hash.ok_or_else(|| {
            refused("cooperative editor save requires a disk-content CAS hash")
        })?;
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
    use crate::documents::io::HubIo;
    use crate::documents::session::{DiskError, SessionIo};
    use crate::history::service::HistoryFault;
    use std::sync::{Arc, Mutex};

    #[test]
    fn optin_actual_editor_io_uses_os_guard_and_never_falls_back_after_lock_swap() {
        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let data = temp.path().join("appdata");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&data).unwrap();
        let vault = crate::search::paths::canonical_path(&vault);
        let note = vault.join("Shared.md");
        fs::write(&note, b"original").unwrap();
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
        activate_registered_s3_editor(
            &core, &vault, &profile_db, &custody_db, &scope,
        ).unwrap();

        let store = Mutex::new(None);
        let editor = HubIo {
            core: &core, path: &note, key: "Shared.md", store: &store,
            offset_seconds: 0, source: Source::Me,
        };
        let first = "first guarded edit ".repeat(90);
        let hash = blake3::hash(b"original").to_hex().to_string();
        let result = SessionIo::write(&editor, &first, &hash);
        assert!(result.is_ok(), "editor guarded write: {result:?}");
        assert_eq!(fs::read_to_string(&note).unwrap(), first);
        assert!(core.history.page(&|| vec![vault.clone()], &note, 0, 8).total > 0);

        // Editor conflict/truncation preservation may not silently create an
        // unguarded file while this cooperative workspace is armed.
        assert!(matches!(
            SessionIo::preserve(
                &editor, "unsaved", crate::documents::conflict::ConflictReport {
                    cause: crate::documents::conflict::ConflictCause::Diverged,
                    disk_hash: None, synced_hash: None,
                }
            ),
            Err(DiskError::Other { code: "preservation_failed", .. })
        ));

        let current = blake3::hash(first.as_bytes()).to_hex().to_string();
        core.history.fail_once(HistoryFault::CreateVersion);
        let second = "second guarded edit ".repeat(90);
        let ambiguous = SessionIo::write(&editor, &second, &current);
        assert!(matches!(
            ambiguous, Err(DiskError::Other { code: "needs_review", .. })
        ), "history failure must not be acknowledged: {ambiguous:?}");
        assert_eq!(fs::read_to_string(&note).unwrap(), second);

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
        let latest = blake3::hash(second.as_bytes()).to_hex().to_string();
        let denied = SessionIo::write(&editor, "unapproved", &latest);
        assert!(denied.is_err());
        assert_eq!(fs::read_to_string(&note).unwrap(), second);
        assert_eq!(fs::read(&old).unwrap(), b"stable existing OS lock");
    }
}
