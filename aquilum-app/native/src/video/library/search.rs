use super::{alias, default_root, outside_vaults};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn search_root(profile: &Path, vaults: &[PathBuf]) -> Option<PathBuf> {
    let path = default_root(profile).ok()?;
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.is_dir() || alias(&meta) {
        return None;
    }
    let canonical = fs::canonicalize(&path).ok()?;
    outside_vaults(&canonical, vaults).ok()?;
    Some(canonical)
}
