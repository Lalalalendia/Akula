//! Dropbox login without a Vault. Consent confirms only the account,
//! not the remote publisher, filesystem access or any synchronization.

use super::resume::{self, ResumeError};
use akula_dropbox_auth_lab::{
    callback::{BoundLogin, CallbackError},
    exchange::{exchange_once, ExchangeError},
    AuthError, TokenExchange,
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
    BrowserUnavailable,
    Callback(CallbackError),
    Exchange(ExchangeError),
    Refresh(ResumeError),
    MissingSavedConsent,
    AccountMismatch,
    InvalidAccount,
    NetworkUnavailable,
    SecretStore(SecretError),
}

/// Safe diagnostic categories: never include OAuth codes, tokens, account IDs
/// or Dropbox-provided error text. Every rejection keeps sync disabled.
impl AccountLoginError {
    pub fn public_status(&self) -> &'static str {
        match self {
            Self::InvalidPort | Self::Callback(CallbackError::PortUnavailable) =>
                "DROPBOX_CALLBACK_PORT_UNAVAILABLE / SYNC_DISABLED",
            Self::BrowserUnavailable => "DROPBOX_BROWSER_UNAVAILABLE / SYNC_DISABLED",
            Self::Callback(CallbackError::Expired) =>
                "DROPBOX_BROWSER_TIMEOUT / SYNC_DISABLED",
            Self::Callback(CallbackError::Authorization(AuthError::Denied)) =>
                "DROPBOX_CONSENT_DENIED / SYNC_DISABLED",
            Self::Callback(CallbackError::Authorization(AuthError::StateMismatch)) =>
                "DROPBOX_OAUTH_STATE_MISMATCH / SYNC_DISABLED",
            Self::Callback(_) => "DROPBOX_CALLBACK_REJECTED / SYNC_DISABLED",
            Self::Exchange(ExchangeError::UnexpectedScope)
            | Self::Refresh(ResumeError::ScopeEscalation) =>
                "DROPBOX_READ_SCOPES_REJECTED / SYNC_DISABLED",
            Self::Exchange(ExchangeError::Rejected) =>
                "DROPBOX_OAUTH_APP_OR_REDIRECT_REJECTED / SYNC_DISABLED",
            Self::Exchange(ExchangeError::TransportUncertain)
            | Self::Refresh(ResumeError::RefreshUncertain) =>
                "DROPBOX_NETWORK_UNAVAILABLE / SYNC_DISABLED",
            Self::Exchange(_) => "DROPBOX_TOKEN_RESPONSE_REJECTED / SYNC_DISABLED",
            Self::Refresh(ResumeError::RefreshRejected) =>
                "DROPBOX_SAVED_CONSENT_REVOKED / SYNC_DISABLED",
            Self::Refresh(_) => "DROPBOX_REFRESH_REJECTED / SYNC_DISABLED",
            Self::MissingSavedConsent =>
                "DROPBOX_NO_SAVED_CONSENT / SYNC_DISABLED",
            Self::AccountMismatch =>
                "DROPBOX_ACCOUNT_OR_ROOT_MISMATCH / SYNC_DISABLED",
            Self::InvalidAccount =>
                "DROPBOX_ACCOUNT_IDENTITY_INVALID / SYNC_DISABLED",
            Self::NetworkUnavailable =>
                "DROPBOX_ACCOUNT_LOOKUP_FAILED / SYNC_DISABLED",
            Self::SecretStore(SecretError::StoreUnavailable)
            | Self::SecretStore(SecretError::StoreFailure) =>
                "DROPBOX_OS_KEYRING_UNAVAILABLE / SYNC_DISABLED",
            Self::SecretStore(_) =>
                "DROPBOX_OS_KEYRING_VERIFY_FAILED / SYNC_DISABLED",
        }
    }
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
        self.finish_for(timeout, None)
    }
    pub fn finish_for(
        self,
        timeout: Duration,
        expected: Option<&AccountIdentity>,
    ) -> Result<AccountIdentity, AccountLoginError> {
        let exchange = self.login.receive(timeout).map_err(AccountLoginError::Callback)?;
        connect_with(&LivePorts, exchange, &self.app_key, expected)
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
/// Only this private seam can substitute a local endpoint in tests. The
/// public account login always calls Dropbox's hard-coded HTTPS API.
fn lookup_account_at(endpoint: &str, access: &str)
    -> Result<AccountIdentity, AccountLoginError>
{
        #[derive(Deserialize)]
        struct Root { root_namespace_id: String }
        #[derive(Deserialize)]
        struct Wire { account_id: String, root_info: Root }
        let auth = Zeroizing::new(format!("Bearer {access}"));
        let agent = ureq::AgentBuilder::new().redirects(0)
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(20))
            .timeout_write(Duration::from_secs(20)).build();
        let response = agent.post(endpoint)
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
        lookup_account_at(ACCOUNT_ROUTE, access)
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
    ports: &P, code: TokenExchange, app: &str, expected: Option<&AccountIdentity>,
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
    // Re-consent is bound to the saved account; never orphan its old OS key.
    if expected.is_some_and(|previous| previous != &identity) {
        return Err(AccountLoginError::AccountMismatch);
    }
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
        // Reuse the exact fail-closed Vault refresh validation, but retain
        // account-only authority and return only a zeroizing short-lived token.
        resume::refresh_once(app, saved)
            .map(|session| session.token.into_sensitive_string())
            .map_err(AccountLoginError::Refresh)
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
#[path = "account/tests.rs"]
mod tests;
