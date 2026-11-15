//! Yandex ID OAuth Authorization Code + PKCE/S256 preparation and callback validation.
//!
//! Offline-only, no browser launch, HTTP token exchange, token persistence or
//! authorization to list or mutate Akula data. A UI host must register the exact
//! redirect URI with Yandex, bind one outstanding request to its browser session,
//! and handle one callback by consuming the pending state.
//!
//! No Debug / Serialize / Clone for secrets; verifier, state and code are kept in
//! zeroizing buffers. OAuth or refresh tokens NEVER enter this module.

use ring::digest::{digest, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use std::time::{Duration, Instant};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

pub const YANDEX_AUTHORIZATION_URL: &str = "https://oauth.yandex.com/authorize";
pub const YANDEX_TOKEN_URL: &str = "https://oauth.yandex.com/token";
pub const YANDEX_DISK_APP_SCOPE: &str = "cloud_api:disk.app_folder";
const MAX_CALLBACK_LEN: usize = 4096;
const AUTH_CODE_LIFETIME: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YandexOAuthError {
    InvalidClientId,
    InvalidRedirect,
    EntropyUnavailable,
    CallbackExpired,
    CallbackMalformed,
    CallbackWrongOrigin,
    CallbackWrongState,
    AuthorizationDenied,
    InvalidCode,
}

pub struct YandexPkceStart {
    /// May be passed to a system browser. It does not contain code_verifier.
    pub authorization_url: String,
    /// One-use, process-local session. Do not serialize or log it.
    pub pending: YandexPkcePending,
}

pub struct YandexPkcePending {
    client_id: String,
    redirect_uri: Url,
    state: Zeroizing<String>,
    verifier: Zeroizing<String>,
    issued_at: Instant,
}

/// May be submitted to the fixed Yandex token endpoint by a later OS-backed
/// worker. Neither code nor verifier can be logged via derived Debug.
pub struct YandexTokenExchange {
    client_id: String,
    authorization_code: Zeroizing<String>,
    verifier: Zeroizing<String>,
}

impl YandexTokenExchange {
    pub fn endpoint(&self) -> &'static str {
        YANDEX_TOKEN_URL
    }

    /// Caller must send this as an ephemeral application/x-www-form-urlencoded
    /// POST body over a strict fixed-host HTTPS transport. Never persist it.
    pub fn form_body(&self) -> Zeroizing<String> {
        let mut builder = url::form_urlencoded::Serializer::new(String::new());
        builder
            .append_pair("grant_type", "authorization_code")
            .append_pair("client_id", &self.client_id)
            .append_pair("code", &self.authorization_code)
            .append_pair("code_verifier", &self.verifier);
        Zeroizing::new(builder.finish())
    }
}

impl YandexPkceStart {
    /// Redirect URI MUST already be registered in the Yandex OAuth application.
    /// Allow https without embedded credentials, or explicit IPv4 loopback with
    /// an OS-bound port. A custom non-HTTPS scheme is not admitted by this slice.
    pub fn begin(client_id: &str, redirect_uri: &str) -> Result<Self, YandexOAuthError> {
        if !(4..=128).contains(&client_id.len())
            || !client_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(YandexOAuthError::InvalidClientId);
        }
        let redirect = validate_redirect(redirect_uri)?;
        let rng = SystemRandom::new();
        let mut verifier_bytes = [0u8; 32];
        let mut state_bytes = [0u8; 32];
        rng.fill(&mut verifier_bytes)
            .map_err(|_| YandexOAuthError::EntropyUnavailable)?;
        rng.fill(&mut state_bytes)
            .map_err(|_| YandexOAuthError::EntropyUnavailable)?;
        let verifier = Zeroizing::new(lower_hex(&verifier_bytes));
        let state = Zeroizing::new(lower_hex(&state_bytes));
        verifier_bytes.zeroize();
        state_bytes.zeroize();

        let challenge = s256_challenge(verifier.as_str());
        let mut auth = Url::parse(YANDEX_AUTHORIZATION_URL)
            .expect("constant HTTPS Yandex endpoint");
        auth.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect.as_str())
            .append_pair("scope", YANDEX_DISK_APP_SCOPE)
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");

        Ok(Self {
            authorization_url: auth.to_string(),
            pending: YandexPkcePending {
                client_id: client_id.to_owned(),
                redirect_uri: redirect,
                state,
                verifier,
                issued_at: Instant::now(),
            },
        })
    }
}

