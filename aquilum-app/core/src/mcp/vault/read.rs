//! Pin Unix MCP read-only note I/O to held directory descriptors.
use super::Vault;
use std::path::Path;

impl Vault {
    /// Only MCP read_note / note_outline use this read-only primitive.
    /// A pinned Unix parent prevents a post-preflight symlink swap from
    /// redirecting a leaf open outside the selected Vault.
    pub(crate) fn read_note_bytes(&self, path: &Path) -> Result<Vec<u8>, String> {
        #[cfg(unix)]
        {
            read_unix(self, path)
        }
        #[cfg(not(unix))]
        {
            // This is still a preflight, not race-free Windows containment.
            self.check_existing_components(path)?;
            std::fs::read(path).map_err(|error| error.to_string())
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::{Path, Vault};
    use rustix::fs::{openat, Mode, OFlags};
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io::{Read, Result as IoResult};
    use std::path::Component;

    fn open_dir(parent: &File, component: &OsStr) -> IoResult<File> {
        let fd = openat(parent, component,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty())?;
        Ok(File::from(fd))
    }

    fn open_root(root: &Path) -> IoResult<File> {
        if !root.is_absolute() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,
                "MCP Vault root must be absolute"));
        }
        // Anchor at the filesystem root: no ancestor of the selected Vault
        // can be swapped to a symlink between lexical preflight and open.
        let mut dir = File::open("/")?;
        for part in root.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(name) => dir = open_dir(&dir, name)?,
                _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,
                    "Invalid MCP Vault root component")),
            }
        }
        Ok(dir)
    }

    fn open_regular(parent: &File, component: &OsStr) -> IoResult<File> {
        // NONBLOCK prevents a FIFO/device from hanging before type checking.
        let fd = openat(parent, component,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty())?;
        let file = File::from(fd);
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,
                "MCP Vault reads require regular files"));
        }
        Ok(file)
    }

    pub(super) fn read_unix(vault: &Vault, path: &Path) -> Result<Vec<u8>, String> {
        let relative = path.strip_prefix(&vault.root)
            .map_err(|_| format!("Path outside MCP Vault: {}", path.display()))?;
        let mut parts = relative.components().peekable();
        // Only normal path components can be opened. Reject '.', '..', roots,
        // and an empty or trailing-directory-only path fail-closed.
        let mut directory = open_root(&vault.root).map_err(|error| error.to_string())?;
        if !directory.metadata().map_err(|error| error.to_string())?.is_dir() {
            return Err("MCP Vault root is not a directory".to_owned());
        }
        loop {
            let component = parts.next().ok_or("Empty MCP note path")?;
            let Component::Normal(name) = component else {
                return Err("Non-normal component in MCP note path".to_owned());
            };
            if parts.peek().is_none() {
                let mut file = open_regular(&directory, name)
                    .map_err(|error| error.to_string())?;
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
                return Ok(bytes);
            }
            directory = open_dir(&directory, name).map_err(|error| error.to_string())?;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{open_dir, open_regular, open_root, read_unix};
        use crate::mcp::vault::Vault;
        use std::ffi::OsStr;
        use std::fs::{self, File};
        use std::io::Read;
        use std::os::unix::fs::symlink;

        fn fixture() -> (tempfile::TempDir, Vault) {
            let dir = tempfile::tempdir().unwrap();
            let root = fs::canonicalize(dir.path()).unwrap();
            fs::create_dir_all(root.join("Проекты")).unwrap();
            fs::write(root.join("Проекты/Глава.md"), "содержимое").unwrap();
            (dir, Vault { root, visible: true })
        }

        #[test]
        fn normal_cyrillic_note_and_rejected_leaf_symlink() {
            let (_dir, vault) = fixture();
            assert_eq!(read_unix(&vault, &vault.root.join("Проекты/Глава.md")).unwrap(),
                "содержимое".as_bytes());
            let outside = tempfile::tempdir().unwrap();
            fs::write(outside.path().join("secret.md"), b"secret").unwrap();
            symlink(outside.path().join("secret.md"), vault.root.join("Проекты/Escape.md")).unwrap();
            assert!(read_unix(&vault, &vault.root.join("Проекты/Escape.md")).is_err());
            symlink(outside.path(), vault.root.join("External")).unwrap();
            assert!(read_unix(&vault, &vault.root.join("External/secret.md")).is_err());
            assert!(read_unix(&vault, &vault.root.join("../other.md")).is_err());
        }

        #[test]
        fn replaced_root_path_cannot_redirect_pinned_read() {
            let parent = tempfile::tempdir().unwrap();
            let root = parent.path().join("vault");
            fs::create_dir(&root).unwrap();
            fs::write(root.join("Note.md"), b"inside").unwrap();
            let outside = tempfile::tempdir().unwrap();
            fs::write(outside.path().join("Note.md"), b"outside").unwrap();
            let vault = Vault { root: root.clone(), visible: true };
            fs::rename(&root, parent.path().join("vault-old")).unwrap();
            symlink(outside.path(), &root).unwrap();
            assert!(read_unix(&vault, &root.join("Note.md")).is_err());
            assert!(open_root(&root).is_err());
            assert_eq!(fs::read(outside.path().join("Note.md")).unwrap(), b"outside");
        }

        #[test]
        fn held_directory_survives_adversarial_rename_and_replaced_symlink() {
            let (_dir, vault) = fixture();
            let outside = tempfile::tempdir().unwrap();
            fs::write(outside.path().join("Глава.md"), "outside secret").unwrap();
            let root_fd = open_root(&vault.root).unwrap();
            let held = open_dir(&root_fd, OsStr::new("Проекты")).unwrap();
            fs::rename(vault.root.join("Проекты"), vault.root.join("detached")).unwrap();
            symlink(outside.path(), vault.root.join("Проекты")).unwrap();
            let mut pinned_note = open_regular(&held, OsStr::new("Глава.md")).unwrap();
            let mut content = String::new();
            pinned_note.read_to_string(&mut content).unwrap();
            assert_eq!(content, "содержимое", "held descriptor changed destination");
            assert!(read_unix(&vault, &vault.root.join("Проекты/Глава.md")).is_err());
            assert_eq!(fs::read_to_string(outside.path().join("Глава.md")).unwrap(), "outside secret");
        }
    }
}
#[cfg(unix)]
use unix::read_unix;
