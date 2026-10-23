//! Dropbox login without a Vault. Consent confirms only the account,
//! not the remote publisher, filesystem access or any synchronization.

use super::resume::{self, ResumeError};
use akula_dropbox_auth_lab::{
    callback::{BoundLogin, CallbackError},
    exchange::{exchange_once, ExchangeError},
    TokenExchange,
};
use akula_dropbox_secret_store::{
    account::{AccountCredentialScope, AccountCredentialStore},
    RefreshToken, SecretError,
};
use serde::Deserialize;
use std::{io::Read, time::Duration};
use zeroize::Zeroizing;

const ACCOUNT_ROUTE: &str = "https://api.dropboxapi.com/2/users/get_current_account";
const MAX_ACCOUNT_REPLY: u64 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    pub account_id: String,
    pub root_namespace_id: String,
}
impl AccountIdentity {
    fn validate(&self, app: &str) -> Result<AccountCredentialScope, AccountLoginError> {
        AccountCredentialScope::new(app, &self.account_id, &self.root_namespace_id)
            .map_err(|_| AccountLoginError::InvalidAccount)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountLoginError {
    InvalidPort,
    Callback(CallbackError),
    Exchange(ExchangeError),
    Refresh(ResumeError),
    MissingSavedConsent,
    AccountMismatch,
    InvalidAccount,
    NetworkUnavailable,
    SecretStore(SecretError),
}

/// One consent session. The fixed client ID/port must come from Akula's
/// product-owned App Folder configuration, never a user-supplied URL.
pub struct AccountSession {
    login: BoundLogin,
    app_key: String,
}
impl AccountSession {
    pub fn begin(app_key: &str, registered_port: u16) -> Result<Self, AccountLoginError> {
        if registered_port == 0 { return Err(AccountLoginError::InvalidPort); }
        let login = BoundLogin::bind(app_key, registered_port)
            .map_err(AccountLoginError::Callback)?;
        Ok(Self { login, app_key: app_key.to_owned() })
    }
    pub fn browser_authorize_url(&self) -> &str { self.login.authorize_url() }
    pub fn finish(self, timeout: Duration) -> Result<AccountIdentity, AccountLoginError> {
        let exchange = self.login.receive(timeout).map_err(AccountLoginError::Callback)?;
        connect_with(&LivePorts, exchange, &self.app_key)
    }
}

struct SessionTokens {
    account_id: String,
    access: Zeroizing<String>,
    refresh: Zeroizing<String>,
}
trait AccountPorts {
    fn exchange(&self, code: TokenExchange) -> Result<SessionTokens, AccountLoginError>;
    fn get_account(&self, access: &str) -> Result<AccountIdentity, AccountLoginError>;
    fn save(&self, scope: &AccountCredentialScope, refresh: &str)
        -> Result<(), AccountLoginError>;
}
struct LivePorts;
impl AccountPorts for LivePorts {
    fn exchange(&self, code: TokenExchange) -> Result<SessionTokens, AccountLoginError> {
        let reply = exchange_once(code).map_err(AccountLoginError::Exchange)?;
        Ok(SessionTokens {
            account_id: reply.account_id().to_owned(),
            access: Zeroizing::new(reply.access_token().to_owned()),
            refresh: Zeroizing::new(reply.refresh_token().to_owned()),
        })
    }
    fn get_account(&self, access: &str) -> Result<AccountIdentity, AccountLoginError> {
        #[derive(Deserialize)]
        struct Root { root_namespace_id: String }
        #[derive(Deserialize)]
        struct Wire { account_id: String, root_info: Root }
        let auth = Zeroizing::new(format!("Bearer {access}"));
        let agent = ureq::AgentBuilder::new().redirects(0)
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(20))
            .timeout_write(Duration::from_secs(20)).build();
        let response = agent.post(ACCOUNT_ROUTE)
            .set("Authorization", &auth)
            .set("Accept", "application/json")
            .call().map_err(|_| AccountLoginError::NetworkUnavailable)?;
        if response.header("Content-Length")
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|n| n > MAX_ACCOUNT_REPLY)
        { return Err(AccountLoginError::InvalidAccount); }
        let mut body = Vec::new();
        response.into_reader().take(MAX_ACCOUNT_REPLY + 1)
            .read_to_end(&mut body)
            .map_err(|_| AccountLoginError::NetworkUnavailable)?;
        if body.len() as u64 > MAX_ACCOUNT_REPLY {
            return Err(AccountLoginError::InvalidAccount);
        }
        let wire: Wire = serde_json::from_slice(&body)
            .map_err(|_| AccountLoginError::InvalidAccount)?;
        Ok(AccountIdentity {
            account_id: wire.account_id,
            root_namespace_id: wire.root_info.root_namespace_id,
        })
    }
    fn save(&self, scope: &AccountCredentialScope, refresh: &str)
        -> Result<(), AccountLoginError>
    {
        let credential = RefreshToken::new(refresh.to_owned())
            .map_err(AccountLoginError::SecretStore)?;
        AccountCredentialStore::system()
            .map_err(AccountLoginError::SecretStore)?
            .save_refresh(scope, &credential)
            .map_err(AccountLoginError::SecretStore)
    }
}

