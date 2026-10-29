//! Windows NT directory-handle-relative reader for stage 04g-2.
//!
//! The Windows Win32 CreateFile API has no root-directory-handle parameter.
//! This narrow, read-only boundary uses the NT file-open API for each SINGLE
//! child component, with RootDirectory bound to a live parent handle.
//! Neither this module nor its Unix counterpart grants sync write authority.

#![cfg(windows)]

use super::VaultPortError;
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::ptr;

const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

const FILE_READ_DATA_OR_LIST_DIRECTORY: u32 = 0x0000_0001;
const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
const READ_CONTROL_ACCESS: u32 = 0x0002_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const SHARE_ALL: u32 = 0x0000_0007;
const FILE_OPEN: u32 = 1;
const FILE_CREATE: u32 = 2;
const FILE_ADD_FILE_OR_WRITE_DATA: u32 = 0x0000_0002;
const DELETE_ACCESS: u32 = 0x0001_0000;
const STATUS_OBJECT_NAME_COLLISION: u32 = 0xc000_0035;
const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xc000_0034;
const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const OBJ_CASE_INSENSITIVE: u32 = 0x0000_0040;
const OBJ_DONT_REPARSE: u32 = 0x0000_1000;

#[repr(C)]
struct NtUnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[repr(C)]
struct NtObjectAttributes {
    length: u32,
    root_directory: *mut c_void,
    object_name: *const NtUnicodeString,
    attributes: u32,
    security_descriptor: *mut c_void,
    security_quality_of_service: *mut c_void,
}

/// The initial union is pointer-sized; the second member is ULONG_PTR.
#[repr(C)]
struct NtIoStatusBlock {
    status_or_pointer: isize,
    information: usize,
}

// Stable Win32 file identity by HANDLE. Rust MetadataExt::{file_index,
// volume_serial_number} requires an unstable feature on stable toolchains.
// Never use path strings or creation timestamps as a file identity.
#[repr(C)]
#[derive(Default)]
struct WinFileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
#[derive(Default)]
struct WinByHandleFileInformation {
    attributes: u32,
    created: WinFileTime,
    accessed: WinFileTime,
    written: WinFileTime,
    volume_serial: u32,
    size_high: u32,
    size_low: u32,
    hard_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetFileInformationByHandle(
        file: *mut c_void,
        information: *mut WinByHandleFileInformation,
    ) -> i32;
}

/// Directory identity for the particular pinned handle, not an ambient path.
pub(super) fn file_identity(file: &File) -> Result<(u32, u64), VaultPortError> {
    let (volume, index, _) = file_identity_with_links(file)?;
    Ok((volume, index))
}

/// Identity and hard-link count from the same opened HANDLE.
pub(crate) fn file_identity_with_links(file: &File) -> Result<(u32, u64, u32), VaultPortError> {
    identity_from_borrowed_handle(file.as_raw_handle())
}

/// Use the SQLite Windows VFS-owned native HANDLE in an isolated Core test.
/// The caller MUST keep its sqlite3 connection alive and cannot close or
/// transfer ownership of this borrowed HANDLE. Normal builds lack this API.
#[cfg(test)]
pub(crate) fn borrowed_file_identity_for_lab(
    handle: *mut c_void,
) -> Result<(u32, u64), VaultPortError> {
    if handle.is_null() || handle as isize == -1 {
        return Err(VaultPortError::PermissionDenied);
    }
    let (volume, index, _) = identity_from_borrowed_handle(handle)?;
    Ok((volume, index))
}

fn identity_from_borrowed_handle(
    handle: *mut c_void,
) -> Result<(u32, u64, u32), VaultPortError> {
    let mut info = WinByHandleFileInformation::default();
    // SAFETY: the OS HANDLE remains valid for this synchronous call. The
    // output struct is fully initialized and never escapes this function.
    let ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    if ok == 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    let index = (u64::from(info.file_index_high) << 32)
        | u64::from(info.file_index_low);
    if info.volume_serial == 0 || index == 0 {
        return Err(VaultPortError::PermissionDenied);
    }
    Ok((info.volume_serial, index, info.hard_links))
}

