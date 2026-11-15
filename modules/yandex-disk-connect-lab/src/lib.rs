#![forbid(unsafe_code)]
//! Native-only, explicit two-phase connection to personal Yandex Disk.
//! No Sync Core dependency, no local file writes, no remote uploads/deletes.
//! A successful account/app-folder probe is NOT a verified remote snapshot,
//! writer authority, file entitlement, or authorization to start Sync.
//!
//! Do not call finish from the native GUI thread. No automatic startup,
//! browser launch, callback listener, HTTP or token refresh happens until
//! a host explicitly begins a user-driven session and opens its URL.
pub mod local_lifecycle;
use akula_yandex_disk_rest::{
    oauth::{
        callback::{BoundLogin, CallbackError},
        exchange::{YandexTokenExchangeError, YandexEphemeralTokens},
        identity::YandexAccountError,
        YandexTokenExchange,
    },
    YandexAppPath, YandexDiskReadOnlyClient, YandexDiskRestError,
};
use akula_yandex_disk_secret_store::{
    SecretError, YandexCredentialScope, YandexCredentialVault, YandexRefreshToken,
};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const MAX_PENDING_REVIEW: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ConnectError {
    InvalidLocalScope,
    Callback(CallbackError),
    Exchange(YandexTokenExchangeError),
    Account(YandexAccountError),
    Disk(YandexDiskRestError),
    MissingAppRoot,
    MissingRefreshToken,
    AccountNotConfirmed,
    ReviewExpired,
    SecretStore(SecretError),
}

impl ConnectError {
    /// Public status only; never expose token, OAuth code, network body or
    /// account identifiers in an error or GUI log.
    pub fn public_status(&self) -> &'static str {
        match self {
            Self::InvalidLocalScope => "YANDEX_LOCAL_SCOPE_INVALID / SYNC_DISABLED",
            Self::Callback(_) => "YANDEX_CALLBACK_REJECTED / SYNC_DISABLED",
            Self::Exchange(_) => "YANDEX_OAUTH_EXCHANGE_FAILED / SYNC_DISABLED",
            Self::Account(_) => "YANDEX_ACCOUNT_VERIFICATION_FAILED / SYNC_DISABLED",
            Self::Disk(_) => "YANDEX_APP_FOLDER_UNAVAILABLE / SYNC_DISABLED",
            Self::MissingAppRoot => "YANDEX_APP_ROOT_UNVERIFIED / SYNC_DISABLED",
            Self::MissingRefreshToken => "YANDEX_REFRESH_TOKEN_MISSING / SYNC_DISABLED",
            Self::AccountNotConfirmed => "YANDEX_ACCOUNT_NOT_CONFIRMED / SYNC_DISABLED",
            Self::ReviewExpired => "YANDEX_CONFIRMATION_EXPIRED / SYNC_DISABLED",
            Self::SecretStore(_) => "YANDEX_OS_KEYRING_ERROR / SYNC_DISABLED",
        }
    }
}

/// A connection has an exact registered app, local Vault and local profile
/// before the browser is opened. No account is silently selected from
/// existing login state or a previous credential.
pub struct ConnectSession {
    callback: BoundLogin,
    app: String,
    vault: String,
    profile: String,
}

impl ConnectSession {
    pub fn begin(
        registered_app: &str,
        registered_port: u16,
        local_vault_uuid: &str,
        local_profile: &str,
    ) -> Result<Self, ConnectError> {
        // Validate the stable local scope before binding any listening port.
        // The dummy numeric ID is NOT used to access or store a credential.
        YandexCredentialScope::new("1", local_vault_uuid, local_profile, registered_app)
            .map_err(|_| ConnectError::InvalidLocalScope)?;
        let callback = BoundLogin::bind(registered_app, registered_port)
            .map_err(ConnectError::Callback)?;
        Ok(Self {
            callback,
            app: registered_app.to_owned(),
            vault: local_vault_uuid.to_owned(),
            profile: local_profile.to_owned(),
        })
    }

