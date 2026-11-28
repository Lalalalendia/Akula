use super::*;
#[test]
fn installed_download_refusal_creates_no_job_or_local_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let url = "https://youtu.be/AbCdEfGhIJK";
    let callback = Arc::new(|_: bool, _: &str| {});
    assert_eq!(enqueue(url.into(), temp.path().to_path_buf(), vec![], callback, |_| panic!("refused download must never publish")),
        Ok(true));
    // A missing verified tool fails asynchronously, but cannot run any
    // executable from PATH or create media bytes.
    for _ in 0..100 {
        if matches!(status(url), Some(Status::Failed(_))) { break; }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(matches!(status(url), Some(Status::Failed(_))));
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