#[link(name = "ntdll")]
#[allow(non_snake_case)]
extern "system" {
    fn NtCreateFile(
        file_handle: *mut *mut c_void,
        desired_access: u32,
        object_attributes: *const NtObjectAttributes,
        io_status_block: *mut NtIoStatusBlock,
        allocation_size: *const i64,
        file_attributes: u32,
        share_access: u32,
        create_disposition: u32,
        create_options: u32,
        ea_buffer: *mut c_void,
        ea_length: u32,
    ) -> i32;

    fn NtSetInformationFile(
        file_handle: *mut c_void,
        io_status_block: *mut NtIoStatusBlock,
        file_information: *mut c_void,
        length: u32,
        file_information_class: u32,
    ) -> i32;
}

#[repr(C)]
struct NtRenameInfo {
    replace_if_exists: u8,
    root_directory: *mut c_void,
    file_name_length: u32,
    file_name: [u16; 1],
}
const FILE_RENAME_INFORMATION_CLASS: u32 = 10;

fn io(error: impl std::fmt::Display) -> VaultPortError {
    VaultPortError::Io(error.to_string())
}

fn reject_reparse(meta: &std::fs::Metadata) -> Result<(), VaultPortError> {
    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(VaultPortError::UnsafePath);
    }
    Ok(())
}

/// Open and retain the selected directory itself. The caller must validate
/// that this path belongs to the saved host workspace registry.
pub(crate) fn open_root(root: &Path) -> Result<File, VaultPortError> {
    let mut options = OpenOptions::new();
    options.read(true);
    options.share_mode(SHARE_ALL);
    options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    let handle = options.open(root).map_err(io)?;
    let meta = handle.metadata().map_err(io)?;
    reject_reparse(&meta)?;
    if !meta.is_dir() {
        return Err(VaultPortError::UnsafePath);
    }
    Ok(handle)
}

/// One trusted registered root opened with the least directory write access
/// required for child creation. The caller MUST compare OS identity against its
/// existing pinned read-only root before publishing anything.
pub(super) fn open_root_for_create(root: &Path) -> Result<File, VaultPortError> {
    let mut options = OpenOptions::new();
    options.access_mode(FILE_READ_DATA_OR_LIST_DIRECTORY | FILE_ADD_FILE_OR_WRITE_DATA
        | FILE_READ_ATTRIBUTES | SYNCHRONIZE);
    options.share_mode(SHARE_ALL);
    options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    let handle = options.open(root).map_err(io)?;
    let meta = handle.metadata().map_err(io)?;
    reject_reparse(&meta)?;
    if !meta.is_dir() {
        return Err(VaultPortError::UnsafePath);
    }
    Ok(handle)
}

/// Only ONE already-validated portable name component is accepted.
/// OBJ_DONT_REPARSE rejects traversal; reparse points and hard links are not
/// trusted as directories. FILE_CREATE (stage only) refuses any existing name.
pub(crate) fn open_child(
    parent: &File,
    name: &str,
    leaf: bool,
) -> Result<File, VaultPortError> {
    open_child_access(parent, name, leaf,
        FILE_READ_DATA_OR_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE, FILE_OPEN)
}

/// The parent is opened with child-add authority without passing an arbitrary
/// global pathname to the Windows kernel.
pub(super) fn open_child_dir_for_create(
    parent: &File, name: &str,
) -> Result<File, VaultPortError> {
    open_child_access(parent, name, false,
        FILE_READ_DATA_OR_LIST_DIRECTORY | FILE_ADD_FILE_OR_WRITE_DATA
        | FILE_READ_ATTRIBUTES | SYNCHRONIZE, FILE_OPEN)
}

