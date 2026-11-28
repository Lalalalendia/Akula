//! Strict Windows-only Akula + real yt-dlp + QuickJS + Rust mux integration.
//! The test downloads only a synthetic MPD from 127.0.0.1 in a dedicated CI job.
use super::*;
const URL: &str = "https://youtu.be/dQw4w9WgXcQ";
const ID: &str = "dQw4w9WgXcQ";


#[cfg(windows)]
#[test]
fn real_windows_native_pipeline_remuxes_local_dash_without_syncing_media() {
    // Missing all five env vars means the ordinary offline unit run should
    // not download third-party executables. Partial configuration is an
    // error: the dedicated CI workflow must provide every explicit input.
    let keys = [
        "AKULA_VIDEO12_MPD_URL",
        "AKULA_VIDEO12_YTDLP_BIN",
        "AKULA_VIDEO12_QUICKJS_BIN",
        "AKULA_VIDEO12_RUST_MUX_BIN",
        "AKULA_VIDEO12_CAPTURE",
    ];
    let values: Vec<_> = keys.iter().map(|key| std::env::var_os(key)).collect();
    if values.iter().all(Option::is_none) {
        println!("VIDEO12_NATIVE_REAL_TOOLCHAIN_NOT_CONFIGURED no_download=1");
        return;
    }
    assert!(values.iter().all(Option::is_some),
        "VIDEO-12 CI requires every explicit test tool, URL and capture path");
    let manifest = values[0].as_ref().unwrap().to_string_lossy();
    assert!(manifest.starts_with("http://127.0.0.1:")
            && manifest.ends_with("/manifest.mpd"),
        "the real downloader may only fetch the loopback synthetic MPD");
    let ytdlp = PathBuf::from(values[1].as_ref().unwrap().as_os_str());
    let quickjs = PathBuf::from(values[2].as_ref().unwrap().as_os_str());
    let rust_mux = PathBuf::from(values[3].as_ref().unwrap().as_os_str());
    let capture = PathBuf::from(values[4].as_ref().unwrap().as_os_str());
    assert!(capture.is_absolute() && !capture.exists());
    verify_pinned_runtime(&ytdlp, &quickjs)
        .expect("no unpinned yt-dlp or JS runtime may run in the native acceptance");
    check_mux(&rust_mux).expect("real Rust mux helper required");

    let scratch = tempfile::tempdir().unwrap();
    let vault = scratch.path().join("synced-notes");
    fs::create_dir(&vault).unwrap();
    let note = vault.join("note.md");
    let original = format!("Video link: {URL}\n");
    fs::write(&note, &original).unwrap();
    let library = Library::at(scratch.path().join("device-media"), &[vault.clone()])
        .expect("private video library outside synchronized notes");

    let mut progress = Vec::new();
    let output = download_rust_from_source(
        &library, ytdlp.as_os_str(), &rust_mux, &quickjs,
        URL, &manifest, |point| progress.push(point),
    ).expect("real yt-dlp and Rust mux must publish a valid local MP4");
    assert!(output.newly_published); let output = output.path; assert_eq!(output, library.root().join(format!("{ID}.mp4")));
    assert!(output.metadata().unwrap().len() > 1_000);
    assert!(progress.iter().any(|p| p.downloaded > 0),
        "native progress callback must receive bytes from real yt-dlp");

    let second = download_rust_from_source(
        &library, OsStr::new("missing-downloader"), Path::new("no-mux"),
        Path::new("no-quickjs"), URL, "http://127.0.0.1:1/invalid", |_| {},
    ).expect("existing media must be deduplicated before looking up executables");
    assert!(!second.newly_published); assert_eq!(second.path, output);
    assert_eq!(fs::read_dir(library.root()).unwrap().count(), 1,
        "staging files must be removed after successful mux and publication");
    assert_eq!(fs::read_to_string(&note).unwrap(), original,
        "synchronized Markdown source must never be modified");
    assert_eq!(fs::read_dir(&vault).unwrap().count(), 1,
        "no MP4, M4A, or transient yt-dlp files may enter a vault");

    fs::copy(&output, &capture).expect("capture the synthetic MP4 outside vault for reference decoder");
    println!("VIDEO12_NATIVE_REAL_TOOLCHAIN_PASS local_only=1 md_unchanged=1 dedupe=1 progress_events={} output_bytes={} no_youtube_requests=1",
        progress.len(), output.metadata().unwrap().len());
}
