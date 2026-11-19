use super::*;
#[test]
fn installed_download_refusal_creates_no_job_or_local_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let url = "https://youtu.be/AbCdEfGhIJK";
    let callback = Arc::new(|_: bool, _: &str| {});
    assert_eq!(enqueue(url.into(), temp.path().to_path_buf(), vec![], callback),
        Err("Video Tools execution is not approved for installed Akula"));
    assert!(status(url).is_none());
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
