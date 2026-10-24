//! B1-E2E-07: separate Windows owner Ed25519 identity, in OS credentials.
//!
//! NEVER reuse the P2P device key or the Node release-signing key.
//! There is no implicit key creation on load, fallback to plaintext, or
//! automatic install. Server root must independently pin this public key.
//! A valid local owner signature is NOT a root CAS receipt/activation grant.
#![cfg(all(target_os = "windows", feature = "server-owner-windows-signer"))]

use crate::server_owner_approval_wire::{OwnerApprovalWireV1, WireError};
use crate::server_release_lab::review::InertUserConsent;
use ring::{
    rand::{SecureRandom, SystemRandom},
    signature::{Ed25519KeyPair, KeyPair},
};
use std::ptr;
use windows::{
    core::{w, PCWSTR, PWSTR},
    Win32::{
        Foundation::{CloseHandle, ERROR_NOT_FOUND, HANDLE},
        Security::Credentials::{
            CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
            CRED_TYPE_GENERIC,
        },
        System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    },
};
use zeroize::Zeroizing;

const TARGET: &str = "Lalalalendia/Akula/ServerOwner/Ed25519/v1";
const MAGIC: &[u8; 8] = b"AKOWNR01";
const RECORD_LEN: usize = 8 + 32 + 32;
const OWNER_ID: &str = "akula-owner-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerIdentityError {
    NotInitialized,
    AlreadyInitialized,
    CredentialStore,
    CredentialCorrupt,
    SeedUnavailable,
    Busy,
    InvalidScope,
    InvalidClaim(WireError),
}

pub struct OwnerIdentity {
    seed: Zeroizing<[u8; 32]>,
    public: [u8; 32],
}

impl OwnerIdentity {
    fn from_seed(seed: Zeroizing<[u8; 32]>) -> Result<Self, OwnerIdentityError> {
        let key = Ed25519KeyPair::from_seed_unchecked(&seed[..])
            .map_err(|_| OwnerIdentityError::CredentialCorrupt)?;
        let mut public = [0u8; 32];
        public.copy_from_slice(key.public_key().as_ref());
        Ok(Self { seed, public })
    }

    pub fn public_key_ed25519(&self) -> [u8; 32] { self.public }

    #[cfg(test)]
    pub(crate) fn from_test_seed(seed: [u8; 32]) -> Self {
        Self::from_seed(Zeroizing::new(seed)).expect("test seed")
    }
    pub fn signer_id(&self) -> &'static str { OWNER_ID }

    /// Sign only the exact once-reviewed host/release/CAS scope. Root still
    /// independently verifies the actual owner key, floor, stage and nonce.
    pub fn sign_consented_claim(
        &self, reviewed: &InertUserConsent,
        claim: &OwnerApprovalWireV1, now_unix: u64,
    ) -> Result<[u8; 64], OwnerIdentityError> {
        claim.validate_time_and_shape(now_unix)
            .map_err(OwnerIdentityError::InvalidClaim)?;
        if claim.approver_id != OWNER_ID
            || !reviewed.matches_owner_claim(claim)
        {
            return Err(OwnerIdentityError::InvalidScope);
        }
        let signer = Ed25519KeyPair::from_seed_unchecked(&self.seed[..])
            .map_err(|_| OwnerIdentityError::CredentialCorrupt)?;
        signer.sign(&claim.canonical_bytes()).as_ref()
            .try_into().map_err(|_| OwnerIdentityError::CredentialCorrupt)
    }
}

fn record(identity: &OwnerIdentity) -> Zeroizing<[u8; RECORD_LEN]> {
    let mut bytes = Zeroizing::new([0u8; RECORD_LEN]);
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..40].copy_from_slice(&identity.public);
    bytes[40..72].copy_from_slice(&identity.seed[..]);
    bytes
}

fn parse(bytes: &[u8]) -> Result<OwnerIdentity, OwnerIdentityError> {
    if bytes.len() != RECORD_LEN || &bytes[..8] != MAGIC {
        return Err(OwnerIdentityError::CredentialCorrupt);
    }
    let mut secret = Zeroizing::new([0u8; 32]);
    secret.copy_from_slice(&bytes[40..72]);
    let identity = OwnerIdentity::from_seed(secret)?;
    if identity.public != bytes[8..40] {
        return Err(OwnerIdentityError::CredentialCorrupt);
    }
    Ok(identity)
}

