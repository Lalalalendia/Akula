//! Stage 04b-1: real Akula workspace read/preflight bridge.
//!
//! This is deliberately NOT a VaultPort executor: standard pathname reads,
//! even with repeated symlink checks, cannot prove against a hostile TOCTOU
//! rename/symlink race. No snapshot here authorizes an upload, deletion or
//! local overwrite. PR 04b-2 must integrate a host-granted file-handle
//! authority (and native editor buffers) before any destructive operation.

use super::{reject_path_collisions, resolve_no_symlinks, LocalFileEntry, LocalFileSnapshot,
    SyncRelativePath, VaultFileKind, VaultPortError};
use crate::app_core::Core;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalScanTrust {
    /// Scan is useful for UX preview/diagnostics ONLY. It is not a stable,
    /// atomic filesystem snapshot and cannot establish a verified absence.
    PreflightOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalScanPreview {
    pub entries: Vec<LocalFileEntry>,
    pub trust: LocalScanTrust,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanLimits {
    pub max_files: usize,
    pub max_file_bytes: u64,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self { max_files: 10_000, max_file_bytes: 32 * 1024 * 1024 }
    }
}

/// A read-only bridge from the configured Akula workspace to a *preview*
/// snapshot. Does not implement VaultPort, intentionally: callers cannot
/// turn a non-atomic directory walk into deletion authority by accident.
pub struct CoreVaultScanner<'a> {
    core: &'a Core,
    root: PathBuf,
    limits: ScanLimits,
}

