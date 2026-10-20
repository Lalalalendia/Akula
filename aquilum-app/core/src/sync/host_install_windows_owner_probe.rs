//! RN-4E1b-03b0: read-only Windows installer-owned registry preflight.
//!
//! A host SQLite file with valid JSON, BLAKE3 and physical IDs is not a
//! source of authority when an ordinary user may rewrite the DB or WAL.
//! This isolated opt-in probe requires SYSTEM/Administrators owner and no
//! generic/namespace write ACE for a normal user, on BOTH preexisting parent
//! chains and the DB + independent witness file.
//!
//! This does not start a service, change ACLs, issue credentials, install a
//! privileged broker or activate P2P. No success is write authorization:
//! current-user admin elevation, privileged hostile processes, filesystem
//! transactions, concurrent substitutions and signed epoch receipts still
//! need explicit separate handling.

#![cfg(windows)]

use super::filegate_lock_probe::PinnedProbeRoot;
use super::filegate_windows_acl;
use super::host_custody_registry::HostCustodyError;
use super::host_ledger_identity::checked_member;
use std::fs;
use std::path::Path;

/// Bounded, read-only negative signal. Must be called BEFORE opening the
/// SQLite connection; never open and close another Unix SQLite DB descriptor
/// while its fcntl writer lock is held. Windows is the only enabled platform.
#[allow(dead_code)]
pub(crate) fn preflight_installer_owned_host_namespace(
    db_path: &Path,
    witness_path: &Path,
) -> Result<(), HostCustodyError> {
    if !db_path.is_absolute() || !witness_path.is_absolute()
        || db_path == witness_path
    {
        return Err(HostCustodyError::InvalidHostLocation);
    }
    let db_parent = db_path.parent().ok_or(HostCustodyError::InvalidHostLocation)?;
    let witness_parent = witness_path.parent()
        .ok_or(HostCustodyError::InvalidHostLocation)?;
    let db_canonical = fs::canonicalize(db_parent)?;
    let witness_canonical = fs::canonicalize(witness_parent)?;
    if db_canonical.starts_with(&witness_canonical)
        || witness_canonical.starts_with(&db_canonical)
    {
        // "Independent" evidence located in the same writeable DB subtree
        // is circular and must never bootstrap an installer identity.
        return Err(HostCustodyError::InvalidHostLocation);
    }

    for parent in [db_parent, witness_parent] {
        // Audit every ancestor, including the filesystem root, on an opened
        // pinned NT handle. Read-only: do not create or repair any directory.
        // Reparse/symlink leaves are rejected by PinnedProbeRoot.
        for ancestor in parent.ancestors() {
            let pinned = PinnedProbeRoot::open(ancestor)
                .map_err(HostCustodyError::Io)?;
            pinned.audit_windows_installer_acl()
                .map_err(HostCustodyError::Io)?;
            pinned.recheck_root().map_err(HostCustodyError::Io)?;
        }
    }

    for path in [db_path, witness_path] {
        let (file, _) = checked_member(path)?
            .ok_or(HostCustodyError::InvalidHostLocation)?;
        filegate_windows_acl::audit_installer_owner_dacl(&file)
            .map_err(|_| HostCustodyError::InvalidHostLocation)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::host_custody_registry::SharedHostCustodyRegistry;
    use std::fs;

    #[test]
    fn fake_host_registry_and_witness_are_not_installer_authority() {
        let root = tempfile::tempdir().unwrap();
        let db_dir = root.path().join("untrusted-host");
        let witness_dir = root.path().join("untrusted-witness");
        fs::create_dir_all(&db_dir).unwrap();
        fs::create_dir_all(&witness_dir).unwrap();
        let db = db_dir.join("approved.sqlite3");
        let witness = witness_dir.join("receipt.json");
        // A genuinely valid, cold-reopenable host registry is still not
        // installed-host trust when the current user can provision it.
        drop(SharedHostCustodyRegistry::provision_new(&db, &[]).unwrap());
        assert!(SharedHostCustodyRegistry::open_existing(&db).is_ok());
        let db_bytes = fs::read(&db).unwrap();
        fs::write(&witness, b"well formed looking receipt").unwrap();
        // This OS principal can create and modify both inputs. Structurally
        // valid files do not become installer-owned simply by naming them.
        assert!(preflight_installer_owned_host_namespace(&db, &witness).is_err());
        assert_eq!(fs::read(&db).unwrap(), db_bytes);
        assert!(SharedHostCustodyRegistry::open_existing(&db).is_ok());
        assert_eq!(fs::read(&witness).unwrap(),
                   b"well formed looking receipt");
    }

    #[test]
    fn absent_and_same_namespace_proofs_fail_closed_without_bootstrap() {
        let root = tempfile::tempdir().unwrap();
        let host = root.path().join("host");
        fs::create_dir(&host).unwrap();
        let db = host.join("host.sqlite3");
        let witness = host.join("witness.json");
        assert!(preflight_installer_owned_host_namespace(&db, &witness).is_err());
        fs::write(&db, b"no authority").unwrap();
        fs::write(&witness, b"no authority").unwrap();
        assert!(preflight_installer_owned_host_namespace(&db, &witness).is_err());
        assert!(preflight_installer_owned_host_namespace(&db, &db).is_err());
        assert!(!host.join("new-privileged-identity.sqlite3").exists());
    }

    #[test]
    fn relative_user_paths_cannot_be_host_installer_evidence() {
        assert!(preflight_installer_owned_host_namespace(
            Path::new("private-host.sqlite3"),
            Path::new("private-witness.json"),
        ).is_err());
    }
}
