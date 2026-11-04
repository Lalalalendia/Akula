use super::*;
use crate::ObjectKey;

fn scope() -> Scope {
    Scope::new("dbid:alice", "12345",
        "12345678-1234-1234-1234-123456789abc", "home").unwrap()
}

fn key() -> ObjectKey { ObjectKey::head() }

fn metadata(size: usize, hash: Option<&str>, rev: &str, id: &str) -> Vec<u8> {
    serde_json::json!({
        ".tag": "file", "id": id, "rev": rev, "size": size,
        "content_hash": hash, "is_downloadable": true
    }).to_string().into_bytes()
}

#[test]
fn stable_empty_and_chunk_boundaries() {
    assert_eq!(dropbox_content_hash(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(dropbox_content_hash(b"hello akula"),
        "dce2aacbd1d6797e219a93e0f484eef7ab0b6b8779f0eccd8cff61f64dafa98c");
    let block = vec![0x41; DROPBOX_HASH_BLOCK];
    assert_ne!(dropbox_content_hash(&block), dropbox_content_hash(&block[..block.len()-1]));
    let mut two = block.clone();
    two.push(0x42);
    assert_ne!(dropbox_content_hash(&two), dropbox_content_hash(&block));
    assert_eq!(dropbox_content_hash(&two).len(), 64);
}

#[test]
fn correct_pinned_readback_is_not_an_apply_receipt() {
    let bytes = b"hello akula".to_vec();
    let h = dropbox_content_hash(&bytes);
    let response = metadata(bytes.len(), Some(&h), "a1c10ce0dd78", "id:123");
    let original = parse_metadata(&scope(), &key(), &response).unwrap();
    let trusted = TrustedBlake3::parse(
        &blake3::hash(&bytes).to_hex().to_string()).unwrap();
    let result = verify_download(&scope(), &key(), &original, &response, bytes.clone(),
        &trusted).unwrap();
    assert_eq!(result.bytes, bytes);
    assert_eq!(result.revision.as_str(), "a1c10ce0dd78");
    assert_eq!(pinned_download_arg(&result.revision),
        r#"{"path":"rev:a1c10ce0dd78"}"#);
}

#[test]
fn wrong_length_same_length_bad_bytes_and_wrong_revision_fail() {
    let bytes = b"1234".to_vec();
    let original = parse_metadata(&scope(), &key(),
        &metadata(4, None, "a1c10ce0dd78", "id:123")).unwrap();
    let trusted = TrustedBlake3::parse(
        &blake3::hash(&bytes).to_hex().to_string()).unwrap();
    assert_eq!(verify_download(&scope(), &key(), &original,
        &metadata(4, None, "a1c10ce0dd78", "id:123"), b"1235".to_vec(),
        &trusted), Err(VerifyError::TrustedHashMismatch));
    assert_eq!(verify_download(&scope(), &key(), &original,
        &metadata(4, None, "a1c10ce0dd78", "id:123"), b"12".to_vec(),
        &trusted), Err(VerifyError::WrongLength));
    assert_eq!(verify_download(&scope(), &key(), &original,
        &metadata(4, None, "a1c10ce0dd79", "id:123"), bytes.clone(),
        &trusted), Err(VerifyError::WrongRevision));
    assert_eq!(verify_download(&scope(), &key(), &original,
        &metadata(4, None, "a1c10ce0dd78", "id:456"), bytes,
        &trusted), Err(VerifyError::WrongFileId));
}

#[test]
fn wrong_dropbox_hash_and_untrusted_digest_fail() {
    let bytes = b"data".to_vec();
    let response = metadata(4, Some(&"0".repeat(64)), "a1c10ce0dd78", "id:123");
    let original = parse_metadata(&scope(), &key(), &response).unwrap();
    let trusted = TrustedBlake3::parse(
        &blake3::hash(&bytes).to_hex().to_string()).unwrap();
    assert_eq!(verify_download(&scope(), &key(), &original, &response, bytes,
        &trusted), Err(VerifyError::DropboxHashMismatch));
    assert_eq!(TrustedBlake3::parse("not-a-trusted-digest"),
        Err(VerifyError::InvalidTrustedDigest));
}

#[test]
fn large_objects_refused_in_this_bounded_reader() {
    let oversized = MAX_READ_BYTES + 1;
    let response = metadata(oversized, None, "a1c10ce0dd78", "id:123");
    let original = parse_metadata(&scope(), &key(), &response).unwrap();
    let trusted = TrustedBlake3::parse(&"a".repeat(64)).unwrap();
    assert_eq!(verify_download(&scope(), &key(), &original,
        &response, Vec::new(), &trusted), Err(VerifyError::TooLarge));
}
