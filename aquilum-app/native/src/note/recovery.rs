//! Staged DESK-15 primitive: integrity-checked, no-overwrite recovery candidates.
//! NOT wired to note edits or startup yet. The record is not an authenticated
//! signature and cannot prove zero data loss before its durable-write ACK.
#![allow(dead_code)]
mod queue;
pub(super) use queue::{CaptureStatus, SnapshotQueue};

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAGIC: &[u8] = b"AKULA-NOTE-SHADOW-V1";
const MAX_TEXT: usize = 16 * 1024 * 1024;
const MAX_WIRE: usize = 48 * 1024 * 1024;
const MAX_RECORDS: usize = 256;

#[derive(Serialize, Deserialize)]
struct Shadow {
    version: u8,
    workspace: String,
    relative: String,
    baseline: String,
    revision: u64,
    body_hash: String,
    body: String,
}

pub(super) struct Candidate {
    pub(super) revision: u64,
    pub(super) body: String,
    pub(super) baseline: String,
    pub(super) source_changed: bool,
    pub(super) already_saved: bool,
    pub(super) file: PathBuf,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

#[cfg(target_os = "windows")]
fn reparse(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}

#[cfg(not(target_os = "windows"))]
fn reparse(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

fn checked_relative(vault: &Path, document: &Path) -> io::Result<String> {
    let relative = document.strip_prefix(vault).map_err(|_| denied("document outside workspace"))?;
    if relative.as_os_str().is_empty() || relative.components().any(|part| !matches!(part, Component::Normal(_))) {
        return Err(denied("unnormalized document path"));
    }
    if reparse(&fs::symlink_metadata(vault)?) {
        return Err(denied("workspace is a link/reparse point"));
    }
    let mut walked = vault.to_path_buf();
    for part in relative.components() {
        walked.push(part.as_os_str());
        if reparse(&fs::symlink_metadata(&walked)?) {
            return Err(denied("document path includes link/reparse point"));
        }
    }
    let root = fs::canonicalize(vault)?;
    let resolved = fs::canonicalize(document)?;
    if !resolved.starts_with(&root) || resolved == root || !fs::metadata(document)?.is_file() {
        return Err(denied("document does not resolve to a regular workspace file"));
    }
    relative.to_str().map(str::to_owned).ok_or_else(|| denied("non-UTF8 document path requires review"))
}

fn checked_directory(path: &Path, create: bool) -> io::Result<bool> {
    if create {
        match fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !reparse(&meta) => Ok(true),
        Ok(_) => Err(denied("recovery directory is a link or non-directory")),
        Err(error) if !create && error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn folder(data: &Path, workspace: Uuid, create: bool) -> io::Result<Option<PathBuf>> {
    let data_meta = fs::symlink_metadata(data)?;
    if !data_meta.is_dir() || reparse(&data_meta) {
        return Err(denied("application data is a link/reparse point"));
    }
    let base = data.join("akula-note-recovery-v1");
    let bucket = base.join(workspace.to_string());
    // Verify each segment BEFORE traversing or chmod: never follow a
    // pre-existing symlink/junction into an attacker-selected location.
    if !checked_directory(&base, create)? || !checked_directory(&bucket, create)? {
        return Ok(None);
    }
    #[cfg(unix)]
    if create {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&bucket, fs::Permissions::from_mode(0o700))?;
    }
    Ok(Some(bucket))
}

fn key(relative: &str) -> String {
    blake3::hash(relative.as_bytes()).to_hex().to_string()
}

fn wire(image: &Shadow) -> io::Result<Vec<u8>> {
    let payload = serde_json::to_vec(image).map_err(io::Error::other)?;
    if payload.len() > MAX_WIRE - MAGIC.len() - 4 - 32 {
        return Err(invalid("snapshot exceeds maximum wire size"));
    }
    let mut bytes = Vec::with_capacity(MAGIC.len() + 4 + payload.len() + 32);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&payload);
    let digest = blake3::hash(&bytes);
    bytes.extend_from_slice(digest.as_bytes());
    Ok(bytes)
}

fn decode(bytes: &[u8]) -> io::Result<Shadow> {
    if !bytes.starts_with(MAGIC) || bytes.len() < MAGIC.len() + 4 + 32 || bytes.len() > MAX_WIRE {
        return Err(invalid("invalid recovery header/length"));
    }
    let offset = MAGIC.len();
    let count = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    let end = offset + 4 + count;
    if end.checked_add(32) != Some(bytes.len()) {
        return Err(invalid("truncated or trailing recovery bytes"));
    }
    if blake3::hash(&bytes[..end]).as_bytes().as_slice() != &bytes[end..] {
        return Err(invalid("recovery checksum mismatch"));
    }
    let record: Shadow = serde_json::from_slice(&bytes[offset + 4..end]).map_err(io::Error::other)?;
    if record.version != 1
        || record.body.len() > MAX_TEXT
        || blake3::hash(record.body.as_bytes()).to_hex().as_str() != record.body_hash
    {
        return Err(invalid("invalid version or recovered body hash"));
    }
    Ok(record)
}

fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || reparse(&meta) || meta.len() > MAX_WIRE as u64 {
        return Err(denied("unsafe or oversized recovery record"));
    }
    let mut bytes = Vec::new();
    File::open(path)?.take((MAX_WIRE + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_WIRE {
        return Err(invalid("recovery record changed size while reading"));
    }
    Ok(bytes)
}

/// Stage an immutable snapshot in private AppData; never touch the document.
/// A successful return means the snapshot file was synced and linked without
/// overwriting another revision. Parent-dir power-loss durability is separate.
pub(super) fn stage(
    data: &Path,
    vault: &Path,
    workspace: Uuid,
    document: &Path,
    baseline: &str,
    revision: u64,
    body: &str,
) -> io::Result<PathBuf> {
    if body.len() > MAX_TEXT || baseline.len() != 64 || !baseline.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid snapshot body or source digest"));
    }
    let relative = checked_relative(vault, document)?;
    let bucket = folder(data, workspace, true)?.ok_or_else(|| invalid("missing recovery directory"))?;
    let target = bucket.join(format!("{}-{revision:020}.shadow", key(&relative)));
    let image = Shadow {
        version: 1,
        workspace: workspace.to_string(),
        relative,
        baseline: baseline.to_owned(),
        revision,
        body_hash: blake3::hash(body.as_bytes()).to_hex().to_string(),
        body: body.to_owned(),
    };
    let bytes = wire(&image)?;
    let pending = bucket.join(format!(".{}.pending", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&pending)?;
    let written = file.write_all(&bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(&pending);
        return Err(error);
    }
    // A hard link creates the final name atomically with NO-REPLACE semantics,
    // unlike POSIX rename, which silently overwrites an existing record.
    let published = fs::hard_link(&pending, &target);
    let _ = fs::remove_file(&pending);
    match published {
        Ok(()) => {
            #[cfg(unix)]
            File::open(&bucket)?.sync_all()?;
            Ok(target)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && read_bounded(&target)? == bytes => Ok(target),
        Err(error) => Err(error),
    }
}

fn records(bucket: &Path, prefix: &str) -> io::Result<Vec<PathBuf>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(bucket)? {
        let entry = entry?;
        let filename = entry.file_name();
        let name = filename.to_str().ok_or_else(|| invalid("unreadable snapshot filename"))?;
        if name.starts_with(prefix) {
            if !name.ends_with(".shadow") || name.len() != prefix.len() + 20 + ".shadow".len()
                || !name[prefix.len()..prefix.len() + 20].bytes().all(|b| b.is_ascii_digit())
            {
                return Err(invalid("unrecognized snapshot revision"));
            }
            names.push(entry.path());
            if names.len() > MAX_RECORDS {
                return Err(invalid("too many snapshots, manual review required"));
            }
        }
    }
    names.sort();
    Ok(names)
}

fn verified_record(path: &Path, workspace: Uuid, relative: &str) -> io::Result<Shadow> {
    let record = decode(&read_bounded(path)?)?;
    if record.workspace != workspace.to_string() || record.relative != relative {
        return Err(invalid("snapshot identity mismatch"));
    }
    let expected = format!("{}-{:020}.shadow", key(relative), record.revision);
    if path.file_name().and_then(|n| n.to_str()) != Some(expected.as_str()) {
        return Err(invalid("snapshot revision does not match filename"));
    }
    Ok(record)
}

/// Return the newest candidate for explicit user review; NEVER write to Vault.
pub(super) fn inspect(
    data: &Path,
    vault: &Path,
    workspace: Uuid,
    document: &Path,
) -> io::Result<Option<Candidate>> {
    let relative = checked_relative(vault, document)?;
    let Some(bucket) = folder(data, workspace, false)? else { return Ok(None) };
    let prefix = format!("{}-", key(&relative));
    let mut names = records(&bucket, &prefix)?;
    let Some(path) = names.pop() else { return Ok(None) };
    let record = verified_record(&path, workspace, &relative)?;
    let disk = aquilum_core::files::document::read_file_hash_impl(document)
        .map_err(|_| denied("cannot verify current source, manual review required"))?;
    Ok(Some(Candidate {
        revision: record.revision,
        body: record.body,
        baseline: record.baseline.clone(),
        source_changed: disk != record.baseline,
        already_saved: disk == record.body_hash,
        file: path,
    }))
}

/// Only an exact recorded revision that is already present on disk may lose
/// its backup. An asynchronous save request is never sufficient evidence.
pub(super) fn clear_after_verified_save(
    data: &Path, vault: &Path, workspace: Uuid, document: &Path, revision: u64,
) -> io::Result<bool> {
    let Some(candidate) = inspect(data, vault, workspace, document)? else { return Ok(false) };
    if candidate.revision != revision || !candidate.already_saved {
        return Ok(false);
    }
    // Once the newest revision has an exact on-disk ACK, all older
    // snapshots for this same note are superseded. Validate the entire set
    // BEFORE deleting any record: a corrupted older backup requires review
    // and must not be concealed by partial garbage collection.
    let relative = checked_relative(vault, document)?;
    let Some(bucket) = folder(data, workspace, false)? else { return Ok(false) };
    let names = records(&bucket, &format!("{}-", key(&relative)))?;
    for path in &names {
        let record = verified_record(path, workspace, &relative)?;
        if record.revision > revision {
            return Ok(false);
        }
    }
    for path in names {
        fs::remove_file(path)?;
    }
    #[cfg(unix)]
    File::open(&bucket)?.sync_all()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(path: &Path) -> String {
        blake3::hash(&fs::read(path).unwrap()).to_hex().to_string()
    }

    #[test]
    fn two_tabs_keep_utf8_drafts_without_touching_the_source() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let workspace = Uuid::new_v4();
        let a = vault.path().join("one.md");
        let b = vault.path().join("two.md");
        fs::write(&a, "first").unwrap();
        fs::write(&b, "second").unwrap();
        stage(data.path(), vault.path(), workspace, &a, &digest(&a), 1, "новое 😀").unwrap();
        stage(data.path(), vault.path(), workspace, &b, &digest(&b), 2, "другое 🌐").unwrap();
        assert_eq!(inspect(data.path(), vault.path(), workspace, &a).unwrap().unwrap().body, "новое 😀");
        assert_eq!(inspect(data.path(), vault.path(), workspace, &b).unwrap().unwrap().body, "другое 🌐");
        assert_eq!(fs::read_to_string(&a).unwrap(), "first");
        assert_eq!(fs::read_to_string(&b).unwrap(), "second");
        assert!(inspect(data.path(), vault.path(), Uuid::new_v4(), &a).unwrap().is_none());
    }

    #[test]
    fn refuses_corruption_and_same_revision_overwrite() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let doc = vault.path().join("note.md");
        fs::write(&doc, "base").unwrap();
        let id = Uuid::new_v4();
        let base = digest(&doc);
        let path = stage(data.path(), vault.path(), id, &doc, &base, 7, "draft").unwrap();
        assert!(stage(data.path(), vault.path(), id, &doc, &base, 7, "different").is_err());
        assert_eq!(inspect(data.path(), vault.path(), id, &doc).unwrap().unwrap().body, "draft");
        fs::write(path, "partial").unwrap();
        assert!(inspect(data.path(), vault.path(), id, &doc).is_err());
        assert_eq!(fs::read_to_string(doc).unwrap(), "base");
    }

    #[test]
    fn changed_source_remains_held_until_exact_disk_bytes_and_revision() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let doc = vault.path().join("note.md");
        fs::write(&doc, "base").unwrap();
        let id = Uuid::new_v4();
        stage(data.path(), vault.path(), id, &doc, &digest(&doc), 9, "draft").unwrap();
        fs::write(&doc, "external").unwrap();
        let state = inspect(data.path(), vault.path(), id, &doc).unwrap().unwrap();
        assert!(state.source_changed && !state.already_saved);
        assert!(!clear_after_verified_save(data.path(), vault.path(), id, &doc, 9).unwrap());
        fs::write(&doc, "draft").unwrap();
        assert!(!clear_after_verified_save(data.path(), vault.path(), id, &doc, 8).unwrap());
        assert!(clear_after_verified_save(data.path(), vault.path(), id, &doc, 9).unwrap());
        assert!(inspect(data.path(), vault.path(), id, &doc).unwrap().is_none());
    }