/// No Vault identity, external HEAD, File Gate, scheduler, upload or delete
/// is created or modified. Persist the token only after account/root proof.
fn connect_with<P: AccountPorts>(
    ports: &P, code: TokenExchange, app: &str,
) -> Result<AccountIdentity, AccountLoginError> {
    // Never send an OAuth code on behalf of malformed app identity.
    AccountCredentialScope::new(app, "dbid:preflight", "1")
        .map_err(|_| AccountLoginError::InvalidAccount)?;
    let tokens = ports.exchange(code)?;
    let identity = ports.get_account(&tokens.access)?;
    if tokens.account_id != identity.account_id {
        return Err(AccountLoginError::AccountMismatch);
    }
    let scope = identity.validate(app)?;
    ports.save(&scope, &tokens.refresh)?;
    Ok(identity)
}


/// Explicit account-only recovery after a cold restart. Uses the exact
/// account and namespace that were previously verified, without opening
/// a browser and without granting Vault publisher/file-system authority.
pub fn resume_account(
    app: &str, expected: &AccountIdentity,
) -> Result<AccountIdentity, AccountLoginError> {
    resume_account_with(&LivePorts, app, expected)
}

trait AccountResumePorts {
    fn load(&self, scope: &AccountCredentialScope)
        -> Result<Option<RefreshToken>, AccountLoginError>;
    fn refresh(&self, app: &str, saved: &RefreshToken)
        -> Result<Zeroizing<String>, AccountLoginError>;
    fn get_account(&self, access: &str)
        -> Result<AccountIdentity, AccountLoginError>;
}
impl AccountResumePorts for LivePorts {
    fn load(&self, scope: &AccountCredentialScope)
        -> Result<Option<RefreshToken>, AccountLoginError>
    {
        AccountCredentialStore::system().map_err(AccountLoginError::SecretStore)?
            .load_refresh(scope).map_err(AccountLoginError::SecretStore)
    }
    fn refresh(&self, app: &str, saved: &RefreshToken)
        -> Result<Zeroizing<String>, AccountLoginError>
    {
        resume::refresh_account_access(app, saved).map_err(AccountLoginError::Refresh)
    }
    fn get_account(&self, access: &str)
        -> Result<AccountIdentity, AccountLoginError>
    {
        <Self as AccountPorts>::get_account(self, access)
    }
}

