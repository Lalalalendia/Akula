use super::*;

fn opened(root: &Path, bytes: &[u8]) -> File {
    let path = root.join("private-stage.bin");
    fs::write(&path, bytes).unwrap();
    File::open(path).unwrap()
}

#[test]
fn creates_new_verified_file_with_bounded_copy_and_exact_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = (0..(64 * 1024 + 23))
        .map(|i| ((i * 31 + 5) % 251) as u8)
        .collect::<Vec<_>>();
    let mut stage = opened(dir.path(), &bytes);
    let output = dir.path().join("user-chosen-name.bin");
    let expected = *blake3::hash(&bytes).as_bytes();
    let receipt =
        save_verified_stage_as_new(&mut stage, bytes.len() as u64, expected, &output).unwrap();
    assert_eq!(receipt.path, output);
    assert_eq!(receipt.bytes, bytes.len() as u64);
    assert_eq!(receipt.blake3_root, expected);
    assert_eq!(fs::read(&output).unwrap(), bytes);
    assert_eq!(
        fs::read(dir.path().join("private-stage.bin")).unwrap(),
        bytes
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn a_preexisting_destination_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let mut stage = opened(dir.path(), b"selected bytes");
    let output = dir.path().join("keep.bin");
    fs::write(&output, b"my existing document").unwrap();
    let outcome = save_verified_stage_as_new(
        &mut stage,
        14,
        *blake3::hash(b"selected bytes").as_bytes(),
        &output,
    );
    assert!(matches!(outcome, Err(InboxError::AlreadyExists)));
    assert_eq!(fs::read(&output).unwrap(), b"my existing document");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn wrong_root_and_changed_size_never_publish_partial_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut stage = opened(dir.path(), b"byte-for-byte sample");
    let output = dir.path().join("never-created.bin");
    let bad_hash = *blake3::hash(b"forged commitment").as_bytes();
    assert!(matches!(
        save_verified_stage_as_new(&mut stage, 20, bad_hash, &output),
        Err(InboxError::DigestMismatch)
    ));
    assert!(!output.exists());
    assert!(matches!(
        save_verified_stage_as_new(&mut stage, 19, bad_hash, &output),
        Err(InboxError::SizeMismatch)
    ));
    assert!(!output.exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn zero_or_oversized_manifest_is_refused_before_publication() {
    let dir = tempfile::tempdir().unwrap();
    let mut stage = opened(dir.path(), b"stage bytes");
    let target = dir.path().join("invalid.bin");
    let hash = *blake3::hash(b"stage bytes").as_bytes();
    assert!(matches!(
        save_verified_stage_as_new(&mut stage, 0, hash, &target),
        Err(InboxError::InvalidManifest)
    ));
    assert!(matches!(
        save_verified_stage_as_new(&mut stage, MAX_INBOX_ITEM_BYTES + 1, hash, &target),
        Err(InboxError::InvalidManifest)
    ));
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn stage_path_swap_after_open_cannot_change_published_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("private-stage.bin");
    fs::write(&source, b"pinned original data").unwrap();
    let mut opened_stage = File::open(&source).unwrap();
    fs::rename(&source, dir.path().join("renamed-stage.bin")).unwrap();
    fs::write(&source, b"attacker replacement").unwrap();
    let output = dir.path().join("correct-copy.bin");
    save_verified_stage_as_new(
        &mut opened_stage,
        20,
        *blake3::hash(b"pinned original data").as_bytes(),
        &output,
    )
    .unwrap();
    assert_eq!(fs::read(&output).unwrap(), b"pinned original data");
}

#[cfg(unix)]
#[test]
fn symlinked_destination_parent_is_refused() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let parent_link = dir.path().join("redirect");
    symlink(&outside, &parent_link).unwrap();
    let mut stage = opened(dir.path(), b"trusted");
    let target = parent_link.join("unexpected.bin");
    assert!(matches!(
        save_verified_stage_as_new(&mut stage, 7, *blake3::hash(b"trusted").as_bytes(), &target),
        Err(InboxError::InvalidDestination)
    ));
    assert!(!outside.join("unexpected.bin").exists());
}
