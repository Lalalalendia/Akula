//! Non-authorizing inspection of a shared, existing OS lock OUTSIDE vaults.
//!
//! The caller must provide an external custody scope. A private owner-only
//! Unix mode is a useful negative check, but NOT protection from same-UID
//! processes. Windows now inspects owner/DACL on live handles, but this is
//! not a complete ACL/effective-access or immutable-custody proof.
//! Neither this module nor its callers may enable File Gate writes.

use super::filegate_lock_probe::PinnedProbeRoot;
use super::VaultPortError;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum CustodyPermissionEvidence {
    /// Unix owner matches effective UID and directory/file have 0700/0600
    /// modes. ACL and malicious same-user replacement are NOT ruled out.
    UnixPrivateModeOnly,
    /// Windows handle owner equals current token; present DACL was walked
    /// and non-owner / non-SYSTEM / non-Administrators write grants rejected.
    /// This is a limited negative audit, NOT verified immutable custody.
    WindowsOwnerDaclRestricted,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) struct CustodyObservation {
    pub(crate) registered_root: (u64, u64),
    pub(crate) custody_scope: (u64, u64),
    pub(crate) lock_object: (u64, u64),
    pub(crate) permission_evidence: CustodyPermissionEvidence,
}

fn reject() -> VaultPortError {
    VaultPortError::PermissionDenied
}

fn io(error: impl std::fmt::Display) -> VaultPortError {
    VaultPortError::Io(error.to_string())
}

/// The local UI workspace UUID is NOT shared between independent Core DBs.
/// The physical root ID is a deterministic negative-scoping key, NOT a
/// persistent authenticated VaultId. Inode recycling and drive migration
/// require a separately persisted custody contract.
pub(crate) fn scoped_leaf(root_id: (u64, u64)) -> String {
    format!("vault-{:016x}-{:016x}", root_id.0, root_id.1)
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// Refuse direct symlinks or Windows junction/reparse points in every
/// ancestor of the candidate. This is a PRECONDITION check on a canonical,
/// absolute host path, NOT a race-free namespace pin (ancestor replacement
/// by a hostile same-user actor remains possible immediately afterward).
pub(super) fn require_no_alias_ancestors(scope: &Path) -> Result<(), VaultPortError> {
    if !scope.is_absolute() {
        return Err(reject());
    }
    for ancestor in scope.ancestors() {
        let meta = fs::symlink_metadata(ancestor).map_err(io)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(reject());
        }
        #[cfg(windows)]
        if meta.file_attributes() & 0x0000_0400 != 0 {
            // The reparse flag also catches directory junction ancestors.
            return Err(reject());
        }
    }
    Ok(())
}

pub(super) fn require_outside_registered_roots(
    scope: &Path,
    registered_roots: &[PathBuf],
) -> Result<(), VaultPortError> {
    let scope = fs::canonicalize(scope).map_err(io)?;
    for registered in registered_roots {
        // An unavailable registry entry is not an invitation to ignore it:
        // fail closed until the host can establish the workspace boundaries.
        let root = fs::canonicalize(registered).map_err(io)?;
        if overlaps(&scope, &root) {
            return Err(reject());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn require_private_unix(
    metadata: &fs::Metadata,
    expected_mode: u32,
) -> Result<(), VaultPortError> {
    let actual_mode = metadata.permissions().mode() & 0o7777;
    if metadata.uid() != rustix::process::geteuid().as_raw() || actual_mode != expected_mode {
        return Err(reject());
    }
    Ok(())
}

/// Observe ONE pre-existing lock in an explicitly selected external scope.
/// Never create, delete, mutate or repair a lock file; the OS lock is dropped
/// before returning. Only an independent, trusted host could establish ACL,
/// stable VaultId and common custody-root authenticity for real writers.
#[allow(dead_code)]
pub(crate) fn inspect_external_scope(
    selected_root: &Path,
    expected_root_id: (u64, u64),
    current_registered_roots: impl Fn() -> Result<Vec<PathBuf>, VaultPortError>,
    candidate_scope: &Path,
    recheck_registration: impl Fn() -> Result<(), VaultPortError>,
) -> Result<CustodyObservation, VaultPortError> {
    recheck_registration()?;
    require_no_alias_ancestors(candidate_scope)?;
    let selected = fs::symlink_metadata(candidate_scope).map_err(io)?;
    if selected.file_type().is_symlink() || !selected.is_dir() {
        return Err(reject());
    }
    let expected_leaf = scoped_leaf(expected_root_id);
    if candidate_scope.file_name().and_then(|name| name.to_str())
        != Some(expected_leaf.as_str())
    {
        return Err(reject());
    }
    require_outside_registered_roots(candidate_scope, &current_registered_roots()?)?;

    let workspace = PinnedProbeRoot::open(selected_root).map_err(io)?;
    if workspace.identity() != expected_root_id {
        return Err(VaultPortError::FileChanged);
    }
    let external = PinnedProbeRoot::open(candidate_scope).map_err(io)?;
    if external.identity() == expected_root_id {
        return Err(reject());
    }

    #[cfg(unix)]
    require_private_unix(&external.opened_root_metadata().map_err(io)?, 0o700)?;
    #[cfg(windows)]
    external.audit_windows_acl().map_err(io)?;

    let held = external.try_lock().map_err(io)?.ok_or(VaultPortError::PermissionDenied)?;

    #[cfg(unix)]
    require_private_unix(&held.opened_file_metadata().map_err(io)?, 0o600)?;
    #[cfg(windows)]
    held.audit_windows_acl().map_err(io)?;

    // The caller's SQLite registration and opened physical workspace must
    // still describe the same root while this external OS lock is held.
    recheck_registration()?;
    workspace.recheck_root().map_err(io)?;
    external.recheck_root().map_err(io)?;
    if workspace.identity() != expected_root_id {
        return Err(VaultPortError::FileChanged);
    }
    require_no_alias_ancestors(candidate_scope)?;
    // A fresh SQLite registry snapshot closes the prior stale-list gap.
    require_outside_registered_roots(candidate_scope, &current_registered_roots()?)?;
    #[cfg(unix)]
    {
        require_private_unix(&external.opened_root_metadata().map_err(io)?, 0o700)?;
        require_private_unix(&held.opened_file_metadata().map_err(io)?, 0o600)?;
    }
    #[cfg(windows)]
    {
        external.audit_windows_acl().map_err(io)?;
        held.audit_windows_acl().map_err(io)?;
    }

    #[cfg(unix)]
    let permission_evidence = CustodyPermissionEvidence::UnixPrivateModeOnly;
    #[cfg(windows)]
    let permission_evidence = CustodyPermissionEvidence::WindowsOwnerDaclRestricted;
    #[cfg(not(any(unix, windows)))]
    return Err(reject());

    Ok(CustodyObservation {
        registered_root: expected_root_id,
        custody_scope: external.identity(),
        lock_object: held.identity(),
        permission_evidence,
    })
}
