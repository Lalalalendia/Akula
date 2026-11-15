//! Explicit-only yt-dlp adapter. No shell, no arbitrary URL, no vault writes.
//! Download to a disposable directory on the SAME local filesystem, verify
//! the completed file, then publish with atomic no-clobber hard-link.
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::library::{Library, LIBRARY_MAX, SINGLE_VIDEO_MAX};
use super::youtube;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

fn parse_progress(line: &str) -> Option<Progress> {
    let body = line.strip_prefix("AKULA_PROGRESS:")?;
    let mut fields = body.split(':');
    let downloaded = fields.next()?.trim().parse().ok()?;
    let total = fields.next().and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .or_else(|| fields.next().and_then(|v| v.trim().parse::<u64>().ok()).filter(|n| *n > 0));
    Some(Progress { downloaded, total })
}

fn command_args(url: &str, stage: &Path, max_bytes: u64) -> Vec<OsString> {
    let home = format!("home:{}", stage.display());
    let temp = format!("temp:{}", stage.display());
    [
        "--ignore-config", "--no-cache-dir", "--no-cookies", "--no-playlist", "--no-exec", "--no-overwrites",
        "--no-color", "--no-write-info-json", "--no-write-thumbnail",
        "--no-write-subs", "--no-write-auto-subs", "--no-write-comments",
        "--no-download-archive", "--restrict-filenames", "--newline",
        "--progress", "--no-warnings", "--retries", "3",
        "--fragment-retries", "3", "--socket-timeout", "20",
        "--format", "bv*[height<=720]+ba/b[height<=720]",
        "--merge-output-format", "mp4",
        "--output", "%(id)s.%(ext)s",
        "--paths", &home, "--paths", &temp,
        "--max-filesize", &max_bytes.to_string(),
        "--progress-template",
        "download:AKULA_PROGRESS:%(progress.downloaded_bytes|0)s:%(progress.total_bytes|0)s:%(progress.total_bytes_estimate|0)s",
        "--", url,
    ].into_iter().map(OsString::from).collect()
}

/// If the progress stream cannot be decoded or inspected, the download
/// process must not continue writing after the staging directory is dropped.
struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn get_error(mut reader: impl Read) -> String {
    let mut bytes = [0_u8; 4096];
    let mut recent = Vec::<u8>::new();
    while let Ok(n) = reader.read(&mut bytes) {
        if n == 0 { break; }
        recent.extend_from_slice(&bytes[..n]);
        if recent.len() > 8192 {
            recent.drain(..recent.len() - 8192);
        }
    }
    String::from_utf8_lossy(&recent).trim().chars().take(500).collect()
}

/// Private same-volume staging directory. Unlike a persistent video, it is
/// removed on failure, process errors and normal completion.
struct Staging(PathBuf);

