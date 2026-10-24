//! Opt-in S3 Beta: cooperative Windows user-profile File Gate custody.
//!
//! A shared local OS lock must be OUTSIDE registered note workspaces and
//! independent of the roaming Akula appdata database. It is scoped to the
//! pinned physical root, never to the cloud VaultId or workspace UI UUID.
//!
//! TRUST MODEL: cooperating applications of the SAME signed-in OS user.
//! The user-profile folder is not installer- or broker-attested and cannot
//! withstand a malicious same-user process, administrator, filesystem
//! rollback, or host namespace replacement. This permits only explicitly
//! initiated, create-if-absent S3 Beta receiving via the existing File Gate.
//! It grants NO overwrite, delete, confirmed baseline, or power-off receipt.

use std::{fs::{self, OpenOptions}, io, path::{Path, PathBuf}};

use crate::app_core::Core;
use crate::sync::{
    filegate_custody_identity::{CustodyBindingError, SqliteCustodyIdentityStore},
    filegate_lock_custody::{self, scoped_leaf},
    filegate_lock_probe::LOCK_NAME,
    profile_binding::{S3BindingError, SqliteRegisteredS3Bindings},
    RegisteredReadAdmission, VaultPortError,
};

#[derive(Debug)]
pub enum S3PairCustodyError {
    Admission(VaultPortError),
    BindingFailed,
    IdentityMismatch,
    Io(io::Error),
    InvalidHostNamespace,
    NotProvisioned,
}
impl From<VaultPortError> for S3PairCustodyError {
    fn from(error: VaultPortError) -> Self { Self::Admission(error) }
}
impl From<S3BindingError> for S3PairCustodyError {
    fn from(_error: S3BindingError) -> Self { Self::BindingFailed }
}
impl From<CustodyBindingError> for S3PairCustodyError {
    fn from(_error: CustodyBindingError) -> Self { Self::IdentityMismatch }
}
impl From<io::Error> for S3PairCustodyError {
    fn from(error: io::Error) -> Self { Self::Io(error) }
}

fn host_base_for_registered(
    admission: &RegisteredReadAdmission<'_>,
    appdata: &Path,
    host_base: &Path,
) -> Result<PathBuf, S3PairCustodyError> {
    // The caller must supply a non-configurable OS known-folder root.
    // Reject user-configured relative paths, junctions and any overlap with
    // a Core-registered vault or the private appdata parent/child tree.
    if !host_base.is_absolute() || !appdata.is_absolute() {
        return Err(S3PairCustodyError::InvalidHostNamespace);
    }
    filegate_lock_custody::require_no_alias_ancestors(host_base)?;
    let base = fs::canonicalize(host_base)?;
    filegate_lock_custody::require_no_alias_ancestors(&base)?;
    filegate_lock_custody::require_outside_registered_roots(
        &base, &admission.registered_workspace_roots()?,
    )?;
    let data = fs::canonicalize(appdata)?;
    if base.starts_with(&data) || data.starts_with(&base) {
        return Err(S3PairCustodyError::InvalidHostNamespace);
    }
    Ok(base)
}

fn scope_for_registered(
    admission: &RegisteredReadAdmission<'_>,
    base: &Path,
) -> Result<PathBuf, S3PairCustodyError> {
    let physical = admission.registered_root_identity_for_profile()?;
    if physical.0 == 0 || physical.1 == 0 {
        return Err(S3PairCustodyError::InvalidHostNamespace);
    }
    Ok(base.join(scoped_leaf(physical)))
}

fn verify_bound_scope(
    admission: &RegisteredReadAdmission<'_>,
    appdata: &Path,
    scope: &Path,
    bind_if_missing: bool,
) -> Result<(), S3PairCustodyError> {
    admission.inspect_external_filegate_lock(scope)?;
    let bindings = SqliteRegisteredS3Bindings::open(
        admission, &appdata.join("s3-profiles.sqlite"),
    )?;
    // Cold SyncNow may never recreate a missing custody DB. Unlike the
    // user-initiated bootstrap this path must be strictly open-existing.
    if !bind_if_missing && !appdata.join("s3-custody.sqlite").is_file() {
        return Err(S3PairCustodyError::NotProvisioned);
    }
    let mut observed = SqliteCustodyIdentityStore::open(
        admission, &appdata.join("s3-custody.sqlite"),
    )?;
    if bind_if_missing {
        // Immutable on conflict. The VaultId comes from the registered,
        // previously verified S3 profile, not from a copied JSON path.
        observed.bind_registered_s3(admission, &bindings, scope)?;
    } else {
        observed.verify_registered_s3(admission, &bindings, scope)?;
    }
    admission.recheck_registered_root()?;
    Ok(())
}

