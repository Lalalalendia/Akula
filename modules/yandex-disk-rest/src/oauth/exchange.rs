//! One-shot Yandex authorization-code token exchange; fixed HTTPS endpoint.
//!
//! Explicitly opt-in, no browser listener, no secret persistence, no refresh,
//! no File Gate, and no sync activation. Call only after a validated OAuth
//! callback and from a dedicated worker, never the native UI thread.
//!
//! Avoid logging the HTTP request/response: both contain credential material.
//! HTTPS clients and allocators can retain copies despite local zeroization.

use super::{YANDEX_DISK_APP_SCOPE, YandexTokenExchange};
use serde::Deserialize;
use std::io::Read;
use std::time::Duration;
use zeroize::Zeroizing;

const MAX_TOKEN_RESPONSE_BYTES: u64 = 16 * 1024;
const MAX_OAUTH_TOKEN_LENGTH: usize = 4096;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum YandexTokenExchangeError {
    InvalidOrExpiredCode,
    ClientRejected,
    ScopeRejected,
    AuthorizationRejected,
    RateLimited,
    ProviderUnavailable,
    NetworkOrTls,
    ResponseTooLarge,
    InvalidResponse,
}

pub struct YandexEphemeralTokens {
    access: Zeroizing<String>,
    refresh: Option<Zeroizing<String>>,
    expires_in_seconds: u64,
    scope_explicitly_confirmed: bool,
}

impl YandexEphemeralTokens {
    /// A short-lived borrowing interface. The caller must never log or persist
    /// the access token outside an OS-approved secret store. No Sync authority.
    pub fn with_access_token<R>(&self, action: impl FnOnce(&str) -> R) -> R {
        action(self.access.as_str())
    }

    pub fn with_refresh_token<R>(&self, action: impl FnOnce(Option<&str>) -> R) -> R {
        action(self.refresh.as_ref().map(|v| v.as_str()))
    }

    pub fn expires_in_seconds(&self) -> u64 {
        self.expires_in_seconds
    }

