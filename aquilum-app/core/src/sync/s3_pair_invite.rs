//! Portable S3 pairing invitation: authenticated password-encrypted transport.
//!
//! This is NOT a remote trust oracle or permission to apply S3 data to a vault.
//! A receiving host must check the pinned cloud genesis against authenticated
//! provider data before binding it to a separate locally registered workspace.
//! PBKDF2-HMAC-SHA256 is deliberately pinned in wire V1 because it ships in the
//! already-locked ring dependency; a future Argon2id format needs a NEW version.

use std::num::NonZeroU32;

use ring::{
    aead::{self, Aad, LessSafeKey, Nonce, UnboundKey},
    pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::{S3ConnectionProfile, S3Keyspace, VaultId};

const MAGIC: &[u8; 8] = b"AKS3V1\0\0";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
const KDF_ITERATIONS: u32 = 600_000;
const HEADER_LEN: usize = MAGIC.len() + 4 + SALT_LEN + NONCE_LEN;
pub const MAX_PAIRING_FILE_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = MAX_PAIRING_FILE_BYTES - HEADER_LEN - TAG_LEN;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PairInviteError {
    InvalidInvitation,
    UnsupportedVersion,
    WeakPassword,
    InvalidScope,
    TooLarge,
    RandomUnavailable,
    CipherUnavailable,
    AuthenticationFailed,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct S3PairInvite {
    pub provider: String,
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub cloud_vault_id: VaultId,
    /// Hex BLAKE3 of the independently user-authorized genesis root bytes.
    pub genesis_fingerprint: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// 0 = permanent scoped key. Other values require a future issuer/revoker.
    pub key_expires_at_ms: u64,
}

impl Drop for S3PairInvite {
    fn drop(&mut self) {
        self.access_key_id.zeroize();
        self.secret_access_key.zeroize();
    }
}

impl S3PairInvite {
    /// Validate the same scope that will be bound to a new local workspace.
    /// Never accept a remote-supplied local filesystem path or DB file.
    pub fn validate(&self) -> Result<(), PairInviteError> {
        let kind = self.provider.as_str();
        if !matches!(kind, "yandex" | "aws" | "minio" | "custom") {
            return Err(PairInviteError::InvalidScope);
        }
        let profile = S3ConnectionProfile {
            bucket: self.bucket.clone(),
            region: self.region.clone(),
            endpoint: self.endpoint.clone(),
            force_path_style: kind != "aws",
        };
        profile.validate().map_err(|_| PairInviteError::InvalidScope)?;
        S3Keyspace::new(&self.prefix, self.cloud_vault_id)
            .map_err(|_| PairInviteError::InvalidScope)?;
        if kind == "yandex" && (self.endpoint.as_deref() != Some("https://storage.yandexcloud.net")
            || self.region != "ru-central1")
            || kind == "aws" && self.endpoint.is_some()
            || self.genesis_fingerprint.len() != 64
            || !self.genesis_fingerprint.bytes().all(|b| b.is_ascii_digit()
                || (b'a'..=b'f').contains(&b))
            || !(3..=128).contains(&self.access_key_id.len())
            || !self.access_key_id.bytes().all(|b| b.is_ascii_alphanumeric()
                || b == b'-' || b == b'_')
            || self.key_expires_at_ms != 0
            || !(8..=255).contains(&self.secret_access_key.len())
            || !self.secret_access_key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(PairInviteError::InvalidScope);
        }
        Ok(())
    }
}

fn password_valid(password: &str) -> bool {
    (20..=1024).contains(&password.len())
        && !password.bytes().any(|b| b == 0)
}

fn derive_key(password: &str, salt: &[u8]) -> Zeroizing<[u8; KEY_LEN]> {
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(KDF_ITERATIONS).expect("nonzero constant"),
        salt,
        password.as_bytes(),
        &mut *key,
    );
    key
}

fn cipher(key: &[u8; KEY_LEN]) -> Result<LessSafeKey, PairInviteError> {
    UnboundKey::new(&aead::AES_256_GCM, key)
        .map(LessSafeKey::new)
        .map_err(|_| PairInviteError::CipherUnavailable)
}

/// The password is supplied by a masked OS-native host dialog. The caller
/// must zeroize that UI buffer after use. No password or credential is logged.
pub fn seal_s3_pair_invite(
    invite: &S3PairInvite,
    password: &str,
) -> Result<Vec<u8>, PairInviteError> {
    invite.validate()?;
    if !password_valid(password) {
        return Err(PairInviteError::WeakPassword);
    }
    let mut body = Zeroizing::new(
        serde_json::to_vec(invite).map_err(|_| PairInviteError::InvalidInvitation)?
    );
    if body.len() > MAX_BODY_BYTES {
        return Err(PairInviteError::TooLarge);
    }
    let rng = SystemRandom::new();
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    rng.fill(&mut salt).map_err(|_| PairInviteError::RandomUnavailable)?;
    rng.fill(&mut nonce).map_err(|_| PairInviteError::RandomUnavailable)?;
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&KDF_ITERATIONS.to_le_bytes());
    header.extend_from_slice(&salt);
    header.extend_from_slice(&nonce);
    let key = derive_key(password, &salt);
    cipher(&key)?.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(header.as_slice()),
        &mut *body,
    ).map_err(|_| PairInviteError::CipherUnavailable)?;
    header.extend_from_slice(&body);
    Ok(header)
}

