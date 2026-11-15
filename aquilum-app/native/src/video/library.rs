//! Per-device video storage. This directory must never be a vault attachment,
//! S3 input, Dropbox/Yandex source, or part of Markdown state.
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

pub const SINGLE_VIDEO_MAX: u64 = 2 * 1024 * 1024 * 1024;
pub const LIBRARY_MAX: u64 = 20 * 1024 * 1024 * 1024;
const EXTENSIONS: &[&str] = &["mp4", "webm", "mkv", "mov", "m4v"];

fn denied(reason: &str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, reason)
}

/// The normal Windows app profile is in roaming %APPDATA%. Never store video
/// there: instead use %LOCALAPPDATA%. Explicit isolated test profiles use a
/// sibling media directory under the explicitly supplied test profile.
pub fn default_root(app_data: &Path) -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    if crate::workspace::data_dir_or_default(None).ok().as_deref() == Some(app_data) {
        let local = std::env::var_os("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| denied("LOCALAPPDATA unavailable; local video storage disabled"))?;
        return Ok(PathBuf::from(local).join(crate::workspace::APP_ID).join("videos"));
    }
    Ok(app_data.join("local-videos"))
}

fn alias(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return meta.file_type().is_symlink() || meta.file_attributes() & 0x0000_0400 != 0;
    }
    #[cfg(not(windows))]
    { meta.file_type().is_symlink() }
}

fn outside_vaults(root: &Path, vaults: &[PathBuf]) -> io::Result<()> {
    for vault in vaults {
        // Unknown registered roots are not permission to store in a
        // potentially synchronized directory.
        let canonical = fs::canonicalize(vault)?;
        if root.starts_with(&canonical) || canonical.starts_with(root) {
            return Err(denied("local video library overlaps a registered workspace"));
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Library {
    root: PathBuf,
}

impl Library {
    pub fn new(app_data: &Path, registered_roots: &[PathBuf]) -> io::Result<Self> {
        Self::at(default_root(app_data)?, registered_roots)
    }

    pub fn at(root: PathBuf, registered_roots: &[PathBuf]) -> io::Result<Self> {
        if !root.is_absolute() {
            return Err(denied("video library path must be absolute"));
        }
        // Reject obvious overlap before writing anything to the destination.
        outside_vaults(&root, registered_roots)?;
        fs::create_dir_all(&root)?;
        let meta = fs::symlink_metadata(&root)?;
        if !meta.is_dir() || alias(&meta) {
            return Err(denied("local video library is an alias or not a directory"));
        }
        let physical = fs::canonicalize(&root)?;
        outside_vaults(&physical, registered_roots)?;
        Ok(Self { root: physical })
    }

    pub fn root(&self) -> &Path { &self.root }

    pub fn existing(&self, id: &str) -> io::Result<Option<PathBuf>> {
        if !valid(id) {
            return Err(denied("invalid video id"));
        }
        for ext in EXTENSIONS {
            let candidate = self.root.join(format!("{id}.{ext}"));
            let meta = match fs::symlink_metadata(&candidate) {
                Ok(meta) => meta,
                Err(e) if e.kind() == ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            if !meta.is_file() || alias(&meta) || meta.len() == 0 || meta.len() > SINGLE_VIDEO_MAX {
                return Err(denied("local video file is unsafe or outside size limits"));
            }
            return Ok(Some(candidate));
        }
        Ok(None)
    }

    pub fn published(&self, id: &str, ext: &str) -> io::Result<PathBuf> {
        if !valid(id) || !EXTENSIONS.contains(&ext) {
            return Err(denied("invalid video identifier or extension"));
        }
        Ok(self.root.join(format!("{id}.{ext}")))
    }

    pub fn used_bytes(&self) -> io::Result<u64> {
        let mut total = 0_u64;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let meta = fs::symlink_metadata(entry.path())?;
            if meta.is_file() && !alias(&meta) {
                total = total.saturating_add(meta.len());
            }
        }
        Ok(total)
    }
}

fn valid(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_device_library_is_not_a_vault_attachment() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("book");
        let local = dir.path().join("device-media");
        fs::create_dir_all(&vault).unwrap();
        let library = Library::at(local.clone(), &[vault.clone()]).unwrap();
        assert!(!library.root().starts_with(&vault));
        assert!(library.existing("dQw4w9WgXcQ").unwrap().is_none());
        let file = library.published("dQw4w9WgXcQ", "mp4").unwrap();
        fs::write(&file, b"synthetic test").unwrap();
        assert_eq!(library.existing("dQw4w9WgXcQ").unwrap(), Some(file));
        assert_eq!(library.used_bytes().unwrap(), 14);
    }

    #[test]
    fn reject_vault_inside_library_and_library_inside_vault() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("book");
        fs::create_dir_all(&vault).unwrap();
        assert!(Library::at(vault.join("Videos"), &[vault.clone()]).is_err());
        assert!(!vault.join("Videos").exists(), "overlap rejected before new folder is made");
        assert!(Library::at(dir.path().to_path_buf(), &[vault]).is_err());
    }

    #[test]
    fn no_symlink_or_empty_file_is_a_valid_download() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::at(dir.path().join("local"), &[]).unwrap();
        assert!(library.published("../badpath", "mp4").is_err());
        assert!(library.published("dQw4w9WgXcQ", "exe").is_err());
        let target = library.published("dQw4w9WgXcQ", "mp4").unwrap();
        fs::write(&target, b"").unwrap();
        assert!(library.existing("dQw4w9WgXcQ").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn link_into_vault_does_not_bypass_local_only_boundary() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("book");
        fs::create_dir_all(&vault).unwrap();
        let link = dir.path().join("local_link");
        symlink(&vault, &link).unwrap();
        assert!(Library::at(link, &[vault]).is_err());
    }
}
