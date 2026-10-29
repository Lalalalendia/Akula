//! Windows 7 same-HANDLE delete probe. QA-only; no shipping API or GUI.
use super::{pinned_windows, VaultPortError};
use std::{
    ffi::c_void,
    fs,
    io::Read,
    os::windows::{fs::MetadataExt, io::AsRawHandle},
    path::Path,
};

mod relative;
use relative::open_delete_candidate;
use uuid::Uuid;

const REPARSE_ATTRIBUTE: u32 = 0x0000_0400;
const FILE_DISPOSITION_INFO: u32 = 4;
const MAX_BYTES: u64 = 8 * 1024 * 1024;

#[link(name = "kernel32")]
extern "system" {
    fn SetFileInformationByHandle(
        handle: *mut c_void,
        class: u32,
        info: *mut c_void,
        size: u32,
    ) -> i32;
}

/// Lab-only. Production still requires explicit authenticated SQLite consent.
fn delete_disposable_fixture(
    root: &Path,
    id: Uuid,
    expected_bytes: u64,
    expected_hash: [u8; 32],
) -> Result<(), VaultPortError> {
    if id.is_nil() || expected_bytes == 0 || expected_bytes > MAX_BYTES || expected_hash == [0; 32]
    {
        return Err(VaultPortError::UnsafePath);
    }
    let (_root_handle, objects, _db_handle) = super::pin_p2p_read_snapshot(root)?;
    let name = format!("{}.blob", id.hyphenated());
    // Stable directory-relative witness; no sender-controlled path.
    let witness = pinned_windows::open_child(&objects, &name, true)?;
    let identity = pinned_windows::file_identity_with_links(&witness)?;
    if identity.2 != 1 {
        return Err(VaultPortError::FileChanged);
    }

    // Open DELETE-capable HANDLE using NtCreateFile RootDirectory = the
    // pinned objects directory. Never re-resolve its ambient pathname.
    let mut candidate = open_delete_candidate(&objects, &name)?;
    let meta = candidate
        .metadata()
        .map_err(|e| VaultPortError::Io(e.to_string()))?;
    if !meta.is_file()
        || meta.file_attributes() & REPARSE_ATTRIBUTE != 0
        || meta.len() != expected_bytes
        || pinned_windows::file_identity_with_links(&candidate)? != identity
    {
        return Err(VaultPortError::FileChanged);
    }
    let mut total = 0u64;
    let mut hasher = blake3::Hasher::new();
    let mut block = [0u8; 64 * 1024];
    loop {
        let n = candidate
            .read(&mut block)
            .map_err(|e| VaultPortError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .ok_or(VaultPortError::FileChanged)?;
        if total > expected_bytes {
            return Err(VaultPortError::FileChanged);
        }
        hasher.update(&block[..n]);
    }
    if total != expected_bytes
        || *hasher.finalize().as_bytes() != expected_hash
        || candidate
            .metadata()
            .map_err(|e| VaultPortError::Io(e.to_string()))?
            .len()
            != expected_bytes
        || pinned_windows::file_identity_with_links(&candidate)? != identity
    {
        return Err(VaultPortError::FileChanged);
    }
    drop(witness);
    // Win7-compatible deletion is marked on the checked HANDLE, not a path.
    let mut disposition = 1u8;
    // SAFETY: candidate holds DELETE access; synchronous API receives one
    // valid FILE_DISPOSITION_INFO byte and a live OS file handle.
    let ok = unsafe {
        SetFileInformationByHandle(
            candidate.as_raw_handle(),
            FILE_DISPOSITION_INFO,
            (&mut disposition as *mut u8).cast::<c_void>(),
            1,
        )
    };
    if ok == 0 {
        return Err(VaultPortError::Io(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    drop(candidate);
    // Verify absence using the pinned objects HANDLE. Delete-pending and any
    // unexpected replacement remain errors, never a successful receipt.
    if pinned_windows::open_optional_child(&objects, &name)?.is_some() {
        return Err(VaultPortError::FileChanged);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("inbox");
        fs::create_dir_all(root.join("objects")).unwrap();
        fs::write(root.join("received-inbox.sqlite3"), b"dummy").unwrap();
        (dir, root)
    }

    #[test]
    fn only_verified_uuid_leaf_is_deleted() {
        let (_temp, root) = fixture();
        let id = Uuid::new_v4();
        let bytes = b"checked fixture";
        let leaf = root
            .join("objects")
            .join(format!("{}.blob", id.hyphenated()));
        let other = root.join("objects").join("unrelated");
        fs::write(&leaf, bytes).unwrap();
        fs::write(&other, b"retain").unwrap();
        assert!(delete_disposable_fixture(
            &root,
            Uuid::new_v4(),
            bytes.len() as u64,
            *blake3::hash(bytes).as_bytes(),
        )
        .is_err());
        assert_eq!(fs::read(&leaf).unwrap(), bytes);
        delete_disposable_fixture(
            &root,
            id,
            bytes.len() as u64,
            *blake3::hash(bytes).as_bytes(),
        )
        .unwrap();
        assert!(!leaf.exists());
        assert_eq!(fs::read(&other).unwrap(), b"retain");
    }

    #[test]
    fn refuses_bad_hash_size_and_multiple_hard_links() {
        let (_temp, root) = fixture();
        let id = Uuid::new_v4();
        let bytes = b"still owned";
        let leaf = root
            .join("objects")
            .join(format!("{}.blob", id.hyphenated()));
        fs::write(&leaf, bytes).unwrap();
        assert!(delete_disposable_fixture(&root, id, bytes.len() as u64, [9; 32]).is_err());
        assert!(delete_disposable_fixture(
            &root,
            id,
            bytes.len() as u64 + 1,
            *blake3::hash(bytes).as_bytes(),
        )
        .is_err());
        let alias = root.join("objects").join("hard-link");
        fs::hard_link(&leaf, &alias).unwrap();
        assert!(delete_disposable_fixture(
            &root,
            id,
            bytes.len() as u64,
            *blake3::hash(bytes).as_bytes(),
        )
        .is_err());
        assert_eq!(fs::read(&leaf).unwrap(), bytes);
        assert_eq!(fs::read(&alias).unwrap(), bytes);
    }
}
