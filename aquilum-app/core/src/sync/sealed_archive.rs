//! SYNC-04g-M1d-f1: a NON-ACTIVATED versioned AEAD envelope for future
//! encrypted local forensic copies. This is not cloud Sync encryption.
//!
//! SQLite archive writes remain in the old legacy format until OS key
//! custody and migration have separately passed security review. A SQL
//! overwrite cannot securely erase old pages, WAL, backups or SSD copies.
//! All keys must be supplied by the trusted native host, never by a remote
//! provider, plugin or document. Do not persist keys inside a vault or DB.

use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

use super::{OperationId, SyncRelativePath, VaultId};

const MAGIC: &[u8; 8] = b"AKARV1\0\0";
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const HEADER_BYTES: usize = MAGIC.len() + NONCE_BYTES;
const MAX_PLAINTEXT_BYTES: usize = 32 * 1024 * 1024;
const MAX_SEALED_BYTES: usize = HEADER_BYTES + MAX_PLAINTEXT_BYTES + TAG_BYTES;
const AAD_DOMAIN: &[u8] = b"akula:sync:recovery:archive:aes256gcm:v1\0";

/// The host is responsible for random 256-bit key generation, OS-keystore
/// persistence, recovery and authenticated delivery. NOT a password KDF.
/// Secret input is zeroized on drop. No plaintext key getter or Debug impl.
pub struct RecoveryArchiveKey {
    secret: Zeroizing<[u8; 32]>,
}

impl RecoveryArchiveKey {
    pub fn from_secret(secret: [u8; 32]) -> Self {
        Self { secret: Zeroizing::new(secret) }
    }

    fn cipher(&self) -> Result<LessSafeKey, SealedArchiveError> {
        let unbound = UnboundKey::new(&aead::AES_256_GCM, &self.secret[..])
            .map_err(|_| SealedArchiveError::CipherUnavailable)?;
        Ok(LessSafeKey::new(unbound))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryArchiveSlot {
    Staged,
    ExistingFinal,
}

impl RecoveryArchiveSlot {
    fn as_byte(self) -> u8 {
        match self {
            Self::Staged => 1,
            Self::ExistingFinal => 2,
        }
    }
}

/// The values must originate from the Core-registered vault and immutable
/// local publication intent, never from untrusted network-provided metadata.
#[derive(Clone, Copy)]
pub struct RecoveryArchiveBinding<'a> {
    pub vault_id: VaultId,
    pub operation_id: OperationId,
    pub historical_path: &'a SyncRelativePath,
    pub slot: RecoveryArchiveSlot,
}

fn authenticated_context(binding: &RecoveryArchiveBinding<'_>) -> Vec<u8> {
    let path = binding.historical_path.as_str().as_bytes();
    let mut aad = Vec::with_capacity(AAD_DOMAIN.len() + 16 + 16 + 8 + path.len() + 1);
    aad.extend_from_slice(AAD_DOMAIN);
    aad.extend_from_slice(binding.vault_id.as_uuid().as_bytes());
    aad.extend_from_slice(binding.operation_id.as_uuid().as_bytes());
    aad.extend_from_slice(&(path.len() as u64).to_le_bytes());
    aad.extend_from_slice(path);
    aad.push(binding.slot.as_byte());
    aad
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SealedArchiveError {
    InvalidEnvelope,
    ExceedsSafetyLimit,
    RandomnessUnavailable,
    CipherUnavailable,
    AuthenticationFailed,
}

/// Format: 8-byte AKARV1 marker, random 96-bit nonce, AES-256-GCM body
/// with 16-byte authentication tag. Empty file content is valid.
/// Fresh randomness is requested on EVERY call, never replayed from a DB.
pub fn seal_recovery_archive(
    key: &RecoveryArchiveKey,
    binding: &RecoveryArchiveBinding<'_>,
    plaintext: &[u8],
) -> Result<Vec<u8>, SealedArchiveError> {
    if plaintext.len() > MAX_PLAINTEXT_BYTES {
        return Err(SealedArchiveError::ExceedsSafetyLimit);
    }
    let mut nonce_bytes = [0u8; NONCE_BYTES];
    SystemRandom::new().fill(&mut nonce_bytes)
        .map_err(|_| SealedArchiveError::RandomnessUnavailable)?;
    // If sealing fails, the transient plaintext buffer is zeroized.
    let mut body = Zeroizing::new(plaintext.to_vec());
    let aad = authenticated_context(binding);
    key.cipher()?.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce_bytes),
        Aad::from(aad.as_slice()),
        &mut *body,
    ).map_err(|_| SealedArchiveError::CipherUnavailable)?;
    let mut envelope = Vec::with_capacity(HEADER_BYTES + body.len());
    envelope.extend_from_slice(MAGIC);
    envelope.extend_from_slice(&nonce_bytes);
    envelope.extend_from_slice(&body);
    Ok(envelope)
}

