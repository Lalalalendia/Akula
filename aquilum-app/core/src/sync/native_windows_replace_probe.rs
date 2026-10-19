//! Windows-only P0e falsifier for native existing-file replacement.
//!
//! TEST ONLY: no production call-site, File Gate grant, S3 executor, receipt,
//! replay permission or ConfirmedBaseline transition. ReplaceFileW is a
//! name-based operation, NOT a compare-and-swap on the old file's HANDLE.
//! Uncertain results retain all names for operator inspection.
#![cfg(all(test, windows))]

use super::{pinned_windows, OperationId, PinnedFileIdentity};
use std::ffi::c_void;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

const MAX_BYTES: usize = 32 * 1024 * 1024;

#[link(name = "kernel32")]
extern "system" {
    fn ReplaceFileW(
        replaced: *const u16,
        replacement: *const u16,
        backup: *const u16,
        flags: u32,
        exclude: *mut c_void,
        reserved: *mut c_void,
    ) -> i32;
}

#[derive(Debug, PartialEq, Eq)]
enum ProbeError {
    Invalid,
    Io(String),
    // ALL errors after entering ReplaceFileW are ambiguous. The Win32 API
    // can rename files / transfer attributes even when it reports failure.
    Ambiguous(String),
}

fn io(err: impl std::fmt::Debug) -> ProbeError {
    ProbeError::Io(format!("{err:?}"))
}

fn identity(file: &File) -> Result<PinnedFileIdentity, ProbeError> {
    if !file.metadata().map_err(io)?.is_file() {
        return Err(ProbeError::Invalid);
    }
    let (volume, file_id, links) =
        pinned_windows::file_identity_with_links(file).map_err(io)?;
    if links != 1 || file_id == 0 || volume == 0 {
        return Err(ProbeError::Invalid);
    }
    Ok(PinnedFileIdentity {
        device: u64::from(volume),
        file: file_id,
    })
}

