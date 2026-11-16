//! No-network acceptance of click-only separate-track loading + Rust mux.
use super::*;

const URL: &str = "https://youtu.be/dQw4w9WgXcQ";
const ID: &str = "dQw4w9WgXcQ";

fn fixture(root: &Path) -> PathBuf {
    let path = root.join("fixture.mp4");
    fs::write(&path, include_bytes!("fixtures/native_avc_aac_test.mp4")).unwrap();
    path
}

#[cfg(unix)]
fn script(path: PathBuf, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn two_explicit_tracks_are_remuxed_once_without_any_cloud_or_vault_write() {
    let temp = tempfile::tempdir().unwrap();
    let vault = temp.path().join("vault");
    fs::create_dir(&vault).unwrap();
    let library = Library::at(temp.path().join("local-videos"), &[vault.clone()]).unwrap();
    let fixture_path = fixture(temp.path());
    let yt = script(temp.path().join("fake-yt-dlp.sh"), r#"
        if test -f video.mp4; then
          printf synthetic-audio > audio.m4a
        else
          printf synthetic-video > video.mp4
        fi
        printf 'AKULA_PROGRESS:16:16:0\n'
    "#);
    let mux_body = format!("test \"$1\" = 'mux-pair'\ntest -s \"$2\"\ntest -s \"$3\"\ncp '{}' \"$4\"\necho 'DASH_RUST_MUX_PAIR_PASS protocol=v2 duration_ms=6000 video_samples=60 audio_samples=94 ffmpeg_in_runtime=0'", fixture_path.display());
    let mux = script(temp.path().join("fake-rust-mux.sh"), &mux_body);
    let mut progress = Vec::new();
    let first = download_rust(&library, yt.as_os_str(), &mux, Path::new("/tmp/mock-qjs"), URL, |p| progress.push(p))
        .expect("separate streams should publish an original local media file");
    assert_eq!(first, library.root().join(format!("{ID}.mp4")));
    assert_eq!(fs::read(&first).unwrap(), fs::read(fixture_path).unwrap());
    assert!(progress.iter().any(|p| p.downloaded > 16),
        "second track progress should be aggregated");
    assert_eq!(download_rust(&library, OsStr::new("missing-yt-dlp"), &temp.path().join("no-mux"), Path::new("/tmp/mock-qjs"), URL, |_| {}).unwrap(), first);
    assert_eq!(fs::read_dir(library.root()).unwrap().count(), 1, "staging must be removed");
    assert_eq!(fs::read_dir(vault).unwrap().count(), 0, "vault must stay empty");
}

#[cfg(unix)]
#[test]
fn second_track_failure_never_publishes_or_leaves_temporary_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let library = Library::at(temp.path().join("local"), &[]).unwrap();
    let mux = script(temp.path().join("fake-mux.sh"), "exit 0");
    let downloader = script(temp.path().join("bad-yt-dlp.sh"), r#"
        if test -f video.mp4; then
          printf 'partial' > audio.m4a.part
          exit 12
        fi
        printf synthetic-video > video.mp4
        printf 'AKULA_PROGRESS:15:15:0\n'
    "#);
    assert!(download_rust(&library, downloader.as_os_str(), &mux, Path::new("/tmp/mock-qjs"), URL, |_| {}).is_err());
    assert!(library.existing(ID).unwrap().is_none());
    assert_eq!(fs::read_dir(library.root()).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn native_windows_two_track_mock_uses_local_library_and_writable_flush() {
    let temp = tempfile::tempdir().unwrap();
    let vault = temp.path().join("vault");
    fs::create_dir(&vault).unwrap();
    let library = Library::at(temp.path().join("device-only"), &[vault.clone()]).unwrap();
    let fixture_path = fixture(temp.path());
    let yt = temp.path().join("fake-yt-dlp.cmd");
    fs::write(&yt, "@echo off\r\nif not exist video.mp4 (\r\n  > video.mp4 echo mock-video\r\n) else (\r\n  > audio.m4a echo mock-audio\r\n)\r\necho AKULA_PROGRESS:12:12:0\r\nexit /b 0\r\n").unwrap();
    let mux = temp.path().join("fake-rust-mux.cmd");
    fs::write(&mux, "@echo off\r\nif not \"%~1\"==\"mux-pair\" exit /b 4\r\ncopy /y \"%~dp0fixture.mp4\" \"%~4\" >nul\r\nif errorlevel 1 exit /b 5\r\necho DASH_RUST_MUX_PAIR_PASS protocol=v2 duration_ms=6000 video_samples=60 audio_samples=94 ffmpeg_in_runtime=0\r\nexit /b 0\r\n").unwrap();
    let first = download_rust(&library, yt.as_os_str(), &mux, Path::new("C:\\mock-qjs.exe"), URL, |_| {})
        .expect("Windows mock Rust mux should publish after successful FlushFileBuffers");
    assert_eq!(fs::read(&first).unwrap(), fs::read(fixture_path).unwrap());
    assert_eq!(fs::read_dir(library.root()).unwrap().count(), 1);
    assert_eq!(fs::read_dir(vault).unwrap().count(), 0);
}

#[test]
fn rejects_invalid_video_url_or_untrusted_mux_without_stage_creation() {
    let temp = tempfile::tempdir().unwrap();
    let library = Library::at(temp.path().join("local"), &[]).unwrap();
    let missing = temp.path().join("missing-rust-mux");
    assert!(download_rust(&library, OsStr::new("missing-yt-dlp"), &missing, Path::new("/tmp/mock-qjs"),
        "https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ", |_| {}).is_err());
    assert!(download_rust(&library, OsStr::new("missing-yt-dlp"), Path::new("relative/mux"), Path::new("/tmp/mock-qjs"),
        URL, |_| {}).unwrap_err().contains("absolute"));
    assert_eq!(fs::read_dir(library.root()).unwrap().count(), 0);
}

#[test]
fn yt_dlp_single_track_args_do_not_request_merge_or_postprocessing() {
    let stage = Path::new("/tmp/private-device-media");
    for track in [Track::Video, Track::Audio] {
        let args: Vec<String> = track_args(stage, track, SINGLE_VIDEO_MAX, URL, Path::new("/tmp/mock-qjs"))
            .iter().map(|s| s.to_string_lossy().into_owned()).collect();
        assert!(args.contains(&"--ignore-config".into()));
        assert!(args.contains(&"--no-plugin-dirs".into()));
        assert!(args.contains(&"--no-remote-components".into()));
        assert!(args.contains(&"--no-js-runtimes".into()));
        assert!(args.windows(2).any(|a| a[0] == "--js-runtimes"
            && a[1] == "quickjs:/tmp/mock-qjs"),
            "the pinned runtime must be the only JS engine");
        assert!(args.contains(&"--no-playlist".into()));
        assert!(args.contains(&"--no-exec".into()));
        assert!(args.windows(2).any(|a| a[0] == "--fixup" && a[1] == "never"));
        assert!(!args.contains(&"--merge-output-format".into()));
        assert!(!args.contains(&"--postprocessor-args".into()));
        assert_eq!(args.last().map(String::as_str), Some(URL));
        assert!(args.windows(2).any(|a| a[0] == "--format" && a[1] == track.format()));
        assert!(args.windows(2).any(|a| a[0] == "--output" && a[1] == track.output()));
    }
}


#[cfg(windows)]
#[test]
fn rejects_modified_or_untrusted_runtime_files_before_starting_ytdlp() {
    let temp = tempfile::tempdir().unwrap();
    let wrong = temp.path().join("wrong-runtime.exe");
    fs::write(&wrong, b"NOT A TRUSTED JS RUNTIME").unwrap();
    assert!(verify_pin(&wrong, QUICKJS_WINDOWS_BYTES, QUICKJS_WINDOWS_SHA256, "QuickJS").is_err());
    assert!(verify_pin(Path::new("qjs.exe"), QUICKJS_WINDOWS_BYTES, QUICKJS_WINDOWS_SHA256, "QuickJS").is_err());
    let wrong_size = temp.path().join("wrong-size.exe");
    fs::write(&wrong_size, vec![b'x'; QUICKJS_WINDOWS_BYTES as usize]).unwrap();
    assert!(verify_pin(&wrong_size, QUICKJS_WINDOWS_BYTES, QUICKJS_WINDOWS_SHA256, "QuickJS")
        .unwrap_err().contains("SHA-256"), "even the expected size must not be enough");
}

#[cfg(windows)]
#[test]
fn official_pins_accept_only_the_checked_windows_quickjs_and_ytdlp() {
    let qjs = std::env::var_os("AKULA_TEST_QUICKJS_BIN");
    let yt = std::env::var_os("AKULA_TEST_YTDLP_BIN");
    match (qjs, yt) {
        (Some(qjs), Some(yt)) => {
            verify_pinned_runtime(&PathBuf::from(yt), &PathBuf::from(qjs))
                .expect("official Windows toolchain must match both pinned SHA256 values");
            println!("VIDEO09_OFFICIAL_RUNTIME_SHA256_PASS official_quickjs=1 official_ytdlp=1");
        }
        (None, None) => {
            // Local developer builds do not need to download third-party binaries.
            println!("VIDEO09_OFFICIAL_RUNTIME_NOT_SUPPLIED local_developer_test=1");
        }
        _ => panic!("both AKULA_TEST_QUICKJS_BIN and AKULA_TEST_YTDLP_BIN must be supplied"),
    }
}


#[test]
fn refuses_old_unversioned_or_zero_duration_mux_receipts() {
    let valid = "DASH_RUST_MUX_PAIR_PASS protocol=v2 duration_ms=6000 video_samples=60 audio_samples=94 ffmpeg_in_runtime=0\n";
    assert!(valid_mux_receipt(valid));
    for bad in [
        "",
        "DASH_RUST_MUX_PAIR_PASS video_samples=60 audio_samples=94 ffmpeg_in_runtime=0\n",
        "DASH_RUST_MUX_PAIR_PASS protocol=v1 duration_ms=6000 video_samples=60 audio_samples=94 ffmpeg_in_runtime=0\n",
        "DASH_RUST_MUX_PAIR_PASS protocol=v2 duration_ms=0 video_samples=60 audio_samples=94 ffmpeg_in_runtime=0\n",
        "DASH_RUST_MUX_PAIR_PASS protocol=v2 duration_ms=6000 video_samples=0 audio_samples=94 ffmpeg_in_runtime=0\n",
        "DASH_RUST_MUX_PAIR_PASS protocol=v2 duration_ms=6000 video_samples=60 audio_samples=94 ffmpeg_in_runtime=1\n",
    ] {
        assert!(!valid_mux_receipt(bad), "accepted invalid receipt: {bad}");
    }
}

#[cfg(unix)]
#[test]
fn successful_but_stale_rust_mux_does_not_publish_any_video() {
    let temp = tempfile::tempdir().unwrap();
    let library = Library::at(temp.path().join("local"), &[]).unwrap();
    let fixture_path = fixture(temp.path());
    let yt = script(temp.path().join("yt.sh"), r#"
        if test -f video.mp4; then
            printf synthetic-audio > audio.m4a
        else
            printf synthetic-video > video.mp4
        fi
    "#);
    let old_mux = script(temp.path().join("old-mux.sh"),
        &format!("cp '{}' \"$4\"\necho 'DASH_RUST_MUX_PAIR_PASS video_samples=60 audio_samples=94'", fixture_path.display()));
    let error = download_rust(&library, yt.as_os_str(), &old_mux,
        Path::new("/tmp/mock-qjs"), URL, |_| {}).unwrap_err();
    assert!(error.contains("outdated"), "{error}");
    assert!(library.existing(ID).unwrap().is_none());
    assert_eq!(fs::read_dir(library.root()).unwrap().count(), 0);
}
