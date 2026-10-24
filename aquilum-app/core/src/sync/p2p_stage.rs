#![cfg(all(windows, feature = "p2p-inbox-lab"))]
// Handle-relative P2P stage inside Core's existing Windows sync custody boundary.
// The later SQLite/object publication still needs a separate custody check.
use super::{filegate_windows_acl, pinned_windows, VaultPortError};
use std::{fs::File, path::Path};

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
