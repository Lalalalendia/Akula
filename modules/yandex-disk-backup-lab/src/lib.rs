#![forbid(unsafe_code)]
//! OFFLINE, explicit one-object backup protocol for *personal* Yandex Disk.
//!
//! Production network/GUI/File Gate integration is not present. Each backup
//! encrypts an in-memory bounded payload under a 256-bit recovery key and
//! selects a random immutable name in app:/ (no folder setup). A create-only
//! transfer MUST request overwrite=false; 201/202 are NOT success receipts.
//! Only exact bounded remote readback is sufficient for this stage's receipt.
//! No listing, snapshot, deletion, local file write, or bidirectional Sync.
//! No key material is saved to AppConfig, a workspace, or the cloud.
#[cfg(feature = "network")] pub mod transport;
use akula_yandex_disk_rest::YandexAppPath;
use ring::{aead, digest, rand::{SecureRandom, SystemRandom}};
use zeroize::Zeroizing;

/// Single-object laboratory limit. Large vaults need chunking & an encrypted
/// manifest in a later independently reviewed protocol.
pub const MAX_PLAINTEXT_BYTES: usize = 8 * 1024 * 1024;
const MAGIC: &[u8] = b"AKULA-YD-BACKUP-01\0";
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
pub const MAX_ENCRYPTED_BYTES: usize = MAX_PLAINTEXT_BYTES + MAGIC.len() + NONCE_BYTES + TAG_BYTES;
const FILE_PREFIX: &str = "akula-backup-v1-";
const FILE_SUFFIX: &str = ".akulaenc";

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BackupError {
    EmptyPayload,
    PayloadTooLarge,
    InvalidPath,
    KeyGenerationFailed,
    EncryptionFailed,
    InvalidEnvelope,
    AuthenticationFailed,
    PreexistingRemoteObject,
    ProviderDenied,
    ProviderUnavailable,
    /// The remote PUT may have reached Yandex despite the lost reply.
    CommitUncertain,
    /// 202 or a pending PUT has no independently observable bytes yet.
    NotYetVerified,
    MissingRemoteObject,
    RemoteObjectTooLarge,
    RemoteContentMismatch,
}

/// A 32-byte independently generated recovery key. It is never included in
/// the archive, log, remote filename, or JSON. The user must preserve it in a
/// trusted recovery flow before any real upload can be enabled.
pub struct RecoveryKey(Zeroizing<[u8; 32]>);

impl RecoveryKey {
    pub fn generate() -> Result<Self, BackupError> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        SystemRandom::new().fill(&mut *bytes)
            .map_err(|_| BackupError::KeyGenerationFailed)?;
        Ok(Self(bytes))
    }

    /// Import an existing 256-bit recovery key. Do not derive this directly
    /// from a human password (no KDF has been defined or verified here).
    pub fn from_raw_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Explicit transfer to a recovery-key UI or trusted secret store. The
    /// caller owns zeroization of the returned array and must never log it.
    pub fn export_raw_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(*self.0)
    }
}

fn seal_key(key: &RecoveryKey) -> Result<aead::LessSafeKey, BackupError> {
    aead::UnboundKey::new(&aead::AES_256_GCM, &key.0[..])
        .map(aead::LessSafeKey::new)
        .map_err(|_| BackupError::EncryptionFailed)
}

fn unique_app_path() -> Result<YandexAppPath, BackupError> {
    let mut random = [0u8; 16];
    SystemRandom::new().fill(&mut random)
        .map_err(|_| BackupError::KeyGenerationFailed)?;
    let mut name = String::from(FILE_PREFIX);
    for b in random {
        use std::fmt::Write;
        write!(&mut name, "{b:02x}").map_err(|_| BackupError::InvalidPath)?;
    }
    name.push_str(FILE_SUFFIX);
    YandexAppPath::root().child(&name).map_err(|_| BackupError::InvalidPath)
}