/// Strict parse: the envelope version and bounded KDF params are verified
/// BEFORE expensive work. Authentication happens BEFORE JSON parsing.
pub fn open_s3_pair_invite(
    encrypted: &[u8],
    password: &str,
) -> Result<S3PairInvite, PairInviteError> {
    if encrypted.len() > MAX_PAIRING_FILE_BYTES {
        return Err(PairInviteError::TooLarge);
    }
    if encrypted.len() < HEADER_LEN + TAG_LEN {
        return Err(PairInviteError::InvalidInvitation);
    }
    if !encrypted.starts_with(MAGIC) {
        return Err(PairInviteError::UnsupportedVersion);
    }
    if !password_valid(password) {
        return Err(PairInviteError::WeakPassword);
    }
    let iterations: [u8; 4] = encrypted[MAGIC.len()..MAGIC.len() + 4]
        .try_into().map_err(|_| PairInviteError::InvalidInvitation)?;
    if u32::from_le_bytes(iterations) != KDF_ITERATIONS {
        return Err(PairInviteError::UnsupportedVersion);
    }
    let salt_start = MAGIC.len() + 4;
    let salt_end = salt_start + SALT_LEN;
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&encrypted[salt_end..HEADER_LEN]);
    let mut body = Zeroizing::new(encrypted[HEADER_LEN..].to_vec());
    let key = derive_key(password, &encrypted[salt_start..salt_end]);
    let length = cipher(&key)?.open_in_place(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(&encrypted[..HEADER_LEN]),
        &mut *body,
    ).map_err(|_| PairInviteError::AuthenticationFailed)?.len();
    body.truncate(length);
    let result: S3PairInvite = serde_json::from_slice(&body)
        .map_err(|_| PairInviteError::InvalidInvitation)?;
    result.validate()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> S3PairInvite {
        S3PairInvite {
            provider: "yandex".into(),
            endpoint: Some("https://storage.yandexcloud.net".into()),
            region: "ru-central1".into(),
            bucket: "akula-private".into(),
            prefix: "akula-sync/pair".into(),
            cloud_vault_id: VaultId::new(),
            genesis_fingerprint: "a".repeat(64),
            access_key_id: "akula-test-id".into(),
            secret_access_key: "not-a-real-secret-123456789".into(),
            key_expires_at_ms: 0,
        }
    }

    #[test]
    fn encrypted_roundtrip_randomized_and_no_plaintext_keys() {
        let source = fixture();
        let pass = "two devices protected by a long password";
        let first = seal_s3_pair_invite(&source, pass).unwrap();
        let second = seal_s3_pair_invite(&source, pass).unwrap();
        assert_ne!(first, second);
        for bytes in [&first, &second] {
            assert!(!bytes.windows(source.secret_access_key.len())
                .any(|w| w == source.secret_access_key.as_bytes()));
            assert!(!bytes.windows(source.access_key_id.len())
                .any(|w| w == source.access_key_id.as_bytes()));
            let imported = open_s3_pair_invite(bytes, pass).unwrap();
            assert_eq!(imported.bucket, source.bucket);
            assert_eq!(imported.cloud_vault_id, source.cloud_vault_id);
            assert_eq!(imported.secret_access_key, source.secret_access_key);
        }
    }

    #[test]
    fn wrong_password_tamper_and_header_tamper_are_rejected() {
        let source = fixture();
        let pass = "two devices protected by a long password";
        let mut bytes = seal_s3_pair_invite(&source, pass).unwrap();
        assert!(matches!(open_s3_pair_invite(&bytes, "wrong password 123456"),
            Err(PairInviteError::AuthenticationFailed)));
        let index = bytes.len() - 2;
        bytes[index] ^= 1;
        assert!(matches!(open_s3_pair_invite(&bytes, pass),
            Err(PairInviteError::AuthenticationFailed)));
        bytes[index] ^= 1;
        bytes[HEADER_LEN - 1] ^= 1;
        assert!(matches!(open_s3_pair_invite(&bytes, pass),
            Err(PairInviteError::AuthenticationFailed)));
    }

    #[test]
    fn rejects_incomplete_oversize_wrong_version_and_kdf_work_factors() {
        let source = fixture();
        let pass = "two devices protected by a long password";
        let bytes = seal_s3_pair_invite(&source, pass).unwrap();
        assert!(matches!(open_s3_pair_invite(&bytes[..10], pass),
            Err(PairInviteError::InvalidInvitation)));
        let mut oversized = bytes.clone();
        oversized.resize(MAX_PAIRING_FILE_BYTES + 1, 0);
        assert!(matches!(open_s3_pair_invite(&oversized, pass),
            Err(PairInviteError::TooLarge)));
        let mut version = bytes.clone();
        version[0] ^= 1;
        assert!(matches!(open_s3_pair_invite(&version, pass),
            Err(PairInviteError::UnsupportedVersion)));
        let mut iterations = bytes;
        iterations[MAGIC.len()] ^= 1;
        assert!(matches!(open_s3_pair_invite(&iterations, pass),
            Err(PairInviteError::UnsupportedVersion)));
    }

    #[test]
    fn refuses_unsafe_scopes_and_short_passwords() {
        let mut source = fixture();
        assert!(matches!(seal_s3_pair_invite(&source, "123"),
            Err(PairInviteError::WeakPassword)));
        source.bucket = "invalid bucket".into();
        assert!(matches!(source.validate(), Err(PairInviteError::InvalidScope)));
        source.bucket = "akula-private".into();
        source.prefix = "other/../vault".into();
        assert!(matches!(source.validate(), Err(PairInviteError::InvalidScope)));
        source.prefix = "akula-sync/pair".into();
        source.genesis_fingerprint = "not a digest".into();
        assert!(matches!(source.validate(), Err(PairInviteError::InvalidScope)));
    }
}