/// Strict: no plaintext fallback or unknown version. A wrong key, nonce,
/// ciphertext, tag, vault, operation, historical path or slot must fail
/// before any file content is returned to the host.
pub fn open_recovery_archive(
    key: &RecoveryArchiveKey,
    binding: &RecoveryArchiveBinding<'_>,
    sealed: &[u8],
) -> Result<Vec<u8>, SealedArchiveError> {
    if sealed.len() > MAX_SEALED_BYTES {
        return Err(SealedArchiveError::ExceedsSafetyLimit);
    }
    if sealed.len() < HEADER_BYTES + TAG_BYTES || !sealed.starts_with(MAGIC) {
        return Err(SealedArchiveError::InvalidEnvelope);
    }
    let mut nonce_bytes = [0u8; NONCE_BYTES];
    nonce_bytes.copy_from_slice(&sealed[MAGIC.len()..HEADER_BYTES]);
    // Authentication failure may leave partially decrypted bytes in memory:
    // scrub the entire transient buffer on every failure path.
    let mut body = Zeroizing::new(sealed[HEADER_BYTES..].to_vec());
    let aad = authenticated_context(binding);
    let plaintext_len = key.cipher()?.open_in_place(
        Nonce::assume_unique_for_key(nonce_bytes),
        Aad::from(aad.as_slice()),
        &mut body,
    ).map_err(|_| SealedArchiveError::AuthenticationFailed)?.len();
    body.truncate(plaintext_len);
    // Ownership transfers to the trusted caller on successful authentication.
    // The caller MUST scrub this result when finished with the secret bytes.
    Ok(std::mem::take(&mut *body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (RecoveryArchiveKey, VaultId, OperationId, SyncRelativePath) {
        (
            RecoveryArchiveKey::from_secret([7; 32]),
            VaultId::new(),
            OperationId::new(),
            SyncRelativePath::parse("notes/конфликт.md").unwrap(),
        )
    }

    #[test]
    fn m1df1_empty_unicode_binary_roundtrip_and_nonce_uniqueness() {
        let (key, vault, op, path) = fixture();
        let binding = RecoveryArchiveBinding {
            vault_id: vault, operation_id: op,
            historical_path: &path, slot: RecoveryArchiveSlot::Staged,
        };
        for bytes in [b"".as_slice(), b"private document content",
            &[0, 255, 1, 42][..], "Привет мир 世界".as_bytes()] {
            let one = seal_recovery_archive(&key, &binding, bytes).unwrap();
            let two = seal_recovery_archive(&key, &binding, bytes).unwrap();
            assert_eq!(&one[..MAGIC.len()], MAGIC);
            assert_ne!(one, two, "fresh nonce expected");
            if bytes.len() > 16 {
                assert!(!one.windows(bytes.len()).any(|v| v == bytes));
            }
            assert_eq!(open_recovery_archive(&key, &binding, &one).unwrap(), bytes);
            assert_eq!(open_recovery_archive(&key, &binding, &two).unwrap(), bytes);
            assert_eq!(one.len(), HEADER_BYTES + bytes.len() + TAG_BYTES);
        }
    }

    #[test]
    fn m1df1_wrong_key_nonce_ciphertext_tag_fail_closed() {
        let (key, vault, op, path) = fixture();
        let binding = RecoveryArchiveBinding {
            vault_id: vault, operation_id: op,
            historical_path: &path, slot: RecoveryArchiveSlot::ExistingFinal,
        };
        let mut data = seal_recovery_archive(&key, &binding, b"secret content").unwrap();
        assert_eq!(open_recovery_archive(
            &RecoveryArchiveKey::from_secret([9; 32]), &binding, &data,
        ), Err(SealedArchiveError::AuthenticationFailed));
        for offset in [MAGIC.len(), HEADER_BYTES, data.len() - 1] {
            data[offset] ^= 1;
            assert_eq!(open_recovery_archive(&key, &binding, &data),
                Err(SealedArchiveError::AuthenticationFailed));
            data[offset] ^= 1;
        }
    }

    #[test]
    fn m1df1_cross_vault_operation_path_slot_substitution_denied() {
        let (key, vault, op, path) = fixture();
        let binding = RecoveryArchiveBinding {
            vault_id: vault, operation_id: op,
            historical_path: &path, slot: RecoveryArchiveSlot::Staged,
        };
        let sealed = seal_recovery_archive(&key, &binding, b"conflict").unwrap();
        let alternate = SyncRelativePath::parse("notes/other.md").unwrap();
        for switched in [
            RecoveryArchiveBinding { vault_id: VaultId::new(), ..binding },
            RecoveryArchiveBinding { operation_id: OperationId::new(), ..binding },
            RecoveryArchiveBinding { historical_path: &alternate, ..binding },
            RecoveryArchiveBinding { slot: RecoveryArchiveSlot::ExistingFinal, ..binding },
        ] {
            assert_eq!(open_recovery_archive(&key, &switched, &sealed),
                Err(SealedArchiveError::AuthenticationFailed));
        }
        assert_eq!(open_recovery_archive(&key, &binding, &sealed).unwrap(), b"conflict");
    }

    #[test]
    fn m1df1_unknown_version_plaintext_truncation_and_oversize_denied() {
        let (key, vault, op, path) = fixture();
        let binding = RecoveryArchiveBinding {
            vault_id: vault, operation_id: op,
            historical_path: &path, slot: RecoveryArchiveSlot::Staged,
        };
        let sealed = seal_recovery_archive(&key, &binding, b"secret").unwrap();
        let mut bad = sealed.clone();
        bad[0] = b'X';
        assert_eq!(open_recovery_archive(&key, &binding, &bad),
            Err(SealedArchiveError::InvalidEnvelope));
        assert_eq!(open_recovery_archive(&key, &binding, b"plaintext"),
            Err(SealedArchiveError::InvalidEnvelope));
        for n in [0, MAGIC.len(), HEADER_BYTES, HEADER_BYTES + TAG_BYTES - 1] {
            assert_eq!(open_recovery_archive(&key, &binding, &sealed[..n]),
                Err(SealedArchiveError::InvalidEnvelope));
        }
        let too_long = vec![0u8; MAX_PLAINTEXT_BYTES + 1];
        assert_eq!(seal_recovery_archive(&key, &binding, &too_long),
            Err(SealedArchiveError::ExceedsSafetyLimit));
        let mut oversized = vec![0; MAX_SEALED_BYTES + 1];
        oversized[..MAGIC.len()].copy_from_slice(MAGIC);
        assert_eq!(open_recovery_archive(&key, &binding, &oversized),
            Err(SealedArchiveError::ExceedsSafetyLimit));
    }
}
