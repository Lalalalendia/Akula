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
#[path = "read_verify/tests.rs"]
mod tests;
