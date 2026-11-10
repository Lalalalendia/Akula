//! Keep Windows parent folders pinned across a whole knowledge-base copy.
//! Fail closed when directories are busy, linked, renamed or too numerous.
use std::io;
use std::path::Path;

#[cfg(windows)]
pub(super) struct DirectoryPins(Vec<std::fs::File>);

#[cfg(windows)]
impl DirectoryPins {
    pub(super) fn acquire(root: &Path) -> io::Result<Self> {
        let mut pins = Self(Vec::new());
        pins.visit(root, 0)?;
        Ok(pins)
    }

    fn visit(&mut self, path: &Path, depth: usize) -> io::Result<()> {
        use std::fs::{self, OpenOptions};
        use std::os::windows::fs::OpenOptionsExt;

        if depth > super::MAX_DEPTH || self.0.len() >= 8_192 {
            return Err(io::Error::other("directory pin limit exceeded"));
        }
        if super::fingerprint(&fs::symlink_metadata(path)?)?.kind != super::EntryKind::Folder {
            return Err(io::Error::other("linked or invalid source directory"));
        }
        // BACKUP_SEMANTICS opens directories, OPEN_REPARSE_POINT exposes links
        // without following them. No SHARE_DELETE blocks directory renames.
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        let mut opts = OpenOptions::new();
        opts.read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
        let handle = opts.open(path)?;
        if super::fingerprint(&handle.metadata()?)?.kind != super::EntryKind::Folder {
            return Err(io::Error::other("opened source directory is linked or invalid"));
        }
        self.0.push(handle);
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if aquilum_core::search::paths::is_hidden_name(&entry.file_name()) {
                continue;
            }
            let kind = super::fingerprint(&fs::symlink_metadata(entry.path())?)?.kind;
            if kind == super::EntryKind::Folder {
                self.visit(&entry.path(), depth + 1)?;
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
pub(super) struct DirectoryPins;

#[cfg(not(windows))]
impl DirectoryPins {
    pub(super) fn acquire(_root: &Path) -> io::Result<Self> {
        Ok(Self)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn pinned_root_and_subfolder_cannot_be_renamed_until_handles_close() {
        let temp = crate::test_dir::tempdir();
        let root = temp.path().join("Knowledge");
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("note.md"), "keep").unwrap();
        let pins = DirectoryPins::acquire(&root).unwrap();
        assert_eq!(pins.0.len(), 2, "both root and nested folder must be pinned");
        assert!(
            std::fs::rename(&nested, root.join("renamed")).is_err(),
            "nested path must stay pinned while a copy is in progress",
        );
        assert!(
            std::fs::rename(&root, temp.path().join("renamed-root")).is_err(),
            "root path must stay pinned while a copy is in progress",
        );
        drop(pins);
        std::fs::rename(&nested, root.join("renamed")).unwrap();
    }
}
