//! Device-local lifecycle for Yandex Disk's OS-stored refresh token.
//! No network, browser, OAuth refresh request, vault writes or Sync authority.
//! The account hint may come from untrusted JSON; pin the registered OAuth
//! app identity *before* constructing the credential namespace.

use akula_yandex_disk_secret_store::{
    YandexCredentialScope, YandexCredentialVault,
};
pub use akula_yandex_disk_secret_store::StoredRefreshPresence;
use super::ConnectError;

/// An exact, validated OS keyring namespace for one Yandex subject, one
/// product OAuth application, one device-local workspace UUID and profile.
/// No credential data or user-entered host is stored here.
pub struct LocalCredentialScope {
    scope: YandexCredentialScope,
}

impl LocalCredentialScope {
    pub fn from_public_hint(
        yandex_subject_id: &str,
        saved_oauth_app_id: &str,
        compiled_oauth_app_id: &str,
        local_workspace_uuid: &str,
        profile_id: &str,
    ) -> Result<Self, ConnectError> {
        if saved_oauth_app_id.is_empty()
            || saved_oauth_app_id != compiled_oauth_app_id
        {
            return Err(ConnectError::InvalidLocalScope);
        }
        let scope = YandexCredentialScope::new(
            yandex_subject_id, local_workspace_uuid,
            profile_id, compiled_oauth_app_id,
        ).map_err(ConnectError::SecretStore)?;
        Ok(Self { scope })
    }

    /// Presence does not prove a live OAuth grant. No request to Yandex is
    /// issued; always label any found token PresentUnverified.
    pub fn inspect(&self) -> Result<StoredRefreshPresence, ConnectError> {
        YandexCredentialVault::system().map_err(ConnectError::SecretStore)?
            .inspect_local(&self.scope)
            .map_err(ConnectError::SecretStore)
    }

    /// Exact local-only deletion; the OS backend confirms the key is absent.
    /// Remote Yandex OAuth permissions and server-side sessions remain intact.
    pub fn forget_local(self) -> Result<(), ConnectError> {
        YandexCredentialVault::system().map_err(ConnectError::SecretStore)?
            .forget_local(&self.scope)
            .map_err(ConnectError::SecretStore)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akula_yandex_disk_secret_store::SecretError;

    const APP: &str = "4760187d81bc4b7799476b42b5103713";
    const VAULT: &str = "12345678-1234-1234-1234-123456789abc";

    #[test]
    fn wrong_app_cannot_alias_existing_os_credential_namespace() {
        let refused = LocalCredentialScope::from_public_hint(
            "1000034426", "untrusted-app", APP, VAULT, "personal_disk"
        );
        assert!(matches!(refused, Err(ConnectError::InvalidLocalScope)));
        let refused = LocalCredentialScope::from_public_hint(
            "1000034426", "", APP, VAULT, "personal_disk"
        );
        assert!(matches!(refused, Err(ConnectError::InvalidLocalScope)));
    }

    #[test]
    fn forged_account_and_path_do_not_reach_os_backend() {
        for (account, vault, profile) in [
            ("alice", VAULT, "personal_disk"),
            ("1000034426", "../parent", "personal_disk"),
            ("1000034426", VAULT, "../test"),
        ] {
            assert!(matches!(
                LocalCredentialScope::from_public_hint(
                    account, APP, APP, vault, profile
                ),
                Err(ConnectError::SecretStore(SecretError::InvalidScope))
            ));
        }
    }

    #[test]
    fn disabled_system_store_fails_closed_and_stays_offline() {
        if !cfg!(feature = "system-store") {
            let scope = LocalCredentialScope::from_public_hint(
                "1000034426", APP, APP, VAULT, "personal_disk"
            ).unwrap();
            assert!(matches!(
                scope.inspect(),
                Err(ConnectError::SecretStore(SecretError::StoreUnavailable))
            ));
            let scope = LocalCredentialScope::from_public_hint(
                "1000034426", APP, APP, VAULT, "personal_disk"
            ).unwrap();
            assert!(matches!(
                scope.forget_local(),
                Err(ConnectError::SecretStore(SecretError::StoreUnavailable))
            ));
        }
    }
}
