//! Device-local token lifecycle. No automatic OAuth, no files and no remote
//! revocation. All statuses are OS observations, never valid access proofs.
use super::*;

/// Presence of a nonempty well-formed OS-stored refresh token. No Yandex
/// server has accepted it; this MUST NOT enable Sync or read-write access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredRefreshPresence {
    Missing,
    PresentUnverified,
}

impl YandexCredentialVault {
    /// Only retrieve a protected token inside the OS-backed boundary;
    /// return a public flag without leaking account or token information.
    pub fn inspect_local(&self, scope: &YandexCredentialScope)
        -> Result<StoredRefreshPresence, SecretError>
    {
        match self.backend.get(scope.username())? {
            None => Ok(StoredRefreshPresence::Missing),
            Some(raw) => {
                let _validated = YandexRefreshToken::from_protected(raw)?;
                Ok(StoredRefreshPresence::PresentUnverified)
            }
        }
    }

    /// Local forgetting does NOT revoke at Yandex. Require a fresh absence
    /// observation after deletion; a "delete succeeded" OS response is not
    /// sufficient. A concurrent process could still reinsert a credential.
    pub fn forget_local(&self, scope: &YandexCredentialScope) -> Result<(), SecretError> {
        self.backend.delete(scope.username())?;
        if self.backend.get(scope.username())?.is_some() {
            return Err(SecretError::VerificationFailed);
        }
        Ok(())
    }
}
