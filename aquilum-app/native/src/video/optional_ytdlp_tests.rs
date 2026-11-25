use super::*;
use std::{fs::File, time::Duration};
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

fn transfer_fixture(bytes: u64, sha256: &str, notice_every: u64) -> pinned_transfer::Transfer<'_> {
    pinned_transfer::Transfer {
        url: "unused in offline test", bytes, sha256, notice_every,
        errors: ["request", "status", "read", "oversize", "write", "flush", "mismatch"],
    }
}

#[test]
fn shared_transfer_accepts_exact_bytes_and_reports_progress() {
    // Known SHA-256 of "abc"; no executable or network request involved.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stage.part");
    let mut notices = Vec::new();
    transfer_fixture(3, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", 2)
        .copy_checked(&mut &b"abc"[..], File::create(&path).unwrap(), |n| notices.push(n))
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"abc");
    assert_eq!(notices, vec![3]);
}

#[test]
fn shared_transfer_rejects_truncated_oversize_and_same_length_tampered_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stage.part");
    let pin = transfer_fixture(3, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", 2);
    for (bytes, error, written) in [
        (&b"ab"[..], "mismatch", &b"ab"[..]),
        (&b"abcd"[..], "oversize", &b""[..]),
        (&b"abd"[..], "mismatch", &b"abd"[..]),
    ] {
        let mut stream = bytes;
        assert_eq!(pin.copy_checked(&mut stream, File::create(&path).unwrap(), |_| {}).unwrap_err(), error);
        assert_eq!(fs::read(&path).unwrap(), written);
    }
}

#[test]
fn shared_transfer_preserves_stream_and_write_failures() {
    struct Broken;
    impl std::io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("synthetic interrupted transfer"))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stage.part");
    let pin = transfer_fixture(3, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", 2);
    assert_eq!(pin.copy_checked(&mut Broken, File::create(&path).unwrap(), |_| {}).unwrap_err(),
        "read: synthetic interrupted transfer");
    assert!(pin.copy_checked(&mut &b"abc"[..], File::open(&path).unwrap(), |_| {}).unwrap_err().starts_with("write: "));
}

#[test]
fn shared_lifecycle_rejects_duplicate_start_and_keeps_tool_and_profile_state_separate() {
    static FIRST: ToolState = ToolState::new();
    static SECOND: ToolState = ToolState::new();
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("profile");
    let other = dir.path().join("other");
    let (release, wait) = std::sync::mpsc::channel();
    let (notify, notices) = std::sync::mpsc::channel();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || { let _ = notify.send(()); });
    FIRST.start(profile.clone(), wake.clone(), |_| Status::Missing, move |_, _| {
        wait.recv_timeout(Duration::from_secs(10)).map_err(|e| e.to_string())?;
        Ok(())
    }, ["busy", "installed"]).unwrap();
    notices.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(FIRST.revision(), 1);
    assert_eq!(FIRST.states().lock().unwrap().get(&profile), Some(&Status::Downloading(0)));
    assert!(!FIRST.states().lock().unwrap().contains_key(&other));
    assert!(!SECOND.states().lock().unwrap().contains_key(&profile));
    assert_eq!(SECOND.revision(), 0);
    assert_eq!(FIRST.start(profile.clone(), wake, |_| panic!("busy must stop before disk check"),
        |_, _| panic!("busy must not spawn"), ["busy", "installed"]).unwrap_err(), "busy");
    assert_eq!(FIRST.revision(), 1);
    release.send(()).unwrap();
    notices.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(FIRST.states().lock().unwrap().get(&profile), Some(&Status::Installed));
    assert_eq!(FIRST.revision(), 2);
    assert_eq!(SECOND.revision(), 0);
}

#[test]
fn shared_lifecycle_rejects_already_installed_and_reports_fetch_failure() {
    static TOOL: ToolState = ToolState::new();
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("profile");
    let (notify, notices) = std::sync::mpsc::channel();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || { let _ = notify.send(()); });
    assert_eq!(TOOL.start(profile.clone(), wake.clone(), |_| Status::Installed,
        |_, _| panic!("installed must not spawn"), ["busy", "installed"]).unwrap_err(), "installed");
    assert_eq!(TOOL.revision(), 0);
    assert!(!TOOL.states().lock().unwrap().contains_key(&profile));
    assert!(notices.try_recv().is_err());
    TOOL.start(profile.clone(), wake, |_| Status::Missing, |_, _| Err("synthetic failure".into()),
        ["busy", "installed"]).unwrap();
    for _ in 0..2 { notices.recv_timeout(Duration::from_secs(10)).unwrap(); }
    assert_eq!(TOOL.states().lock().unwrap().get(&profile),
        Some(&Status::Failed("synthetic failure".into())));
    assert_eq!(TOOL.revision(), 2);
}