impl YandexPkcePending {
    /// Consumes this exact attempt on success or failure. The caller cannot
    /// unintentionally process another callback with the same in-memory state.
    pub fn complete(self, callback_url: &str) -> Result<YandexTokenExchange, YandexOAuthError> {
        if self.issued_at.elapsed() > AUTH_CODE_LIFETIME {
            return Err(YandexOAuthError::CallbackExpired);
        }
        if callback_url.is_empty() || callback_url.len() > MAX_CALLBACK_LEN {
            return Err(YandexOAuthError::CallbackMalformed);
        }
        let callback = Url::parse(callback_url)
            .map_err(|_| YandexOAuthError::CallbackMalformed)?;
        if callback.username() != ""
            || callback.password().is_some()
            || callback.fragment().is_some()
            || callback.origin() != self.redirect_uri.origin()
            || callback.path() != self.redirect_uri.path()
        {
            return Err(YandexOAuthError::CallbackWrongOrigin);
        }

        let mut code: Option<String> = None;
        let mut state: Option<String> = None;
        let mut error: Option<String> = None;
        let mut description_seen = false;
        for (key, value) in callback.query_pairs() {
            match key.as_ref() {
                "code" if code.is_none() => code = Some(value.into_owned()),
                "state" if state.is_none() => state = Some(value.into_owned()),
                "error" if error.is_none() => error = Some(value.into_owned()),
                "error_description" if !description_seen => description_seen = true,
                _ => return Err(YandexOAuthError::CallbackMalformed),
            }
        }
        if state.as_deref() != Some(self.state.as_str()) {
            return Err(YandexOAuthError::CallbackWrongState);
        }
        if error.is_some() && code.is_some() {
            return Err(YandexOAuthError::CallbackMalformed);
        }
        if error.is_some() {
            return Err(YandexOAuthError::AuthorizationDenied);
        }
        let code = code.ok_or(YandexOAuthError::InvalidCode)?;
        if code.is_empty()
            || code.len() > 2048
            || !code.bytes().all(|c| c.is_ascii_graphic())
        {
            return Err(YandexOAuthError::InvalidCode);
        }
        Ok(YandexTokenExchange {
            client_id: self.client_id,
            authorization_code: Zeroizing::new(code),
            verifier: self.verifier,
        })
    }
}

fn validate_redirect(uri: &str) -> Result<Url, YandexOAuthError> {
    if uri.len() > 2048 {
        return Err(YandexOAuthError::InvalidRedirect);
    }
    let redirect = Url::parse(uri).map_err(|_| YandexOAuthError::InvalidRedirect)?;
    if redirect.username() != ""
        || redirect.password().is_some()
        || redirect.fragment().is_some()
        || redirect.query().is_some()
        || redirect.host_str().is_none()
    {
        return Err(YandexOAuthError::InvalidRedirect);
    }
    match redirect.scheme() {
        "https" => {}
        "http" if redirect.host_str() == Some("127.0.0.1") && redirect.port().is_some() => {}
        _ => return Err(YandexOAuthError::InvalidRedirect),
    };
    Ok(redirect)
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 15) as usize] as char);
    }
    text
}