/// Create a hidden in-progress file exclusively relative to its pinned parent.
/// DELETE access is required to rename the SAME opened handle later.
pub(super) fn create_stage_child(
    parent: &File, name: &str,
) -> Result<File, VaultPortError> {
    open_child_access(parent, name, true,
        FILE_READ_DATA_OR_LIST_DIRECTORY | FILE_ADD_FILE_OR_WRITE_DATA
        | FILE_READ_ATTRIBUTES | DELETE_ACCESS | SYNCHRONIZE, FILE_CREATE)
}

/// Dormant diagnostic: open an ALREADY EXISTING lock file relative to the
/// pinned root HANDLE. Never create, truncate or replace it. This is not
/// exposed as normal File Gate or sync write authority.
pub(super) fn open_existing_probe_lock_child(
    parent: &File,
    name: &str,
) -> Result<File, VaultPortError> {
    open_child_access(parent, name, true,
        FILE_READ_DATA_OR_LIST_DIRECTORY | FILE_ADD_FILE_OR_WRITE_DATA
        | FILE_READ_ATTRIBUTES | READ_CONTROL_ACCESS | SYNCHRONIZE,
        FILE_OPEN,
    )
}

/// Test-only exclusive bootstrap of a lock leaf under the pinned parent.
/// FILE_CREATE refuses an existing name; no ambient global path, replacement
/// or truncation is possible at this primitive. No production caller exists.
#[cfg(test)]
pub(super) fn create_probe_lock_child(
    parent: &File, name: &str,
) -> Result<File, VaultPortError> {
    open_child_access(parent, name, true,
        FILE_READ_DATA_OR_LIST_DIRECTORY | FILE_ADD_FILE_OR_WRITE_DATA
        | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_CREATE,
    )
}

/// Inspection-only, relative to the pinned directory HANDLE. Only the
/// native STATUS_OBJECT_NAME_NOT_FOUND status means absent; all other NT
/// errors and reparse/special-object aliases fail closed.
pub(super) fn open_optional_child(parent: &File, name: &str)
    -> Result<Option<File>, VaultPortError>
{
    open_child_access_optional(
        parent, name, true,
        FILE_READ_DATA_OR_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_OPEN,
    )
}

fn open_child_access(
    parent: &File, name: &str, leaf: bool, desired_access: u32,
    disposition: u32,
) -> Result<File, VaultPortError> {
    open_child_access_optional(parent, name, leaf, desired_access, disposition)?
        .ok_or(VaultPortError::FileChanged)
}

fn open_child_access_optional(
    parent: &File, name: &str, leaf: bool, desired_access: u32,
    disposition: u32,
) -> Result<Option<File>, VaultPortError> {
    if name.is_empty() || name == "." || name == ".."
        || name.chars().any(|ch| matches!(ch, '/' | '\\' | ':' | '\0'))
    {
        return Err(VaultPortError::UnsafePath);
    }
    let mut wide: Vec<u16> = name.encode_utf16().collect();
    let bytes = wide.len().checked_mul(2).ok_or(VaultPortError::UnsafePath)?;
    if bytes == 0 || bytes > u16::MAX as usize {
        return Err(VaultPortError::UnsafePath);
    }
    let unicode = NtUnicodeString {
        length: bytes as u16,
        maximum_length: bytes as u16,
        buffer: wide.as_mut_ptr(),
    };
    let attributes = NtObjectAttributes {
        length: std::mem::size_of::<NtObjectAttributes>() as u32,
        root_directory: parent.as_raw_handle(),
        object_name: &unicode,
        attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        security_descriptor: ptr::null_mut(),
        security_quality_of_service: ptr::null_mut(),
    };
    let mut status_block = NtIoStatusBlock {
        status_or_pointer: 0,
        information: 0,
    };
    let mut raw = ptr::null_mut();
    let opts = FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT
        | if leaf { FILE_NON_DIRECTORY_FILE } else { FILE_DIRECTORY_FILE };

    // SAFETY: all pointers are valid for the synchronous NT call; parent
    // handle stays open; each name is one validated component; on success the
    // returned handle is uniquely owned by File. FILE_CREATE is exclusive.
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            desired_access,
            &attributes,
            &mut status_block,
            ptr::null(),
            0,
            SHARE_ALL,
            disposition,
            opts,
            ptr::null_mut(),
            0,
        )
    };
    if status < 0 || raw.is_null() {
        if status as u32 == STATUS_OBJECT_NAME_NOT_FOUND && disposition == FILE_OPEN {
            return Ok(None);
        }
        return if status as u32 == STATUS_OBJECT_NAME_COLLISION {
            Err(VaultPortError::DestinationExists)
        } else {
            Err(VaultPortError::Io(format!(
                "root-relative NT open failed (NTSTATUS 0x{:08x})",
                status as u32
            )))
        };
    }
    // SAFETY: NtCreateFile succeeded and returned a unique owned HANDLE.
    let file = unsafe { File::from_raw_handle(raw) };
    let meta = file.metadata().map_err(io)?;
    reject_reparse(&meta)?;
    if leaf && !meta.is_file() || !leaf && !meta.is_dir() {
        return Err(VaultPortError::UnsafePath);
    }
    Ok(Some(file))
}

