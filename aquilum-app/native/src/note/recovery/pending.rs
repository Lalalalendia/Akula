//! Bound stale private .pending files left by hard-kill/power loss before
//! immutable shadow publication. Never delete fresh writes from another app.
use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

use super::{denied, invalid, reparse};

const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_FRESH_PENDING: usize = 64;

pub(super) fn clean_stale(bucket: &Path) -> io::Result<()> {
    let mut live = 0;
    let now = SystemTime::now();
    for entry in fs::read_dir(bucket)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| invalid("unreadable recovery entry filename"))?;
        if !name.starts_with('.') || !name.ends_with(".pending") {
            continue;
        }
        let id = name.trim_start_matches('.').trim_end_matches(".pending");
        // Own only .<canonical-v4-UUID>.pending. Never touch unrelated files.
        let uuid = Uuid::parse_str(id).map_err(|_| invalid("unknown recovery pending filename"))?;
        if name != format!(".{uuid}.pending") {
            return Err(invalid("noncanonical recovery pending filename"));
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        if !meta.is_file() || reparse(&meta) {
            return Err(denied("unsafe recovery pending file"));
        }
        if now.duration_since(meta.modified()?).is_ok_and(|age| age >= STALE_AFTER) {
            fs::remove_file(path)?;
        } else {
            live += 1;
            if live >= MAX_FRESH_PENDING {
                return Err(denied("too many recent recovery pending files; manual review required"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::Write;

    #[test]
    fn stale_crash_pending_is_removed_but_fresh_writer_and_original_survive() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let doc = vault.path().join("draft.md");
        fs::write(&doc, "persisted").unwrap();
        let workspace = Uuid::new_v4();
        let baseline = blake3::hash(b"persisted").to_hex().to_string();
        let bucket = super::super::folder(data.path(), workspace, true).unwrap().unwrap();
        let stale = bucket.join(format!(".{}.pending", Uuid::new_v4()));
        let fresh = bucket.join(format!(".{}.pending", Uuid::new_v4()));
        let mut old = OpenOptions::new().write(true).create_new(true).open(&stale).unwrap();
        old.write_all(b"truncated plaintext").unwrap();
        old.set_modified(SystemTime::now() - Duration::from_secs(48 * 60 * 60)).unwrap();
        drop(old);
        File::create(&fresh).unwrap().write_all(b"active pending").unwrap();
        super::super::stage(data.path(), vault.path(), workspace, &doc, &baseline, 1, "new draft").unwrap();
        assert!(!stale.exists());
        assert!(fresh.exists(), "another process may still be writing");
        assert_eq!(fs::read_to_string(&doc).unwrap(), "persisted");
        assert_eq!(
            super::super::inspect(data.path(), vault.path(), workspace, &doc).unwrap().unwrap().body,
            "new draft"
        );
    }

    #[test]
    fn anomalous_or_excess_pending_never_triggers_untrusted_deletion() {
        let data = tempfile::tempdir().unwrap();
        let workspace = Uuid::new_v4();
        let bucket = super::super::folder(data.path(), workspace, true).unwrap().unwrap();
        let unrelated = bucket.join(".not-a-uuid.pending");
        fs::write(&unrelated, "unknown").unwrap();
        assert!(clean_stale(&bucket).is_err());
        assert!(unrelated.exists(), "unknown files must not be deleted");
        fs::remove_file(&unrelated).unwrap();
        for _ in 0..MAX_FRESH_PENDING {
            fs::write(bucket.join(format!(".{}.pending", Uuid::new_v4())), "fresh").unwrap();
        }
        assert!(clean_stale(&bucket).is_err(), "a bounded journal must not silently fill the disk");
    }
}