/// Never convert a saved account_id to proof of consent. Recovery is
/// successful only after keyring read, OAuth refresh and a fresh Dropbox
/// account/root identity check. Invalid scope prevents any network call.
fn resume_account_with<P: AccountResumePorts>(
    ports: &P, app: &str, expected: &AccountIdentity,
) -> Result<AccountIdentity, AccountLoginError> {
    let scope = expected.validate(app)?;
    let saved = ports.load(&scope)?.ok_or(AccountLoginError::MissingSavedConsent)?;
    let access = ports.refresh(app, &saved)?;
    let actual = ports.get_account(&access)?;
    if actual.account_id != expected.account_id
        || actual.root_namespace_id != expected.root_namespace_id
    {
        return Err(AccountLoginError::AccountMismatch);
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;
    use akula_dropbox_auth_lab::begin_readonly;
    use std::sync::Mutex;
    use url::Url;

    fn code() -> TokenExchange {
        let start = begin_readonly("app_key_123",
            "http://127.0.0.1:47219/dropbox/callback").unwrap();
        let state = Url::parse(&start.authorize_url).unwrap().query_pairs()
            .find(|(key, _)| key == "state").unwrap().1.into_owned();
        start.complete(&format!(
            "http://127.0.0.1:47219/dropbox/callback?state={state}&code=once",
        )).unwrap()
    }
    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<&'static str>>,
        other_account: bool,
        bad_namespace: bool,
        read_failed: bool,
        store_failed: bool,
    }
    impl Fake {
        fn calls(&self) -> Vec<&'static str> { self.calls.lock().unwrap().clone() }
    }
    impl AccountPorts for Fake {
        fn exchange(&self, code: TokenExchange) -> Result<SessionTokens, AccountLoginError> {
            self.calls.lock().unwrap().push("exchange");
            let form = code.into_form_body();
            assert!(form.contains("code=once") && !form.contains("client_secret"));
            Ok(SessionTokens {
                account_id: "dbid:alice".into(),
                access: Zeroizing::new("synthetic-access".into()),
                refresh: Zeroizing::new("synthetic-refresh".into()),
            })
        }
        fn get_account(&self, access: &str) -> Result<AccountIdentity, AccountLoginError> {
            self.calls.lock().unwrap().push("get_current_account");
            assert_eq!(access, "synthetic-access");
            if self.read_failed { return Err(AccountLoginError::NetworkUnavailable); }
            Ok(AccountIdentity {
                account_id: if self.other_account { "dbid:bob" } else { "dbid:alice" }.into(),
                root_namespace_id: if self.bad_namespace { "../bad" } else { "12345" }.into(),
            })
        }
        fn save(&self, _: &AccountCredentialScope, refresh: &str)
            -> Result<(), AccountLoginError>
        {
            self.calls.lock().unwrap().push("save_os");
            assert_eq!(refresh, "synthetic-refresh");
            if self.store_failed {
                return Err(AccountLoginError::SecretStore(SecretError::StoreUnavailable));
            }
            Ok(())
        }
    }

    #[test]
    fn account_consent_uses_no_vault_and_only_stores_verified_identity() {
        let fake = Fake::default();
        let account = connect_with(&fake, code(), "app_key_123").unwrap();
        assert_eq!(account.account_id, "dbid:alice");
        assert_eq!(account.root_namespace_id, "12345");
        assert_eq!(fake.calls(), vec!["exchange", "get_current_account", "save_os"]);
    }

    #[test]
    fn mismatched_account_and_invalid_root_never_store_tokens() {
        for fake in [
            Fake { other_account: true, ..Default::default() },
            Fake { bad_namespace: true, ..Default::default() },
            Fake { read_failed: true, ..Default::default() },
        ] {
            assert!(connect_with(&fake, code(), "app_key_123").is_err());
            assert_eq!(fake.calls(), vec!["exchange", "get_current_account"]);
        }
    }

    #[test]
    fn unavailable_keyring_fails_closed_and_never_reports_logged_in() {
        let fake = Fake { store_failed: true, ..Default::default() };
        assert_eq!(connect_with(&fake, code(), "app_key_123"),
            Err(AccountLoginError::SecretStore(SecretError::StoreUnavailable)));
        assert_eq!(fake.calls(), vec!["exchange", "get_current_account", "save_os"]);
    }

    #[derive(Default)]
    struct FakeRecovery {
        calls: Mutex<Vec<&'static str>>,
        missing: bool,
        locked: bool,
        refresh_denied: bool,
        network_down: bool,
        other_account: bool,
        other_root: bool,
    }
    impl FakeRecovery {
        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().unwrap().clone()
        }
    }
    impl AccountResumePorts for FakeRecovery {
        fn load(&self, _: &AccountCredentialScope)
            -> Result<Option<RefreshToken>, AccountLoginError>
        {
            self.calls.lock().unwrap().push("load_keyring");
            if self.locked {
                return Err(AccountLoginError::SecretStore(SecretError::StoreUnavailable));
            }
            if self.missing { return Ok(None); }
            Ok(Some(RefreshToken::new("synthetic-saved-refresh".into()).unwrap()))
        }
        fn refresh(&self, app: &str, saved: &RefreshToken)
            -> Result<Zeroizing<String>, AccountLoginError>
        {
            self.calls.lock().unwrap().push("refresh_once");
            assert_eq!(app, "app_key_123");
            assert_eq!(saved.as_str(), "synthetic-saved-refresh");
            if self.refresh_denied {
                return Err(AccountLoginError::Refresh(ResumeError::RefreshRejected));
            }
            Ok(Zeroizing::new("synthetic-new-access".into()))
        }
        fn get_account(&self, access: &str)
            -> Result<AccountIdentity, AccountLoginError>
        {
            self.calls.lock().unwrap().push("get_current_account");
            assert_eq!(access, "synthetic-new-access");
            if self.network_down { return Err(AccountLoginError::NetworkUnavailable); }
            Ok(AccountIdentity {
                account_id: if self.other_account { "dbid:bob" }
                    else { "dbid:alice" }.into(),
                root_namespace_id: if self.other_root { "98765" }
                    else { "12345" }.into(),
            })
        }
    }
    fn expected_account() -> AccountIdentity {
        AccountIdentity {
            account_id: "dbid:alice".into(),
            root_namespace_id: "12345".into(),
        }
    }

    #[test]
    fn restart_recovers_exact_account_without_browser_or_vault() {
        let ports = FakeRecovery::default();
        assert_eq!(resume_account_with(&ports, "app_key_123", &expected_account()),
            Ok(expected_account()));
        assert_eq!(ports.calls(),
            vec!["load_keyring", "refresh_once", "get_current_account"]);
    }

    #[test]
    fn restart_rejects_missing_locked_revoked_or_offline_consent() {
        for ports in [
            FakeRecovery { missing: true, ..Default::default() },
            FakeRecovery { locked: true, ..Default::default() },
            FakeRecovery { refresh_denied: true, ..Default::default() },
            FakeRecovery { network_down: true, ..Default::default() },
        ] {
            assert!(resume_account_with(&ports, "app_key_123", &expected_account()).is_err());
            assert!(!ports.calls().is_empty());
        }
    }

    #[test]
    fn restart_cannot_claim_foreign_account_or_changed_namespace() {
        for ports in [
            FakeRecovery { other_account: true, ..Default::default() },
            FakeRecovery { other_root: true, ..Default::default() },
        ] {
            assert_eq!(
                resume_account_with(&ports, "app_key_123", &expected_account()),
                Err(AccountLoginError::AccountMismatch));
            assert_eq!(ports.calls(),
                vec!["load_keyring", "refresh_once", "get_current_account"]);
        }
    }

    #[test]
    fn forged_saved_app_or_account_fails_before_keyring_or_network() {
        let ports = FakeRecovery::default();
        assert_eq!(resume_account_with(&ports, "../wrong", &expected_account()),
            Err(AccountLoginError::InvalidAccount));
        assert_eq!(resume_account_with(&ports, "app_key_123", &AccountIdentity {
            account_id: "other".into(), root_namespace_id: "12345".into(),
        }), Err(AccountLoginError::InvalidAccount));
        assert!(ports.calls().is_empty());
    }

    #[test]
    fn malformed_app_id_rejected_before_oauth_exchange() {
        let fake = Fake::default();
        assert!(matches!(connect_with(&fake, code(), "../bad_app"),
            Err(AccountLoginError::InvalidAccount)));
        assert!(fake.calls().is_empty());
    }
}
