//! Fail-closed, workspace-local intent record for history directory relocation.
//!
//! This is a pathname-based, cooperating-process checkpoint, not a trusted
//! interprocess lock or a handle-relative filesystem mutation capability.

use super::store;
use crate::files::error::FileCommandError;
use crate::search::paths::{identity, same_path, strip_root};
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
const CROSS_VAULT_HOLD_FILE: &str = "cross-vault-hold-v1.json";
const MAX_CROSS_VAULT_HOLD_BYTES: u64 = 4096;

// This is a read-only, untrusted stop signal. No production writer, pair
// admission, move, cleanup, or automatic release exists in 04g-3b-7c-a.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CrossVaultHoldSide {
    Source,
    Destination,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CrossVaultHold {
    schema: u32,
    operation_id: String,
    side: CrossVaultHoldSide,
    source_vault: String,
    destination_vault: String,
    source_note: String,
    destination_note: String,
    primary_blake3: String,
    history_inventory_blake3: String,
}

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

fn cross_vault_hold_path(vault: &Path) -> PathBuf {
    journal_dir(vault).join(CROSS_VAULT_HOLD_FILE)
}

/// A single side is enough to stop local history changes, including on a
/// destination-only Core after restart. Even a matching pair is NOT a
/// prepare/commit authorization: two pathname markers are not atomic and
/// cannot exclude independent or noncooperating processes.
fn read_cross_vault_hold(vault: &Path) -> io::Result<bool> {
    let dir = checked_journal(vault)?;
    if !ordinary(&dir, true)? {
        return Ok(false);
    }
    let marker = cross_vault_hold_path(vault);
    if !ordinary(&marker, false)? {
        return Ok(false);
    }
    let mut file = File::open(&marker)?;
    if file.metadata()?.len() > MAX_CROSS_VAULT_HOLD_BYTES {
        return Err(denied("oversize cross-vault hold marker"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_CROSS_VAULT_HOLD_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CROSS_VAULT_HOLD_BYTES {
        return Err(denied("oversize cross-vault hold marker"));
    }
    let record: CrossVaultHold = serde_json::from_slice(&bytes)
        .map_err(|error| denied(format!("invalid cross-vault hold marker: {error}")))?;
    let transaction = uuid::Uuid::parse_str(&record.operation_id)
        .map_err(|_| denied("invalid cross-vault hold transaction id"))?;
    let lowercase_blake3 = |value: &str| {
        value.len() == 64 && value.bytes().all(|byte| {
            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
        })
    };
    let checked_note = |root: &str, note: &str| {
        let path = Path::new(note);
        let root = Path::new(root);
        root.is_absolute()
            && path.is_absolute()
            && !root.components().any(|part| {
                matches!(part, std::path::Component::ParentDir | std::path::Component::CurDir)
            })
            && !path.components().any(|part| {
                matches!(part, std::path::Component::ParentDir | std::path::Component::CurDir)
            })
            && strip_root(root, path).is_some_and(|relative| !relative.as_os_str().is_empty())
    };
    let expected_side_vault = match record.side {
        CrossVaultHoldSide::Source => &record.source_vault,
        CrossVaultHoldSide::Destination => &record.destination_vault,
    };
    if record.schema != 1
        || transaction.is_nil()
        || record.source_vault == record.destination_vault
        || identity(Path::new(&record.source_vault)) != record.source_vault
        || identity(Path::new(&record.destination_vault)) != record.destination_vault
        || expected_side_vault != &identity(vault)
        || !checked_note(&record.source_vault, &record.source_note)
        || !checked_note(&record.destination_vault, &record.destination_note)
        || !lowercase_blake3(&record.primary_blake3)
        || !lowercase_blake3(&record.history_inventory_blake3)
    {
        return Err(denied("invalid cross-vault hold identity, digest or schema"));
    }
    Ok(true)
}

pub(super) fn pending(vault: &Path) -> io::Result<bool> {
    // Check both records, even if the legacy marker already exists. Never let
    // one valid journal conceal corruption or an unsafe alias in the other.
    let legacy = read_pending(vault)?;
    let bilateral_hold = read_cross_vault_hold(vault)?;
    Ok(legacy.is_some() || bilateral_hold)
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

    fn write_cross_vault_test_hold(
        owner: &Path,
        source: &Path,
        destination: &Path,
        side: &str,
    ) -> serde_json::Value {
        let record = serde_json::json!({
            "schema": 1,
            "operation_id": "550e8400-e29b-41d4-a716-446655440000",
            "side": side,
            "source_vault": identity(source),
            "destination_vault": identity(destination),
            "source_note": identity(&source.join("Original.md")),
            "destination_note": identity(&destination.join("Transferred.md")),
            "primary_blake3": blake3::hash(b"original").to_hex().to_string(),
            "history_inventory_blake3": blake3::hash(b"inventory").to_hex().to_string(),
        });
        ensure_journal(owner).unwrap();
        fs::write(cross_vault_hold_path(owner), serde_json::to_vec(&record).unwrap()).unwrap();
        record
    }

    #[test]
    fn history_cross_vault_hold_marker_is_visible_from_either_independent_root() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        assert!(!pending(&source).unwrap());
        assert!(!pending(&destination).unwrap());
        write_cross_vault_test_hold(&source, &source, &destination, "source");
        assert!(pending(&source).unwrap());
        assert!(!pending(&destination).unwrap());
        write_cross_vault_test_hold(&destination, &source, &destination, "destination");
        assert!(pending(&source).unwrap());
        assert!(pending(&destination).unwrap());
        // Both matching markers remain HOLD, never an implicit commit receipt.
        assert!(cross_vault_hold_path(&source).exists());
        assert!(cross_vault_hold_path(&destination).exists());
    }

    #[test]
    fn history_cross_vault_hold_rejects_corruption_future_fields_and_wrong_owner() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        let good = write_cross_vault_test_hold(&destination, &source, &destination, "destination");
        let marker = cross_vault_hold_path(&destination);

        for broken in [
            b"{\"partial\":".to_vec(),
            vec![b'A'; 4097],
            serde_json::to_vec(&serde_json::json!({"schema": 99})).unwrap(),
        ] {
            fs::write(&marker, broken).unwrap();
            assert!(pending(&destination).is_err());
        }
        for patch in [
            serde_json::json!({"schema": 2}),
            serde_json::json!({"side": "source"}),
            serde_json::json!({"source_vault": identity(&destination)}),
            serde_json::json!({"operation_id": "not-a-uuid"}),
            serde_json::json!({"primary_blake3": "forged"}),
            serde_json::json!({"unexpected_write_authority": true}),
        ] {
            let mut record = good.clone();
            for (key, value) in patch.as_object().unwrap() {
                record[key] = value.clone();
            }
            fs::write(&marker, serde_json::to_vec(&record).unwrap()).unwrap();
            assert!(pending(&destination).is_err(), "unsafe record accepted: {record}");
        }
        fs::write(&marker, serde_json::to_vec(&good).unwrap()).unwrap();
        assert!(pending(&destination).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn history_cross_vault_hold_symlink_marker_is_never_followed() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        let _good = write_cross_vault_test_hold(&source, &source, &destination, "source");
        let outside = temp.path().join("outside.json");
        fs::rename(cross_vault_hold_path(&source), &outside).unwrap();
        symlink(&outside, cross_vault_hold_path(&source)).unwrap();
        assert!(pending(&source).is_err());
        assert!(fs::read(outside).unwrap().starts_with(b"{"));
    }

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
