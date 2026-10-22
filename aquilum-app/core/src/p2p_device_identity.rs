//! AK-P2P-DEVICE-IDENTITY-01: persistent Iroh Ed25519 seed in OS credentials.
//!
//! Explicit enrollment only. This module neither connects to a peer nor issues
//! Vault, Staya GET or File Gate permissions. The ordinary build excludes it.
//! Windows Credential Manager is the initial supported persistent backend;
//! other operating systems fail closed until a native protected store is added.
#![deny(clippy::all)]

use ring::{
    rand::{SecureRandom, SystemRandom},
    signature::{Ed25519KeyPair, KeyPair},
};
use zeroize::Zeroizing;

const TARGET: &str = "Lalalalendia/Akula/P2P/IrohEndpoint/v1";
const MAGIC: &[u8; 8] = b"AKP2PID1";
const RECORD_SIZE: usize = 72;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    UnsupportedPlatform,
    NotInitialized,
    AlreadyInitialized,
    StoreUnavailable,
    CorruptCredential,
    EntropyUnavailable,
    EnrollmentChanged,
    EnrollmentBusy,
    InvalidPeerId,
    PeerMismatch,
    PeerNotFound,
    PeerRevoked,
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::UnsupportedPlatform => "no supported protected credential store",
            Self::NotInitialized => "device identity has not been initialized",
            Self::AlreadyInitialized => "device identity already exists; refusing overwrite",
            Self::StoreUnavailable => "protected credential store is unavailable",
            Self::CorruptCredential => "protected device identity is malformed",
            Self::EntropyUnavailable => "secure randomness is unavailable",
            Self::EnrollmentChanged => "device identity changed during enrollment",
            Self::EnrollmentBusy => "another enrollment operation holds the device lock",
            Self::InvalidPeerId => "invalid full Iroh EndpointId",
            Self::PeerMismatch => "verified peer identity does not match selected peer",
            Self::PeerNotFound => "peer is not confirmed",
            Self::PeerRevoked => "peer has been revoked",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for IdentityError {}

/// Private key is never printable or serializable by this API. The returned
/// public key is exactly the Ed25519 key used as the Iroh 1.3 EndpointId.
pub struct DeviceIdentity {
    seed: Zeroizing<[u8; 32]>,
    endpoint_public: [u8; 32],
}

impl DeviceIdentity {
    fn from_seed(seed: Zeroizing<[u8; 32]>) -> Result<Self, IdentityError> {
        let key = Ed25519KeyPair::from_seed_unchecked(&seed[..])
            .map_err(|_| IdentityError::CorruptCredential)?;
        let mut endpoint_public = [0; 32];
        endpoint_public.copy_from_slice(key.public_key().as_ref());
        Ok(Self {
            seed,
            endpoint_public,
        })
    }

    pub fn endpoint_public_bytes(&self) -> [u8; 32] {
        self.endpoint_public
    }

    pub fn endpoint_id_hex(&self) -> String {
        self.endpoint_public
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// Intended only for constructing the existing Iroh SecretKey::from_bytes.
    /// No key bytes are written to stdout, file paths, settings or telemetry.
    pub fn with_secret_seed<R>(&self, use_seed: impl FnOnce(&[u8; 32]) -> R) -> R {
        use_seed(&self.seed)
    }
}

fn generate_identity() -> Result<DeviceIdentity, IdentityError> {
    let mut seed = Zeroizing::new([0u8; 32]);
    SystemRandom::new()
        .fill(&mut seed[..])
        .map_err(|_| IdentityError::EntropyUnavailable)?;
    DeviceIdentity::from_seed(seed)
}

fn encode(identity: &DeviceIdentity) -> Zeroizing<[u8; RECORD_SIZE]> {
    let mut bytes = Zeroizing::new([0u8; RECORD_SIZE]);
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..40].copy_from_slice(&identity.endpoint_public);
    bytes[40..72].copy_from_slice(&identity.seed[..]);
    bytes
}