    #[test]
    fn acknowledged_newest_revision_prunes_stale_drafts_without_losing_source() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let doc = vault.path().join("multi.md");
        fs::write(&doc, "base").unwrap();
        let id = Uuid::new_v4();
        let baseline = digest(&doc);
        stage(data.path(), vault.path(), id, &doc, &baseline, 41, "first").unwrap();
        stage(data.path(), vault.path(), id, &doc, &baseline, 42, "second").unwrap();
        fs::write(&doc, "second").unwrap();
        assert!(!clear_after_verified_save(data.path(), vault.path(), id, &doc, 41).unwrap());
        assert!(clear_after_verified_save(data.path(), vault.path(), id, &doc, 42).unwrap());
        assert!(inspect(data.path(), vault.path(), id, &doc).unwrap().is_none());
        assert_eq!(fs::read_to_string(doc).unwrap(), "second");
    }

    #[test]
    fn corrupt_older_revision_blocks_cleanup_without_deleting_newest() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let doc = vault.path().join("multi.md");
        fs::write(&doc, "base").unwrap();
        let id = Uuid::new_v4();
        let baseline = digest(&doc);
        let old = stage(data.path(), vault.path(), id, &doc, &baseline, 41, "first").unwrap();
        let new = stage(data.path(), vault.path(), id, &doc, &baseline, 42, "second").unwrap();
        fs::write(&old, b"truncated").unwrap();
        fs::write(&doc, "second").unwrap();
        assert!(clear_after_verified_save(data.path(), vault.path(), id, &doc, 42).is_err());
        assert!(new.exists(), "latest approved snapshot must survive a corrupt older record");
        assert_eq!(fs::read_to_string(doc).unwrap(), "second");
    }

    // This child intentionally bypasses all destructors and shutdown hooks.
    #[test]
    fn abrupt_child_exit_after_snapshot_sync() {
        let Ok(data) = std::env::var("AKULA_DESK15_CRASH_CHILD_DATA") else { return };
        let vault = PathBuf::from(std::env::var("AKULA_DESK15_CRASH_CHILD_VAULT").unwrap());
        let workspace = Uuid::parse_str(&std::env::var("AKULA_DESK15_CRASH_CHILD_ID").unwrap()).unwrap();
        let doc = vault.join("unsaved.md");
        stage(Path::new(&data), &vault, workspace, &doc, &digest(&doc), 87, "unsaved 😀\nline two").unwrap();
        std::process::exit(37);
    }

    #[test]
    fn synced_snapshot_survives_forced_process_exit_without_touching_file() {
        use std::process::Command;
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let doc = vault.path().join("unsaved.md");
        fs::write(&doc, "last disk version").unwrap();
        let id = Uuid::new_v4();
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("abrupt_child_exit_after_snapshot_sync")
            .arg("--nocapture")
            .env("AKULA_DESK15_CRASH_CHILD_DATA", data.path())
            .env("AKULA_DESK15_CRASH_CHILD_VAULT", vault.path())
            .env("AKULA_DESK15_CRASH_CHILD_ID", id.to_string())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(37), "child must skip graceful shutdown");
        assert_eq!(fs::read_to_string(&doc).unwrap(), "last disk version");
        let restored = inspect(data.path(), vault.path(), id, &doc).unwrap().unwrap();
        assert_eq!(restored.revision, 87);
        assert_eq!(restored.body, "unsaved 😀\nline two");
        assert!(!restored.source_changed);
        assert!(!restored.already_saved);
    }

    #[test]
    fn abrupt_child_exit_with_two_unsaved_tabs() {
        let Ok(data) = std::env::var("AKULA_DESK15_TWO_TAB_CHILD_DATA") else { return };
        let vault = PathBuf::from(std::env::var("AKULA_DESK15_TWO_TAB_CHILD_VAULT").unwrap());
        let workspace = Uuid::parse_str(&std::env::var("AKULA_DESK15_TWO_TAB_CHILD_WORKSPACE").unwrap()).unwrap();
        for (name, revision, body) in [
            ("one.md", 81, "first tab 😀"),
            ("two.md", 82, "second tab 漢字"),
        ] {
            let path = vault.join(name);
            stage(Path::new(&data), &vault, workspace, &path, &digest(&path), revision, body).unwrap();
        }
        std::process::exit(47);
    }

    #[test]
    fn two_unsaved_tabs_survive_abrupt_process_exit_without_overwriting_sources() {
        use std::process::Command;
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let workspace = Uuid::new_v4();
        for (name, original) in [("one.md", "disk one"), ("two.md", "disk two")] {
            fs::write(vault.path().join(name), original).unwrap();
        }
        let child = Command::new(std::env::current_exe().unwrap())
            .arg("abrupt_child_exit_with_two_unsaved_tabs")
            .arg("--nocapture")
            .env("AKULA_DESK15_TWO_TAB_CHILD_DATA", data.path())
            .env("AKULA_DESK15_TWO_TAB_CHILD_VAULT", vault.path())
            .env("AKULA_DESK15_TWO_TAB_CHILD_WORKSPACE", workspace.to_string())
            .status().unwrap();
        assert_eq!(child.code(), Some(47));
        for (name, original, draft) in [
            ("one.md", "disk one", "first tab 😀"),
            ("two.md", "disk two", "second tab 漢字"),
        ] {
            let path = vault.path().join(name);
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
            let record = inspect(data.path(), vault.path(), workspace, &path).unwrap().unwrap();
            assert_eq!(record.body, draft);
            assert!(!record.source_changed);
            assert!(!record.already_saved);
        }
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_recovery_bucket_is_refused_without_writing_outside_appdata() {
        use std::os::unix::fs::symlink;
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let doc = vault.path().join("note.md");
        fs::write(&doc, "base").unwrap();
        symlink(outside.path(), data.path().join("akula-note-recovery-v1")).unwrap();
        assert!(stage(data.path(), vault.path(), Uuid::new_v4(), &doc, &digest(&doc), 1, "draft").is_err());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn outside_vault_and_symlink_document_are_rejected() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let doc = other.path().join("other.md");
        fs::write(&doc, "outside").unwrap();
        assert!(stage(data.path(), vault.path(), Uuid::new_v4(), &doc, &digest(&doc), 1, "bad").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = vault.path().join("link.md");
            symlink(&doc, &link).unwrap();
            assert!(stage(data.path(), vault.path(), Uuid::new_v4(), &link, &digest(&doc), 1, "bad").is_err());
        }
    }
}
