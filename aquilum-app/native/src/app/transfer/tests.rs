use super::*;

#[test]
fn drafts_are_metadata_only_and_cannot_start_transfer() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("archive.bin");
    std::fs::write(&file, b"hello").unwrap();
    let mut queue = TransferState::default();
    queue.stage(vec![file.clone(), file]);
    assert_eq!(queue.drafts.len(), 1);
    assert_eq!(queue.drafts[0].bytes, 5);
    assert!(!queue.drafts[0].paused);
    queue.toggle(0);
    assert!(queue.drafts[0].paused);
    queue.toggle(0);
    assert!(!queue.drafts[0].paused);
    queue.remove(0);
    assert!(queue.drafts.is_empty());
}

#[test]
fn directories_and_missing_files_are_not_staged() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing.bin");
    let mut queue = TransferState::default();
    queue.stage(vec![root.path().to_path_buf(), missing]);
    assert!(queue.drafts.is_empty());
    assert!(queue.error.is_some());
}

#[cfg(unix)]
#[test]
fn symlink_is_never_staged() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("real.bin");
    let link = root.path().join("link.bin");
    std::fs::write(&file, b"hello").unwrap();
    symlink(&file, &link).unwrap();
    let mut queue = TransferState::default();
    queue.stage(vec![link]);
    assert!(queue.drafts.is_empty());
}

#[test]
fn drafts_are_bounded() {
    let root = tempfile::tempdir().unwrap();
    let paths: Vec<PathBuf> = (0..MAX_DRAFTS + 2)
        .map(|n| {
            let p = root.path().join(format!("{n}.bin"));
            std::fs::write(&p, b"x").unwrap();
            p
        })
        .collect();
    let mut queue = TransferState::default();
    queue.stage(paths);
    assert_eq!(queue.drafts.len(), MAX_DRAFTS);
    assert!(queue.error.is_some());
}
