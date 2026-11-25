//! VIDEO-21: explicit, SHA-pinned QuickJS-NG installation for yt-dlp EJS.
//! This is not a Video Tools bundle, updater input, or synchronized attachment.
//! Never trust PATH, user configuration, or an arbitrary executable.
use super::{Status, Partial, checked_path, check_ancestors, ensure_directory, version_gate};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, atomic::{AtomicU64, Ordering}};

pub(crate) const VERSION: &str = "0.18.0";
pub(crate) const BYTES: u64 = 1_873_152;
pub(crate) const SHA256: &str =
    "ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7";
pub(crate) const URL: &str =
    "https://github.com/quickjs-ng/quickjs/releases/download/v0.18.0/qjs-windows-x86_64.exe";
const FILENAME: &str = "qjs.exe";
static STATES: OnceLock<Mutex<HashMap<PathBuf, Status>>> = OnceLock::new();
static REVISION: AtomicU64 = AtomicU64::new(0);

fn states() -> &'static Mutex<HashMap<PathBuf, Status>> {
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}
pub(crate) fn revision() -> u64 { REVISION.load(Ordering::Relaxed) }
fn set_state(profile: &Path, next: Status, wake: &Arc<dyn Fn() + Send + Sync>) {
    states().lock().unwrap_or_else(|e| e.into_inner()).insert(profile.to_path_buf(), next);
    REVISION.fetch_add(1, Ordering::Relaxed);
    wake();
}
fn local_root(profile: &Path) -> Result<PathBuf, String> {
    Ok(super::local_root(profile)?.parent()
        .ok_or("optional tool parent directory unavailable")?.join("quickjs"))
}
fn executable(profile: &Path) -> Result<PathBuf, String> {
    Ok(local_root(profile)?.join(FILENAME))
}
fn verify(path: &Path) -> Result<(), String> {
    super::pinned_file::verify(path, BYTES, SHA256, [
        "QuickJS executable lacks parent",
        "QuickJS size differs from pinned official executable",
        "QuickJS executable grew while verifying",
        "QuickJS SHA-256 does not match pinned upstream release",
        "QuickJS size changed while verifying",
    ])
}
pub(crate) fn verified_executable(profile: &Path) -> Result<PathBuf, String> {
    let path = executable(profile)?;
    verify(&path)?;
    Ok(path)
}
fn status_disk(profile: &Path) -> Status {
    match verified_executable(profile) {
        Ok(_) => Status::Installed,
        Err(_) if executable(profile).is_ok_and(|p| p.exists()) => Status::Invalid,
        Err(_) => Status::Missing,
    }
}
pub(crate) fn status(profile: &Path) -> Status {
    let prior = states().lock().unwrap_or_else(|e| e.into_inner()).get(profile).cloned();
    if let Some(Status::Downloading(n)) = prior { return Status::Downloading(n); }
    match status_disk(profile) {
        Status::Missing => match prior {
            Some(Status::Failed(reason)) => Status::Failed(reason),
            _ => Status::Missing,
        },
        other => other,
    }
}
fn fetch(profile: &Path, wake: &Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
    version_gate::require_pinned_windows_video_runtime()?;
    let root = local_root(profile)?;
    ensure_directory(&root)?;
    let target = root.join(FILENAME);
    if target.exists() {
        return Err("QuickJS already exists; remove invalid file before reinstalling".into());
    }
    let stage = Partial(root.join(format!(".quickjs-{}.part", uuid::Uuid::new_v4())));
    let dest = OpenOptions::new().write(true).create_new(true).open(&stage.0)
        .map_err(|e| format!("cannot stage optional QuickJS: {e}"))?;
    super::pinned_transfer::Transfer {
        url: URL, bytes: BYTES, sha256: SHA256, notice_every: 262_144,
        errors: [
            "official QuickJS download failed",
            "official QuickJS release returned a non-200 response",
            "QuickJS fetch interrupted",
            "QuickJS exceeded pinned size",
            "QuickJS temporary write failed",
            "QuickJS temporary flush failed",
            "QuickJS download size or SHA-256 mismatch; refusing install",
        ],
    }.download(dest, |done| set_state(profile, Status::Downloading(done), wake))?;
    fs::hard_link(&stage.0, &target)
        .map_err(|e| format!("cannot publish QuickJS without overwrite: {e}"))?;
    verify(&target)?;
    Ok(())
}
pub(crate) fn start(profile: PathBuf, wake: Arc<dyn Fn() + Send + Sync>) -> Result<(), String> {
    {
        let mut lock = states().lock().unwrap_or_else(|e| e.into_inner());
        if matches!(lock.get(&profile), Some(Status::Downloading(_))) {
            return Err("QuickJS download already in progress".into());
        }
        if matches!(status_disk(&profile), Status::Installed) {
            return Err("pinned QuickJS already installed".into());
        }
        lock.insert(profile.clone(), Status::Downloading(0));
    }
    REVISION.fetch_add(1, Ordering::Relaxed);
    wake();
    std::thread::spawn(move || {
        let next = match fetch(&profile, &wake) {
            Ok(()) => Status::Installed,
            Err(reason) => Status::Failed(reason),
        };
        set_state(&profile, next, &wake);
    });
    Ok(())
}
pub(crate) fn remove(profile: &Path) -> Result<(), String> {
    let mut lock = states().lock().unwrap_or_else(|e| e.into_inner());
    if matches!(lock.get(profile), Some(Status::Downloading(_))) {
        return Err("cannot remove QuickJS during download".into());
    }
    let path = executable(profile)?;
    if path.exists() {
        checked_path(&path, false)?;
        check_ancestors(path.parent().ok_or("QuickJS has no parent")?)?;
        fs::remove_file(&path).map_err(|e| format!("cannot remove QuickJS: {e}"))?;
    }
    lock.remove(profile);
    REVISION.fetch_add(1, Ordering::Relaxed);
    Ok(())
}
#[cfg(test)]
#[path = "optional_quickjs_tests.rs"]
mod tests;
