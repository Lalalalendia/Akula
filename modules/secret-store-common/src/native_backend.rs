//! Private source shared by the two unpublished provider crates.
//! The parent supplies its own Backend, SecretError and fixed SERVICE.
//! Compiled separately in each crate: no public cross-provider store API.

use super::{Backend, SecretError, SERVICE};
use zeroize::Zeroizing;

pub(super) struct NativeBackend {
    /// Serialize accesses inside one application instance (Windows does
    /// not guarantee ordering for writes to one credential).
    operations: std::sync::Mutex<()>,
}

impl NativeBackend {
    pub(super) fn new() -> Self { Self { operations: std::sync::Mutex::new(()) } }

    fn entry(&self, username: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(SERVICE, username).map_err(|_| SecretError::StoreUnavailable)
    }
}

impl Backend for NativeBackend {
    fn put(&self, username: &str, token: &str) -> Result<(), SecretError> {
        let _lock = self.operations.lock().map_err(|_| SecretError::StoreFailure)?;
        self.entry(username)?.set_password(token)
            .map_err(|_| SecretError::StoreFailure)
    }

    fn get(&self, username: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
        let _lock = self.operations.lock().map_err(|_| SecretError::StoreFailure)?;
        match self.entry(username)?.get_password() {
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(SecretError::StoreFailure),
        }
    }

    fn delete(&self, username: &str) -> Result<(), SecretError> {
        let _lock = self.operations.lock().map_err(|_| SecretError::StoreFailure)?;
        match self.entry(username)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(SecretError::StoreFailure),
        }
    }
}