    pub fn browser_authorize_url(&self) -> &str {
        self.callback.browser_authorize_url()
    }

    /// Consume a one-use loopback callback, exchange its code, verify Yandex
    /// user + OAuth app and observe app folder. NO secret is saved yet.
    /// This function is a blocking network action for a dedicated worker.
    pub fn finish(self, timeout: Duration) -> Result<ReadOnlyReview, ConnectError> {
        let code = self.callback.receive(timeout).map_err(ConnectError::Callback)?;
        review_with(&LivePorts, code, &self.app, &self.vault, &self.profile)
    }
}

struct ServerProof {
    subject_id: String,
    root_observed: bool,
    refresh: Option<Zeroizing<String>>,
}

trait ConnectPorts {
    fn verify(
        &self,
        one_use_code: YandexTokenExchange,
        expected_app: &str,
    ) -> Result<ServerProof, ConnectError>;

    fn save(
        &self,
        scope: &YandexCredentialScope,
        refresh_token: &str,
    ) -> Result<(), ConnectError>;
}

struct LivePorts;

impl ConnectPorts for LivePorts {
    fn verify(
        &self,
        one_use_code: YandexTokenExchange,
        expected_app: &str,
    ) -> Result<ServerProof, ConnectError> {
        let tokens: YandexEphemeralTokens =
            one_use_code.exchange_tokens().map_err(ConnectError::Exchange)?;
        let account = tokens.verify_yandex_account(expected_app)
            .map_err(ConnectError::Account)?;
        let remote_root = tokens.with_access_token(|token| {
            YandexDiskReadOnlyClient::new().list_app_folder_page(
                token, &YandexAppPath::root(), None, 0, 1,
            )
        }).map_err(ConnectError::Disk)?;

        // An offset-based list can never prove that absent objects were
        // deleted. The root observation proves only a successful read-only
        // API response in this one account/app/token session.
        let root_observed = !remote_root
            .root_observation
            .observed_disk_path()
            .is_empty();
        let refresh = tokens.with_refresh_token(|value| {
            value.map(|raw| Zeroizing::new(raw.to_owned()))
        });
        Ok(ServerProof {
            subject_id: account.subject_id().to_owned(),
            root_observed,
            refresh,
        })
    }

    fn save(
        &self,
        scope: &YandexCredentialScope,
        refresh_token: &str,
    ) -> Result<(), ConnectError> {
        let vault = YandexCredentialVault::system()
            .map_err(ConnectError::SecretStore)?;
        let protected = YandexRefreshToken::new(refresh_token.to_owned())
            .map_err(ConnectError::SecretStore)?;
        // The OS backend must complete exact readback before success.
        vault.save_refresh(scope, &protected)
            .map_err(ConnectError::SecretStore)
    }
}

fn review_with<P: ConnectPorts>(
    ports: &P,
    code: YandexTokenExchange,
    app: &str,
    vault: &str,
    profile: &str,
) -> Result<ReadOnlyReview, ConnectError> {
    // Guard again even if a future host bypasses ConnectSession::begin.
    YandexCredentialScope::new("1", vault, profile, app)
        .map_err(|_| ConnectError::InvalidLocalScope)?;
    let proof = ports.verify(code, app)?;
    if !proof.root_observed {
        return Err(ConnectError::MissingAppRoot);
    }
    let refresh = proof.refresh.ok_or(ConnectError::MissingRefreshToken)?;
    YandexCredentialScope::new(&proof.subject_id, vault, profile, app)
        .map_err(|_| ConnectError::InvalidLocalScope)?;
    Ok(ReadOnlyReview {
        account_id: proof.subject_id,
        app_id: app.to_owned(),
        vault_id: vault.to_owned(),
        profile_id: profile.to_owned(),
        refresh,
        created_at: Instant::now(),
    })
}

