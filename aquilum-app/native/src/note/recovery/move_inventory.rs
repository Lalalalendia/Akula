//! Fail-closed, bounded inventory for a folder move: recovery records are
//! keyed by path digests, so open tabs alone cannot reveal hidden old drafts.
use std::fs;
use std::io;
use std::path::{Component, Path};
use uuid::Uuid;

use super::{checked_relative, decode, denied, folder, invalid, key, read_bounded, reparse};

// Bounded synchronous disk work: larger/abnormal AppData requires manual review.
const MAX_MOVE_RECORDS: usize = 1_024;
const MAX_MOVE_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn has_unreviewed_shadow_under_move(
    data: &Path, vault: &Path, workspace: Uuid, from: &Path,
) -> io::Result<bool> {
    let moving = from.strip_prefix(vault)
        .map_err(|_| denied("move source outside workspace"))?;
    if !normal_relative(moving) {
        return Err(denied("unnormalized move source"));
    }
    let Some(bucket) = folder(data, workspace, false)? else { return Ok(false) };
    let workspace_id = workspace.to_string();
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in fs::read_dir(bucket)? {
        let entry = entry?;
        count += 1;
        if count > MAX_MOVE_RECORDS {
            return Err(denied("too many recovery records to inspect during move"));
        }
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| invalid("unreadable recovery filename"))?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        if !meta.is_file() || reparse(&meta) {
            return Err(denied("unsafe recovery entry during move"));
        }
        if name.starts_with('.') && name.ends_with(".pending") {
            return Err(denied("pending recovery write requires review before moving a folder"));
        }
        if !name.ends_with(".shadow") {
            return Err(denied("unexpected recovery entry requires manual review"));
        }
        bytes = bytes.checked_add(meta.len())
            .ok_or_else(|| denied("recovery inventory size overflow"))?;
        if bytes > MAX_MOVE_BYTES {
            return Err(denied("recovery inventory too large for synchronous move preflight"));
        }

        // Verify the full authenticated-by-checksum wire before trusting its
        // embedded relative pathname. Reject corrupt records even if unrelated.
        let record = decode(&read_bounded(&path)?)?;
        let relative = Path::new(&record.relative);
        if !normal_relative(relative) || record.workspace != workspace_id {
            return Err(invalid("invalid recovery workspace or relative path"));
        }
        let expected = format!("{}-{:020}.shadow", key(&record.relative), record.revision);
        if name != expected {
            return Err(invalid("recovery record path key or revision mismatch"));
        }
        if !relative.starts_with(moving) {
            continue;
        }
        let document = vault.join(relative);
        if checked_relative(vault, &document)? != record.relative {
            return Err(invalid("recovery path changed during move preflight"));
        }
        let disk = aquilum_core::files::document::read_file_hash_impl(&document)
            .map_err(|_| denied("cannot verify recovery source during move"))?;
        if disk != record.body_hash {
            return Ok(true);
        }
    }
    Ok(false)
}

fn normal_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|part| matches!(part, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(path: &Path) -> String {
        blake3::hash(&fs::read(path).unwrap()).to_hex().to_string()
    }

    #[test]
    fn closed_tab_shadow_blocks_only_its_folder_until_exact_disk_bytes_match() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let workspace = Uuid::new_v4();
        let folder = vault.path().join("Folder");
        let other = vault.path().join("Other");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&other).unwrap();
        let child = folder.join("closed.md");
        let unrelated = other.join("elsewhere.md");
        fs::write(&child, "disk source").unwrap();
        fs::write(&unrelated, "unrelated disk").unwrap();
        super::super::stage(
            data.path(), vault.path(), workspace, &unrelated,
            &digest(&unrelated), 1, "unsaved elsewhere",
        ).unwrap();
        assert!(!has_unreviewed_shadow_under_move(
            data.path(), vault.path(), workspace, &folder,
        ).unwrap());

        super::super::stage(
            data.path(), vault.path(), workspace, &child,
            &digest(&child), 1, "unsaved closed-tab draft",
        ).unwrap();
        assert!(has_unreviewed_shadow_under_move(
            data.path(), vault.path(), workspace, &folder,
        ).unwrap());
        fs::write(&child, "unsaved closed-tab draft").unwrap();
        assert!(!has_unreviewed_shadow_under_move(
            data.path(), vault.path(), workspace, &folder,
        ).unwrap());
    }

    #[test]
    fn pending_or_corrupt_recovery_cannot_be_ignored_for_directory_move() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let workspace = Uuid::new_v4();
        let folder = vault.path().join("Folder");
        fs::create_dir_all(&folder).unwrap();
        let bucket = super::super::folder(data.path(), workspace, true).unwrap().unwrap();
        let pending = bucket.join(format!(".{}.pending", Uuid::new_v4()));
        fs::write(&pending, "incomplete").unwrap();
        assert!(has_unreviewed_shadow_under_move(
            data.path(), vault.path(), workspace, &folder,
        ).is_err());
        assert!(pending.exists());
        fs::remove_file(&pending).unwrap();
        let bad = bucket.join(format!("{}-{:020}.shadow", "a".repeat(64), 1));
        fs::write(&bad, b"damaged shadow").unwrap();
        assert!(has_unreviewed_shadow_under_move(
            data.path(), vault.path(), workspace, &folder,
        ).is_err());
        assert!(bad.exists());
    }
}
