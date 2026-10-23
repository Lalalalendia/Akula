//! AK-DOC-C1a: official JSON Canvas 1.0 allows missing "nodes" and "edges".
//! These are independent optional arrays, not a requirement to invent blanks.
use aquilum_core::files::structured::StructuredDocument;
use serde_json::{json, Value};
use std::fs;

#[test]
fn valid_optional_sections_open_without_implicitly_mutating_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let cases: &[&[u8]] = &[
        b"{}\r\n",
        br#"{"nodes":[]}"#,
        br#"{"edges":[]}"#,
        br#"{"nodes":[],"edges":[]}"#,
        br#"{"nodes":[],"custom":{"opaque":42,"future":[1,2]}}"#,
        br#"{"edges":[],"extra":["preserve"]}"#,
    ];
    for (index, raw) in cases.iter().enumerate() {
        let path = dir.path().join(format!("valid-{index}.canvas"));
        fs::write(&path, raw).unwrap();
        let doc = StructuredDocument::open(&path).expect("optional arrays are valid");
        let value = doc.canvas().unwrap();
        assert!(value.is_object());
        assert_eq!(fs::read(&path).unwrap(), *raw, "read must not normalize JSON");
        // The Core snapshot normalizes CRLF in memory; only the file's
        // original bytes must remain byte-for-byte unchanged on disk.
        let normalized = std::str::from_utf8(raw).unwrap().replace("\r\n", "\n");
        assert_eq!(doc.text(), normalized);
    }
}

#[test]
fn editing_supported_subset_keeps_unknown_fields_and_missing_sections_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vendor.canvas");
    let original = br#"{"nodes":[],"vendor":{"deep":{"custom":true}}}"#;
    fs::write(&path, original).unwrap();
    let mut doc = StructuredDocument::open(&path).unwrap();
    let mut value: Value = doc.canvas().unwrap();
    value["description"] = json!("updated");
    doc.replace_canvas(&value).unwrap();
    let edited = doc.canvas().unwrap();
    assert_eq!(edited["vendor"]["deep"]["custom"], true);
    assert_eq!(edited["description"], "updated");
    assert!(!edited.as_object().unwrap().contains_key("edges"),
        "must not add a previously absent section");
    assert_eq!(fs::read(&path).unwrap(), original, "unsaved edits stay local");
}

#[test]
fn malformed_present_sections_still_fail_closed_without_touching_file() {
    let dir = tempfile::tempdir().unwrap();
    let invalid = [
        "[]",
        "null",
        r#"{"nodes":{}}"#,
        r#"{"nodes":null}"#,
        r#"{"edges":{}}"#,
        r#"{"edges":null}"#,
        r#"{"nodes":[],"edges":2}"#,
        "{",
    ];
    for (i, text) in invalid.iter().enumerate() {
        let path = dir.path().join(format!("invalid-{i}.canvas"));
        fs::write(&path, text).unwrap();
        assert!(StructuredDocument::open(&path).is_err(), "invalid input {i}: {text}");
        assert_eq!(fs::read_to_string(&path).unwrap(), *text);
    }
}