/// The OS token has NOT been persisted. Only the selected subject ID may be
/// shown to the user for explicit confirmation. Never print or serialize
/// this object; it includes a protected refresh token in memory.
pub struct ReadOnlyReview {
    account_id: String,
    app_id: String,
    vault_id: String,
    profile_id: String,
    refresh: Zeroizing<String>,
    created_at: Instant,
}

impl ReadOnlyReview {
    pub fn observed_yandex_account_id(&self) -> &str {
        &self.account_id
    }

    pub fn is_sync_enabled(&self) -> bool {
        false
    }

    /// Only an explicit user confirmation for this exact verified Yandex ID
    /// can persist the refresh token, within two minutes of review.
    /// No file/sync rights are issued by this method.
    pub fn confirm_and_save(self, confirmed_account_id: &str)
        -> Result<ReadOnlyReceipt, ConnectError>
    {
        confirm_with(&LivePorts, self, confirmed_account_id)
    }
}

fn confirm_with<P: ConnectPorts>(
    ports: &P,
    review: ReadOnlyReview,
    confirmed_account_id: &str,
) -> Result<ReadOnlyReceipt, ConnectError> {
    if review.created_at.elapsed() > MAX_PENDING_REVIEW {
        return Err(ConnectError::ReviewExpired);
    }
    if review.account_id != confirmed_account_id {
        return Err(ConnectError::AccountNotConfirmed);
    }
    let scope = YandexCredentialScope::new(
        &review.account_id,
        &review.vault_id,
        &review.profile_id,
        &review.app_id,
    ).map_err(|_| ConnectError::InvalidLocalScope)?;
    ports.save(&scope, &review.refresh)?;
    Ok(ReadOnlyReceipt {
        account_id: review.account_id,
        app_id: review.app_id,
    })
}

/// OS keyring contains a Yandex refresh token bound to one account/app/Vault.
/// This is NOT a remote revision, a trusted Sync baseline or file-write right.
pub struct ReadOnlyReceipt {
    account_id: String,
    app_id: String,
}

