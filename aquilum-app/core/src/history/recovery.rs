//! Fail-closed, workspace-local intent record for history directory relocation.
//!
//! This is a pathname-based, cooperating-process checkpoint, not a trusted
//! interprocess lock or a handle-relative filesystem mutation capability.

use super::store;
use crate::files::error::FileCommandError;
use crate::search::paths::{identity, same_path};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SCHEMA: u32 = 1;
const MAX_INTENT_BYTES: u64 = 16 * 1024 * 1024;
const JOURNAL_FOLDER: &str = "history-moves";
const JOURNAL_FILE: &str = "pending-v1.json";

#[derive(Debug, Serialize, Deserialize)]
struct MoveIntent {
    schema: u32,
    vault: String,
    kind: String,
    from_note: String,
    to_note: String,
    from_dir: String,
    to_dir: String,
    /// Relative history filename -> BLAKE3 of its exact bytes.
    /// The union must remain unchanged as files move between directories.
    expected: BTreeMap<String, String>,
    same_tree: bool,
}

fn denied(message: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, message.into())
}

fn journal_dir(vault: &Path) -> PathBuf {
    vault.join(store::AQUILUM_FOLDER).join(JOURNAL_FOLDER)
}

pub(super) fn marker_path(vault: &Path) -> PathBuf {
    journal_dir(vault).join(JOURNAL_FILE)
}

/// Reject symbolic links, Windows junctions and other reparse points rather
/// than trusting Path::exists/is_file (which follow filesystem aliases).
fn ordinary(path: &Path, expected_dir: bool) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x0000_0400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if metadata.file_type().is_symlink()
        || reparse
        || (expected_dir && !metadata.is_dir())
        || (!expected_dir && !metadata.is_file())
    {
        return Err(denied(format!("unsafe history recovery entry: {}", path.display())));
    }
    Ok(true)
}

fn checked_journal(vault: &Path) -> io::Result<PathBuf> {
    // The strict history prefix walk verifies the registered vault and every
    // existing ancestor without following symlinks or NTFS reparse points.
    store::checked_history_recovery_directory(vault)?;
    let dir = journal_dir(vault);
    ordinary(&dir, true)?;
    Ok(dir)
}

#[cfg(unix)]
fn sync_dir(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_path: &Path) -> io::Result<()> {
    // FlushFileBuffers on the intent file is used on Windows, but there is
    // no certified directory fsync here. Do not claim power-loss durability.
    Ok(())
}

fn ensure_journal(vault: &Path) -> io::Result<PathBuf> {
    let directory = checked_journal(vault)?;
    let meta = vault.join(store::AQUILUM_FOLDER);
    if !ordinary(&meta, true)? {
        fs::create_dir(&meta)?;
        sync_dir(vault)?;
    }
    if !ordinary(&directory, true)? {
        fs::create_dir(&directory)?;
        sync_dir(&meta)?;
    }
    Ok(directory)
}