impl<'a> CoreVaultScanner<'a> {
    pub fn new(core: &'a Core, workspace_root: &Path, limits: ScanLimits)
        -> Result<Self, VaultPortError>
    {
        if limits.max_files == 0 || limits.max_file_bytes == 0 {
            return Err(VaultPortError::PermissionDenied);
        }
        let root = fs::canonicalize(workspace_root)
            .map_err(|error| VaultPortError::Io(error.to_string()))?;
        if !root.is_dir() || fs::symlink_metadata(workspace_root)
            .map_err(|error| VaultPortError::Io(error.to_string()))?
            .file_type().is_symlink()
        {
            return Err(VaultPortError::UnsafePath);
        }

        // Use the current on-disk UI workspace registry, not a path provided
        // by an untrusted remote manifest or arbitrary caller string.
        let configured = core.ui_state.workspace_roots()
            .map_err(|_| VaultPortError::PermissionDenied)?;
        let allowed = configured.iter().any(|registered| {
            fs::canonicalize(registered).ok().as_deref() == Some(root.as_path())
        });
        if !allowed {
            return Err(VaultPortError::PermissionDenied);
        }
        Ok(Self { core, root, limits })
    }

    fn check_root(&self) -> Result<(), VaultPortError> {
        if fs::symlink_metadata(&self.root)
            .map_err(|error| VaultPortError::Io(error.to_string()))?
            .file_type().is_symlink()
            || fs::canonicalize(&self.root)
                .map_err(|error| VaultPortError::Io(error.to_string()))? != self.root
        {
            return Err(VaultPortError::UnsafePath);
        }
        let registered = self.core.ui_state.workspace_roots()
            .map_err(|_| VaultPortError::PermissionDenied)?;
        if !registered.iter().any(|path| {
            fs::canonicalize(path).ok().as_deref() == Some(self.root.as_path())
        }) {
            return Err(VaultPortError::PermissionDenied);
        }
        Ok(())
    }

    fn relative(&self, absolute: &Path) -> Result<SyncRelativePath, VaultPortError> {
        let inner = absolute.strip_prefix(&self.root).map_err(|_| VaultPortError::UnsafePath)?;
        let mut components = Vec::new();
        for component in inner.components() {
            let Component::Normal(name) = component else { return Err(VaultPortError::UnsafePath) };
            let name = name.to_str().ok_or(VaultPortError::UnsafePath)?;
            components.push(name);
        }
        SyncRelativePath::parse(&components.join("/")).map_err(|_| VaultPortError::UnsafePath)
    }

    fn read_bounded(&self, relative: &SyncRelativePath)
        -> Result<LocalFileSnapshot, VaultPortError>
    {
        self.check_root()?;
        let path = resolve_no_symlinks(&self.root, relative)
            .map_err(|_| VaultPortError::UnsafePath)?;
        let before = fs::symlink_metadata(&path)
            .map_err(|error| VaultPortError::Io(error.to_string()))?;
        if before.file_type().is_symlink() || !before.is_file() {
            return Err(VaultPortError::UnsafePath);
        }
        if before.len() > self.limits.max_file_bytes {
            return Err(VaultPortError::PermissionDenied);
        }
        let file = File::open(&path).map_err(|error| VaultPortError::Io(error.to_string()))?;
        let mut bytes = Vec::new();
        file.take(self.limits.max_file_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| VaultPortError::Io(error.to_string()))?;
        if bytes.len() as u64 > self.limits.max_file_bytes {
            return Err(VaultPortError::PermissionDenied);
        }
        let after = fs::symlink_metadata(&path)
            .map_err(|error| VaultPortError::Io(error.to_string()))?;
        if after.file_type().is_symlink() || !after.is_file()
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
            || after.len() != bytes.len() as u64
        {
            return Err(VaultPortError::FileChanged);
        }
        self.check_root()?;
        Ok(LocalFileSnapshot {
            content_hash: blake3::hash(&bytes).to_hex().to_string(),
            bytes,
        })
    }

    /// Preview all reachable files, failing on symbolic links, incomplete
    /// traversal, malformed portable names, duplicate case/NFC identities,
    /// file-count or byte bounds. Never skip an error and interpret it as a
    /// legitimate remote/local deletion.
    pub fn scan_preflight(&self) -> Result<LocalScanPreview, VaultPortError> {
        self.check_root()?;
        let mut entries = Vec::new();
        for next in WalkDir::new(&self.root).follow_links(false).sort_by_file_name().into_iter() {
            let item = next.map_err(|error| VaultPortError::Io(error.to_string()))?;
            if item.path() == self.root { continue; }
            if item.file_type().is_symlink() {
                return Err(VaultPortError::UnsafePath);
            }
            // Reject special OS filesystem objects: only regular files and
            // directories can participate in a preview.
            if item.file_type().is_dir() { continue; }
            if !item.file_type().is_file() {
                return Err(VaultPortError::UnsafePath);
            }
            if entries.len() >= self.limits.max_files {
                return Err(VaultPortError::PermissionDenied);
            }
            let relative = self.relative(item.path())?;
            let snapshot = self.read_bounded(&relative)?;
            entries.push(LocalFileEntry {
                kind: if relative.as_str().to_ascii_lowercase().ends_with(".md") {
                    VaultFileKind::Markdown
                } else {
                    VaultFileKind::Attachment
                },
                byte_length: snapshot.bytes.len() as u64,
                path: relative,
                content_hash: snapshot.content_hash,
            });
        }
        reject_path_collisions(entries.iter().map(|item| &item.path))
            .map_err(|_| VaultPortError::UnsafePath)?;
        self.check_root()?;
        Ok(LocalScanPreview { entries, trust: LocalScanTrust::PreflightOnly })
    }

    /// Individual read preview. Not a retained file descriptor and not a
    /// capability to authorize remote upload or guarded local mutation.
    pub fn read_preflight(&self, relative: &SyncRelativePath)
        -> Result<LocalFileSnapshot, VaultPortError>
    {
        self.read_bounded(relative)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::{Core, CoreEvent, EventSink};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Sink(Mutex<Vec<String>>);
    impl EventSink for Sink {
        fn emit(&self, event: CoreEvent) {
            self.0.lock().unwrap().push(event.name().into());
        }
    }

    fn fixture() -> (tempfile::TempDir, Arc<Core>, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        fs::create_dir_all(&root).unwrap();
        let appdata = temp.path().join("appdata");
        fs::create_dir_all(&appdata).unwrap();
        let core = Core::open(&appdata, Arc::new(Sink::default()));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        (temp, core, root)
    }

    #[test]
    fn registered_workspace_preview_hashes_real_markdown_and_attachments() {
        let (_temp, core, root) = fixture();
        fs::create_dir_all(root.join("Notes")).unwrap();
        fs::write(root.join("Notes").join("a.md"), b"hello\n").unwrap();
        fs::write(root.join("image.png"), [0, 255, 42, 0]).unwrap();
        let scanner = CoreVaultScanner::new(&core, &root, ScanLimits::default()).unwrap();
        let result = scanner.scan_preflight().unwrap();
        assert_eq!(result.trust, LocalScanTrust::PreflightOnly);
        assert_eq!(result.entries.len(), 2);
        let note = SyncRelativePath::parse("Notes/a.md").unwrap();
        let original = scanner.read_preflight(&note).unwrap();
        assert_eq!(original.bytes, b"hello\n");
        assert_eq!(original.content_hash, blake3::hash(b"hello\n").to_hex().to_string());
        assert!(result.entries.iter().any(|entry| entry.path == note
            && entry.kind == VaultFileKind::Markdown));
    }

    #[test]
    fn arbitrary_unregistered_directory_is_not_scannable() {
        let (_temp, core, root) = fixture();
        let outside = tempfile::tempdir().unwrap();
        assert!(matches!(
            CoreVaultScanner::new(&core, outside.path(), ScanLimits::default()),
            Err(VaultPortError::PermissionDenied)
        ));
        assert!(CoreVaultScanner::new(&core, &root, ScanLimits::default()).is_ok());
    }

    #[test]
    fn scanner_refuses_duplicate_portable_case_names() {
        let (_temp, core, root) = fixture();
        fs::write(root.join("AAA.md"), b"first").unwrap();
        fs::write(root.join("aaa.md"), b"second").unwrap();
        let scanner = CoreVaultScanner::new(&core, &root, ScanLimits::default()).unwrap();
        // On case-insensitive hosts the second write targets the same file;
        // on case-sensitive hosts it becomes a genuine portable collision.
        if fs::read_dir(&root).unwrap().count() == 2 {
            assert_eq!(scanner.scan_preflight(), Err(VaultPortError::UnsafePath));
        }
    }

    #[test]
    fn excessive_file_count_and_byte_limit_are_fatal_not_partial_listing() {
        let (_temp, core, root) = fixture();
        fs::write(root.join("one.md"), b"12345").unwrap();
        fs::write(root.join("two.md"), b"67890").unwrap();
        let capped = CoreVaultScanner::new(&core, &root, ScanLimits {
            max_files: 1, max_file_bytes: 10,
        }).unwrap();
        assert_eq!(capped.scan_preflight(), Err(VaultPortError::PermissionDenied));
        let too_large = CoreVaultScanner::new(&core, &root, ScanLimits {
            max_files: 10, max_file_bytes: 2,
        }).unwrap();
        assert_eq!(too_large.scan_preflight(), Err(VaultPortError::PermissionDenied));
    }

    #[test]
    fn an_existing_symlink_fails_closed_instead_of_being_scanned() {
        let (_temp, core, root) = fixture();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = tempfile::tempdir().unwrap();
            fs::write(target.path().join("secret.md"), b"secret").unwrap();
            symlink(target.path(), root.join("escape")).unwrap();
            let scanner = CoreVaultScanner::new(&core, &root, ScanLimits::default()).unwrap();
            assert_eq!(scanner.scan_preflight(), Err(VaultPortError::UnsafePath));
            assert!(matches!(scanner.read_preflight(
                &SyncRelativePath::parse("escape/secret.md").unwrap()),
                Err(VaultPortError::UnsafePath)));
        }
    }

    #[test]
    fn corrupted_or_changed_root_registry_revokes_preview_authority() {
        let (_temp, core, root) = fixture();
        let scanner = CoreVaultScanner::new(&core, &root, ScanLimits::default()).unwrap();
        let workspace = core.ui_state.list_workspaces().unwrap();
        core.ui_state.forget_workspace(workspace[0].id).unwrap();
        assert_eq!(scanner.scan_preflight(), Err(VaultPortError::PermissionDenied));
    }
}
