//! Opt-in FFmpeg-free video download path. Only an explicit UI click can call
//! it, and staging/final bytes are always under the device-local Library.
//! The old yt-dlp route remains available until the new installed toolchain
//! has been pinned and approved; this experiment never installs executables.
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use super::{download, library::{Library, LIBRARY_MAX, SINGLE_VIDEO_MAX}, youtube};

struct Stage(PathBuf);
impl Stage {
    fn new(library: &Library) -> Result<Self, String> {
        let path = library.root().join(format!(".akula-light-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).map_err(|e| format!("local media staging failed: {e}"))?;
        Ok(Self(path))
    }
    fn path(&self) -> &Path { &self.0 }
}
impl Drop for Stage {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn tail(mut stream: impl Read) -> String {
    let mut buf = [0u8; 4096];
    let mut latest = Vec::new();
    while let Ok(len) = stream.read(&mut buf) {
        if len == 0 { break; }
        latest.extend_from_slice(&buf[..len]);
        if latest.len() > 8192 { latest.drain(..latest.len() - 8192); }
    }
    String::from_utf8_lossy(&latest).chars().take(500).collect()
}

#[derive(Clone, Copy)]
enum Track { Video, Audio }
impl Track {
    fn output(self) -> &'static str {
        match self { Track::Video => "video.%(ext)s", Track::Audio => "audio.%(ext)s" }
    }
    fn format(self) -> &'static str {
        match self {
            Track::Video => "bv[vcodec^=avc1][ext=mp4][height<=720]",
            Track::Audio => "ba[acodec^=mp4a][ext=m4a]",
        }
    }
    fn filename(self) -> &'static str {
        match self { Track::Video => "video.mp4", Track::Audio => "audio.m4a" }
    }
}

fn safe_file(path: &Path, max_bytes: u64) -> Result<u64, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("expected local media not found: {e}"))?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() == 0 || meta.len() > max_bytes {
        return Err("local media is missing, aliased, empty or exceeds the quota".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x0000_0400 != 0 {
            return Err("local media is a Windows reparse point".into());
        }
    }
    Ok(meta.len())
}

fn track_args(stage: &Path, track: Track, remaining: u64, url: &str) -> Vec<OsString> {
    let home = format!("home:{}", stage.display());
    let temp = format!("temp:{}", stage.display());
    let cap = remaining.to_string();
    [
        "--ignore-config", "--no-cache-dir", "--no-cookies", "--no-playlist",
        "--no-exec", "--no-overwrites", "--no-color", "--no-write-info-json",
        "--no-write-thumbnail", "--no-write-subs", "--no-write-auto-subs",
        "--no-write-comments", "--no-download-archive", "--restrict-filenames",
        "--newline", "--progress", "--no-warnings", "--retries", "3",
        "--fragment-retries", "3", "--socket-timeout", "20", "--fixup", "never",
        "--format", track.format(), "--output", track.output(), "--paths", &home,
        "--paths", &temp, "--max-filesize", &cap, "--progress-template",
        "download:AKULA_PROGRESS:%(progress.downloaded_bytes|0)s:%(progress.total_bytes|0)s:%(progress.total_bytes_estimate|0)s",
        "--", url,
    ].into_iter().map(OsString::from).collect()
}

fn run_track(
    executable: &OsStr, stage: &Path, url: &str, track: Track,
    remaining: u64, offset: u64, report: &mut impl FnMut(download::Progress),
) -> Result<u64, String> {
    let mut child = ChildGuard(Command::new(executable)
        .current_dir(stage)
        .args(track_args(stage, track, remaining, url))
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("yt-dlp unavailable: {e}"))?);
    let errors = child.0.stderr.take().ok_or("missing yt-dlp stderr")?;
    let diagnostics = std::thread::spawn(move || tail(errors));
    let stdout = child.0.stdout.take().ok_or("missing yt-dlp stdout")?;
    let mut too_large = false;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(p) = download::parse_progress_for_light(&line) {
            if p.downloaded > remaining {
                too_large = true;
                let _ = child.0.kill();
                break;
            }
            report(download::Progress {
                downloaded: offset.saturating_add(p.downloaded),
                total: p.total.map(|v| offset.saturating_add(v)),
            });
        }
    }
    let exit = child.0.wait().map_err(|e| e.to_string())?;
    let error = diagnostics.join().unwrap_or_default();
    if too_large { return Err("video download exceeded remaining local quota".into()); }
    if !exit.success() { return Err(format!("yt-dlp failed on a separate media stream ({exit}): {error}")); }
    safe_file(&stage.join(track.filename()), remaining)
}