fn read_child(
    root: &File,
    name: &str,
    limit: usize,
) -> Result<(PinnedFileIdentity, Vec<u8>), ProbeError> {
    let handle = pinned_windows::open_child(root, name, true).map_err(io)?;
    let before = identity(&handle)?;
    let mut content = Vec::new();
    (&handle)
        .take(limit as u64 + 1)
        .read_to_end(&mut content)
        .map_err(io)?;
    if content.len() > limit || identity(&handle)? != before {
        return Err(ProbeError::Invalid);
    }
    Ok((before, content))
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

fn valid_leaf(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with(".akula-")
        && !name.ends_with('.')
        && !name.ends_with(' ')
        && !name.chars().any(|c| {
            matches!(c, '/' | '\\' | ':' | '\0' | '*' | '?' | '"' | '<' | '>' | '|')
                || c.is_control()
        })
}

fn stage_name(op: OperationId) -> String {
    format!(".akula-win-stage-{op}")
}

fn backup_name(op: OperationId) -> String {
    format!(".akula-win-backup-{op}")
}

/// Returns a backup *name for inspection only*. This never yields a receipt.
/// Root is created by the disposable test fixture; not an S3-selected path.
fn run(
    root_path: &Path,
    leaf: &str,
    op: OperationId,
    expected_id: PinnedFileIdentity,
    original: &[u8],
    updated: &[u8],
    before: impl FnOnce(),
    after: impl FnOnce(),
) -> Result<String, ProbeError> {
    if !valid_leaf(leaf)
        || original.len() > MAX_BYTES
        || updated.len() > MAX_BYTES
    {
        return Err(ProbeError::Invalid);
    }
    let root = pinned_windows::open_root_for_create(root_path).map_err(io)?;
    let root_id = pinned_windows::file_identity(&root).map_err(io)?;
    if read_child(&root, leaf, original.len())?
        != (expected_id, original.to_vec())
    {
        return Err(ProbeError::Invalid);
    }

    let stage_leaf = stage_name(op);
    let backup_leaf = backup_name(op);
    // Do not overwrite an existing backup, even in a throwaway test directory.
    if pinned_windows::open_optional_child(&root, &backup_leaf)
        .map_err(io)?
        .is_some()
    {
        return Err(ProbeError::Invalid);
    }
    let mut stage =
        pinned_windows::create_stage_child(&root, &stage_leaf).map_err(io)?;
    stage.write_all(updated).map_err(io)?;
    stage.sync_all().map_err(io)?;
    let staged_id = identity(&stage)?;
    drop(stage);

    // Controlled racing-writer injection. In production this gap cannot be
    // authorized by a pathname precheck alone.
    before();
    let observed_root =
        pinned_windows::open_root(root_path).map_err(io)?;
    if pinned_windows::file_identity(&root).map_err(io)? != root_id
        || pinned_windows::file_identity(&observed_root).map_err(io)? != root_id
        || read_child(&root, leaf, original.len())?
            != (expected_id, original.to_vec())
        || read_child(&root, &stage_leaf, updated.len())?
            != (staged_id, updated.to_vec())
        || pinned_windows::open_optional_child(&root, &backup_leaf)
            .map_err(io)?
            .is_some()
    {
        return Err(ProbeError::Invalid);
    }
    drop(observed_root);

    let dest_path = root_path.join(leaf);
    let stage_path = root_path.join(&stage_leaf);
    let backup_path = root_path.join(&backup_leaf);
    let dest_wide = wide(&dest_path);
    let stage_wide = wide(&stage_path);
    let backup_wide = wide(&backup_path);

    // ReplaceFileW is not fd-relative. Even after a successful last check,
    // another same-UID writer can alter any NAME before this syscall.
    // Flags=0: REPLACEFILE_WRITE_THROUGH is documented unsupported.
    let result = unsafe {
        ReplaceFileW(
            dest_wide.as_ptr(),
            stage_wide.as_ptr(),
            backup_wide.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        // Error 1175/1176/1177 can mean distinct namespace outcomes.
        // Never retry or cleanup automatically, regardless of error code.
        return Err(ProbeError::Ambiguous(
            std::io::Error::last_os_error().to_string(),
        ));
    }

    after(); // Panic/lost-ACK test hook; not a power-cut or persisted receipt.
    let dest = read_child(&root, leaf, updated.len())
        .map_err(|e| ProbeError::Ambiguous(format!("new-name audit: {e:?}")))?;
    let backup = read_child(&root, &backup_leaf, original.len())
        .map_err(|e| ProbeError::Ambiguous(format!("backup audit: {e:?}")))?;
    if dest != (staged_id, updated.to_vec())
        || backup != (expected_id, original.to_vec())
        || pinned_windows::open_optional_child(&root, &stage_leaf)
            .map_err(|e| ProbeError::Ambiguous(format!("stage audit: {e:?}")))?
            .is_some()
        || pinned_windows::file_identity(&root)
            .map_err(|e| ProbeError::Ambiguous(format!("root audit: {e:?}")))?
            != root_id
    {
        return Err(ProbeError::Ambiguous(
            "postflight identity or byte mismatch".into(),
        ));
    }

    // Stage had File::sync_all before publication. Windows offers no proven
    // portable parent-directory fsync here. Do NOT mint a native receipt.
    Ok(backup_leaf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    const OLD: &[u8] = b"original confirmed";
    const NEW: &[u8] = b"updated cloud bytes";

    fn fixture() -> (tempfile::TempDir, PinnedFileIdentity) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.md"), OLD).unwrap();
        let root = pinned_windows::open_root(dir.path()).unwrap();
        let (old_id, content) = read_child(&root, "file.md", OLD.len()).unwrap();
        assert_eq!(content, OLD);
        (dir, old_id)
    }

    #[test]
    fn replacefilew_preserves_original_in_named_backup() {
        let (dir, old_id) = fixture();
        let op = OperationId::new();
        let backup = run(dir.path(), "file.md", op, old_id, OLD, NEW, || {}, || {})
            .expect("native ReplaceFileW must succeed for local disposable files");
        assert_eq!(fs::read(dir.path().join("file.md")).unwrap(), NEW);
        assert_eq!(fs::read(dir.path().join(backup)).unwrap(), OLD);
        assert!(!dir.path().join(stage_name(op)).exists());
    }

    #[test]
    fn existing_stage_and_backup_must_not_be_replaced() {
        for collision_backup in [false, true] {
            let (dir, old_id) = fixture();
            let op = OperationId::new();
            let collision = if collision_backup { backup_name(op) } else { stage_name(op) };
            fs::write(dir.path().join(&collision), b"foreign sentinel").unwrap();
            assert!(run(dir.path(), "file.md", op, old_id, OLD, NEW, || {}, || {})
                .is_err());
            assert_eq!(fs::read(dir.path().join("file.md")).unwrap(), OLD);
            assert_eq!(fs::read(dir.path().join(collision)).unwrap(), b"foreign sentinel");
        }
    }

    #[test]
    fn stage_name_substitution_with_identical_bytes_is_refused() {
        let (dir, old_id) = fixture();
        let op = OperationId::new();
        let staged = dir.path().join(stage_name(op));
        let parked = dir.path().join("parked-stage");
        let result = run(dir.path(), "file.md", op, old_id, OLD, NEW, || {
            fs::rename(&staged, &parked).unwrap();
            fs::write(&staged, NEW).unwrap();
        }, || {});
        assert_eq!(result, Err(ProbeError::Invalid));
        assert_eq!(fs::read(dir.path().join("file.md")).unwrap(), OLD);
        assert_eq!(fs::read(parked).unwrap(), NEW);
    }

    #[test]
    fn original_name_substitution_with_identical_bytes_is_refused() {
        let (dir, old_id) = fixture();
        let op = OperationId::new();
        let target = dir.path().join("file.md");
        let parked = dir.path().join("parked-original");
        let result = run(dir.path(), "file.md", op, old_id, OLD, NEW, || {
            fs::rename(&target, &parked).unwrap();
            fs::write(&target, OLD).unwrap();
        }, || {});
        assert_eq!(result, Err(ProbeError::Invalid));
        assert_eq!(fs::read(&target).unwrap(), OLD);
        assert_eq!(fs::read(&parked).unwrap(), OLD);
    }

    #[test]
    fn sharing_denial_is_ambiguous_and_keeps_forensic_names() {
        let (dir, old_id) = fixture();
        let op = OperationId::new();
        let mut locked = None;
        let result = run(dir.path(), "file.md", op, old_id, OLD, NEW, || {
            // Omit FILE_SHARE_DELETE. ReplaceFileW must not bypass this lock.
            locked = Some(OpenOptions::new().read(true).share_mode(0x3)
                .open(dir.path().join("file.md")).unwrap());
        }, || {});
        assert!(matches!(result, Err(ProbeError::Ambiguous(_))),
            "Win32 failure may mutate metadata but cannot authorize retry: {result:?}");
        drop(locked);
        // Inspect, never repair. This lock-denial fixture leaves both names.
        assert_eq!(fs::read(dir.path().join("file.md")).unwrap(), OLD);
        assert_eq!(fs::read(dir.path().join(stage_name(op))).unwrap(), NEW);
    }

    #[test]
    fn lost_ack_after_success_keeps_backup_not_a_baseline() {
        let (dir, old_id) = fixture();
        let op = OperationId::new();
        let outcome = std::panic::catch_unwind(|| {
            let _ = run(dir.path(), "file.md", op, old_id, OLD, NEW, || {}, || {
                panic!("simulated ACK loss; no journal confirmation");
            });
        });
        assert!(outcome.is_err());
        assert_eq!(fs::read(dir.path().join("file.md")).unwrap(), NEW);
        assert_eq!(fs::read(dir.path().join(backup_name(op))).unwrap(), OLD);
        // This is NOT a second-process SIGKILL, disk failure or durable receipt.
    }
}
