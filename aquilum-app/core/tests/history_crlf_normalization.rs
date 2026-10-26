//! Regressions for verifying history snapshots after newline normalization.
use aquilum_core::history::store::{has_text, VersionFile};
use aquilum_core::history::Source;
use std::fs;

fn version() -> VersionFile {
    VersionFile {
        id: "sample.md".to_owned(),
        at_ms: 0,
        device: "test".to_owned(),
        source: Source::Me,
        from_ms: None,
        name: None,
        is_current: false,
    }
}

#[test]
fn crlf_history_matches_normalized_lf_text() {
    let root = tempfile::tempdir().unwrap();
    let raw = "# Note\r\nТекст 🦈\r\n";
    let normalized = "# Note\nТекст 🦈\n";
    let file = root.path().join(version().id);
    fs::write(&file, raw).unwrap();

    assert_ne!(fs::metadata(&file).unwrap().len(), normalized.len() as u64);
    assert!(has_text(root.path(), &version(), normalized));
}

#[test]
fn crlf_length_allowance_does_not_accept_different_content() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(version().id), "one\r\nTWO\r\n").unwrap();
    assert!(!has_text(root.path(), &version(), "one\ntwo\n"));
    assert!(!has_text(root.path(), &version(), "one\n"));
}

#[test]
fn missing_history_snapshot_is_not_a_match() {
    let root = tempfile::tempdir().unwrap();
    assert!(!has_text(root.path(), &version(), "one\n"));
}

#[test]
fn mixed_cr_and_crlf_history_snapshot_matches_without_trailing_newline() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(version().id), "first\r\nsecond\rthird\r\nfourth").unwrap();
    assert!(has_text(root.path(), &version(), "first\nsecond\nthird\nfourth"));
    assert!(!has_text(root.path(), &version(), "first\nsecond\nthird\nother"));
}