fn base64url_no_padding(bytes: &[u8]) -> String {
    const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((bytes.len() * 4 + 2) / 3);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(B64[(a >> 2) as usize] as char);
        out.push(B64[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        if chunk.len() >= 2 {
            out.push(B64[(((b & 15) << 2) | (c >> 6)) as usize] as char);
        }
        if chunk.len() == 3 {
            out.push(B64[(c & 63) as usize] as char);
        }
    }
    out
}

fn s256_challenge(verifier: &str) -> String {
    base64url_no_padding(digest(&SHA256, verifier.as_bytes()).as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT_ID: &str = "0123456789abcdef0123456789abcdef";
    const REDIRECT: &str = "http://127.0.0.1:42012/callback";

    #[test]
    fn rfc7636_pkce_s256_reference_vector() {
        assert_eq!(
            s256_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn start_is_unique_and_does_not_expose_verifier_or_token() {
        let a = YandexPkceStart::begin(CLIENT_ID, REDIRECT).unwrap();
        let b = YandexPkceStart::begin(CLIENT_ID, REDIRECT).unwrap();
        let url = Url::parse(&a.authorization_url).unwrap();
        let params: std::collections::BTreeMap<_, _> =
            url.query_pairs().into_owned().collect();
        assert_eq!(url.as_str().split('?').next(), Some(YANDEX_AUTHORIZATION_URL));
        assert_eq!(params["scope"], YANDEX_DISK_APP_SCOPE);
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["redirect_uri"], REDIRECT);
        assert_eq!(params["response_type"], "code");
        assert_eq!(params["code_challenge"].len(), 43);
        assert_eq!(params["state"].len(), 64);
        assert_ne!(a.pending.state.as_str(), b.pending.state.as_str());
        assert_ne!(a.pending.verifier.as_str(), b.pending.verifier.as_str());
        assert!(!a.authorization_url.contains(a.pending.verifier.as_str()));
        assert!(!a.authorization_url.contains("access_token"));
    }

    #[test]
    fn only_explicit_https_or_loopback_redirect_is_admitted() {
        for uri in [
            "http://example.org/callback",
            "http://localhost:42012/callback",
            "http://127.0.0.1/callback",
            "https://user:password@example.com/callback",
            "https://example.com/callback?code=evil",
            "https://example.com/callback#fragment",
            "file:///tmp/callback",
        ] {
            assert_eq!(validate_redirect(uri), Err(YandexOAuthError::InvalidRedirect));
        }
        assert!(validate_redirect(REDIRECT).is_ok());
        assert!(validate_redirect("https://oauth.yandex.com/verification_code").is_ok());
        assert_eq!(
            YandexPkceStart::begin("broken id!", REDIRECT).err(),
            Some(YandexOAuthError::InvalidClientId)
        );
    }

    #[test]
    fn matching_callback_prepares_one_time_secret_free_token_request() {
        let s = YandexPkceStart::begin(CLIENT_ID, REDIRECT).unwrap();
        let state = s.pending.state.to_string();
        let callback = format!("{REDIRECT}?code=exampleCode123&state={state}");
        let exchange = s.pending.complete(&callback).unwrap();
        assert_eq!(exchange.endpoint(), YANDEX_TOKEN_URL);
        let pairs: std::collections::BTreeMap<_, _> =
            url::form_urlencoded::parse(exchange.form_body().as_bytes())
                .into_owned()
                .collect();
        assert_eq!(pairs["grant_type"], "authorization_code");
        assert_eq!(pairs["client_id"], CLIENT_ID);
        assert_eq!(pairs["code"], "exampleCode123");
        assert_eq!(pairs["code_verifier"].len(), 64);
        assert!(!pairs.contains_key("client_secret"));
    }

    #[test]
    fn forged_duplicate_and_error_callbacks_fail_closed() {
        for suffix in [
            "code=abc&state=forged",
            "code=abc&code=again&state={state}",
            "code=abc&state={state}&state={state}",
            "code=abc&state={state}&access_token=leak",
            "error=access_denied&code=abc&state={state}",
            "error=access_denied&state={state}",
            "code=%0A&state={state}",
        ] {
            let s = YandexPkceStart::begin(CLIENT_ID, REDIRECT).unwrap();
            let tail = suffix.replace("{state}", s.pending.state.as_str());
            let url = format!("{REDIRECT}?{tail}");
            let result = s.pending.complete(&url);
            assert!(result.is_err());
        }
    }

    #[test]
    fn wrong_origin_and_expired_request_are_rejected() {
        let s = YandexPkceStart::begin(CLIENT_ID, REDIRECT).unwrap();
        let state = s.pending.state.to_string();
        let wrong = format!("http://127.0.0.1:42013/callback?code=abc&state={state}");
        assert!(matches!(
            s.pending.complete(&wrong),
            Err(YandexOAuthError::CallbackWrongOrigin)
        ));

        let mut s = YandexPkceStart::begin(CLIENT_ID, REDIRECT).unwrap();
        s.pending.issued_at = Instant::now() - Duration::from_secs(601);
        let valid = format!("{REDIRECT}?code=abc&state={}", s.pending.state.as_str());
        assert!(matches!(
            s.pending.complete(&valid),
            Err(YandexOAuthError::CallbackExpired)
        ));
    }
}