impl ReadOnlyReceipt {
    pub fn account_id(&self) -> &str {
        &self.account_id
    }
    pub fn oauth_app_id(&self) -> &str {
        &self.app_id
    }
    pub fn is_sync_enabled(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akula_yandex_disk_rest::oauth::YandexPkceStart;
    use std::sync::Mutex;
    use url::Url;

    const APP: &str = "4760187d81bc4b7799476b42b5103713";
    const VAULT: &str = "12345678-1234-1234-1234-123456789abc";

    fn one_time_exchange() -> YandexTokenExchange {
        let start = YandexPkceStart::begin(APP, "http://127.0.0.1:42737/akula/yandex/callback")
            .unwrap();
        let csrf = Url::parse(&start.authorization_url).unwrap().query_pairs()
            .find(|(key, _)| key == "state").unwrap().1.into_owned();
        start.pending.complete(&format!(
            "http://127.0.0.1:42737/akula/yandex/callback?state={csrf}&code=testCode",
        )).unwrap()
    }

    struct FakePorts {
        proof: Mutex<Option<Result<ServerProof, ConnectError>>>,
        events: Mutex<Vec<&'static str>>,
    }

    impl FakePorts {
        fn success(subject: &str) -> Self {
            Self {
                proof: Mutex::new(Some(Ok(ServerProof {
                    subject_id: subject.to_owned(),
                    root_observed: true,
                    refresh: Some(Zeroizing::new("test-refresh".to_owned())),
                }))),
                events: Mutex::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<&'static str> {
            self.events.lock().unwrap().clone()
        }
    }

    impl ConnectPorts for FakePorts {
        fn verify(
            &self,
            code: YandexTokenExchange,
            expected_app: &str,
        ) -> Result<ServerProof, ConnectError> {
            self.events.lock().unwrap().push("verify");
            assert_eq!(expected_app, APP);
            assert!(code.form_body().contains("code=testCode"));
            self.proof.lock().unwrap().take().expect("once only")
        }

        fn save(
            &self,
            _scope: &YandexCredentialScope,
            token: &str,
        ) -> Result<(), ConnectError> {
            self.events.lock().unwrap().push("save");
            assert_eq!(token, "test-refresh");
            Ok(())
        }
    }

    #[test]
    fn verified_root_then_explicit_confirmation_then_os_save() {
        let ports = FakePorts::success("1000034426");
        let review = review_with(&ports, one_time_exchange(), APP, VAULT, "default").unwrap();
        assert!(!review.is_sync_enabled());
        assert_eq!(review.observed_yandex_account_id(), "1000034426");
        assert_eq!(ports.events(), vec!["verify"]);
        let receipt = confirm_with(&ports, review, "1000034426").unwrap();
        assert!(!receipt.is_sync_enabled());
        assert_eq!(receipt.oauth_app_id(), APP);
        assert_eq!(ports.events(), vec!["verify", "save"]);
    }

    #[test]
    fn wrong_account_never_saves_secret() {
        let ports = FakePorts::success("1000034426");
        let review = review_with(&ports, one_time_exchange(), APP, VAULT, "default").unwrap();
        assert!(matches!(
            confirm_with(&ports, review, "1000034427"),
            Err(ConnectError::AccountNotConfirmed)
        ));
        assert_eq!(ports.events(), vec!["verify"]);
    }

    #[test]
    fn absent_root_or_refresh_never_returns_review() {
        let ports = FakePorts {
            proof: Mutex::new(Some(Ok(ServerProof {
                subject_id: "1000034426".into(),
                root_observed: false,
                refresh: Some(Zeroizing::new("token".into())),
            }))),
            events: Mutex::new(Vec::new()),
        };
        assert!(matches!(
            review_with(&ports, one_time_exchange(), APP, VAULT, "default"),
            Err(ConnectError::MissingAppRoot)
        ));
        assert_eq!(ports.events(), vec!["verify"]);

        let ports = FakePorts {
            proof: Mutex::new(Some(Ok(ServerProof {
                subject_id: "1000034426".into(),
                root_observed: true,
                refresh: None,
            }))),
            events: Mutex::new(Vec::new()),
        };
        assert!(matches!(
            review_with(&ports, one_time_exchange(), APP, VAULT, "default"),
            Err(ConnectError::MissingRefreshToken)
        ));
        assert_eq!(ports.events(), vec!["verify"]);
    }

    #[test]
    fn malformed_local_scope_fails_before_exchanging_token() {
        let ports = FakePorts::success("1000034426");
        assert!(matches!(
            review_with(&ports, one_time_exchange(), APP, "../bad", "default"),
            Err(ConnectError::InvalidLocalScope)
        ));
        assert_eq!(ports.events(), Vec::<&str>::new());
        assert!(matches!(
            ConnectSession::begin(APP, 0, VAULT, "default"),
            Err(ConnectError::Callback(CallbackError::PortUnavailable))
        ));
    }

    #[test]
    fn two_minute_review_expiry_blocks_storage() {
        let ports = FakePorts::success("1000034426");
        let mut review = review_with(&ports, one_time_exchange(), APP, VAULT, "default").unwrap();
        review.created_at = Instant::now() - Duration::from_secs(121);
        assert!(matches!(
            confirm_with(&ports, review, "1000034426"),
            Err(ConnectError::ReviewExpired)
        ));
        assert_eq!(ports.events(), vec!["verify"]);
    }

    #[test]
    fn denied_server_operations_keep_sync_disabled() {
        let ports = FakePorts {
            proof: Mutex::new(Some(Err(ConnectError::Account(
                YandexAccountError::AccessDenied,
            )))),
            events: Mutex::new(Vec::new()),
        };
        assert!(matches!(
            review_with(&ports, one_time_exchange(), APP, VAULT, "default"),
            Err(ConnectError::Account(YandexAccountError::AccessDenied))
        ));
        assert_eq!(ports.events(), vec!["verify"]);
        assert_eq!(ConnectError::MissingAppRoot.public_status(),
            "YANDEX_APP_ROOT_UNVERIFIED / SYNC_DISABLED");
    }
}
