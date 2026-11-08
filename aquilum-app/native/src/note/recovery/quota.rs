//! Cooperative cross-process admission, not an OS-enforced disk quota.
mod lock;
pub(super) use lock::{acquire, try_acquire};
use std::{fs, io, path::Path};
use uuid::Uuid;
use super::{denied, invalid, reparse};

pub(super) const MAX_WORKSPACE_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const MAX_APPDATA_BYTES: u64 = 512 * 1024 * 1024;

fn owned_filename(name: &str) -> bool {
    if let Some(stem) = name.strip_suffix(".shadow")
        && let Some((digest, revision)) = stem.split_once('-')
    {
        return digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())
            && revision.len() == 20 && revision.bytes().all(|b| b.is_ascii_digit());
    }
    name.strip_prefix('.')
        .and_then(|part| part.strip_suffix(".pending"))
        .is_some_and(|id| Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id))
}

fn within_budget(local: u64, global: u64, incoming: u64) -> io::Result<()> {
    let workspace = local.checked_add(incoming).ok_or_else(|| denied("workspace recovery budget overflow"))?;
    let total = global.checked_add(incoming).ok_or_else(|| denied("AppData recovery budget overflow"))?;
    if workspace > MAX_WORKSPACE_BYTES || total > MAX_APPDATA_BYTES {
        return Err(denied("recovery snapshot budget exhausted; save the note or review old backups"));
    }
    Ok(())
}

/// Refuse excess capacity; never delete existing backups to make room.
pub(super) fn admit(bucket: &Path, incoming: usize) -> io::Result<()> {
    let root = bucket.parent().ok_or_else(|| invalid("recovery workspace has no parent"))?;
    let incoming = u64::try_from(incoming).map_err(|_| denied("snapshot size exceeds u64"))?;
    let (mut local, mut global) = (0u64, 0u64);
    for workspace in fs::read_dir(root)? {
        let workspace = workspace?;
        let name = workspace.file_name();
        let name = name.to_str().ok_or_else(|| invalid("non-UTF8 recovery workspace"))?;
        if name == lock::FILENAME { lock::validate(&workspace.path())?; continue; }
        if !Uuid::parse_str(name).is_ok_and(|uuid| uuid.to_string() == name) {
            return Err(denied("unrecognized recovery workspace directory"));
        }
        let path = workspace.path();
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || reparse(&metadata) {
            return Err(denied("unsafe recovery workspace directory"));
        }
        let own = path == bucket;
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| invalid("non-UTF8 recovery file"))?;
            if !owned_filename(name) {
                return Err(denied("unrecognized recovery file; manual review required"));
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            if !metadata.is_file() || reparse(&metadata) {
                return Err(denied("unsafe recovery file"));
            }
            global = global.checked_add(metadata.len()).ok_or_else(|| denied("AppData recovery size overflow"))?;
            if own {
                local = local.checked_add(metadata.len()).ok_or_else(|| denied("workspace recovery size overflow"))?;
            }
        }
    }
    within_budget(local, global, incoming)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn arithmetic_rejects_overflow_and_preserves_exact_budget_boundary() {
        assert!(within_budget(MAX_WORKSPACE_BYTES - 2, MAX_APPDATA_BYTES - 2, 2).is_ok());
        assert!(within_budget(MAX_WORKSPACE_BYTES - 1, 0, 2).is_err());
        assert!(within_budget(0, MAX_APPDATA_BYTES - 1, 2).is_err());
        assert!(within_budget(u64::MAX, 0, 2).is_err());
        assert!(within_budget(0, u64::MAX, 2).is_err());
    }

    #[test]
    fn workspace_budget_refuses_new_shadow_without_mutating_existing_backup() {
        let data = tempfile::tempdir().unwrap();
        let base = data.path().join("akula-note-recovery-v1");
        let bucket = base.join(Uuid::new_v4().to_string());
        fs::create_dir_all(&bucket).unwrap();
        let old = bucket.join(format!("{}-{:020}.shadow", "a".repeat(64), 1));
        File::create(&old).unwrap().set_len(MAX_WORKSPACE_BYTES - 128).unwrap();
        assert!(admit(&bucket, 129).is_err());
        assert_eq!(fs::metadata(&old).unwrap().len(), MAX_WORKSPACE_BYTES - 128);
        assert!(admit(&bucket, 128).is_ok());
    }

    #[test]
    fn unknown_workspace_file_is_not_deleted_or_counted_as_free_space() {
        let data = tempfile::tempdir().unwrap();
        let bucket = data.path().join("akula-note-recovery-v1").join(Uuid::new_v4().to_string());
        fs::create_dir_all(&bucket).unwrap();
        fs::write(bucket.join("unowned"), "untrusted").unwrap();
        assert!(admit(&bucket, 1).is_err());
        assert_eq!(fs::read(bucket.join("unowned")).unwrap(), b"untrusted");
    }
}
