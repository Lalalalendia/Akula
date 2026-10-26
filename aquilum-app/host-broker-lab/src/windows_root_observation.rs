//! AKAI-HOST-ROOT-OBSERVE-00: read-only Windows physical root observation.
//!
//! This module does not enroll vaults, issue a token or authorize any MCP
//! operation. The caller's path is NOT an installer-owned registration.
//! It opens and retains each directory ancestor, rejects reparse points,
//! and obtains FILE_ID_INFO from the final open handle. Windows may rename
//! a directory despite open handles and no FILE_SHARE_DELETE: we pin the
//! open object identity, NOT an immutable path. The resulting pair
//! distinguishes live objects on this host, but it is not a durable issuer,
//! rollback protection, volume authenticity, or proof of filesystem ownership.
//! This standalone Broker is still not installed as an Akula service.

#![cfg(windows)]
#![allow(dead_code)] // No production issuer or MCP consumer may call this yet.

use std::ffi::{c_void, OsStr};
use std::io::{self, ErrorKind};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr;

type Handle = *mut c_void;
const INVALID_HANDLE: Handle = -1isize as Handle;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const READ_CONTROL: u32 = 0x0002_0000;
const FILE_SHARE_READ: u32 = 0x01;
const FILE_SHARE_WRITE: u32 = 0x02;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const FILE_ID_INFO_CLASS: u32 = 0x12;
const DRIVE_FIXED: u32 = 3;

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
#[derive(Default)]
struct HandleInfo {
    attributes: u32,
    created: FileTime,
    accessed: FileTime,
    written: FileTime,
    volume_serial: u32,
    size_high: u32,
    size_low: u32,
    link_count: u32,
    index_high: u32,
    index_low: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RootIdentity {
    volume_serial: u64,
    file_id: [u8; 16],
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        path: *const u16,
        access: u32,
        share: u32,
        security: Handle,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetFileInformationByHandle(handle: Handle, info: *mut HandleInfo) -> i32;
    fn GetFileInformationByHandleEx(handle: Handle, class: u32, output: Handle, size: u32) -> i32;
    fn GetDriveTypeW(root: *const u16) -> u32;
}

fn refuse(reason: &'static str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, reason)
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut text: Vec<u16> = value.encode_wide().collect();
    if text.contains(&0) {
        return Err(refuse("embedded NUL in claimed directory"));
    }
    text.push(0);
    Ok(text)
}

struct OpenedDirectory(Handle);

impl OpenedDirectory {
    fn open(path: &Path) -> io::Result<Self> {
        let text = wide(path.as_os_str())?;
        let raw = unsafe {
            CreateFileW(
                text.as_ptr(),
                FILE_READ_ATTRIBUTES | READ_CONTROL,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if raw.is_null() || raw == INVALID_HANDLE {
            return Err(io::Error::last_os_error());
        }
        let result = Self(raw);
        let mut metadata = HandleInfo::default();
        if unsafe { GetFileInformationByHandle(result.0, &mut metadata) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if metadata.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(refuse("any ancestor reparse point is not an enrolled root"));
        }
        if metadata.attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err(refuse("claimed vault root is not a directory"));
        }
        Ok(result)
    }

    fn identity(&self) -> io::Result<RootIdentity> {
        let mut identity = RootIdentity::default();
        let ok = unsafe {
            GetFileInformationByHandleEx(
                self.0,
                FILE_ID_INFO_CLASS,
                (&mut identity as *mut RootIdentity).cast(),
                size_of::<RootIdentity>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if identity.volume_serial == 0 || identity.file_id == [0; 16] {
            return Err(refuse("filesystem supplied a non-identity"));
        }
        Ok(identity)
    }
}

impl Drop for OpenedDirectory {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn local_components(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut iter = path.components();
    let prefix = match iter.next() {
        Some(Component::Prefix(value)) if matches!(value.kind(), Prefix::Disk(_)) => value,
        _ => return Err(refuse("only an absolute local drive path is supported")),
    };
    if !matches!(iter.next(), Some(Component::RootDir)) {
        return Err(refuse("drive-relative or incomplete path"));
    }
    let mut current = PathBuf::from(prefix.as_os_str());
    current.push("\\");
    let disk = wide(current.as_os_str())?;
    if unsafe { GetDriveTypeW(disk.as_ptr()) } != DRIVE_FIXED {
        return Err(refuse("only a local fixed disk is supported"));
    }
    let mut paths = vec![current.clone()];
    for element in iter {
        let Component::Normal(name) = element else {
            return Err(refuse("relative or unsupported root component"));
        };
        let units: Vec<u16> = name.encode_wide().collect();
        if units.is_empty()
            || units.contains(&0)
            || units.contains(&(':' as u16))
            || units.last() == Some(&('.' as u16))
            || units.last() == Some(&(' ' as u16))
        {
            return Err(refuse("ambiguous or alternate-stream root component"));
        }
        current.push(name);
        paths.push(current.clone());
    }
    if paths.len() < 2 {
        return Err(refuse("drive root cannot be enrolled"));
    }
    Ok(paths)
}

/// A temporarily pinned observation, not a trusted registered vault.
/// The handles do not guarantee the original names cannot be renamed.
/// Even a successful path recheck is only a snapshot, never authorization.
pub(crate) struct ObservedRoot {
    _ancestors: Vec<OpenedDirectory>,
    identity: RootIdentity,
}

impl ObservedRoot {
    pub(crate) fn inspect_untrusted_path(path: &Path) -> io::Result<Self> {
        let components = local_components(path)?;
        let mut handles = Vec::with_capacity(components.len());
        for component in components {
            handles.push(OpenedDirectory::open(&component)?);
        }
        let identity = handles
            .last()
            .ok_or_else(|| refuse("missing final root handle"))?
            .identity()?;
        Ok(Self {
            _ancestors: handles,
            identity,
        })
    }

    pub(crate) fn identity(&self) -> RootIdentity {
        self.identity
    }

    /// Reopen and compare a claimed path to the pinned object. Missing or
    /// substituted names must HOLD; even a match is only a snapshot.
    pub(crate) fn still_names_this_root(&self, path: &Path) -> io::Result<bool> {
        let latest = Self::inspect_untrusted_path(path)?;
        Ok(latest.identity() == self.identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::{self, Command};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(name: &str) -> PathBuf {
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let root = std::env::temp_dir().join(format!(
            "akai-root-id-{name}-{}-{}",
            process::id(),
            seed.as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn same_opened_root_matches_and_independent_roots_differ() {
        assert_eq!(size_of::<RootIdentity>(), 24);
        assert_eq!(size_of::<HandleInfo>(), 52);
        let dir = fixture("distinct");
        let x = dir.join("x");
        let y = dir.join("y");
        fs::create_dir(&x).unwrap();
        fs::create_dir(&y).unwrap();
        let first = ObservedRoot::inspect_untrusted_path(&x).unwrap();
        let again = ObservedRoot::inspect_untrusted_path(&x).unwrap();
        let other = ObservedRoot::inspect_untrusted_path(&y).unwrap();
        assert_eq!(first.identity(), again.identity());
        assert_ne!(first.identity(), other.identity());
        let moved = dir.join("moved");
        if fs::rename(&x, &moved).is_ok() {
            // Windows may rename a directory despite pinned non-delete-
            // sharing HANDLEs. Its identity survives, its name does not.
            assert!(first.still_names_this_root(&x).is_err());
            let renamed = ObservedRoot::inspect_untrusted_path(&moved).unwrap();
            assert_eq!(first.identity(), renamed.identity());
            drop(renamed);
            fs::create_dir(&x).unwrap();
            assert!(!first.still_names_this_root(&x).unwrap());
        } else {
            assert!(first.still_names_this_root(&x).unwrap());
        }
        drop(first);
        drop(again);
        drop(other);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_file_relative_path_unc_and_parent_traversal() {
        let dir = fixture("invalid");
        let x = dir.join("x");
        fs::create_dir(&x).unwrap();
        let file = x.join("note.md");
        fs::write(&file, b"not a directory").unwrap();
        assert!(ObservedRoot::inspect_untrusted_path(&file).is_err());
        assert!(ObservedRoot::inspect_untrusted_path(Path::new("relative")).is_err());
        assert!(
            ObservedRoot::inspect_untrusted_path(Path::new(r"\\server\share\not-local")).is_err()
        );
        assert!(ObservedRoot::inspect_untrusted_path(&x.join("..").join("x")).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn junction_on_final_path_is_never_followed() {
        let dir = fixture("junction");
        let x = dir.join("real");
        let alias = dir.join("alias");
        fs::create_dir(&x).unwrap();
        let status = Command::new("cmd.exe")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&alias)
            .arg(&x)
            .status()
            .unwrap();
        assert!(status.success(), "Windows junction fixture must exist");
        assert!(ObservedRoot::inspect_untrusted_path(&alias).is_err());
        fs::remove_dir(&alias).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
