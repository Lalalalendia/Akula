use super::*;
use std::time::Duration;
#[test]
fn upstream_pin_and_local_root_are_not_arbitrary() {
    assert_eq!(VERSION, "0.18.0");
    assert_eq!(BYTES, 1_873_152);
    assert_eq!(SHA256.len(), 64);
    assert!(URL.starts_with("https://github.com/quickjs-ng/quickjs/releases/download/"));
    let folder = tempfile::tempdir().unwrap();
    let profile = folder.path().join("profile");
    fs::create_dir(&profile).unwrap();
    assert_eq!(status(&profile), Status::Missing);
    assert!(!local_root(&profile).unwrap().exists());
    assert!(local_root(&profile).unwrap().ends_with("tools/quickjs"));
}
#[test]
fn tampered_runtime_refused_and_removal_keeps_media() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("profile");
    fs::create_dir(&profile).unwrap();
    let library = profile.join("local-videos");
    fs::create_dir(&library).unwrap();
    let media = library.join("saved.mp4");
    fs::write(&media, b"existing offline video").unwrap();
    let root = local_root(&profile).unwrap();
    ensure_directory(&root).unwrap();
    fs::write(root.join(FILENAME), b"untrusted executable").unwrap();
    assert_eq!(status(&profile), Status::Invalid);
    assert!(verified_executable(&profile).is_err());
    remove(&profile).unwrap();
    assert_eq!(status(&profile), Status::Missing);
    assert_eq!(fs::read(&media).unwrap(), b"existing offline video");
}
#[test]
#[ignore = "CI-only explicit upstream QuickJS fetch; never installed automatically"]
fn user_optin_real_quickjs_download_remove_ci_only() {
    assert_eq!(std::env::var("AKULA_VIDEO21_RUN_LAB").ok().as_deref(), Some("1"));
    assert_eq!(std::env::var("RUNNER_ENVIRONMENT").ok().as_deref(), Some("github-hosted"));
    let folder = tempfile::tempdir().unwrap();
    let profile = folder.path().join("isolated");
    fs::create_dir(&profile).unwrap();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    start(profile.clone(), wake).expect("explicit QuickJS install should start");
    let mut ready = false;
    for _ in 0..240 {
        match status(&profile) {
            Status::Installed => { ready = true; break; },
            Status::Failed(reason) => panic!("upstream QuickJS fetch failed: {reason}"),
            _ => std::thread::sleep(Duration::from_millis(250)),
        }
    }
    assert!(ready, "pinned QuickJS should be ready after consent");
    let path = verified_executable(&profile).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), BYTES);
    remove(&profile).unwrap();
    assert_eq!(status(&profile), Status::Missing);
    assert!(!path.exists());
    println!("VIDEO21_QUICKJS_PINNED_OPTIN_PASS verified=1 removed=1 release=0");
}

#[test]
fn refuses_wrong_length_hash_and_non_file() {
    super::super::tests::assert_invalid_pinned_file(BYTES, verify, [
        "QuickJS size differs from pinned official executable",
        "QuickJS SHA-256 does not match pinned upstream release",
    ]);
}
