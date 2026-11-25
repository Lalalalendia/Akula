//! Shared staging and no-overwrite publication for pinned optional tools.
use super::ensure_directory;
use std::{fs::{self, File, OpenOptions}, path::{Path, PathBuf}};

struct Partial(PathBuf);
impl Drop for Partial {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}

pub(super) fn install(root: &Path, filename: &str, prefix: &str,
    [exists, stage_error, publish_error]: [&str; 3],
    transfer: impl FnOnce(File) -> Result<(), String>,
    verify: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    ensure_directory(root)?;
    let target = root.join(filename);
    if target.exists() { return Err(exists.into()); }
    let path = root.join(format!(".{prefix}-{}.part", uuid::Uuid::new_v4()));
    let dest = OpenOptions::new().write(true).create_new(true).open(&path)
        .map_err(|e| format!("{stage_error}: {e}"))?;
    // Only clean up a staging file that we successfully created ourselves.
    let stage = Partial(path);
    transfer(dest)?;
    // Same-volume, atomic, no-clobber publication, followed by verification.
    fs::hard_link(&stage.0, &target).map_err(|e| format!("{publish_error}: {e}"))?;
    verify(&target)
}
