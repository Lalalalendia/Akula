use super::{hash_bytes, write_file_atomic_impl, write_history_snapshot_atomic_impl};
use std::fs;

#[test]
fn unused_history_digest_can_be_skipped_without_changing_published_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let large = "История 😀漢字\r\nданные\0".repeat(100_000);
    for (index, content) in ["", "ASCII", "CRLF\r\nLF\n😀漢字", &large].into_iter().enumerate() {
        let hashed = temp.path().join(format!("hashed-{index}.md"));
        let history = temp.path().join(format!("history-{index}.md"));
        let expected_hash = hash_bytes(content.as_bytes());
        let returned = write_file_atomic_impl(&hashed, content, None).unwrap();
        write_history_snapshot_atomic_impl(&history, content).unwrap();

        assert_eq!(returned.hash, expected_hash);
        assert_eq!(fs::read(&hashed).unwrap(), content.as_bytes());
        assert_eq!(fs::read(&history).unwrap(), fs::read(&hashed).unwrap());

        let replacement = format!("версия {index}\n{}", content);
        let second = write_file_atomic_impl(&hashed, &replacement, None).unwrap();
        write_history_snapshot_atomic_impl(&history, &replacement).unwrap();
        assert_eq!(second.hash, hash_bytes(replacement.as_bytes()));
        assert_eq!(fs::read(&history).unwrap(), fs::read(&hashed).unwrap());
    }
}

#[test]
fn primary_note_cas_still_checks_raw_crlf_disk_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("CAS.md");
    let raw = b"old\r\nraw bytes";
    fs::write(&path, raw).unwrap();
    // This is the editor-normalized hash, NOT the actual disk hash.
    let wrong_expected = hash_bytes(b"old\nraw bytes");
    let error = write_file_atomic_impl(&path, "new version", Some(&wrong_expected))
        .unwrap_err();
    assert!(matches!(
        error,
        crate::files::error::FileCommandError::Conflict { .. }
    ));
    assert_eq!(fs::read(&path).unwrap(), raw);
}

#[test]
fn history_writer_rejects_missing_parent_without_publishing_any_version() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("no-such-history").join("version.md");
    assert!(write_history_snapshot_atomic_impl(&missing, "do not create").is_err());
    assert!(!missing.exists());
}