fn check_mux(executable: &Path) -> Result<(), String> {
    if !executable.is_absolute() {
        return Err("AKULA_RUST_MUX_BIN must be an absolute executable path".into());
    }
    safe_file(executable, 128 * 1024 * 1024).map(|_| ())
}

fn run_mux(executable: &Path, stage: &Path, result: &Path) -> Result<(), String> {
    let mut child = ChildGuard(Command::new(executable)
        .arg("mux-pair").arg(stage.join("video.mp4")).arg(stage.join("audio.m4a"))
        .arg(result).current_dir(stage)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("Rust MP4 muxer cannot start: {e}"))?);
    let errors = child.0.stderr.take().ok_or("missing mux stderr")?;
    let diagnostics = std::thread::spawn(move || tail(errors));
    let exit = child.0.wait().map_err(|e| e.to_string())?;
    let error = diagnostics.join().unwrap_or_default();
    if !exit.success() { return Err(format!("Rust MP4 muxer rejected H264/AAC ({exit}): {error}")); }
    Ok(())
}

fn download_rust(
    library: &Library, executable: &OsStr, mux: &Path, url: &str,
    mut on_progress: impl FnMut(download::Progress),
) -> Result<PathBuf, String> {
    let id = youtube::video_id(url).ok_or("unsupported single YouTube URL")?;
    if let Some(existing) = library.existing(id).map_err(|e| e.to_string())? {
        return Ok(existing);
    }
    check_mux(mux)?;
    let used = library.used_bytes().map_err(|e| e.to_string())?;
    let remaining = LIBRARY_MAX.saturating_sub(used).min(SINGLE_VIDEO_MAX);
    if remaining < 1024 * 1024 { return Err("local video quota exhausted".into()); }
    let stage = Stage::new(library)?;
    let video = run_track(executable, stage.path(), url, Track::Video, remaining, 0, &mut on_progress)?;
    if remaining.saturating_sub(video) < 1024 {
        return Err("no space for an AAC track after video download".into());
    }
    let audio = run_track(executable, stage.path(), url, Track::Audio,
        remaining - video, video, &mut on_progress)?;
    if video.saturating_add(audio) > remaining {
        return Err("media input streams exceeded local quota".into());
    }
    let output = stage.path().join(format!("{id}.mp4"));
    run_mux(mux, stage.path(), &output)?;
    let bytes = safe_file(&output, remaining)?;
    if used.saturating_add(bytes) > LIBRARY_MAX {
        return Err("remuxed video exceeds local library capacity".into());
    }
    // Windows FlushFileBuffers needs a writable file handle.
    fs::OpenOptions::new().read(true).write(true).open(&output)
        .and_then(|file| file.sync_all()).map_err(|e| e.to_string())?;
    let target = library.published(id, "mp4").map_err(|e| e.to_string())?;
    if let Some(existing) = library.existing(id).map_err(|e| e.to_string())? {
        return Ok(existing);
    }
    fs::hard_link(&output, &target)
        .map_err(|e| format!("cannot atomically publish FFmpeg-free local video: {e}"))?;
    Ok(target)
}

/// Opt-in staging route, preserving legacy behavior until there is an
/// independently audited installer for yt-dlp, QuickJS and the Rust muxer.
pub(super) fn download_one_or_legacy(
    library: &Library, executable: &OsStr, url: &str,
    on_progress: impl FnMut(download::Progress),
) -> Result<PathBuf, String> {
    let requested = std::env::var_os("AKULA_RUST_MUX_BIN").filter(|s| !s.is_empty());
    match requested {
        Some(mux) => download_rust(library, executable, &PathBuf::from(mux), url, on_progress),
        None => download::download_one(library, executable, url, on_progress),
    }
}

#[cfg(test)]
#[path = "light_tests.rs"]
mod tests;