fn valid_backup_path(path: &YandexAppPath) -> bool {
    let Some(file) = path.as_str().strip_prefix("app:/") else { return false };
    let Some(hex) = file.strip_prefix(FILE_PREFIX).and_then(|v| v.strip_suffix(FILE_SUFFIX))
        else { return false };
    hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Sealed object, never a Sync engine revision. Owned bytes are safe to pass
/// to a create-only remote transport, but transfer receipts remain untrusted
/// until a separate complete readback succeeds.
pub struct SealedBackup {
    path: YandexAppPath,
    encrypted: Vec<u8>,
    hash: [u8; 32],
}

impl SealedBackup {
    pub fn seal(key: &RecoveryKey, plaintext: &[u8]) -> Result<Self, BackupError> {
        if plaintext.is_empty() { return Err(BackupError::EmptyPayload); }
        if plaintext.len() > MAX_PLAINTEXT_BYTES { return Err(BackupError::PayloadTooLarge); }
        let path = unique_app_path()?;
        let mut nonce = [0u8; NONCE_BYTES];
        SystemRandom::new().fill(&mut nonce)
            .map_err(|_| BackupError::KeyGenerationFailed)?;
        let mut data = Zeroizing::new(plaintext.to_vec());
        seal_key(key)?.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(path.as_str().as_bytes()), &mut *data,
        ).map_err(|_| BackupError::EncryptionFailed)?;
        let mut encrypted = Vec::with_capacity(MAGIC.len() + NONCE_BYTES + data.len());
        encrypted.extend_from_slice(MAGIC);
        encrypted.extend_from_slice(&nonce);
        encrypted.extend_from_slice(data.as_slice());
        let mut hash = [0u8; 32];
        hash.copy_from_slice(digest::digest(&digest::SHA256, &encrypted).as_ref());
        Ok(Self { path, encrypted, hash })
    }

    pub fn path(&self) -> &YandexAppPath { &self.path }
    pub fn encrypted_bytes(&self) -> &[u8] { &self.encrypted }
    pub fn encrypted_len(&self) -> usize { self.encrypted.len() }

    /// Cryptographic restoration into memory only. The caller must apply an
    /// independently reviewed local File Gate before any actual restoration.
    pub fn open_from_remote(
        key: &RecoveryKey, path: &YandexAppPath, encrypted: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, BackupError> {
        if !valid_backup_path(path) { return Err(BackupError::InvalidPath); }
        if encrypted.len() > MAX_ENCRYPTED_BYTES {
            return Err(BackupError::RemoteObjectTooLarge);
        }
        let header = MAGIC.len() + NONCE_BYTES;
        if !encrypted.starts_with(MAGIC) || encrypted.len() < header + TAG_BYTES + 1 {
            return Err(BackupError::InvalidEnvelope);
        }
        let mut nonce = [0u8; NONCE_BYTES];
        nonce.copy_from_slice(&encrypted[MAGIC.len()..header]);
        let mut data = Zeroizing::new(encrypted[header..].to_vec());
        let len = seal_key(key)?.open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(path.as_str().as_bytes()), &mut data,
        ).map_err(|_| BackupError::AuthenticationFailed)?.len();
        if len > MAX_PLAINTEXT_BYTES || len == 0 {
            return Err(BackupError::InvalidEnvelope);
        }
        data.truncate(len);
        Ok(data)
    }

    /// Independent readback, never a simulated "201 means done" shortcut.
    /// No upload retry after a lost response. Bounded ciphertext digest is
    /// compared in constant time, not via Yandex md5 or an ETag hint.
    pub fn verify_existing(
        &self, provider: &mut dyn CreateOnlyBackupPort,
    ) -> Result<VerifiedBackupReceipt, BackupError> {
        let bytes = match provider.read_exact_bounded(&self.path, MAX_ENCRYPTED_BYTES)? {
            Some(bytes) => bytes,
            None => return Err(BackupError::NotYetVerified),
        };
        if bytes.len() > MAX_ENCRYPTED_BYTES {
            return Err(BackupError::RemoteObjectTooLarge);
        }
        if bytes.len() != self.encrypted.len() {
            return Err(BackupError::RemoteContentMismatch);
        }
        let observed = digest::digest(&digest::SHA256, &bytes);
        if observed.as_ref() != self.hash.as_slice() {
            return Err(BackupError::RemoteContentMismatch);
        }
        Ok(VerifiedBackupReceipt {
            path: self.path.clone(), encrypted_len: bytes.len() as u64,
            sha256: self.hash,
        })
    }

    /// One create-if-absent attempt only. A provider must never implement
    /// create_only using overwrite=true, deletes, or a compare-and-swap claim.
    /// 202 and uncertain outcomes require readback and may remain pending.
    pub fn create_once_and_verify(
        &self, provider: &mut dyn CreateOnlyBackupPort,
    ) -> Result<VerifiedBackupReceipt, BackupError> {
        match provider.create_only(&self.path, &self.encrypted) {
            Ok(UploadAcknowledgment::Created201 | UploadAcknowledgment::Pending202) => {}
            Err(BackupError::CommitUncertain) => {}
            Err(other) => return Err(other),
        }
        self.verify_existing(provider)
    }
}

/// Only a hint from Yandex's upload endpoint. Neither 201 nor 202 is a
/// verified remote content proof. A separate full byte readback is required.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UploadAcknowledgment { Created201, Pending202 }

/// The caller must expose an *already scoped* app-folder, no-write-to-existing
/// path adapter. This trait intentionally has no update/delete/list methods.
/// A future HTTP implementation must validate all transient href origins,
/// disable redirects, set overwrite=false, and bound readback.
pub trait CreateOnlyBackupPort {
    fn create_only(
        &mut self, path: &YandexAppPath, ciphertext: &[u8],
    ) -> Result<UploadAcknowledgment, BackupError>;
    fn read_exact_bounded(
        &mut self, path: &YandexAppPath, max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, BackupError>;
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct VerifiedBackupReceipt {
    pub path: YandexAppPath,
    pub encrypted_len: u64,
    /// SHA-256 of the *entire encrypted object*, independently read back.
    /// This is not a provider ETag, a durable sync baseline, or a remote CAS.
    pub sha256: [u8; 32],
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
