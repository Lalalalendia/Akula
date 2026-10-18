//! P0-B6b-2a: read-only, Unix root-owned install receipt admission.
//! This is an additional opt-in diagnostic contract, NOT a privileged broker.
//! It protects the receipt pathname against cooperative unprivileged rename,
//! but the SQLite ledger is still writable by the normal desktop principal.
//! This does NOT defend the SQLite row content against a hostile same-UID
//! process, and must not enable S3 mutation or a public protected release.

use super::host_custody_registry::HostCustodyError;
use std::path::Path;

/// Fail closed on every platform that has not been independently implemented.
/// Unix: every ancestor, including '/', must be an actual root-owned
/// directory without group/other write permission, and the witness must be a
/// root-owned, non-hardlinked, regular file with no group/other write bit.
/// Reject any lexical alias rather than allowing /tmp, user HOME, /var
/// symlinks or user-controlled intermediate directories to satisfy this.
/// Windows always returns HOLD until a tested OS-owned DACL/broker exists.
pub(crate) fn require_root_owned_witness_path(
    witness_path: &Path,
) -> Result<(), HostCustodyError> {
    #[cfg(unix)]
    {
        use std::fs;
        use std::os::unix::fs::MetadataExt;

        if !witness_path.is_absolute()
            || fs::canonicalize(witness_path)? != witness_path
        {
            return Err(HostCustodyError::InvalidHostLocation);
        }
        for (index, member) in witness_path.ancestors().enumerate() {
            let meta = fs::symlink_metadata(member)?;
            if meta.file_type().is_symlink() || meta.uid() != 0
                || meta.mode() & 0o022 != 0
            {
                return Err(HostCustodyError::InvalidHostLocation);
            }
            if index == 0 {
                if !meta.is_file() || meta.nlink() != 1 {
                    return Err(HostCustodyError::InvalidHostLocation);
                }
            } else if !meta.is_dir() {
                return Err(HostCustodyError::InvalidHostLocation);
            }
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = witness_path;
        Err(HostCustodyError::InvalidHostLocation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    const CHILD: &str =
        "sync::host_install_unix_root::tests::root_owned_receipt_second_process";

    #[test]
    fn root_owner_preflight_rejects_untrusted_user_namespace() {
        let temp = tempfile::tempdir().unwrap();
        let receipt = temp.path().join("receipt.json");
        fs::write(&receipt, b"even a valid signed-looking receipt is not authority").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::{symlink, PermissionsExt};
            fs::set_permissions(&receipt, fs::Permissions::from_mode(0o444)).unwrap();
            // chmod alone never establishes a different OS security principal.
            assert!(require_root_owned_witness_path(&receipt).is_err());
            let alias = temp.path().join("receipt-link.json");
            symlink(&receipt, &alias).unwrap();
            assert!(require_root_owned_witness_path(&alias).is_err());
            // A writable ancestor is unsafe even if the leaf is read-only.
            assert!(require_root_owned_witness_path(&temp.path().join("missing")).is_err());
        }
        #[cfg(not(unix))]
        assert!(require_root_owned_witness_path(&receipt).is_err());
    }

    #[test]
    fn valid_sqlite_and_user_issued_receipt_are_still_hold() {
        use crate::sync::host_custody_registry::SharedHostCustodyRegistry;
        use crate::sync::host_install_witness::{
            create_test_witness_once, read_installer_witness,
        };

        let temp = tempfile::tempdir().unwrap();
        let host = temp.path().join("host-ledger");
        let anchor = temp.path().join("separate-anchor");
        fs::create_dir(&host).unwrap();
        fs::create_dir(&anchor).unwrap();
        let db = host.join("legitimate.sqlite3");
        let receipt = anchor.join("well-formed.json");
        drop(SharedHostCustodyRegistry::provision_new(&db, &[]).unwrap());
        create_test_witness_once(&db, &receipt).unwrap();
        assert!(SharedHostCustodyRegistry::open_existing(&db).is_ok(),
            "test must use a valid, existing SQLite ledger");
        assert!(receipt.is_file());
        assert!(read_installer_witness(&db, &receipt).is_err(),
            "the correct schema, path and physical IDs cannot replace OS-owned authority");
        // No bootstrap, deletion, repair or alteration of either input.
        assert!(SharedHostCustodyRegistry::open_existing(&db).is_ok());
        assert!(receipt.is_file());
    }

    #[test]
    fn root_owned_receipt_second_process() {
        if std::env::var_os("AKULA_B6B2_CHILD").is_none() { return; }
        let path = PathBuf::from(
            std::env::var_os("AKULA_B6B2_ROOT_RECEIPT").expect("CI-issued Unix root fixture")
        );
        #[cfg(unix)]
        assert!(require_root_owned_witness_path(&path).is_ok());
        #[cfg(not(unix))]
        assert!(require_root_owned_witness_path(&path).is_err());
    }

    #[test]
    fn root_owner_preflight_requires_real_os_issued_fixture_across_processes() {
        let Some(path) = std::env::var_os("AKULA_B6B2_ROOT_RECEIPT").map(PathBuf::from)
        else {
            // Ordinary developer runs cannot create root-owned trust fixtures.
            return;
        };
        #[cfg(unix)]
        assert!(require_root_owned_witness_path(&path).is_ok());
        #[cfg(not(unix))]
        assert!(require_root_owned_witness_path(&path).is_err());
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact").arg(CHILD).arg("--nocapture")
            .env("AKULA_B6B2_CHILD", "1")
            .env("AKULA_B6B2_ROOT_RECEIPT", &path)
            .output().unwrap();
        assert!(output.status.success(), "second OS process failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout)
            .contains("root_owned_receipt_second_process ... ok"),
            "expected actual independent child execution");
    }
}
