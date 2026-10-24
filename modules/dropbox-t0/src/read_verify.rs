//! Bounded, read-only verification of an explicitly pinned Dropbox revision.
//! A caller MUST obtain expected BLAKE3 from a trusted Akula manifest/anchor.
//! Dropbox metadata alone cannot establish trust, consent or publication.

use crate::{parse_metadata, DropboxFileRef, DropboxRevision, ObjectKey, Scope};
use sha2::{Digest, Sha256};

pub const MAX_READ_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_METADATA_BYTES: usize = 64 * 1024;
const DROPBOX_HASH_BLOCK: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    InvalidTrustedDigest,
    MissingDownloadPermission,
    WrongLogicalObject,
    WrongFileId,
    WrongRevision,
    WrongLength,
    TooLarge,
    InvalidResponseMetadata,
    DropboxHashMismatch,
    TrustedHashMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedBlake3(String);

impl TrustedBlake3 {
    pub fn parse(digest: &str) -> Result<Self, VerifyError> {
        if digest.len() != 64 || !digest.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(VerifyError::InvalidTrustedDigest);
        }
        Ok(Self(digest.to_owned()))
    }

    pub fn as_str(&self) -> &str { &self.0 }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedRead {
    pub logical_key: ObjectKey,
    pub revision: DropboxRevision,
    /// Exactly the bytes whose BLAKE3 matched the caller's trusted digest.
    /// This is not permission to apply, import, publish or overwrite anything.
    pub bytes: Vec<u8>,
}

pub fn dropbox_content_hash(bytes: &[u8]) -> String {
    let mut outer = Sha256::new();
    for part in bytes.chunks(DROPBOX_HASH_BLOCK) {
        outer.update(Sha256::digest(part));
    }
    outer.finalize().iter().map(|byte| format!("{byte:02x}")).collect::<String>()
}

/// Download-by-revision arguments. Not a pathname read and never a remote write.
pub fn pinned_download_arg(revision: &DropboxRevision) -> String {
    serde_json::json!({"path": format!("rev:{}", revision.as_str())}).to_string()
}

/// Cross-check response metadata with the exact previously inspected object.
/// All three identifiers (logical object, native file id, revision) must agree.
pub fn verify_download(
    scope: &Scope,
    logical_key: &ObjectKey,
    expected: &DropboxFileRef,
    returned_header: &[u8],
    bytes: Vec<u8>,
    expected_hash: &TrustedBlake3,
) -> Result<VerifiedRead, VerifyError> {
    if expected.logical_key != *logical_key {
        return Err(VerifyError::WrongLogicalObject);
    }
    if !expected.downloadable { return Err(VerifyError::MissingDownloadPermission); }
    if expected.byte_length > MAX_READ_BYTES as u64 || bytes.len() > MAX_READ_BYTES {
        return Err(VerifyError::TooLarge);
    }
    let returned = parse_metadata(scope, logical_key, returned_header)
        .map_err(|_| VerifyError::InvalidResponseMetadata)?;
    if returned.logical_key != *logical_key {
        return Err(VerifyError::WrongLogicalObject);
    }
    if returned.native_file_id != expected.native_file_id {
        return Err(VerifyError::WrongFileId);
    }
    if returned.revision != expected.revision {
        return Err(VerifyError::WrongRevision);
    }
    if returned.byte_length != expected.byte_length
        || bytes.len() as u64 != expected.byte_length
    {
        return Err(VerifyError::WrongLength);
    }
    if !returned.downloadable {
        return Err(VerifyError::MissingDownloadPermission);
    }
    if let (Some(a), Some(b)) = (&expected.dropbox_hash, &returned.dropbox_hash) {
        if a != b { return Err(VerifyError::DropboxHashMismatch); }
    }
    let hash = dropbox_content_hash(&bytes);
    for known in [&expected.dropbox_hash, &returned.dropbox_hash].into_iter().flatten() {
        if known.as_str() != hash {
            return Err(VerifyError::DropboxHashMismatch);
        }
    }
    if blake3::hash(&bytes).to_hex().as_str() != expected_hash.as_str() {
        return Err(VerifyError::TrustedHashMismatch);
    }
    Ok(VerifiedRead {
        logical_key: logical_key.clone(),
        revision: expected.revision.clone(),
        bytes,
    })
}

#[cfg(test)]
mod tests {
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
}
