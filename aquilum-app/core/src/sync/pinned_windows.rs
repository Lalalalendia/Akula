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
const SYNCHRONIZE: u32 = 0x0010_0000;
const SHARE_ALL: u32 = 0x0000_0007;
const FILE_OPEN: u32 = 1;
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
pub(super) fn file_identity_with_links(file: &File) -> Result<(u32, u64, u32), VaultPortError> {
    let mut info = WinByHandleFileInformation::default();
    // SAFETY: file is live, the output struct is fully initialized, and
    // the synchronous Win32 call writes into our valid stack allocation.
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) };
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
}

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
pub(super) fn open_root(root: &Path) -> Result<File, VaultPortError> {
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

/// Only ONE already-validated portable name component is accepted.
/// OBJ_DONT_REPARSE rejects traversal, FILE_OPEN_REPARSE_POINT opens a reparse
/// target as the reparse object itself, and its attribute is then rejected.
pub(super) fn open_child(
    parent: &File,
    name: &str,
    leaf: bool,
) -> Result<File, VaultPortError> {
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

    // SAFETY:
    // - All pointers refer to live, stack-owned C ABI structures/UTF-16 bytes.
    // - The parent handle remains open for the duration of the synchronous call.
    // - Each name is one path component, with no separators, colon or NUL.
    // - FILE_OPEN is read-only and cannot create or overwrite objects.
    // - On NT_SUCCESS the returned raw handle is uniquely owned by File.
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            FILE_READ_DATA_OR_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            &attributes,
            &mut status_block,
            ptr::null(),
            0,
            SHARE_ALL,
            FILE_OPEN,
            opts,
            ptr::null_mut(),
            0,
        )
    };
    if status < 0 || raw.is_null() {
        return Err(VaultPortError::Io(format!(
            "root-relative NT open failed (NTSTATUS 0x{:08x})",
            status as u32
        )));
    }
    // SAFETY: NtCreateFile succeeded and returned a unique owned HANDLE.
    let file = unsafe { File::from_raw_handle(raw) };
    let meta = file.metadata().map_err(io)?;
    reject_reparse(&meta)?;
    if leaf && !meta.is_file() || !leaf && !meta.is_dir() {
        return Err(VaultPortError::UnsafePath);
    }
    Ok(file)
}
