#![forbid(unsafe_code)]
//! Explicit read-only Dropbox onboarding and restart verification.
//! Only independently pinned head bytes; no publisher trust or file mutation.

pub mod resume;

use akula_dropbox_auth_lab::{
    callback::{BoundLogin, CallbackError},
    exchange::{exchange_once, ExchangeError},
    TokenExchange,
};
use akula_dropbox_secret_store::{
    CredentialScope, CredentialVault, RefreshToken, SecretError,
};
use akula_dropbox_t0::{
    read_http::{AccessToken, DropboxReadOnly, ReadError},
    read_verify::{TrustedBlake3, MAX_READ_BYTES},
    DropboxRevision, ObjectKey, Scope,
};
use std::time::Duration;
use zeroize::Zeroizing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    InvalidOAuthApp,
    InvalidPort,
    Callback(CallbackError),
    Exchange(ExchangeError),
    AccountMismatch,
    SecretStore(SecretError),
    Remote(ReadError),
    WrongLogicalHead,
    WrongPinnedBytes,
    Oversize,
}

struct SessionTokens {
    account_id: String,
    access: Zeroizing<String>,
    refresh: Zeroizing<String>,
}

struct PinnedHead {
    key: ObjectKey,
    revision: DropboxRevision,
    bytes: Vec<u8>,
}

/// Internal boundary; tests mock only this boundary. Production uses all
/// three independently verified native packages (no test-only fallback).
trait ConnectionPorts {
    fn exchange(&self, once: TokenExchange) -> Result<SessionTokens, ConnectError>;
    fn read_head(
        &self, scope: &Scope, access_token: &str, pin: &TrustedBlake3,
    ) -> Result<PinnedHead, ConnectError>;
    fn save_refresh(
        &self, scope: &CredentialScope, refresh_token: &str,
    ) -> Result<(), ConnectError>;
}

struct LivePorts;

impl ConnectionPorts for LivePorts {
    fn exchange(&self, once: TokenExchange) -> Result<SessionTokens, ConnectError> {
        let tokens = exchange_once(once).map_err(ConnectError::Exchange)?;
        Ok(SessionTokens {
            account_id: tokens.account_id().to_owned(),
            access: Zeroizing::new(tokens.access_token().to_owned()),
            refresh: Zeroizing::new(tokens.refresh_token().to_owned()),
        })
    }

    fn read_head(
        &self, scope: &Scope, access_token: &str, pin: &TrustedBlake3,
    ) -> Result<PinnedHead, ConnectError> {
        let access = AccessToken::new(access_token.to_owned()).map_err(ConnectError::Remote)?;
        let reader = DropboxReadOnly::connect(scope.clone(), access)
            .map_err(ConnectError::Remote)?;
        let read = reader.read_key_verified(&ObjectKey::head(), pin)
            .map_err(ConnectError::Remote)?;
        Ok(PinnedHead {
            key: read.logical_key,
            revision: read.revision,
            bytes: read.bytes,
        })
    }

    fn save_refresh(
        &self, scope: &CredentialScope, refresh_token: &str,
    ) -> Result<(), ConnectError> {
        let vault = CredentialVault::system().map_err(ConnectError::SecretStore)?;
        let protected = RefreshToken::new(refresh_token.to_owned())
            .map_err(ConnectError::SecretStore)?;
        vault.save_refresh(scope, &protected).map_err(ConnectError::SecretStore)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// The authenticated Dropbox account and exact remote bytes were checked,
    /// but the remote publisher and signed DAG were NOT authenticated.
    RemotePinnedNeedsPublisherApproval,
}

pub struct ReadOnlyHeadCandidate {
    scope: Scope,
    revision: DropboxRevision,
    bytes: Vec<u8>,
}

impl std::fmt::Debug for ReadOnlyHeadCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReadOnlyHeadCandidate(PENDING_PUBLISHER_APPROVAL)")
    }
}

impl ReadOnlyHeadCandidate {
    pub fn status(&self) -> ConnectionStatus {
        ConnectionStatus::RemotePinnedNeedsPublisherApproval
    }
    pub fn remote_revision(&self) -> &DropboxRevision { &self.revision }
    pub fn expected_scope(&self) -> &Scope { &self.scope }
    pub fn read_only_head_bytes(&self) -> &[u8] { &self.bytes }
}

/// A single native login; the OAuth app key and credential namespace are
/// fixed together before the system browser is opened. This prevents mixing
/// two Dropbox App Folder applications in the local secret store.
pub struct ConnectSession {
    login: BoundLogin,
    scope: Scope,
    oauth_app_key: String,
    trusted_head_digest: TrustedBlake3,
}

impl ConnectSession {
    pub fn begin(
        scope: Scope, oauth_app_key: &str, registered_port: u16,
        independently_pinned_head: TrustedBlake3,
    ) -> Result<Self, ConnectError> {
        if registered_port == 0 { return Err(ConnectError::InvalidPort); }
        credential_scope(&scope, oauth_app_key)?;
        let login = BoundLogin::bind(oauth_app_key, registered_port)
            .map_err(ConnectError::Callback)?;
        Ok(Self {
            login, scope, oauth_app_key: oauth_app_key.to_owned(),
            trusted_head_digest: independently_pinned_head,
        })
    }