fn read_pending(vault: &Path) -> io::Result<Option<MoveIntent>> {
    let dir = checked_journal(vault)?;
    if !ordinary(&dir, true)? {
        return Ok(None);
    }
    let marker = marker_path(vault);
    if !ordinary(&marker, false)? {
        return Ok(None);
    }
    let mut file = File::open(marker)?;
    if file.metadata()?.len() > MAX_INTENT_BYTES {
        return Err(denied("oversize history recovery intent"));
    }
    let mut bytes = Vec::new();
    (&mut file).take(MAX_INTENT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INTENT_BYTES {
        return Err(denied("oversize history recovery intent"));
    }
    let intent: MoveIntent =
        serde_json::from_slice(&bytes).map_err(|e| denied(format!("invalid history recovery intent: {e}")))?;
    if intent.schema != SCHEMA || intent.vault != identity(vault)
        || intent.kind.is_empty() || intent.from_note.is_empty() || intent.to_note.is_empty()
        || intent.from_dir.is_empty() || intent.to_dir.is_empty()
    {
        return Err(denied("invalid history recovery intent identity or schema"));
    }
    Ok(Some(intent))
}

pub(super) fn pending(vault: &Path) -> io::Result<bool> {
    Ok(read_pending(vault)?.is_some())
}

fn fingerprint(path: &Path) -> io::Result<String> {
    let mut input = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut bytes = [0_u8; 16 * 1024];
    loop {
        let n = input.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hasher.update(&bytes[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// No recursive symlinks, junctions, non-regular entries, unreadable files,
/// unrepresentable names or silent enumeration failures are allowed.
fn inventory(directory: &Path) -> io::Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    if !ordinary(directory, true)? {
        return Ok(result);
    }
    for entry in WalkDir::new(directory).follow_links(false).min_depth(1) {
        let entry = entry.map_err(io::Error::other)?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(path)?;
        #[cfg(windows)]
        let reparse = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x0000_0400 != 0
        };
        #[cfg(not(windows))]
        let reparse = false;
        if metadata.file_type().is_symlink() || reparse {
            return Err(denied("history version tree contains a symlink or reparse point"));
        }
        if metadata.is_dir() {
            continue;
        }
        if !metadata.is_file() {
            return Err(denied("history version tree contains a non-regular file"));
        }
        let relative = path.strip_prefix(directory).map_err(io::Error::other)?;
        let name = relative.to_str()
            .ok_or_else(|| denied("history version filename is not valid UTF-8"))?
            .replace('\\', "/");
        if result.insert(name, fingerprint(path)?).is_some() {
            return Err(denied("duplicate normalized history version filename"));
        }
    }
    Ok(result)
}

fn union(
    a: &BTreeMap<String, String>,
    b: &BTreeMap<String, String>,
) -> io::Result<BTreeMap<String, String>> {
    let mut result = a.clone();
    for (file, digest) in b {
        if result.insert(file.clone(), digest.clone()).is_some() {
            return Err(denied(format!("conflicting version filename: {file}")));
        }
    }
    Ok(result)
}

fn verify_split(
    from: &Path,
    to: &Path,
    expected: &BTreeMap<String, String>,
    same_tree: bool,
) -> io::Result<()> {
    let observed = if same_tree {
        inventory(to)?
    } else {
        union(&inventory(from)?, &inventory(to)?)?
    };
    if &observed != expected {
        return Err(denied("history recovery inventory differs from the recorded file digests"));
    }
    Ok(())
}

fn write_intent(vault: &Path, intent: &MoveIntent) -> io::Result<()> {
    let dir = ensure_journal(vault)?;
    let serialized = serde_json::to_vec(intent).map_err(io::Error::other)?;
    if serialized.len() as u64 > MAX_INTENT_BYTES {
        return Err(denied("history recovery intent is too large"));
    }
    // O_EXCL is a narrow admission gate for cooperating processes; it does
    // not solve all cross-process races or network-filesystem semantics.
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(marker_path(vault))?;
    file.write_all(&serialized)?;
    file.sync_all()?;
    sync_dir(&dir)?;
    Ok(())
}

fn clear_intent(vault: &Path) -> io::Result<()> {
    let marker = marker_path(vault);
    if !ordinary(&marker, false)? {
        return Err(denied("history recovery marker disappeared before verification"));
    }
    fs::remove_file(&marker)?;
    let dir = journal_dir(vault);
    sync_dir(&dir)?;
    // Best-effort removal of empty bookkeeping directories; never delete
    // nonempty or foreign directories, or the workspace root.
    if fs::remove_dir(&dir).is_ok() {
        let meta = vault.join(store::AQUILUM_FOLDER);
        sync_dir(&meta)?;
        if fs::remove_dir(&meta).is_ok() {
            sync_dir(vault)?;
        }
    }
    Ok(())
}

/// File Gate has *already moved the primary note*. This function journals
/// and verifies the separate version history before returning an ack.
/// It never deletes or repairs unknown bytes.
pub(super) fn transfer(
    vault: &Path,
    kind: &str,
    from_note: &Path,
    to_note: &Path,
    from_base: &Path,
    to_base: &Path,
    from: &Path,
    to: &Path,
    attempt: impl FnOnce() -> Result<(), FileCommandError>,
) -> io::Result<()> {
    store::checked_history_note_directory(from_base, from)?;
    store::checked_history_note_directory(to_base, to)?;
    let same_tree = same_path(from, to);
    let existing = read_pending(vault)?;
    let manifest = if let Some(existing) = existing {
        if existing.kind != kind || existing.from_note != identity(from_note)
            || existing.to_note != identity(to_note)
            || existing.from_dir != identity(from) || existing.to_dir != identity(to)
            || existing.same_tree != same_tree
        {
            return Err(denied("another unfinished history transfer owns this workspace"));
        }
        existing
    } else {
        let from_files = inventory(from)?;
        let to_files = if same_tree { BTreeMap::new() } else { inventory(to)? };
        if from_files.is_empty() {
            if !to_files.is_empty() {
                return Err(denied("history source absent but destination already contains versions"));
            }
            return Ok(());
        }
        let expected = union(&from_files, &to_files)?;
        let manifest = MoveIntent {
            schema: SCHEMA,
            vault: identity(vault),
            kind: kind.to_owned(),
            from_note: identity(from_note),
            to_note: identity(to_note),
            from_dir: identity(from),
            to_dir: identity(to),
            expected,
            same_tree,
        };
        write_intent(vault, &manifest)?;
        manifest
    };

    // Verify every recorded byte is still present on precisely one side.
    // An absent source is NOT sufficient evidence of a successful transfer.
    verify_split(from, to, &manifest.expected, same_tree)?;
    attempt().map_err(|error| io::Error::other(error.to_string()))?;
    verify_split(from, to, &manifest.expected, same_tree)?;
    if (!same_tree && !inventory(from)?.is_empty()) || inventory(to)? != manifest.expected {
        return Err(denied("history transfer has not finished moving all recorded files"));
    }
    clear_intent(vault)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_rejects_deleted_version_even_when_source_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let a = vault.join(".aquilum/history/a.md");
        let b = vault.join(".aquilum/history/b.md");
        fs::create_dir_all(&a).unwrap();
        fs::write(a.join("1_device_agent.md"), b"original").unwrap();
        let from_note = vault.join("a.md");
        let to_note = vault.join("b.md");
        let fail = transfer(vault, "move", &from_note, &to_note, vault, vault, &a, &b, || {
            fs::create_dir_all(&b).unwrap();
            fs::rename(a.join("1_device_agent.md"), b.join("1_device_agent.md")).unwrap();
            Err(FileCommandError::Io { message: "interrupted".into() })
        });
        assert!(fail.is_err());
        assert!(pending(vault).unwrap());
        // Source no longer exists as a populated history, but the destination
        // cannot be trusted once one of its committed snapshots disappears.
        fs::remove_file(b.join("1_device_agent.md")).unwrap();
        assert!(transfer(vault, "move", &from_note, &to_note, vault, vault, &a, &b, || Ok(())).is_err());
        assert!(pending(vault).unwrap());
    }

    #[test]
    fn inventory_same_history_tree_is_not_treated_as_duplicate() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let a = vault.join(".aquilum/history/idea.md");
        fs::create_dir_all(&a).unwrap();
        fs::write(a.join("1_device_me.md"), b"one").unwrap();
        transfer(
            vault, "move", &vault.join("idea.md"), &vault.join("idea.md"),
            vault, vault, &a, &a, || Ok(())
        ).unwrap();
        assert_eq!(inventory(&a).unwrap().len(), 1);
        assert!(!pending(vault).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_recovery_directory_is_never_followed() {
        use std::os::unix::fs::symlink;
        let vault = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        fs::create_dir(vault.path().join(".aquilum")).unwrap();
        symlink(external.path(), journal_dir(vault.path())).unwrap();
        assert!(pending(vault.path()).is_err());
        let source = vault.path().join(".aquilum/history/source.md");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("1_device_me.md"), "private").unwrap();
        let before = fs::read_dir(external.path()).unwrap().count();
        assert!(transfer(
            vault.path(), "move", &vault.path().join("source.md"),
            &vault.path().join("target.md"), vault.path(), vault.path(),
            &source, &vault.path().join(".aquilum/history/target.md"), || Ok(())
        ).is_err());
        assert_eq!(fs::read_dir(external.path()).unwrap().count(), before);
    }

    #[cfg(windows)]
    #[test]
    fn junction_recovery_directory_is_never_followed() {
        use std::process::Command;
        let vault = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(vault.path().join(".aquilum")).unwrap();
        let target = journal_dir(vault.path());
        let output = Command::new("cmd")
            .arg("/C").arg("mklink").arg("/J")
            .arg(&target).arg(outside.path()).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(pending(vault.path()).is_err());
        fs::remove_dir(target).unwrap();
    }

    #[test]
    fn malformed_recovery_marker_never_allows_history_access() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        ensure_journal(vault).unwrap();
        fs::write(marker_path(vault), b"{\"incomplete\":").unwrap();
        assert!(pending(vault).is_err());
    }

    #[test]
    fn complete_target_can_be_verified_after_source_disappeared() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path();
        let a = vault.join(".aquilum/history/from.md");
        let b = vault.join(".aquilum/history/to.md");
        fs::create_dir_all(&a).unwrap();
        fs::write(a.join("1_device_agent.md"), b"original").unwrap();
        let from_note = vault.join("from.md");
        let to_note = vault.join("to.md");
        let _ = transfer(vault, "move", &from_note, &to_note, vault, vault, &a, &b, || {
            fs::rename(&a, &b).unwrap();
            Err(FileCommandError::Io { message: "interrupted after rename".into() })
        });
        assert!(pending(vault).unwrap());
        transfer(vault, "move", &from_note, &to_note, vault, vault, &a, &b, || Ok(())).unwrap();
        assert!(!pending(vault).unwrap());
        assert_eq!(fs::read(b.join("1_device_agent.md")).unwrap(), b"original");
    }
}
