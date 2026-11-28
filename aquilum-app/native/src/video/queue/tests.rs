use super::*;
#[test]
fn untrusted_link_never_creates_a_download_job() {
    let dir = tempfile::tempdir().unwrap();
    let notify = Arc::new(|_: bool, _: &str| {});
    assert!(
        enqueue(
            "https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ".into(),
            dir.path().to_path_buf(),
            vec![],
            notify,
            |_| panic!("invalid URL must never publish")
        )
        .is_err()
    );
    assert!(status("https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ").is_none());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
