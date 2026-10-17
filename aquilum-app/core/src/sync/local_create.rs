//! SYNC-04g-M1: internal, additive-only local publication of staged bytes.
//!
//! Deliberately NOT a VaultPort executor. Callers must hold Core admission,
//! registry checks and the shared File Gate; this module only makes a new
//! name appear atomically, without replacing any existing name. A return
//! value is NOT a durable remote baseline or permission to delete/overwrite.
//! Temporary files are tied to the operation UUID; ambiguous failures leave
//! them for inspection rather than risk deleting an externally swapped file.

use super::{OperationId, VaultPortError};
use super::pinned_read::PinnedFileIdentity;
use std::fs::File;
use std::io::{Read, Seek, Write};

#[cfg(unix)]
use rustix::fs::{openat, Mode, OFlags};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UnconfirmedLocalCreate {
    pub file: PinnedFileIdentity,
    pub byte_length: u64,
}

fn identity(file: &File) -> Result<PinnedFileIdentity, VaultPortError> {
    let metadata = file.metadata()
        .map_err(|error| VaultPortError::Io(error.to_string()))?;
    if !metadata.is_file() {
        return Err(VaultPortError::UnsafePath);
    }
    #[cfg(unix)]
    {
        if metadata.ino() == 0 || metadata.nlink() != 1 {
            return Err(VaultPortError::UnsafePath);
        }
        Ok(PinnedFileIdentity { device: metadata.dev(), file: metadata.ino() })
    }
    #[cfg(windows)]
    {
        let (device, id, links) = super::pinned_windows::file_identity_with_links(file)?;
        if links != 1 {
            return Err(VaultPortError::UnsafePath);
        }
        Ok(PinnedFileIdentity { device: u64::from(device), file: id })
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(VaultPortError::PermissionDenied)
    }
}

pub(super) fn directory_identity(parent: &File) -> Result<PinnedFileIdentity, VaultPortError> {
    let meta = parent.metadata().map_err(|error| VaultPortError::Io(error.to_string()))?;
    if !meta.is_dir() {
        return Err(VaultPortError::UnsafePath);
    }
    #[cfg(unix)]
    {
        Ok(PinnedFileIdentity { device: meta.dev(), file: meta.ino() })
    }
    #[cfg(windows)]
    {
        let (volume, id) = super::pinned_windows::file_identity(parent)?;
        Ok(PinnedFileIdentity { device: u64::from(volume), file: id })
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(VaultPortError::PermissionDenied)
    }
}

#[cfg(unix)]
fn stage_file(parent: &File, name: &str) -> Result<File, VaultPortError> {
    let flags = OFlags::RDWR | OFlags::CREATE | OFlags::EXCL
        | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    openat(parent, name, flags, Mode::RUSR | Mode::WUSR)
        .map(File::from)
        .map_err(|error| {
            if error == rustix::io::Errno::EXIST {
                VaultPortError::DestinationExists
            } else {
                VaultPortError::Io(error.to_string())
            }
        })
}

#[cfg(windows)]
fn stage_file(parent: &File, name: &str) -> Result<File, VaultPortError> {
    super::pinned_windows::create_stage_child(parent, name)
}

#[cfg(not(any(unix, windows)))]
fn stage_file(_parent: &File, _name: &str) -> Result<File, VaultPortError> {
    Err(VaultPortError::PermissionDenied)
}

#[cfg(target_os = "linux")]
fn publish_no_replace(parent: &File, stage: &File, temporary: &str, leaf: &str)
    -> Result<(), VaultPortError>
{
    use rustix::fs::{renameat_with, RenameFlags};
    let _ = stage;
    renameat_with(parent, temporary, parent, leaf, RenameFlags::NOREPLACE)
        .map_err(|error| {
            if error == rustix::io::Errno::EXIST {
                VaultPortError::DestinationExists
            } else {
                VaultPortError::Io(error.to_string())
            }
        })
}

#[cfg(target_os = "macos")]
fn publish_no_replace(parent: &File, stage: &File, temporary: &str, leaf: &str)
    -> Result<(), VaultPortError>
{
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    // RENAME_EXCL is 0x4 in XNU sys/stdio.h; no replacement on collision.
    #[link(name = "System")]
    extern "C" {
        fn renameatx_np(old_dirfd: i32, old_path: *const std::ffi::c_char,
                        new_dirfd: i32, new_path: *const std::ffi::c_char,
                        flags: u32) -> i32;
    }
    let _ = stage;
    let source = CString::new(temporary).map_err(|_| VaultPortError::UnsafePath)?;
    let destination = CString::new(leaf).map_err(|_| VaultPortError::UnsafePath)?;
    let code = unsafe {
        renameatx_np(parent.as_raw_fd(), source.as_ptr(), parent.as_raw_fd(),
                     destination.as_ptr(), 0x0000_0004)
    };
    if code == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        Err(VaultPortError::DestinationExists)
    } else {
        Err(VaultPortError::Io(error.to_string()))
    }
}