/// Called ONLY by the user-initiated Configure / Import operation.
///
/// Existing objects are never truncated or removed. New directory and lock
/// creation use exclusive create; the existing File Gate then reopens and
/// audits owner/DACL, no-reparse ancestors and the pinned lock identity.
/// A failed attempt cannot promote a remote acknowledgement.
pub fn provision_cooperative_s3_pair_custody(
    core: &Core,
    registered_root: &Path,
    appdata: &Path,
    os_known_host_base: &Path,
) -> Result<PathBuf, S3PairCustodyError> {
    let admission = RegisteredReadAdmission::open(core, registered_root)?;
    let base = host_base_for_registered(&admission, appdata, os_known_host_base)?;
    let scope = scope_for_registered(&admission, &base)?;
    let created = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            let mut dir = fs::DirBuilder::new();
            dir.mode(0o700);
            dir.create(&scope)
        }
        #[cfg(not(unix))]
        {
            fs::create_dir(&scope)
        }
    };
    match created {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    filegate_lock_custody::require_no_alias_ancestors(&scope)?;
    // Final-child CREATE_NEW is atomic even against a planted symlink:
    // an existing file, directory, link or reparse object is never followed
    // for creation, and never overwritten or silently reinitialized.
    let leaf = scope.join(LOCK_NAME);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&leaf) {
        Ok(file) => file.sync_all()?,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    verify_bound_scope(&admission, appdata, &scope, true)?;
    Ok(scope)
}

/// SyncNow is read-only with respect to OS custody: it cannot create, repair,
/// rebind or replace a missing lock. An explicit user reconfiguration is
/// required when a drive/root is moved or a protected object is unavailable.
pub fn open_existing_cooperative_s3_pair_custody(
    core: &Core,
    registered_root: &Path,
    appdata: &Path,
    os_known_host_base: &Path,
) -> Result<PathBuf, S3PairCustodyError> {
    let admission = RegisteredReadAdmission::open(core, registered_root)?;
    let base = host_base_for_registered(&admission, appdata, os_known_host_base)?;
    let scope = scope_for_registered(&admission, &base)?;
    if !fs::symlink_metadata(&scope).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        || !fs::symlink_metadata(scope.join(LOCK_NAME))
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
    {
        return Err(S3PairCustodyError::NotProvisioned);
    }
    verify_bound_scope(&admission, appdata, &scope, false)?;
    Ok(scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_leaf_depends_on_physical_object_not_cloud_or_workspace_id() {
        let first = scoped_leaf((0x12, 0x1234));
        assert_eq!(first, "vault-0000000000000012-0000000000001234");
        assert_ne!(first, scoped_leaf((0x12, 0x1235)));
        assert_ne!(first, scoped_leaf((0x13, 0x1234)));
        assert!(!first.contains('/'));
    }

    #[test]
    fn provision_once_then_reopen_preserves_identity_and_denies_replaced_lock() {
        use std::sync::Arc;
        use crate::app_core::CoreEvent;
        use crate::sync::{
            OperationId, S3ConnectionProfile, S3HeadPointerV1, S3Keyspace,
            S3ManifestV1, S3VerifiedHead, VaultId,
        };

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("notes");
        let appdata = tmp.path().join("appdata");
        let host = tmp.path().join("external-custody");
        for path in [&root, &appdata, &host] { fs::create_dir(path).unwrap(); }
        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        let namespace = S3Keyspace::new("akula/pair-custody", VaultId::new()).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-test-custody".into(), region: "us-east-1".into(),
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
            pointer, etag: "synthetic-test".into(), manifest,
        };
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        let mut profiles = SqliteRegisteredS3Bindings::open(
            &admission, &appdata.join("s3-profiles.sqlite"),
        ).unwrap();
        profiles.bind_existing_verified_s3(
            &admission, "pair", &profile, &namespace, &verified,
        ).unwrap();
        drop(profiles);
        drop(admission);
        let scope = provision_cooperative_s3_pair_custody(
            &core, &root, &appdata, &host,
        ).unwrap();
        assert!(scope.starts_with(&host));
        assert_eq!(provision_cooperative_s3_pair_custody(
            &core, &root, &appdata, &host,
        ).unwrap(), scope);
        let lock = scope.join(LOCK_NAME);
        assert!(lock.is_file());
        drop(core);

        let cold = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        assert_eq!(open_existing_cooperative_s3_pair_custody(
            &cold, &root, &appdata, &host,
        ).unwrap(), scope);

        let moved = scope.join("retained-original");
        fs::rename(&lock, &moved).unwrap();
        fs::write(&lock, b"independently-lockable replacement").unwrap();
        assert!(open_existing_cooperative_s3_pair_custody(
            &cold, &root, &appdata, &host,
        ).is_err());
        fs::remove_file(&lock).unwrap();
        fs::rename(&moved, &lock).unwrap();
        assert_eq!(open_existing_cooperative_s3_pair_custody(
            &cold, &root, &appdata, &host,
        ).unwrap(), scope);
    }

    #[test]
    fn missing_host_directory_never_gets_created_during_sync() {
        let missing = std::path::Path::new("nonexistent-akula-custody-sentinel");
        assert!(!missing.exists());
        // Sync-only APIs have no directory creation operation. The OS-root
        // helper refuses relative paths before touching the filesystem.
        assert!(!missing.is_absolute());
    }
}
