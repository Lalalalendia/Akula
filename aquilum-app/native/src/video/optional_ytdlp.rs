//! VIDEO-20: user-initiated, pinned yt-dlp acquisition for Windows.
//!
//! The Akula installer does not bundle yt-dlp, and this code never updates
//! the Akula updater or authorizes the old 3-EXE Video Tools package.
//! Download only after a Settings confirmation; no PATH/env executable input.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::Ordering};

pub(crate) const VERSION: &str = "2026.10.26";
pub(crate) const BYTES: u64 = 23_575_744;
pub(crate) const SHA256: &str =
    "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d";
pub(super) const URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.26/yt-dlp.exe";
const FILENAME: &str = "yt-dlp-2026.10.26.exe";
#[path = "optional_tool_verify.rs"] mod pinned_file;
#[path = "optional_tool_transfer.rs"] mod pinned_transfer;
#[path = "optional_tool_state.rs"] mod tool_state;
#[path = "optional_tool_install.rs"] mod pinned_install;
use tool_state::ToolState;
#[path = "light_windows_version.rs"]
mod version_gate; #[path = "optional_quickjs.rs"] pub(crate) mod quickjs;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Missing,
    Downloading(u64),
    Installed,
    Invalid,
    Failed(String),
}
static STATE: ToolState = ToolState::new();
pub(crate) fn revision() -> u64 { STATE.revision() }

fn local_root(app_data: &Path) -> Result<PathBuf, String> {
    // Reuse the native video library's existing Windows local-data selection,
    // but NOT the videos directory, a synced vault, or the roaming profile.
    let library = super::library::default_root(app_data)
        .map_err(|e| format!("local tool location unavailable: {e}"))?;
    let root = library.parent().ok_or("local video directory has no parent")?;
    if !root.is_absolute() {
        return Err("local tool root must be absolute".into());
    }
    Ok(root.join("tools").join("yt-dlp"))
}
fn executable(app_data: &Path) -> Result<PathBuf, String> {
    Ok(local_root(app_data)?.join(FILENAME))
}
fn checked_path(path: &Path, is_dir: bool) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() ||
        (is_dir && !meta.is_dir()) || (!is_dir && !meta.is_file())
    {
        return Err("optional downloader path is aliased or not regular".into());
    }
    use std::os::windows::fs::MetadataExt;
    if meta.file_attributes() & 0x400 != 0 {
        return Err("optional downloader path contains a Windows reparse point".into());
    }
    Ok(())
}
fn check_ancestors(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        if ancestor.exists() {
            checked_path(ancestor, true)?;
        }
    }
    Ok(())
}
fn ensure_directory(path: &Path) -> Result<(), String> {
    // Check existing ancestors BEFORE any creation so a junction cannot
    // redirect our write into a vault or arbitrary installed software.
    check_ancestors(path)?;
    let mut chain: Vec<_> = path.ancestors().collect();
    chain.reverse();
    for dir in chain {
        if !dir.exists() {
            fs::create_dir(dir).map_err(|e| format!("cannot create local tool directory: {e}"))?;
        }
        checked_path(dir, true)?;
    }
    Ok(())
}
fn verified(path: &Path) -> Result<(), String> {
    pinned_file::verify(path, BYTES, SHA256, [
        "tool has no parent",
        "yt-dlp executable length does not match the pinned release",
        "yt-dlp executable grew during verification",
        "yt-dlp SHA-256 differs from the pinned official release",
        "yt-dlp executable changed during verification",
    ])
}
pub(crate) fn verified_executable(app_data: &Path) -> Result<PathBuf, String> {
    let path = executable(app_data)?;
    verified(&path)?;
    Ok(path)
}
pub(crate) fn status(app_data: &Path) -> Status {
    if let Some(Status::Downloading(n)) = STATE.states().lock().unwrap_or_else(|e| e.into_inner())
        .get(app_data).cloned()
    {
        return Status::Downloading(n);
    }
    let prior = STATE.states().lock().unwrap_or_else(|e| e.into_inner())
        .get(app_data).cloned();
    match verified_executable(app_data) {
        Ok(_) => Status::Installed,
        Err(_) => {
            let path = match executable(app_data) {
                Ok(path) => path,
                Err(e) => return Status::Failed(e),
            };
            if path.exists() { Status::Invalid }
            else if let Some(Status::Failed(reason)) = prior { Status::Failed(reason) }
            else { Status::Missing }
        }
    }
}
fn fetch(app_data: &Path, wake: &Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
    version_gate::require_pinned_windows_video_runtime()?;
    let root = local_root(app_data)?;
    pinned_install::install(&root, FILENAME, "yt-dlp", [
        "yt-dlp already exists; remove the invalid file before reinstalling",
        "cannot stage the optional downloader",
        "cannot commit yt-dlp without overwriting another file",
    ], |dest| pinned_transfer::Transfer {
        url: URL, bytes: BYTES, sha256: SHA256, notice_every: 1_048_576,
        errors: [
            "official yt-dlp download failed",
            "official yt-dlp release returned a non-200 response",
            "yt-dlp download interrupted",
            "yt-dlp downloaded bytes exceed the pinned length",
            "yt-dlp temporary write failed",
            "yt-dlp temporary flush failed",
            "official yt-dlp size or SHA-256 mismatch; refusing installation",
        ],
    }.download(dest, |done| STATE.set_state(app_data, Status::Downloading(done), wake)), verified)
}

/// Called exclusively by the Settings confirmation button; no startup fetch.
pub(crate) fn start(app_data: PathBuf, wake: Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
    STATE.start(app_data, wake, status_disk, fetch, [
        "yt-dlp download is already in progress",
        "the pinned yt-dlp version is already installed",
    ])
}
fn status_disk(app_data: &Path) -> Status {
    match verified_executable(app_data) {
        Ok(_) => Status::Installed,
        Err(_) if executable(app_data).is_ok_and(|p| p.exists()) => Status::Invalid,
        Err(_) => Status::Missing,
    }
}
pub(crate) fn remove(app_data: &Path) -> Result<(), String> {
    let mut lock = STATE.states().lock().unwrap_or_else(|e| e.into_inner());
    if matches!(lock.get(app_data), Some(Status::Downloading(_))) {
        return Err("cannot remove yt-dlp during an active download".into());
    }
    let path = executable(app_data)?;
    if path.exists() {
        checked_path(&path, false)?;
        check_ancestors(path.parent().ok_or("optional tool has no parent")?)?;
        fs::remove_file(&path).map_err(|e| format!("cannot remove optional yt-dlp: {e}"))?;
    }
    lock.remove(app_data);
    STATE.revision.fetch_add(1, Ordering::Relaxed);
    Ok(())
}
#[cfg(test)]
#[path = "optional_ytdlp_tests.rs"]
mod tests;
