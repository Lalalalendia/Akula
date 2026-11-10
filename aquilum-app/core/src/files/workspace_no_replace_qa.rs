//! Test-only, unconnected filesystem primitive for the future workspace rename.
//!
//! Safety scope: same-parent directory rename with native NOREPLACE; it cannot
//! merge directories, overwrite an occupied destination, copy to another disk,
//! alter SQLite, or remove a source directory. All fixtures use temporary roots.
//!
//! NOT an admission capability. Filesystem path preflight is vulnerable to
//! hostile path substitution (especially Windows path-based MoveFileExW).
//! Multi-process File Gate/History/S3 custody and a crash-recovery journal must
//! precede any runtime call. Do not expose this module outside #[cfg(test)].
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Component, Path};

fn denied(reason: &'static str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, reason)
}

fn plain_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(denied("source or ancestor is not a plain directory"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x0000_0400 != 0 {
            return Err(denied("directory reparse points are forbidden"));
        }
    }
    Ok(())
}

fn checked_parent(path: &Path) -> io::Result<&Path> {
    if !path.is_absolute() ||
        path.components().any(|c| matches!(c, Component::CurDir | Component::ParentDir))
    {
        return Err(io::Error::new(ErrorKind::InvalidInput, "absolute paths without dot components required"));
    }
    let parent = path.parent().ok_or_else(|| denied("cannot rename a volume root"))?;
    if path.file_name().is_none() {
        return Err(denied("source or destination has no leaf name"));
    }
    for ancestor in parent.ancestors() {
        if !ancestor.as_os_str().is_empty() {
            plain_directory(ancestor)?;
        }
    }
    Ok(parent)
}

