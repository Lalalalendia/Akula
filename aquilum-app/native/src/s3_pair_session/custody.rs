//! Windows OS-known, per-user File Gate lock namespace for S3 Sync Beta.
//!
//! This is intentionally NOT a privileged installer witness. It supports
//! cooperative processes of the same logged-on user for NEW-FILE-ONLY beta.
//! The parent comes from SHGetKnownFolderPath, never an imported S3 config,
//! environment variable, vault folder or arbitrary command-line --data-dir.

use std::{
    ffi::{c_void, OsString},
    fs, io,
    os::windows::{ffi::OsStringExt, fs::MetadataExt},
    path::PathBuf,
    ptr,
};

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}
// Microsoft FOLDERID_LocalAppData:
// {F1B32785-6FBA-4FCF-9D55-7B8E7F157091}
const LOCAL_APPDATA: Guid = Guid {
    data1: 0xF1B3_2785,
    data2: 0x6FBA,
    data3: 0x4FCF,
    data4: [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
};
const ROOT_LEAF: &str = "Akula-S3Pair-FileGate-V1";

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHGetKnownFolderPath(
        id: *const Guid, flags: u32, token: *mut c_void,
        result: *mut *mut u16,
    ) -> i32;
}
#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
    fn CoUninitialize();
    fn CoTaskMemFree(value: *mut c_void);
}
struct ScopedComInitialization;
impl Drop for ScopedComInitialization {
    fn drop(&mut self) {
        // SAFETY: successful CoInitializeEx on this same worker thread
        // must be paired with one CoUninitialize even for S_FALSE.
        unsafe { CoUninitialize(); }
    }
}
fn initialize_worker_com() -> io::Result<Option<ScopedComInitialization>> {
    // COINIT_MULTITHREADED = 0. The pairing operation runs in a dedicated
    // worker thread and never accesses the GUI COM apartment.
    let hr = unsafe { CoInitializeEx(ptr::null_mut(), 0) };
    if hr >= 0 {
        Ok(Some(ScopedComInitialization))
    } else if hr == -2_147_417_850 { // RPC_E_CHANGED_MODE (0x80010106)
        Ok(None) // Already initialized to another apartment, no uninit.
    } else {
        Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "Windows COM worker initialization failed"))
    }
}

fn known_local_appdata() -> io::Result<PathBuf> {
    let _com = initialize_worker_com()?;
    let mut ptr: *mut u16 = ptr::null_mut();
    // SAFETY: GUID shape/ABI matches KNOWNFOLDERID. Windows allocates a
    // null-terminated UTF-16 buffer with CoTaskMemAlloc.
    let hr = unsafe {
        SHGetKnownFolderPath(&LOCAL_APPDATA, 0, ptr::null_mut(), &mut ptr)
    };
    if hr < 0 || ptr.is_null() {
        if !ptr.is_null() {
            // SAFETY: ppszPath is owned by the caller even after API failure.
            unsafe { CoTaskMemFree(ptr.cast()); }
        }
        return Err(io::Error::new(io::ErrorKind::NotFound,
            "Windows LocalAppData known folder unavailable"));
    }
    let mut len = 0;
    // SAFETY: SHGetKnownFolderPath returns a valid, terminated buffer. We
    // cap the scan at the documented extended Win32 maximum path length.
    while len < 32_767 && unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    let path = if len > 0 && len < 32_767 {
        // SAFETY: the buffer remains alive until CoTaskMemFree below.
        let utf16 = unsafe { std::slice::from_raw_parts(ptr, len) };
        Some(PathBuf::from(OsString::from_wide(utf16)))
    } else {
        None
    };
    // SAFETY: exactly one free of the OS-owned string after copying it.
    unsafe { CoTaskMemFree(ptr.cast()); }
    let path = path.ok_or_else(|| io::Error::new(
        io::ErrorKind::InvalidData, "invalid known-folder path"))?;
    if !path.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "LocalAppData is not absolute"));
    }
    Ok(path)
}

/// Only user-initiated Configure/Import may create the base directory.
pub(super) fn prepare_host_base() -> io::Result<PathBuf> {
    let path = known_local_appdata()?.join(ROOT_LEAF);
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    if !fs::symlink_metadata(&path)?.is_dir() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "S3 File Gate base replaced or inaccessible"));
    }
    Ok(path)
}

/// SyncNow never creates or repairs the host namespace, not even the base.
pub(super) fn existing_host_base() -> io::Result<PathBuf> {
    let path = known_local_appdata()?.join(ROOT_LEAF);
    let meta = fs::symlink_metadata(&path)?;
    if !meta.is_dir() || meta.file_type().is_symlink()
        || meta.file_attributes() & 0x400 != 0
    {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied,
            "S3 File Gate host base is not an ordinary Windows directory"));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_folder_is_os_sourced_and_never_an_imported_config_path() {
        assert_eq!(ROOT_LEAF, "Akula-S3Pair-FileGate-V1");
        assert!(!ROOT_LEAF.chars().any(|ch| ch == '/' || ch == '\\' || ch == ':'));
        assert!(!ROOT_LEAF.starts_with('.'));
    }
}
