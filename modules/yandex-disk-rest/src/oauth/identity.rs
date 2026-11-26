//! Read-only Yandex ID account binding using any valid Yandex OAuth token.
//!
//! No login:info privilege is requested. Yandex documents the standard
//! id/client_id fields for tokens issued with other Yandex service scopes.
//! An identity observation is NOT a Vault grant or permission to publish Sync.

use super::exchange::YandexEphemeralTokens;
use serde::Deserialize;
use std::io::Read;
use std::time::Duration;
use zeroize::Zeroizing;

const ACCOUNT_ENDPOINT: &str = "https://login.yandex.ru/info";
const MAX_ACCOUNT_BYTES: u64 = 8192;
const MAX_OAUTH_TOKEN_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum YandexAccountError {
    InvalidOAuthClient,
    InvalidToken,
    AccessDenied,
    RateLimited,
    ServiceUnavailable,
    NetworkOrTls,
    OversizedResponse,
    InvalidResponse,
    WrongOAuthApplication,
}

/// Stable Yandex subject ID seen through one caller-owned OAuth credential.
/// The account ID and app ID must be bound to a separate Vault+profile scope
/// BEFORE any refresh token is saved in a platform credential manager.
pub struct YandexAccountIdentity {
    subject_id: String,
    oauth_client_id: String,
}

impl YandexAccountIdentity {
    pub fn subject_id(&self) -> &str {
        &self.subject_id
    }

    pub fn oauth_client_id(&self) -> &str {
        &self.oauth_client_id
    }
}

#[derive(Deserialize)]
struct WireAccount {
    id: String,
    client_id: String,
}

fn valid_client_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn parse_account(
    body: &[u8],
    expected_client_id: &str,
) -> Result<YandexAccountIdentity, YandexAccountError> {
    if body.len() as u64 > MAX_ACCOUNT_BYTES {
        return Err(YandexAccountError::OversizedResponse);
    }
    if !valid_client_id(expected_client_id) {
        return Err(YandexAccountError::InvalidOAuthClient);
    }
    let account: WireAccount =
        serde_json::from_slice(body).map_err(|_| YandexAccountError::InvalidResponse)?;
    if account.client_id != expected_client_id {
        return Err(YandexAccountError::WrongOAuthApplication);
    }
    if account.id.is_empty()
        || account.id.len() > 64
        || !account.id.bytes().all(|c| c.is_ascii_digit())
    {
        return Err(YandexAccountError::InvalidResponse);
    }
    Ok(YandexAccountIdentity {
        subject_id: account.id,
        oauth_client_id: account.client_id,
    })
}

/// A single fixed-host GET. Never puts bearer credentials in a query string,
/// redirects, SQLite, a logging context, or any Akula workspace file.
fn read_account(token: &str, expected_client_id: &str) -> Result<YandexAccountIdentity, YandexAccountError> {
    if token.is_empty()
        || token.len() > MAX_OAUTH_TOKEN_BYTES
        || !token.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(YandexAccountError::InvalidToken);
    }
    if !valid_client_id(expected_client_id) {
        return Err(YandexAccountError::InvalidOAuthClient);
    }
    let header = Zeroizing::new(format!("OAuth {token}"));
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(4))
        .timeout_read(Duration::from_secs(12))
        .timeout_write(Duration::from_secs(12))
        .redirects(0)
        .build();
    let result = agent
        .get(ACCOUNT_ENDPOINT)
        .query("format", "json")
        .set("Authorization", header.as_str())
        .set("Accept", "application/json")
        .call();
    let response = match result {
        Ok(r) if r.status() == 200 => r,
        Ok(_) => return Err(YandexAccountError::InvalidResponse),
        Err(ureq::Error::Status(401 | 403, _)) => return Err(YandexAccountError::AccessDenied),
        Err(ureq::Error::Status(429, _)) => return Err(YandexAccountError::RateLimited),
        Err(ureq::Error::Status(500..=599, _)) => {
            return Err(YandexAccountError::ServiceUnavailable);
        }
        Err(ureq::Error::Status(_, _)) => return Err(YandexAccountError::InvalidResponse),
        Err(ureq::Error::Transport(_)) => return Err(YandexAccountError::NetworkOrTls),
    };
    let mut bytes = Zeroizing::new(Vec::new());
    response
        .into_reader()
        .take(MAX_ACCOUNT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| YandexAccountError::NetworkOrTls)?;
    parse_account(bytes.as_slice(), expected_client_id)
}

impl YandexEphemeralTokens {
    /// Caller-owned, explicitly requested read-only probe.
    /// This does not save the identity or authorize file synchronization.
    pub fn verify_yandex_account(
        &self,
        expected_client_id: &str,
    ) -> Result<YandexAccountIdentity, YandexAccountError> {
        self.with_access_token(|token| read_account(token, expected_client_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const APP: &str = "4760187d81bc4b7799476b42b5103713";

    #[test]
    fn official_standard_user_info_fields_bind_oauth_app() {
        let json = br#"{"login":"ivan","id":"1000034426","client_id":"4760187d81bc4b7799476b42b5103713","psuid":"1.TEST"}"#;
        let account = parse_account(json, APP).unwrap();
        assert_eq!(account.subject_id(), "1000034426");
        assert_eq!(account.oauth_client_id(), APP);
    }

    #[test]
    fn different_app_or_identity_encoding_fails_closed() {
        let mismatched = json!({"id":"1000034426","client_id":"differentapp"});
        assert!(matches!(
            parse_account(&serde_json::to_vec(&mismatched).unwrap(), APP),
            Err(YandexAccountError::WrongOAuthApplication)
        ));
        let invalid = [
            json!({"id":1,"client_id":APP}),
            json!({"id":"abc","client_id":APP}),
            json!({"id":"0\n1","client_id":APP}),
            json!({"id":"","client_id":APP}),
            json!({"client_id":APP}),
            json!({"id":"123"}),
        ];
        for value in invalid {
            assert!(matches!(
                parse_account(&serde_json::to_vec(&value).unwrap(), APP),
                Err(YandexAccountError::InvalidResponse)
            ));
        }
    }

    #[test]
    fn no_scope_escalation_and_no_arbitrary_json_or_oversized_response() {
        assert!(matches!(
            parse_account(b"null", APP),
            Err(YandexAccountError::InvalidResponse)
        ));
        assert!(matches!(
            parse_account(&vec![b'a'; (MAX_ACCOUNT_BYTES + 1) as usize], APP),
            Err(YandexAccountError::OversizedResponse)
        ));
        assert!(matches!(
            parse_account(br#"{"id":"1","client_id":"x"}"#, "\n"),
            Err(YandexAccountError::InvalidOAuthClient)
        ));
    }
}
