//! Opt-in FFmpeg-free video download path. Only an explicit UI click can call
//! it, and staging/final bytes are always under the device-local Library.
//! The old yt-dlp route remains available until the new installed toolchain
//! has been pinned and approved; this experiment never installs executables.
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{BufRead, BufReader, Read};
#[cfg(windows)]
use sha2::{Digest, Sha256};
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

fn track_args(stage: &Path, track: Track, remaining: u64, url: &str, quickjs: &Path) -> Vec<OsString> {
    let home = format!("home:{}", stage.display());
    let temp = format!("temp:{}", stage.display());
    let cap = remaining.to_string();
    // Restrict EJS execution to the explicitly pinned QuickJS binary.
    // --no-js-runtimes removes yt-dlp's default Deno runtime.
    let js_runtime = format!("quickjs:{}", quickjs.display());
    [
        "--ignore-config", "--no-plugin-dirs", "--no-remote-components",
        "--no-js-runtimes", "--js-runtimes", &js_runtime, "--no-cache-dir", "--no-cookies", "--no-playlist",
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
    remaining: u64, offset: u64, quickjs: &Path,
    report: &mut impl FnMut(download::Progress),
) -> Result<u64, String> {
    let mut child = ChildGuard(Command::new(executable)
        .current_dir(stage)
        .args(track_args(stage, track, remaining, url, quickjs))
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("yt-dlp unavailable: {e}"))?);
    let errors = child.0.stderr.take().ok_or("missing yt-dlp stderr")?;
    let diagnostics = std::thread::spawn(move || tail(errors));
    let stdout = child.0.stdout.take().ok_or("missing yt-dlp stdout")?;
    let mut too_large = false;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(p) = download::parse_progress(&line) {
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

// A VIDEO-07 helper could return exit=0 for a structurally readable MP4
// with a zero-length elst and no decodable frames. Require a versioned
// positive-duration receipt from the corrected VIDEO-11 helper before any
// file is published. This is an accidental-compatibility guard, not a
// cryptographic signature of arbitrary user-supplied executable bytes.
fn valid_mux_receipt(stdout: &str) -> bool {
    let mut lines = stdout.lines().filter(|l| l.starts_with("DASH_RUST_MUX_PAIR_PASS "));
    let Some(line) = lines.next() else { return false; };
    if lines.next().is_some() { return false; }
    let parts: Vec<_> = line.split_whitespace().collect();
    if !parts.contains(&"protocol=v2") || !parts.contains(&"ffmpeg_in_runtime=0") {
        return false;
    }
    for field in ["duration_ms=", "video_samples=", "audio_samples="] {
        let Some(value) = parts.iter().find_map(|part| part.strip_prefix(field)) else {
            return false;
        };
        let Ok(value) = value.parse::<u64>() else { return false; };
        if value == 0 || (field == "duration_ms=" && value > 86_400_000) {
            return false;
        }
    }
    true
}

fn run_mux(executable: &Path, stage: &Path, result: &Path) -> Result<(), String> {
    let mut child = ChildGuard(Command::new(executable)
        .arg("mux-pair").arg(stage.join("video.mp4")).arg(stage.join("audio.m4a"))
        .arg(result).current_dir(stage)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("Rust MP4 muxer cannot start: {e}"))?);
    let errors = child.0.stderr.take().ok_or("missing mux stderr")?;
    let receipts = child.0.stdout.take().ok_or("missing mux stdout")?;
    let diagnostics = std::thread::spawn(move || tail(errors));
    let result_report = std::thread::spawn(move || tail(receipts));
    let exit = child.0.wait().map_err(|e| e.to_string())?;
    let error = diagnostics.join().unwrap_or_default();
    let receipt = result_report.join().unwrap_or_default();
    if !exit.success() {
        return Err(format!("Rust MP4 muxer rejected H264/AAC ({exit}): {error}"));
    }
    if !valid_mux_receipt(&receipt) {
        return Err("Rust MP4 muxer is outdated or produced no verified duration; refusing publication".into());
    }
    Ok(())
}

fn download_rust(
    library: &Library, executable: &OsStr, mux: &Path, quickjs: &Path, url: &str,
    on_progress: impl FnMut(download::Progress),
) -> Result<PathBuf, String> {
    #[cfg(all(windows, feature = "video-ui-e2e"))]
    let transport = super::ui_acceptance::transport_url(url)?;
    #[cfg(not(all(windows, feature = "video-ui-e2e")))]
    let transport = url.to_owned();
    download_rust_from_source(library, executable, mux, quickjs, url, &transport, on_progress)
}
// VIDEO-12/16: synthetic localhost transport is confined to native tests or the CI-only GUI feature.
fn download_rust_from_source(
    library: &Library, executable: &OsStr, mux: &Path, quickjs: &Path,
    url: &str, transport_url: &str,
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
    let video = run_track(executable, stage.path(), transport_url, Track::Video, remaining, 0, quickjs, &mut on_progress)?;
    if remaining.saturating_sub(video) < 1024 {
        return Err("no space for an AAC track after video download".into());
    }
    let audio = run_track(executable, stage.path(), transport_url, Track::Audio,
        remaining - video, video, quickjs, &mut on_progress)?;
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

// Official Windows binaries checked against the exact VIDEO-05 proof.
// Updating either runtime requires a version/size/hash change plus CI proof.
// Never download or update an executable from user content or runtime EJS.
#[cfg(windows)]
const QUICKJS_WINDOWS_BYTES: u64 = 1_873_152;
#[cfg(windows)]
const YTDLP_WINDOWS_BYTES: u64 = 23_575_744;
#[cfg(windows)]
const QUICKJS_WINDOWS_SHA256: &str =
    "ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7";
#[cfg(windows)]
const YTDLP_WINDOWS_SHA256: &str =
    "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d";

#[cfg(windows)]
fn verify_pin(path: &Path, size: u64, expected: &str, name: &str) -> Result<(), String> {
    if !path.is_absolute() { return Err(format!("{name} requires an absolute path")); }
    let measured = safe_file(path, size)?;
    if measured != size { return Err(format!("{name} has a different executable size")); }
    let mut source = fs::File::open(path).map_err(|e| format!("{name} cannot be opened: {e}"))?;
    let mut sha = Sha256::new();
    let mut buffer = [0u8; 65_536];
    loop {
        let read = source.read(&mut buffer).map_err(|e| format!("{name} verification failed: {e}"))?;
        if read == 0 { break; }
        sha.update(&buffer[..read]);
    }
    // sha2 0.11 Output is an Array, not a LowerHex implementation.
    let digest: String = sha.finalize().iter()
        .map(|byte| format!("{byte:02x}")).collect();
    if digest != expected { return Err(format!("{name} SHA-256 does not match pinned runtime")); }
    Ok(())
}

#[cfg(windows)]
fn verify_pinned_runtime(ytdlp: &Path, quickjs: &Path) -> Result<(), String> {
    verify_pin(ytdlp, YTDLP_WINDOWS_BYTES, YTDLP_WINDOWS_SHA256, "yt-dlp 2026.10.26")?;
    verify_pin(quickjs, QUICKJS_WINDOWS_BYTES, QUICKJS_WINDOWS_SHA256, "QuickJS-NG v0.18.0")
}

#[cfg(not(windows))]
fn verify_pinned_runtime(_ytdlp: &Path, _quickjs: &Path) -> Result<(), String> {
    Err("VIDEO-09 opt-in is Windows-only until non-Windows executable pins are validated".into())
}

// Experimental Rust mux is resolved beside Akula, pinned at build time.
// Never trust an adjacent unsigned manifest or an arbitrary helper path.
#[cfg(windows)]
fn bundled_mux_from_exe(application: &Path, expected_sha256: &str) -> Result<PathBuf, String> {
    if expected_sha256.len() != 64
        || !expected_sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid compiled-in Akula Rust mux SHA-256".into());
    }
    if !application.is_absolute() {
        return Err("Akula executable path must be absolute".into());
    }
    let directory = application.parent().ok_or("Akula executable has no parent directory")?;
    let parent = fs::symlink_metadata(directory)
        .map_err(|e| format!("cannot inspect Akula installation directory: {e}"))?;
    if !parent.is_dir() || parent.file_type().is_symlink() {
        return Err("Akula installation directory is not a regular directory".into());
    }
    use std::os::windows::fs::MetadataExt;
    if parent.file_attributes() & 0x0000_0400 != 0 {
        return Err("Akula installation directory is a reparse point".into());
    }
    let mux = directory.join("akula-video-mux.exe");
    let size = safe_file(&mux, 128 * 1024 * 1024)?;
    verify_pin(&mux, size, expected_sha256, "Akula bundled Rust mux")?;
    Ok(mux)
}

// The pinned official yt-dlp.exe requires Windows 10 or newer. Do not rely
// on GetVersionExW, whose answer can depend on the application manifest.
// The version-only gate does not authorize shipping or certify OS APIs.
#[cfg(all(windows, feature = "video-rust-bundle"))]
fn supports_pinned_windows_video_runtime(major: u32) -> bool {
    major >= 10
}

#[cfg(all(windows, feature = "video-rust-bundle"))]
fn require_pinned_windows_video_runtime() -> Result<(), String> {
    // Exact C layout of RTL_OSVERSIONINFOW / OSVERSIONINFOW.
    #[repr(C)]
    struct OsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version: *mut OsVersionInfo) -> i32;
    }
    let mut version = OsVersionInfo {
        size: std::mem::size_of::<OsVersionInfo>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
    };
    // NTSTATUS_SUCCESS is zero; an unqueryable OS fails closed.
    if unsafe { RtlGetVersion(&mut version) } != 0 {
        return Err("VIDEO-18b cannot verify Windows version; pinned yt-dlp must not run".into());
    }
    if !supports_pinned_windows_video_runtime(version.major) {
        return Err(format!(
            "VIDEO-18b pinned yt-dlp requires Windows 10 or newer; found {}.{}",
            version.major, version.minor
        ));
    }
    Ok(())
}

