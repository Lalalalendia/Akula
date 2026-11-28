#[cfg(any(all(windows, target_arch = "x86_64"), test))]
mod install;
mod pins;

use super::Tool;
use pins::{FFMPEG_FILES, MODEL, Pin, VAD, WHISPER_FILES};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

pub(super) fn supported() -> bool {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        return crate::video::optional_ytdlp::version_gate::require_pinned_windows_video_runtime()
            .is_ok();
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    false
}

pub(super) fn download(
    profile: &Path,
    progress: impl FnMut(u8),
) -> Result<tempfile::TempDir, String> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        install::check_windows_runtime()?;
        let library =
            crate::video::library::default_root(profile).map_err(|error| error.to_string())?;
        let root = library
            .parent()
            .ok_or("speech tool directory is unavailable")?;
        install::stage(&root.join("tools").join("speech"), progress)
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = (profile, progress);
        Err("automatic speech setup requires Windows 10 or later (x64)".into())
    }
}

pub(super) fn path(root: &Path, tool: Tool) -> PathBuf {
    let (directory, pin, _) = entry(tool);
    root.join(directory).join(pin.name)
}

pub(super) fn checked(root: &Path, tool: Tool) -> Result<PathBuf, String> {
    if !root.is_absolute() {
        return Err("speech toolkit directory must be absolute".into());
    }
    let (directory, pin, siblings) = entry(tool);
    let folder = root.join(directory);
    for ancestor in folder.ancestors() {
        check_path(ancestor, true)?;
    }
    if !siblings.is_empty() {
        verify_directory(&folder, siblings)?;
    }
    let file = folder.join(pin.name);
    verify_file(&file, pin)?;
    Ok(file)
}

fn entry(tool: Tool) -> (&'static str, &'static Pin, &'static [Pin]) {
    match tool {
        Tool::Probe => ("media", &FFMPEG_FILES[0], &FFMPEG_FILES),
        Tool::Ffmpeg => ("media", &FFMPEG_FILES[1], &FFMPEG_FILES),
        Tool::Whisper => ("engine", &WHISPER_FILES[0], &WHISPER_FILES),
        Tool::Model => ("models", &MODEL, &[]),
        Tool::Vad => ("models", &VAD, &[]),
    }
}

fn verify_directory(folder: &Path, pins: &[Pin]) -> Result<(), String> {
    for entry in fs::read_dir(folder).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !pins.iter().any(|pin| entry.file_name() == pin.name) {
            return Err(format!(
                "unexpected file in speech toolkit: {}",
                entry.path().display()
            ));
        }
    }
    // FFmpeg and FFprobe are standalone. Verify every sibling the Windows
    // loader or ggml can load from the Whisper working installation.
    for pin in pins.iter().filter(|pin| pin.name.ends_with(".dll")) {
        verify_file(&folder.join(pin.name), pin)?;
    }
    Ok(())
}

fn verify_file(file: &Path, pin: &Pin) -> Result<(), String> {
    check_path(file, false)?;
    let mut reader = File::open(file)
        .map_err(|error| error.to_string())?
        .take(pin.bytes + 1);
    let mut hash = Sha256::new();
    let mut buffer = [0; 65_536];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let hex = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if reader.limit() != 1 || hex != pin.sha256 {
        return Err(format!(
            "speech toolkit file changed: {}. Reinstall in Settings",
            pin.name
        ));
    }
    Ok(())
}

fn check_path(path: &Path, directory: bool) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if is_alias(&metadata)
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(format!("invalid speech toolkit path: {}", path.display()));
    }
    Ok(())
}

fn is_alias(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    metadata.is_symlink()
}
