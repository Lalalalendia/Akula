//! Offline yt-dlp regression tests, including synthetic Windows child processes.
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
        assert_eq!(args.windows(2).find(|pair| pair[0] == "--format").map(|pair| pair[1].as_str()),
            Some("bv[vcodec^=avc1][height<=720]+ba[acodec^=mp4a]/b[vcodec^=avc1][acodec^=mp4a][height<=720]"),
            "native Windows decoder needs AVC/AAC; an MP4 extension alone does not ensure playback");
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


    #[cfg(windows)]
    #[test]
    fn synthetic_windows_download_flushes_publishes_and_deduplicates() {
        // The real Windows child-process + FlushFileBuffers + hard-link path;
        // earlier Linux-only synthetic tests cannot catch Win32 handle rights.
        let temp = tempfile::tempdir().unwrap();
        let library = Library::at(temp.path().join("device-only"), &[]).unwrap();
        let stub = temp.path().join("fake-yt-dlp.cmd");
        fs::write(&stub, "@echo off\r\n> dQw4w9WgXcQ.mp4 echo synthetic-windows-video\r\necho AKULA_PROGRESS:25:25:0\r\nexit /b 0\r\n").unwrap();
        let mut progress = Vec::new();
        let first = download_one(
            &library, stub.as_os_str(), "https://youtu.be/dQw4w9WgXcQ",
            |p| progress.push(p),
        ).expect("Windows fake downloader must publish after successful flush");
        assert_eq!(first.parent(), Some(library.root()));
        assert!(fs::read(&first).unwrap().starts_with(b"synthetic-windows-video"));
        assert!(progress.iter().any(|p| p.total == Some(25)));
        assert_eq!(
            download_one(&library, OsStr::new("missing-yt-dlp.exe"),
                "https://youtu.be/dQw4w9WgXcQ", |_| {}).unwrap(),
            first,
            "a second note must reuse the same device-local file",
        );
        assert_eq!(fs::read_dir(library.root()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn synthetic_avc_aac_download_survives_restart_and_decodes_natively() {
        // Original two-second black/quiet fixture generated offline with
        // libx264 constrained-baseline + AAC-LC. The .mp4 test file is device
        // local and never copied into a workspace or sync directory.
        let temp = tempfile::tempdir().unwrap();
        let device_dir = temp.path().join("device-only");
        let library = Library::at(device_dir.clone(), &[]).unwrap();
        let fixture = temp.path().join("fixture.mp4");
        fs::write(&fixture, include_bytes!("fixtures/native_avc_aac_test.mp4")).unwrap();
        let stub = temp.path().join("fake-codec-yt-dlp.cmd");
        fs::write(&stub, "@echo off\r\ncopy /y \"%~dp0fixture.mp4\" \"dQw4w9WgXcQ.mp4\" >nul\r\nif errorlevel 1 exit /b 21\r\necho AKULA_PROGRESS:2956:2956:0\r\nexit /b 0\r\n").unwrap();
        let path = download_one(&library, stub.as_os_str(), "https://youtu.be/dQw4w9WgXcQ", |_| {})
            .expect("fixture must publish after synthetic downloader completes");
        assert_eq!(fs::read(&path).unwrap(), fs::read(&fixture).unwrap());
        drop(library);
        let reopened = Library::at(device_dir, &[]).unwrap();
        assert_eq!(reopened.existing("dQw4w9WgXcQ").unwrap(), Some(path.clone()),
            "device-local media must be discoverable after application restart");
        assert!(crate::video::toggle(&path),
            "Windows Media Foundation must open AVC/AAC native playback");
        let mut player = crate::video::Owned::default();
        player.add(path.clone());
        let mut decoded_frame = false;
        for _ in 0..150 {
            let _ = player.tick();
            if crate::video::state(&path).is_some_and(|state| state.frame.is_some()) {
                decoded_frame = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(decoded_frame, "downloaded AVC/AAC must produce a native decoded frame");
        player.stop();
        assert_eq!(fs::read_dir(reopened.root()).unwrap().count(), 1,
            "no duplicate or temporary video remains in local library");
    }

    #[cfg(windows)]
    #[test]
    fn synthetic_windows_failure_does_not_publish_a_partial_video() {
        let temp = tempfile::tempdir().unwrap();
        let library = Library::at(temp.path().join("device-only"), &[]).unwrap();
        let stub = temp.path().join("fake-failed-yt-dlp.cmd");
        fs::write(&stub, "@echo off\r\n> dQw4w9WgXcQ.mp4.part echo incomplete\r\nexit /b 12\r\n").unwrap();
        let outcome = download_one(&library, stub.as_os_str(),
            "https://youtu.be/dQw4w9WgXcQ", |_| {});
        assert!(outcome.is_err(), "nonzero yt-dlp exit must be rejected");
        assert!(library.existing("dQw4w9WgXcQ").unwrap().is_none());
        assert_eq!(fs::read_dir(library.root()).unwrap().count(), 0,
            "failed Windows staging must be removed");
    }

    #[test]
    fn invalid_host_is_rejected_before_spawning_executable() {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::at(dir.path().join("media"), &[]).unwrap();
        let err = download_one(&library, OsStr::new("nonexistent-yt-dlp"), "https://evil.example/video", |_| {});
        assert!(err.unwrap_err().contains("unsupported"));
        assert_eq!(fs::read_dir(library.root()).unwrap().count(), 0);
    }