/// The only production Rust path is the installer-staged, build-time-pinned
/// sidecar next to the running Akula executable. The old arbitrary-path opt-in
/// is rejected. No download of executables happens at runtime.
pub(super) fn download_one_or_legacy(
    library: &Library, executable: &OsStr, url: &str,
    on_progress: impl FnMut(download::Progress),
) -> Result<PathBuf, String> {
    if std::env::var_os("AKULA_RUST_MUX_BIN").is_some() {
        return Err("AKULA_RUST_MUX_BIN is no longer supported: arbitrary mux executables are refused".into());
    }
    #[cfg(all(windows, feature = "video-rust-bundle"))]
    {
        // Reject unsupported Windows before opening or spawning any sidecar.
        require_pinned_windows_video_runtime()?;
        let expected = option_env!("AKULA_BUNDLED_MUX_SHA256")
            .ok_or("video-rust-bundle requires a compiled-in mux SHA-256; refusing legacy fallback")?;
        let application = std::env::current_exe()
            .map_err(|e| format!("cannot locate installed Akula executable: {e}"))?;
        let mux = bundled_mux_from_exe(&application, expected)?;
        let directory = application.parent().ok_or("Akula executable has no parent directory")?;
        // VIDEO-15: both tools are pinned siblings; never execute the queue's
        // PATH/env-selected executable or a user-selected JS runtime.
        let ytdlp = directory.join("akula-yt-dlp.exe");
        let quickjs = directory.join("akula-quickjs.exe");
        verify_pinned_runtime(&ytdlp, &quickjs)?;
        download_rust(library, ytdlp.as_os_str(), &mux, &quickjs, url, on_progress)
    }
    #[cfg(all(not(windows), feature = "video-rust-bundle"))]
    {
        Err("video-rust-bundle is Windows-only: refusing unpinned legacy fallback".into())
    }
    #[cfg(not(feature = "video-rust-bundle"))]
    {
        download::download_one(library, executable, url, on_progress)
    }
}

#[cfg(test)]
#[path = "light_tests.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "light_integration.rs"]
mod integration_tests;