    /// A missing scope in the Yandex response is explicitly allowed by the
    /// API for a full grant. In that case API access is not yet independently
    /// verified and the caller must still perform a read-only 200 probe.
    pub fn scope_explicitly_confirmed(&self) -> bool {
        self.scope_explicitly_confirmed
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    token_type: String,
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct TokenFailure {
    error: String,
}

fn valid_token(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= MAX_OAUTH_TOKEN_LENGTH
        && raw.bytes().all(|b| b.is_ascii_graphic())
}

fn parse_success(body: &[u8]) -> Result<YandexEphemeralTokens, YandexTokenExchangeError> {
    if body.len() as u64 > MAX_TOKEN_RESPONSE_BYTES {
        return Err(YandexTokenExchangeError::ResponseTooLarge);
    }
    let raw: TokenResponse =
        serde_json::from_slice(body).map_err(|_| YandexTokenExchangeError::InvalidResponse)?;
    let access = Zeroizing::new(raw.access_token);
    let refresh = raw.refresh_token.map(Zeroizing::new);
    if !raw.token_type.eq_ignore_ascii_case("bearer")
        || raw.expires_in == 0
        || !valid_token(access.as_str())
        || refresh
            .as_ref()
            .is_some_and(|token| !valid_token(token.as_str()))
    {
        return Err(YandexTokenExchangeError::InvalidResponse);
    }
    let scope_explicitly_confirmed = match raw.scope {
        Some(scope) => {
            if !scope
                .split_ascii_whitespace()
                .any(|v| v == YANDEX_DISK_APP_SCOPE)
            {
                return Err(YandexTokenExchangeError::ScopeRejected);
            }
            true
        }
        None => false,
    };
    Ok(YandexEphemeralTokens {
        access,
        refresh,
        expires_in_seconds: raw.expires_in,
        scope_explicitly_confirmed,
    })
}

fn classify_failure(status: u16, body: &[u8]) -> YandexTokenExchangeError {
    match status {
        429 => YandexTokenExchangeError::RateLimited,
        500..=599 => YandexTokenExchangeError::ProviderUnavailable,
        400 | 401 | 403 => {
            let parsed: TokenFailure = match serde_json::from_slice(body) {
                Ok(err) => err,
                Err(_) => return YandexTokenExchangeError::InvalidResponse,
            };
            match parsed.error.as_str() {
                "invalid_grant" | "bad_verification_code" => {
                    YandexTokenExchangeError::InvalidOrExpiredCode
                }
                "invalid_scope" => YandexTokenExchangeError::ScopeRejected,
                "invalid_client" | "unauthorized_client" => {
                    YandexTokenExchangeError::ClientRejected
                }
                _ => YandexTokenExchangeError::AuthorizationRejected,
            }
        }
        _ => YandexTokenExchangeError::ProviderUnavailable,
    }
}

fn read_response(response: ureq::Response) -> Result<Zeroizing<Vec<u8>>, YandexTokenExchangeError> {
    let mut bytes = Zeroizing::new(Vec::new());
    response
        .into_reader()
        .take(MAX_TOKEN_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| YandexTokenExchangeError::NetworkOrTls)?;
    if bytes.len() as u64 > MAX_TOKEN_RESPONSE_BYTES {
        return Err(YandexTokenExchangeError::ResponseTooLarge);
    }
    Ok(bytes)
}

impl YandexTokenExchange {
    /// One attempt only: consumes the authorization code and verifier.
    /// This method intentionally does not retry an uncertain server response.
    pub fn exchange_tokens(self) -> Result<YandexEphemeralTokens, YandexTokenExchangeError> {
        let body = self.form_body();
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(15))
            .timeout_write(Duration::from_secs(15))
            .redirects(0)
            .build();
        let request = agent
            .post(self.endpoint())
            .set("Accept", "application/json")
            .set("Content-Type", "application/x-www-form-urlencoded");
        match request.send_string(body.as_str()) {
            Ok(response) if response.status() == 200 => {
                let bytes = read_response(response)?;
                parse_success(bytes.as_slice())
            }
            Ok(_) => Err(YandexTokenExchangeError::InvalidResponse),
            Err(ureq::Error::Status(code, response)) => {
                let bytes = read_response(response)?;
                Err(classify_failure(code, bytes.as_slice()))
            }
            Err(ureq::Error::Transport(_)) => Err(YandexTokenExchangeError::NetworkOrTls),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(scope: Option<&str>) -> Vec<u8> {
        let mut v = json!({
            "token_type": "bearer",
            "access_token": "AQAAA-example-token",
            "refresh_token": "refresh-protected-value",
            "expires_in": 3600
        });
        if let Some(scope) = scope {
            v["scope"] = json!(scope);
        }
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn granted_scope_and_ephemeral_secrets_remain_local() {
        let tokens = parse_success(&response(Some(YANDEX_DISK_APP_SCOPE))).unwrap();
        assert_eq!(tokens.expires_in_seconds(), 3600);
        assert!(tokens.scope_explicitly_confirmed());
        tokens.with_access_token(|t| assert_eq!(t, "AQAAA-example-token"));
        tokens.with_refresh_token(|t| assert_eq!(t, Some("refresh-protected-value")));
    }

    #[test]
    fn optional_omitted_scope_requires_separate_api_verification() {
        let tokens = parse_success(&response(None)).unwrap();
        assert!(!tokens.scope_explicitly_confirmed());
    }

    #[test]
    fn wrong_scope_is_never_implicitly_upgraded() {
        assert!(matches!(
            parse_success(&response(Some("cloud_api:disk.read"))),
            Err(YandexTokenExchangeError::ScopeRejected)
        ));
        assert!(matches!(
            parse_success(&response(Some(""))),
            Err(YandexTokenExchangeError::ScopeRejected)
        ));
    }

    #[test]
    fn invalid_token_type_and_control_characters_fail_closed() {
        let cases = [
            json!({"token_type": "mac", "access_token": "test", "expires_in": 1}),
            json!({"token_type": "bearer", "access_token": "a\nb", "expires_in": 1}),
            json!({"token_type": "bearer", "access_token": "", "expires_in": 1}),
            json!({"token_type": "bearer", "access_token": "test", "refresh_token":"bad\n", "expires_in":1}),
            json!({"token_type": "bearer", "access_token": "test", "expires_in": 0}),
            json!({"token_type": "bearer", "access_token": "test"}),
            json!({"token_type": "bearer", "access_token": 17, "expires_in": 1}),
        ];
        for value in cases {
            let data = serde_json::to_vec(&value).unwrap();
            assert!(matches!(
                parse_success(&data),
                Err(YandexTokenExchangeError::InvalidResponse)
            ));
        }
    }

    #[test]
    fn unbounded_tokens_and_responses_fail_closed() {
        let mut body = response(None);
        body.extend(vec![b' '; (MAX_TOKEN_RESPONSE_BYTES + 1) as usize]);
        assert!(matches!(
            parse_success(&body),
            Err(YandexTokenExchangeError::ResponseTooLarge)
        ));
        let long = json!({
            "token_type": "bearer", "access_token": "a".repeat(MAX_OAUTH_TOKEN_LENGTH + 1),
            "expires_in": 1
        });
        assert!(matches!(
            parse_success(&serde_json::to_vec(&long).unwrap()),
            Err(YandexTokenExchangeError::InvalidResponse)
        ));
    }

    #[test]
    fn oauth_errors_are_typed_and_never_expose_the_server_description() {
        let cases = [
            (400, "invalid_grant", YandexTokenExchangeError::InvalidOrExpiredCode),
            (401, "invalid_client", YandexTokenExchangeError::ClientRejected),
            (403, "unauthorized_client", YandexTokenExchangeError::ClientRejected),
            (400, "invalid_scope", YandexTokenExchangeError::ScopeRejected),
            (400, "invalid_request", YandexTokenExchangeError::AuthorizationRejected),
        ];
        for (http, error, expected) in cases {
            let body = serde_json::to_vec(&json!({"error":error, "error_description":"SECRET"})).unwrap();
            assert_eq!(classify_failure(http, &body), expected);
        }
        assert_eq!(classify_failure(429, b""), YandexTokenExchangeError::RateLimited);
        assert_eq!(classify_failure(503, b""), YandexTokenExchangeError::ProviderUnavailable);
        assert_eq!(classify_failure(401, b"invalid json"), YandexTokenExchangeError::InvalidResponse);
    }
}