#[cfg(windows)]
fn publish_no_replace(parent: &File, stage: &File, _temporary: &str, leaf: &str)
    -> Result<(), VaultPortError>
{
    super::pinned_windows::rename_staged_no_replace(stage, parent, leaf)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn publish_no_replace(_parent: &File, _stage: &File, _temporary: &str, _leaf: &str)
    -> Result<(), VaultPortError>
{
    Err(VaultPortError::PermissionDenied)
}

fn open_published(parent: &File, leaf: &str) -> Result<File, VaultPortError> {
    #[cfg(unix)]
    {
        openat(parent, leaf, OFlags::RDONLY | OFlags::NOFOLLOW
               | OFlags::CLOEXEC | OFlags::NONBLOCK, Mode::empty())
            .map(File::from)
            .map_err(|error| VaultPortError::Io(error.to_string()))
    }
    #[cfg(windows)]
    {
        super::pinned_windows::open_child(parent, leaf, true)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (parent, leaf);
        Err(VaultPortError::PermissionDenied)
    }
}

/// Never call with a remote path or unverified remote bytes. The parent must
/// be an OS-handle-pinned directory opened by the registered Core host.
/// The hook supports deterministic fault/foreign-process tests. It runs only
/// after a fully written and synced stage, before final publication.
pub(super) fn create_with_hook(
    parent: &File,
    leaf: &str,
    operation: OperationId,
    bytes: &[u8],
    before_publish: impl FnOnce(),
) -> Result<UnconfirmedLocalCreate, VaultPortError> {
    const MAX_BYTES: usize = 32 * 1024 * 1024;
    if bytes.len() > MAX_BYTES || leaf.is_empty() || leaf == "." || leaf == ".."
        || leaf.chars().any(|ch| ch == '/' || ch == '\\' || ch == ':' || ch == '\0')
    {
        return Err(VaultPortError::UnsafePath);
    }

    // A deterministic stage name makes process-kill recovery explicit.
    // Never automatically remove a stage on failure: another process may
    // have replaced the temporary name, and unlink would become destructive.
    let temporary = format!(".akula-sync-stage-{}", operation);
    let mut staged = stage_file(parent, &temporary)?;
    staged.write_all(bytes).map_err(|error| VaultPortError::Io(error.to_string()))?;
    staged.sync_all().map_err(|error| VaultPortError::Io(error.to_string()))?;
    let staged_id = identity(&staged)?;
    staged.rewind().map_err(|error| VaultPortError::Io(error.to_string()))?;
    let mut check = Vec::new();
    (&mut staged).take(bytes.len() as u64 + 1).read_to_end(&mut check)
        .map_err(|error| VaultPortError::Io(error.to_string()))?;
    if check != bytes {
        return Err(VaultPortError::FileChanged);
    }
    // Need the original FD after checking; Read::take consumed only the &mut File.
    before_publish();
    publish_no_replace(parent, &staged, &temporary, leaf)?;

    // A mutation is *potentially* visible now. Even if verification fails,
    // do not delete the destination: another process could own it already.
    let mut opened = open_published(parent, leaf)?;
    if identity(&opened)? != staged_id {
        return Err(VaultPortError::FileChanged);
    }
    let mut observed = Vec::new();
    (&mut opened).take(bytes.len() as u64 + 1).read_to_end(&mut observed)
        .map_err(|error| VaultPortError::Io(error.to_string()))?;
    if observed != bytes || identity(&opened)? != staged_id {
        return Err(VaultPortError::FileChanged);
    }
    #[cfg(unix)]
    parent.sync_all().map_err(|error| VaultPortError::Io(error.to_string()))?;
    // Windows does not provide a generally reliable directory fsync through
    // this handle. Preserve the staged journal state until a stronger
    // platform-specific durable acknowledgment is implemented.
    Ok(UnconfirmedLocalCreate {
        file: staged_id,
        byte_length: bytes.len() as u64,
    })
}
