//! Windows 7-compatible directory HANDLE-relative MCP note reads.
//! Reuse the Sync NT open implementation: no pathname open after the volume root.
use super::super::Vault;
use crate::sync::pinned_windows::{file_identity_with_links, open_child, open_root};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

fn open_anchored_root(root: &Path) -> Result<File, String> {
    let mut parts = root.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return Err("MCP Vault root needs a Windows volume/share prefix".to_owned());
    };
    if !matches!(parts.next(), Some(Component::RootDir)) {
        return Err("MCP Vault root must be absolute".to_owned());
    }
    // The drive/share root cannot be redirected by replacing a child name.
    // Every remaining parent (including the Vault root) is opened with
    // NtCreateFile(RootDirectory=held HANDLE, FILE_OPEN_REPARSE_POINT).
    let anchor = PathBuf::from(prefix.as_os_str()).join("\\");
    let mut directory = open_root(&anchor).map_err(|error| format!("{error:?}"))?;
    for part in parts {
        let Component::Normal(name) = part else {
            return Err("Non-normal MCP Vault root component".to_owned());
        };
        directory = open_child(&directory,
            name.to_str().ok_or("Non-Unicode MCP Vault root component")?, false)
            .map_err(|error| format!("{error:?}"))?;
    }
    Ok(directory)
}

pub(super) fn read_windows(vault: &Vault, path: &Path) -> Result<Vec<u8>, String> {
    let relative = path.strip_prefix(&vault.root)
        .map_err(|_| "MCP note path is outside selected Vault".to_owned())?;
    let mut names = relative.components().peekable();
    let mut directory = open_anchored_root(&vault.root)?;
    while let Some(part) = names.next() {
        let Component::Normal(name) = part else {
            return Err("Non-normal MCP note component".to_owned());
        };
        let leaf = names.peek().is_none();
        let file = open_child(&directory,
            name.to_str().ok_or("Non-Unicode MCP note name")?, leaf)
            .map_err(|error| format!("{error:?}"))?;
        if leaf {
            // Hardlinking an outside file into the Vault must not grant read
            // access to the external inode. Use the same pinned file HANDLE.
            let (_volume, _identity, links) = file_identity_with_links(&file)
                .map_err(|error| format!("{error:?}"))?;
            if links != 1 {
                return Err("MCP note has multiple hard links".to_owned());
            }
            let mut opened = file;
            let mut bytes = Vec::new();
            opened.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
            return Ok(bytes);
        }
        directory = file;
    }
    Err("MCP note path cannot be empty".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{open_anchored_root, read_windows};
    use crate::mcp::vault::Vault;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    fn fixture() -> (tempfile::TempDir, Vault) {
        let base = tempfile::tempdir().unwrap();
        let root = crate::search::paths::canonical_path(base.path());
        fs::create_dir(root.join("Раздел")).unwrap();
        fs::write(root.join("Раздел/Заметка.md"), "текст").unwrap();
        (base, Vault { root, visible: true })
    }

    fn junction(link: &Path, destination: &Path) {
        let escape = |path: &Path| path.to_string_lossy().replace('\'', "''");
        let script = format!(
            "New-Item -ItemType Junction -Path '{}' -Target '{}' -ErrorAction Stop | Out-Null",
            escape(link), escape(destination)
        );
        let result = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output().unwrap();
        assert!(result.status.success(),
            "Windows junction fixture failed: {}",
            String::from_utf8_lossy(&result.stderr));
    }

    #[test]
    fn reads_unicode_note_through_pinned_windows_handles() {
        let (_base, vault) = fixture();
        assert_eq!(read_windows(&vault, &vault.root.join("Раздел/Заметка.md")).unwrap(),
            "текст".as_bytes());
        assert!(read_windows(&vault, &vault.root.join("../outside.md")).is_err());
    }

    #[test]
    fn refuses_reparse_directory_and_root_replacement() {
        let (_base, vault) = fixture();
        let outsider = tempfile::tempdir().unwrap();
        fs::write(outsider.path().join("secret.md"), b"outside secret").unwrap();
        junction(&vault.root.join("Junction"), outsider.path());
        assert!(read_windows(&vault, &vault.root.join("Junction/secret.md")).is_err());

        // Replace a directory *after* the caller selected the Vault.
        let held = open_anchored_root(&vault.root).unwrap();
        let original = vault.root.join("Раздел");
        let moved = vault.root.join("detached");
        fs::rename(&original, &moved).unwrap();
        junction(&original, outsider.path());
        assert!(read_windows(&vault, &original.join("secret.md")).is_err());
        assert!(held.metadata().unwrap().is_dir());
        assert_eq!(fs::read(outsider.path().join("secret.md")).unwrap(), b"outside secret");
    }

    #[test]
    fn rejects_external_file_hardlink() {
        let (base, vault) = fixture();
        let outsider = base.path().join("external.md");
        fs::write(&outsider, b"not granted").unwrap();
        let alias = vault.root.join("hardlink.md");
        fs::hard_link(&outsider, &alias).unwrap();
        assert!(read_windows(&vault, &alias).is_err());
        assert_eq!(fs::read(&outsider).unwrap(), b"not granted");
    }
}