fn decode(bytes: &[u8]) -> Result<DeviceIdentity, IdentityError> {
    if bytes.len() != RECORD_SIZE || &bytes[..8] != MAGIC {
        return Err(IdentityError::CorruptCredential);
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    seed.copy_from_slice(&bytes[40..72]);
    let identity = DeviceIdentity::from_seed(seed)?;
    if identity.endpoint_public != bytes[8..40] {
        return Err(IdentityError::CorruptCredential);
    }
    Ok(identity)
}

/// Read an existing device key. Missing, malformed and inaccessible credential
/// stores do NOT result in silent regeneration or a new EndpointId.
pub fn load() -> Result<DeviceIdentity, IdentityError> {
    #[cfg(windows)]
    {
        windows_store::load_named(TARGET)
    }
    #[cfg(not(windows))]
    {
        Err(IdentityError::UnsupportedPlatform)
    }
}

/// Explicit first-time enrollment only. Never replaces an existing entry.
/// Call from a user-confirmed setup action, never during ordinary app startup.
pub fn initialize() -> Result<DeviceIdentity, IdentityError> {
    #[cfg(windows)]
    {
        windows_store::initialize_named(TARGET)
    }
    #[cfg(not(windows))]
    {
        Err(IdentityError::UnsupportedPlatform)
    }
}

pub mod peer_pin;

#[cfg(windows)]
mod windows_store {
    use super::*;
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

    struct CredentialBuffer(*mut CREDENTIALW);

    impl Drop for CredentialBuffer {
        fn drop(&mut self) {
            if self.0.is_null() {
                return;
            }
            // CredRead allocates the entire structure; scrub bounded binary
            // data before releasing the Windows-managed plaintext buffer.
            unsafe {
                let credential = &mut *self.0;
                if !credential.CredentialBlob.is_null()
                    && credential.CredentialBlobSize as usize <= 2560
                {
                    ptr::write_bytes(
                        credential.CredentialBlob,
                        0,
                        credential.CredentialBlobSize as usize,
                    );
                }
                CredFree(self.0.cast());
            }
        }
    }

    pub(super) struct EnrollmentLock(HANDLE);

    impl EnrollmentLock {
        pub(super) fn acquire() -> Result<Self, IdentityError> {
            // A bounded per-logon-session mutex prevents two cooperating Akula
            // processes from racing to replace a newly enrolled credential.
            let handle =
                unsafe { CreateMutexW(None, false, w!("Local\\AkulaP2PIrohEnrollmentV1")) }
                    .map_err(|_| IdentityError::StoreUnavailable)?;
            let outcome = unsafe { WaitForSingleObject(handle, 30_000) };
            if outcome.0 != 0 && outcome.0 != 0x80 {
                let _ = unsafe { CloseHandle(handle) };
                return Err(IdentityError::EnrollmentBusy);
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

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Fixed binary credentials for both secret identity and durable peer pins.
    pub(super) fn read_fixed<const N: usize>(
        target: &str,
    ) -> Result<Zeroizing<[u8; N]>, IdentityError> {
        if N == 0 || N > 2560 {
            return Err(IdentityError::StoreUnavailable);
        }
        let target = wide(target);
        let mut raw: *mut CREDENTIALW = ptr::null_mut();
        match unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut raw) } {
            Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => {
                return Err(IdentityError::NotInitialized);
            }
            Err(_) => return Err(IdentityError::StoreUnavailable),
            Ok(()) => {}
        }
        if raw.is_null() {
            return Err(IdentityError::StoreUnavailable);
        }
        let owned = CredentialBuffer(raw);
        let value = unsafe { &*owned.0 };
        if value.Type != CRED_TYPE_GENERIC
            || value.CredentialBlobSize as usize != N
            || value.CredentialBlob.is_null()
        {
            return Err(IdentityError::CorruptCredential);
        }
        let mut record = Zeroizing::new([0u8; N]);
        let slice = unsafe { std::slice::from_raw_parts(value.CredentialBlob, N) };
        record.copy_from_slice(slice);
        Ok(record)
    }

    pub(super) fn write_fixed<const N: usize>(
        target: &str,
        record: &mut Zeroizing<[u8; N]>,
    ) -> Result<(), IdentityError> {
        if N == 0 || N > 2560 {
            return Err(IdentityError::StoreUnavailable);
        }
        let mut target = wide(target);
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_mut_ptr()),
            CredentialBlob: record.as_mut_ptr(),
            CredentialBlobSize: N as u32,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..CREDENTIALW::default()
        };
        unsafe { CredWriteW(&credential, 0) }.map_err(|_| IdentityError::StoreUnavailable)
    }

    fn read_record(target: &str) -> Result<Zeroizing<[u8; RECORD_SIZE]>, IdentityError> {
        read_fixed::<RECORD_SIZE>(target)
    }

    fn write_record(target: &str, identity: &DeviceIdentity) -> Result<(), IdentityError> {
        let mut record = encode(identity);
        write_fixed(target, &mut record)
    }

    pub(super) fn load_named(target: &str) -> Result<DeviceIdentity, IdentityError> {
        let record = read_record(target)?;
        decode(&record[..])
    }

    pub(super) fn initialize_named(target: &str) -> Result<DeviceIdentity, IdentityError> {
        let _lock = EnrollmentLock::acquire()?;
        match read_record(target) {
            Err(IdentityError::NotInitialized) => {}
            Ok(_) => return Err(IdentityError::AlreadyInitialized),
            Err(error) => return Err(error),
        };
        let candidate = generate_identity()?;
        write_record(target, &candidate)?;
        let reloaded = load_named(target)?;
        if reloaded.endpoint_public != candidate.endpoint_public {
            return Err(IdentityError::EnrollmentChanged);
        }
        Ok(reloaded)
    }

    #[cfg(test)]
    pub(super) fn delete_named_test_only(target: &str) {
        use windows::Win32::Security::Credentials::CredDeleteW;
        let target = wide(target);
        let _ = unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_seed_roundtrip_and_malformed_records_fail_closed() {
        let identity = DeviceIdentity::from_seed(Zeroizing::new([7u8; 32])).unwrap();
        let record = encode(&identity);
        let reread = decode(&record[..]).unwrap();
        assert_eq!(
            reread.endpoint_public_bytes(),
            identity.endpoint_public_bytes()
        );
        assert_eq!(reread.endpoint_id_hex().len(), 64);
        let mut corrupted = record.clone();
        corrupted[8] ^= 1;
        assert!(matches!(
            decode(&corrupted[..]),
            Err(IdentityError::CorruptCredential)
        ));
        corrupted[8] ^= 1;
        corrupted[0] ^= 1;
        assert!(matches!(
            decode(&corrupted[..]),
            Err(IdentityError::CorruptCredential)
        ));
        assert!(matches!(
            decode(&record[..71]),
            Err(IdentityError::CorruptCredential)
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn never_falls_back_to_plaintext_storage_without_os_backend() {
        assert!(matches!(load(), Err(IdentityError::UnsupportedPlatform)));
        assert!(matches!(
            initialize(),
            Err(IdentityError::UnsupportedPlatform)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_credential_is_stable_and_cannot_be_silently_replaced() {
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                windows_store::delete_named_test_only(&self.0);
            }
        }
        let target = format!("Lalalalendia/Akula/P2P/test/{}", uuid::Uuid::new_v4());
        let _cleanup = Cleanup(target.clone());
        assert!(matches!(
            windows_store::load_named(&target),
            Err(IdentityError::NotInitialized)
        ));
        let initialized = windows_store::initialize_named(&target).unwrap();
        let expected = initialized.endpoint_id_hex();
        assert!(matches!(
            windows_store::initialize_named(&target),
            Err(IdentityError::AlreadyInitialized)
        ));
        drop(initialized);
        assert_eq!(
            windows_store::load_named(&target)
                .unwrap()
                .endpoint_id_hex(),
            expected
        );

        // The test harness is a second real OS process, not a fresh object in
        // the same address space. It must reopen Credential Manager itself.
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("p2p_device_identity::tests::windows_child_reads_same_key")
            .arg("--nocapture")
            .env("AKULA_P2P_TEST_CREDENTIAL_TARGET", &target)
            .env("AKULA_P2P_TEST_EXPECTED_PUBLIC", expected)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "separate process failed to reopen OS credential"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_child_reads_same_key() {
        let Ok(target) = std::env::var("AKULA_P2P_TEST_CREDENTIAL_TARGET") else {
            return;
        };
        let expected = std::env::var("AKULA_P2P_TEST_EXPECTED_PUBLIC").unwrap();
        assert_eq!(
            windows_store::load_named(&target)
                .unwrap()
                .endpoint_id_hex(),
            expected
        );
    }
}