impl Staging {
    fn new(root: &Path) -> Result<Self, String> {
        let path = root.join(format!(".akula-video-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).map_err(|e| format!("local video staging failed: {e}"))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path { &self.0 }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn completed(stage: &Path, id: &str) -> Result<PathBuf, String> {
    let mut candidate = None;
    for entry in fs::read_dir(stage).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.file_stem() != Some(OsStr::new(id)) {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !["mp4", "webm", "mkv", "mov", "m4v"].contains(&ext) {
            continue;
        }
        let info = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !info.is_file() || info.file_type().is_symlink() || info.len() == 0 || info.len() > SINGLE_VIDEO_MAX {
            return Err("yt-dlp produced an unsafe, empty or oversized media file".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if info.file_attributes() & 0x0000_0400 != 0 {
                return Err("yt-dlp output is a reparse-point file".into());
            }
        }
        if candidate.replace(path).is_some() {
            return Err("yt-dlp produced multiple candidate video files".into());
        }
    }
    candidate.ok_or_else(|| "yt-dlp completed without a verified single video file".into())
}

/// This method runs synchronously. The UI MUST call it from its download
/// worker, never from the rendering thread. It does nothing on note-open.
pub fn download_one(
    library: &Library,
    executable: &OsStr,
    url: &str,
    mut on_progress: impl FnMut(Progress),
) -> Result<PathBuf, String> {
    let id = youtube::video_id(url).ok_or("unsupported or invalid single YouTube URL")?;
    if let Some(existing) = library.existing(id).map_err(|e| e.to_string())? {
        return Ok(existing);
    }
    let used = library.used_bytes().map_err(|e| e.to_string())?;
    let allowance = LIBRARY_MAX.saturating_sub(used).min(SINGLE_VIDEO_MAX);
    if allowance < 1024 * 1024 {
        return Err("local video-library quota is exhausted".into());
    }
    let stage = Staging::new(library.root())?;
    let mut child = ChildGuard(Command::new(executable)
        .current_dir(stage.path())
        .args(command_args(url, stage.path(), allowance))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn().map_err(|e| format!("yt-dlp unavailable or could not start: {e}"))?);
    let stderr = child.0.stderr.take().ok_or("yt-dlp error stream unavailable")?;
    let diagnostics = std::thread::spawn(move || get_error(stderr));
    let stdout = child.0.stdout.take().ok_or("yt-dlp progress stream unavailable")?;
    let mut over_limit = false;
    for line in BufReader::new(stdout).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(progress) = parse_progress(line.trim()) {
            if progress.downloaded > allowance {
                over_limit = true;
                let _ = child.0.kill();
                break;
            }
            on_progress(progress);
        }
    }
    let status = child.0.wait().map_err(|e| e.to_string())?;
    let stderr = diagnostics.join().unwrap_or_default();
    if over_limit {
        return Err("video download exceeded the local-file quota".into());
    }
    if !status.success() {
        return Err(format!("yt-dlp exited with {status}: {stderr}"));
    }
    let staged = completed(stage.path(), id)?;
    let bytes = fs::metadata(&staged).map_err(|e| e.to_string())?.len();
    if used.saturating_add(bytes) > LIBRARY_MAX || bytes > allowance {
        return Err("video would exceed the local library quota".into());
    }
    File::open(&staged).and_then(|file| file.sync_all()).map_err(|e| e.to_string())?;
    let ext = staged.extension().and_then(|e| e.to_str()).ok_or("invalid yt-dlp output extension")?;
    let target = library.published(id, ext).map_err(|e| e.to_string())?;
    if let Some(existing) = library.existing(id).map_err(|e| e.to_string())? {
        return Ok(existing);
    }
    // Same-volume hard-link is an atomic create-new operation; unlike rename,
    // it does not replace a file committed by another Akula process.
    fs::hard_link(&staged, &target).map_err(|e| format!("unable to publish local video without overwrite: {e}"))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_is_structured_and_untrusted_lines_are_ignored() {
        assert_eq!(parse_progress("AKULA_PROGRESS:100:200:0"), Some(Progress { downloaded: 100, total: Some(200) }));
        assert_eq!(parse_progress("AKULA_PROGRESS:100:0:180"), Some(Progress { downloaded: 100, total: Some(180) }));
        assert!(parse_progress("[download] 100% of 50 MiB").is_none());
        assert!(parse_progress("AKULA_PROGRESS:not-a-number:30:10").is_none());
    }

    #[test]
    fn command_never_invokes_shell_or_uses_external_config() {
        let args: Vec<String> = command_args(
            "https://youtu.be/dQw4w9WgXcQ", Path::new("/tmp/private"), SINGLE_VIDEO_MAX
        ).iter().map(|s| s.to_string_lossy().into_owned()).collect();
        for needed in ["--ignore-config", "--no-playlist", "--no-exec", "--restrict-filenames", "--max-filesize"] {
            assert!(args.contains(&needed.to_owned()), "{needed}");
        }
        assert_eq!(args.last().unwrap(), "https://youtu.be/dQw4w9WgXcQ");
        assert!(args.contains(&"home:/tmp/private".to_string()));
        assert!(args.contains(&"--no-cookies".to_owned()));
        assert!(!args.iter().any(|s| s == "--cookies" || s == "--cookies-from-browser" || s.starts_with("--cookies=")));
    }

    #[test]
    fn incomplete_or_aliased_download_never_becomes_published() {
        let dir = tempfile::tempdir().unwrap();
        let id = "dQw4w9WgXcQ";
        fs::write(dir.path().join(format!("{id}.part")), b"part").unwrap();
        assert!(completed(dir.path(), id).is_err());
        fs::write(dir.path().join(format!("{id}.mp4")), b"file").unwrap();
        assert!(completed(dir.path(), id).is_ok());
        fs::write(dir.path().join(format!("{id}.webm")), b"second").unwrap();
        assert!(completed(dir.path(), id).is_err());
    }

    #[cfg(unix)]
    fn fake_executable(root: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let file = root.join("fake-yt-dlp.sh");
        fs::write(&file, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        let mut perms = fs::metadata(&file).unwrap().permissions();
        perms.set_mode(0o700);
        fs::set_permissions(&file, perms).unwrap();
        file
    }

    #[cfg(unix)]
    #[test]
    fn synthetic_process_downloads_once_and_preserves_completed_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let library = Library::at(temp.path().join("media"), &[]).unwrap();
        let exe = fake_executable(temp.path(), r#"
            stage=""
            for arg do
              case "$arg" in
                home:*) stage=$(printf '%s' "$arg" | cut -c6-);;
              esac
            done
            test -n "$stage"
            printf 'synthetic-media-bytes' > "$stage/dQw4w9WgXcQ.mp4"
            printf 'AKULA_PROGRESS:21:21:0\n'
        "#);
        let mut seen = Vec::new();
        let first = download_one(&library, exe.as_os_str(), "https://youtu.be/dQw4w9WgXcQ", |p| seen.push(p)).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"synthetic-media-bytes");
        assert!(seen.iter().any(|p| p.total == Some(21)));
        let second = download_one(&library, OsStr::new("missing-executable"), "https://youtu.be/dQw4w9WgXcQ", |_| {}).unwrap();
        assert_eq!(first, second, "dedup must avoid a second download");
        assert_eq!(fs::read_dir(library.root()).unwrap().count(), 1, "no partial temp directories");
    }

    #[cfg(unix)]
    #[test]
    fn failed_process_leaves_no_download_or_partials() {
        let temp = tempfile::tempdir().unwrap();
        let library = Library::at(temp.path().join("media"), &[]).unwrap();
        let exe = fake_executable(temp.path(), r#"
            stage=""
            for arg do
              case "$arg" in home:*) stage=$(printf '%s' "$arg" | cut -c6-);; esac
            done
            test -n "$stage"
            printf 'incomplete' > "$stage/dQw4w9WgXcQ.mp4.part"
            exit 12
        "#);
        assert!(download_one(&library, exe.as_os_str(), "https://youtu.be/dQw4w9WgXcQ", |_| {}).is_err());
        assert!(library.existing("dQw4w9WgXcQ").unwrap().is_none());
        assert_eq!(fs::read_dir(library.root()).unwrap().count(), 0, "temporary files cleaned");
    }

    #[test]
    fn invalid_host_is_rejected_before_spawning_executable() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::at(dir.path().join("media"), &[]).unwrap();
        let err = download_one(&library, OsStr::new("nonexistent-yt-dlp"), "https://evil.example/video", |_| {});
        assert!(err.unwrap_err().contains("unsupported"));
        assert_eq!(fs::read_dir(library.root()).unwrap().count(), 0);
    }
}