struct CredentialBuffer(*mut CREDENTIALW);
impl Drop for CredentialBuffer {
    fn drop(&mut self) {
        if self.0.is_null() { return; }
        // Scrub only the owned bounded blob; no secret content in diagnostics.
        unsafe {
            let credential = &mut *self.0;
            if !credential.CredentialBlob.is_null()
                && credential.CredentialBlobSize as usize <= 2560
            {
                ptr::write_bytes(
                    credential.CredentialBlob, 0,
                    credential.CredentialBlobSize as usize,
                );
            }
            CredFree(self.0.cast());
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn read_named(target: &str) -> Result<OwnerIdentity, OwnerIdentityError> {
    let name = wide(target);
    let mut raw: *mut CREDENTIALW = ptr::null_mut();
    match unsafe {
        CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut raw)
    } {
        Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() =>
            return Err(OwnerIdentityError::NotInitialized),
        Err(_) => return Err(OwnerIdentityError::CredentialStore),
        Ok(()) => {}
    }
    if raw.is_null() { return Err(OwnerIdentityError::CredentialStore); }
    let owned = CredentialBuffer(raw);
    let entry = unsafe { &*owned.0 };
    if entry.Type != CRED_TYPE_GENERIC
        || entry.CredentialBlobSize as usize != RECORD_LEN
        || entry.CredentialBlob.is_null()
    {
        return Err(OwnerIdentityError::CredentialCorrupt);
    }
    let mut bytes = Zeroizing::new([0u8; RECORD_LEN]);
    let raw = unsafe {
        std::slice::from_raw_parts(entry.CredentialBlob, RECORD_LEN)
    };
    bytes.copy_from_slice(raw);
    parse(&bytes[..])
}

fn write_named(target: &str, identity: &OwnerIdentity)
    -> Result<(), OwnerIdentityError>
{
    let mut value = record(identity);
    let mut name = wide(target);
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(name.as_mut_ptr()),
        CredentialBlob: value.as_mut_ptr(),
        CredentialBlobSize: RECORD_LEN as u32,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..CREDENTIALW::default()
    };
    unsafe { CredWriteW(&credential, 0) }
        .map_err(|_| OwnerIdentityError::CredentialStore)
}

struct EnrollmentLock(HANDLE);
impl EnrollmentLock {
    fn acquire() -> Result<Self, OwnerIdentityError> {
        let handle = unsafe {
            CreateMutexW(None, false, w!("Local\\AkulaServerOwnerEnrollV1"))
        }.map_err(|_| OwnerIdentityError::CredentialStore)?;
        let outcome = unsafe { WaitForSingleObject(handle, 30_000) };
        if outcome.0 != 0 && outcome.0 != 0x80 {
            let _ = unsafe { CloseHandle(handle) };
            return Err(OwnerIdentityError::Busy);
        }
        Ok(Self(handle))
    }
}
impl Drop for EnrollmentLock {
    fn drop(&mut self) {
        let _ = unsafe { ReleaseMutex(self.0) };
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// No silent identity regeneration; an unknown old owner key cannot be
/// replaced without a separate root-owner recovery/revocation ceremony.
pub fn load() -> Result<OwnerIdentity, OwnerIdentityError> {
    read_named(TARGET)
}

/// Only call from a user-confirmed owner enrollment action. The ordinary
/// installed build has no entrypoint to this experimental feature.
pub fn initialize() -> Result<OwnerIdentity, OwnerIdentityError> {
    let _lock = EnrollmentLock::acquire()?;
    match read_named(TARGET) {
        Err(OwnerIdentityError::NotInitialized) => {}
        Ok(_) => return Err(OwnerIdentityError::AlreadyInitialized),
        Err(error) => return Err(error),
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    SystemRandom::new().fill(&mut seed[..])
        .map_err(|_| OwnerIdentityError::SeedUnavailable)?;
    let identity = OwnerIdentity::from_seed(seed)?;
    write_named(TARGET, &identity)?;
    let loaded = read_named(TARGET)?;
    if identity.public != loaded.public {
        return Err(OwnerIdentityError::CredentialCorrupt);
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_owner_seed_is_separate_and_corrupted_records_fail_closed() {
        let identity = OwnerIdentity::from_seed(Zeroizing::new([0x99; 32])).unwrap();
        let bytes = record(&identity);
        assert_eq!(parse(&bytes[..]).unwrap().public_key_ed25519(),
            identity.public_key_ed25519());
        let mut changed = bytes.clone();
        changed[8] ^= 1;
        assert!(matches!(parse(&changed[..]),
            Err(OwnerIdentityError::CredentialCorrupt)));
        assert!(matches!(parse(&bytes[..71]),
            Err(OwnerIdentityError::CredentialCorrupt)));
        assert_eq!(identity.signer_id(), "akula-owner-v1");
    }
}
