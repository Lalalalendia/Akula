//! Win7-compatible, root-relative NT open of one existing DELETE-capable file.
//! This is a test-only module: no GUI or shipping callers may reach it.
use super::VaultPortError;
use std::{
    ffi::c_void,
    fs::File,
    os::windows::io::{AsRawHandle, FromRawHandle},
    ptr,
};

const READ_DATA: u32 = 0x0000_0001;
const READ_ATTRIBUTES: u32 = 0x0000_0080;
const DELETE_ACCESS: u32 = 0x0001_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const SHARE_READ: u32 = 0x0000_0001;
const FILE_OPEN: u32 = 1;
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

#[repr(C)]
struct NtIoStatusBlock {
    status_or_pointer: isize,
    information: usize,
}

#[link(name = "ntdll")]
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

/// Open one leaf relative to the already pinned objects directory; no path
/// traversal, create, truncate or replace authority. Deny write/delete sharing.
pub(super) fn open_delete_candidate(parent: &File, leaf: &str) -> Result<File, VaultPortError> {
    if leaf.is_empty()
        || leaf == "."
        || leaf == ".."
        || leaf.chars().any(|c| matches!(c, '/' | '\\' | ':' | '\0'))
    {
        return Err(VaultPortError::UnsafePath);
    }
    let mut wide: Vec<u16> = leaf.encode_utf16().collect();
    let bytes = wide
        .len()
        .checked_mul(2)
        .ok_or(VaultPortError::UnsafePath)?;
    if bytes == 0 || bytes > u16::MAX as usize {
        return Err(VaultPortError::UnsafePath);
    }
    let name = NtUnicodeString {
        length: bytes as u16,
        maximum_length: bytes as u16,
        buffer: wide.as_mut_ptr(),
    };
    let attributes = NtObjectAttributes {
        length: std::mem::size_of::<NtObjectAttributes>() as u32,
        root_directory: parent.as_raw_handle(),
        object_name: &name,
        attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        security_descriptor: ptr::null_mut(),
        security_quality_of_service: ptr::null_mut(),
    };
    let mut iosb = NtIoStatusBlock {
        status_or_pointer: 0,
        information: 0,
    };
    let mut raw: *mut c_void = ptr::null_mut();
    // SAFETY: live, pinned RootDirectory, one validated component, synchronous
    // NT open and uniquely owned result. The borrowed UTF16 buffer cannot move.
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            READ_DATA | READ_ATTRIBUTES | DELETE_ACCESS | SYNCHRONIZE,
            &attributes,
            &mut iosb,
            ptr::null(),
            0,
            SHARE_READ,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            ptr::null_mut(),
            0,
        )
    };
    if status < 0 || raw.is_null() {
        return Err(VaultPortError::Io(format!(
            "pinned Inbox delete open refused: NTSTATUS 0x{:08x}",
            status as u32
        )));
    }
    // SAFETY: NtCreateFile returned a unique valid HANDLE owned by this File.
    Ok(unsafe { File::from_raw_handle(raw) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn relative_open_refuses_escapes_and_directory_leaf() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("objects");
        fs::create_dir(&parent).unwrap();
        fs::create_dir(parent.join("nested")).unwrap();
        let opened = super::super::pinned_windows::open_root(&parent).unwrap();
        for name in ["", ".", "..", "../escape", "a\\b", "C:evil"] {
            assert!(open_delete_candidate(&opened, name).is_err());
        }
        assert!(open_delete_candidate(&opened, "missing.blob").is_err());
        assert!(open_delete_candidate(&opened, "nested").is_err());
    }
}
