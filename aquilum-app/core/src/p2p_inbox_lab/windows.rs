//! Windows P2P Inbox handle custody. This is not a Vault File Gate writer.
//! Export pins the complete directory chain, but SQLite WAL and candidate
//! namespace integrity still require independent release acceptance.
use super::InboxError;
use std::{fs::{File, OpenOptions}, os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path}};

/// Windows-only private staging. Returns the directory handle as well as
/// the exclusively created file; retain both through the receiver operation.
/// Audits owner/DACL on an independently opened root and compares live NT
/// identities before creating a child relative to the writable directory handle.
/// The later SQLite/object publication is still path-based and NOT certified.
#[cfg(windows)]
pub fn create_pinned_windows_inbox_stage(
    root: &Path,
    name: &str,
) -> Result<(File, File), InboxError> {
    crate::sync::p2p_stage::create_pinned_p2p_inbox_stage(root, name)
        .map_err(|_| InboxError::InvalidDestination)
}

/// Keep root, objects and SQLite HANDLEs live for the entire read/copy.
/// This only rejects observed ACL, reparse and identity anomalies; it does
/// not yet certify SQLite WAL namespace or same-user race immunity.
#[cfg(windows)]
pub struct PinnedWindowsInboxRead {
    _root: File,
    _objects: File,
    _sqlite: File,
}

#[cfg(windows)]
pub fn pin_windows_inbox_read(root: &Path) -> Result<PinnedWindowsInboxRead, InboxError> {
    let (root, objects, sqlite) = crate::sync::p2p_stage::pin_p2p_read_snapshot(root)
        .map_err(|_| InboxError::InvalidDestination)?;
    Ok(PinnedWindowsInboxRead {
        _root: root,
        _objects: objects,
        _sqlite: sqlite,
    })
}


/// Pin every absolute destination ancestor without FILE_SHARE_DELETE.
/// The target folder cannot be renamed out from under this guard; reject
/// directory junctions/reparse points before allowing an Inbox export.
/// This is only a directory gate, not an ACK or full source/output-file seal.
pub struct PinnedWindowsExportParent {
    _chain: Vec<File>,
}

pub fn pin_windows_export_parent(parent: &Path) -> Result<PinnedWindowsExportParent, InboxError> {
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

    if !parent.is_absolute()
        || parent.components().any(|part| matches!(part, Component::ParentDir))
    {
        return Err(InboxError::InvalidDestination);
    }
    let mut ancestors = parent.ancestors()
        .filter(|path| !path.as_os_str().is_empty()).collect::<Vec<_>>();
    if ancestors.is_empty() || ancestors.len() > 128 {
        return Err(InboxError::InvalidDestination);
    }
    ancestors.reverse();
    let mut chain = Vec::with_capacity(ancestors.len());
    for directory in ancestors {
        let mut options = OpenOptions::new();
        options.read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
        let opened = options.open(directory)?;
        let metadata = opened.metadata()?;
        if !metadata.is_dir()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(InboxError::InvalidDestination);
        }
        chain.push(opened);
    }
    Ok(PinnedWindowsExportParent { _chain: chain })
}