fn no_replace_directory_rename(source: &Path, destination: &Path) -> io::Result<()> {
    let source_parent = checked_parent(source)?;
    let destination_parent = checked_parent(destination)?;
    if source_parent != destination_parent {
        return Err(denied("only sibling directory renames are permitted"));
    }
    if source.file_name() == destination.file_name() {
        return Err(io::Error::new(ErrorKind::AlreadyExists, "destination matches source"));
    }
    plain_directory(source)?;
    // This check is only fast failure; the OS move itself MUST enforce
    // NOREPLACE atomically, including against a concurrent creator.
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err(io::Error::new(ErrorKind::AlreadyExists, "destination occupied")),
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use rustix::fs::{renameat_with, RenameFlags};
        let pinned_parent = fs::File::open(source_parent)?;
        if !pinned_parent.metadata()?.is_dir() {
            return Err(denied("pinned parent is not a directory"));
        }
        // Both names resolve relative to the SAME opened parent fd; the
        // kernel rejects an occupied destination as part of renameat2.
        return renameat_with(
            &pinned_parent,
            source.file_name().ok_or_else(|| denied("missing source leaf"))?,
            &pinned_parent,
            destination.file_name().ok_or_else(|| denied("missing destination leaf"))?,
            RenameFlags::NOREPLACE,
        ).map_err(Into::into);
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::fd::AsRawFd;
        use std::os::raw::c_char;
        use std::os::unix::ffi::OsStrExt;
        unsafe extern "C" {
            fn renameatx_np(
                old_dirfd: i32, old_name: *const c_char,
                new_dirfd: i32, new_name: *const c_char,
                flags: u32,
            ) -> i32;
        }
        const RENAME_EXCL: u32 = 0x0000_0004;
        let old_name = CString::new(
            source.file_name().ok_or_else(|| denied("missing source leaf"))?.as_bytes()
        ).map_err(|_| io::Error::new(ErrorKind::InvalidInput, "embedded NUL in source name"))?;
        let new_name = CString::new(
            destination.file_name().ok_or_else(|| denied("missing destination leaf"))?.as_bytes()
        ).map_err(|_| io::Error::new(ErrorKind::InvalidInput, "embedded NUL in destination name"))?;
        // Apple renameatx_np atomically enforces RENAME_EXCL on APFS.
        // Resolve both leaf names against the same open parent directory.
        // No unguarded rename(), hardlink/unlink fallback, or copy/delete.
        let pinned_parent = fs::File::open(source_parent)?;
        let ok = unsafe {
            renameatx_np(
                pinned_parent.as_raw_fd(), old_name.as_ptr(),
                pinned_parent.as_raw_fd(), new_name.as_ptr(), RENAME_EXCL,
            )
        };
        return if ok == 0 { Ok(()) } else { Err(io::Error::last_os_error()) };
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "Kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
        }
        let from: Vec<u16> = source.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let to: Vec<u16> = destination.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // Flags=0: no REPLACE_EXISTING, no COPY_ALLOWED, no reboot or delayed
        // moves. Windows still resolves the pathname afresh: NOT race-safe
        // against hostile same-user filesystem namespace replacement.
        let ok = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) };
        return if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) };
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos", windows)))]
    {
        let _ = source_parent;
        Err(io::Error::new(ErrorKind::Unsupported, "no verified native no-replace move on this OS"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn root() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        // Canonical temp root prevents macOS /var -> /private/var alias from
    // accidentally passing an untrusted symlink ancestor into the rename.
    let from = fs::canonicalize(tmp.path()).unwrap().join("Original");
        fs::create_dir(&from).unwrap();
        fs::write(from.join("chapter.md"), "Original text").unwrap();
        (tmp, from)
    }

    #[test]
    fn sibling_rename_preserves_nested_bytes_without_copy_or_merge() {
        let (_temp, source) = root();
        fs::create_dir(source.join("media")).unwrap();
        fs::write(source.join("media/image.bin"), [0_u8, 255, 1]).unwrap();
        let destination = source.with_file_name("Renamed");
        if !cfg!(any(target_os = "linux", target_os = "android", target_os = "macos", windows)) {
            assert_eq!(no_replace_directory_rename(&source, &destination).unwrap_err().kind(), ErrorKind::Unsupported);
            return;
        }
        no_replace_directory_rename(&source, &destination).unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read_to_string(destination.join("chapter.md")).unwrap(), "Original text");
        assert_eq!(fs::read(destination.join("media/image.bin")).unwrap(), [0, 255, 1]);
    }

    #[test]
    fn occupied_directory_is_never_merged_or_overwritten() {
        let (_tmp, source) = root();
        let dest = source.with_file_name("Existing");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("do-not-change.md"), "DESTINATION").unwrap();
        assert_eq!(no_replace_directory_rename(&source, &dest).unwrap_err().kind(), ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(source.join("chapter.md")).unwrap(), "Original text");
        assert_eq!(fs::read_to_string(dest.join("do-not-change.md")).unwrap(), "DESTINATION");
        assert!(!dest.join("chapter.md").exists());
    }

    #[test]
    fn occupied_file_is_never_replaced_by_directory() {
        let (_tmp, source) = root();
        let dest = source.with_file_name("File");
        fs::write(&dest, "DO NOT OVERWRITE").unwrap();
        assert_eq!(no_replace_directory_rename(&source, &dest).unwrap_err().kind(), ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&dest).unwrap(), "DO NOT OVERWRITE");
        assert!(source.join("chapter.md").exists());
    }

    #[test]
    fn a_move_outside_the_same_parent_is_denied_before_mutation() {
        let (tmp, source) = root();
        let other = tmp.path().join("OtherParent");
        fs::create_dir(&other).unwrap();
        let destination = other.join("Moved");
        assert!(no_replace_directory_rename(&source, &destination).is_err());
        assert!(source.join("chapter.md").exists());
        assert!(!destination.exists());
    }

    #[test]
    fn two_sources_racing_for_one_destination_never_overwrite() {
        if !cfg!(any(target_os = "linux", target_os = "android", target_os = "macos", windows)) { return; }
        let (tmp, first) = root();
        let second = first.with_file_name("Second");
        fs::create_dir(&second).unwrap();
        fs::write(second.join("chapter.md"), "Second text").unwrap();
        let dest = first.with_file_name("Destination");
        let barrier = Arc::new(Barrier::new(3));
        let joins: Vec<_> = [first.clone(), second.clone()].into_iter().map(|from| {
            let target = dest.clone();
            let start = Arc::clone(&barrier);
            thread::spawn(move || {
                start.wait();
                no_replace_directory_rename(&from, &target)
            })
        }).collect();
        barrier.wait();
        let results: Vec<_> = joins.into_iter().map(|job| job.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|x| x.is_ok()).count(), 1);
        let text = fs::read_to_string(dest.join("chapter.md")).unwrap();
        assert!(text == "Original text" || text == "Second text");
        // The loser remains untouched, rather than being merged or deleted.
        let loser = if first.exists() { first } else { second };
        assert_eq!(fs::read_to_string(loser.join("chapter.md")).unwrap(),
            if text == "Original text" { "Second text" } else { "Original text" });
    }

    #[cfg(unix)]
    #[test]
    fn source_symlink_and_parent_symlink_are_denied() {
        use std::os::unix::fs::symlink;
        let (tmp, root) = root();
        let link = tmp.path().join("LinkedRoot");
        symlink(&root, &link).unwrap();
        assert!(no_replace_directory_rename(&link, &tmp.path().join("New")).is_err());
        let parent_link = tmp.path().join("ParentLink");
        symlink(tmp.path(), &parent_link).unwrap();
        assert!(no_replace_directory_rename(&parent_link.join("Original"), &parent_link.join("Moved")).is_err());
        assert!(root.join("chapter.md").is_file());
    }

    #[cfg(windows)]
    #[test]
    fn source_junction_is_denied_without_following_external_data() {
        use std::process::Command;
        let (tmp, root) = root();
        let link = tmp.path().join("LinkedRoot");
        let out = Command::new("cmd").arg("/C").arg("mklink").arg("/J")
            .arg(&link).arg(&root).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(no_replace_directory_rename(&link, &tmp.path().join("New")).is_err());
        assert!(root.join("chapter.md").is_file());
        assert!(!tmp.path().join("New").exists());
    }
}