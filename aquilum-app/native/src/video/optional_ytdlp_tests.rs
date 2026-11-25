use super::*;
#[test]
fn official_release_pin_is_fixed_and_distinct_from_user_paths() {
    assert_eq!(VERSION, "2026.10.26");
    assert!(URL.starts_with("https://github.com/yt-dlp/yt-dlp/releases/download/"));
    assert_eq!(SHA256.len(), 64);
    assert_eq!(BYTES, 23_575_744);
}
#[test]
fn missing_invalid_and_manual_removal_do_not_touch_local_media() {
    let temp = tempfile::tempdir().unwrap();
    let profile = temp.path().join("AkulaProfile");
    fs::create_dir(&profile).unwrap();
    let videos = profile.join("local-videos");
    fs::create_dir(&videos).unwrap();
    let saved = videos.join("already-downloaded.mp4");
    fs::write(&saved, b"local offline video stays").unwrap();
    assert_eq!(status(&profile), Status::Missing);
    assert!(!local_root(&profile).unwrap().exists(),
            "checking status cannot install tools or create directories");
    let root = local_root(&profile).unwrap();
    ensure_directory(&root).unwrap();
    fs::write(root.join(FILENAME), b"malicious executable").unwrap();
    assert_eq!(status(&profile), Status::Invalid);
    assert!(verified_executable(&profile).is_err());
    remove(&profile).unwrap();
    assert_eq!(status(&profile), Status::Missing);
    assert_eq!(fs::read(&saved).unwrap(), b"local offline video stays");
}

#[test]
#[ignore = "CI-only, user-initiated official GitHub download; no executable execution"]
fn user_optin_real_download_remove_ci_only() {
    assert_eq!(std::env::var("AKULA_VIDEO20_RUN_LAB").ok().as_deref(), Some("1"));
    assert_eq!(std::env::var("RUNNER_ENVIRONMENT").ok().as_deref(), Some("github-hosted"));
    let temp = tempfile::tempdir().unwrap();
    let profile = temp.path().join("isolated-profile");
    fs::create_dir(&profile).unwrap();
    assert_eq!(status(&profile), Status::Missing);
    assert!(!local_root(&profile).unwrap().exists());
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    start(profile.clone(), wake).expect("explicit opt-in starts a single background download");
    let mut ok = false;
    for _ in 0..240 {
        match status(&profile) {
            Status::Installed => { ok = true; break; },
            Status::Failed(reason) => panic!("pinned official download failed: {reason}"),
            _ => std::thread::sleep(Duration::from_millis(250)),
        }
    }
    assert!(ok, "pinned yt-dlp must be installed after user consent in CI lab");
    let binary = verified_executable(&profile).unwrap();
    assert_eq!(fs::metadata(&binary).unwrap().len(), BYTES);
    // This CI-only test exercises the exact pinned executable, not
    // arbitrary PATH tools. Installation itself never executes it.
    let version = std::process::Command::new(&binary)
        .args(["--ignore-config", "--no-plugin-dirs", "--no-js-runtimes", "--version"])
        .output().expect("verified official yt-dlp should launch on Windows");
    assert!(version.status.success(), "verified yt-dlp --version failed");
    assert_eq!(String::from_utf8_lossy(&version.stdout).trim(), VERSION);
    remove(&profile).unwrap();
    assert_eq!(status(&profile), Status::Missing);
    assert!(!local_root(&profile).unwrap().join(FILENAME).exists());
    println!("VIDEO20_USER_INITIATED_OFFICIAL_YTDLP_PASS file_verified=1 removed=1 release=0");
}
#[test]
fn never_accept_an_unpinned_download_or_arbitrary_binary() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join(FILENAME);
    fs::write(&file, b"untrusted payload").unwrap();
    assert!(verified(&file).is_err());
}

pub(super) fn assert_invalid_pinned_file(
    bytes: u64,
    verify: impl Fn(&Path) -> Result<(), String>,
    [length_error, hash_error]: [&str; 2],
) {
    let scratch = tempfile::tempdir().unwrap();
    let candidate = scratch.path().join("candidate.exe");
    assert!(verify(&candidate).is_err(), "missing file must be refused");

    // Real files with synthetic, never-executed contents.
    // Exact length is necessary but not sufficient for admission.
    for (len, error) in [(bytes - 1, length_error), (bytes + 1, length_error),
                         (bytes, hash_error)] {
        let file = File::create(&candidate).unwrap();
        file.set_len(len).unwrap();
        drop(file);
        assert_eq!(verify(&candidate).unwrap_err(), error);
        assert_eq!(fs::metadata(&candidate).unwrap().len(), len);
    }

    let directory = scratch.path().join("directory.exe");
    fs::create_dir(&directory).unwrap();
    assert_eq!(verify(&directory).unwrap_err(),
        "optional downloader path is aliased or not regular");
    assert!(directory.is_dir());
}

#[test]
fn refuses_wrong_length_hash_and_non_file() {
    assert_invalid_pinned_file(BYTES, verified, [
        "yt-dlp executable length does not match the pinned release",
        "yt-dlp SHA-256 differs from the pinned official release",
    ]);
}
