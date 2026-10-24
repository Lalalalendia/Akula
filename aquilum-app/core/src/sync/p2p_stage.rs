#![cfg(all(windows, feature = "p2p-inbox-lab"))]
// Handle-relative P2P stage inside Core's existing Windows sync custody boundary.
// The later SQLite/object publication still needs a separate custody check.
use super::{filegate_windows_acl, pinned_windows, VaultPortError};
use std::{
    fs::{File, OpenOptions},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

pub(crate) fn create_pinned_p2p_inbox_stage(
    root: &Path,
    name: &str,
) -> Result<(File, File), VaultPortError> {
    let inspected = pinned_windows::open_root(root)?;
    filegate_windows_acl::audit_owner_dacl(&inspected)?;
    let writable = pinned_windows::open_root_for_create(root)?;
    if pinned_windows::file_identity(&inspected)? != pinned_windows::file_identity(&writable)? {
        return Err(VaultPortError::FileChanged);
    }
    let stage = pinned_windows::create_stage_child(&writable, name)?;
    Ok((writable, stage))
}

/// Preserve live NT handles to the fixed receiver root, objects directory
/// and SQLite leaf while the caller reads or publishes. This is a negative
/// ACL/identity gate; SQLite's ambient pathname/WAL operations are NOT yet
/// handle-relative, so this is not a proof against hostile ancestor races.
pub(crate) fn pin_p2p_read_snapshot(root: &Path) -> Result<(File, File, File), VaultPortError> {
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const REPARSE_ATTRIBUTE: u32 = 0x0000_0400;

    let root_handle = pinned_windows::open_root(root)?;
    filegate_windows_acl::audit_owner_dacl(&root_handle)?;

    let objects = pinned_windows::open_child(&root_handle, "objects", false)?;
    let objects_direct = pinned_windows::open_root(&root.join("objects"))?;
    filegate_windows_acl::audit_owner_dacl(&objects_direct)?;
    if pinned_windows::file_identity(&objects)?
        != pinned_windows::file_identity(&objects_direct)?
    {
        return Err(VaultPortError::FileChanged);
    }

    let database = pinned_windows::open_child(&root_handle, "received-inbox.sqlite3", true)?;
    let mut options = OpenOptions::new();
    options.read(true).custom_flags(OPEN_REPARSE_POINT);
    let db_direct = options
        .open(root.join("received-inbox.sqlite3"))
        .map_err(|error| VaultPortError::Io(error.to_string()))?;
    let db_meta = db_direct
        .metadata()
        .map_err(|error| VaultPortError::Io(error.to_string()))?;
    if !db_meta.is_file() || db_meta.file_attributes() & REPARSE_ATTRIBUTE != 0 {
        return Err(VaultPortError::UnsafePath);
    }
    filegate_windows_acl::audit_owner_dacl(&db_direct)?;
    if pinned_windows::file_identity(&database)?
        != pinned_windows::file_identity(&db_direct)?
    {
        return Err(VaultPortError::FileChanged);
    }
    Ok((root_handle, objects, database))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn read_snapshot_pins_real_children_and_refuses_fake_sqlite_or_objects() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("received-inbox");
        fs::create_dir(&root).unwrap();
        let objects = root.join("objects");
        fs::create_dir(&objects).unwrap();
        let sqlite = root.join("received-inbox.sqlite3");
        fs::write(&sqlite, b"placeholder fixture").unwrap();

        // The gate checks object identity and ACL; SQL validity is separately
        // tested through the ledger with its own read-only SQLite connection.
        let pinned = pin_p2p_read_snapshot(&root);
        assert!(pinned.is_ok(), "safe receiver fixture: {:?}", pinned.err());
        drop(pinned);

        fs::remove_file(&sqlite).unwrap();
        fs::create_dir(&sqlite).unwrap();
        assert!(pin_p2p_read_snapshot(&root).is_err());
        fs::remove_dir(&sqlite).unwrap();
        fs::write(&sqlite, b"placeholder fixture").unwrap();

        fs::remove_dir(&objects).unwrap();
        fs::write(&objects, b"not a directory").unwrap();
        assert!(pin_p2p_read_snapshot(&root).is_err());
    }
}
