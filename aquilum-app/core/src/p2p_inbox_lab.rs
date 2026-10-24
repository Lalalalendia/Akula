//! Explicit receiver "Save as NEW file" from a pinned, on-disk verified stage.
//!
//! This optional manual P2P inbox path is deliberately NOT Akula Vault sync,
//! File Gate application or an ACK. The caller must independently trust the
//! expected root and size from an approved OfferSession. This code checks the
//! actual opened stage bytes again and NEVER replaces an existing destination.
//! No server, transport, UI action or unattended write is started here.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

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

pub const MAX_INBOX_ITEM_BYTES: u64 = 8 * 1024 * 1024;
const COPY_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub enum InboxError {
    InvalidManifest,
    InvalidDestination,
    NotRegularStage,
    SizeMismatch,
    DigestMismatch,
    AlreadyExists,
    Io(io::Error),
}

impl From<io::Error> for InboxError {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::AlreadyExists {
            Self::AlreadyExists
        } else {
            Self::Io(error)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewInboxFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub blake3_root: [u8; 32],
}

/// Keep a failed/incomplete candidate from being mistaken for a saved file.
/// The independent destination name only becomes visible after a full digest
/// match and an atomic no-replace hard-link to the completely written candidate.
struct RemoveScratch(PathBuf);

impl Drop for RemoveScratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Save exactly one user-accepted, hash-pinned staged file at a user-selected
/// NEW path. The caller retains ownership of the opened `File` and MUST obtain
/// the expected root/size from independently authenticated Akula consent,
/// never from a remotely supplied display name or staged file itself.
///
/// * Reads from one already-opened regular-file handle; no second stage-path
///   lookup, no unbounded allocation (64 KiB scratch buffer).
/// * Does not publish a destination until the entire physical stage has been
///   copied, rehashed and synced to a private new sibling candidate.
/// * `hard_link` is a same-directory atomic create-NO-REPLACE publication;
///   existing files and symlinks cannot be overwritten, including under races.
/// * It refuses unsupported hard-link filesystems rather than falling back
///   to unsafe rename-overwrite.
/// * This is NOT a power-loss durable receipt, protected destination-dir
///   custody or a trusted Vault File Gate write. Never advertise APPLIED/ACK.
pub fn save_verified_stage_as_new(
    stage: &mut File,
    expected_bytes: u64,
    expected_root: [u8; 32],
    destination: &Path,
) -> Result<NewInboxFile, InboxError> {
    if expected_bytes == 0 || expected_bytes > MAX_INBOX_ITEM_BYTES || expected_root == [0; 32] {
        return Err(InboxError::InvalidManifest);
    }
    let Some(name) = destination.file_name() else {
        return Err(InboxError::InvalidDestination);
    };
    if name.is_empty() {
        return Err(InboxError::InvalidDestination);
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // A caller chooses the destination. Reject a symlinked immediate parent,
    // but do not claim that all ancestor path races are solved by this check.
    let parent_info = fs::symlink_metadata(parent)?;
    if !parent_info.is_dir() || parent_info.file_type().is_symlink() {
        return Err(InboxError::InvalidDestination);
    }
    // Catch existing destinations including dangling symlinks. hard_link below
    // enforces the same no-replace rule atomically if another writer races us.
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err(InboxError::AlreadyExists),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    let before = stage.metadata()?;
    if !before.is_file() {
        return Err(InboxError::NotRegularStage);
    }
    if before.len() != expected_bytes {
        return Err(InboxError::SizeMismatch);
    }
    stage.seek(SeekFrom::Start(0))?;
    let scratch_path = parent.join(format!(".akula-inbox-{}.part", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut scratch = options.open(&scratch_path)?;
    let _remove_if_incomplete = RemoveScratch(scratch_path.clone());

    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    let mut hasher = blake3::Hasher::new();
    let mut copied = 0u64;
    loop {
        let got = stage.read(&mut buffer)?;
        if got == 0 {
            break;
        }
        copied = copied
            .checked_add(got as u64)
            .ok_or(InboxError::SizeMismatch)?;
        if copied > expected_bytes {
            return Err(InboxError::SizeMismatch);
        }
        hasher.update(&buffer[..got]);
        scratch.write_all(&buffer[..got])?;
    }
    if copied != expected_bytes || stage.metadata()?.len() != expected_bytes {
        return Err(InboxError::SizeMismatch);
    }
    if *hasher.finalize().as_bytes() != expected_root {
        return Err(InboxError::DigestMismatch);
    }
    scratch.sync_all()?;
    drop(scratch);

    fs::hard_link(&scratch_path, destination).map_err(InboxError::from)?;
    Ok(NewInboxFile {
        path: destination.to_path_buf(),
        bytes: expected_bytes,
        blake3_root: expected_root,
    })
}

#[cfg(test)]
mod tests;