    /// This URL must be opened ONLY in the user's system browser.
    /// The exact localhost redirect must be registered for oauth_app_key.
    pub fn browser_authorize_url(&self) -> &str { self.login.authorize_url() }

    /// One-time user-driven completion; does not automatically modify files.
    pub fn finish(self, timeout: Duration) -> Result<ReadOnlyHeadCandidate, ConnectError> {
        let exchange = self.login.receive(timeout).map_err(ConnectError::Callback)?;
        connect_with_ports(&LivePorts, exchange, &self.scope,
            &self.oauth_app_key, &self.trusted_head_digest)
    }
}

fn credential_scope(scope: &Scope, app: &str) -> Result<CredentialScope, ConnectError> {
    CredentialScope::new(
        scope.account_id(), scope.root_namespace_id(), scope.vault_id(),
        scope.profile_id(), app,
    ).map_err(|_| ConnectError::InvalidOAuthApp)
}

fn connect_with_ports<P: ConnectionPorts>(
    ports: &P, one_time_exchange: TokenExchange, scope: &Scope,
    oauth_app_key: &str, head_pin: &TrustedBlake3,
) -> Result<ReadOnlyHeadCandidate, ConnectError> {
    // Reject invalid app/credential namespace before consuming the code.
    let secret_scope = credential_scope(scope, oauth_app_key)?;
    let tokens = ports.exchange(one_time_exchange)?;
    if tokens.account_id != scope.account_id() {
        return Err(ConnectError::AccountMismatch);
    }

    // DropboxReadOnly::connect enforces account_id AND root_namespace_id.
    // Its read_key_verified enforces native file identity, exact revision,
    // response metadata, byte limit, Dropbox SHA-256 and trusted BLAKE3.
    // The coordinator independently recomputes BLAKE3 as a final fence.
    let head = ports.read_head(scope, &tokens.access, head_pin)?;
    if head.key != ObjectKey::head() { return Err(ConnectError::WrongLogicalHead); }
    if head.bytes.len() > MAX_READ_BYTES { return Err(ConnectError::Oversize); }
    if blake3::hash(&head.bytes).to_hex().as_str() != head_pin.as_str() {
        return Err(ConnectError::WrongPinnedBytes);
    }

    // No refresh token is saved on failed auth, wrong remote scope or
    // incomplete head readback; the OS store itself requires cold readback.
    ports.save_refresh(&secret_scope, &tokens.refresh)?;
    Ok(ReadOnlyHeadCandidate {
        scope: scope.clone(),
        revision: head.revision,
        bytes: head.bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use akula_dropbox_auth_lab::begin_readonly;
    use std::sync::Mutex;
    use url::Url;

    const HEAD: &[u8] = b"pinned synthetic manifest bytes; not user notes";
    fn scope() -> Scope {
        Scope::new("dbid:alice", "12345",
            "12345678-1234-1234-1234-123456789abc", "profile_a").unwrap()
    }
    fn pin() -> TrustedBlake3 {
        TrustedBlake3::parse(&blake3::hash(HEAD).to_hex().to_string()).unwrap()
    }
    fn exchange() -> TokenExchange {
        let start = begin_readonly("app_key_123",
            "http://127.0.0.1:47219/dropbox/callback").unwrap();
        let state = Url::parse(&start.authorize_url).unwrap().query_pairs()
            .find(|(k, _)| k == "state").unwrap().1.into_owned();
        start.complete(&format!(
            "http://127.0.0.1:47219/dropbox/callback?state={state}&code=once-only",
        )).unwrap()
    }

    #[derive(Default)]
    struct FakePorts {
        calls: Mutex<Vec<&'static str>>,
        account: Option<String>,
        read_error: bool,
        store_error: bool,
        wrong_key: bool,
        corrupt: bool,
        oversized: bool,
    }
    impl FakePorts {
        fn calls(&self) -> Vec<&'static str> { self.calls.lock().unwrap().clone() }
    }
    impl ConnectionPorts for FakePorts {
        fn exchange(&self, once: TokenExchange) -> Result<SessionTokens, ConnectError> {
            self.calls.lock().unwrap().push("exchange");
            let payload = once.into_form_body();
            assert!(payload.contains("code=once-only"));
            Ok(SessionTokens {
                account_id: self.account.clone().unwrap_or("dbid:alice".into()),
                access: Zeroizing::new("synthetic-access-token".into()),
                refresh: Zeroizing::new("synthetic-refresh-token".into()),
            })
        }
        fn read_head(&self, scope: &Scope, access: &str, _: &TrustedBlake3)
            -> Result<PinnedHead, ConnectError>
        {
            self.calls.lock().unwrap().push("remote");
            assert_eq!(scope.account_id(), "dbid:alice");
            assert_eq!(scope.root_namespace_id(), "12345");
            assert_eq!(access, "synthetic-access-token");
            if self.read_error {
                return Err(ConnectError::Remote(ReadError::AccountMismatch));
            }
            let bytes = if self.oversized { vec![0; MAX_READ_BYTES + 1] }
                else if self.corrupt { b"corrupted synthetic manifest bytes".to_vec() }
                else { HEAD.to_vec() };
            Ok(PinnedHead {
                key: if self.wrong_key {
                    ObjectKey::parse(&format!("blobs/{}", "a".repeat(64))).unwrap()
                } else { ObjectKey::head() },
                revision: DropboxRevision::parse("abcdef012345").unwrap(),
                bytes,
            })
        }
        fn save_refresh(&self, scope: &CredentialScope, refresh: &str)
            -> Result<(), ConnectError>
        {
            self.calls.lock().unwrap().push("store");
            assert_eq!(refresh, "synthetic-refresh-token");
            let separate_app = credential_scope(&crate::tests::scope(), "drop_app").unwrap();
            assert!(!scope.is_same_scope(&separate_app));
            if self.store_error { return Err(ConnectError::SecretStore(
                SecretError::StoreUnavailable)); }
            Ok(())
        }
    }

    fn run(p: &FakePorts) -> Result<ReadOnlyHeadCandidate, ConnectError> {
        connect_with_ports(p, exchange(), &scope(), "app_key_123", &pin())
    }

    #[test]
    fn proof_is_read_only_and_requires_independent_publisher_admission() {
        let fake = FakePorts::default();
        let proof = run(&fake).unwrap();
        assert_eq!(fake.calls(), vec!["exchange", "remote", "store"]);
        assert_eq!(proof.read_only_head_bytes(), HEAD);
        assert_eq!(proof.remote_revision().as_str(), "abcdef012345");
        assert_eq!(proof.expected_scope(), &scope());
        assert_eq!(proof.status(), ConnectionStatus::RemotePinnedNeedsPublisherApproval);
        assert_eq!(format!("{proof:?}"),
            "ReadOnlyHeadCandidate(PENDING_PUBLISHER_APPROVAL)");
    }

    #[test]
    fn other_dropbox_account_rejected_before_remote_or_store() {
        let fake = FakePorts { account: Some("dbid:bob".into()), ..Default::default() };
        assert!(matches!(run(&fake), Err(ConnectError::AccountMismatch)));
        assert_eq!(fake.calls(), vec!["exchange"]);
    }

    #[test]
    fn invalid_oauth_app_prevents_token_exchange() {
        let fake = FakePorts::default();
        let result = connect_with_ports(&fake, exchange(), &scope(), "../wrong_app", &pin());
        assert!(matches!(result, Err(ConnectError::InvalidOAuthApp)));
        assert!(fake.calls().is_empty());
    }

    #[test]
    fn remote_root_mismatch_is_hold_and_does_not_store_credentials() {
        let fake = FakePorts { read_error: true, ..Default::default() };
        assert!(matches!(run(&fake), Err(ConnectError::Remote(ReadError::AccountMismatch))));
        assert_eq!(fake.calls(), vec!["exchange", "remote"]);
    }

    #[test]
    fn malformed_head_object_cannot_be_marked_as_connected() {
        let fake = FakePorts { wrong_key: true, ..Default::default() };
        assert!(matches!(run(&fake), Err(ConnectError::WrongLogicalHead)));
        assert_eq!(fake.calls(), vec!["exchange", "remote"]);
    }

    #[test]
    fn corrupt_or_rolled_back_head_fails_pin_and_never_stores_token() {
        let fake = FakePorts { corrupt: true, ..Default::default() };
        assert!(matches!(run(&fake), Err(ConnectError::WrongPinnedBytes)));
        assert_eq!(fake.calls(), vec!["exchange", "remote"]);
    }

    #[test]
    fn bounded_head_readback_refuses_oversize() {
        let fake = FakePorts { oversized: true, ..Default::default() };
        assert!(matches!(run(&fake), Err(ConnectError::Oversize)));
        assert_eq!(fake.calls(), vec!["exchange", "remote"]);
    }

    #[test]
    fn unavailable_os_keyring_cannot_mark_connection_successful() {
        let fake = FakePorts { store_error: true, ..Default::default() };
        assert!(matches!(run(&fake), Err(ConnectError::SecretStore(
            SecretError::StoreUnavailable))));
        assert_eq!(fake.calls(), vec!["exchange", "remote", "store"]);
    }

    #[test]
    fn nonregistered_zero_port_is_never_used_for_real_browser_login() {
        let result = ConnectSession::begin(scope(), "app_key_123", 0, pin());
        assert!(matches!(result, Err(ConnectError::InvalidPort)));
    }

    #[test]
    fn scope_app_identity_is_partitioned_even_for_same_account_and_vault() {
        let primary = credential_scope(&scope(), "vault_sync_app").unwrap();
        let drop_app = credential_scope(&scope(), "drop_app").unwrap();
        assert!(!primary.is_same_scope(&drop_app));
    }
}