/// Rename the fully flushed opened stage HANDLE with an OS-atomic
/// no-replace predicate. RootDirectory is the pinned parent HANDLE, not a
/// path re-resolved through an ambient process root.
pub(super) fn rename_staged_no_replace(
    stage: &File, parent: &File, leaf: &str,
) -> Result<(), VaultPortError> {
    if leaf.is_empty() || leaf == "." || leaf == ".."
        || leaf.chars().any(|ch| matches!(ch, '/' | '\\' | ':' | '\0'))
    {
        return Err(VaultPortError::UnsafePath);
    }
    let name: Vec<u16> = leaf.encode_utf16().collect();
    let bytes = name.len().checked_mul(2).ok_or(VaultPortError::UnsafePath)?;
    if bytes == 0 || bytes > u32::MAX as usize {
        return Err(VaultPortError::UnsafePath);
    }
    // Heap allocation aligned for pointers; struct contains one WCHAR at its
    // tail followed by enough space for the entire validated component.
    let total = std::mem::size_of::<NtRenameInfo>() + bytes;
    let words = (total + std::mem::size_of::<usize>() - 1)
        / std::mem::size_of::<usize>();
    let mut buffer = vec![0usize; words];
    let info = buffer.as_mut_ptr().cast::<NtRenameInfo>();
    // SAFETY: buffer length/alignment account for the header and full UTF-16
    // component; no references alias the raw write; the NT call is synchronous
    // and both directory and staged handles stay live.
    let status = unsafe {
        ptr::write(info, NtRenameInfo {
            replace_if_exists: 0,
            root_directory: parent.as_raw_handle(),
            file_name_length: bytes as u32,
            file_name: [0u16; 1],
        });
        let dst = ptr::addr_of_mut!((*info).file_name).cast::<u16>();
        ptr::copy_nonoverlapping(name.as_ptr(), dst, name.len());
        let mut iosb = NtIoStatusBlock {
            status_or_pointer: 0, information: 0,
        };
        NtSetInformationFile(
            stage.as_raw_handle(), &mut iosb,
            buffer.as_mut_ptr().cast::<c_void>(),
            total as u32,
            FILE_RENAME_INFORMATION_CLASS,
        )
    };
    if status >= 0 {
        return Ok(());
    }
    if status as u32 == STATUS_OBJECT_NAME_COLLISION {
        Err(VaultPortError::DestinationExists)
    } else {
        Err(VaultPortError::Io(format!(
            "root-relative NT no-replace rename failed (NTSTATUS 0x{:08x})",
            status as u32
        )))
    }
}
